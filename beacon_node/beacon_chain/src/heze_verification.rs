//! Provides verification for Heze FOCIL (EIP-7805) messages:
//!
//! - `SignedInclusionList` - Inclusion list submissions from committee members
//!
//! These messages are received via gossip and must be validated before:
//! 1. Propagation to other peers
//! 2. Import to the InclusionListStore

use crate::{BeaconChain, BeaconChainError, BeaconChainTypes, metrics};
use slot_clock::SlotClock;
use state_processing::per_block_processing::heze::{
    get_inclusion_list_committee, is_valid_inclusion_list_signature,
};
use strum::AsRefStr;
use tree_hash::TreeHash;
use types::{EthSpec, Hash256, SignedInclusionList, Slot, Unsigned};

/// Maximum total transaction bytes per inclusion list.
/// Spec: `MAX_BYTES_PER_INCLUSION_LIST = 8192`
const MAX_BYTES_PER_INCLUSION_LIST: usize = 8192;

/// Returned when an inclusion list was not successfully verified.
#[derive(Debug, AsRefStr)]
pub enum InclusionListError {
    /// The inclusion list slot is not the current or previous slot.
    ///
    /// Spec: `[REJECT] message.slot is equal to the previous or current slot.`
    SlotNotCurrentOrPrevious { il_slot: Slot, current_slot: Slot },
    /// The inclusion list is from the previous slot but the current time has passed
    /// the attestation due deadline.
    ///
    /// Spec: `[IGNORE] message.slot is equal to the current slot, or it is equal to
    /// the previous slot and the current time is less than get_attestation_due_ms(epoch)
    /// milliseconds into the slot.`
    PreviousSlotTooLate { il_slot: Slot, current_slot: Slot },
    /// The total transaction bytes exceed MAX_BYTES_PER_INCLUSION_LIST.
    ///
    /// Spec: `[REJECT] The size of message.transactions is within upperbound
    /// MAX_BYTES_PER_INCLUSION_LIST.`
    TransactionsTooLarge { total_bytes: usize },
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
    /// Spec: `[IGNORE] The inclusion_list_committee for slot message.slot on the current
    /// branch corresponds to message.inclusion_list_committee_root.`
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
    /// Spec: `[IGNORE] The message is either the first or second valid message received
    /// from the validator.`
    Duplicate { validator_index: u64, slot: Slot },
    /// This validator has already been marked as an equivocator (third+ message).
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
    /// Checks (per spec p2p-interface.md):
    /// 1. Transaction size within MAX_BYTES_PER_INCLUSION_LIST
    /// 2. Slot is current or previous
    /// 3. Timing: current slot always ok, previous slot only before attestation_due
    /// 4. Committee root matches (IGNORE on mismatch — depends on chain view)
    /// 5. Validator is in the inclusion list committee (REJECT)
    /// 6. Not a duplicate/equivocator (IGNORE)
    /// 7. Signature is valid (REJECT)
    #[allow(clippy::result_large_err)]
    pub fn verify_inclusion_list_for_gossip(
        &self,
        signed_il: SignedInclusionList<T::EthSpec>,
    ) -> Result<VerifiedInclusionList<T>, InclusionListError> {
        let _timer = metrics::start_timer(&metrics::INCLUSION_LIST_GOSSIP_VERIFICATION_TIMES);
        let il = &signed_il.message;
        let il_slot = il.slot;
        let validator_index = il.validator_index;

        // Check 1: Transaction size within MAX_BYTES_PER_INCLUSION_LIST
        let total_tx_bytes: usize = il
            .transactions
            .iter()
            .map(ssz_types::VariableList::len)
            .sum();
        if total_tx_bytes > MAX_BYTES_PER_INCLUSION_LIST {
            return Err(InclusionListError::TransactionsTooLarge {
                total_bytes: total_tx_bytes,
            });
        }

        // Check 2: Slot is current or previous
        let current_slot = self
            .slot_clock
            .now()
            .ok_or(BeaconChainError::UnableToReadSlot)?;

        let previous_slot = current_slot.saturating_sub(1u64);
        if il_slot != current_slot && il_slot != previous_slot {
            return Err(InclusionListError::SlotNotCurrentOrPrevious {
                il_slot,
                current_slot,
            });
        }

        // Check 3: If previous slot, must be before attestation_due into the current slot
        if il_slot == previous_slot && il_slot != current_slot {
            let current_epoch = current_slot.epoch(<T::EthSpec as EthSpec>::slots_per_epoch());
            let attestation_due_ms = self.spec.get_attestation_due_ms(current_epoch);

            let ms_into_slot = self
                .slot_clock
                .millis_from_current_slot_start()
                .map_or(u64::MAX, |d| d.as_millis() as u64);

            if ms_into_slot >= attestation_due_ms {
                return Err(InclusionListError::PreviousSlotTooLate {
                    il_slot,
                    current_slot,
                });
            }
        }

        // Check: Fork is Heze or later
        let fork_name = self.spec.fork_name_at_slot::<T::EthSpec>(il_slot);
        if !fork_name.heze_enabled() {
            return Err(InclusionListError::PreHezeFork { il_slot });
        }

        // Get head state for committee and signature validation
        let head = self.canonical_head.cached_head();
        let state = &head.snapshot.beacon_state;

        // Compute committee for this slot
        let committee = get_inclusion_list_committee(state, il_slot, &self.spec)
            .map_err(BeaconChainError::BlockProcessingError)?;

        // Check 4: Committee root matches (IGNORE on mismatch — peer may have different chain view)
        // Spec: hash_tree_root(get_inclusion_list_committee(state, slot))
        // Must check root BEFORE membership — if root mismatches, membership check is meaningless.
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

        // Check 5: Validator is in the inclusion list committee
        if !committee.contains(&validator_index) {
            return Err(InclusionListError::NotInCommittee {
                validator_index,
                slot: il_slot,
            });
        }

        // Check 7: Signature is valid
        let sig_valid = is_valid_inclusion_list_signature(state, &signed_il, &self.spec)
            .map_err(BeaconChainError::BlockProcessingError)?;

        if !sig_valid {
            return Err(InclusionListError::InvalidSignature);
        }

        // Check 6: Not a duplicate/equivocator — check if this validator already submitted for this slot.
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
                && lists.iter().any(|existing| existing == il)
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
    pub fn import_inclusion_list(&self, verified: VerifiedInclusionList<T>) {
        let mut store = self.inclusion_list_store.lock();
        store.process_signed_inclusion_list(
            verified.signed_il,
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
    /// from non-equivocating committee members at slot - 1.
    ///
    /// Spec: `record_payload_inclusion_list_satisfaction` uses `Slot(state.slot - 1)` —
    /// inclusion lists broadcast at slot N-1 constrain the payload at slot N.
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

        // Spec: get_inclusion_list_transactions(store, state, Slot(state.slot - 1))
        // ILs broadcast at slot N-1 constrain the payload at slot N.
        let il_slot = match envelope.slot.as_u64().checked_sub(1) {
            Some(s) => Slot::new(s),
            None => return true, // slot 0 has no previous slot
        };

        let head = self.canonical_head.cached_head();
        let state = &head.snapshot.beacon_state;

        let Ok(committee) = get_inclusion_list_committee(state, il_slot, &self.spec) else {
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
        let il_txs = store.get_inclusion_list_transactions(il_slot, committee_root);

        if il_txs.is_empty() {
            return true;
        }

        // Build a set of payload transactions for O(1) lookup using references to avoid cloning.
        let payload_tx_set: std::collections::HashSet<&[u8]> = envelope
            .payload
            .transactions
            .iter()
            .map(|tx| tx.as_ref() as &[u8])
            .collect();

        // Check that every IL transaction is included in the payload.
        let satisfied = il_txs
            .iter()
            .all(|il_tx| payload_tx_set.contains(il_tx.as_slice()));
        if satisfied {
            metrics::inc_counter(&metrics::INCLUSION_LIST_SATISFACTION_PASS_TOTAL);
        } else {
            metrics::inc_counter(&metrics::INCLUSION_LIST_SATISFACTION_FAIL_TOTAL);
        }
        satisfied
    }

    /// Compute `inclusion_list_bits` for self-build block production (Heze).
    ///
    /// Returns a BitVector with bits set for IL committee members whose inclusion
    /// lists have been observed at slot - 1.
    ///
    /// Spec: `bid.inclusion_list_bits` must satisfy
    /// `is_inclusion_list_bits_inclusive(store, state, slot - 1, bits)`.
    /// ILs broadcast at slot N-1 constrain the bid/payload at slot N.
    ///
    /// Pre-Heze: returns all-zeros default.
    pub fn compute_inclusion_list_bits_for_slot(
        &self,
        slot: Slot,
    ) -> types::BitVector<<T::EthSpec as EthSpec>::InclusionListCommitteeSize> {
        let fork_name = self.spec.fork_name_at_slot::<T::EthSpec>(slot);
        if !fork_name.heze_enabled() {
            return types::BitVector::default();
        }

        // Spec: is_inclusion_list_bits_inclusive(store, state, slot - 1, bits)
        // ILs broadcast at slot N-1 constrain the bid at slot N.
        let il_slot = match slot.as_u64().checked_sub(1) {
            Some(s) => Slot::new(s),
            None => return types::BitVector::default(),
        };

        let head = self.canonical_head.cached_head();
        let state = &head.snapshot.beacon_state;

        let Ok(committee) = get_inclusion_list_committee(state, il_slot, &self.spec) else {
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
        let bits = store.get_inclusion_list_bits(&committee, committee_root, il_slot);

        let mut bitvector = types::BitVector::default();
        for (i, &bit) in bits.iter().enumerate() {
            if bit && bitvector.set(i, true).is_err() {
                break;
            }
        }
        bitvector
    }
}
