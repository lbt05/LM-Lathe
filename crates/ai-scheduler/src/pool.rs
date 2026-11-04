/// A fixed-size memory budget shared by every model runtime.
///
/// `reserved_bytes` models the amount of memory that must stay free for the
/// operating system and for transient allocations (KV-cache growth, vocoder
/// staging buffers, ...). Only `capacity - used` is handed out to tenants.
#[derive(Debug, Clone)]
pub struct MemoryPool {
    total_bytes: u64,
    reserved_bytes: u64,
    used_bytes: u64,
}

impl MemoryPool {
    /// Creates a pool. `reserved_bytes` is clamped to the total capacity.
    pub fn new(total_bytes: u64, reserved_bytes: u64) -> Self {
        let reserved_bytes = reserved_bytes.min(total_bytes);
        Self {
            total_bytes,
            reserved_bytes,
            used_bytes: 0,
        }
    }

    /// Total physical budget of the pool.
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    /// Memory that is never handed out.
    pub fn reserved_bytes(&self) -> u64 {
        self.reserved_bytes
    }

    /// Memory currently reserved by tenants.
    pub fn used_bytes(&self) -> u64 {
        self.used_bytes
    }

    /// Memory that can still be reserved.
    pub fn available_bytes(&self) -> u64 {
        self.total_bytes
            .saturating_sub(self.reserved_bytes)
            .saturating_sub(self.used_bytes)
    }

    /// Attempts to reserve `bytes`. Returns `false` when the pool is too small.
    pub fn try_reserve(&mut self, bytes: u64) -> bool {
        if self.available_bytes() < bytes {
            return false;
        }
        self.used_bytes = self.used_bytes.saturating_add(bytes);
        true
    }

    /// Releases a previously reserved amount. Never underflows.
    pub fn release(&mut self, bytes: u64) {
        self.used_bytes = self.used_bytes.saturating_sub(bytes);
    }

    /// Grows the budget, e.g. after the user raised the limit in settings.
    pub fn set_total_bytes(&mut self, total_bytes: u64) {
        self.total_bytes = total_bytes;
        self.reserved_bytes = self.reserved_bytes.min(total_bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserve_and_release() {
        let mut pool = MemoryPool::new(1000, 100);
        assert_eq!(pool.available_bytes(), 900);
        assert!(pool.try_reserve(500));
        assert_eq!(pool.used_bytes(), 500);
        assert_eq!(pool.available_bytes(), 400);
        assert!(!pool.try_reserve(401));
        pool.release(500);
        assert_eq!(pool.available_bytes(), 900);
    }

    #[test]
    fn reserved_is_clamped() {
        let pool = MemoryPool::new(100, 500);
        assert_eq!(pool.reserved_bytes(), 100);
        assert_eq!(pool.available_bytes(), 0);
    }
}
