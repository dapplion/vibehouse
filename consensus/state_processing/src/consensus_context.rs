use crate::EpochCacheError;
use crate::common::{attesting_indices_base, attesting_indices_electra};
use crate::per_block_processing::errors::{AttestationInvalid, BlockOperationError};
use std::collections::{HashMap, hash_map::Entry};
use tree_hash::TreeHash;
use types::{
    AbstractExecPayload, AttestationRef, BeaconState, BeaconStateError, ChainSpec, Epoch, EthSpec,
    Hash256, IndexedAttestation, IndexedAttestationRef, SignedBeaconBlock, Slot,
};

#[derive(Debug, PartialEq, Clone)]
pub struct ConsensusContext<E: EthSpec> {
    /// Slot to act as an identifier/safeguard
    pub slot: Slot,
    /// Previous epoch of the `slot` precomputed for optimization purpose.
    pub previous_epoch: Epoch,
    /// Current epoch of the `slot` precomputed for optimization purpose.
    pub current_epoch: Epoch,
    /// Proposer index of the block at `slot`.
    pub proposer_index: Option<u64>,
    /// Block root of the block at `slot`.
    pub current_block_root: Option<Hash256>,
    /// Cache of indexed attestations constructed during block processing.
    pub indexed_attestations: HashMap<Hash256, IndexedAttestation<E>>,
}

#[derive(Debug, PartialEq, Clone)]
pub enum ContextError {
    BeaconState(BeaconStateError),
    EpochCache(EpochCacheError),
    SlotMismatch { slot: Slot, expected: Slot },
    EpochMismatch { epoch: Epoch, expected: Epoch },
}

impl From<BeaconStateError> for ContextError {
    fn from(e: BeaconStateError) -> Self {
        Self::BeaconState(e)
    }
}

impl From<EpochCacheError> for ContextError {
    fn from(e: EpochCacheError) -> Self {
        Self::EpochCache(e)
    }
}

impl<E: EthSpec> ConsensusContext<E> {
    pub fn new(slot: Slot) -> Self {
        let current_epoch = slot.epoch(E::slots_per_epoch());
        let previous_epoch = current_epoch.saturating_sub(1u64);
        Self {
            slot,
            previous_epoch,
            current_epoch,
            proposer_index: None,
            current_block_root: None,
            indexed_attestations: HashMap::new(),
        }
    }

    #[must_use]
    pub fn set_proposer_index(mut self, proposer_index: u64) -> Self {
        self.proposer_index = Some(proposer_index);
        self
    }

    /// Strict method for fetching the proposer index.
    ///
    /// Gets the proposer index for `self.slot` while ensuring that it matches `state.slot()`. This
    /// method should be used in block processing and almost everywhere the proposer index is
    /// required. If the slot check is too restrictive, see `get_proposer_index_from_epoch_state`.
    pub fn get_proposer_index(
        &mut self,
        state: &BeaconState<E>,
        spec: &ChainSpec,
    ) -> Result<u64, ContextError> {
        self.check_slot(state.slot())?;
        self.get_proposer_index_no_checks(state, spec)
    }

    /// More liberal method for fetching the proposer index.
    ///
    /// Fetches the proposer index for `self.slot` but does not require the state to be from an
    /// exactly matching slot (merely a matching epoch). This is useful in batch verification where
    /// we want to extract the proposer index from a single state for every slot in the epoch.
    pub fn get_proposer_index_from_epoch_state(
        &mut self,
        state: &BeaconState<E>,
        spec: &ChainSpec,
    ) -> Result<u64, ContextError> {
        self.check_epoch(state.current_epoch())?;
        self.get_proposer_index_no_checks(state, spec)
    }

    fn get_proposer_index_no_checks(
        &mut self,
        state: &BeaconState<E>,
        spec: &ChainSpec,
    ) -> Result<u64, ContextError> {
        if let Some(proposer_index) = self.proposer_index {
            return Ok(proposer_index);
        }

        let proposer_index = state.get_beacon_proposer_index(self.slot, spec)? as u64;
        self.proposer_index = Some(proposer_index);
        Ok(proposer_index)
    }

    #[must_use]
    pub fn set_current_block_root(mut self, block_root: Hash256) -> Self {
        self.current_block_root = Some(block_root);
        self
    }

    pub fn get_current_block_root<Payload: AbstractExecPayload<E>>(
        &mut self,
        block: &SignedBeaconBlock<E, Payload>,
    ) -> Result<Hash256, ContextError> {
        self.check_slot(block.slot())?;

        if let Some(current_block_root) = self.current_block_root {
            return Ok(current_block_root);
        }

        let current_block_root = block.message().tree_hash_root();
        self.current_block_root = Some(current_block_root);
        Ok(current_block_root)
    }

