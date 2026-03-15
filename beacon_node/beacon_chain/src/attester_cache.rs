//! This module provides the `AttesterCache`, a cache designed for reducing state-reads when
//! validators produce `AttestationData`.
//!
//! This cache is required *as well as* the `ShufflingCache` since the `ShufflingCache` does not
//! provide any information about the `state.current_justified_checkpoint`. It is not trivial to add
//! the justified checkpoint to the `ShufflingCache` since that cache is keyed by shuffling decision
//! root, which is not suitable for the justified checkpoint. Whilst we can know the shuffling for
//! epoch `n` during `n - 1`, we *cannot* know the justified checkpoint. Instead, we *must* perform
//! `per_epoch_processing` to transform the state from epoch `n - 1` to epoch `n` so that rewards
//! and penalties can be computed and the `state.current_justified_checkpoint` can be updated.

use crate::{BeaconChain, BeaconChainError, BeaconChainTypes};
use parking_lot::RwLock;
use state_processing::state_advance::{Error as StateAdvanceError, partial_state_advance};
use std::collections::HashMap;
use std::ops::Range;
use types::{
    BeaconState, BeaconStateError, ChainSpec, Checkpoint, Epoch, EthSpec, FixedBytesExtended,
    Hash256, RelativeEpoch, Slot,
    attestation::Error as AttestationError,
    beacon_state::{
        compute_committee_index_in_epoch, compute_committee_range_in_epoch, epoch_committee_count,
    },
};

type JustifiedCheckpoint = Checkpoint;
type CommitteeLength = usize;
type CommitteeIndex = u64;
type CacheHashMap = HashMap<AttesterCacheKey, AttesterCacheValue>;

/// The maximum number of `AttesterCacheValues` to be kept in memory.
///
/// Each `AttesterCacheValues` is very small (~16 bytes) and the cache will generally be kept small
/// by pruning on finality.
///
/// The value provided here is much larger than will be used during ideal network conditions,
/// however we make it large since the values are so small.
const MAX_CACHE_LEN: usize = 1_024;

#[derive(Debug)]
pub enum Error {
    BeaconState(BeaconStateError),
    // Boxed to avoid an infinite-size recursion issue.
    BeaconChain(Box<BeaconChainError>),
    MissingBeaconState(Hash256),
    FailedToTransitionState(StateAdvanceError),
    CannotAttestToFutureState {
        state_slot: Slot,
        request_slot: Slot,
    },
    /// Indicates a cache inconsistency.
    WrongEpoch {
        request_epoch: Epoch,
        epoch: Epoch,
    },
    InvalidCommitteeIndex {
        committee_index: u64,
    },
    /// Indicates an inconsistency with the beacon state committees.
    InverseRange {
        range: Range<usize>,
    },
    AttestationError(AttestationError),
}

impl From<BeaconStateError> for Error {
    fn from(e: BeaconStateError) -> Self {
        Error::BeaconState(e)
    }
}

impl From<BeaconChainError> for Error {
    fn from(e: BeaconChainError) -> Self {
        Error::BeaconChain(Box::new(e))
    }
}

/// Stores the minimal amount of data required to compute the committee length for any committee at any
/// slot in a given `epoch`.
pub struct CommitteeLengths {
    /// The `epoch` to which the lengths pertain.
    epoch: Epoch,
    /// The length of the shuffling in `self.epoch`.
    active_validator_indices_len: usize,
}

impl CommitteeLengths {
    /// Construct `CommitteeLengths` directly for unit tests (fields are private).
    #[cfg(test)]
    pub fn new_for_testing(epoch: Epoch, active_validator_indices_len: usize) -> Self {
        Self {
            epoch,
            active_validator_indices_len,
        }
    }

