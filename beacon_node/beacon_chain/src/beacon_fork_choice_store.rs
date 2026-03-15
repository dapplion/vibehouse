//! Defines the `BeaconForkChoiceStore` which provides the persistent storage for the `ForkChoice`
//! struct.
//!
//! Additionally, the `BalancesCache` struct is defined; a cache designed to avoid database
//! reads when fork choice requires the validator balances of the justified state.

use crate::{BeaconSnapshot, metrics};
use educe::Educe;
use fork_choice::ForkChoiceStore;
use proto_array::JustifiedBalances;
use safe_arith::ArithError;
use ssz_derive::{Decode, Encode};
use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::sync::Arc;
use store::{Error as StoreError, HotColdDB, ItemStore};
use types::{
    AbstractExecPayload, BeaconBlockRef, BeaconState, BeaconStateError, Checkpoint, Epoch, EthSpec,
    FixedBytesExtended, Hash256, Slot,
};

#[derive(Debug)]
pub enum Error {
    FailedToReadBlock(StoreError),
    MissingBlock(Hash256),
    FailedToReadState(StoreError),
    MissingState(Hash256),
    BeaconStateError(BeaconStateError),
    UnalignedCheckpoint { block_slot: Slot, state_slot: Slot },
    Arith(ArithError),
}

impl From<BeaconStateError> for Error {
    fn from(e: BeaconStateError) -> Self {
        Error::BeaconStateError(e)
    }
}

impl From<ArithError> for Error {
    fn from(e: ArithError) -> Self {
        Error::Arith(e)
    }
}

/// The number of validator balance sets that are cached within `BalancesCache`.
const MAX_BALANCE_CACHE_SIZE: usize = 4;

#[derive(PartialEq, Clone, Debug, Encode, Decode)]
pub(crate) struct CacheItem {
    pub(crate) block_root: Hash256,
    pub(crate) epoch: Epoch,
    pub(crate) balances: Vec<u64>,
}

#[derive(PartialEq, Clone, Default, Debug, Encode, Decode)]
pub struct BalancesCache {
    pub(crate) items: Vec<CacheItem>,
}

impl BalancesCache {
    /// Inspect the given `state` and determine the root of the block at the first slot of
    /// `state.current_epoch`. If there is not already some entry for the given block root, then
    /// add the effective balances from the `state` to the cache.
    pub fn process_state<E: EthSpec>(
        &mut self,
        block_root: Hash256,
        state: &BeaconState<E>,
    ) -> Result<(), Error> {
        let epoch = state.current_epoch();
        let epoch_boundary_slot = epoch.start_slot(E::slots_per_epoch());
        let epoch_boundary_root = if epoch_boundary_slot == state.slot() {
            block_root
        } else {
            // This call remains sensible as long as `state.block_roots` is larger than a single
            // epoch.
            *state.get_block_root(epoch_boundary_slot)?
        };

        // Check if there already exists a cache entry for the epoch boundary block of the current
        // epoch. We rely on the invariant that effective balances do not change for the duration
        // of a single epoch, so even if the block on the epoch boundary itself is skipped we can
        // still update its cache entry from any subsequent state in that epoch.
        if self.position(epoch_boundary_root, epoch).is_none() {
            let item = CacheItem {
                block_root: epoch_boundary_root,
                epoch,
                balances: JustifiedBalances::from_justified_state(state)?.effective_balances,
            };

            if self.items.len() == MAX_BALANCE_CACHE_SIZE {
                self.items.remove(0);
            }

            self.items.push(item);
        }

        Ok(())
    }

    fn position(&self, block_root: Hash256, epoch: Epoch) -> Option<usize> {
        self.items
            .iter()
            .position(|item| item.block_root == block_root && item.epoch == epoch)
    }

    /// Get the balances for the given `block_root`, if any.
    ///
    /// If some balances are found, they are cloned from the cache.
    pub fn get(&mut self, block_root: Hash256, epoch: Epoch) -> Option<Vec<u64>> {
        let i = self.position(block_root, epoch)?;
        Some(self.items[i].balances.clone())
    }
}

