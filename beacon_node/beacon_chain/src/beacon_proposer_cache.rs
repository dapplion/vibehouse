//! The `BeaconProposer` cache stores the proposer indices for some epoch.
//!
//! This cache is keyed by `(epoch, block_root)` where `block_root` is the block root at
//! `end_slot(epoch - 1)`. We make the assertion that the proposer shuffling is identical for all
//! blocks in `epoch` which share the common ancestor of `block_root`.
//!
//! The cache is a fairly unintelligent LRU cache that is not pruned after finality. This makes it
//! very simple to reason about, but it might store values that are useless due to finalization. The
//! values it stores are very small, so this should not be an issue.

use crate::{BeaconChain, BeaconChainError, BeaconChainTypes};
use fork_choice::ExecutionStatus;
use lru::LruCache;
use once_cell::sync::OnceCell;
use safe_arith::SafeArith;
use smallvec::SmallVec;
use state_processing::state_advance::partial_state_advance;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tracing::instrument;
use types::non_zero_usize::new_non_zero_usize;
use types::{
    BeaconState, BeaconStateError, ChainSpec, Epoch, EthSpec, Fork, Hash256, Slot, Unsigned,
};

/// The number of sets of proposer indices that should be cached.
const CACHE_SIZE: NonZeroUsize = new_non_zero_usize(16);

/// This value is fairly unimportant, it's used to avoid heap allocations. The result of it being
/// incorrect is non-substantial from a consensus perspective (and probably also from a
/// performance perspective).
pub const TYPICAL_SLOTS_PER_EPOCH: usize = 32;

/// For some given slot, this contains the proposer index (`index`) and the `fork` that should be
/// used to verify their signature.
pub struct Proposer {
    pub index: usize,
    pub fork: Fork,
}

/// The list of proposers for some given `epoch`, alongside the `fork` that should be used to verify
/// their signatures.
pub struct EpochBlockProposers {
    /// The epoch to which the proposers pertain.
    pub(crate) epoch: Epoch,
    /// The fork that should be used to verify proposer signatures.
    pub(crate) fork: Fork,
    /// A list of length `T::EthSpec::slots_per_epoch()`, representing the proposers for each slot
    /// in that epoch.
    ///
    /// E.g., if `self.epoch == 1`, then `self.proposers[0]` contains the proposer for slot `32`.
    pub(crate) proposers: SmallVec<[usize; TYPICAL_SLOTS_PER_EPOCH]>,
}

impl EpochBlockProposers {
    pub fn new(epoch: Epoch, fork: Fork, proposers: Vec<usize>) -> Self {
        Self {
            epoch,
            fork,
            proposers: proposers.into(),
        }
    }

    pub fn get_slot<E: EthSpec>(&self, slot: Slot) -> Result<Proposer, BeaconChainError> {
        let epoch = slot.epoch(E::slots_per_epoch());
        if epoch == self.epoch {
            self.proposers
                .get(slot.as_usize() % E::SlotsPerEpoch::to_usize())
                .map(|&index| Proposer {
                    index,
                    fork: self.fork,
                })
                .ok_or(BeaconChainError::ProposerCacheOutOfBounds { slot, epoch })
        } else {
            Err(BeaconChainError::ProposerCacheWrongEpoch {
                request_epoch: epoch,
                cache_epoch: self.epoch,
            })
        }
    }
}

/// A cache to store the proposers for some epoch.
///
/// See the module-level documentation for more information.
pub struct BeaconProposerCache {
    cache: LruCache<(Epoch, Hash256), Arc<OnceCell<EpochBlockProposers>>>,
}

impl Default for BeaconProposerCache {
    fn default() -> Self {
        Self {
            cache: LruCache::new(CACHE_SIZE),
        }
    }
}

impl BeaconProposerCache {
    /// If it is cached, returns the proposer for the block at `slot` where the block has the
    /// ancestor block root of `shuffling_decision_block` at `end_slot(slot.epoch() - 1)`.
    pub fn get_slot<E: EthSpec>(
        &mut self,
        shuffling_decision_block: Hash256,
        slot: Slot,
    ) -> Option<Proposer> {
        let epoch = slot.epoch(E::slots_per_epoch());
        let key = (epoch, shuffling_decision_block);
        let cache = self.cache.get(&key)?.get()?;
        cache.get_slot::<E>(slot).ok()
    }

