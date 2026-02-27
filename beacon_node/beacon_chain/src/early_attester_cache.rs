use crate::data_availability_checker::{AvailableBlock, AvailableBlockData};
use crate::{
    attester_cache::{CommitteeLengths, Error},
    metrics,
};
use parking_lot::RwLock;
use proto_array::Block as ProtoBlock;
use std::sync::Arc;
use types::*;

pub struct CacheItem<E: EthSpec> {
    /*
     * Values used to create attestations.
     */
    epoch: Epoch,
    committee_lengths: CommitteeLengths,
    beacon_block_root: Hash256,
    source: Checkpoint,
    target: Checkpoint,
    /*
     * Values used to make the block available.
     */
    block: Arc<SignedBeaconBlock<E>>,
    blobs: Option<BlobSidecarList<E>>,
    data_columns: Option<DataColumnSidecarList<E>>,
    proto_block: ProtoBlock,
}

/// Provides a single-item cache which allows for attesting to blocks before those blocks have
/// reached the database.
///
/// This cache stores enough information to allow Lighthouse to:
///
/// - Produce an attestation without using `chain.canonical_head`.
/// - Verify that a block root exists (i.e., will be imported in the future) during attestation
///   verification.
/// - Provide a block which can be sent to peers via RPC.
#[derive(Default)]
pub struct EarlyAttesterCache<E: EthSpec> {
    item: RwLock<Option<CacheItem<E>>>,
}

impl<E: EthSpec> EarlyAttesterCache<E> {
    /// Removes the cached item, meaning that all future calls to `Self::try_attest` will return
    /// `None` until a new cache item is added.
    pub fn clear(&self) {
        *self.item.write() = None
    }

    /// Updates the cache item, so that `Self::try_attest` with return `Some` when given suitable
    /// parameters.
    pub fn add_head_block(
        &self,
        beacon_block_root: Hash256,
        block: &AvailableBlock<E>,
        proto_block: ProtoBlock,
        state: &BeaconState<E>,
        spec: &ChainSpec,
    ) -> Result<(), Error> {
        let epoch = state.current_epoch();
        let committee_lengths = CommitteeLengths::new(state, spec)?;
        let source = state.current_justified_checkpoint();
        let target_slot = epoch.start_slot(E::slots_per_epoch());
        let target = Checkpoint {
            epoch,
            root: if state.slot() <= target_slot {
                beacon_block_root
            } else {
                *state.get_block_root(target_slot)?
            },
        };

        let (blobs, data_columns) = match block.data() {
            AvailableBlockData::NoData => (None, None),
            AvailableBlockData::Blobs(blobs) => (Some(blobs.clone()), None),
            AvailableBlockData::DataColumns(data_columns) => (None, Some(data_columns.clone())),
        };

        let item = CacheItem {
            epoch,
            committee_lengths,
            beacon_block_root,
            source,
            target,
            block: block.block_cloned(),
            blobs,
            data_columns,
            proto_block,
        };

        *self.item.write() = Some(item);

        Ok(())
    }

    /// Will return `Some(attestation)` if all the following conditions are met:
    ///
    /// - There is a cache `item` present.
    /// - If `request_slot` is in the same epoch as `item.epoch`.
    /// - If `request_index` does not exceed `item.committee_count`.
    pub fn try_attest(
        &self,
        request_slot: Slot,
        request_index: CommitteeIndex,
        spec: &ChainSpec,
    ) -> Result<Option<Attestation<E>>, Error> {
        let lock = self.item.read();
        let Some(item) = lock.as_ref() else {
            return Ok(None);
        };

        let request_epoch = request_slot.epoch(E::slots_per_epoch());
        if request_epoch != item.epoch {
            return Ok(None);
        }

        if request_slot < item.block.slot() {
            return Ok(None);
        }

        let committee_count = item
            .committee_lengths
            .get_committee_count_per_slot::<E>(spec)?;
        if request_index >= committee_count as u64 {
            return Ok(None);
        }

        let committee_len =
            item.committee_lengths
                .get_committee_length::<E>(request_slot, request_index, spec)?;

        // [Gloas/EIP-7732] For non-same-slot attestations (request_slot > block.slot),
        // the payload is considered present since the cached block has been fully imported
        // with its envelope. Same-slot attestations always have payload_present = false.
        let payload_present = spec.fork_name_at_slot::<E>(request_slot).gloas_enabled()
            && request_slot > item.block.slot()
            && item.proto_block.payload_revealed;

        let attestation = Attestation::empty_for_signing(
            request_index,
            committee_len,
            request_slot,
            item.beacon_block_root,
            item.source,
            item.target,
            spec,
            payload_present,
        )
        .map_err(Error::AttestationError)?;

        metrics::inc_counter(&metrics::BEACON_EARLY_ATTESTER_CACHE_HITS);

        Ok(Some(attestation))
    }