    /// Instantiate `Self` using `state.current_epoch()`.
    pub fn new<E: EthSpec>(state: &BeaconState<E>, spec: &ChainSpec) -> Result<Self, Error> {
        let active_validator_indices_len = if let Ok(committee_cache) =
            state.committee_cache(RelativeEpoch::Current)
        {
            committee_cache.active_validator_indices().len()
        } else {
            // Building the cache like this avoids taking a mutable reference to `BeaconState`.
            let committee_cache = state.initialize_committee_cache(state.current_epoch(), spec)?;
            committee_cache.active_validator_indices().len()
        };

        Ok(Self {
            epoch: state.current_epoch(),
            active_validator_indices_len,
        })
    }

    /// Get the count of committees per each slot of `self.epoch`.
    pub fn get_committee_count_per_slot<E: EthSpec>(
        &self,
        spec: &ChainSpec,
    ) -> Result<usize, Error> {
        E::get_committee_count_per_slot(self.active_validator_indices_len, spec).map_err(Into::into)
    }

    /// Get the length of the committee at the given `slot` and `committee_index`.
    pub fn get_committee_length<E: EthSpec>(
        &self,
        slot: Slot,
        committee_index: CommitteeIndex,
        spec: &ChainSpec,
    ) -> Result<CommitteeLength, Error> {
        let slots_per_epoch = E::slots_per_epoch();
        let request_epoch = slot.epoch(slots_per_epoch);

        // Sanity check.
        if request_epoch != self.epoch {
            return Err(Error::WrongEpoch {
                request_epoch,
                epoch: self.epoch,
            });
        }

        let slots_per_epoch = slots_per_epoch as usize;
        let committees_per_slot = self.get_committee_count_per_slot::<E>(spec)?;
        let index_in_epoch = compute_committee_index_in_epoch(
            slot,
            slots_per_epoch,
            committees_per_slot,
            committee_index as usize,
        );
        let range = compute_committee_range_in_epoch(
            epoch_committee_count(committees_per_slot, slots_per_epoch),
            index_in_epoch,
            self.active_validator_indices_len,
        )
        .ok_or(Error::InvalidCommitteeIndex { committee_index })?;

        range
            .end
            .checked_sub(range.start)
            .ok_or(Error::InverseRange { range })
    }
}

/// Provides the following information for some epoch:
///
/// - The `state.current_justified_checkpoint` value.
/// - The committee lengths for all indices and slots.
///
/// These values are used during attestation production.
pub struct AttesterCacheValue {
    current_justified_checkpoint: Checkpoint,
    committee_lengths: CommitteeLengths,
}

impl AttesterCacheValue {
    /// Instantiate `Self` using `state.current_epoch()`.
    pub fn new<E: EthSpec>(state: &BeaconState<E>, spec: &ChainSpec) -> Result<Self, Error> {
        let current_justified_checkpoint = state.current_justified_checkpoint();
        let committee_lengths = CommitteeLengths::new(state, spec)?;
        Ok(Self {
            current_justified_checkpoint,
            committee_lengths,
        })
    }

    /// Get the justified checkpoint and committee length for some `slot` and `committee_index`.
    fn get<E: EthSpec>(
        &self,
        slot: Slot,
        committee_index: CommitteeIndex,
        spec: &ChainSpec,
    ) -> Result<(JustifiedCheckpoint, CommitteeLength), Error> {
        self.committee_lengths
            .get_committee_length::<E>(slot, committee_index, spec)
            .map(|committee_length| (self.current_justified_checkpoint, committee_length))
    }
}

/// The `AttesterCacheKey` is fundamentally the same thing as the proposer shuffling decision root,
/// however here we use it as an identity for both of the following values:
///
/// 1. The `state.current_justified_checkpoint`.
/// 2. The attester shuffling.
///
/// This struct relies upon the premise that the `state.current_justified_checkpoint` in epoch `n`
/// is determined by the root of the latest block in epoch `n - 1`. Notably, this is identical to
/// how the proposer shuffling is keyed in `BeaconProposerCache`.
///
/// It is also safe, but not maximally efficient, to key the attester shuffling with the same
/// strategy. For better shuffling keying strategies, see the `ShufflingCache`.
#[derive(Debug, Eq, PartialEq, Hash, Clone, Copy)]
pub struct AttesterCacheKey {
    /// The epoch from which the justified checkpoint should be observed.
    ///
    /// Attestations which use `self.epoch` as `target.epoch` should use this key.
    epoch: Epoch,
    /// The root of the block at the last slot of `self.epoch - 1`.
    decision_root: Hash256,
}

