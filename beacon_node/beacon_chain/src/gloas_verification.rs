//! Provides verification for the following gloas ePBS messages:
//!
//! - `SignedExecutionPayloadBid` - Builder bids for execution payload slots
//! - `SignedExecutionPayloadEnvelope` - Builder payload reveals
//! - `PayloadAttestation` - PTC member payload attestations
//!
//! These messages are received via gossip and must be validated before:
//! 1. Propagation to other peers
//! 2. Import to fork choice
//! 3. Storage in the operation pool
//!
//! Verification follows the same pattern as sync committee and attestation verification:
//! - Wrapper types represent different verification stages
//! - Early rejection for invalid messages (peer scoring)
//! - Equivocation detection via observed message tracking
//! - Signature verification batching where applicable

use crate::{BeaconChain, BeaconChainError, BeaconChainTypes};
use bls::PublicKey;
use safe_arith::ArithError;
use slot_clock::SlotClock;
use state_processing;
use state_processing::signature_sets::{
    execution_payload_bid_signature_set, execution_payload_envelope_signature_set,
    payload_attestation_signature_set,
};
use std::borrow::Cow;
use std::sync::Arc;
use strum::AsRefStr;
use tree_hash::TreeHash;
use types::{
    BeaconStateError, BuilderIndex, EthSpec, ExecutionBlockHash, Hash256, PayloadAttestation,
    SignedExecutionPayloadBid, SignedExecutionPayloadEnvelope, Slot,
    consts::gloas::BUILDER_INDEX_SELF_BUILD,
};

/// Returned when an execution payload bid was not successfully verified.
#[derive(Debug, AsRefStr)]
pub enum ExecutionBidError {
    /// The bid slot is not the current or next slot.
    ///
    /// Spec: `[IGNORE] bid.slot is the current slot or the next slot.`
    ///
    /// ## Peer scoring
    /// Not malicious, just not timely.
    SlotNotCurrentOrNext { bid_slot: Slot, current_slot: Slot },
    /// The bid's execution_payment field is zero.
    ///
    /// Spec: `[REJECT] bid.execution_payment is zero.`
    ///
    /// ## Peer scoring
    /// The peer has sent an invalid message.
    ZeroExecutionPayment,
    /// The builder index does not exist in the builder registry.
    ///
    /// ## Peer scoring
    /// The peer has sent an invalid message.
    UnknownBuilder { builder_index: BuilderIndex },
    /// The builder is not active at the current finalized epoch.
    ///
    /// ## Peer scoring
    /// The peer has sent an invalid message.
    InactiveBuilder { builder_index: BuilderIndex },
    /// The builder has insufficient balance to cover the bid value.
    ///
    /// ## Peer scoring
    /// The peer has sent an invalid message.
    InsufficientBuilderBalance {
        builder_index: BuilderIndex,
        balance: u64,
        bid_value: u64,
    },
    /// We have already observed a different bid from this builder for this slot.
    /// This is equivocation.
    ///
    /// ## Peer scoring
    /// The peer is relaying equivocating messages. Penalize heavily.
    BuilderEquivocation {
        builder_index: BuilderIndex,
        slot: Slot,
        previous_bid_root: Hash256,
        new_bid_root: Hash256,
    },
    /// We have already seen this exact bid (same root).
    ///
    /// ## Peer scoring
    /// Duplicate message, ignore but don't penalize.
    DuplicateBid { bid_root: Hash256 },
    /// The bid signature is invalid.
    ///
    /// ## Peer scoring
    /// The peer has sent an invalid message.
    InvalidSignature,
    /// The parent block root does not match the head.
    ///
    /// ## Peer scoring
    /// The peer may be on a different fork or sending stale bids.
    InvalidParentRoot {
        expected: Hash256,
        received: Hash256,
    },
    /// Beacon chain error occurred during validation.
    BeaconChainError(BeaconChainError),
    /// State error occurred during validation.
    BeaconStateError(BeaconStateError),
    /// Arithmetic error.
    ArithError(ArithError),
}

