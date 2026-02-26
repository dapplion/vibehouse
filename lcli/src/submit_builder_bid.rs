//! # submit-builder-bid
//!
//! Submit an external builder bid to a beacon node via `POST /eth/v1/builder/bids`.
//!
//! This command first signs and submits proposer preferences for the target slot
//! (via `POST /eth/v1/beacon/pool/proposer_preferences`), then submits the bid.
//! The bid's fee_recipient and gas_limit are taken from flags or set to defaults;
//! the preferences submitted use the same values so validation passes.
//!
//! The builder keypair is deterministic: keypair at index `validator_count + builder_index`.
//! Run the beacon node with `--genesis-builders N` to pre-register builders at genesis.
//!
//! The proposer's keypair for signing preferences is the interop keypair at the
//! validator_index returned by the proposer duties API for the target slot.
//!
//! ## Example
//!
//! ```ignore
//! lcli submit-builder-bid \
//!     --spec minimal \
//!     --beacon-url http://localhost:5052 \
//!     --builder-index 0 \
//!     --validator-count 64 \
//!     --bid-value 1000000000
//! ```
use clap::ArgMatches;
use clap_utils::{parse_optional, parse_required};
use environment::Environment;
use eth2::{BeaconNodeHttpClient, SensitiveUrl, Timeouts, types::StateId};
use eth2_network_config::Eth2NetworkConfig;
use std::time::Duration;
use tracing::info;
use types::{
    Address, ChainSpec, Domain, Epoch, EthSpec, ExecutionBlockHash, ExecutionPayloadBid, Hash256,
    ProposerPreferences, SignedExecutionPayloadBid, SignedProposerPreferences, SignedRoot, Slot,
    test_utils::generate_deterministic_keypairs,
};

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
/// Default gas limit for proposer preferences + bid.
const DEFAULT_GAS_LIMIT: u64 = 30_000_000;

