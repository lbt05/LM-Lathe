//! Global hardware-resource scheduler for every local AI model runtime.
//!
//! The TTS subsystem must **not** own a private memory pool. LLM, TTS and any
//! future diffusion runtime all register as *tenants* of a single [`Scheduler`]
//! and compete for the same budget.
//!
//! The scheduler guarantees the constraints listed in the TTS specification:
//!
//! 1. pinned models are never evicted automatically,
//! 2. models with in-flight requests are never force-unloaded,
//! 3. resources are only released after the owning process has exited,
//! 4. an unsatisfiable request fails with an explicit error,
//! 5. acquisition supports timeout and cancellation.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use chrono::{DateTime, Utc};

pub mod error;
pub mod lease;
pub mod pool;
pub mod scheduler;
pub mod tenant;

pub use error::SchedulerError;
pub use lease::MemoryLease;
pub use pool::MemoryPool;
pub use scheduler::{evictor_from_fn, Evictor, Scheduler, SchedulerConfig, SchedulerSnapshot};
pub use tenant::{TenantEntry, TenantKind, TenantSnapshot, TenantState};

/// Shared handle to the global scheduler.
pub type SharedScheduler = Arc<Scheduler>;

/// Convenience helper used by runtimes that want to fail fast instead of
/// waiting for the full acquisition timeout.
pub fn is_cancelled(cancelled: &AtomicBool) -> bool {
    cancelled.load(Ordering::Relaxed)
}

/// Snapshot of every registered tenant, sorted by key for stable output.
pub fn tenant_table(entries: &HashMap<String, TenantEntry>) -> Vec<TenantSnapshot> {
    let mut out: Vec<TenantSnapshot> = entries.values().map(TenantEntry::snapshot).collect();
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

/// Current wall-clock time; isolated so tests can reason about idle windows.
pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// Weak handle helper used by eviction callbacks to avoid reference cycles.
pub fn downgrade(scheduler: &Arc<Scheduler>) -> Weak<Scheduler> {
    Arc::downgrade(scheduler)
}
