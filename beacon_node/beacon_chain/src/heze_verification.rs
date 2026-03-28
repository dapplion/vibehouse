//! Provides verification for Heze FOCIL (EIP-7805) messages:
//!
//! - `SignedInclusionList` - Inclusion list submissions from committee members
//!
//! These messages are received via gossip and must be validated before:
//! 1. Propagation to other peers
//! 2. Import to the InclusionListStore

use crate::{BeaconChain, BeaconChainError, BeaconChainTypes};
use slot_clock::SlotClock;
use state_processing::per_block_processing::heze::{
    get_inclusion_list_committee, is_valid_inclusion_list_signature,
};
use strum::AsRefStr;
use tree_hash::TreeHash;
use types::{EthSpec, Hash256, SignedInclusionList, Slot, Unsigned};

/// Returned when an inclusion list was not successfully verified.
#[derive(Debug, AsRefStr)]
pub enum InclusionListError {
    /// The inclusion list slot is not the current slot.
    ///
    /// Spec: `[IGNORE] inclusion_list.slot is the current slot.`
    SlotNotCurrent { il_slot: Slot, current_slot: Slot },
    /// The fork is not Heze or later.
    ///
    /// Spec: `[REJECT] The inclusion_list is from a Heze (or later) fork.`
    PreHezeFork { il_slot: Slot },
    /// The validator is not a member of the inclusion list committee for this slot.
    ///
    /// Spec: `[REJECT] inclusion_list.validator_index is in the inclusion list committee.`
    NotInCommittee { validator_index: u64, slot: Slot },
    /// The inclusion_list_committee_root does not match the computed committee root.
    ///
    /// Spec: `[REJECT] inclusion_list.inclusion_list_committee_root matches.`
    CommitteeRootMismatch {
        expected: Hash256,
        received: Hash256,
    },
    /// The inclusion list signature is invalid.
    ///
    /// Spec: `[REJECT] The signature is valid.`
    InvalidSignature,
    /// This is a duplicate inclusion list (same content from same validator).
    ///
    /// Spec: `[IGNORE] This is the first valid inclusion list from this validator for this slot.`
    Duplicate { validator_index: u64, slot: Slot },
    /// This validator has already been marked as an equivocator.
    ///
    /// The InclusionListStore handles equivocation detection internally.
    Equivocator { validator_index: u64, slot: Slot },
    /// A beacon chain error occurred during verification.
    BeaconChainError(BeaconChainError),
}

impl From<BeaconChainError> for InclusionListError {
    fn from(e: BeaconChainError) -> Self {
        InclusionListError::BeaconChainError(e)
    }
}

/// A verified inclusion list ready for import into the InclusionListStore.
pub struct VerifiedInclusionList<T: BeaconChainTypes> {
    pub signed_il: SignedInclusionList<T::EthSpec>,
    /// Whether this IL was received before the view freeze cutoff (75% of slot).
    pub is_before_view_freeze_cutoff: bool,
}