impl AttesterCacheKey {
    /// Instantiate `Self` to key `state.current_epoch()`.
    ///
    /// The `latest_block_root` should be the latest block that has been applied to `state`. This
    /// parameter is required since the state does not store the block root for any block with the
    /// same slot as `state.slot()`.
    ///
    /// ## Errors
    ///
    /// May error if `epoch` is out of the range of `state.block_roots`.
    pub fn new<E: EthSpec>(
        epoch: Epoch,
        state: &BeaconState<E>,
        latest_block_root: Hash256,
    ) -> Result<Self, Error> {
        let slots_per_epoch = E::slots_per_epoch();
        let decision_slot = epoch.start_slot(slots_per_epoch).saturating_sub(1_u64);

        let decision_root = if decision_slot.epoch(slots_per_epoch) == epoch {
            // This scenario is only possible during the genesis epoch. In this scenario, all-zeros
            // is used as an alias to the genesis block.
            Hash256::zero()
        } else if epoch > state.current_epoch() {
            // If the requested epoch is higher than the current epoch, the latest block will always
            // be the decision root.
            latest_block_root
        } else {
            *state.get_block_root(decision_slot)?
        };

        Ok(Self {
            epoch,
            decision_root,
        })
    }
}

/// Provides a cache for the justified checkpoint and committee length when producing an
/// attestation.
///
/// See the module-level documentation for more information.
#[derive(Default)]
pub struct AttesterCache {
    cache: RwLock<CacheHashMap>,
}

impl AttesterCache {
    /// Get the justified checkpoint and committee length for the `slot` and `committee_index` in
    /// the state identified by the cache `key`.
    pub fn get<E: EthSpec>(
        &self,
        key: &AttesterCacheKey,
        slot: Slot,
        committee_index: CommitteeIndex,
        spec: &ChainSpec,
    ) -> Result<Option<(JustifiedCheckpoint, CommitteeLength)>, Error> {
        self.cache
            .read()
            .get(key)
            .map(|cache_item| cache_item.get::<E>(slot, committee_index, spec))
            .transpose()
    }

    /// Cache the `state.current_epoch()` values if they are not already present in the state.
    pub fn maybe_cache_state<E: EthSpec>(
        &self,
        state: &BeaconState<E>,
        latest_block_root: Hash256,
        spec: &ChainSpec,
    ) -> Result<(), Error> {
        let key = AttesterCacheKey::new(state.current_epoch(), state, latest_block_root)?;
        let mut cache = self.cache.write();
        if !cache.contains_key(&key) {
            let cache_item = AttesterCacheValue::new(state, spec)?;
            Self::insert_respecting_max_len(&mut cache, key, cache_item);
        }
        Ok(())
    }

