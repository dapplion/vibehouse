use crate::hdiff::HDiffBuffer;
use crate::{
    Error,
    metrics::{self, HOT_METRIC},
};
use lru::LruCache;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::num::NonZeroUsize;
use tracing::instrument;
use types::{BeaconState, ChainSpec, Epoch, EthSpec, Hash256, Slot};

/// Fraction of the LRU cache to leave intact during culling.
const CULL_EXEMPT_NUMERATOR: usize = 1;
const CULL_EXEMPT_DENOMINATOR: usize = 10;

/// States that are less than or equal to this many epochs old *could* become finalized and will not
/// be culled from the cache.
const EPOCH_FINALIZATION_LIMIT: u64 = 4;

#[derive(Debug)]
pub struct FinalizedState<E: EthSpec> {
    state_root: Hash256,
    state: BeaconState<E>,
}

/// Map from block_root -> slot -> state_root.
#[derive(Debug, Default)]
pub struct BlockMap {
    blocks: HashMap<Hash256, SlotMap>,
}

/// Map from slot -> state_root.
#[derive(Debug, Default)]
pub struct SlotMap {
    slots: BTreeMap<Slot, Hash256>,
}

#[derive(Debug)]
pub struct StateCache<E: EthSpec> {
    finalized_state: Option<FinalizedState<E>>,
    // Stores the tuple (state_root, state) as LruCache only returns the value on put and we need
    // the state_root
    states: LruCache<Hash256, (Hash256, BeaconState<E>)>,
    block_map: BlockMap,
    hdiff_buffers: HotHDiffBufferCache,
    max_epoch: Epoch,
    head_block_root: Hash256,
    headroom: NonZeroUsize,
}

/// Cache of hdiff buffers for hot states.
///
/// This cache only keeps buffers prior to the finalized state, which are required by the
/// hierarchical state diff scheme to construct newer unfinalized states.
///
/// The cache always retains the hdiff buffer for the most recent snapshot so that even if the
/// cache capacity is 1, this snapshot never needs to be loaded from disk.
#[derive(Debug)]
pub struct HotHDiffBufferCache {
    /// Cache of HDiffBuffers for states *prior* to the `finalized_state`.
    ///
    /// Maps state_root -> (slot, buffer).
    hdiff_buffers: LruCache<Hash256, (Slot, HDiffBuffer)>,
}

#[derive(Debug)]
pub enum PutStateOutcome {
    /// State is prior to the cache's finalized state (lower slot) and was cached as an HDiffBuffer.
    PreFinalizedHDiffBuffer,
    /// State is equal to the cache's finalized state and was not inserted.
    Finalized,
    /// State was already present in the cache.
    Duplicate,
    /// State is new to the cache and was inserted.
    ///
    /// Includes deleted states as a result of this insertion.
    New(Vec<Hash256>),
}

#[allow(clippy::len_without_is_empty)]
impl<E: EthSpec> StateCache<E> {
    pub fn new(
        state_capacity: NonZeroUsize,
        headroom: NonZeroUsize,
        hdiff_capacity: NonZeroUsize,
    ) -> Self {
        StateCache {
            finalized_state: None,
            states: LruCache::new(state_capacity),
            block_map: BlockMap::default(),
            hdiff_buffers: HotHDiffBufferCache::new(hdiff_capacity),
            max_epoch: Epoch::new(0),
            head_block_root: Hash256::ZERO,
            headroom,
        }
    }

    pub fn len(&self) -> usize {
        self.states.len()
    }

    pub fn capacity(&self) -> usize {
        self.states.cap().get()
    }

    pub fn num_hdiff_buffers(&self) -> usize {
        self.hdiff_buffers.len()
    }

    pub fn hdiff_buffer_mem_usage(&self) -> usize {
        self.hdiff_buffers.mem_usage()
    }

