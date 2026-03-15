use crate::{BeaconChain, BeaconChainError, BeaconChainTypes};
use itertools::process_results;
use lru::LruCache;
use parking_lot::Mutex;
use std::num::NonZeroUsize;
use std::time::Duration;
use tracing::debug;
use types::Hash256;
use types::non_zero_usize::new_non_zero_usize;

const BLOCK_ROOT_CACHE_LIMIT: NonZeroUsize = new_non_zero_usize(512);
const LOOKUP_LIMIT: NonZeroUsize = new_non_zero_usize(8);
const METRICS_TIMEOUT: Duration = Duration::from_millis(100);

/// Cache for rejecting attestations to blocks from before finalization.
///
/// It stores a collection of block roots that are pre-finalization and therefore not known to fork
/// choice in `verify_head_block_is_known` during attestation processing.
#[derive(Default)]
pub struct PreFinalizationBlockCache {
    cache: Mutex<Cache>,
}

struct Cache {
    /// Set of block roots that are known to be pre-finalization.
    block_roots: LruCache<Hash256, ()>,
    /// Set of block roots that are the subject of single block lookups.
    in_progress_lookups: LruCache<Hash256, ()>,
}

impl Default for Cache {
    fn default() -> Self {
        Cache {
            block_roots: LruCache::new(BLOCK_ROOT_CACHE_LIMIT),
            in_progress_lookups: LruCache::new(LOOKUP_LIMIT),
        }
    }
}

impl<T: BeaconChainTypes> BeaconChain<T> {
    /// Check whether the block with `block_root` is known to be pre-finalization.
    ///
    /// The provided `block_root` is assumed to be unknown to fork choice. I.e., it
    /// is not known to be a descendant of the finalized block.
    ///
    /// Return `true` if the attestation to this block should be rejected outright,
    /// return `false` if more information is needed from a single-block-lookup.
    pub fn is_pre_finalization_block(&self, block_root: Hash256) -> Result<bool, BeaconChainError> {
        let mut cache = self.pre_finalization_block_cache.cache.lock();

        // Check the cache to see if we already know this pre-finalization block root.
        if cache.block_roots.contains(&block_root) {
            return Ok(true);
        }

        // Avoid repeating the disk lookup for blocks that are already subject to a network lookup.
        // Sync will take care of de-duplicating the single block lookups.
        if cache.in_progress_lookups.contains(&block_root) {
            return Ok(false);
        }

        // 1. Check memory for a recent pre-finalization block.
        let is_recent_finalized_block = self.with_head(|head| {
            process_results(
                head.beacon_state.rev_iter_block_roots(&self.spec),
                |mut iter| iter.any(|(_, root)| root == block_root),
            )
            .map_err(BeaconChainError::BeaconStateError)
        })?;
        if is_recent_finalized_block {
            cache.block_roots.put(block_root, ());
            return Ok(true);
        }

        // 2. Check on disk.
        if self.store.get_blinded_block(&block_root)?.is_some() {
            cache.block_roots.put(block_root, ());
            return Ok(true);
        }

        // 3. Check the network with a single block lookup.
        cache.in_progress_lookups.put(block_root, ());
        if cache.in_progress_lookups.len() == LOOKUP_LIMIT.get() {
            // NOTE: we expect this to occur sometimes if a lot of blocks that we look up fail to be
            // imported for reasons other than being pre-finalization. The cache will eventually
            // self-repair in this case by replacing old entries with new ones until all the failed
            // blocks have been flushed out. Solving this issue isn't as simple as hooking the
            // beacon processor's functions that handle failed blocks because we need the block root
            // and it has been erased from the `BlockError` by that point.
            debug!("Pre-finalization lookup cache is full");
        }
        Ok(false)
    }

    pub fn pre_finalization_block_rejected(&self, block_root: Hash256) {
        // Future requests can know that this block is invalid without having to look it up again.
        let mut cache = self.pre_finalization_block_cache.cache.lock();
        cache.in_progress_lookups.pop(&block_root);
        cache.block_roots.put(block_root, ());
    }
}

impl PreFinalizationBlockCache {
    pub fn block_processed(&self, block_root: Hash256) {
        // Future requests will find this block in fork choice, so no need to cache it in the
        // ongoing lookup cache any longer.
        self.cache.lock().in_progress_lookups.pop(&block_root);
    }

    pub fn contains(&self, block_root: Hash256) -> bool {
        self.cache.lock().block_roots.contains(&block_root)
    }