    /// Read the state identified by `state_root` from the database, advance it to the required
    /// slot, use it to prime the cache and return the values for the provided `slot` and
    /// `committee_index`.
    ///
    /// ## Notes
    ///
    /// This function takes a write-lock on the internal cache. Prefer attempting a `Self::get` call
    /// before running this function as `Self::get` only takes a read-lock and is therefore less
    /// likely to create contention.
    pub fn load_and_cache_state<T: BeaconChainTypes>(
        &self,
        state_root: Hash256,
        key: AttesterCacheKey,
        slot: Slot,
        committee_index: CommitteeIndex,
        chain: &BeaconChain<T>,
    ) -> Result<(JustifiedCheckpoint, CommitteeLength), Error> {
        let spec = &chain.spec;
        let slots_per_epoch = T::EthSpec::slots_per_epoch();
        let epoch = slot.epoch(slots_per_epoch);

        // Take a write-lock on the cache before starting the state read.
        //
        // Whilst holding the write-lock during the state read will create contention, it prevents
        // the scenario where multiple requests from separate threads cause duplicate state reads.
        let mut cache = self.cache.write();

        // Try the cache to see if someone has already primed it between the time the function was
        // called and when the cache write-lock was obtained. This avoids performing duplicate state
        // reads.
        if let Some(value) = cache
            .get(&key)
            .map(|cache_item| cache_item.get::<T::EthSpec>(slot, committee_index, spec))
            .transpose()?
        {
            return Ok(value);
        }

        // We use `cache_state = true` here because if we are attesting to the state it's likely
        // to be recent and useful for other things.
        let mut state: BeaconState<T::EthSpec> = chain
            .get_state(&state_root, None, true)?
            .ok_or(Error::MissingBeaconState(state_root))?;

        if state.slot() > slot {
            // This indicates an internal inconsistency.
            return Err(Error::CannotAttestToFutureState {
                state_slot: state.slot(),
                request_slot: slot,
            });
        } else if state.current_epoch() < epoch {
            // Only perform a "partial" state advance since we do not require the state roots to be
            // accurate.
            partial_state_advance(
                &mut state,
                Some(state_root),
                epoch.start_slot(slots_per_epoch),
                spec,
            )
            .map_err(Error::FailedToTransitionState)?;
            state.build_committee_cache(RelativeEpoch::Current, spec)?;
        }

        let cache_item = AttesterCacheValue::new(&state, spec)?;
        let value = cache_item.get::<T::EthSpec>(slot, committee_index, spec)?;
        Self::insert_respecting_max_len(&mut cache, key, cache_item);
        Ok(value)
    }

    /// Insert a value to `cache`, ensuring it does not exceed the maximum length.
    ///
    /// If the cache is already full, the item with the lowest epoch will be removed.
    fn insert_respecting_max_len(
        cache: &mut CacheHashMap,
        key: AttesterCacheKey,
        value: AttesterCacheValue,
    ) {
        while cache.len() >= MAX_CACHE_LEN {
            if let Some(oldest) = cache.keys().copied().min_by_key(|key| key.epoch) {
                cache.remove(&oldest);
            } else {
                break;
            }
        }

        cache.insert(key, value);
    }