    pub fn update_finalized_state(
        &mut self,
        state_root: Hash256,
        block_root: Hash256,
        state: BeaconState<E>,
        pre_finalized_slots_to_retain: &[Slot],
    ) -> Result<(), Error> {
        if state.slot() % E::slots_per_epoch() != 0 {
            return Err(Error::FinalizedStateUnaligned);
        }

        if self
            .finalized_state
            .as_ref()
            .is_some_and(|finalized_state| state.slot() < finalized_state.state.slot())
        {
            return Err(Error::FinalizedStateDecreasingSlot);
        }

        // Add to block map.
        self.block_map.insert(block_root, state.slot(), state_root);

        // Prune block map.
        let state_roots_to_prune = self.block_map.prune(state.slot());

        // Prune HDiffBuffers that are no longer required by the hdiff grid of the finalized state.
        // We need to do this prior to copying in any new hdiff buffers, because the cache
        // preferences older slots.
        // NOTE: This isn't perfect as it prunes by slot: there could be multiple buffers
        // at some slots in the case of long forks without finality.
        let new_hdiff_cache = HotHDiffBufferCache::new(self.hdiff_buffers.cap());
        let old_hdiff_cache = std::mem::replace(&mut self.hdiff_buffers, new_hdiff_cache);
        for (state_root, (slot, buffer)) in old_hdiff_cache.hdiff_buffers {
            if pre_finalized_slots_to_retain.contains(&slot) {
                self.hdiff_buffers.put(state_root, slot, buffer);
            }
        }

        // Delete states.
        for state_root in state_roots_to_prune {
            if let Some((_, state)) = self.states.pop(&state_root) {
                // Add the hdiff buffer for this state to the hdiff cache if it is now part of
                // the pre-finalized grid. The `put` method will take care of keeping the most
                // useful buffers.
                let slot = state.slot();
                if pre_finalized_slots_to_retain.contains(&slot) {
                    let hdiff_buffer = HDiffBuffer::from_state(state);
                    self.hdiff_buffers.put(state_root, slot, hdiff_buffer);
                }
            }
        }

        // Update finalized state.
        self.finalized_state = Some(FinalizedState { state_root, state });
        Ok(())
    }

    /// Update the state cache's view of the enshrined head block.
    ///
    /// We never prune the unadvanced state for the head block.
    pub fn update_head_block_root(&mut self, head_block_root: Hash256) {
        self.head_block_root = head_block_root;
    }

    /// Rebase the given state on the finalized state in order to reduce its memory consumption.
    ///
    /// This function should only be called on states that are likely not to already share tree
    /// nodes with the finalized state, e.g. states loaded from disk.
    ///
    /// If the finalized state is not initialized this function is a no-op.
    pub fn rebase_on_finalized(
        &self,
        state: &mut BeaconState<E>,
        spec: &ChainSpec,
    ) -> Result<(), Error> {
        // Do not attempt to rebase states prior to the finalized state. This method might be called
        // with states on the hdiff grid prior to finalization, as part of the reconstruction of
        // some later unfinalized state.
        if let Some(finalized_state) = &self.finalized_state
            && state.slot() >= finalized_state.state.slot()
        {
            state.rebase_on(&finalized_state.state, spec)?;
        }

        Ok(())
    }

    /// Return a status indicating whether the state already existed in the cache.
    pub fn put_state(
        &mut self,
        state_root: Hash256,
        block_root: Hash256,
        state: &BeaconState<E>,
    ) -> Result<PutStateOutcome, Error> {
        if let Some(ref finalized_state) = self.finalized_state {
            if finalized_state.state_root == state_root {
                return Ok(PutStateOutcome::Finalized);
            } else if state.slot() <= finalized_state.state.slot() {
                // We assume any state being inserted into the cache is grid-aligned (it is the
                // caller's responsibility to not feed us garbage) as we don't want to thread the
                // hierarchy config through here. So any state received is converted to an
                // HDiffBuffer and saved.
                let hdiff_buffer = HDiffBuffer::from_state(state.clone());
                self.hdiff_buffers
                    .put(state_root, state.slot(), hdiff_buffer);
                return Ok(PutStateOutcome::PreFinalizedHDiffBuffer);
            }
        }

        if self.states.peek(&state_root).is_some() {
            return Ok(PutStateOutcome::Duplicate);
        }

        // Refuse states with pending mutations: we want cached states to be as small as possible
        // i.e. stored entirely as a binary merkle tree with no updates overlaid.
        if state.has_pending_mutations() {
            return Err(Error::StateForCacheHasPendingUpdates {
                state_root,
                slot: state.slot(),
            });
        }

        // Update the cache's idea of the max epoch.
        self.max_epoch = std::cmp::max(state.current_epoch(), self.max_epoch);

        // If the cache is full, use the custom cull routine to make room.
        let mut deleted_states =
            if let Some(over_capacity) = self.len().checked_sub(self.capacity()) {
                // The `over_capacity` should always be 0, but we add it here just in case.
                self.cull(over_capacity + self.headroom.get())
            } else {
                vec![]
            };

        // Insert the full state into the cache.
        if let Some((deleted_state_root, _)) =
            self.states.put(state_root, (state_root, state.clone()))
        {
            deleted_states.push(deleted_state_root);
        }

        // Record the connection from block root and slot to this state.
        let slot = state.slot();
        self.block_map.insert(block_root, slot, state_root);

        Ok(PutStateOutcome::New(deleted_states))
    }

