//! A `SignatureSet` is an abstraction over the components of a signature. A `SignatureSet` may be
//! validated individually, or alongside in others in a potentially cheaper bulk operation.
//!
//! This module exposes one function to extract each type of `SignatureSet` from a `BeaconBlock`.
use bls::SignatureSet;
use ssz::DecodeError;
use std::borrow::Cow;
use tree_hash::TreeHash;
use types::{
    AbstractExecPayload, AggregateSignature, AttesterSlashingRef, BeaconBlockRef, BeaconState,
    BeaconStateError, ChainSpec, DepositData, Domain, Epoch, EthSpec, Fork, Hash256,
    InconsistentFork, IndexedAttestation, IndexedAttestationRef, ProposerSlashing, PublicKey,
    PublicKeyBytes, Signature, SignedAggregateAndProof, SignedBeaconBlock, SignedBeaconBlockHeader,
    SignedBlsToExecutionChange, SignedContributionAndProof, SignedRoot, SignedVoluntaryExit,
    SigningData, Slot, SyncAggregate, SyncAggregatorSelectionData, Unsigned,
};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, PartialEq, Clone)]
pub enum Error {
    /// Signature verification failed. The block is invalid.
    SignatureInvalid(DecodeError),
    /// There was an error attempting to read from a `BeaconState`. Block
    /// validity was not determined.
    BeaconStateError(BeaconStateError),
    /// Attempted to find the public key of a validator that does not exist. You cannot distinguish
    /// between an error and an invalid block in this case.
    ValidatorUnknown(u64),
    /// Attempted to find the public key of a validator that does not exist. You cannot distinguish
    /// between an error and an invalid block in this case.
    ValidatorPubkeyUnknown(PublicKeyBytes),
    /// The `BeaconBlock` has a `proposer_index` that does not match the index we computed locally.
    ///
    /// The block is invalid.
    IncorrectBlockProposer { block: u64, local_shuffling: u64 },
    /// The public keys supplied do not match the number of objects requiring keys. Block validity
    /// was not determined.
    MismatchedPublicKeyLen { pubkey_len: usize, other_len: usize },
    /// Pubkey decompression failed. The block is invalid.
    PublicKeyDecompressionFailed,
    /// The public key bytes stored in the `BeaconState` were not valid. This is a serious internal
    /// error.
    BadBlsBytes { validator_index: u64 },
    /// The block structure is not appropriate for the fork at `block.slot()`.
    InconsistentBlockFork(InconsistentFork),
}

impl From<BeaconStateError> for Error {
    fn from(e: BeaconStateError) -> Error {
        Error::BeaconStateError(e)
    }
}

/// Helper function to get a public key from a `state`.
pub fn get_pubkey_from_state<E>(
    state: &BeaconState<E>,
    validator_index: usize,
) -> Option<Cow<'_, PublicKey>>
where
    E: EthSpec,
{
    state
        .validators()
        .get(validator_index)
        .and_then(|v| {
            let pk: Option<PublicKey> = v.pubkey.decompress().ok();
            pk
        })
        .map(Cow::Owned)
}

/// A signature set that is valid if a block was signed by the expected block producer.
pub fn block_proposal_signature_set<'a, E, F, Payload: AbstractExecPayload<E>>(
    state: &'a BeaconState<E>,
    get_pubkey: F,
    signed_block: &'a SignedBeaconBlock<E, Payload>,
    block_root: Option<Hash256>,
    verified_proposer_index: Option<u64>,
    spec: &'a ChainSpec,
) -> Result<SignatureSet<'a>>
where
    E: EthSpec,
    F: Fn(usize) -> Option<Cow<'a, PublicKey>>,
{
    let block = signed_block.message();

    let proposer_index = if let Some(proposer_index) = verified_proposer_index {
        proposer_index
    } else {
        state.get_beacon_proposer_index(block.slot(), spec)? as u64
    };
    if proposer_index != block.proposer_index() {
        return Err(Error::IncorrectBlockProposer {
            block: block.proposer_index(),
            local_shuffling: proposer_index,
        });
    }

    block_proposal_signature_set_from_parts(
        signed_block,
        block_root,
        proposer_index,
        &state.fork(),
        state.genesis_validators_root(),
        get_pubkey,
        spec,
    )
}

/// A signature set that is valid if a block was signed by the expected block producer.
///
/// Unlike `block_proposal_signature_set` this does **not** check that the proposer index is
/// correct according to the shuffling. It should only be used if no suitable `BeaconState` is
/// available.
pub fn block_proposal_signature_set_from_parts<'a, E, F, Payload: AbstractExecPayload<E>>(
    signed_block: &'a SignedBeaconBlock<E, Payload>,
    block_root: Option<Hash256>,
    proposer_index: u64,
    fork: &Fork,
    genesis_validators_root: Hash256,
    get_pubkey: F,
    spec: &'a ChainSpec,
) -> Result<SignatureSet<'a>>
where
    E: EthSpec,
    F: Fn(usize) -> Option<Cow<'a, PublicKey>>,
{
    // Verify that the `SignedBeaconBlock` instantiation matches the fork at `signed_block.slot()`.
    signed_block
        .fork_name(spec)
        .map_err(Error::InconsistentBlockFork)?;

    let block = signed_block.message();
    let domain = spec.get_domain(
        block.slot().epoch(E::slots_per_epoch()),
        Domain::BeaconProposer,
        fork,
        genesis_validators_root,
    );

    let message = if let Some(root) = block_root {
        SigningData {
            object_root: root,
            domain,
        }
        .tree_hash_root()
    } else {
        block.signing_root(domain)
    };

    Ok(SignatureSet::single_pubkey(
        signed_block.signature(),
        get_pubkey(proposer_index as usize).ok_or(Error::ValidatorUnknown(proposer_index))?,
        message,
    ))
}

pub fn bls_execution_change_signature_set<'a, E: EthSpec>(
    state: &'a BeaconState<E>,
    signed_address_change: &'a SignedBlsToExecutionChange,
    spec: &'a ChainSpec,
) -> Result<SignatureSet<'a>> {
    let domain = spec.compute_domain(
        Domain::BlsToExecutionChange,
        spec.genesis_fork_version,
        state.genesis_validators_root(),
    );
    let message = signed_address_change.message.signing_root(domain);
    let signing_key = Cow::Owned(
        signed_address_change
            .message
            .from_bls_pubkey
            .decompress()
            .map_err(|_| Error::PublicKeyDecompressionFailed)?,
    );

    Ok(SignatureSet::single_pubkey(
        &signed_address_change.signature,
        signing_key,
        message,
    ))
}