/// Implements `fork_choice::ForkChoiceStore` in order to provide a persistent backing to the
/// `fork_choice::ForkChoice` struct.
#[derive(Debug, Educe)]
#[educe(PartialEq(bound(E: EthSpec, Hot: ItemStore<E>, Cold: ItemStore<E>)))]
pub struct BeaconForkChoiceStore<E: EthSpec, Hot: ItemStore<E>, Cold: ItemStore<E>> {
    #[educe(PartialEq(ignore))]
    store: Arc<HotColdDB<E, Hot, Cold>>,
    balances_cache: BalancesCache,
    time: Slot,
    finalized_checkpoint: Checkpoint,
    justified_checkpoint: Checkpoint,
    justified_balances: JustifiedBalances,
    justified_state_root: Hash256,
    unrealized_justified_checkpoint: Checkpoint,
    unrealized_justified_state_root: Hash256,
    unrealized_finalized_checkpoint: Checkpoint,
    proposer_boost_root: Hash256,
    equivocating_indices: BTreeSet<u64>,
    _phantom: PhantomData<E>,
}

impl<E, Hot, Cold> BeaconForkChoiceStore<E, Hot, Cold>
where
    E: EthSpec,
    Hot: ItemStore<E>,
    Cold: ItemStore<E>,
{
    /// Initialize `Self` from some `anchor` checkpoint which may or may not be the genesis state.
    ///
    /// ## Specification
    ///
    /// Equivalent to:
    ///
    /// <https://github.com/ethereum/eth2.0-specs/blob/v0.12.1/specs/phase0/fork-choice.md#get_forkchoice_store>
    ///
    /// ## Notes:
    ///
    /// It is assumed that `anchor` is already persisted in `store`.
    pub fn get_forkchoice_store(
        store: Arc<HotColdDB<E, Hot, Cold>>,
        anchor: BeaconSnapshot<E>,
    ) -> Result<Self, Error> {
        let unadvanced_state_root = anchor.beacon_state_root();
        let mut anchor_state = anchor.beacon_state;
        let mut anchor_block_header = *anchor_state.latest_block_header();

        // The anchor state MUST be on an epoch boundary (it should be advanced by the caller).
        if !anchor_state
            .slot()
            .as_u64()
            .is_multiple_of(E::slots_per_epoch())
        {
            return Err(Error::UnalignedCheckpoint {
                block_slot: anchor_block_header.slot,
                state_slot: anchor_state.slot(),
            });
        }

        // Compute the accurate block root for the checkpoint block.
        if anchor_block_header.state_root.is_zero() {
            anchor_block_header.state_root = unadvanced_state_root;
        }
        let anchor_block_root = anchor_block_header.canonical_root();
        let anchor_epoch = anchor_state.current_epoch();
        let justified_checkpoint = Checkpoint {
            epoch: anchor_epoch,
            root: anchor_block_root,
        };
        let finalized_checkpoint = justified_checkpoint;
        let justified_balances = JustifiedBalances::from_justified_state(&anchor_state)?;
        let justified_state_root = anchor_state.canonical_root()?;

        Ok(Self {
            store,
            balances_cache: <_>::default(),
            time: anchor_state.slot(),
            justified_checkpoint,
            justified_balances,
            justified_state_root,
            finalized_checkpoint,
            unrealized_justified_checkpoint: justified_checkpoint,
            unrealized_justified_state_root: justified_state_root,
            unrealized_finalized_checkpoint: finalized_checkpoint,
            proposer_boost_root: Hash256::zero(),
            equivocating_indices: BTreeSet::new(),
            _phantom: PhantomData,
        })
    }

    /// Save the current state of `Self` to a `PersistedForkChoiceStore` which can be stored to the
    /// on-disk database.
    pub fn to_persisted(&self) -> PersistedForkChoiceStore {
        PersistedForkChoiceStore {
            time: self.time,
            finalized_checkpoint: self.finalized_checkpoint,
            justified_checkpoint: self.justified_checkpoint,
            justified_state_root: self.justified_state_root,
            unrealized_justified_checkpoint: self.unrealized_justified_checkpoint,
            unrealized_justified_state_root: self.unrealized_justified_state_root,
            unrealized_finalized_checkpoint: self.unrealized_finalized_checkpoint,
            proposer_boost_root: self.proposer_boost_root,
            equivocating_indices: self.equivocating_indices.clone(),
        }
    }

    /// Restore `Self` from a previously-generated `PersistedForkChoiceStore`.
    pub fn from_persisted(
        persisted: PersistedForkChoiceStore,
        store: Arc<HotColdDB<E, Hot, Cold>>,
    ) -> Result<Self, Error> {
        let justified_checkpoint = persisted.justified_checkpoint;
        let justified_state_root = persisted.justified_state_root;

        let update_cache = true;
        let justified_state = store
            .get_hot_state(&justified_state_root, update_cache)
            .map_err(Error::FailedToReadState)?
            .ok_or(Error::MissingState(justified_state_root))?;

        let justified_balances = JustifiedBalances::from_justified_state(&justified_state)?;
        Ok(Self {
            store,
            balances_cache: <_>::default(),
            time: persisted.time,
            finalized_checkpoint: persisted.finalized_checkpoint,
            justified_checkpoint,
            justified_balances,
            justified_state_root,
            unrealized_justified_checkpoint: persisted.unrealized_justified_checkpoint,
            unrealized_justified_state_root: persisted.unrealized_justified_state_root,
            unrealized_finalized_checkpoint: persisted.unrealized_finalized_checkpoint,
            proposer_boost_root: persisted.proposer_boost_root,
            equivocating_indices: persisted.equivocating_indices,
            _phantom: PhantomData,
        })
    }
}