    pub fn get_by_state_root(&mut self, state_root: Hash256) -> Option<BeaconState<E>> {
        if let Some(ref finalized_state) = self.finalized_state
            && state_root == finalized_state.state_root
        {
            return Some(finalized_state.state.clone());
        }
        self.states.get(&state_root).map(|(_, state)| state.clone())
    }

    pub fn put_hdiff_buffer(&mut self, state_root: Hash256, slot: Slot, buffer: &HDiffBuffer) {
        // Only accept HDiffBuffers prior to finalization. Later states should be stored as proper
        // states, not HDiffBuffers.
        if let Some(finalized_state) = &self.finalized_state
            && slot >= finalized_state.state.slot()
        {
            return;
        }
        self.hdiff_buffers.put(state_root, slot, buffer.clone());
    }

    pub fn get_hdiff_buffer_by_state_root(&mut self, state_root: Hash256) -> Option<HDiffBuffer> {
        if let Some(buffer) = self.hdiff_buffers.get(&state_root) {
            metrics::inc_counter_vec(&metrics::STORE_BEACON_HDIFF_BUFFER_CACHE_HIT, HOT_METRIC);
            let timer =
                metrics::start_timer_vec(&metrics::BEACON_HDIFF_BUFFER_CLONE_TIME, HOT_METRIC);
            let result = Some(buffer.clone());
            drop(timer);
            return result;
        }
        if let Some(buffer) = self
            .get_by_state_root(state_root)
            .map(HDiffBuffer::from_state)
        {
            metrics::inc_counter_vec(&metrics::STORE_BEACON_HDIFF_BUFFER_CACHE_HIT, HOT_METRIC);
            return Some(buffer);
        }
        metrics::inc_counter_vec(&metrics::STORE_BEACON_HDIFF_BUFFER_CACHE_MISS, HOT_METRIC);
        None
    }

    #[instrument(skip_all, fields(?block_root, %slot), level = "debug")]
    pub fn get_by_block_root(
        &mut self,
        block_root: Hash256,
        slot: Slot,
    ) -> Option<(Hash256, BeaconState<E>)> {
        let slot_map = self.block_map.blocks.get(&block_root)?;

        // Find the state at `slot`, or failing that the most recent ancestor.
        let state_root = slot_map
            .slots
            .iter()
            .rev()
            .find_map(|(ancestor_slot, state_root)| {
                (*ancestor_slot <= slot).then_some(*state_root)
            })?;

        let state = self.get_by_state_root(state_root)?;
        Some((state_root, state))
    }

    pub fn delete_state(&mut self, state_root: &Hash256) {
        self.states.pop(state_root);
        self.block_map.delete(state_root);
    }

    pub fn delete_block_states(&mut self, block_root: &Hash256) {
        if let Some(slot_map) = self.block_map.delete_block_states(block_root) {
            for state_root in slot_map.slots.values() {
                self.states.pop(state_root);
            }
        }
    }

    /// Cull approximately `count` states from the cache.
    ///
    /// States are culled LRU, with the following extra order imposed:
    ///
    /// - Advanced states.
    /// - Mid-epoch unadvanced states.
    /// - Epoch-boundary states that are too old to be finalized.
    /// - Epoch-boundary states that could be finalized.
    pub fn cull(&mut self, count: usize) -> Vec<Hash256> {
        let cull_exempt = std::cmp::max(
            1,
            self.len() * CULL_EXEMPT_NUMERATOR / CULL_EXEMPT_DENOMINATOR,
        );

        // Stage 1: gather states to cull.
        let mut advanced_state_roots = vec![];
        let mut mid_epoch_state_roots = vec![];
        let mut old_boundary_state_roots = vec![];
        let mut good_boundary_state_roots = vec![];

        // Skip the `cull_exempt` most-recently used, then reverse the iterator to start at
        // least-recently used states.
        for (&state_root, (_, state)) in self.states.iter().skip(cull_exempt).rev() {
            let is_advanced = state.slot() > state.latest_block_header().slot;
            let is_boundary = state.slot() % E::slots_per_epoch() == 0;
            let could_finalize =
                (self.max_epoch - state.current_epoch()) <= EPOCH_FINALIZATION_LIMIT;

            if is_boundary {
                if could_finalize {
                    good_boundary_state_roots.push(state_root);
                } else {
                    old_boundary_state_roots.push(state_root);
                }
            } else if is_advanced {
                advanced_state_roots.push(state_root);
            } else if state.get_latest_block_root(state_root) != self.head_block_root {
                // Never prune the head state
                mid_epoch_state_roots.push(state_root);
            }

            // Terminate early in the common case where we've already found enough junk to cull.
            if advanced_state_roots.len() == count {
                break;
            }
        }

        // Stage 2: delete.
        // This could probably be more efficient in how it interacts with the block map.
        let state_roots_to_delete = advanced_state_roots
            .into_iter()
            .chain(old_boundary_state_roots)
            .chain(mid_epoch_state_roots)
            .chain(good_boundary_state_roots)
            .take(count)
            .collect::<Vec<_>>();

        for state_root in &state_roots_to_delete {
            self.delete_state(state_root);
        }

        state_roots_to_delete
    }
}

