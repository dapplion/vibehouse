use crate::{BeaconChain, BeaconChainTypes};
use state_processing::per_block_processing::signature_sets::indexed_payload_attestation_signature_set;
use state_processing::per_block_processing::gloas::{
    get_indexed_payload_attestation, get_ptc_committee,
};
use std::borrow::Cow;
use types::{
    BeaconStateError, EthSpec, Hash256, PayloadAttestation, Slot,
};

/// Errors that can occur during gossip verification of payload attestations.
#[derive(Debug)]
pub enum PayloadAttestationError {
    /// The attestation is from a slot that is later than the current slot (with respect to the gossip clock disparity).
    FutureSlot {
        attestation_slot: Slot,
        latest_permissible_slot: Slot,
    },
    /// The attestation is from a slot that is prior to the earliest permissible slot (with respect to the gossip clock disparity).
    PastSlot {
        attestation_slot: Slot,
        earliest_permissible_slot: Slot,
    },
    /// The attestation's beacon_block_root is not known to us.
    UnknownBeaconBlock { beacon_block_root: Hash256 },
    /// The attestation references a block with a different slot than claimed.
    SlotMismatch {
        attestation_slot: Slot,
        block_slot: Slot,
    },
    /// The attestation is empty (no attesters).
    EmptyAttestation,
    /// The attestation contains invalid indices (out of bounds or not sorted).
    InvalidIndices { reason: String },
    /// The attesting validators are not all in the PTC for this slot.
    InvalidCommitteeMembers { reason: String },
    /// Duplicate: this exact attestation (same data + aggregation bits) was already seen.
    AttestationAlreadyKnown { attestation_root: Hash256 },
    /// Equivocation: validator(s) already attested with different data.
    AttestationEquivocation {
        validator_index: u64,
        existing_data_root: Hash256,
        new_data_root: Hash256,
    },
    /// The attestation's signature is invalid.
    InvalidSignature,
    /// The attestation signature set could not be created.
    SignatureSetError { reason: String },
    /// The beacon state could not be accessed or is invalid.
    BeaconStateError(BeaconStateError),
}

impl From<BeaconStateError> for PayloadAttestationError {
    fn from(e: BeaconStateError) -> Self {
        PayloadAttestationError::BeaconStateError(e)
    }
}

/// A wrapper around a `PayloadAttestation` that has been verified for gossip.
///
/// This type proves that the attestation has passed:
/// - Slot timing checks
/// - Block existence check
/// - PTC committee membership check
/// - Signature verification
/// - Duplicate/equivocation detection
#[derive(Clone)]
pub struct GossipVerifiedPayloadAttestation<T: BeaconChainTypes> {
    pub attestation: PayloadAttestation<T::EthSpec>,
    pub attestation_root: Hash256,
}