    /// As per `Self::get_slot`, but returns all proposers in all slots for the given `epoch`.
    ///
    /// The nth slot in the returned `SmallVec` will be equal to the nth slot in the given `epoch`.
    /// E.g., if `epoch == 1` then `smallvec[0]` refers to slot 32 (assuming `SLOTS_PER_EPOCH ==
    /// 32`).
    pub fn get_epoch<E: EthSpec>(
        &mut self,
        shuffling_decision_block: Hash256,
        epoch: Epoch,
    ) -> Option<&SmallVec<[usize; TYPICAL_SLOTS_PER_EPOCH]>> {
        let key = (epoch, shuffling_decision_block);
        self.cache
            .get(&key)
            .and_then(|cache_once_cell| cache_once_cell.get().map(|proposers| &proposers.proposers))
    }

    /// Returns the `OnceCell` for the given `(epoch, shuffling_decision_block)` key,
    /// inserting an empty one if it doesn't exist.
    ///
    /// The returned `OnceCell` allows the caller to initialise the value externally
    /// using `get_or_try_init`, enabling deferred computation without holding a mutable
    /// reference to the cache.
    pub fn get_or_insert_key(
        &mut self,
        epoch: Epoch,
        shuffling_decision_block: Hash256,
    ) -> Arc<OnceCell<EpochBlockProposers>> {
        let key = (epoch, shuffling_decision_block);
        self.cache
            .get_or_insert(key, || Arc::new(OnceCell::new()))
            .clone()
    }

    /// Insert the proposers into the cache.
    ///
    /// See `Self::get` for a description of `shuffling_decision_block`.
    ///
    /// The `fork` value must be valid to verify proposer signatures in `epoch`.
    pub fn insert(
        &mut self,
        epoch: Epoch,
        shuffling_decision_block: Hash256,
        proposers: Vec<usize>,
        fork: Fork,
    ) -> Result<(), BeaconStateError> {
        let key = (epoch, shuffling_decision_block);
        if !self.cache.contains(&key) {
            let epoch_proposers = EpochBlockProposers::new(epoch, fork, proposers);
            self.cache
                .put(key, Arc::new(OnceCell::with_value(epoch_proposers)));
        }

        Ok(())
    }
}

/// Compute the proposer duties using the head state without cache.
///
/// Return:
/// - Proposer indices.
/// - True dependent root.
/// - Legacy dependent root (last block of epoch `N - 1`).
/// - Head execution status.
/// - Fork at `request_epoch`.
pub fn compute_proposer_duties_from_head<T: BeaconChainTypes>(
    request_epoch: Epoch,
    chain: &BeaconChain<T>,
) -> Result<(Vec<usize>, Hash256, Hash256, ExecutionStatus, Fork), BeaconChainError> {
    // Atomically collect information about the head whilst holding the canonical head `Arc` as
    // short as possible.
    let (mut state, head_state_root, head_block_root) = {
        let head = chain.canonical_head.cached_head();
        // Take a copy of the head state.
        let head_state = head.snapshot.beacon_state.clone();
        let head_state_root = head.head_state_root();
        let head_block_root = head.head_block_root();
        (head_state, head_state_root, head_block_root)
    };

    let execution_status = chain
        .canonical_head
        .fork_choice_read_lock()
        .get_block_execution_status(&head_block_root)
        .ok_or(BeaconChainError::HeadMissingFromForkChoice(head_block_root))?;

    // Advance the state into the requested epoch.
    ensure_state_can_determine_proposers_for_epoch(
        &mut state,
        head_state_root,
        request_epoch,
        &chain.spec,
    )?;

    let indices = state
        .get_beacon_proposer_indices(request_epoch, &chain.spec)
        .map_err(BeaconChainError::from)?;

    let dependent_root = state
        .proposer_shuffling_decision_root_at_epoch(request_epoch, head_block_root, &chain.spec)
        .map_err(BeaconChainError::from)?;

    // This is only required because the V1 proposer duties endpoint spec wasn't updated for Fulu. We
    // can delete this once the V1 endpoint is deprecated at the Glamsterdam fork.
    let legacy_dependent_root = state
        .legacy_proposer_shuffling_decision_root_at_epoch(request_epoch, head_block_root)
        .map_err(BeaconChainError::from)?;

    // Use fork_at_epoch rather than the state's fork, because post-Fulu we may not have advanced
    // the state completely into the new epoch.
    let fork = chain.spec.fork_at_epoch(request_epoch);

    Ok((
        indices,
        dependent_root,
        legacy_dependent_root,
        execution_status,
        fork,
    ))
}