impl BlockMap {
    fn insert(&mut self, block_root: Hash256, slot: Slot, state_root: Hash256) {
        let slot_map = self.blocks.entry(block_root).or_default();
        slot_map.slots.insert(slot, state_root);
    }

    fn prune(&mut self, finalized_slot: Slot) -> HashSet<Hash256> {
        let mut pruned_states = HashSet::new();

        self.blocks.retain(|_, slot_map| {
            slot_map.slots.retain(|slot, state_root| {
                let keep = *slot >= finalized_slot;
                if !keep {
                    pruned_states.insert(*state_root);
                }
                keep
            });

            !slot_map.slots.is_empty()
        });

        pruned_states
    }

    fn delete(&mut self, state_root_to_delete: &Hash256) {
        self.blocks.retain(|_, slot_map| {
            slot_map
                .slots
                .retain(|_, state_root| state_root != state_root_to_delete);
            !slot_map.slots.is_empty()
        });
    }

    fn delete_block_states(&mut self, block_root: &Hash256) -> Option<SlotMap> {
        self.blocks.remove(block_root)
    }
}

impl HotHDiffBufferCache {
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            hdiff_buffers: LruCache::new(capacity),
        }
    }

    pub fn get(&mut self, state_root: &Hash256) -> Option<HDiffBuffer> {
        self.hdiff_buffers
            .get(state_root)
            .map(|(_, buffer)| buffer.clone())
    }

    /// Put a value in the cache, making room for it if necessary.
    ///
    /// If the value was inserted then `true` is returned.
    pub fn put(&mut self, state_root: Hash256, slot: Slot, buffer: HDiffBuffer) -> bool {
        // If the cache is not full, simply insert the value.
        if self.hdiff_buffers.len() != self.hdiff_buffers.cap().get() {
            self.hdiff_buffers.put(state_root, (slot, buffer));
            return true;
        }

        // If the cache is full, it has room for this new entry if:
        //
        // - The capacity is greater than 1: we can retain the snapshot and the new entry, or
        // - The capacity is 1 and the slot of the new entry is older than the min_slot in the
        //   cache. This is a simplified way of retaining the snapshot in the cache. We don't need
        //   to worry about inserting/retaining states older than the snapshot because these are
        //   pruned on finalization and never reinserted.
        let Some(min_slot) = self.hdiff_buffers.iter().map(|(_, (slot, _))| *slot).min() else {
            // Unreachable: cache is full so should have >0 entries.
            return false;
        };

        if self.hdiff_buffers.cap().get() > 1 || slot < min_slot {
            // Remove LRU value. Cache is now at size `cap - 1`.
            let Some((removed_state_root, (removed_slot, removed_buffer))) =
                self.hdiff_buffers.pop_lru()
            else {
                // Unreachable: cache is full so should have at least one entry to pop.
                return false;
            };

            // Insert new value. Cache size is now at size `cap`.
            self.hdiff_buffers.put(state_root, (slot, buffer));

            // If the removed value had the min slot and we didn't intend to replace it (cap=1)
            // then we reinsert it.
            if removed_slot == min_slot && slot >= min_slot {
                self.hdiff_buffers
                    .put(removed_state_root, (removed_slot, removed_buffer));
            }
            true
        } else {
            // No room.
            false
        }
    }

    pub fn cap(&self) -> NonZeroUsize {
        self.hdiff_buffers.cap()
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.hdiff_buffers.len()
    }

    pub fn mem_usage(&self) -> usize {
        self.hdiff_buffers
            .iter()
            .map(|(_, (_, buffer))| buffer.size())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{ChainSpec, Eth1Data, FixedBytesExtended, MinimalEthSpec};

    type E = MinimalEthSpec;

    fn hash(n: u64) -> Hash256 {
        Hash256::from_low_u64_be(n)
    }

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    fn make_state(slot: u64) -> BeaconState<E> {
        let spec = ChainSpec::minimal();
        let mut state = BeaconState::new(0, Eth1Data::default(), &spec);
        *state.slot_mut() = Slot::new(slot);
        state
    }

    fn make_hdiff_buffer(slot: u64) -> HDiffBuffer {
        HDiffBuffer::from_state(make_state(slot))
    }

    // ── BlockMap tests ──

    #[test]
    fn block_map_insert_and_lookup() {
        let mut bm = BlockMap::default();
        bm.insert(hash(1), Slot::new(10), hash(100));
        assert_eq!(bm.blocks.len(), 1);
        let slot_map = bm.blocks.get(&hash(1)).unwrap();
        assert_eq!(*slot_map.slots.get(&Slot::new(10)).unwrap(), hash(100));
    }

    #[test]
    fn block_map_insert_multiple_slots_same_block() {
        let mut bm = BlockMap::default();
        bm.insert(hash(1), Slot::new(10), hash(100));
        bm.insert(hash(1), Slot::new(11), hash(101));
        assert_eq!(bm.blocks.len(), 1);
        let slot_map = bm.blocks.get(&hash(1)).unwrap();
        assert_eq!(slot_map.slots.len(), 2);
    }

    #[test]
    fn block_map_insert_different_blocks() {
        let mut bm = BlockMap::default();
        bm.insert(hash(1), Slot::new(10), hash(100));
        bm.insert(hash(2), Slot::new(20), hash(200));
        assert_eq!(bm.blocks.len(), 2);
    }

    #[test]
    fn block_map_prune_removes_old_slots() {
        let mut bm = BlockMap::default();
        bm.insert(hash(1), Slot::new(5), hash(50));
        bm.insert(hash(1), Slot::new(10), hash(100));
        bm.insert(hash(1), Slot::new(15), hash(150));

        let pruned = bm.prune(Slot::new(10));
        assert!(pruned.contains(&hash(50)));
        assert!(!pruned.contains(&hash(100)));
        assert!(!pruned.contains(&hash(150)));

        let slot_map = bm.blocks.get(&hash(1)).unwrap();
        assert_eq!(slot_map.slots.len(), 2);
    }

    #[test]
    fn block_map_prune_removes_empty_block_entries() {
        let mut bm = BlockMap::default();
        bm.insert(hash(1), Slot::new(5), hash(50));
        bm.insert(hash(2), Slot::new(15), hash(150));

        let pruned = bm.prune(Slot::new(10));
        assert!(pruned.contains(&hash(50)));
        assert!(!bm.blocks.contains_key(&hash(1)));
        assert!(bm.blocks.contains_key(&hash(2)));
    }

    #[test]
    fn block_map_prune_at_zero_keeps_all() {
        let mut bm = BlockMap::default();
        bm.insert(hash(1), Slot::new(0), hash(100));
        bm.insert(hash(1), Slot::new(5), hash(150));

        let pruned = bm.prune(Slot::new(0));
        assert!(pruned.is_empty());
        assert_eq!(bm.blocks.get(&hash(1)).unwrap().slots.len(), 2);
    }

    #[test]
    fn block_map_delete_state_root() {
        let mut bm = BlockMap::default();
        bm.insert(hash(1), Slot::new(10), hash(100));
        bm.insert(hash(1), Slot::new(11), hash(101));

        bm.delete(&hash(100));
        let slot_map = bm.blocks.get(&hash(1)).unwrap();
        assert_eq!(slot_map.slots.len(), 1);
        assert!(slot_map.slots.contains_key(&Slot::new(11)));
    }

    #[test]
    fn block_map_delete_removes_empty_block() {
        let mut bm = BlockMap::default();
        bm.insert(hash(1), Slot::new(10), hash(100));

        bm.delete(&hash(100));
        assert!(bm.blocks.is_empty());
    }

    #[test]
    fn block_map_delete_block_states() {
        let mut bm = BlockMap::default();
        bm.insert(hash(1), Slot::new(10), hash(100));
        bm.insert(hash(1), Slot::new(11), hash(101));

        let removed = bm.delete_block_states(&hash(1));
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().slots.len(), 2);
        assert!(bm.blocks.is_empty());
    }

    #[test]
    fn block_map_delete_block_states_missing() {
        let mut bm = BlockMap::default();
        assert!(bm.delete_block_states(&hash(99)).is_none());
    }

    // ── HotHDiffBufferCache tests ──

    #[test]
    fn hdiff_cache_new_empty() {
        let cache = HotHDiffBufferCache::new(nz(5));
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.cap(), nz(5));
        assert_eq!(cache.mem_usage(), 0);
    }

    #[test]
    fn hdiff_cache_put_and_get() {
        let mut cache = HotHDiffBufferCache::new(nz(5));
        let buf = make_hdiff_buffer(10);
        assert!(cache.put(hash(1), Slot::new(10), buf));
        assert_eq!(cache.len(), 1);
        assert!(cache.get(&hash(1)).is_some());
        assert!(cache.get(&hash(2)).is_none());
    }

    #[test]
    fn hdiff_cache_put_not_full_always_inserts() {
        let mut cache = HotHDiffBufferCache::new(nz(3));
        for i in 0..3 {
            assert!(cache.put(hash(i), Slot::new(i), make_hdiff_buffer(i)));
        }
        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn hdiff_cache_capacity_one_older_replaces() {
        let mut cache = HotHDiffBufferCache::new(nz(1));
        cache.put(hash(1), Slot::new(10), make_hdiff_buffer(10));
        // Inserting an older slot should succeed (replaces)
        assert!(cache.put(hash(2), Slot::new(5), make_hdiff_buffer(5)));
        assert_eq!(cache.len(), 1);
        // The older slot should now be in the cache
        assert!(cache.get(&hash(2)).is_some());
    }

    #[test]
    fn hdiff_cache_capacity_one_newer_rejected() {
        let mut cache = HotHDiffBufferCache::new(nz(1));
        cache.put(hash(1), Slot::new(10), make_hdiff_buffer(10));
        // Inserting a newer or equal slot at cap=1 should be rejected
        assert!(!cache.put(hash(2), Slot::new(15), make_hdiff_buffer(15)));
        assert_eq!(cache.len(), 1);
        assert!(cache.get(&hash(1)).is_some());
    }

    #[test]
    fn hdiff_cache_capacity_one_equal_slot_rejected() {
        let mut cache = HotHDiffBufferCache::new(nz(1));
        cache.put(hash(1), Slot::new(10), make_hdiff_buffer(10));
        assert!(!cache.put(hash(2), Slot::new(10), make_hdiff_buffer(10)));
        assert!(cache.get(&hash(1)).is_some());
    }

    #[test]
    fn hdiff_cache_capacity_gt_one_evicts_lru() {
        let mut cache = HotHDiffBufferCache::new(nz(2));
        cache.put(hash(1), Slot::new(10), make_hdiff_buffer(10));
        cache.put(hash(2), Slot::new(20), make_hdiff_buffer(20));
        // Cache full, inserting newer should evict LRU (hash(1))
        assert!(cache.put(hash(3), Slot::new(30), make_hdiff_buffer(30)));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn hdiff_cache_mem_usage_nonzero() {
        let mut cache = HotHDiffBufferCache::new(nz(3));
        cache.put(hash(1), Slot::new(10), make_hdiff_buffer(10));
        assert!(cache.mem_usage() > 0);
    }

    #[test]
    fn hdiff_cache_cap_one_reinserts_min_slot() {
        let mut cache = HotHDiffBufferCache::new(nz(2));
        // Insert two entries: slot 5 (older/snapshot) and slot 20
        cache.put(hash(1), Slot::new(5), make_hdiff_buffer(5));
        cache.put(hash(2), Slot::new(20), make_hdiff_buffer(20));
        // Access hash(1) to make hash(2) the LRU
        cache.get(&hash(1));
        // Insert newer slot 30 — should evict LRU (hash(2)), not the snapshot
        assert!(cache.put(hash(3), Slot::new(30), make_hdiff_buffer(30)));
        assert_eq!(cache.len(), 2);
    }

    // ── StateCache tests ──

    fn make_cache(capacity: usize, headroom: usize) -> StateCache<E> {
        StateCache::new(nz(capacity), nz(headroom), nz(2))
    }

    #[test]
    fn state_cache_new_empty() {
        let cache = make_cache(10, 1);
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.capacity(), 10);
        assert_eq!(cache.num_hdiff_buffers(), 0);
    }

    #[test]
    fn state_cache_put_and_get_by_state_root() {
        let mut cache = make_cache(10, 1);
        let state = make_state(8);
        let state_root = hash(1);
        let block_root = hash(10);

        let outcome = cache.put_state(state_root, block_root, &state).unwrap();
        assert!(matches!(outcome, PutStateOutcome::New(_)));
        assert_eq!(cache.len(), 1);

        let retrieved = cache.get_by_state_root(state_root);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().slot(), Slot::new(8));
    }

    #[test]
    fn state_cache_get_missing_returns_none() {
        let mut cache = make_cache(10, 1);
        assert!(cache.get_by_state_root(hash(99)).is_none());
    }

    #[test]
    fn state_cache_duplicate_returns_duplicate() {
        let mut cache = make_cache(10, 1);
        let state = make_state(8);
        let state_root = hash(1);
        let block_root = hash(10);

        cache.put_state(state_root, block_root, &state).unwrap();
        let outcome = cache.put_state(state_root, block_root, &state).unwrap();
        assert!(matches!(outcome, PutStateOutcome::Duplicate));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn state_cache_get_by_block_root_exact_slot() {
        let mut cache = make_cache(10, 1);
        let state = make_state(8);
        let state_root = hash(1);
        let block_root = hash(10);

        cache.put_state(state_root, block_root, &state).unwrap();
        let result = cache.get_by_block_root(block_root, Slot::new(8));
        assert!(result.is_some());
        let (sr, s) = result.unwrap();
        assert_eq!(sr, state_root);
        assert_eq!(s.slot(), Slot::new(8));
    }

    #[test]
    fn state_cache_get_by_block_root_ancestor_slot() {
        let mut cache = make_cache(10, 1);
        let state = make_state(8);
        let state_root = hash(1);
        let block_root = hash(10);

        cache.put_state(state_root, block_root, &state).unwrap();
        // Requesting a later slot should find the ancestor at slot 8
        let result = cache.get_by_block_root(block_root, Slot::new(12));
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, state_root);
    }

    #[test]
    fn state_cache_get_by_block_root_before_all_slots_returns_none() {
        let mut cache = make_cache(10, 1);
        let state = make_state(8);
        let state_root = hash(1);
        let block_root = hash(10);

        cache.put_state(state_root, block_root, &state).unwrap();
        // Requesting a slot before any stored slot
        let result = cache.get_by_block_root(block_root, Slot::new(5));
        assert!(result.is_none());
    }

    #[test]
    fn state_cache_get_by_block_root_missing_block() {
        let mut cache = make_cache(10, 1);
        assert!(cache.get_by_block_root(hash(99), Slot::new(0)).is_none());
    }

    #[test]
    fn state_cache_delete_state() {
        let mut cache = make_cache(10, 1);
        let state = make_state(8);
        let state_root = hash(1);
        let block_root = hash(10);

        cache.put_state(state_root, block_root, &state).unwrap();
        cache.delete_state(&state_root);
        assert_eq!(cache.len(), 0);
        assert!(cache.get_by_state_root(state_root).is_none());
    }

    #[test]
    fn state_cache_delete_block_states() {
        let mut cache = make_cache(10, 1);
        let block_root = hash(10);

        // Insert two states for the same block
        let state1 = make_state(8);
        let state2 = make_state(9);
        cache.put_state(hash(1), block_root, &state1).unwrap();
        cache.put_state(hash(2), block_root, &state2).unwrap();

        cache.delete_block_states(&block_root);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn state_cache_update_head_block_root() {
        let mut cache = make_cache(10, 1);
        cache.update_head_block_root(hash(42));
        assert_eq!(cache.head_block_root, hash(42));
    }

    #[test]
    fn state_cache_put_finalized_state_root_returns_finalized() {
        let mut cache = make_cache(10, 1);
        // Finalized state must be at epoch boundary (slot % 8 == 0 for minimal)
        let state = make_state(8);
        let state_root = hash(1);
        let block_root = hash(10);

        cache
            .update_finalized_state(state_root, block_root, state.clone(), &[])
            .unwrap();

        // Now try to put the same state_root
        let outcome = cache.put_state(state_root, block_root, &state).unwrap();
        assert!(matches!(outcome, PutStateOutcome::Finalized));
    }

    #[test]
    fn state_cache_get_finalized_by_state_root() {
        let mut cache = make_cache(10, 1);
        let state = make_state(8);
        let state_root = hash(1);
        let block_root = hash(10);

        cache
            .update_finalized_state(state_root, block_root, state, &[])
            .unwrap();

        let result = cache.get_by_state_root(state_root);
        assert!(result.is_some());
        assert_eq!(result.unwrap().slot(), Slot::new(8));
    }

    #[test]
    fn state_cache_update_finalized_unaligned_error() {
        let mut cache = make_cache(10, 1);
        // Slot 5 is not epoch-aligned (5 % 8 != 0)
        let state = make_state(5);

        let result = cache.update_finalized_state(hash(1), hash(10), state, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn state_cache_update_finalized_decreasing_slot_error() {
        let mut cache = make_cache(10, 1);
        let state16 = make_state(16);
        let state8 = make_state(8);

        cache
            .update_finalized_state(hash(1), hash(10), state16, &[])
            .unwrap();

        let result = cache.update_finalized_state(hash(2), hash(20), state8, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn state_cache_put_pre_finalized_becomes_hdiff_buffer() {
        let mut cache = make_cache(10, 1);
        // Finalize at slot 16
        let fin_state = make_state(16);
        cache
            .update_finalized_state(hash(1), hash(10), fin_state, &[])
            .unwrap();

        // Put state at slot 8 (before finalized)
        let pre_state = make_state(8);
        let outcome = cache.put_state(hash(2), hash(20), &pre_state).unwrap();
        assert!(matches!(outcome, PutStateOutcome::PreFinalizedHDiffBuffer));
        assert_eq!(cache.len(), 0); // Not in the state cache
        assert_eq!(cache.num_hdiff_buffers(), 1); // But in the hdiff cache
    }

    #[test]
    fn state_cache_put_hdiff_buffer_pre_finalized() {
        let mut cache = make_cache(10, 1);
        let fin_state = make_state(16);
        cache
            .update_finalized_state(hash(1), hash(10), fin_state, &[])
            .unwrap();

        let buf = make_hdiff_buffer(8);
        cache.put_hdiff_buffer(hash(2), Slot::new(8), &buf);
        assert_eq!(cache.num_hdiff_buffers(), 1);
    }

    #[test]
    fn state_cache_put_hdiff_buffer_post_finalized_rejected() {
        let mut cache = make_cache(10, 1);
        let fin_state = make_state(16);
        cache
            .update_finalized_state(hash(1), hash(10), fin_state, &[])
            .unwrap();

        let buf = make_hdiff_buffer(24);
        cache.put_hdiff_buffer(hash(2), Slot::new(24), &buf);
        assert_eq!(cache.num_hdiff_buffers(), 0);
    }

    #[test]
    fn state_cache_cull_respects_order() {
        // Create a cache with capacity 10, headroom 1
        let mut cache = make_cache(10, 1);

        // Insert states at various slots
        // Epoch boundary = slot % 8 == 0, so slots 0, 8, 16, 24...
        // Non-boundary (mid-epoch) = slot 1,2,3...7, 9,10...
        for i in 0u64..8 {
            let slot = i + 1; // slots 1-8 (mid-epoch except slot 8)
            let state = make_state(slot);
            cache
                .put_state(hash(i + 100), hash(i + 200), &state)
                .unwrap();
        }

        assert_eq!(cache.len(), 8);

        // Cull 3 states
        let deleted = cache.cull(3);
        assert_eq!(deleted.len(), 3);
        assert_eq!(cache.len(), 5);
    }

    #[test]
    fn state_cache_update_finalized_prunes_old_states() {
        let mut cache = make_cache(10, 1);

        // Put states at slots 8 and 16 (both epoch-boundary aligned)
        let state8 = make_state(8);
        let state16 = make_state(16);
        cache.put_state(hash(1), hash(10), &state8).unwrap();
        cache.put_state(hash(2), hash(20), &state16).unwrap();
        assert_eq!(cache.len(), 2);

        // Finalize at slot 16 — should prune slot 8 state
        let fin_state = make_state(16);
        cache
            .update_finalized_state(hash(3), hash(30), fin_state, &[])
            .unwrap();

        // State at slot 8 was pruned from the LRU cache
        assert!(cache.get_by_state_root(hash(1)).is_none());
    }

    #[test]
    fn state_cache_get_by_block_root_picks_most_recent_ancestor() {
        let mut cache = make_cache(10, 1);
        let block_root = hash(10);

        let state1 = make_state(1);
        let state5 = make_state(5);
        let state9 = make_state(9);
        cache.put_state(hash(1), block_root, &state1).unwrap();
        cache.put_state(hash(5), block_root, &state5).unwrap();
        cache.put_state(hash(9), block_root, &state9).unwrap();

        // Requesting slot 7 should find slot 5 (most recent <= 7)
        let result = cache.get_by_block_root(block_root, Slot::new(7));
        assert!(result.is_some());
        let (sr, s) = result.unwrap();
        assert_eq!(sr, hash(5));
        assert_eq!(s.slot(), Slot::new(5));
    }

    #[test]
    fn state_cache_rebase_on_finalized_noop_without_finalized() {
        let cache = make_cache(10, 1);
        let spec = ChainSpec::minimal();
        let mut state = make_state(8);
        // Should be a no-op when no finalized state
        assert!(cache.rebase_on_finalized(&mut state, &spec).is_ok());
    }

    #[test]
    fn state_cache_hdiff_buffer_mem_usage() {
        let cache = make_cache(10, 1);
        assert_eq!(cache.hdiff_buffer_mem_usage(), 0);
    }
}