    /// Returns `true` if `block_root` matches the cached item.
    pub fn contains_block(&self, block_root: Hash256) -> bool {
        self.item
            .read()
            .as_ref()
            .is_some_and(|item| item.beacon_block_root == block_root)
    }

    /// Returns the block, if `block_root` matches the cached item.
    pub fn get_block(&self, block_root: Hash256) -> Option<Arc<SignedBeaconBlock<E>>> {
        self.item
            .read()
            .as_ref()
            .filter(|item| item.beacon_block_root == block_root)
            .map(|item| item.block.clone())
    }

    /// Returns the blobs, if `block_root` matches the cached item.
    pub fn get_blobs(&self, block_root: Hash256) -> Option<BlobSidecarList<E>> {
        self.item
            .read()
            .as_ref()
            .filter(|item| item.beacon_block_root == block_root)
            .and_then(|item| item.blobs.clone())
    }

    /// Returns the data columns, if `block_root` matches the cached item.
    pub fn get_data_columns(&self, block_root: Hash256) -> Option<DataColumnSidecarList<E>> {
        self.item
            .read()
            .as_ref()
            .filter(|item| item.beacon_block_root == block_root)
            .and_then(|item| item.data_columns.clone())
    }

    /// Returns the proto-array block, if `block_root` matches the cached item.
    pub fn get_proto_block(&self, block_root: Hash256) -> Option<ProtoBlock> {
        self.item
            .read()
            .as_ref()
            .filter(|item| item.beacon_block_root == block_root)
            .map(|item| item.proto_block.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attester_cache::CommitteeLengths;
    use fork_choice::ExecutionStatus;
    use types::{
        AttestationShufflingId, BeaconBlock, ChainSpec, ForkName, MinimalEthSpec, Signature,
    };

    type E = MinimalEthSpec;

    /// Create a ChainSpec where Gloas is active from genesis.
    fn gloas_spec() -> ChainSpec {
        ForkName::Gloas.make_genesis_spec(ChainSpec::minimal())
    }

    /// Create a ChainSpec where Fulu is the latest fork (Gloas not active).
    fn fulu_spec() -> ChainSpec {
        ForkName::Fulu.make_genesis_spec(ChainSpec::minimal())
    }

    /// Create a dummy ProtoBlock for testing.
    fn make_proto_block(slot: Slot, payload_revealed: bool) -> ProtoBlock {
        let shuffling_id = AttestationShufflingId {
            shuffling_epoch: Epoch::new(0),
            shuffling_decision_block: Hash256::zero(),
        };
        ProtoBlock {
            slot,
            root: Hash256::random(),
            parent_root: Some(Hash256::zero()),
            state_root: Hash256::zero(),
            target_root: Hash256::zero(),
            current_epoch_shuffling_id: shuffling_id.clone(),
            next_epoch_shuffling_id: shuffling_id,
            justified_checkpoint: Checkpoint::default(),
            finalized_checkpoint: Checkpoint::default(),
            execution_status: ExecutionStatus::irrelevant(),
            unrealized_justified_checkpoint: None,
            unrealized_finalized_checkpoint: None,
            builder_index: Some(0),
            payload_revealed,
            ptc_weight: 0,
            ptc_blob_data_available_weight: 0,
            payload_data_available: false,
            bid_block_hash: None,
            bid_parent_block_hash: None,
            proposer_index: 0,
            ptc_timely: false,
            envelope_received: false,
        }
    }

    /// Create a signed beacon block at the given slot.
    fn make_block_at_slot(slot: Slot, spec: &ChainSpec) -> Arc<SignedBeaconBlock<E>> {
        let mut block = BeaconBlock::<E>::empty(spec);
        *block.slot_mut() = slot;
        Arc::new(SignedBeaconBlock::from_block(block, Signature::empty()))
    }

    /// Insert a CacheItem directly into the cache (bypasses add_head_block).
    fn insert_item(
        cache: &EarlyAttesterCache<E>,
        block_slot: Slot,
        epoch: Epoch,
        spec: &ChainSpec,
        payload_revealed: bool,
    ) {
        let item = CacheItem {
            epoch,
            committee_lengths: CommitteeLengths::new_for_testing(epoch, 32),
            beacon_block_root: Hash256::random(),
            source: Checkpoint::default(),
            target: Checkpoint {
                epoch,
                root: Hash256::random(),
            },
            block: make_block_at_slot(block_slot, spec),
            blobs: None,
            data_columns: None,
            proto_block: make_proto_block(block_slot, payload_revealed),
        };
        *cache.item.write() = Some(item);
    }

    #[test]
    fn try_attest_returns_none_when_empty() {
        let cache = EarlyAttesterCache::<E>::default();
        let spec = gloas_spec();
        let result = cache.try_attest(Slot::new(1), 0, &spec).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn try_attest_returns_none_for_wrong_epoch() {
        let cache = EarlyAttesterCache::<E>::default();
        let spec = gloas_spec();
        // Cache item at epoch 0 (slot 0), request at epoch 1 (slot 8)
        insert_item(&cache, Slot::new(0), Epoch::new(0), &spec, true);
        let result = cache
            .try_attest(Slot::new(E::slots_per_epoch()), 0, &spec)
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn try_attest_returns_none_for_slot_before_block() {
        let cache = EarlyAttesterCache::<E>::default();
        let spec = gloas_spec();
        // Cache item at slot 3, request at slot 2 (same epoch but before block)
        insert_item(&cache, Slot::new(3), Epoch::new(0), &spec, true);
        let result = cache.try_attest(Slot::new(2), 0, &spec).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn gloas_same_slot_attestation_has_payload_present_false() {
        let cache = EarlyAttesterCache::<E>::default();
        let spec = gloas_spec();
        // Block at slot 2, payload revealed. Request at same slot 2.
        // Same-slot attestations should have payload_present = false.
        insert_item(&cache, Slot::new(2), Epoch::new(0), &spec, true);
        let att = cache.try_attest(Slot::new(2), 0, &spec).unwrap().unwrap();
        assert_eq!(
            att.data().index,
            0,
            "same-slot Gloas attestation should have index=0 (payload_present=false)"
        );
    }

    #[test]
    fn gloas_next_slot_payload_revealed_has_payload_present_true() {
        let cache = EarlyAttesterCache::<E>::default();
        let spec = gloas_spec();
        // Block at slot 2 with payload_revealed=true. Request at slot 3.
        // Non-same-slot with payload revealed → payload_present=true → index=1.
        insert_item(&cache, Slot::new(2), Epoch::new(0), &spec, true);
        let att = cache.try_attest(Slot::new(3), 0, &spec).unwrap().unwrap();
        assert_eq!(
            att.data().index,
            1,
            "Gloas next-slot attestation with payload_revealed should have index=1"
        );
    }

    #[test]
    fn gloas_next_slot_payload_not_revealed_has_payload_present_false() {
        let cache = EarlyAttesterCache::<E>::default();
        let spec = gloas_spec();
        // Block at slot 2 with payload_revealed=false. Request at slot 3.
        // Non-same-slot but payload NOT revealed → payload_present=false → index=0.
        insert_item(&cache, Slot::new(2), Epoch::new(0), &spec, false);
        let att = cache.try_attest(Slot::new(3), 0, &spec).unwrap().unwrap();
        assert_eq!(
            att.data().index,
            0,
            "Gloas next-slot attestation without payload_revealed should have index=0"
        );
    }

    #[test]
    fn pre_gloas_attestation_always_has_payload_present_false() {
        let cache = EarlyAttesterCache::<E>::default();
        let spec = fulu_spec();
        // Block at slot 2 with payload_revealed=true. Request at slot 3.
        // Pre-Gloas fork → payload_present is always false regardless of payload_revealed.
        insert_item(&cache, Slot::new(2), Epoch::new(0), &spec, true);
        let att = cache.try_attest(Slot::new(3), 0, &spec).unwrap().unwrap();
        assert_eq!(
            att.data().index,
            0,
            "pre-Gloas attestation should always have index=0"
        );
    }
}