/// Returned when a payload attestation was not successfully verified.
#[derive(Debug, AsRefStr)]
pub enum PayloadAttestationError {
    /// The attestation is from a slot in the future.
    FutureSlot {
        attestation_slot: Slot,
        latest_permissible_slot: Slot,
    },
    /// The attestation is from a slot too far in the past.
    PastSlot {
        attestation_slot: Slot,
        earliest_permissible_slot: Slot,
    },
    /// The beacon block root does not match any known block.
    UnknownBeaconBlockRoot { root: Hash256 },
    /// A validator in this attestation has already submitted a conflicting attestation
    /// (different payload_present value for same slot/block).
    ///
    /// ## Peer scoring
    /// The peer is relaying equivocating messages. Penalize heavily.
    ValidatorEquivocation {
        validator_index: u64,
        slot: Slot,
        beacon_block_root: Hash256,
    },
    /// The aggregation bits are invalid (wrong size, etc).
    InvalidAggregationBits,
    /// The signature is invalid.
    InvalidSignature,
    /// No validators attested (empty aggregation bits).
    EmptyAggregationBits,
    /// Failed to get PTC committee for the slot.
    PtcCommitteeError { slot: Slot },
    /// Beacon chain error occurred during validation.
    BeaconChainError(BeaconChainError),
    /// State error occurred during validation.
    BeaconStateError(BeaconStateError),
}

impl From<BeaconChainError> for ExecutionBidError {
    fn from(e: BeaconChainError) -> Self {
        ExecutionBidError::BeaconChainError(e)
    }
}

impl From<BeaconStateError> for ExecutionBidError {
    fn from(e: BeaconStateError) -> Self {
        ExecutionBidError::BeaconStateError(e)
    }
}

impl From<ArithError> for ExecutionBidError {
    fn from(e: ArithError) -> Self {
        ExecutionBidError::ArithError(e)
    }
}

impl From<BeaconChainError> for PayloadAttestationError {
    fn from(e: BeaconChainError) -> Self {
        PayloadAttestationError::BeaconChainError(e)
    }
}

impl From<BeaconStateError> for PayloadAttestationError {
    fn from(e: BeaconStateError) -> Self {
        PayloadAttestationError::BeaconStateError(e)
    }
}

/// A `SignedExecutionPayloadBid` that has been validated for gossip.
#[derive(Debug, Clone)]
pub struct VerifiedExecutionBid<T: BeaconChainTypes> {
    bid: SignedExecutionPayloadBid<T::EthSpec>,
}

impl<T: BeaconChainTypes> VerifiedExecutionBid<T> {
    /// Returns a reference to the underlying bid.
    pub fn bid(&self) -> &SignedExecutionPayloadBid<T::EthSpec> {
        &self.bid
    }

    /// Consume self and return the underlying bid.
    pub fn into_inner(self) -> SignedExecutionPayloadBid<T::EthSpec> {
        self.bid
    }

    /// Create a `VerifiedExecutionBid` without running gossip validation.
    /// For use in integration tests only.
    #[doc(hidden)]
    pub fn __new_for_testing(bid: SignedExecutionPayloadBid<T::EthSpec>) -> Self {
        Self { bid }
    }
}

/// A `PayloadAttestation` that has been validated for gossip.
#[derive(Debug, Clone)]
pub struct VerifiedPayloadAttestation<T: BeaconChainTypes> {
    attestation: PayloadAttestation<T::EthSpec>,
    /// Indices of validators who attested (derived from aggregation bits + PTC).
    indexed_attestation_indices: Vec<u64>,
}

impl<T: BeaconChainTypes> VerifiedPayloadAttestation<T> {
    /// Returns a reference to the underlying attestation.
    pub fn attestation(&self) -> &PayloadAttestation<T::EthSpec> {
        &self.attestation
    }

    /// Returns the attesting validator indices.
    pub fn attesting_indices(&self) -> &[u64] {
        &self.indexed_attestation_indices
    }

    /// Consume self and return the underlying attestation.
    pub fn into_inner(self) -> PayloadAttestation<T::EthSpec> {
        self.attestation
    }
}

