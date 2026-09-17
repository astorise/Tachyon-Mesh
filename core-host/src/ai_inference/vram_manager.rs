use anyhow::{anyhow, Result};
use std::collections::VecDeque;

/// Priority attached to resident accelerator memory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum VramPriority {
    /// Live generation memory. It is protected from predictive eviction.
    #[default]
    Active,
    /// Recently useful memory that can be dropped after `expires_at_secs`.
    Hot { expires_at_secs: u64 },
    /// Predictive prewarm memory. It is always the first reclamation target.
    Volatile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VramAllocation {
    id: String,
    bytes: usize,
    priority: VramPriority,
}

/// Small deterministic VRAM admission controller used by the Candle mapper.
///
/// The real allocator owns device tensors; this controller owns the residency
/// decisions and the ordering guarantees that keep speculative prewarm memory
/// from starving active inference.
#[derive(Clone, Debug)]
pub(crate) struct VramManager {
    capacity_bytes: usize,
    used_bytes: usize,
    allocations: VecDeque<VramAllocation>,
}

impl VramManager {
    pub(crate) fn new(capacity_bytes: usize) -> Self {
        Self {
            capacity_bytes,
            used_bytes: 0,
            allocations: VecDeque::new(),
        }
    }

    pub(crate) fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    pub(crate) fn contains(&self, id: &str) -> bool {
        self.allocations
            .iter()
            .any(|allocation| allocation.id == id)
    }

    pub(crate) fn allocate_safetensors(
        &mut self,
        id: impl Into<String>,
        bytes: usize,
        priority: VramPriority,
        now_secs: u64,
    ) -> Result<()> {
        let id = id.into();
        if bytes > self.capacity_bytes {
            return Err(anyhow!(
                "safetensors allocation requires {bytes} bytes, capacity is {}",
                self.capacity_bytes
            ));
        }

        self.evict_existing(&id);
        self.reclaim_for(bytes, now_secs);
        if self.free_bytes() < bytes {
            return Err(anyhow!(
                "insufficient VRAM after evicting volatile and expired hot allocations"
            ));
        }

        self.used_bytes = self.used_bytes.saturating_add(bytes);
        self.allocations.push_back(VramAllocation {
            id,
            bytes,
            priority,
        });
        Ok(())
    }

    fn free_bytes(&self) -> usize {
        self.capacity_bytes.saturating_sub(self.used_bytes)
    }

    fn evict_existing(&mut self, id: &str) {
        self.evict_matching(|allocation| allocation.id == id);
    }

    fn reclaim_for(&mut self, required_bytes: usize, now_secs: u64) {
        self.evict_matching(|allocation| allocation.priority == VramPriority::Volatile);
        if self.free_bytes() >= required_bytes {
            return;
        }

        self.evict_matching(|allocation| {
            matches!(
                allocation.priority,
                VramPriority::Hot { expires_at_secs } if expires_at_secs <= now_secs
            )
        });
    }

    fn evict_matching(&mut self, mut predicate: impl FnMut(&VramAllocation) -> bool) {
        let mut retained = VecDeque::with_capacity(self.allocations.len());
        while let Some(allocation) = self.allocations.pop_front() {
            if predicate(&allocation) {
                self.used_bytes = self.used_bytes.saturating_sub(allocation.bytes);
            } else {
                retained.push_back(allocation);
            }
        }
        self.allocations = retained;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volatile_allocations_are_evicted_before_active_requests() {
        let mut manager = VramManager::new(100);
        manager
            .allocate_safetensors("tenant-a-lora", 70, VramPriority::Volatile, 10)
            .expect("volatile prewarm should fit");

        manager
            .allocate_safetensors("live-prompt", 80, VramPriority::Active, 11)
            .expect("active request should evict volatile memory");

        assert!(!manager.contains("tenant-a-lora"));
        assert!(manager.contains("live-prompt"));
        assert_eq!(manager.used_bytes(), 80);
    }

    #[test]
    fn active_allocations_are_not_evicted_for_speculative_work() {
        let mut manager = VramManager::new(100);
        manager
            .allocate_safetensors("live-prompt", 80, VramPriority::Active, 10)
            .expect("active allocation should fit");

        let error = manager
            .allocate_safetensors("predictive-lora", 30, VramPriority::Volatile, 11)
            .expect_err("volatile prewarm must not evict active memory");

        assert!(error.to_string().contains("insufficient VRAM"));
        assert!(manager.contains("live-prompt"));
        assert!(!manager.contains("predictive-lora"));
    }

    #[test]
    fn expired_hot_allocations_are_reclaimed_after_volatile_memory() {
        let mut manager = VramManager::new(100);
        manager
            .allocate_safetensors(
                "recent-lora",
                50,
                VramPriority::Hot {
                    expires_at_secs: 20,
                },
                10,
            )
            .expect("hot allocation should fit");
        manager
            .allocate_safetensors("predictive-lora", 20, VramPriority::Volatile, 10)
            .expect("volatile allocation should fit");

        manager
            .allocate_safetensors("live-prompt", 80, VramPriority::Active, 21)
            .expect("expired hot and volatile allocations should be reclaimable");

        assert!(!manager.contains("recent-lora"));
        assert!(!manager.contains("predictive-lora"));
        assert!(manager.contains("live-prompt"));
    }
}