impl<E, Hot, Cold> ForkChoiceStore<E> for BeaconForkChoiceStore<E, Hot, Cold>
where
    E: EthSpec,
    Hot: ItemStore<E>,
    Cold: ItemStore<E>,
{
    type Error = Error;

    fn get_current_slot(&self) -> Slot {
        self.time
    }

    fn set_current_slot(&mut self, slot: Slot) {
        self.time = slot
    }

    fn on_verified_block<Payload: AbstractExecPayload<E>>(
        &mut self,
        _block: BeaconBlockRef<E, Payload>,
        block_root: Hash256,
        state: &BeaconState<E>,
    ) -> Result<(), Self::Error> {
        self.balances_cache.process_state(block_root, state)
    }

    fn justified_checkpoint(&self) -> &Checkpoint {
        &self.justified_checkpoint
    }

    fn justified_state_root(&self) -> Hash256 {
        self.justified_state_root
    }

    fn justified_balances(&self) -> &JustifiedBalances {
        &self.justified_balances
    }

    fn finalized_checkpoint(&self) -> &Checkpoint {
        &self.finalized_checkpoint
    }

    fn unrealized_justified_checkpoint(&self) -> &Checkpoint {
        &self.unrealized_justified_checkpoint
    }

    fn unrealized_justified_state_root(&self) -> Hash256 {
        self.unrealized_justified_state_root
    }

    fn unrealized_finalized_checkpoint(&self) -> &Checkpoint {
        &self.unrealized_finalized_checkpoint
    }

    fn proposer_boost_root(&self) -> Hash256 {
        self.proposer_boost_root
    }

    fn set_finalized_checkpoint(&mut self, checkpoint: Checkpoint) {
        self.finalized_checkpoint = checkpoint
    }

    fn set_justified_checkpoint(
        &mut self,
        checkpoint: Checkpoint,
        justified_state_root: Hash256,
    ) -> Result<(), Error> {
        self.justified_checkpoint = checkpoint;
        self.justified_state_root = justified_state_root;

        if let Some(balances) = self.balances_cache.get(
            self.justified_checkpoint.root,
            self.justified_checkpoint.epoch,
        ) {
            // NOTE: could avoid this re-calculation by introducing a `PersistedCacheItem`.
            metrics::inc_counter(&metrics::BALANCES_CACHE_HITS);
            self.justified_balances = JustifiedBalances::from_effective_balances(balances)?;
        } else {
            metrics::inc_counter(&metrics::BALANCES_CACHE_MISSES);

            // Justified state is reasonably useful to cache, it might be finalized soon.
            let update_cache = true;
            let state = self
                .store
                .get_hot_state(&self.justified_state_root, update_cache)
                .map_err(Error::FailedToReadState)?
                .ok_or(Error::MissingState(self.justified_state_root))?;

            self.justified_balances = JustifiedBalances::from_justified_state(&state)?;
        }

        Ok(())
    }

    fn set_unrealized_justified_checkpoint(&mut self, checkpoint: Checkpoint, state_root: Hash256) {
        self.unrealized_justified_checkpoint = checkpoint;
        self.unrealized_justified_state_root = state_root;
    }

    fn set_unrealized_finalized_checkpoint(&mut self, checkpoint: Checkpoint) {
        self.unrealized_finalized_checkpoint = checkpoint;
    }

    fn set_proposer_boost_root(&mut self, proposer_boost_root: Hash256) {
        self.proposer_boost_root = proposer_boost_root;
    }

    fn equivocating_indices(&self) -> &BTreeSet<u64> {
        &self.equivocating_indices
    }

    fn extend_equivocating_indices(&mut self, indices: impl IntoIterator<Item = u64>) {
        self.equivocating_indices.extend(indices);
    }
}