/// Returned when a payload envelope was not successfully verified.
#[derive(Debug, AsRefStr)]
pub enum PayloadEnvelopeError {
    /// The beacon block root is not known in fork choice.
    ///
    /// ## Peer scoring
    /// May not have received the block yet, ignore.
    BlockRootUnknown { block_root: Hash256 },
    /// The envelope's slot doesn't match the block's slot.
    ///
    /// ## Peer scoring
    /// The peer has sent an invalid message.
    SlotMismatch {
        block_slot: Slot,
        envelope_slot: Slot,
    },
    /// The envelope's builder_index doesn't match the committed bid.
    ///
    /// ## Peer scoring
    /// The peer has sent an invalid message.
    BuilderIndexMismatch { committed_bid: u64, envelope: u64 },
    /// The payload block_hash doesn't match the committed bid's block_hash.
    ///
    /// ## Peer scoring
    /// The peer has sent an invalid message.
    BlockHashMismatch {
        committed_bid: ExecutionBlockHash,
        envelope: ExecutionBlockHash,
    },
    /// The builder signature is invalid.
    ///
    /// ## Peer scoring
    /// The peer has sent an invalid message.
    InvalidSignature,
    /// Could not retrieve the beacon block from the database.
    ///
    /// ## Peer scoring
    /// Internal error, don't penalize.
    MissingBeaconBlock { block_root: Hash256 },
    /// The beacon block doesn't have a signed_execution_payload_bid (not a gloas block).
    ///
    /// ## Peer scoring
    /// The peer has sent an invalid message.
    NotGloasBlock { block_root: Hash256 },
    /// The envelope slot is prior to the latest finalized slot.
    ///
    /// ## Peer scoring
    /// Stale message, ignore.
    PriorToFinalization {
        envelope_slot: Slot,
        finalized_slot: Slot,
    },
    /// Beacon chain error occurred during validation.
    BeaconChainError(BeaconChainError),
    /// State error occurred during validation.
    BeaconStateError(BeaconStateError),
}

impl From<BeaconChainError> for PayloadEnvelopeError {
    fn from(e: BeaconChainError) -> Self {
        PayloadEnvelopeError::BeaconChainError(e)
    }
}

impl From<BeaconStateError> for PayloadEnvelopeError {
    fn from(e: BeaconStateError) -> Self {
        PayloadEnvelopeError::BeaconStateError(e)
    }
}

/// A `SignedExecutionPayloadEnvelope` that has been validated for gossip.
#[derive(Debug, Clone)]
pub struct VerifiedPayloadEnvelope<T: BeaconChainTypes> {
    envelope: Arc<SignedExecutionPayloadEnvelope<T::EthSpec>>,
    beacon_block_root: Hash256,
}

impl<T: BeaconChainTypes> VerifiedPayloadEnvelope<T> {
    /// Returns a reference to the underlying envelope.
    pub fn envelope(&self) -> &SignedExecutionPayloadEnvelope<T::EthSpec> {
        &self.envelope
    }

    /// Returns the beacon block root this envelope is for.
    pub fn beacon_block_root(&self) -> Hash256 {
        self.beacon_block_root
    }

    /// Consume self and return the underlying envelope.
    pub fn into_inner(self) -> Arc<SignedExecutionPayloadEnvelope<T::EthSpec>> {
        self.envelope
    }
}

