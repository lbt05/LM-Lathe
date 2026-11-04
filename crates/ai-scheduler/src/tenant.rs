use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The kind of workload a tenant represents. Used for reporting and for
/// model-specific eviction policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantKind {
    Llm,
    Tts,
    Diffusion,
    Asr,
    Other,
}

impl TenantKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TenantKind::Llm => "llm",
            TenantKind::Tts => "tts",
            TenantKind::Diffusion => "diffusion",
            TenantKind::Asr => "asr",
            TenantKind::Other => "other",
        }
    }
}

/// Lifecycle state of a tenant inside the scheduler.
///
/// This mirrors the runtime lifecycle of the specification:
/// `Unloaded -> Loading -> Running -> Unloading -> Unloaded`, plus `Failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantState {
    /// Registered but nothing is running yet.
    Unloaded,
    /// Resources reserved, process starting up.
    Loading,
    /// Process is up and serving requests.
    Running,
    /// Process is shutting down.
    Unloading,
    /// The runtime failed to start.
    Failed,
}

impl TenantState {
    pub fn as_str(&self) -> &'static str {
        match self {
            TenantState::Unloaded => "unloaded",
            TenantState::Loading => "loading",
            TenantState::Running => "running",
            TenantState::Unloading => "unloading",
            TenantState::Failed => "failed",
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, TenantState::Loading | TenantState::Running)
    }
}

/// Internal bookkeeping for one registered tenant.
#[derive(Clone)]
pub struct TenantEntry {
    /// Stable identifier, e.g. `tts:qwen3-tts-1.7b-voicedesign`.
    pub key: String,
    /// Workload family.
    pub kind: TenantKind,
    /// Pinned tenants are never evicted automatically.
    pub pinned: bool,
    /// Number of in-flight requests.
    pub active_requests: u32,
    /// Memory the runtime estimated it needs.
    pub estimated_bytes: u64,
    /// Memory actually observed once the runtime was up.
    pub actual_bytes: Option<u64>,
    /// Bytes currently reserved in the pool on behalf of this tenant.
    pub reserved_bytes: u64,
    /// Last time the tenant was used (request or explicit touch).
    pub last_used_at: DateTime<Utc>,
    /// Idle time after which the tenant becomes evictable.
    pub ttl: Duration,
    /// Current lifecycle state.
    pub state: TenantState,
    /// Optional callback used by the scheduler to unload the tenant.
    pub evictor: Option<crate::scheduler::Evictor>,
}

impl TenantEntry {
    /// Duration the tenant has been idle.
    pub fn idle_for(&self, now: DateTime<Utc>) -> std::time::Duration {
        (now - self.last_used_at).to_std().unwrap_or_default()
    }

    /// Whether the tenant may be evicted right now.
    ///
    /// Mirrors the specification: a tenant is evictable when it is not pinned,
    /// has no in-flight request and exceeded its TTL. A tenant without an
    /// eviction callback is never evicted automatically, because the
    /// scheduler cannot stop its runtime.
    pub fn is_evictable(&self, now: DateTime<Utc>) -> bool {
        !self.pinned
            && self.active_requests == 0
            && self.state.is_active()
            && self.evictor.is_some()
            && self.idle_for(now) > self.ttl
    }

    /// Public, serialisable view of the tenant.
    pub fn snapshot(&self) -> TenantSnapshot {
        TenantSnapshot {
            key: self.key.clone(),
            kind: self.kind,
            pinned: self.pinned,
            active_requests: self.active_requests,
            estimated_bytes: self.estimated_bytes,
            actual_bytes: self.actual_bytes,
            reserved_bytes: self.reserved_bytes,
            last_used_at: self.last_used_at,
            ttl_secs: self.ttl.as_secs(),
            state: self.state,
        }
    }
}

/// Serialisable view of a tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantSnapshot {
    pub key: String,
    pub kind: TenantKind,
    pub pinned: bool,
    pub active_requests: u32,
    pub estimated_bytes: u64,
    pub actual_bytes: Option<u64>,
    pub reserved_bytes: u64,
    pub last_used_at: DateTime<Utc>,
    pub ttl_secs: u64,
    pub state: TenantState,
}
