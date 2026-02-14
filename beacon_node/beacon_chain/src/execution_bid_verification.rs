//! Provides verification for `SignedExecutionPayloadBid` messages received via gossip or the HTTP API.
//!
//! This module verifies execution payload bids from builders in the Gloas ePBS protocol.
//!
//! ## Verification flow
//!
//! ```ignore
//!      types::SignedExecutionPayloadBid
//!              |
//!              ▼
//!      verify_execution_bid_for_gossip()
//!              |
//!              ▼
//!      GossipVerifiedExecutionBid
//!              |
//!              ▼
//!      passed to fork_choice.on_execution_bid()
//! ```

use crate::{BeaconChain, BeaconChainError, BeaconChainTypes, metrics};
use bls::{PublicKeyBytes, verify_signature_sets};
use derivative::Derivative;
use safe_arith::ArithError;
use slot_clock::SlotClock;
use ssz::Encode;
use state_processing::signature_sets::execution_bid_signature_set;
use std::borrow::Cow;
use strum::AsRefStr;
use tree_hash::TreeHash;
use types::{
    BeaconStateError, ChainSpec, EthSpec, Hash256, SignedExecutionPayloadBid, Slot, Unsigned,
};

/// Returned when an execution bid was not successfully verified. It might not have been verified for
/// two reasons:
///
/// - The execution bid is malformed or inappropriate for the context (indicated by all variants
///   other than `BeaconChainError`).
/// - The application encountered an internal error whilst attempting to determine validity
///   (the `BeaconChainError` variant)
#[derive(Debug, AsRefStr)]
pub enum Error {
    /// The execution bid is from a slot that is later than the current slot (with respect to the
    /// gossip clock disparity).
    ///
    /// ## Peer scoring
    ///
    /// Assuming the local clock is correct, the peer has sent an invalid message.
    FutureSlot {
        bid_slot: Slot,
        latest_permissible_slot: Slot,
    },
    /// The execution bid is from a slot that is prior to the earliest permissible slot (with
    /// respect to the gossip clock disparity).
    ///
    /// ## Peer scoring
    ///
    /// Assuming the local clock is correct, the peer has sent an invalid message.
    PastSlot {
        bid_slot: Slot,
        earliest_permissible_slot: Slot,
    },
    /// The builder_index refers to a builder that does not exist in the beacon state.
    ///
    /// ## Peer scoring
    ///
    /// The peer has sent an invalid message.
    UnknownBuilder { builder_index: u64 },
    /// The builder is not active (not finalized or already exited).
    ///
    /// ## Peer scoring
    ///
    /// The peer has sent an invalid message.
    BuilderNotActive { builder_index: u64 },
    /// The builder does not have sufficient balance to cover the bid value.
    ///
    /// ## Peer scoring
    ///
    /// The peer has sent an invalid message.
    InsufficientBuilderBalance {
        builder_index: u64,
        balance: u64,
        bid_value: u64,
    },
    /// The parent block root does not match the head block root.
    ///
    /// ## Peer scoring
    ///
    /// The bid may be valid for a different fork, but we don't need it.
    InvalidParentRoot {
        bid_parent: Hash256,
        expected_parent: Hash256,
    },
    /// We have already seen an execution bid from this builder for this slot.
    ///
    /// ## Peer scoring
    ///
    /// Duplicate bids are not useful.
    BidAlreadyKnown {
        builder_index: u64,
        slot: Slot,
        bid_root: Hash256,
    },
    /// We have already seen a CONFLICTING execution bid from this builder for this slot (equivocation).
    ///
    /// ## Peer scoring
    ///
    /// The builder is equivocating. Reject and potentially slash.
    BidEquivocation {
        builder_index: u64,
        slot: Slot,
        first_root: Hash256,
        second_root: Hash256,
    },
    /// The bid signature is invalid.
    ///
    /// ## Peer scoring
    ///
    /// The peer has sent an invalid message.
    InvalidSignature,
    /// Self-build bids must have value = 0.
    ///
    /// ## Peer scoring
    ///
    /// The peer has sent an invalid message.
    SelfBuildMustHaveZeroValue { value: u64 },
    /// Self-build bids must have an infinity signature.
    ///
    /// ## Peer scoring
    ///
    /// The peer has sent an invalid message.
    SelfBuildMustHaveInfinitySignature,
    /// The bid value is zero but it's not a self-build.
    ///
    /// ## Peer scoring
    ///
    /// The peer has sent an invalid message.
    ZeroValueNonSelfBuild { builder_index: u64 },
    /// There was an error whilst processing the execution bid. It is not known if it is valid or invalid.
    ///
    /// ## Peer scoring
    ///
    /// We were unable to process this execution bid due to an internal error. It's unclear if the
    /// execution bid is valid.
    BeaconChainError(Box<BeaconChainError>),
    /// There was an error whilst processing the execution bid. It is not known if it is valid or invalid.
    ///
    /// ## Peer scoring
    ///
    /// We were unable to process this execution bid due to an internal error. It's unclear if the
    /// execution bid is valid.
    BeaconStateError(BeaconStateError),
    /// There was an error whilst processing the execution bid. It is not known if it is valid or invalid.
    ///
    /// ## Peer scoring
    ///
    /// We were unable to process this execution bid due to an internal error. It's unclear if the
    /// execution bid is valid.
    ArithError(ArithError),
}