impl<T: BeaconChainTypes> BeaconChain<T> {
    /// Verify an execution payload bid received via gossip.
    ///
    /// This performs the following checks:
    /// 1. Slot is not in the future or too far in the past
    /// 2. Builder exists and is active
    /// 3. Builder has sufficient balance for the bid
    /// 4. No conflicting bid from this builder for this slot (equivocation check)
    /// 5. Parent root matches head (fork choice)
    /// 6. Signature is valid
    #[allow(clippy::result_large_err)]
    pub fn verify_execution_bid_for_gossip(
        &self,
        bid: SignedExecutionPayloadBid<T::EthSpec>,
    ) -> Result<VerifiedExecutionBid<T>, ExecutionBidError> {
        let bid_slot = bid.message.slot;
        let builder_index = bid.message.builder_index;

        // Check 1: Slot validation
        // Spec: [IGNORE] bid.slot is the current slot or the next slot.
        let current_slot = self
            .slot_clock
            .now()
            .ok_or(BeaconChainError::UnableToReadSlot)?;

        let next_slot = current_slot + 1;
        if bid_slot != current_slot && bid_slot != next_slot {
            return Err(ExecutionBidError::SlotNotCurrentOrNext {
                bid_slot,
                current_slot,
            });
        }

        // Check 1b: Spec: [REJECT] bid.execution_payment is zero.
        if bid.message.execution_payment == 0 {
            return Err(ExecutionBidError::ZeroExecutionPayment);
        }

        // Get head state for validation
        let head = self.canonical_head.cached_head();
        let state = &head.snapshot.beacon_state;

        // Check 2: Builder exists and is active
        let builder = state
            .builders()
            .map_err(BeaconChainError::BeaconStateError)?
            .get(builder_index as usize)
            .ok_or(ExecutionBidError::UnknownBuilder { builder_index })?;

        if !builder.is_active_at_finalized_epoch(state.finalized_checkpoint().epoch, &self.spec) {
            return Err(ExecutionBidError::InactiveBuilder { builder_index });
        }

        // Check 2b: Builder has sufficient balance
        if builder.balance < bid.message.value {
            return Err(ExecutionBidError::InsufficientBuilderBalance {
                builder_index,
                balance: builder.balance,
                bid_value: bid.message.value,
            });
        }

        // Check 3: Equivocation detection
        let bid_root = bid.tree_hash_root();

        let observation_outcome =
            self.observed_execution_bids
                .lock()
                .observe_bid(bid_slot, builder_index, bid_root);

        match observation_outcome {
            crate::observed_execution_bids::BidObservationOutcome::New => {
                // Continue with validation
            }
            crate::observed_execution_bids::BidObservationOutcome::Duplicate => {
                return Err(ExecutionBidError::DuplicateBid { bid_root });
            }
            crate::observed_execution_bids::BidObservationOutcome::Equivocation {
                existing_bid_root,
                new_bid_root,
            } => {
                return Err(ExecutionBidError::BuilderEquivocation {
                    builder_index,
                    slot: bid_slot,
                    previous_bid_root: existing_bid_root,
                    new_bid_root,
                });
            }
        }

        // Check 4: Parent root validation
        let head_block_root = head.snapshot.beacon_block_root;
        if bid.message.parent_block_root != head_block_root {
            return Err(ExecutionBidError::InvalidParentRoot {
                expected: head_block_root,
                received: bid.message.parent_block_root,
            });
        }

        // Check 5: Signature verification
        let get_builder_pubkey = |builder_idx: u64| -> Option<Cow<PublicKey>> {
            if builder_idx == BUILDER_INDEX_SELF_BUILD {
                let proposer_index = state.latest_block_header().proposer_index as usize;
                state
                    .validators()
                    .get(proposer_index)
                    .and_then(|v| v.pubkey.decompress().ok().map(Cow::Owned))
            } else {
                state
                    .builders()
                    .ok()?
                    .get(builder_idx as usize)
                    .and_then(|builder| builder.pubkey.decompress().ok().map(Cow::Owned))
            }
        };

        let signature_set =
            execution_payload_bid_signature_set(state, get_builder_pubkey, &bid, &self.spec)
                .map_err(|_| ExecutionBidError::InvalidSignature)?;

        if !signature_set.verify() {
            return Err(ExecutionBidError::InvalidSignature);
        }

        Ok(VerifiedExecutionBid { bid })
    }