/// A signature set that is valid if the block proposers randao reveal signature is correct.
pub fn randao_signature_set<'a, E, F, Payload: AbstractExecPayload<E>>(
    state: &'a BeaconState<E>,
    get_pubkey: F,
    block: BeaconBlockRef<'a, E, Payload>,
    verified_proposer_index: Option<u64>,
    spec: &'a ChainSpec,
) -> Result<SignatureSet<'a>>
where
    E: EthSpec,
    F: Fn(usize) -> Option<Cow<'a, PublicKey>>,
{
    let proposer_index = if let Some(proposer_index) = verified_proposer_index {
        proposer_index
    } else {
        state.get_beacon_proposer_index(block.slot(), spec)? as u64
    };

    let domain = spec.get_domain(
        block.slot().epoch(E::slots_per_epoch()),
        Domain::Randao,
        &state.fork(),
        state.genesis_validators_root(),
    );

    let message = block
        .slot()
        .epoch(E::slots_per_epoch())
        .signing_root(domain);

    Ok(SignatureSet::single_pubkey(
        block.body().randao_reveal(),
        get_pubkey(proposer_index as usize).ok_or(Error::ValidatorUnknown(proposer_index))?,
        message,
    ))
}

/// Returns two signature sets, one for each `BlockHeader` included in the `ProposerSlashing`.
pub fn proposer_slashing_signature_set<'a, E, F>(
    state: &'a BeaconState<E>,
    get_pubkey: F,
    proposer_slashing: &'a ProposerSlashing,
    spec: &'a ChainSpec,
) -> Result<(SignatureSet<'a>, SignatureSet<'a>)>
where
    E: EthSpec,
    F: Fn(usize) -> Option<Cow<'a, PublicKey>>,
{
    let proposer_index = proposer_slashing.signed_header_1.message.proposer_index as usize;

    Ok((
        block_header_signature_set(
            state,
            &proposer_slashing.signed_header_1,
            get_pubkey(proposer_index).ok_or(Error::ValidatorUnknown(proposer_index as u64))?,
            spec,
        ),
        block_header_signature_set(
            state,
            &proposer_slashing.signed_header_2,
            get_pubkey(proposer_index).ok_or(Error::ValidatorUnknown(proposer_index as u64))?,
            spec,
        ),
    ))
}

/// Returns a signature set that is valid if the given `pubkey` signed the `header`.
fn block_header_signature_set<'a, E: EthSpec>(
    state: &'a BeaconState<E>,
    signed_header: &'a SignedBeaconBlockHeader,
    pubkey: Cow<'a, PublicKey>,
    spec: &'a ChainSpec,
) -> SignatureSet<'a> {
    let domain = spec.get_domain(
        signed_header.message.slot.epoch(E::slots_per_epoch()),
        Domain::BeaconProposer,
        &state.fork(),
        state.genesis_validators_root(),
    );

    let message = signed_header.message.signing_root(domain);

    SignatureSet::single_pubkey(&signed_header.signature, pubkey, message)
}

/// Returns the signature set for the given `indexed_attestation`.
pub fn indexed_attestation_signature_set<'a, 'b, E, F>(
    state: &'a BeaconState<E>,
    get_pubkey: F,
    signature: &'a AggregateSignature,
    indexed_attestation: IndexedAttestationRef<'b, E>,
    spec: &'a ChainSpec,
) -> Result<SignatureSet<'a>>
where
    E: EthSpec,
    F: Fn(usize) -> Option<Cow<'a, PublicKey>>,
{
    let mut pubkeys = Vec::with_capacity(indexed_attestation.attesting_indices_len());
    for &validator_idx in indexed_attestation.attesting_indices_iter() {
        pubkeys.push(
            get_pubkey(validator_idx as usize).ok_or(Error::ValidatorUnknown(validator_idx))?,
        );
    }

    let domain = spec.get_domain(
        indexed_attestation.data().target.epoch,
        Domain::BeaconAttester,
        &state.fork(),
        state.genesis_validators_root(),
    );

    let message = indexed_attestation.data().signing_root(domain);

    Ok(SignatureSet::multiple_pubkeys(signature, pubkeys, message))
}

/// Returns the signature set for the given `indexed_attestation` but pubkeys are supplied directly
/// instead of from the state.
pub fn indexed_attestation_signature_set_from_pubkeys<'a, 'b, E, F>(
    get_pubkey: F,
    signature: &'a AggregateSignature,
    indexed_attestation: &'b IndexedAttestation<E>,
    fork: &Fork,
    genesis_validators_root: Hash256,
    spec: &'a ChainSpec,
) -> Result<SignatureSet<'a>>
where
    E: EthSpec,
    F: Fn(usize) -> Option<Cow<'a, PublicKey>>,
{
    let mut pubkeys = Vec::with_capacity(indexed_attestation.attesting_indices_len());
    for &validator_idx in indexed_attestation.attesting_indices_iter() {
        pubkeys.push(
            get_pubkey(validator_idx as usize).ok_or(Error::ValidatorUnknown(validator_idx))?,
        );
    }

    let domain = spec.get_domain(
        indexed_attestation.data().target.epoch,
        Domain::BeaconAttester,
        fork,
        genesis_validators_root,
    );

    let message = indexed_attestation.data().signing_root(domain);

    Ok(SignatureSet::multiple_pubkeys(signature, pubkeys, message))
}

/// Returns the signature set for the given `attester_slashing` and corresponding `pubkeys`.
pub fn attester_slashing_signature_sets<'a, E, F>(
    state: &'a BeaconState<E>,
    get_pubkey: F,
    attester_slashing: AttesterSlashingRef<'a, E>,
    spec: &'a ChainSpec,
) -> Result<(SignatureSet<'a>, SignatureSet<'a>)>
where
    E: EthSpec,
    F: Fn(usize) -> Option<Cow<'a, PublicKey>> + Clone,
{
    Ok((
        indexed_attestation_signature_set(
            state,
            get_pubkey.clone(),
            attester_slashing.attestation_1().signature(),
            attester_slashing.attestation_1(),
            spec,
        )?,
        indexed_attestation_signature_set(
            state,
            get_pubkey,
            attester_slashing.attestation_2().signature(),
            attester_slashing.attestation_2(),
            spec,
        )?,
    ))
}

/// Returns the BLS values in a `Deposit`, if they're all valid. Otherwise, returns `None`.
pub fn deposit_pubkey_signature_message(
    deposit_data: &DepositData,
    spec: &ChainSpec,
) -> Option<(PublicKey, Signature, Hash256)> {
    let pubkey = deposit_data.pubkey.decompress().ok()?;
    let signature = deposit_data.signature.decompress().ok()?;
    let domain = spec.get_deposit_domain();
    let message = deposit_data.as_deposit_message().signing_root(domain);
    Some((pubkey, signature, message))
}