impl<T: BeaconChainTypes> BeaconChain<T> {
    /// Verify a signed inclusion list received via gossip.
    ///
    /// Checks:
    /// 1. Slot is the current slot
    /// 2. Fork is Heze or later
    /// 3. Validator is in the inclusion list committee
    /// 4. Committee root matches
    /// 5. Signature is valid
    /// 6. Not a duplicate (checked via InclusionListStore)
    #[allow(clippy::result_large_err)]
    pub fn verify_inclusion_list_for_gossip(
        &self,
        signed_il: SignedInclusionList<T::EthSpec>,
    ) -> Result<VerifiedInclusionList<T>, InclusionListError> {
        let il = &signed_il.message;
        let il_slot = il.slot;
        let validator_index = il.validator_index;

        // Check 1: Slot is the current slot
        let current_slot = self
            .slot_clock
            .now()
            .ok_or(BeaconChainError::UnableToReadSlot)?;

        if il_slot != current_slot {
            return Err(InclusionListError::SlotNotCurrent {
                il_slot,
                current_slot,
            });
        }

        // Check 2: Fork is Heze or later
        let fork_name = self.spec.fork_name_at_slot::<T::EthSpec>(il_slot);
        if !fork_name.heze_enabled() {
            return Err(InclusionListError::PreHezeFork { il_slot });
        }

        // Get head state for committee and signature validation
        let head = self.canonical_head.cached_head();
        let state = &head.snapshot.beacon_state;

        // Check 3: Validator is in the inclusion list committee
        let committee = get_inclusion_list_committee(state, il_slot, &self.spec)
            .map_err(BeaconChainError::BlockProcessingError)?;

        if !committee.contains(&validator_index) {
            return Err(InclusionListError::NotInCommittee {
                validator_index,
                slot: il_slot,
            });
        }

        // Check 4: Committee root matches
        // Spec: hash_tree_root(get_inclusion_list_committee(state, slot))
        // The committee is a fixed-length vector of u64 (ValidatorIndex).
        let committee_fixed: ssz_types::FixedVector<
            u64,
            <T::EthSpec as EthSpec>::InclusionListCommitteeSize,
        > = ssz_types::FixedVector::new(committee.clone()).map_err(|_| {
            BeaconChainError::DBInconsistent(format!(
                "committee size {} != InclusionListCommitteeSize {}",
                committee.len(),
                <T::EthSpec as EthSpec>::InclusionListCommitteeSize::to_usize(),
            ))
        })?;
        let expected_root = committee_fixed.tree_hash_root();
        if il.inclusion_list_committee_root != expected_root {
            return Err(InclusionListError::CommitteeRootMismatch {
                expected: expected_root,
                received: il.inclusion_list_committee_root,
            });
        }

        // Check 5: Signature is valid
        let sig_valid = is_valid_inclusion_list_signature(state, &signed_il, &self.spec)
            .map_err(BeaconChainError::BlockProcessingError)?;

        if !sig_valid {
            return Err(InclusionListError::InvalidSignature);
        }

        // Check 6: Not a duplicate — check if this validator already submitted for this slot.
        // The InclusionListStore will handle equivocation detection on import, but we
        // can pre-check duplicates here for early IGNORE.
        {
            let store = self.inclusion_list_store.lock();
            let key = (il_slot, il.inclusion_list_committee_root);

            // Check if validator is already an equivocator
            if store
                .equivocators
                .get(&key)
                .is_some_and(|eq| eq.contains(&validator_index))
            {
                return Err(InclusionListError::Equivocator {
                    validator_index,
                    slot: il_slot,
                });
            }

            // Check for exact duplicate
            if let Some(lists) = store.inclusion_lists.get(&key)
                && lists.iter().any(|existing| *existing == il.clone())
            {
                return Err(InclusionListError::Duplicate {
                    validator_index,
                    slot: il_slot,
                });
            }
        }

        // Determine if we're before the view freeze cutoff (75% of slot duration).
        let is_before_view_freeze_cutoff = self
            .slot_clock
            .duration_to_slot(current_slot + 1)
            .is_some_and(|remaining| {
                let slot_duration = self.slot_clock.slot_duration();
                // Before cutoff = more than 25% of slot remaining
                remaining > slot_duration / 4
            });

        Ok(VerifiedInclusionList {
            signed_il,
            is_before_view_freeze_cutoff,
        })
    }

    /// Import a verified inclusion list into the InclusionListStore.
    pub fn import_inclusion_list(&self, verified: &VerifiedInclusionList<T>) {
        let mut store = self.inclusion_list_store.lock();
        store.process_signed_inclusion_list(
            verified.signed_il.clone(),
            verified.is_before_view_freeze_cutoff,
        );
    }

    /// Get signed inclusion lists for the given committee member indices at the specified slot.
    ///
    /// Returns signed inclusion lists from the store's cache for validators at the
    /// requested positions in the inclusion list committee.
    pub fn get_inclusion_lists_by_committee_indices(
        &self,
        slot: Slot,
        committee_indices: &[u64],
    ) -> Result<Vec<SignedInclusionList<T::EthSpec>>, BeaconChainError> {
        let head = self.canonical_head.cached_head();
        let state = &head.snapshot.beacon_state;

        let committee = get_inclusion_list_committee(state, slot, &self.spec)
            .map_err(BeaconChainError::BlockProcessingError)?;

        let committee_fixed: ssz_types::FixedVector<
            u64,
            <T::EthSpec as EthSpec>::InclusionListCommitteeSize,
        > = ssz_types::FixedVector::new(committee.clone()).map_err(|_| {
            BeaconChainError::DBInconsistent(format!(
                "committee size {} != InclusionListCommitteeSize {}",
                committee.len(),
                <T::EthSpec as EthSpec>::InclusionListCommitteeSize::to_usize(),
            ))
        })?;
        let committee_root = committee_fixed.tree_hash_root();

        // Map committee indices to validator indices.
        let requested_validators: std::collections::HashSet<u64> = committee_indices
            .iter()
            .filter_map(|&ci| committee.get(ci as usize).copied())
            .collect();

        let store = self.inclusion_list_store.lock();
        let key = (slot, committee_root);

        let mut result = Vec::new();
        if let Some(signed_ils) = store.signed_cache.get(&key) {
            for (&validator_index, signed_il) in signed_ils {
                if requested_validators.contains(&validator_index) {
                    result.push(signed_il.clone());
                }
            }
        }

        Ok(result)
    }

