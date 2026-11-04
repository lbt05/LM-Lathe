use std::time::Duration;

/// Errors produced by the global scheduler.
#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    /// Not enough memory is available and no evictable tenant could free any.
    #[error("insufficient resources: requested {requested} bytes, {available} bytes available")]
    InsufficientMemory { requested: u64, available: u64 },

    /// The acquisition did not complete within the allotted window.
    #[error("resource acquisition timed out after {0:?}")]
    Timeout(Duration),

    /// The tenant key is unknown to the scheduler.
    #[error("tenant `{0}` is not registered")]
    UnknownTenant(String),

    /// The tenant is pinned and may not be evicted automatically.
    #[error("tenant `{0}` is pinned and cannot be evicted")]
    TenantPinned(String),

    /// The tenant still has in-flight requests.
    #[error("tenant `{0}` has {1} active request(s) and cannot be evicted")]
    TenantBusy(String, u32),

    /// The eviction callback of a tenant failed.
    #[error("eviction of tenant `{tenant}` failed: {reason}")]
    EvictionFailed { tenant: String, reason: String },

    /// The scheduler is shutting down.
    #[error("scheduler is shutting down")]
    ShuttingDown,
}