    /// Remove all entries where the `key.epoch` is lower than the given `epoch`.
    ///
    /// Generally, the provided `epoch` should be the finalized epoch.
    pub fn prune_below(&self, epoch: Epoch) {
        self.cache.write().retain(|target, _| target.epoch >= epoch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{FixedBytesExtended, MinimalEthSpec};

    type E = MinimalEthSpec;

    fn hash(n: u64) -> Hash256 {
        Hash256::from_low_u64_be(n)
    }

    fn spec() -> ChainSpec {
        E::default_spec()
    }

    // ── CommitteeLengths tests ──

    #[test]
    fn committee_lengths_get_committee_count_per_slot() {
        let spec = spec();
        // MinimalEthSpec: 8 slots/epoch, 4 max committees/slot
        // With 128 validators: committees_per_slot = max(1, 128 / 8 / target_committee_size)
        let cl = CommitteeLengths::new_for_testing(Epoch::new(0), 128);
        let count = cl.get_committee_count_per_slot::<E>(&spec).unwrap();
        assert!(count >= 1, "should have at least 1 committee per slot");
        assert!(
            count <= 4,
            "should not exceed MaxCommitteesPerSlot (4 for minimal)"
        );
    }

    #[test]
    fn committee_lengths_get_committee_length_slot_0() {
        let spec = spec();
        let cl = CommitteeLengths::new_for_testing(Epoch::new(0), 128);
        // Slot 0 is in epoch 0
        let len = cl
            .get_committee_length::<E>(Slot::new(0), 0, &spec)
            .unwrap();
        assert!(len > 0, "committee length should be positive");
        assert!(
            len <= 128,
            "committee length should not exceed total validators"
        );
    }

    #[test]
    fn committee_lengths_wrong_epoch_error() {
        let spec = spec();
        let cl = CommitteeLengths::new_for_testing(Epoch::new(0), 128);
        // Slot 8 is in epoch 1 (8 slots/epoch for minimal), but CommitteeLengths is for epoch 0
        let result = cl.get_committee_length::<E>(Slot::new(8), 0, &spec);
        assert!(
            matches!(result, Err(Error::WrongEpoch { .. })),
            "should return WrongEpoch error"
        );
    }

    #[test]
    fn committee_lengths_invalid_committee_index() {
        let spec = spec();
        let cl = CommitteeLengths::new_for_testing(Epoch::new(0), 128);
        // Committee index 99 should be way out of range
        let result = cl.get_committee_length::<E>(Slot::new(0), 99, &spec);
        assert!(
            matches!(result, Err(Error::InvalidCommitteeIndex { .. })),
            "should return InvalidCommitteeIndex error"
        );
    }

    #[test]
    fn committee_lengths_all_slots_in_epoch() {
        let spec = spec();
        let cl = CommitteeLengths::new_for_testing(Epoch::new(0), 128);
        let mut total = 0;
        let committees_per_slot = cl.get_committee_count_per_slot::<E>(&spec).unwrap();
        // Sum all committee lengths across all slots in the epoch
        for slot in 0..8u64 {
            for ci in 0..committees_per_slot as u64 {
                let len = cl
                    .get_committee_length::<E>(Slot::new(slot), ci, &spec)
                    .unwrap();
                assert!(len > 0);
                total += len;
            }
        }
        // Total should equal the number of active validators
        assert_eq!(
            total, 128,
            "sum of all committee lengths should equal active validator count"
        );
    }

    #[test]
    fn committee_lengths_single_validator() {
        let spec = spec();
        // Edge case: only 1 active validator
        let cl = CommitteeLengths::new_for_testing(Epoch::new(0), 1);
        let committees_per_slot = cl.get_committee_count_per_slot::<E>(&spec).unwrap();
        assert_eq!(committees_per_slot, 1, "should have 1 committee per slot");
        // Only one committee across all 8 slots should have the validator
        let mut total = 0;
        for slot in 0..8u64 {
            let len = cl
                .get_committee_length::<E>(Slot::new(slot), 0, &spec)
                .unwrap();
            total += len;
        }
        assert_eq!(total, 1);
    }

    #[test]
    fn committee_lengths_epoch_1() {
        let spec = spec();
        let cl = CommitteeLengths::new_for_testing(Epoch::new(1), 64);
        // Slot 8 is the first slot of epoch 1
        let len = cl
            .get_committee_length::<E>(Slot::new(8), 0, &spec)
            .unwrap();
        assert!(len > 0);
        // Slot 7 (last slot of epoch 0) should fail
        let result = cl.get_committee_length::<E>(Slot::new(7), 0, &spec);
        assert!(matches!(result, Err(Error::WrongEpoch { .. })));
    }

    // ── AttesterCacheValue tests ──

    #[test]
    fn attester_cache_value_get() {
        let spec = spec();
        let cp = Checkpoint {
            epoch: Epoch::new(0),
            root: hash(42),
        };
        let cl = CommitteeLengths::new_for_testing(Epoch::new(0), 128);
        let value = AttesterCacheValue {
            current_justified_checkpoint: cp,
            committee_lengths: cl,
        };
        let (justified, length) = value.get::<E>(Slot::new(0), 0, &spec).unwrap();
        assert_eq!(justified, cp);
        assert!(length > 0);
    }

    // ── AttesterCache tests ──

    fn make_key(epoch: u64, root: u64) -> AttesterCacheKey {
        AttesterCacheKey {
            epoch: Epoch::new(epoch),
            decision_root: hash(root),
        }
    }

    fn make_value(epoch: u64, active_validators: usize) -> AttesterCacheValue {
        AttesterCacheValue {
            current_justified_checkpoint: Checkpoint {
                epoch: Epoch::new(epoch),
                root: hash(epoch),
            },
            committee_lengths: CommitteeLengths::new_for_testing(
                Epoch::new(epoch),
                active_validators,
            ),
        }
    }

    #[test]
    fn cache_get_empty_returns_none() {
        let cache = AttesterCache::default();
        let key = make_key(0, 1);
        let spec = spec();
        let result = cache.get::<E>(&key, Slot::new(0), 0, &spec).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn cache_insert_and_get() {
        let cache = AttesterCache::default();
        let key = make_key(0, 1);
        let value = make_value(0, 128);
        let spec = spec();

        cache.cache.write().insert(key, value);

        let result = cache.get::<E>(&key, Slot::new(0), 0, &spec).unwrap();
        assert!(result.is_some());
        let (cp, length) = result.unwrap();
        assert_eq!(cp.epoch, Epoch::new(0));
        assert!(length > 0);
    }

    #[test]
    fn cache_prune_below_removes_old_entries() {
        let cache = AttesterCache::default();
        let spec = spec();

        // Insert entries for epochs 0, 1, 2, 3
        for epoch in 0..4u64 {
            let key = make_key(epoch, epoch + 10);
            let value = make_value(epoch, 128);
            cache.cache.write().insert(key, value);
        }
        assert_eq!(cache.cache.read().len(), 4);

        // Prune below epoch 2 — should remove epochs 0 and 1
        cache.prune_below(Epoch::new(2));
        assert_eq!(cache.cache.read().len(), 2);

        // Epochs 0 and 1 should be gone
        let result = cache
            .get::<E>(&make_key(0, 10), Slot::new(0), 0, &spec)
            .unwrap();
        assert!(result.is_none());
        let result = cache
            .get::<E>(&make_key(1, 11), Slot::new(8), 0, &spec)
            .unwrap();
        assert!(result.is_none());

        // Epochs 2 and 3 should still be present
        let result = cache
            .get::<E>(&make_key(2, 12), Slot::new(16), 0, &spec)
            .unwrap();
        assert!(result.is_some());
        let result = cache
            .get::<E>(&make_key(3, 13), Slot::new(24), 0, &spec)
            .unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn cache_prune_below_zero_keeps_all() {
        let cache = AttesterCache::default();
        for epoch in 0..3u64 {
            cache
                .cache
                .write()
                .insert(make_key(epoch, epoch), make_value(epoch, 64));
        }
        cache.prune_below(Epoch::new(0));
        assert_eq!(cache.cache.read().len(), 3);
    }

    #[test]
    fn cache_insert_respecting_max_len() {
        let mut map = CacheHashMap::new();

        // Fill to MAX_CACHE_LEN
        for i in 0..MAX_CACHE_LEN as u64 {
            AttesterCache::insert_respecting_max_len(&mut map, make_key(i, i), make_value(i, 64));
        }
        assert_eq!(map.len(), MAX_CACHE_LEN);

        // Insert one more — should evict the entry with the lowest epoch
        AttesterCache::insert_respecting_max_len(
            &mut map,
            make_key(MAX_CACHE_LEN as u64, 9999),
            make_value(MAX_CACHE_LEN as u64, 64),
        );
        assert_eq!(map.len(), MAX_CACHE_LEN);

        // Epoch 0 should have been evicted
        assert!(!map.contains_key(&make_key(0, 0)));
        // The new entry should be present
        assert!(map.contains_key(&make_key(MAX_CACHE_LEN as u64, 9999)));
    }

    #[test]
    fn cache_insert_when_not_full() {
        let mut map = CacheHashMap::new();
        AttesterCache::insert_respecting_max_len(&mut map, make_key(5, 5), make_value(5, 64));
        assert_eq!(map.len(), 1);
        assert!(map.contains_key(&make_key(5, 5)));
    }

    #[test]
    fn cache_key_equality() {
        let key1 = make_key(10, 42);
        let key2 = make_key(10, 42);
        let key3 = make_key(10, 43);
        let key4 = make_key(11, 42);

        assert_eq!(key1, key2, "same epoch and root should be equal");
        assert_ne!(key1, key3, "different root should differ");
        assert_ne!(key1, key4, "different epoch should differ");
    }
}