pub fn run<E: EthSpec>(
    env: Environment<E>,
    network_config: Eth2NetworkConfig,
    matches: &ArgMatches,
) -> Result<(), String> {
    let spec = network_config.chain_spec::<E>()?;
    let executor = env.core_context().executor;

    let beacon_url: SensitiveUrl = parse_required(matches, "beacon-url")?;
    let builder_index: u64 = parse_required(matches, "builder-index")?;
    let validator_count: usize = parse_required(matches, "validator-count")?;
    let bid_value: u64 = parse_required(matches, "bid-value")?;
    let slot_override: Option<u64> = parse_optional(matches, "slot")?;
    let fee_recipient_hex: Option<String> = parse_optional(matches, "fee-recipient")?;
    let block_hash_hex: Option<String> = parse_optional(matches, "block-hash")?;
    let gas_limit: u64 = parse_optional(matches, "gas-limit")?.unwrap_or(DEFAULT_GAS_LIMIT);

    let fee_recipient = if let Some(hex) = fee_recipient_hex {
        let bytes = hex::decode(hex.trim_start_matches("0x"))
            .map_err(|e| format!("Invalid fee-recipient hex: {e}"))?;
        if bytes.len() != 20 {
            return Err("fee-recipient must be 20 bytes".to_string());
        }
        let mut arr = [0u8; 20];
        arr.copy_from_slice(&bytes);
        Address::from(arr)
    } else {
        Address::ZERO
    };

    let block_hash = if let Some(hex) = block_hash_hex {
        let bytes = hex::decode(hex.trim_start_matches("0x"))
            .map_err(|e| format!("Invalid block-hash hex: {e}"))?;
        if bytes.len() != 32 {
            return Err("block-hash must be 32 bytes".to_string());
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        ExecutionBlockHash::from_root(Hash256::from_slice(&arr))
    } else {
        ExecutionBlockHash::zero()
    };

    // Generate the builder keypair deterministically.
    // Builders use keypairs at indices validator_count, validator_count+1, ...
    let keypair_index = validator_count + builder_index as usize;
    let all_keypairs = generate_deterministic_keypairs(keypair_index + 1);
    let builder_keypair = all_keypairs[keypair_index].clone();
    info!(
        builder_index,
        keypair_index,
        pubkey = %builder_keypair.pk,
        "Using builder keypair"
    );

    let client = BeaconNodeHttpClient::new(beacon_url, Timeouts::set_all(HTTP_TIMEOUT));

    executor
        .handle()
        .ok_or("shutdown in progress")?
        .block_on(async move {
            run_async::<E>(
                &client,
                &builder_keypair,
                builder_index,
                validator_count,
                bid_value,
                fee_recipient,
                gas_limit,
                block_hash,
                slot_override,
                &spec,
            )
            .await
        })
        .map_err(|e| format!("Async task failed: {e:?}"))
}

#[allow(clippy::too_many_arguments)]
async fn run_async<E: EthSpec>(
    client: &BeaconNodeHttpClient,
    builder_keypair: &types::Keypair,
    builder_index: u64,
    validator_count: usize,
    bid_value: u64,
    fee_recipient: Address,
    gas_limit: u64,
    block_hash: ExecutionBlockHash,
    slot_override: Option<u64>,
    spec: &ChainSpec,
) -> Result<(), String> {
    // Step 1: Get current head state info
    info!("Fetching head state info from beacon node...");
    let syncing = client
        .get_node_syncing()
        .await
        .map_err(|e| format!("Failed to get syncing status: {e:?}"))?;
    let head_slot = syncing.data.head_slot;
    info!(head_slot = head_slot.as_u64(), "Current head slot");

    // Determine target slot for bid
    let target_slot = Slot::new(slot_override.unwrap_or(head_slot.as_u64() + 1));
    info!(
        target_slot = target_slot.as_u64(),
        "Submitting bid for slot"
    );

    // Get the head block root (parent block root for our bid)
    let head_header = client
        .get_beacon_headers_block_id(eth2::types::BlockId::Head)
        .await
        .map_err(|e| format!("Failed to get head header: {e:?}"))?
        .ok_or("Head header not available")?;
    let parent_block_root = head_header.data.root;
    info!(%parent_block_root, "Parent block root");

    // Get RANDAO mix for target epoch
    let target_epoch = target_slot.epoch(E::slots_per_epoch());
    let randao_resp = client
        .get_beacon_states_randao(StateId::Head, Some(target_epoch))
        .await
        .map_err(|e| format!("Failed to get RANDAO: {e:?}"))?
        .ok_or("RANDAO not available")?;
    let prev_randao = randao_resp.data.randao;
    info!(%prev_randao, "RANDAO mix for target epoch");

    // Get fork for domain computation
    let fork_resp = client
        .get_beacon_states_fork(StateId::Head)
        .await
        .map_err(|e| format!("Failed to get fork: {e:?}"))?
        .ok_or("Fork not available")?;
    let fork = fork_resp.data;

    // Get genesis validators root for domain computation
    let genesis_resp = client
        .get_beacon_genesis()
        .await
        .map_err(|e| format!("Failed to get genesis: {e:?}"))?;
    let genesis_validators_root = genesis_resp.data.genesis_validators_root;

    // Step 2: Look up the proposer for the target slot via proposer duties API.
    // We need to sign proposer preferences as the proposer (using their interop keypair).
    let proposer_epoch = target_slot.epoch(E::slots_per_epoch());
    let duties_resp = client
        .get_validator_duties_proposer(proposer_epoch)
        .await
        .map_err(|e| format!("Failed to get proposer duties for epoch {proposer_epoch}: {e:?}"))?;

    let proposer_duty = duties_resp
        .data
        .iter()
        .find(|d| d.slot == target_slot)
        .ok_or_else(|| {
            format!(
                "No proposer duty found for slot {target_slot} in epoch {proposer_epoch}. \
                 Try a slot in a later epoch."
            )
        })?;
    let proposer_validator_index = proposer_duty.validator_index;
    info!(
        proposer_validator_index,
        slot = target_slot.as_u64(),
        "Found proposer for target slot"
    );

    // Derive the proposer's interop keypair.
    // Interop: keypair at index i is generate_deterministic_keypairs(i+1)[i].
    // This assumes the devnet uses interop validators (no keystores), which is the case
    // for the ethereum-package devnet.
    let num_keypairs = validator_count.max(proposer_validator_index as usize + 1);
    let proposer_keypairs = generate_deterministic_keypairs(num_keypairs);
    let proposer_keypair = proposer_keypairs[proposer_validator_index as usize].clone();
    info!(
        proposer_validator_index,
        pubkey = %proposer_keypair.pk,
        "Using proposer keypair for preferences"
    );

    // Step 3: Sign and submit proposer preferences for the target slot.
    // The preferences must match the bid's fee_recipient and gas_limit for bid validation.
    let pref_message = ProposerPreferences {
        proposal_slot: target_slot.as_u64(),
        validator_index: proposer_validator_index,
        fee_recipient,
        gas_limit,
    };
    let pref_epoch = Epoch::new(proposer_epoch.as_u64());
    let pref_domain = spec.get_domain(
        pref_epoch,
        Domain::ProposerPreferences,
        &fork,
        genesis_validators_root,
    );
    let pref_signing_root = pref_message.signing_root(pref_domain);
    let pref_signature = proposer_keypair.sk.sign(pref_signing_root);

    let signed_preferences = SignedProposerPreferences {
        message: pref_message,
        signature: pref_signature,
    };

    info!(
        slot = target_slot.as_u64(),
        proposer_validator_index,
        %fee_recipient,
        gas_limit,
        "Submitting proposer preferences..."
    );
    client
        .post_beacon_pool_proposer_preferences(&signed_preferences)
        .await
        .map_err(|e| format!("Failed to submit proposer preferences: {e:?}"))?;
    info!("Proposer preferences submitted successfully.");

    // Step 4: Construct and sign the bid.
    let bid_message = ExecutionPayloadBid::<E> {
        slot: target_slot,
        builder_index,
        value: bid_value,
        // parent_block_hash: the EL parent hash for the slot we're bidding on.
        // Without EL access, use zero. Pass --block-hash if you have the real EL hash.
        parent_block_hash: ExecutionBlockHash::zero(),
        parent_block_root,
        prev_randao,
        block_hash,
        fee_recipient,
        gas_limit,
        execution_payment: bid_value,
        blob_kzg_commitments: Default::default(),
    };

    let bid_epoch = target_slot.epoch(E::slots_per_epoch());
    let bid_domain = spec.get_domain(
        bid_epoch,
        Domain::BeaconBuilder,
        &fork,
        genesis_validators_root,
    );
    let bid_signing_root = bid_message.signing_root(bid_domain);
    let bid_signature = builder_keypair.sk.sign(bid_signing_root);

    let signed_bid = SignedExecutionPayloadBid::<E> {
        message: bid_message,
        signature: bid_signature,
    };

    // Step 5: Submit the bid.
    info!("Submitting builder bid to beacon node...");
    client
        .post_builder_bids::<E>(&signed_bid)
        .await
        .map_err(|e| format!("Failed to submit bid: {e:?}"))?;

    info!(
        slot = target_slot.as_u64(),
        builder_index,
        value = bid_value,
        "Bid submitted successfully!"
    );

    Ok(())
}
