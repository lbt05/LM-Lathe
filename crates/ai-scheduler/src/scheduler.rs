use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tokio::task::JoinHandle;

use crate::error::SchedulerError;
use crate::lease::MemoryLease;
use crate::pool::MemoryPool;
use crate::tenant::{TenantEntry, TenantKind, TenantSnapshot, TenantState};

/// Callback invoked when the scheduler decides to evict a tenant.
///
/// The callback is responsible for unloading the runtime and releasing every
/// resource the tenant holds. It is always invoked **without** the scheduler
/// lock held, so the callback may call back into the scheduler.
pub type Evictor = Arc<
    dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<(), SchedulerError>> + Send>>
        + Send
        + Sync,
>;

/// Configuration of the global scheduler.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Total memory budget, in bytes.
    pub total_bytes: u64,
    /// Memory that must stay free for the OS and transient allocations.
    pub reserved_bytes: u64,
    /// Default idle time-to-live for tenants without an explicit TTL.
    pub default_ttl: Duration,
    /// How often the TTL sweeper runs.
    pub sweep_interval: Duration,
    /// Default timeout for memory acquisition.
    pub acquire_timeout: Duration,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            total_bytes: 8 * 1024 * 1024 * 1024,
            reserved_bytes: 512 * 1024 * 1024,
            default_ttl: Duration::from_secs(30 * 60),
            sweep_interval: Duration::from_secs(60),
            acquire_timeout: Duration::from_secs(60),
        }
    }
}

/// Aggregated view of the scheduler, used by the desktop UI and the HTTP API.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SchedulerSnapshot {
    pub total_bytes: u64,
    pub reserved_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub tenants: Vec<TenantSnapshot>,
}

struct SchedulerState {
    pool: MemoryPool,
    tenants: std::collections::HashMap<String, TenantEntry>,
}

/// The global resource scheduler.
pub struct Scheduler {
    state: Mutex<SchedulerState>,
    config: SchedulerConfig,
    shutting_down: AtomicBool,
}

impl Scheduler {
    /// Creates a scheduler with the given configuration.
    pub fn new(config: SchedulerConfig) -> Arc<Self> {
        let pool = MemoryPool::new(config.total_bytes, config.reserved_bytes);
        Arc::new(Self {
            state: Mutex::new(SchedulerState {
                pool,
                tenants: std::collections::HashMap::new(),
            }),
            config,
            shutting_down: AtomicBool::new(false),
        })
    }

    /// Current configuration.
    pub fn config(&self) -> &SchedulerConfig {
        &self.config
    }

    /// Marks the scheduler as shutting down; new acquisitions fail.
    pub fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SchedulerState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Registers (or updates) a tenant.
    ///
    /// Registration alone does not reserve memory; call [`Scheduler::acquire`]
    /// for that.
    pub fn register_tenant(
        &self,
        key: &str,
        kind: TenantKind,
        estimated_bytes: u64,
        ttl: Option<Duration>,
        evictor: Option<Evictor>,
    ) -> TenantSnapshot {
        let now = Utc::now();
        let mut state = self.lock();
        let entry = state
            .tenants
            .entry(key.to_string())
            .or_insert_with(|| TenantEntry {
                key: key.to_string(),
                kind,
                pinned: false,
                active_requests: 0,
                estimated_bytes,
                actual_bytes: None,
                reserved_bytes: 0,
                last_used_at: now,
                ttl: self.config.default_ttl,
                state: TenantState::Unloaded,
                evictor: None,
            });
        entry.kind = kind;
        entry.estimated_bytes = estimated_bytes;
        entry.ttl = ttl.unwrap_or(self.config.default_ttl);
        if evictor.is_some() {
            entry.evictor = evictor;
        }
        entry.last_used_at = now;
        entry.snapshot()
    }

    /// Removes a tenant from the scheduler, releasing any remaining reservation.
    pub fn unregister_tenant(&self, key: &str) -> Option<TenantSnapshot> {
        let mut state = self.lock();
        let entry = state.tenants.remove(key)?;
        let reserved = entry.reserved_bytes;
        if reserved > 0 {
            state.pool.release(reserved);
        }
        Some(entry.snapshot())
    }