    /// Check whether a payload envelope satisfies the inclusion list requirements.
    ///
    /// Pre-Heze: always returns true (no IL requirements).
    /// Heze: checks whether the payload's transactions include all IL transactions
    /// from non-equivocating committee members.
    ///
    /// This is a best-effort check using the local InclusionListStore. If the committee
    /// cannot be computed (e.g., during sync), returns true to avoid blocking.
    pub fn check_inclusion_list_satisfaction(
        &self,
        envelope: &types::ExecutionPayloadEnvelope<T::EthSpec>,
    ) -> bool {
        let fork_name = self.spec.fork_name_at_slot::<T::EthSpec>(envelope.slot);
        if !fork_name.heze_enabled() {
            return true;
        }

        let head = self.canonical_head.cached_head();
        let state = &head.snapshot.beacon_state;

        let Ok(committee) = get_inclusion_list_committee(state, envelope.slot, &self.spec) else {
            return true; // Cannot compute committee (e.g., during sync)
        };

        let Ok(committee_fixed): Result<
            ssz_types::FixedVector<u64, <T::EthSpec as EthSpec>::InclusionListCommitteeSize>,
            _,
        > = ssz_types::FixedVector::new(committee) else {
            return true;
        };
        let committee_root = committee_fixed.tree_hash_root();

        let store = self.inclusion_list_store.lock();
        let il_txs = store.get_inclusion_list_transactions(envelope.slot, committee_root);

        if il_txs.is_empty() {
            return true;
        }

        // Build a set of payload transactions for O(1) lookup.
        let payload_tx_set: std::collections::HashSet<Vec<u8>> = envelope
            .payload
            .transactions
            .iter()
            .map(
                |tx: &ssz_types::VariableList<
                    u8,
                    <T::EthSpec as EthSpec>::MaxBytesPerTransaction,
                >| tx.to_vec(),
            )
            .collect();

        // Check that every IL transaction is included in the payload.
        il_txs.iter().all(|il_tx| payload_tx_set.contains(il_tx))
    }

    /// Compute `inclusion_list_bits` for self-build block production (Heze).
    ///
    /// Returns a BitVector with bits set for IL committee members whose inclusion
    /// lists have been observed. Pre-Heze: returns all-zeros default.
    pub fn compute_inclusion_list_bits_for_slot(
        &self,
        slot: Slot,
    ) -> types::BitVector<<T::EthSpec as EthSpec>::InclusionListCommitteeSize> {
        let fork_name = self.spec.fork_name_at_slot::<T::EthSpec>(slot);
        if !fork_name.heze_enabled() {
            return types::BitVector::default();
        }

        let head = self.canonical_head.cached_head();
        let state = &head.snapshot.beacon_state;

        let Ok(committee) = get_inclusion_list_committee(state, slot, &self.spec) else {
            return types::BitVector::default();
        };

        let committee_fixed: ssz_types::FixedVector<
            u64,
            <T::EthSpec as EthSpec>::InclusionListCommitteeSize,
        > = match ssz_types::FixedVector::new(committee.clone()) {
            Ok(v) => v,
            Err(_) => return types::BitVector::default(),
        };
        let committee_root = committee_fixed.tree_hash_root();

        let store = self.inclusion_list_store.lock();
        let bits = store.get_inclusion_list_bits(&committee, committee_root, slot);

        let mut bitvector = types::BitVector::default();
        for (i, &bit) in bits.iter().enumerate() {
            if bit && bitvector.set(i, true).is_err() {
                break;
            }
        }
        bitvector
    }
}
