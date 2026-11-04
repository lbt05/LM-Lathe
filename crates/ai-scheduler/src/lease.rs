use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use crate::scheduler::Scheduler;

/// RAII handle for a memory reservation.
///
/// The reservation is returned to the pool as soon as the last clone of the
/// lease is dropped, which guarantees that a runtime releasing its lease has
/// already stopped its process.
#[derive(Clone)]
pub struct MemoryLease {
    inner: Arc<LeaseInner>,
}

struct LeaseInner {
    scheduler: Weak<Scheduler>,
    key: String,
    bytes: u64,
    released: AtomicBool,
}

impl MemoryLease {
    pub(crate) fn new(scheduler: Arc<Scheduler>, key: String, bytes: u64) -> Self {
        Self {
            inner: Arc::new(LeaseInner {
                scheduler: Arc::downgrade(&scheduler),
                key,
                bytes,
                released: AtomicBool::new(false),
            }),
        }
    }

    /// Tenant the reservation belongs to.
    pub fn key(&self) -> &str {
        &self.inner.key
    }

    /// Number of reserved bytes.
    pub fn bytes(&self) -> u64 {
        self.inner.bytes
    }

    /// Releases the reservation eagerly. Dropping the lease afterwards is a
    /// no-op.
    pub fn release(&self) {
        if self.inner.released.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(scheduler) = self.inner.scheduler.upgrade() else {
            return;
        };
        scheduler.release(&self.inner.key, self.inner.bytes);
    }

    /// `true` once the reservation has been returned to the pool.
    pub fn is_released(&self) -> bool {
        self.inner.released.load(Ordering::SeqCst)
    }
}

impl Drop for LeaseInner {
    fn drop(&mut self) {
        if self.released.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(scheduler) = self.scheduler.upgrade() else {
            return;
        };
        // `Scheduler::release` only takes the (non-async) state lock, so it is
        // safe to call it here as long as we never drop a lease while holding
        // that lock. A poisoned lock is recovered by `Scheduler::lock`.
        scheduler.release(&self.key, self.bytes);
    }
}

impl std::fmt::Debug for MemoryLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryLease")
            .field("key", &self.inner.key)
            .field("bytes", &self.inner.bytes)
            .field("released", &self.is_released())
            .finish()
    }
}