/// A container which allows persisting the `BeaconForkChoiceStore` to the on-disk database.
#[derive(Encode, Decode)]
pub struct PersistedForkChoiceStore {
    pub time: Slot,
    pub finalized_checkpoint: Checkpoint,
    pub justified_checkpoint: Checkpoint,
    pub justified_state_root: Hash256,
    pub unrealized_justified_checkpoint: Checkpoint,
    pub unrealized_justified_state_root: Hash256,
    pub unrealized_finalized_checkpoint: Checkpoint,
    pub proposer_boost_root: Hash256,
    pub equivocating_indices: BTreeSet<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssz::{Decode, Encode};

    fn hash(n: u64) -> Hash256 {
        Hash256::from_low_u64_be(n)
    }

    fn make_item(root_n: u64, epoch: u64, balances: Vec<u64>) -> CacheItem {
        CacheItem {
            block_root: hash(root_n),
            epoch: Epoch::new(epoch),
            balances,
        }
    }

    #[test]
    fn empty_cache_get_returns_none() {
        let mut cache = BalancesCache::default();
        assert!(cache.get(hash(1), Epoch::new(0)).is_none());
    }

    #[test]
    fn get_returns_matching_balances() {
        let mut cache = BalancesCache {
            items: vec![make_item(1, 5, vec![100, 200, 300])],
        };
        let result = cache.get(hash(1), Epoch::new(5));
        assert_eq!(result, Some(vec![100, 200, 300]));
    }

    #[test]
    fn get_returns_none_for_wrong_block_root() {
        let mut cache = BalancesCache {
            items: vec![make_item(1, 5, vec![100])],
        };
        assert!(cache.get(hash(2), Epoch::new(5)).is_none());
    }

    #[test]
    fn get_returns_none_for_wrong_epoch() {
        let mut cache = BalancesCache {
            items: vec![make_item(1, 5, vec![100])],
        };
        assert!(cache.get(hash(1), Epoch::new(6)).is_none());
    }

    #[test]
    fn get_requires_both_root_and_epoch_match() {
        let mut cache = BalancesCache {
            items: vec![
                make_item(1, 5, vec![10]),
                make_item(1, 6, vec![20]),
                make_item(2, 5, vec![30]),
            ],
        };
        assert_eq!(cache.get(hash(1), Epoch::new(5)), Some(vec![10]));
        assert_eq!(cache.get(hash(1), Epoch::new(6)), Some(vec![20]));
        assert_eq!(cache.get(hash(2), Epoch::new(5)), Some(vec![30]));
        assert!(cache.get(hash(2), Epoch::new(6)).is_none());
    }

    #[test]
    fn position_returns_correct_index() {
        let cache = BalancesCache {
            items: vec![
                make_item(1, 0, vec![]),
                make_item(2, 1, vec![]),
                make_item(3, 2, vec![]),
            ],
        };
        assert_eq!(cache.position(hash(1), Epoch::new(0)), Some(0));
        assert_eq!(cache.position(hash(2), Epoch::new(1)), Some(1));
        assert_eq!(cache.position(hash(3), Epoch::new(2)), Some(2));
        assert_eq!(cache.position(hash(4), Epoch::new(0)), None);
    }

    #[test]
    fn eviction_at_max_size() {
        // MAX_BALANCE_CACHE_SIZE is 4
        let mut cache = BalancesCache {
            items: vec![
                make_item(1, 0, vec![10]),
                make_item(2, 1, vec![20]),
                make_item(3, 2, vec![30]),
                make_item(4, 3, vec![40]),
            ],
        };
        assert_eq!(cache.items.len(), MAX_BALANCE_CACHE_SIZE);

        // Simulate adding a 5th item — process_state would evict the oldest (index 0)
        // We test the eviction logic directly
        if cache.items.len() == MAX_BALANCE_CACHE_SIZE {
            cache.items.remove(0);
        }
        cache.items.push(make_item(5, 4, vec![50]));

        assert_eq!(cache.items.len(), MAX_BALANCE_CACHE_SIZE);
        // First item (root=1, epoch=0) should be evicted
        assert!(cache.position(hash(1), Epoch::new(0)).is_none());
        // New item should be present
        assert!(cache.position(hash(5), Epoch::new(4)).is_some());
        // Others should still be present
        assert!(cache.position(hash(2), Epoch::new(1)).is_some());
        assert!(cache.position(hash(4), Epoch::new(3)).is_some());
    }

    #[test]
    fn no_duplicate_entries_same_root_and_epoch() {
        let mut cache = BalancesCache {
            items: vec![make_item(1, 5, vec![100])],
        };
        // Simulate process_state's dedup check: if position exists, don't insert
        if cache.position(hash(1), Epoch::new(5)).is_none() {
            cache.items.push(make_item(1, 5, vec![200]));
        }
        // Should still have only 1 item
        assert_eq!(cache.items.len(), 1);
        assert_eq!(cache.get(hash(1), Epoch::new(5)), Some(vec![100]));
    }