impl From<BeaconChainError> for Error {
    fn from(e: BeaconChainError) -> Self {
        Error::BeaconChainError(Box::new(e))
    }
}

impl From<BeaconStateError> for Error {
    fn from(e: BeaconStateError) -> Self {
        Error::BeaconStateError(e)
    }
}

impl From<ArithError> for Error {
    fn from(e: ArithError) -> Self {
        Error::ArithError(e)
    }
}

/// A wrapper around a `SignedExecutionPayloadBid` that has been verified for gossip.
#[derive(Debug, Derivative)]
#[derivative(Clone(bound = "E: EthSpec"))]
pub struct GossipVerifiedExecutionBid<E: EthSpec> {
    bid: Box<SignedExecutionPayloadBid<E>>,
    bid_root: Hash256,
}

impl<E: EthSpec> GossipVerifiedExecutionBid<E> {
    pub fn new(
        bid: Box<SignedExecutionPayloadBid<E>>,
        bid_root: Hash256,
    ) -> Self {
        Self { bid, bid_root }
    }

    pub fn bid(&self) -> &SignedExecutionPayloadBid<E> {
        &self.bid
    }

    pub fn bid_root(&self) -> Hash256 {
        self.bid_root
    }

    pub fn into_inner(self) -> Box<SignedExecutionPayloadBid<E>> {
        self.bid
    }

    /// Verify the execution bid for gossip.
    ///
    /// This performs all the checks required before accepting an execution bid from the gossip network:
    /// - Slot timing (not too far in the future or past)
    /// - Builder existence and active status
    /// - Builder balance sufficiency
    /// - Signature validity
    /// - Self-build semantics (value=0, infinity signature)
    /// - Duplicate/equivocation detection
    pub fn verify(
        bid: SignedExecutionPayloadBid<E>,
        chain: &BeaconChain<impl BeaconChainTypes<EthSpec = E>>,
    ) -> Result<Self, Error> {
        let bid_slot = bid.message.slot;
        let builder_index = bid.message.builder_index;
        let bid_value = bid.message.value;

        // Compute bid root for caching
        let bid_root = bid.message.tree_hash_root();

        // 1. Slot timing validation
        let current_slot = chain
            .slot_clock
            .now_or_genesis()
            .ok_or(BeaconChainError::UnableToReadSlot)?;

        let earliest_permissible_slot = current_slot
            .saturating_sub(chain.spec.maximum_gossip_clock_disparity());
        let latest_permissible_slot = current_slot
            .saturating_add(chain.spec.maximum_gossip_clock_disparity());

        if bid_slot > latest_permissible_slot {
            return Err(Error::FutureSlot {
                bid_slot,
                latest_permissible_slot,
            });
        }

        if bid_slot < earliest_permissible_slot {
            return Err(Error::PastSlot {
                bid_slot,
                earliest_permissible_slot,
            });
        }

        // 2. Self-build validation
        let is_self_build = builder_index == chain.spec.builder_index_self_build;

        if is_self_build {
            if bid_value != 0 {
                return Err(Error::SelfBuildMustHaveZeroValue { value: bid_value });
            }
            if !bid.signature.is_infinity() {
                return Err(Error::SelfBuildMustHaveInfinitySignature);
            }
            // Self-build bids don't need further validation
            return Ok(Self::new(Box::new(bid), bid_root));
        }

        // 3. Non-self-build validation

        // Check zero value bids that aren't self-builds
        if bid_value == 0 {
            return Err(Error::ZeroValueNonSelfBuild { builder_index });
        }

        // 4. Check for duplicate or equivocating bids (early exit before expensive state/sig checks)
        if let Some(prev_root) = chain
            .observed_execution_bids
            .lock()
            .observe_bid(builder_index, bid_slot, bid_root)?
        {
            if prev_root == bid_root {
                return Err(Error::BidAlreadyKnown {
                    builder_index,
                    slot: bid_slot,
                    bid_root,
                });
            } else {
                // EQUIVOCATION DETECTED
                return Err(Error::BidEquivocation {
                    builder_index,
                    slot: bid_slot,
                    first_root: prev_root,
                    second_root: bid_root,
                });
            }
        }

        // 5. Get head state to validate builder
        let head_state = chain.head_beacon_state_cloned();
        let state_gloas = head_state
            .as_gloas()
            .map_err(|_| BeaconChainError::IncorrectStateVariant)?;

        // 6. Builder existence and activation check
        let builder = state_gloas
            .builders()
            .get(builder_index as usize)
            .ok_or(Error::UnknownBuilder { builder_index })?;

        let finalized_epoch = head_state.finalized_checkpoint().epoch;
        if !builder.is_active_at_finalized_epoch(finalized_epoch) {
            return Err(Error::BuilderNotActive { builder_index });
        }

        // 7. Builder balance check
        if builder.balance < bid_value {
            return Err(Error::InsufficientBuilderBalance {
                builder_index,
                balance: builder.balance,
                bid_value,
            });
        }

        // 8. Signature verification
        let signature_set = execution_bid_signature_set(
            |_| Ok(Cow::Borrowed(&builder.pubkey)),
            &bid,
            &head_state.fork(),
            head_state.genesis_validators_root(),
            &chain.spec,
        )?;

        if !verify_signature_sets(std::iter::once(&signature_set)) {
            return Err(Error::InvalidSignature);
        }

        // All checks passed
        Ok(Self::new(Box::new(bid), bid_root))
    }
}