impl<T: BeaconChainTypes> GossipVerifiedPayloadAttestation<T> {
    /// Verify a payload attestation for gossip.
    ///
    /// The attestation must:
    /// 1. Be for a slot within the gossip clock disparity
    /// 2. Reference a known beacon block with matching slot
    /// 3. Contain valid, sorted validator indices
    /// 4. Have all attesters be PTC members for the slot
    /// 5. Not be a duplicate or equivocation
    /// 6. Have a valid aggregate BLS signature
    pub fn verify(
        attestation: PayloadAttestation<T::EthSpec>,
        chain: &BeaconChain<T>,
    ) -> Result<Self, PayloadAttestationError> {
        let attestation_slot = attestation.data.slot;
        let attestation_root = attestation.tree_hash_root();

        // Get current slot with gossip clock disparity
        let current_slot = chain.slot()?;
        let gossip_disparity = chain.spec.maximum_gossip_clock_disparity();

        // 1. Timing checks
        let earliest_permissible_slot = current_slot.saturating_sub(gossip_disparity.as_secs());
        let latest_permissible_slot = current_slot.saturating_add(gossip_disparity.as_secs());

        if attestation_slot > latest_permissible_slot {
            return Err(PayloadAttestationError::FutureSlot {
                attestation_slot,
                latest_permissible_slot,
            });
        }

        if attestation_slot < earliest_permissible_slot {
            return Err(PayloadAttestationError::PastSlot {
                attestation_slot,
                earliest_permissible_slot,
            });
        }

        // 2. Block existence check
        let beacon_block_root = attestation.data.beacon_block_root;
        let block = chain
            .get_blinded_block(&beacon_block_root)
            .map_err(|_| PayloadAttestationError::UnknownBeaconBlock { beacon_block_root })?
            .ok_or(PayloadAttestationError::UnknownBeaconBlock { beacon_block_root })?;

        // Verify block slot matches attestation slot
        let block_slot = block.slot();
        if block_slot != attestation_slot {
            return Err(PayloadAttestationError::SlotMismatch {
                attestation_slot,
                block_slot,
            });
        }

        // 3. Convert to indexed attestation
        let state = chain.head_snapshot().beacon_state.clone_with_only_committee_caches();
        let indexed_attestation = get_indexed_payload_attestation(&state, &attestation, &chain.spec)
            .map_err(|e| PayloadAttestationError::InvalidIndices {
                reason: format!("Failed to get indexed attestation: {:?}", e),
            })?;

        // Check not empty
        if indexed_attestation.attesting_indices.is_empty() {
            return Err(PayloadAttestationError::EmptyAttestation);
        }

        // Check indices are sorted (required by spec)
        let indices = &indexed_attestation.attesting_indices;
        if !indices.windows(2).all(|w| w[0] < w[1]) {
            return Err(PayloadAttestationError::InvalidIndices {
                reason: "Indices not sorted".to_string(),
            });
        }

        // 4. Validate PTC committee membership
        let ptc_committee = get_ptc_committee(&state, attestation_slot, &chain.spec)
            .map_err(|e| PayloadAttestationError::InvalidCommitteeMembers {
                reason: format!("Failed to get PTC committee: {:?}", e),
            })?;

        // Check all attesters are in PTC
        for &index in indices.iter() {
            if !ptc_committee.contains(&(index as usize)) {
                return Err(PayloadAttestationError::InvalidCommitteeMembers {
                    reason: format!("Validator {} not in PTC for slot {}", index, attestation_slot),
                });
            }
        }

        // 5. Duplicate/equivocation detection
        let data_root = attestation.data.tree_hash_root();
        {
            let mut observed = chain.observed_payload_attestations.lock();
            
            for &validator_index in indices.iter() {
                match observed.observe_attestation(validator_index, attestation_slot, data_root)? {
                    None => {
                        // New attestation, good
                    }
                    Some(existing_root) if existing_root == data_root => {
                        // Duplicate with same data - could be from aggregation_bits difference
                        // Check if exact same attestation
                        if existing_root == attestation_root {
                            return Err(PayloadAttestationError::AttestationAlreadyKnown {
                                attestation_root,
                            });
                        }
                        // Different aggregation but same data - this is fine (separate messages)
                    }
                    Some(existing_root) => {
                        // Equivocation detected!
                        return Err(PayloadAttestationError::AttestationEquivocation {
                            validator_index,
                            existing_data_root: existing_root,
                            new_data_root: data_root,
                        });
                    }
                }
            }
        }

        // 6. Signature verification
        let signature_set = indexed_payload_attestation_signature_set(
            &state,
            &indexed_attestation.signature,
            &indexed_attestation,
            &chain.spec,
        )
        .map_err(|e| PayloadAttestationError::SignatureSetError {
            reason: format!("{:?}", e),
        })?;

        let signature_is_valid = signature_set.verify();

        if !signature_is_valid {
            return Err(PayloadAttestationError::InvalidSignature);
        }

        Ok(GossipVerifiedPayloadAttestation {
            attestation,
            attestation_root,
        })
    }
}

impl<T: BeaconChainTypes> Into<PayloadAttestation<T::EthSpec>> for GossipVerifiedPayloadAttestation<T> {
    fn into(self) -> PayloadAttestation<T::EthSpec> {
        self.attestation
    }
}