/// If required, advance `state` to the epoch required to determine proposer indices in `target_epoch`.
///
/// ## Details
///
/// - Returns an error if `state.current_epoch() > target_epoch`.
/// - No-op if `state.current_epoch() == target_epoch`.
/// - It must be the case that `state.canonical_root() == state_root`, but this function will not
///   check that.
#[instrument(skip_all, fields(?state_root, %target_epoch, state_slot = %state.slot()), level = "debug")]
pub fn ensure_state_can_determine_proposers_for_epoch<E: EthSpec>(
    state: &mut BeaconState<E>,
    state_root: Hash256,
    target_epoch: Epoch,
    spec: &ChainSpec,
) -> Result<(), BeaconChainError> {
    // The decision slot is the end of an epoch, so we add 1 to reach the first slot of the epoch
    // at which the shuffling is determined.
    let minimum_slot = spec
        .proposer_shuffling_decision_slot::<E>(target_epoch)
        .safe_add(1)?;
    let minimum_epoch = minimum_slot.epoch(E::slots_per_epoch());

    // Before and after Fulu, the oldest epoch reachable from a state at epoch N is epoch N itself,
    // i.e. we can never "look back".
    let maximum_epoch = target_epoch;

    if state.current_epoch() > maximum_epoch {
        Err(BeaconStateError::SlotOutOfBounds.into())
    } else if state.current_epoch() >= minimum_epoch {
        Ok(())
    } else {
        // State's current epoch is less than the minimum epoch.
        // Advance the state up to the minimum epoch.
        partial_state_advance(state, Some(state_root), minimum_slot, spec)
            .map_err(BeaconChainError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{FixedBytesExtended, MinimalEthSpec};

    type E = MinimalEthSpec;

    fn default_fork() -> Fork {
        Fork::default()
    }

    // --- EpochBlockProposers ---

    #[test]
    fn epoch_block_proposers_get_slot_correct_epoch() {
        let epoch = Epoch::new(1);
        let slots_per_epoch = E::slots_per_epoch();
        let proposers: Vec<usize> = (0..slots_per_epoch as usize).collect();
        let ebp = EpochBlockProposers::new(epoch, default_fork(), proposers);

        for i in 0..slots_per_epoch {
            let slot = Slot::new(slots_per_epoch + i);
            let proposer = ebp.get_slot::<E>(slot).unwrap();
            assert_eq!(proposer.index, i as usize);
        }
    }

    #[test]
    fn epoch_block_proposers_get_slot_wrong_epoch() {
        let epoch = Epoch::new(1);
        let proposers = vec![0; E::slots_per_epoch() as usize];
        let ebp = EpochBlockProposers::new(epoch, default_fork(), proposers);

        let result = ebp.get_slot::<E>(Slot::new(0));
        assert!(result.is_err());
    }

    #[test]
    fn epoch_block_proposers_preserves_fork() {
        let fork = Fork {
            previous_version: [1, 2, 3, 4],
            current_version: [5, 6, 7, 8],
            epoch: Epoch::new(10),
        };
        let proposers = vec![42; E::slots_per_epoch() as usize];
        let ebp = EpochBlockProposers::new(Epoch::new(0), fork, proposers);
        let proposer = ebp.get_slot::<E>(Slot::new(0)).unwrap();
        assert_eq!(proposer.fork, fork);
    }

    // --- BeaconProposerCache ---

    #[test]
    fn cache_default_returns_none() {
        let mut cache = BeaconProposerCache::default();
        assert!(cache.get_slot::<E>(Hash256::zero(), Slot::new(0)).is_none());
    }

    #[test]
    fn cache_insert_and_get_slot() {
        let mut cache = BeaconProposerCache::default();
        let root = Hash256::repeat_byte(0xaa);
        let epoch = Epoch::new(2);
        let slots_per_epoch = E::slots_per_epoch() as usize;
        let proposers: Vec<usize> = (100..100 + slots_per_epoch).collect();

        cache
            .insert(epoch, root, proposers.clone(), default_fork())
            .unwrap();

        for i in 0..slots_per_epoch {
            let slot = Slot::new(epoch.start_slot(E::slots_per_epoch()).as_u64() + i as u64);
            let proposer = cache.get_slot::<E>(root, slot).unwrap();
            assert_eq!(proposer.index, 100 + i);
        }
    }

    #[test]
    fn cache_get_slot_wrong_root_returns_none() {
        let mut cache = BeaconProposerCache::default();
        let root = Hash256::repeat_byte(0xaa);
        let proposers = vec![0; E::slots_per_epoch() as usize];
        cache
            .insert(Epoch::new(0), root, proposers, default_fork())
            .unwrap();

        assert!(
            cache
                .get_slot::<E>(Hash256::repeat_byte(0xbb), Slot::new(0))
                .is_none()
        );
    }

    #[test]
    fn cache_get_epoch_returns_all_proposers() {
        let mut cache = BeaconProposerCache::default();
        let root = Hash256::repeat_byte(0xcc);
        let epoch = Epoch::new(1);
        let proposers: Vec<usize> = vec![10, 20, 30, 40, 50, 60, 70, 80];

        cache
            .insert(epoch, root, proposers.clone(), default_fork())
            .unwrap();

        let result = cache.get_epoch::<E>(root, epoch).unwrap();
        assert_eq!(result.as_slice(), &proposers);
    }

    #[test]
    fn cache_get_epoch_wrong_epoch_returns_none() {
        let mut cache = BeaconProposerCache::default();
        let root = Hash256::repeat_byte(0xdd);
        let proposers = vec![0; E::slots_per_epoch() as usize];
        cache
            .insert(Epoch::new(5), root, proposers, default_fork())
            .unwrap();

        assert!(cache.get_epoch::<E>(root, Epoch::new(6)).is_none());
    }

    #[test]
    fn cache_insert_does_not_overwrite() {
        let mut cache = BeaconProposerCache::default();
        let root = Hash256::repeat_byte(0xee);
        let epoch = Epoch::new(3);
        let proposers_1 = vec![1; E::slots_per_epoch() as usize];
        let proposers_2 = vec![2; E::slots_per_epoch() as usize];

        cache
            .insert(epoch, root, proposers_1, default_fork())
            .unwrap();
        cache
            .insert(epoch, root, proposers_2, default_fork())
            .unwrap();

        // First insert wins
        let slot = Slot::new(epoch.start_slot(E::slots_per_epoch()).as_u64());
        let proposer = cache.get_slot::<E>(root, slot).unwrap();
        assert_eq!(proposer.index, 1);
    }

    #[test]
    fn cache_get_or_insert_key_returns_same_arc() {
        let mut cache = BeaconProposerCache::default();
        let root = Hash256::repeat_byte(0xff);
        let epoch = Epoch::new(7);

        let cell1 = cache.get_or_insert_key(epoch, root);
        let cell2 = cache.get_or_insert_key(epoch, root);
        assert!(Arc::ptr_eq(&cell1, &cell2));
    }

    #[test]
    fn cache_lru_eviction() {
        let mut cache = BeaconProposerCache::default();
        let proposers = vec![0; E::slots_per_epoch() as usize];

        // Insert CACHE_SIZE + 1 entries to trigger eviction
        for i in 0..=CACHE_SIZE.get() {
            let root = Hash256::repeat_byte(i as u8);
            cache
                .insert(
                    Epoch::new(i as u64),
                    root,
                    proposers.clone(),
                    default_fork(),
                )
                .unwrap();
        }

        // First entry should be evicted
        let first_root = Hash256::repeat_byte(0);
        assert!(cache.get_slot::<E>(first_root, Slot::new(0)).is_none());

        // Last entry should still be present
        let last_root = Hash256::repeat_byte(CACHE_SIZE.get() as u8);
        let last_epoch = Epoch::new(CACHE_SIZE.get() as u64);
        let last_slot = Slot::new(last_epoch.start_slot(E::slots_per_epoch()).as_u64());
        assert!(cache.get_slot::<E>(last_root, last_slot).is_some());
    }
}