    fn check_slot(&self, slot: Slot) -> Result<(), ContextError> {
        if slot == self.slot {
            Ok(())
        } else {
            Err(ContextError::SlotMismatch {
                slot,
                expected: self.slot,
            })
        }
    }

    fn check_epoch(&self, epoch: Epoch) -> Result<(), ContextError> {
        let expected = self.slot.epoch(E::slots_per_epoch());
        if epoch == expected {
            Ok(())
        } else {
            Err(ContextError::EpochMismatch { epoch, expected })
        }
    }

    #[allow(unknown_lints)]
    #[allow(mismatched_lifetime_syntaxes)]
    pub fn get_indexed_attestation<'a>(
        &'a mut self,
        state: &BeaconState<E>,
        attestation: AttestationRef<'a, E>,
    ) -> Result<IndexedAttestationRef<'a, E>, BlockOperationError<AttestationInvalid>> {
        let key = attestation.tree_hash_root();
        match attestation {
            AttestationRef::Base(attn) => match self.indexed_attestations.entry(key) {
                Entry::Occupied(occupied) => Ok(occupied.into_mut()),
                Entry::Vacant(vacant) => {
                    let committee = state.get_beacon_committee(attn.data.slot, attn.data.index)?;
                    let indexed_attestation =
                        attesting_indices_base::get_indexed_attestation(committee.committee, attn)?;
                    Ok(vacant.insert(indexed_attestation))
                }
            },
            AttestationRef::Electra(attn) => match self.indexed_attestations.entry(key) {
                Entry::Occupied(occupied) => Ok(occupied.into_mut()),
                Entry::Vacant(vacant) => {
                    let indexed_attestation =
                        attesting_indices_electra::get_indexed_attestation_from_state(state, attn)?;
                    Ok(vacant.insert(indexed_attestation))
                }
            },
        }
        .map(|indexed_attestation| (*indexed_attestation).to_ref())
    }

    pub fn num_cached_indexed_attestations(&self) -> usize {
        self.indexed_attestations.len()
    }

    #[must_use]
    pub fn set_indexed_attestations(
        mut self,
        attestations: HashMap<Hash256, IndexedAttestation<E>>,
    ) -> Self {
        self.indexed_attestations = attestations;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{
        DEPOSIT_TREE_DEPTH, FixedBytesExtended, ForkName, MinimalEthSpec,
        test_utils::generate_deterministic_keypairs,
    };

    type E = MinimalEthSpec;

    fn make_spec() -> ChainSpec {
        ForkName::Gloas.make_genesis_spec(E::default_spec())
    }

    fn make_genesis_state(spec: &ChainSpec) -> BeaconState<E> {
        let keypairs = generate_deterministic_keypairs(8);
        let mut deposit_datas = Vec::with_capacity(8);
        for kp in &keypairs {
            let mut creds = [0u8; 32];
            creds[0] = spec.eth1_address_withdrawal_prefix_byte;
            creds[12..].copy_from_slice(&[0xAA; 20]);
            let withdrawal_credentials = Hash256::from_slice(&creds);

            let mut data = types::DepositData {
                pubkey: kp.pk.clone().into(),
                withdrawal_credentials,
                amount: spec.max_effective_balance,
                signature: types::Signature::empty().into(),
            };
            data.signature = data.create_signature(&kp.sk, spec);
            deposit_datas.push(data);
        }

        let deposit_tree_depth = DEPOSIT_TREE_DEPTH;
        let mut tree = crate::common::DepositDataTree::create(&[], 0, deposit_tree_depth);
        let mut deposits = Vec::with_capacity(8);
        for data in deposit_datas {
            tree.push_leaf(data.tree_hash_root())
                .expect("should push leaf");
            let (_leaf, proof_vec) = tree
                .generate_proof(deposits.len())
                .expect("should generate proof");
            let mut proof = types::FixedVector::from(vec![Hash256::zero(); deposit_tree_depth + 1]);
            for (i, node) in proof_vec.iter().enumerate() {
                proof[i] = *node;
            }
            deposits.push(types::Deposit { proof, data });
        }

        crate::initialize_beacon_state_from_eth1::<E>(
            Hash256::repeat_byte(0x42),
            2u64.pow(40),
            deposits,
            None,
            spec,
        )
        .expect("should initialize state")
    }

    #[test]
    fn new_computes_correct_epochs() {
        let slots_per_epoch = E::slots_per_epoch();

        // Slot 0 => epoch 0
        let ctx = ConsensusContext::<E>::new(Slot::new(0));
        assert_eq!(ctx.current_epoch, Epoch::new(0));
        assert_eq!(ctx.previous_epoch, Epoch::new(0)); // saturating_sub

        // First slot of epoch 1
        let slot = Slot::new(slots_per_epoch);
        let ctx = ConsensusContext::<E>::new(slot);
        assert_eq!(ctx.current_epoch, Epoch::new(1));
        assert_eq!(ctx.previous_epoch, Epoch::new(0));

        // Arbitrary slot in epoch 5
        let slot = Slot::new(5 * slots_per_epoch + 3);
        let ctx = ConsensusContext::<E>::new(slot);
        assert_eq!(ctx.current_epoch, Epoch::new(5));
        assert_eq!(ctx.previous_epoch, Epoch::new(4));
    }

    #[test]
    fn new_starts_with_empty_caches() {
        let ctx = ConsensusContext::<E>::new(Slot::new(10));
        assert_eq!(ctx.proposer_index, None);
        assert_eq!(ctx.current_block_root, None);
        assert_eq!(ctx.num_cached_indexed_attestations(), 0);
    }

    #[test]
    fn set_proposer_index_builder_pattern() {
        let ctx = ConsensusContext::<E>::new(Slot::new(0)).set_proposer_index(42);
        assert_eq!(ctx.proposer_index, Some(42));
    }

    #[test]
    fn set_current_block_root_builder_pattern() {
        let root = Hash256::repeat_byte(0xAB);
        let ctx = ConsensusContext::<E>::new(Slot::new(0)).set_current_block_root(root);
        assert_eq!(ctx.current_block_root, Some(root));
    }

    #[test]
    fn get_proposer_index_slot_mismatch() {
        let spec = make_spec();
        let state = make_genesis_state(&spec);
        // State is at slot 0, context at slot 5
        let mut ctx = ConsensusContext::<E>::new(Slot::new(5));
        let result = ctx.get_proposer_index(&state, &spec);
        assert_eq!(
            result,
            Err(ContextError::SlotMismatch {
                slot: Slot::new(0),
                expected: Slot::new(5),
            })
        );
    }

    #[test]
    fn get_proposer_index_matching_slot() {
        let spec = make_spec();
        let state = make_genesis_state(&spec);
        // State is at slot 0, context also at slot 0
        let mut ctx = ConsensusContext::<E>::new(Slot::new(0));
        let proposer = ctx.get_proposer_index(&state, &spec).unwrap();
        // Should cache
        assert_eq!(ctx.proposer_index, Some(proposer));
        // Second call returns same value from cache
        let proposer2 = ctx.get_proposer_index(&state, &spec).unwrap();
        assert_eq!(proposer, proposer2);
    }

    #[test]
    fn get_proposer_index_uses_preset_value() {
        let spec = make_spec();
        let state = make_genesis_state(&spec);
        let mut ctx = ConsensusContext::<E>::new(Slot::new(0)).set_proposer_index(99);
        // Returns the preset value without computing from state
        let proposer = ctx.get_proposer_index(&state, &spec).unwrap();
        assert_eq!(proposer, 99);
    }

    #[test]
    fn get_proposer_index_from_epoch_state_ok() {
        let spec = make_spec();
        let state = make_genesis_state(&spec);
        // State epoch 0, context slot 0 (epoch 0) — same epoch
        let mut ctx = ConsensusContext::<E>::new(Slot::new(0));
        let result = ctx.get_proposer_index_from_epoch_state(&state, &spec);
        assert!(result.is_ok());
    }

    #[test]
    fn get_proposer_index_from_epoch_state_epoch_mismatch() {
        let spec = make_spec();
        let state = make_genesis_state(&spec);
        // State epoch 0, context at epoch 2
        let slot_in_epoch_2 = Slot::new(2 * E::slots_per_epoch());
        let mut ctx = ConsensusContext::<E>::new(slot_in_epoch_2);
        let result = ctx.get_proposer_index_from_epoch_state(&state, &spec);
        assert_eq!(
            result,
            Err(ContextError::EpochMismatch {
                epoch: Epoch::new(0),
                expected: Epoch::new(2),
            })
        );
    }

    #[test]
    fn set_indexed_attestations_replaces_cache() {
        let mut map = HashMap::new();
        let root = Hash256::repeat_byte(0x01);
        let indexed = IndexedAttestation::Base(types::IndexedAttestationBase {
            attesting_indices: types::VariableList::empty(),
            data: types::AttestationData::default(),
            signature: types::AggregateSignature::empty(),
        });
        map.insert(root, indexed);

        let ctx = ConsensusContext::<E>::new(Slot::new(0)).set_indexed_attestations(map);
        assert_eq!(ctx.num_cached_indexed_attestations(), 1);
        assert!(ctx.indexed_attestations.contains_key(&root));
    }

    #[test]
    fn context_error_from_beacon_state_error() {
        let bse = BeaconStateError::InsufficientValidators;
        let ce: ContextError = bse.clone().into();
        assert_eq!(ce, ContextError::BeaconState(bse));
    }

    #[test]
    fn context_error_from_epoch_cache_error() {
        let ece = EpochCacheError::CacheNotInitialized;
        let ce: ContextError = ece.clone().into();
        assert_eq!(ce, ContextError::EpochCache(ece));
    }
}