/// Returns a signature set that is valid if the `SignedVoluntaryExit` was signed by the indicated
/// validator.
pub fn exit_signature_set<'a, E, F>(
    state: &'a BeaconState<E>,
    get_pubkey: F,
    signed_exit: &'a SignedVoluntaryExit,
    spec: &'a ChainSpec,
) -> Result<SignatureSet<'a>>
where
    E: EthSpec,
    F: Fn(usize) -> Option<Cow<'a, PublicKey>>,
{
    let exit = &signed_exit.message;
    let proposer_index = exit.validator_index as usize;

    let domain = if state.fork_name_unchecked().deneb_enabled() {
        // EIP-7044
        spec.compute_domain(
            Domain::VoluntaryExit,
            spec.capella_fork_version,
            state.genesis_validators_root(),
        )
    } else {
        spec.get_domain(
            exit.epoch,
            Domain::VoluntaryExit,
            &state.fork(),
            state.genesis_validators_root(),
        )
    };

    let message = exit.signing_root(domain);

    Ok(SignatureSet::single_pubkey(
        &signed_exit.signature,
        get_pubkey(proposer_index).ok_or(Error::ValidatorUnknown(proposer_index as u64))?,
        message,
    ))
}

pub fn signed_aggregate_selection_proof_signature_set<'a, E, F>(
    get_pubkey: F,
    signed_aggregate_and_proof: &'a SignedAggregateAndProof<E>,
    fork: &Fork,
    genesis_validators_root: Hash256,
    spec: &'a ChainSpec,
) -> Result<SignatureSet<'a>>
where
    E: EthSpec,
    F: Fn(usize) -> Option<Cow<'a, PublicKey>>,
{
    let slot = signed_aggregate_and_proof.message().aggregate().data().slot;

    let domain = spec.get_domain(
        slot.epoch(E::slots_per_epoch()),
        Domain::SelectionProof,
        fork,
        genesis_validators_root,
    );
    let message = slot.signing_root(domain);
    let signature = signed_aggregate_and_proof.message().selection_proof();
    let validator_index = signed_aggregate_and_proof.message().aggregator_index();
    Ok(SignatureSet::single_pubkey(
        signature,
        get_pubkey(validator_index as usize).ok_or(Error::ValidatorUnknown(validator_index))?,
        message,
    ))
}

pub fn signed_aggregate_signature_set<'a, E, F>(
    get_pubkey: F,
    signed_aggregate_and_proof: &'a SignedAggregateAndProof<E>,
    fork: &Fork,
    genesis_validators_root: Hash256,
    spec: &'a ChainSpec,
) -> Result<SignatureSet<'a>>
where
    E: EthSpec,
    F: Fn(usize) -> Option<Cow<'a, PublicKey>>,
{
    let target_epoch = signed_aggregate_and_proof
        .message()
        .aggregate()
        .data()
        .target
        .epoch;

    let domain = spec.get_domain(
        target_epoch,
        Domain::AggregateAndProof,
        fork,
        genesis_validators_root,
    );
    let message = signed_aggregate_and_proof.message().signing_root(domain);
    let signature = signed_aggregate_and_proof.signature();
    let validator_index = signed_aggregate_and_proof.message().aggregator_index();

    Ok(SignatureSet::single_pubkey(
        signature,
        get_pubkey(validator_index as usize).ok_or(Error::ValidatorUnknown(validator_index))?,
        message,
    ))
}

pub fn signed_sync_aggregate_selection_proof_signature_set<'a, E, F>(
    get_pubkey: F,
    signed_contribution_and_proof: &'a SignedContributionAndProof<E>,
    fork: &Fork,
    genesis_validators_root: Hash256,
    spec: &'a ChainSpec,
) -> Result<SignatureSet<'a>>
where
    E: EthSpec,
    F: Fn(usize) -> Option<Cow<'a, PublicKey>>,
{
    let slot = signed_contribution_and_proof.message.contribution.slot;

    let domain = spec.get_domain(
        slot.epoch(E::slots_per_epoch()),
        Domain::SyncCommitteeSelectionProof,
        fork,
        genesis_validators_root,
    );
    let selection_data = SyncAggregatorSelectionData {
        slot,
        subcommittee_index: signed_contribution_and_proof
            .message
            .contribution
            .subcommittee_index,
    };
    let message = selection_data.signing_root(domain);
    let signature = &signed_contribution_and_proof.message.selection_proof;
    let validator_index = signed_contribution_and_proof.message.aggregator_index;

    Ok(SignatureSet::single_pubkey(
        signature,
        get_pubkey(validator_index as usize).ok_or(Error::ValidatorUnknown(validator_index))?,
        message,
    ))
}