    /// Verify a payload attestation received via gossip.
    ///
    /// This performs the following checks:
    /// 1. Slot is not in the future or too far in the past
    /// 2. Beacon block root is known
    /// 3. All attesting indices are in the PTC for this slot
    /// 4. No conflicting attestation from any validator (equivocation check)
    /// 5. Aggregation bits are valid
    /// 6. Signature is valid
    #[allow(clippy::result_large_err)]
    pub fn verify_payload_attestation_for_gossip(
        &self,
        attestation: PayloadAttestation<T::EthSpec>,
    ) -> Result<VerifiedPayloadAttestation<T>, PayloadAttestationError> {
        let attestation_slot = attestation.data.slot;

        // Check 1: Slot validation
        let current_slot = self
            .slot_clock
            .now()
            .ok_or(BeaconChainError::UnableToReadSlot)?;

        let gossip_clock_disparity = self.spec.maximum_gossip_clock_disparity();
        let earliest_permissible_slot = current_slot
            .as_u64()
            .saturating_sub(gossip_clock_disparity.as_secs() / self.spec.seconds_per_slot);
        let latest_permissible_slot = current_slot
            .as_u64()
            .saturating_add(gossip_clock_disparity.as_secs() / self.spec.seconds_per_slot);

        if attestation_slot.as_u64() < earliest_permissible_slot {
            return Err(PayloadAttestationError::PastSlot {
                attestation_slot,
                earliest_permissible_slot: Slot::new(earliest_permissible_slot),
            });
        }

        if attestation_slot.as_u64() > latest_permissible_slot {
            return Err(PayloadAttestationError::FutureSlot {
                attestation_slot,
                latest_permissible_slot: Slot::new(latest_permissible_slot),
            });
        }

        // Check 2: Aggregation bits validation
        if attestation.aggregation_bits.is_zero() {
            return Err(PayloadAttestationError::EmptyAggregationBits);
        }

        // Check 3: Beacon block root is known in fork choice
        {
            let fork_choice = self.canonical_head.fork_choice_read_lock();
            if fork_choice
                .get_block(&attestation.data.beacon_block_root)
                .is_none()
            {
                return Err(PayloadAttestationError::UnknownBeaconBlockRoot {
                    root: attestation.data.beacon_block_root,
                });
            }
        }

        // Check 4: Get PTC committee for this slot
        let head = self.canonical_head.cached_head();
        let state = &head.snapshot.beacon_state;

        // Get PTC committee using state processing function
        let ptc_indices = state_processing::per_block_processing::gloas::get_ptc_committee(
            state,
            attestation_slot,
            &self.spec,
        )
        .map_err(|_| PayloadAttestationError::PtcCommitteeError {
            slot: attestation_slot,
        })?;

        // Convert aggregation bits to attesting indices
        let mut indexed_attestation_indices = Vec::new();
        for (i, &validator_index) in ptc_indices.iter().enumerate() {
            if attestation
                .aggregation_bits
                .get(i)
                .map_err(|_| PayloadAttestationError::InvalidAggregationBits)?
            {
                indexed_attestation_indices.push(validator_index);
            }
        }

        if indexed_attestation_indices.is_empty() {
            return Err(PayloadAttestationError::EmptyAggregationBits);
        }

        // Check 5: Equivocation detection
        let beacon_block_root = attestation.data.beacon_block_root;
        let payload_present = attestation.data.payload_present;

        let mut observed_attestations = self.observed_payload_attestations.lock();
        for &validator_index in &indexed_attestation_indices {
            let outcome = observed_attestations.observe_attestation(
                attestation_slot,
                beacon_block_root,
                validator_index,
                payload_present,
            );

            match outcome {
                crate::observed_payload_attestations::AttestationObservationOutcome::New => {
                    // Continue
                }
                crate::observed_payload_attestations::AttestationObservationOutcome::Duplicate => {
                    // This validator already attested with same value, skip
                    continue;
                }
                crate::observed_payload_attestations::AttestationObservationOutcome::Equivocation {
                    ..
                } => {
                    return Err(PayloadAttestationError::ValidatorEquivocation {
                        validator_index,
                        slot: attestation_slot,
                        beacon_block_root,
                    });
                }
            }
        }

        // Check 6: Signature verification
        let get_pubkey = |validator_idx: usize| -> Option<Cow<PublicKey>> {
            state
                .validators()
                .get(validator_idx)
                .and_then(|validator| validator.pubkey.decompress().ok().map(Cow::Owned))
        };

        let signature_set = payload_attestation_signature_set(
            state,
            get_pubkey,
            &attestation,
            &indexed_attestation_indices,
            &self.spec,
        )
        .map_err(|_| PayloadAttestationError::InvalidSignature)?;

        if !signature_set.verify() {
            return Err(PayloadAttestationError::InvalidSignature);
        }

        Ok(VerifiedPayloadAttestation {
            attestation,
            indexed_attestation_indices,
        })
    }