    /// Reserves `bytes` for `key`.
    ///
    /// If the pool is too small, unpinned and idle tenants are evicted in
    /// least-recently-used order until the request can be satisfied. The call
    /// fails with [`SchedulerError::InsufficientMemory`] when nothing can be
    /// evicted and with [`SchedulerError::Timeout`] when the deadline passes.
    pub async fn acquire(
        self: &Arc<Self>,
        key: &str,
        bytes: u64,
        timeout: Option<Duration>,
    ) -> Result<MemoryLease, SchedulerError> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(SchedulerError::ShuttingDown);
        }
        let timeout = timeout.unwrap_or(self.config.acquire_timeout);
        let deadline = Instant::now() + timeout;

        loop {
            let candidate = {
                let mut state = self.lock();
                if !state.tenants.contains_key(key) {
                    return Err(SchedulerError::UnknownTenant(key.to_string()));
                }
                if state.pool.try_reserve(bytes) {
                    let entry = state.tenants.get_mut(key).expect("tenant checked above");
                    entry.reserved_bytes = entry.reserved_bytes.saturating_add(bytes);
                    entry.last_used_at = Utc::now();
                    entry.state = TenantState::Loading;
                    return Ok(MemoryLease::new(self.clone(), key.to_string(), bytes));
                }
                let available = state.pool.available_bytes();
                // Least recently used tenant that may be evicted.
                let candidate = state
                    .tenants
                    .values()
                    .filter(|tenant| {
                        tenant.key != key
                            && !tenant.pinned
                            && tenant.active_requests == 0
                            && tenant.state.is_active()
                            && tenant.evictor.is_some()
                    })
                    .min_by_key(|tenant| tenant.last_used_at)
                    .map(|tenant| tenant.key.clone());
                match candidate {
                    Some(candidate) => candidate,
                    None => {
                        return Err(SchedulerError::InsufficientMemory {
                            requested: bytes,
                            available,
                        })
                    }
                }
            };

            if Instant::now() >= deadline {
                return Err(SchedulerError::Timeout(timeout));
            }

            self.evict(&candidate).await?;
        }
    }

    /// Releases `bytes` previously reserved for `key`.
    ///
    /// The release is ignored when the tenant is unknown, which happens when
    /// the scheduler already reclaimed the reservation while evicting the
    /// tenant. This keeps the pool consistent when a lease is dropped after
    /// its tenant was removed.
    pub fn release(&self, key: &str, bytes: u64) {
        let mut state = self.lock();
        let Some(entry) = state.tenants.get_mut(key) else {
            return;
        };
        let released = entry.reserved_bytes.min(bytes);
        entry.reserved_bytes -= released;
        state.pool.release(released);
    }

    /// Updates the last-used timestamp of a tenant.
    pub fn touch(&self, key: &str) {
        let mut state = self.lock();
        if let Some(entry) = state.tenants.get_mut(key) {
            entry.last_used_at = Utc::now();
        }
    }

    /// Marks the beginning of an inference request.
    pub fn begin_request(&self, key: &str) -> Result<u32, SchedulerError> {
        let mut state = self.lock();
        let entry = state
            .tenants
            .get_mut(key)
            .ok_or_else(|| SchedulerError::UnknownTenant(key.to_string()))?;
        entry.active_requests = entry.active_requests.saturating_add(1);
        entry.last_used_at = Utc::now();
        Ok(entry.active_requests)
    }

    /// Marks the end of an inference request.
    pub fn end_request(&self, key: &str) -> Result<u32, SchedulerError> {
        let mut state = self.lock();
        let entry = state
            .tenants
            .get_mut(key)
            .ok_or_else(|| SchedulerError::UnknownTenant(key.to_string()))?;
        entry.active_requests = entry.active_requests.saturating_sub(1);
        entry.last_used_at = Utc::now();
        Ok(entry.active_requests)
    }

    /// Pins or unpins a tenant.
    pub fn set_pinned(&self, key: &str, pinned: bool) -> Result<(), SchedulerError> {
        let mut state = self.lock();
        let entry = state
            .tenants
            .get_mut(key)
            .ok_or_else(|| SchedulerError::UnknownTenant(key.to_string()))?;
        entry.pinned = pinned;
        Ok(())
    }

    /// Records the memory a runtime actually uses.
    pub fn set_actual_bytes(&self, key: &str, bytes: u64) -> Result<(), SchedulerError> {
        let mut state = self.lock();
        let entry = state
            .tenants
            .get_mut(key)
            .ok_or_else(|| SchedulerError::UnknownTenant(key.to_string()))?;
        entry.actual_bytes = Some(bytes);
        Ok(())
    }

    /// Updates the lifecycle state of a tenant.
    pub fn set_state(&self, key: &str, state: TenantState) -> Result<(), SchedulerError> {
        let mut inner = self.lock();
        let entry = inner
            .tenants
            .get_mut(key)
            .ok_or_else(|| SchedulerError::UnknownTenant(key.to_string()))?;
        entry.state = state;
        Ok(())
    }

    /// Current snapshot of pool and tenants.
    pub fn snapshot(&self) -> SchedulerSnapshot {
        let state = self.lock();
        let mut tenants: Vec<TenantSnapshot> =
            state.tenants.values().map(TenantEntry::snapshot).collect();
        tenants.sort_by(|a, b| a.key.cmp(&b.key));
        SchedulerSnapshot {
            total_bytes: state.pool.total_bytes(),
            reserved_bytes: state.pool.reserved_bytes(),
            used_bytes: state.pool.used_bytes(),
            available_bytes: state.pool.available_bytes(),
            tenants,
        }
    }

    /// Snapshot of a single tenant.
    pub fn tenant(&self, key: &str) -> Option<TenantSnapshot> {
        self.lock().tenants.get(key).map(TenantEntry::snapshot)
    }

    /// Evicts every tenant whose TTL expired. Returns the evicted keys.
    pub async fn sweep_expired(&self) -> Result<Vec<String>, SchedulerError> {
        let now = Utc::now();
        let due: Vec<String> = {
            let state = self.lock();
            state
                .tenants
                .values()
                .filter(|t| t.is_evictable(now))
                .map(|t| t.key.clone())
                .collect()
        };
        let mut evicted = Vec::new();
        for key in due {
            match self.evict(&key).await {
                Ok(()) => evicted.push(key),
                Err(SchedulerError::EvictionFailed { tenant, .. }) => {
                    tracing::warn!(tenant = %tenant, "eviction callback failed");
                }
                Err(other) => return Err(other),
            }
        }
        Ok(evicted)
    }

    /// Starts the background TTL sweeper.
    pub fn spawn_sweeper(self: &Arc<Self>) -> JoinHandle<()> {
        let scheduler = self.clone();
        let interval = self.config.sweep_interval;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                if scheduler.shutting_down.load(Ordering::SeqCst) {
                    break;
                }
                if let Err(err) = scheduler.sweep_expired().await {
                    tracing::warn!(error = %err, "ttl sweep failed");
                }
            }
        })
    }

    /// Evicts a single tenant. The callback runs without the lock held.
    async fn evict(&self, key: &str) -> Result<(), SchedulerError> {
        let evictor = {
            let state = self.lock();
            state.tenants.get(key).and_then(|t| t.evictor.clone())
        };
        if let Some(evictor) = evictor {
            evictor(key.to_string())
                .await
                .map_err(|err| SchedulerError::EvictionFailed {
                    tenant: key.to_string(),
                    reason: err.to_string(),
                })?;
        }
        // The callback is expected to have stopped the runtime; make sure the
        // tenant disappears from the scheduler even if it forgot to unregister.
        let mut state = self.lock();
        if let Some(entry) = state.tenants.remove(key) {
            if entry.reserved_bytes > 0 {
                state.pool.release(entry.reserved_bytes);
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler")
            .field("total_bytes", &self.config.total_bytes)
            .field("tenants", &self.lock().tenants.len())
            .finish()
    }
}

/// Helper for building an evictor from a plain async closure.
pub fn evictor_from_fn<F, Fut>(f: F) -> Evictor
where
    F: Fn(String) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), SchedulerError>> + Send + 'static,
{
    Arc::new(move |key| Box::pin(f(key)))
}

/// Convenience accessor for the current time used by tests.
pub fn current_time() -> DateTime<Utc> {
    Utc::now()
}