pub fn signed_sync_aggregate_signature_set<'a, E, F>(
    get_pubkey: F,
    signed_contribution_and_proof: &'a SignedContributionAndProof<E>,
    fork: &Fork,
    genesis_validators_root: Hash256,
    spec: &'a ChainSpec,
) -> Result<SignatureSet<'a>>
where
    E: EthSpec,
    F: Fn(usize) -> Option<Cow<'a, PublicKey>>,
{
    let epoch = signed_contribution_and_proof
        .message
        .contribution
        .slot
        .epoch(E::slots_per_epoch());

    let domain = spec.get_domain(
        epoch,
        Domain::ContributionAndProof,
        fork,
        genesis_validators_root,
    );
    let message = signed_contribution_and_proof.message.signing_root(domain);
    let signature = &signed_contribution_and_proof.signature;
    let validator_index = signed_contribution_and_proof.message.aggregator_index;

    Ok(SignatureSet::single_pubkey(
        signature,
        get_pubkey(validator_index as usize).ok_or(Error::ValidatorUnknown(validator_index))?,
        message,
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn sync_committee_contribution_signature_set_from_pubkeys<'a, E, F>(
    get_pubkey: F,
    pubkey_bytes: &[PublicKeyBytes],
    signature: &'a AggregateSignature,
    epoch: Epoch,
    beacon_block_root: Hash256,
    fork: &Fork,
    genesis_validators_root: Hash256,
    spec: &'a ChainSpec,
) -> Result<SignatureSet<'a>>
where
    E: EthSpec,
    F: Fn(&PublicKeyBytes) -> Option<Cow<'a, PublicKey>>,
{
    let mut pubkeys = Vec::with_capacity(E::SyncSubcommitteeSize::to_usize());
    for pubkey in pubkey_bytes {
        pubkeys.push(get_pubkey(pubkey).ok_or(Error::ValidatorPubkeyUnknown(*pubkey))?);
    }

    let domain = spec.get_domain(epoch, Domain::SyncCommittee, fork, genesis_validators_root);

    let message = beacon_block_root.signing_root(domain);

    Ok(SignatureSet::multiple_pubkeys(signature, pubkeys, message))
}

pub fn sync_committee_message_set_from_pubkeys<'a, E>(
    pubkey: Cow<'a, PublicKey>,
    signature: &'a AggregateSignature,
    epoch: Epoch,
    beacon_block_root: Hash256,
    fork: &Fork,
    genesis_validators_root: Hash256,
    spec: &'a ChainSpec,
) -> Result<SignatureSet<'a>>
where
    E: EthSpec,
{
    let domain = spec.get_domain(epoch, Domain::SyncCommittee, fork, genesis_validators_root);

    let message = beacon_block_root.signing_root(domain);

    Ok(SignatureSet::single_pubkey(signature, pubkey, message))
}

/// Signature set verifier for a block's `sync_aggregate` (Altair and later).
///
/// The `slot` should be the slot of the block that the sync aggregate is included in, which may be
/// different from `state.slot()`. The `block_root` should be the block root that the sync aggregate
/// signs over. It's passed in rather than extracted from the `state` because when verifying a batch
/// of blocks the `state` will not yet have had the blocks applied.
///
/// Returns `Ok(None)` in the case where `sync_aggregate` has 0 signatures. The spec
/// uses a separate function `eth2_fast_aggregate_verify` for this, but we can equivalently
/// check the exceptional case eagerly and do a `fast_aggregate_verify` in the case where the
/// check fails (by returning `Some(signature_set)`).
pub fn sync_aggregate_signature_set<'a, E, D>(
    decompressor: D,
    sync_aggregate: &'a SyncAggregate<E>,
    slot: Slot,
    block_root: Hash256,
    state: &'a BeaconState<E>,
    spec: &ChainSpec,
) -> Result<Option<SignatureSet<'a>>>
where
    E: EthSpec,
    D: Fn(&'a PublicKeyBytes) -> Option<Cow<'a, PublicKey>>,
{
    // Allow the point at infinity to count as a signature for 0 validators as per
    // `eth2_fast_aggregate_verify` from the spec.
    if sync_aggregate.sync_committee_bits.is_zero()
        && sync_aggregate.sync_committee_signature.is_infinity()
    {
        return Ok(None);
    }

    let committee_pubkeys = &state
        .get_built_sync_committee(slot.epoch(E::slots_per_epoch()), spec)?
        .pubkeys;

    let participant_pubkeys = committee_pubkeys
        .iter()
        .zip(sync_aggregate.sync_committee_bits.iter())
        .filter_map(|(pubkey, bit)| {
            if bit {
                Some(decompressor(pubkey))
            } else {
                None
            }
        })
        .collect::<Option<Vec<_>>>()
        .ok_or(Error::PublicKeyDecompressionFailed)?;

    let previous_slot = slot.saturating_sub(1u64);

    let domain = spec.get_domain(
        previous_slot.epoch(E::slots_per_epoch()),
        Domain::SyncCommittee,
        &state.fork(),
        state.genesis_validators_root(),
    );

    let message = SigningData {
        object_root: block_root,
        domain,
    }
    .tree_hash_root();

    Ok(Some(SignatureSet::multiple_pubkeys(
        &sync_aggregate.sync_committee_signature,
        participant_pubkeys,
        message,
    )))
}

/// A signature set that is valid if an execution payload bid was signed by the builder.
///
/// This checks the `SignedExecutionPayloadBid` signature against the builder's public key
/// using the `DOMAIN_BEACON_BUILDER` domain.
pub fn execution_payload_bid_signature_set<'a, E, F>(
    state: &'a BeaconState<E>,
    get_builder_pubkey: F,
    signed_bid: &'a types::SignedExecutionPayloadBid<E>,
    spec: &'a ChainSpec,
) -> Result<SignatureSet<'a>>
where
    E: EthSpec,
    F: Fn(u64) -> Option<Cow<'a, PublicKey>>,
{
    let builder_index = signed_bid.message.builder_index;

    let builder_pubkey =
        get_builder_pubkey(builder_index).ok_or(Error::ValidatorUnknown(builder_index))?;

    let epoch = signed_bid.message.slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(
        epoch,
        Domain::BeaconBuilder,
        &state.fork(),
        state.genesis_validators_root(),
    );

    let message = signed_bid.message.signing_root(domain);

    Ok(SignatureSet::single_pubkey(
        &signed_bid.signature,
        builder_pubkey,
        message,
    ))
}

/// A signature set that is valid if a payload attestation was signed by the PTC members.
///
/// This checks the aggregated `PayloadAttestation` signature against the PTC members' public keys
/// using the `DOMAIN_PTC_ATTESTER` domain.
pub fn payload_attestation_signature_set<'a, E, F>(
    state: &'a BeaconState<E>,
    get_pubkey: F,
    attestation: &'a types::PayloadAttestation<E>,
    attesting_indices: &'a [u64],
    spec: &'a ChainSpec,
) -> Result<SignatureSet<'a>>
where
    E: EthSpec,
    F: Fn(usize) -> Option<Cow<'a, PublicKey>>,
{
    // Collect public keys for all attesting validators
    let mut pubkeys = Vec::with_capacity(attesting_indices.len());
    for &validator_index in attesting_indices.iter() {
        let pubkey =
            get_pubkey(validator_index as usize).ok_or(Error::ValidatorUnknown(validator_index))?;
        pubkeys.push(pubkey);
    }

    let epoch = attestation.data.slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(
        epoch,
        Domain::PtcAttester,
        &state.fork(),
        state.genesis_validators_root(),
    );

    let message = attestation.data.signing_root(domain);

    Ok(SignatureSet::multiple_pubkeys(
        &attestation.signature,
        pubkeys,
        message,
    ))
}