    /// Verify a payload envelope received via gossip.
    ///
    /// This performs the following checks:
    /// 1. Block root is known in fork choice
    /// 2. Slot is not prior to finalization
    /// 3. Slot matches the beacon block
    /// 4. Builder index matches the committed bid
    /// 5. Payload block_hash matches the committed bid's block_hash
    /// 6. Signature is valid (using DOMAIN_BEACON_BUILDER)
    #[allow(clippy::result_large_err)]
    pub fn verify_payload_envelope_for_gossip(
        &self,
        signed_envelope: Arc<SignedExecutionPayloadEnvelope<T::EthSpec>>,
    ) -> Result<VerifiedPayloadEnvelope<T>, PayloadEnvelopeError> {
        let envelope = &signed_envelope.message;
        let beacon_block_root = envelope.beacon_block_root;

        // Check 1: Block root is known in fork choice.
        //
        // When the block root is unknown, the envelope arrived before its block (a
        // common timing race — they are propagated independently on the p2p network).
        // Store the envelope in `pending_gossip_envelopes` so it can be processed after
        // the block is imported, rather than permanently dropping it.
        let fork_choice = self.canonical_head.fork_choice_read_lock();
        let Some(proto_block) = fork_choice.get_block(&beacon_block_root) else {
            // Buffer the envelope for later processing after the block arrives.
            self.pending_gossip_envelopes
                .lock()
                .insert(beacon_block_root, signed_envelope);
            return Err(PayloadEnvelopeError::BlockRootUnknown {
                block_root: beacon_block_root,
            });
        };
        drop(fork_choice);

        // Check 2: Not prior to finalization
        let finalized_slot = self
            .canonical_head
            .cached_head()
            .finalized_checkpoint()
            .epoch
            .start_slot(T::EthSpec::slots_per_epoch());
        if envelope.slot < finalized_slot {
            return Err(PayloadEnvelopeError::PriorToFinalization {
                envelope_slot: envelope.slot,
                finalized_slot,
            });
        }

        // Check 3: Slot matches the block
        if envelope.slot != proto_block.slot {
            return Err(PayloadEnvelopeError::SlotMismatch {
                block_slot: proto_block.slot,
                envelope_slot: envelope.slot,
            });
        }

        // Get the blinded block to access the committed bid
        // (bid is a consensus field, available in both full and blinded blocks)
        let block = self
            .store
            .get_blinded_block(&beacon_block_root)
            .map_err(|e| PayloadEnvelopeError::BeaconChainError(e.into()))?
            .ok_or(PayloadEnvelopeError::MissingBeaconBlock {
                block_root: beacon_block_root,
            })?;

        let execution_bid = block
            .message()
            .body()
            .signed_execution_payload_bid()
            .map_err(|_| PayloadEnvelopeError::NotGloasBlock {
                block_root: beacon_block_root,
            })?;

        // Check 4: Builder index matches the committed bid
        if envelope.builder_index != execution_bid.message.builder_index {
            return Err(PayloadEnvelopeError::BuilderIndexMismatch {
                committed_bid: execution_bid.message.builder_index,
                envelope: envelope.builder_index,
            });
        }

        // Check 5: Payload block_hash matches the committed bid's block_hash
        if envelope.payload.block_hash != execution_bid.message.block_hash {
            return Err(PayloadEnvelopeError::BlockHashMismatch {
                committed_bid: execution_bid.message.block_hash,
                envelope: envelope.payload.block_hash,
            });
        }

        // Check 6: Signature verification
        //
        // Self-build envelopes (builder_index == BUILDER_INDEX_SELF_BUILD) are signed
        // with an empty/infinity signature — they have no external builder and the
        // proposer is always authorized for their own slot. Skip BLS verification for
        // these; the bid already committed to the block_hash and builder_index (checked
        // above), so accepting the envelope is safe.
        if envelope.builder_index != BUILDER_INDEX_SELF_BUILD {
            let head = self.canonical_head.cached_head();
            let state = &head.snapshot.beacon_state;

            let get_builder_pubkey = |builder_idx: u64| -> Option<Cow<PublicKey>> {
                state
                    .builders()
                    .ok()?
                    .get(builder_idx as usize)
                    .and_then(|builder| builder.pubkey.decompress().ok().map(Cow::Owned))
            };

            let signature_set = execution_payload_envelope_signature_set(
                state,
                get_builder_pubkey,
                &signed_envelope,
                &self.spec,
            )
            .map_err(|_| PayloadEnvelopeError::InvalidSignature)?;

            if !signature_set.verify() {
                return Err(PayloadEnvelopeError::InvalidSignature);
            }
        }

        Ok(VerifiedPayloadEnvelope {
            envelope: signed_envelope,
            beacon_block_root,
        })
    }
}