    pub fn metrics(&self) -> Option<(usize, usize)> {
        let cache = self.cache.try_lock_for(METRICS_TIMEOUT)?;
        Some((cache.block_roots.len(), cache.in_progress_lookups.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::FixedBytesExtended;

    fn root(byte: u8) -> Hash256 {
        Hash256::repeat_byte(byte)
    }

    #[test]
    fn empty_cache_contains_nothing() {
        let cache = PreFinalizationBlockCache::default();
        assert!(!cache.contains(root(1)));
        assert!(!cache.contains(root(0)));
    }

    #[test]
    fn empty_cache_metrics() {
        let cache = PreFinalizationBlockCache::default();
        let (block_roots, lookups) = cache.metrics().unwrap();
        assert_eq!(block_roots, 0);
        assert_eq!(lookups, 0);
    }

    #[test]
    fn block_processed_removes_from_lookups() {
        let cache = PreFinalizationBlockCache::default();
        {
            let mut inner = cache.cache.lock();
            inner.in_progress_lookups.put(root(1), ());
        }
        let (_, lookups) = cache.metrics().unwrap();
        assert_eq!(lookups, 1);

        cache.block_processed(root(1));
        let (_, lookups) = cache.metrics().unwrap();
        assert_eq!(lookups, 0);
    }

    #[test]
    fn block_processed_noop_for_unknown_root() {
        let cache = PreFinalizationBlockCache::default();
        cache.block_processed(root(99));
        let (block_roots, lookups) = cache.metrics().unwrap();
        assert_eq!(block_roots, 0);
        assert_eq!(lookups, 0);
    }

    #[test]
    fn contains_reflects_block_roots_cache() {
        let cache = PreFinalizationBlockCache::default();
        {
            let mut inner = cache.cache.lock();
            inner.block_roots.put(root(1), ());
        }
        assert!(cache.contains(root(1)));
        assert!(!cache.contains(root(2)));
    }

    #[test]
    fn block_roots_lru_eviction() {
        let cache = PreFinalizationBlockCache::default();
        {
            let mut inner = cache.cache.lock();
            for i in 0..=BLOCK_ROOT_CACHE_LIMIT.get() {
                inner
                    .block_roots
                    .put(Hash256::from_low_u64_be(i as u64), ());
            }
        }
        // The first entry (0) should have been evicted
        assert!(!cache.contains(Hash256::from_low_u64_be(0)));
        // The last entry should still be present
        assert!(cache.contains(Hash256::from_low_u64_be(BLOCK_ROOT_CACHE_LIMIT.get() as u64)));
    }

    #[test]
    fn lookups_lru_eviction() {
        let cache = PreFinalizationBlockCache::default();
        {
            let mut inner = cache.cache.lock();
            for i in 0..=LOOKUP_LIMIT.get() {
                inner
                    .in_progress_lookups
                    .put(Hash256::from_low_u64_be(i as u64), ());
            }
        }
        let (_, lookups) = cache.metrics().unwrap();
        assert_eq!(lookups, LOOKUP_LIMIT.get());
    }

    #[test]
    fn metrics_returns_correct_counts() {
        let cache = PreFinalizationBlockCache::default();
        {
            let mut inner = cache.cache.lock();
            inner.block_roots.put(root(1), ());
            inner.block_roots.put(root(2), ());
            inner.block_roots.put(root(3), ());
            inner.in_progress_lookups.put(root(10), ());
            inner.in_progress_lookups.put(root(11), ());
        }
        let (block_roots, lookups) = cache.metrics().unwrap();
        assert_eq!(block_roots, 3);
        assert_eq!(lookups, 2);
    }

    #[test]
    fn block_processed_does_not_affect_block_roots() {
        let cache = PreFinalizationBlockCache::default();
        {
            let mut inner = cache.cache.lock();
            inner.block_roots.put(root(1), ());
            inner.in_progress_lookups.put(root(1), ());
        }
        cache.block_processed(root(1));
        assert!(cache.contains(root(1)));
        let (block_roots, lookups) = cache.metrics().unwrap();
        assert_eq!(block_roots, 1);
        assert_eq!(lookups, 0);
    }

    #[test]
    fn duplicate_block_root_insertions() {
        let cache = PreFinalizationBlockCache::default();
        {
            let mut inner = cache.cache.lock();
            inner.block_roots.put(root(1), ());
            inner.block_roots.put(root(1), ());
            inner.block_roots.put(root(1), ());
        }
        let (block_roots, _) = cache.metrics().unwrap();
        assert_eq!(block_roots, 1);
    }
}