    #[test]
    fn same_root_different_epochs_are_separate_entries() {
        let mut cache = BalancesCache {
            items: vec![make_item(1, 5, vec![100]), make_item(1, 6, vec![200])],
        };
        assert_eq!(cache.items.len(), 2);
        assert_eq!(cache.get(hash(1), Epoch::new(5)), Some(vec![100]));
        assert_eq!(cache.get(hash(1), Epoch::new(6)), Some(vec![200]));
    }

    #[test]
    fn same_epoch_different_roots_are_separate_entries() {
        let mut cache = BalancesCache {
            items: vec![make_item(1, 5, vec![100]), make_item(2, 5, vec![200])],
        };
        assert_eq!(cache.items.len(), 2);
        assert_eq!(cache.get(hash(1), Epoch::new(5)), Some(vec![100]));
        assert_eq!(cache.get(hash(2), Epoch::new(5)), Some(vec![200]));
    }

    #[test]
    fn empty_balances_are_valid() {
        let mut cache = BalancesCache {
            items: vec![make_item(1, 0, vec![])],
        };
        assert_eq!(cache.get(hash(1), Epoch::new(0)), Some(vec![]));
    }

    #[test]
    fn get_clones_balances() {
        let mut cache = BalancesCache {
            items: vec![make_item(1, 0, vec![100, 200])],
        };
        let balances1 = cache.get(hash(1), Epoch::new(0)).unwrap();
        let balances2 = cache.get(hash(1), Epoch::new(0)).unwrap();
        assert_eq!(balances1, balances2);
        // Both should be independent clones
        assert_eq!(balances1, vec![100, 200]);
    }

    #[test]
    fn persisted_fork_choice_store_ssz_roundtrip() {
        let original = PersistedForkChoiceStore {
            time: Slot::new(42),
            finalized_checkpoint: Checkpoint {
                epoch: Epoch::new(1),
                root: hash(10),
            },
            justified_checkpoint: Checkpoint {
                epoch: Epoch::new(2),
                root: hash(20),
            },
            justified_state_root: hash(30),
            unrealized_justified_checkpoint: Checkpoint {
                epoch: Epoch::new(3),
                root: hash(40),
            },
            unrealized_justified_state_root: hash(50),
            unrealized_finalized_checkpoint: Checkpoint {
                epoch: Epoch::new(4),
                root: hash(60),
            },
            proposer_boost_root: hash(70),
            equivocating_indices: BTreeSet::from([1, 5, 10]),
        };

        let bytes = original.as_ssz_bytes();
        let decoded = PersistedForkChoiceStore::from_ssz_bytes(&bytes).unwrap();

        assert_eq!(original.time, decoded.time);
        assert_eq!(original.finalized_checkpoint, decoded.finalized_checkpoint);
        assert_eq!(original.justified_checkpoint, decoded.justified_checkpoint);
        assert_eq!(original.justified_state_root, decoded.justified_state_root);
        assert_eq!(
            original.unrealized_justified_checkpoint,
            decoded.unrealized_justified_checkpoint
        );
        assert_eq!(
            original.unrealized_justified_state_root,
            decoded.unrealized_justified_state_root
        );
        assert_eq!(
            original.unrealized_finalized_checkpoint,
            decoded.unrealized_finalized_checkpoint
        );
        assert_eq!(original.proposer_boost_root, decoded.proposer_boost_root);
        assert_eq!(original.equivocating_indices, decoded.equivocating_indices);
    }

    #[test]
    fn cache_item_ssz_roundtrip() {
        let item = CacheItem {
            block_root: hash(99),
            epoch: Epoch::new(7),
            balances: vec![1000, 2000, 3000, 4000],
        };

        let bytes = item.as_ssz_bytes();
        let decoded = CacheItem::from_ssz_bytes(&bytes).unwrap();

        assert_eq!(item.block_root, decoded.block_root);
        assert_eq!(item.epoch, decoded.epoch);
        assert_eq!(item.balances, decoded.balances);
    }

    #[test]
    fn balances_cache_ssz_roundtrip() {
        let cache = BalancesCache {
            items: vec![
                make_item(1, 0, vec![100, 200]),
                make_item(2, 1, vec![300]),
                make_item(3, 2, vec![]),
            ],
        };

        let bytes = cache.as_ssz_bytes();
        let decoded = BalancesCache::from_ssz_bytes(&bytes).unwrap();

        assert_eq!(cache, decoded);
    }
}