/// A signature set that is valid if an execution payload envelope was signed by the builder.
///
/// This checks the `SignedExecutionPayloadEnvelope` signature against the builder's public key
/// using the `DOMAIN_BEACON_BUILDER` domain.
pub fn execution_payload_envelope_signature_set<'a, E, F>(
    state: &'a BeaconState<E>,
    get_builder_pubkey: F,
    signed_envelope: &'a types::SignedExecutionPayloadEnvelope<E>,
    spec: &'a ChainSpec,
) -> Result<SignatureSet<'a>>
where
    E: EthSpec,
    F: Fn(u64) -> Option<Cow<'a, PublicKey>>,
{
    let builder_index = signed_envelope.message.builder_index;

    let builder_pubkey =
        get_builder_pubkey(builder_index).ok_or(Error::ValidatorUnknown(builder_index))?;

    let epoch = signed_envelope.message.slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(
        epoch,
        Domain::BeaconBuilder,
        &state.fork(),
        state.genesis_validators_root(),
    );

    let message = signed_envelope.message.signing_root(domain);

    Ok(SignatureSet::single_pubkey(
        &signed_envelope.signature,
        builder_pubkey,
        message,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssz_types::BitVector;
    use std::sync::Arc;
    use types::{
        BeaconBlockHeader, BeaconStateGloas, BuilderPendingPayment, BuilderPubkeyCache,
        CACHED_EPOCHS, Checkpoint, CommitteeCache, Epoch, ExecutionBlockHash, ExecutionPayloadBid,
        ExecutionPayloadEnvelope, ExitCache, FixedVector, Fork, List, MinimalEthSpec,
        PayloadAttestation, PayloadAttestationData, ProgressiveBalancesCache, PubkeyCache,
        PublicKeyBytes, SignedExecutionPayloadBid, SignedExecutionPayloadEnvelope, SlashingsCache,
        SyncCommittee, Unsigned, Vector,
    };

    type E = MinimalEthSpec;

    fn gloas_spec() -> ChainSpec {
        let mut spec = E::default_spec();
        spec.altair_fork_epoch = Some(Epoch::new(0));
        spec.bellatrix_fork_epoch = Some(Epoch::new(0));
        spec.capella_fork_epoch = Some(Epoch::new(0));
        spec.deneb_fork_epoch = Some(Epoch::new(0));
        spec.electra_fork_epoch = Some(Epoch::new(0));
        spec.fulu_fork_epoch = Some(Epoch::new(0));
        spec.gloas_fork_epoch = Some(Epoch::new(0));
        spec
    }

    /// Extract error from Result when Ok type doesn't implement Debug.
    fn extract_err<T>(result: std::result::Result<T, Error>) -> Error {
        match result {
            Err(e) => e,
            Ok(_) => panic!("expected Err, got Ok"),
        }
    }

    /// Build a minimal Gloas state at slot 8 (epoch 1).
    /// Returns keypairs for signing tests.
    fn make_gloas_state() -> (BeaconState<E>, ChainSpec, Vec<bls::Keypair>) {
        let spec = gloas_spec();
        let slot = Slot::new(8);

        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

        let keypairs = types::test_utils::generate_deterministic_keypairs(2);

        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                PublicKeyBytes::empty();
                <E as EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: PublicKeyBytes::empty(),
        });

        let state = BeaconState::Gloas(BeaconStateGloas {
            genesis_time: 0,
            genesis_validators_root: Hash256::repeat_byte(0xAA),
            slot,
            fork: Fork {
                previous_version: spec.fulu_fork_version,
                current_version: spec.gloas_fork_version,
                epoch: Epoch::new(0),
            },
            latest_block_header: BeaconBlockHeader {
                slot: slot.saturating_sub(1u64),
                proposer_index: 0,
                parent_root: Hash256::ZERO,
                state_root: Hash256::ZERO,
                body_root: Hash256::ZERO,
            },
            block_roots: Vector::new(vec![Hash256::ZERO; slots_per_hist]).unwrap(),
            state_roots: Vector::new(vec![Hash256::ZERO; slots_per_hist]).unwrap(),
            historical_roots: List::default(),
            eth1_data: types::Eth1Data::default(),
            eth1_data_votes: List::default(),
            eth1_deposit_index: 0,
            validators: List::default(),
            balances: List::default(),
            randao_mixes: Vector::new(vec![Hash256::ZERO; epochs_per_vector]).unwrap(),
            slashings: Vector::new(vec![0; epochs_per_slash]).unwrap(),
            previous_epoch_participation: List::default(),
            current_epoch_participation: List::default(),
            justification_bits: BitVector::new(),
            previous_justified_checkpoint: Checkpoint::default(),
            current_justified_checkpoint: Checkpoint::default(),
            finalized_checkpoint: Checkpoint::default(),
            inactivity_scores: List::default(),
            current_sync_committee: sync_committee.clone(),
            next_sync_committee: sync_committee,
            latest_execution_payload_bid: ExecutionPayloadBid::default(),
            next_withdrawal_index: 0,
            next_withdrawal_validator_index: 0,
            historical_summaries: List::default(),
            deposit_requests_start_index: u64::MAX,
            deposit_balance_to_consume: 0,
            exit_balance_to_consume: 0,
            earliest_exit_epoch: Epoch::new(0),
            consolidation_balance_to_consume: 0,
            earliest_consolidation_epoch: Epoch::new(0),
            pending_deposits: List::default(),
            pending_partial_withdrawals: List::default(),
            pending_consolidations: List::default(),
            proposer_lookahead: Vector::new(vec![
                0u64;
                <E as EthSpec>::ProposerLookaheadSlots::to_usize()
            ])
            .unwrap(),
            builders: List::default(),
            next_withdrawal_builder_index: 0,
            execution_payload_availability: BitVector::new(),
            builder_pending_payments: Vector::new(vec![
                BuilderPendingPayment::default();
                E::builder_pending_payments_limit()
            ])
            .unwrap(),
            builder_pending_withdrawals: List::default(),
            latest_block_hash: ExecutionBlockHash::zero(),
            payload_expected_withdrawals: List::default(),
            total_active_balance: None,
            progressive_balances_cache: ProgressiveBalancesCache::default(),
            committee_caches: <[Arc<CommitteeCache>; CACHED_EPOCHS]>::default(),
            pubkey_cache: PubkeyCache::default(),
            builder_pubkey_cache: BuilderPubkeyCache::default(),
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: types::EpochCache::default(),
        });

        (state, spec, keypairs)
    }

    // ──────── execution_payload_bid_signature_set tests ────────

    #[test]
    fn bid_signature_set_unknown_builder_returns_error() {
        let (state, spec, _keypairs) = make_gloas_state();
        let signed_bid = SignedExecutionPayloadBid::<E>::empty();

        let err = extract_err(execution_payload_bid_signature_set(
            &state,
            |_| None,
            &signed_bid,
            &spec,
        ));
        assert_eq!(err, Error::ValidatorUnknown(0));
    }

    #[test]
    fn bid_signature_set_unknown_high_index_returns_error() {
        let (state, spec, _keypairs) = make_gloas_state();
        let mut signed_bid = SignedExecutionPayloadBid::<E>::empty();
        signed_bid.message.builder_index = 99999;

        let err = extract_err(execution_payload_bid_signature_set(
            &state,
            |_| None,
            &signed_bid,
            &spec,
        ));
        assert_eq!(err, Error::ValidatorUnknown(99999));
    }

    #[test]
    fn bid_signature_set_valid_signature_verifies() {
        let (state, spec, keypairs) = make_gloas_state();

        let bid = ExecutionPayloadBid::<E> {
            builder_index: 0,
            slot: Slot::new(8),
            value: 100,
            ..Default::default()
        };

        let epoch = bid.slot.epoch(E::slots_per_epoch());
        let domain = spec.get_domain(
            epoch,
            Domain::BeaconBuilder,
            &state.fork(),
            state.genesis_validators_root(),
        );
        let signing_root = bid.signing_root(domain);
        let signature = keypairs[0].sk.sign(signing_root);

        let signed_bid = SignedExecutionPayloadBid {
            message: bid,
            signature,
        };

        let pubkey = keypairs[0].pk.clone();
        let sig_set = execution_payload_bid_signature_set(
            &state,
            |idx| {
                if idx == 0 {
                    Some(Cow::Owned(pubkey.clone()))
                } else {
                    None
                }
            },
            &signed_bid,
            &spec,
        )
        .expect("should succeed");

        assert!(sig_set.verify());
    }

    #[test]
    fn bid_signature_set_wrong_key_fails_verification() {
        let (state, spec, keypairs) = make_gloas_state();

        let bid = ExecutionPayloadBid::<E> {
            builder_index: 0,
            slot: Slot::new(8),
            ..Default::default()
        };

        let epoch = bid.slot.epoch(E::slots_per_epoch());
        let domain = spec.get_domain(
            epoch,
            Domain::BeaconBuilder,
            &state.fork(),
            state.genesis_validators_root(),
        );
        let signing_root = bid.signing_root(domain);

        // Sign with key 0 but verify with key 1
        let signature = keypairs[0].sk.sign(signing_root);
        let signed_bid = SignedExecutionPayloadBid {
            message: bid,
            signature,
        };

        let wrong_pubkey = keypairs[1].pk.clone();
        let sig_set = execution_payload_bid_signature_set(
            &state,
            |idx| {
                if idx == 0 {
                    Some(Cow::Owned(wrong_pubkey.clone()))
                } else {
                    None
                }
            },
            &signed_bid,
            &spec,
        )
        .expect("should succeed constructing set");

        assert!(!sig_set.verify());
    }

    #[test]
    fn bid_signature_set_uses_beacon_builder_domain() {
        let (state, spec, keypairs) = make_gloas_state();

        let bid = ExecutionPayloadBid::<E> {
            builder_index: 0,
            slot: Slot::new(8),
            ..Default::default()
        };

        // Sign with WRONG domain (BeaconProposer instead of BeaconBuilder)
        let epoch = bid.slot.epoch(E::slots_per_epoch());
        let wrong_domain = spec.get_domain(
            epoch,
            Domain::BeaconProposer,
            &state.fork(),
            state.genesis_validators_root(),
        );
        let signature = keypairs[0].sk.sign(bid.signing_root(wrong_domain));
        let signed_bid = SignedExecutionPayloadBid {
            message: bid,
            signature,
        };

        let pubkey = keypairs[0].pk.clone();
        let sig_set = execution_payload_bid_signature_set(
            &state,
            |idx| {
                if idx == 0 {
                    Some(Cow::Owned(pubkey.clone()))
                } else {
                    None
                }
            },
            &signed_bid,
            &spec,
        )
        .expect("should succeed constructing set");

        assert!(!sig_set.verify());
    }

    // ──────── payload_attestation_signature_set tests ────────

    #[test]
    fn payload_attestation_unknown_validator_returns_error() {
        let (state, spec, _keypairs) = make_gloas_state();
        let attestation = PayloadAttestation::<E>::empty();

        let err = extract_err(payload_attestation_signature_set(
            &state,
            |_| None,
            &attestation,
            &[42],
            &spec,
        ));
        assert_eq!(err, Error::ValidatorUnknown(42));
    }

    #[test]
    fn payload_attestation_multiple_keys_one_unknown_returns_error() {
        let (state, spec, keypairs) = make_gloas_state();
        let attestation = PayloadAttestation::<E>::empty();

        let pubkey0 = keypairs[0].pk.clone();
        let err = extract_err(payload_attestation_signature_set(
            &state,
            |idx| {
                if idx == 0 {
                    Some(Cow::Owned(pubkey0.clone()))
                } else {
                    None
                }
            },
            &attestation,
            &[0, 1], // validator 1 unknown
            &spec,
        ));
        assert_eq!(err, Error::ValidatorUnknown(1));
    }

    #[test]
    fn payload_attestation_valid_single_signer_verifies() {
        let (state, spec, keypairs) = make_gloas_state();

        let data = PayloadAttestationData {
            beacon_block_root: Hash256::repeat_byte(0xBB),
            slot: Slot::new(8),
            payload_present: true,
            blob_data_available: false,
        };

        let epoch = data.slot.epoch(E::slots_per_epoch());
        let domain = spec.get_domain(
            epoch,
            Domain::PtcAttester,
            &state.fork(),
            state.genesis_validators_root(),
        );
        let signing_root = data.signing_root(domain);

        let sig = keypairs[0].sk.sign(signing_root);
        let mut agg_sig = bls::AggregateSignature::infinity();
        agg_sig.add_assign(&sig);

        let attestation = PayloadAttestation {
            aggregation_bits: BitVector::new(),
            data,
            signature: agg_sig,
        };

        let pubkey0 = keypairs[0].pk.clone();
        let sig_set = payload_attestation_signature_set(
            &state,
            |idx| {
                if idx == 0 {
                    Some(Cow::Owned(pubkey0.clone()))
                } else {
                    None
                }
            },
            &attestation,
            &[0],
            &spec,
        )
        .expect("should succeed");

        assert!(sig_set.verify());
    }

    #[test]
    fn payload_attestation_uses_ptc_attester_domain() {
        let (state, spec, keypairs) = make_gloas_state();

        let data = PayloadAttestationData {
            beacon_block_root: Hash256::repeat_byte(0xBB),
            slot: Slot::new(8),
            payload_present: true,
            blob_data_available: false,
        };

        // Sign with wrong domain (BeaconProposer)
        let epoch = data.slot.epoch(E::slots_per_epoch());
        let wrong_domain = spec.get_domain(
            epoch,
            Domain::BeaconProposer,
            &state.fork(),
            state.genesis_validators_root(),
        );
        let sig = keypairs[0].sk.sign(data.signing_root(wrong_domain));
        let mut agg_sig = bls::AggregateSignature::infinity();
        agg_sig.add_assign(&sig);

        let attestation = PayloadAttestation {
            aggregation_bits: BitVector::new(),
            data,
            signature: agg_sig,
        };

        let pubkey0 = keypairs[0].pk.clone();
        let sig_set = payload_attestation_signature_set(
            &state,
            |idx| {
                if idx == 0 {
                    Some(Cow::Owned(pubkey0.clone()))
                } else {
                    None
                }
            },
            &attestation,
            &[0],
            &spec,
        )
        .expect("should succeed constructing set");

        assert!(!sig_set.verify());
    }

    // ──────── execution_payload_envelope_signature_set tests ────────

    #[test]
    fn envelope_signature_set_unknown_builder_returns_error() {
        let (state, spec, _keypairs) = make_gloas_state();
        let signed_envelope = SignedExecutionPayloadEnvelope::<E>::empty();

        let err = extract_err(execution_payload_envelope_signature_set(
            &state,
            |_| None,
            &signed_envelope,
            &spec,
        ));
        assert_eq!(err, Error::ValidatorUnknown(0));
    }

    #[test]
    fn envelope_signature_set_valid_signature_verifies() {
        let (state, spec, keypairs) = make_gloas_state();

        let mut envelope = ExecutionPayloadEnvelope::<E>::empty();
        envelope.builder_index = 1;
        envelope.slot = Slot::new(8);
        envelope.beacon_block_root = Hash256::repeat_byte(0xDD);

        let epoch = envelope.slot.epoch(E::slots_per_epoch());
        let domain = spec.get_domain(
            epoch,
            Domain::BeaconBuilder,
            &state.fork(),
            state.genesis_validators_root(),
        );
        let signing_root = envelope.signing_root(domain);
        let signature = keypairs[1].sk.sign(signing_root);

        let signed_envelope = SignedExecutionPayloadEnvelope {
            message: envelope,
            signature,
        };

        let pubkey1 = keypairs[1].pk.clone();
        let sig_set = execution_payload_envelope_signature_set(
            &state,
            |idx| {
                if idx == 1 {
                    Some(Cow::Owned(pubkey1.clone()))
                } else {
                    None
                }
            },
            &signed_envelope,
            &spec,
        )
        .expect("should succeed");

        assert!(sig_set.verify());
    }

    #[test]
    fn envelope_signature_set_wrong_key_fails_verification() {
        let (state, spec, keypairs) = make_gloas_state();

        let mut envelope = ExecutionPayloadEnvelope::<E>::empty();
        envelope.builder_index = 1;
        envelope.slot = Slot::new(8);

        let epoch = envelope.slot.epoch(E::slots_per_epoch());
        let domain = spec.get_domain(
            epoch,
            Domain::BeaconBuilder,
            &state.fork(),
            state.genesis_validators_root(),
        );
        let signing_root = envelope.signing_root(domain);

        // Sign with key 0 but look up key for builder_index=1
        let signature = keypairs[0].sk.sign(signing_root);
        let signed_envelope = SignedExecutionPayloadEnvelope {
            message: envelope,
            signature,
        };

        let pubkey1 = keypairs[1].pk.clone();
        let sig_set = execution_payload_envelope_signature_set(
            &state,
            |idx| {
                if idx == 1 {
                    Some(Cow::Owned(pubkey1.clone()))
                } else {
                    None
                }
            },
            &signed_envelope,
            &spec,
        )
        .expect("should succeed constructing set");

        assert!(!sig_set.verify());
    }

    #[test]
    fn envelope_signature_set_uses_beacon_builder_domain() {
        let (state, spec, keypairs) = make_gloas_state();

        let mut envelope = ExecutionPayloadEnvelope::<E>::empty();
        envelope.builder_index = 0;
        envelope.slot = Slot::new(8);

        // Sign with wrong domain (PtcAttester instead of BeaconBuilder)
        let epoch = envelope.slot.epoch(E::slots_per_epoch());
        let wrong_domain = spec.get_domain(
            epoch,
            Domain::PtcAttester,
            &state.fork(),
            state.genesis_validators_root(),
        );
        let signature = keypairs[0].sk.sign(envelope.signing_root(wrong_domain));
        let signed_envelope = SignedExecutionPayloadEnvelope {
            message: envelope,
            signature,
        };

        let pubkey0 = keypairs[0].pk.clone();
        let sig_set = execution_payload_envelope_signature_set(
            &state,
            |idx| {
                if idx == 0 {
                    Some(Cow::Owned(pubkey0.clone()))
                } else {
                    None
                }
            },
            &signed_envelope,
            &spec,
        )
        .expect("should succeed constructing set");

        assert!(!sig_set.verify());
    }

    // ──────── Gloas signature set edge case tests (run 204) ────────

    #[test]
    fn payload_attestation_multiple_valid_signers_verifies() {
        // Aggregate signature from two PTC members should verify when both
        // individual signatures are combined. This tests the multi-pubkey
        // aggregation path in payload_attestation_signature_set, which all
        // prior tests skip by using a single signer.
        let (state, spec, keypairs) = make_gloas_state();

        let data = PayloadAttestationData {
            beacon_block_root: Hash256::repeat_byte(0xCC),
            slot: Slot::new(8),
            payload_present: true,
            blob_data_available: true,
        };

        let epoch = data.slot.epoch(E::slots_per_epoch());
        let domain = spec.get_domain(
            epoch,
            Domain::PtcAttester,
            &state.fork(),
            state.genesis_validators_root(),
        );
        let signing_root = data.signing_root(domain);

        // Both signers contribute to the aggregate
        let sig0 = keypairs[0].sk.sign(signing_root);
        let sig1 = keypairs[1].sk.sign(signing_root);
        let mut agg_sig = bls::AggregateSignature::infinity();
        agg_sig.add_assign(&sig0);
        agg_sig.add_assign(&sig1);

        let attestation = PayloadAttestation {
            aggregation_bits: BitVector::new(),
            data,
            signature: agg_sig,
        };

        let pk0 = keypairs[0].pk.clone();
        let pk1 = keypairs[1].pk.clone();
        let sig_set = payload_attestation_signature_set(
            &state,
            |idx| match idx {
                0 => Some(Cow::Owned(pk0.clone())),
                1 => Some(Cow::Owned(pk1.clone())),
                _ => None,
            },
            &attestation,
            &[0, 1],
            &spec,
        )
        .expect("should succeed");

        assert!(
            sig_set.verify(),
            "aggregate of two valid signer signatures should verify"
        );
    }

    #[test]
    fn payload_attestation_wrong_data_field_invalidates() {
        // A payload attestation signed with payload_present=true should
        // NOT verify when the data is changed to payload_present=false.
        // This confirms that the PayloadAttestationData signing_root covers
        // the payload_present field — a critical property since PTC members
        // vote on payload timeliness and a bit-flip would reverse the vote.
        let (state, spec, keypairs) = make_gloas_state();

        let mut data = PayloadAttestationData {
            beacon_block_root: Hash256::repeat_byte(0xEE),
            slot: Slot::new(8),
            payload_present: true,
            blob_data_available: false,
        };

        let epoch = data.slot.epoch(E::slots_per_epoch());
        let domain = spec.get_domain(
            epoch,
            Domain::PtcAttester,
            &state.fork(),
            state.genesis_validators_root(),
        );
        let signing_root = data.signing_root(domain);
        let sig = keypairs[0].sk.sign(signing_root);
        let mut agg_sig = bls::AggregateSignature::infinity();
        agg_sig.add_assign(&sig);

        // Flip payload_present AFTER signing
        data.payload_present = false;

        let attestation = PayloadAttestation {
            aggregation_bits: BitVector::new(),
            data,
            signature: agg_sig,
        };

        let pk0 = keypairs[0].pk.clone();
        let sig_set = payload_attestation_signature_set(
            &state,
            |idx| {
                if idx == 0 {
                    Some(Cow::Owned(pk0.clone()))
                } else {
                    None
                }
            },
            &attestation,
            &[0],
            &spec,
        )
        .expect("should succeed constructing set");

        assert!(
            !sig_set.verify(),
            "attestation with flipped payload_present should fail verification"
        );
    }

    #[test]
    fn bid_signature_at_different_epoch_fails_cross_epoch() {
        // A bid signed at epoch 1 (slot 8) should NOT verify when the
        // signature set is constructed at epoch 2 (slot 16), because the
        // domain includes the epoch. This confirms that moving a valid
        // bid to a different slot/epoch invalidates its signature.
        let (state, spec, keypairs) = make_gloas_state();

        let bid_epoch1 = ExecutionPayloadBid::<E> {
            builder_index: 0,
            slot: Slot::new(8), // epoch 1
            value: 42,
            ..Default::default()
        };

        // Sign with epoch 1 domain
        let epoch1 = bid_epoch1.slot.epoch(E::slots_per_epoch());
        let domain1 = spec.get_domain(
            epoch1,
            Domain::BeaconBuilder,
            &state.fork(),
            state.genesis_validators_root(),
        );
        let signature = keypairs[0].sk.sign(bid_epoch1.signing_root(domain1));

        // Now create a bid at epoch 2 with the same signature
        let bid_epoch2 = ExecutionPayloadBid::<E> {
            builder_index: 0,
            slot: Slot::new(16), // epoch 2
            value: 42,
            ..Default::default()
        };
        let signed_bid = SignedExecutionPayloadBid {
            message: bid_epoch2,
            signature,
        };

        let pubkey = keypairs[0].pk.clone();
        let sig_set = execution_payload_bid_signature_set(
            &state,
            |idx| {
                if idx == 0 {
                    Some(Cow::Owned(pubkey.clone()))
                } else {
                    None
                }
            },
            &signed_bid,
            &spec,
        )
        .expect("should succeed constructing set");

        assert!(
            !sig_set.verify(),
            "bid signed at epoch 1 should not verify when slot is epoch 2"
        );
    }

    #[test]
    fn bid_signature_modified_value_invalidates() {
        // A valid bid signature should fail verification if any message
        // field is modified after signing. This tests that the signing_root
        // covers all bid fields — a modified `value` changes the root.
        let (state, spec, keypairs) = make_gloas_state();

        let mut bid = ExecutionPayloadBid::<E> {
            builder_index: 0,
            slot: Slot::new(8),
            value: 100,
            ..Default::default()
        };

        let epoch = bid.slot.epoch(E::slots_per_epoch());
        let domain = spec.get_domain(
            epoch,
            Domain::BeaconBuilder,
            &state.fork(),
            state.genesis_validators_root(),
        );
        let signature = keypairs[0].sk.sign(bid.signing_root(domain));

        // Tamper with the bid value after signing
        bid.value = 999;

        let signed_bid = SignedExecutionPayloadBid {
            message: bid,
            signature,
        };

        let pubkey = keypairs[0].pk.clone();
        let sig_set = execution_payload_bid_signature_set(
            &state,
            |idx| {
                if idx == 0 {
                    Some(Cow::Owned(pubkey.clone()))
                } else {
                    None
                }
            },
            &signed_bid,
            &spec,
        )
        .expect("should succeed constructing set");

        assert!(
            !sig_set.verify(),
            "bid with modified value should fail signature verification"
        );
    }

    #[test]
    fn bid_and_envelope_same_builder_same_domain_different_roots() {
        // Both bids and envelopes use DOMAIN_BEACON_BUILDER, so a signature
        // valid for a bid MUST NOT verify as an envelope (and vice versa),
        // because the messages have different SSZ tree roots. This tests
        // cross-type signature non-transferability.
        let (state, spec, keypairs) = make_gloas_state();

        let bid = ExecutionPayloadBid::<E> {
            builder_index: 0,
            slot: Slot::new(8),
            value: 50,
            ..Default::default()
        };

        let epoch = bid.slot.epoch(E::slots_per_epoch());
        let domain = spec.get_domain(
            epoch,
            Domain::BeaconBuilder,
            &state.fork(),
            state.genesis_validators_root(),
        );

        // Sign the BID
        let bid_signature = keypairs[0].sk.sign(bid.signing_root(domain));

        // Create an envelope with the SAME builder_index and slot, using the BID's signature
        let mut envelope = ExecutionPayloadEnvelope::<E>::empty();
        envelope.builder_index = 0;
        envelope.slot = Slot::new(8);

        let signed_envelope = SignedExecutionPayloadEnvelope {
            message: envelope,
            signature: bid_signature,
        };

        let pubkey = keypairs[0].pk.clone();
        let sig_set = execution_payload_envelope_signature_set(
            &state,
            |idx| {
                if idx == 0 {
                    Some(Cow::Owned(pubkey.clone()))
                } else {
                    None
                }
            },
            &signed_envelope,
            &spec,
        )
        .expect("should succeed constructing set");

        assert!(
            !sig_set.verify(),
            "bid signature should not verify as an envelope signature (different signing roots)"
        );
    }
}
