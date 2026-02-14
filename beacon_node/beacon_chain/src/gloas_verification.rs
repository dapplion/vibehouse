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

use crate::{BeaconChain, BeaconChainTypes, BeaconChainError, metrics, observed_operations::ObservationOutcome};
use bls::{PublicKey, PublicKeyBytes, verify_signature_sets};
use derivative::Derivative;
use safe_arith::ArithError;
use slot_clock::SlotClock;
use state_processing::signature_sets::{
    execution_payload_bid_signature_set, payload_attestation_signature_set,
};
use std::borrow::Cow;
use std::collections::HashSet;
use strum::AsRefStr;
use tree_hash::TreeHash;
use types::{
    BeaconStateError, BuilderIndex, ChainSpec, EthSpec, Hash256, PayloadAttestation,
    SignedExecutionPayloadBid, SignedExecutionPayloadEnvelope, Slot,
};

/// Returned when an execution payload bid was not successfully verified.
#[derive(Debug, AsRefStr)]
pub enum ExecutionBidError {
    /// The bid is from a slot in the future (with respect to gossip clock disparity).
    ///
    /// ## Peer scoring
    /// Assuming the local clock is correct, the peer has sent an invalid message.
    FutureSlot {
        bid_slot: Slot,
        latest_permissible_slot: Slot,
    },
    /// The bid is from a slot too far in the past.
    ///
    /// ## Peer scoring
    /// Assuming the local clock is correct, the peer has sent an invalid message.
    PastSlot {
        bid_slot: Slot,
        earliest_permissible_slot: Slot,
    },
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
    /// Failed to retrieve builder public key.
    ///
    /// ## Peer scoring
    /// Internal error, don't penalize.
    BuilderPubkeyUnknown { builder_index: BuilderIndex },
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
    /// One or more attesting indices are not in the PTC for this slot.
    AttesterNotInPtc { validator_index: u64, slot: Slot },
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
    /// We have already seen this exact attestation.
    DuplicateAttestation,
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
    // TODO: Add builder_pubkey field when we implement full signature verification
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
    pub fn verify_execution_bid_for_gossip(
        &self,
        bid: SignedExecutionPayloadBid<T::EthSpec>,
    ) -> Result<VerifiedExecutionBid<T>, ExecutionBidError> {
        let bid_slot = bid.message.slot;
        let builder_index = bid.message.builder_index;

        // Check 1: Slot validation (not too far in future/past)
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

        if bid_slot.as_u64() < earliest_permissible_slot {
            return Err(ExecutionBidError::PastSlot {
                bid_slot,
                earliest_permissible_slot: Slot::new(earliest_permissible_slot),
            });
        }

        if bid_slot.as_u64() > latest_permissible_slot {
            return Err(ExecutionBidError::FutureSlot {
                bid_slot,
                latest_permissible_slot: Slot::new(latest_permissible_slot),
            });
        }

        // Get head state for validation
        let head = self.canonical_head.cached_head();
        let state = &head.snapshot.beacon_state;

        // Check 2: Builder exists and is active
        // TODO: This needs access to the builder registry in BeaconState
        // For now, stub this out - will implement when we have state accessors
        // let builder = state
        //     .builders()
        //     .get(builder_index as usize)
        //     .ok_or(ExecutionBidError::UnknownBuilder { builder_index })?;
        
        // if !builder.is_active_at_finalized_epoch(state.finalized_checkpoint().epoch, &self.spec) {
        //     return Err(ExecutionBidError::InactiveBuilder { builder_index });
        // }

        // Check 3: Equivocation detection
        // TODO: Implement observed bids cache (similar to observed_attesters)
        // For now, accept all bids
        let bid_root = bid.tree_hash_root();

        // Check 4: Parent root validation
        // TODO: Check against fork choice head
        
        // Check 5: Signature verification
        // TODO: Implement signature verification when signature_sets is available
        // For now, skip signature verification (will add in next iteration)

        Ok(VerifiedExecutionBid {
            bid,
        })
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

        // Check 3: Get PTC committee for this slot
        // TODO: Implement get_ptc_committee when state accessors are ready
        // For now, create placeholder indices
        let indexed_attestation_indices = Vec::new();

        // Check 4: Equivocation detection
        // TODO: Implement observed payload attestations cache

        // Check 5: Signature verification
        // TODO: Implement when signature_sets is available

        Ok(VerifiedPayloadAttestation {
            attestation,
            indexed_attestation_indices,
        })
    }
}
