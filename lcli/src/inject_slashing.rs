//! # inject-slashing
//!
//! Create and submit proposer or attester slashings to a beacon node via HTTP API.
//! Intended for devnet testing of slashing detection.
//!
//! Uses interop (deterministic) keypairs, so the beacon node must be using
//! interop validators (ethereum-package devnet).
//!
//! ## Proposer slashing
//!
//! Creates two conflicting block headers at the same slot signed by `validator_index`.
//! Uses the current head slot as the slashable slot.
//!
//! ```ignore
//! lcli inject-slashing \
//!     --spec minimal \
//!     --beacon-url http://localhost:5052 \
//!     --type proposer \
//!     --validator-index 0
//! ```
//!
//! ## Attester slashing (double vote)
//!
//! Creates two attestations from `validator_index` with the same target epoch
//! but different attestation data.
//!
//! ```ignore
//! lcli inject-slashing \
//!     --spec minimal \
//!     --beacon-url http://localhost:5052 \
//!     --type attester \
//!     --validator-index 0
//! ```
use clap::ArgMatches;
use clap_utils::parse_required;
use environment::Environment;
use eth2::{BeaconNodeHttpClient, SensitiveUrl, Timeouts, types::StateId};
use eth2_network_config::Eth2NetworkConfig;
use std::time::Duration;
use tracing::info;
use types::{
    AggregateSignature, AttestationData, AttesterSlashing, AttesterSlashingBase,
    AttesterSlashingElectra, BeaconBlockHeader, ChainSpec, Checkpoint, Domain, Epoch, EthSpec,
    FixedBytesExtended, ForkName, Hash256, IndexedAttestationBase, IndexedAttestationElectra,
    ProposerSlashing, SignedRoot, Slot, VariableList, test_utils::generate_deterministic_keypairs,
};

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

pub fn run<E: EthSpec>(
    env: Environment<E>,
    network_config: Eth2NetworkConfig,
    matches: &ArgMatches,
) -> Result<(), String> {
    let spec = network_config.chain_spec::<E>()?;
    let executor = env.core_context().executor;

    let beacon_url: SensitiveUrl = parse_required(matches, "beacon-url")?;
    let slashing_type: String = parse_required(matches, "type")?;
    let validator_index: u64 = parse_required(matches, "validator-index")?;

    let client = BeaconNodeHttpClient::new(beacon_url, Timeouts::set_all(HTTP_TIMEOUT));

    executor
        .handle()
        .ok_or("shutdown in progress")?
        .block_on(
            async move { run_async::<E>(&client, &spec, &slashing_type, validator_index).await },
        )
        .map_err(|e| format!("Async task failed: {e:?}"))
}

async fn run_async<E: EthSpec>(
    client: &BeaconNodeHttpClient,
    spec: &ChainSpec,
    slashing_type: &str,
    validator_index: u64,
) -> Result<(), String> {
    // Fetch chain info needed for signing
    let fork_resp = client
        .get_beacon_states_fork(StateId::Head)
        .await
        .map_err(|e| format!("Failed to get fork: {e:?}"))?
        .ok_or("Fork not available")?;
    let fork = fork_resp.data;

    let genesis_resp = client
        .get_beacon_genesis()
        .await
        .map_err(|e| format!("Failed to get genesis: {e:?}"))?;
    let genesis_validators_root = genesis_resp.data.genesis_validators_root;

    let syncing = client
        .get_node_syncing()
        .await
        .map_err(|e| format!("Failed to get syncing status: {e:?}"))?;
    let head_slot = syncing.data.head_slot;

    // Get the fork name to choose correct slashing variant
    let fork_name = spec.fork_name_at_epoch(head_slot.epoch(E::slots_per_epoch()));
    info!(
        %head_slot,
        %fork_name,
        validator_index,
        slashing_type,
        "Injecting slashing"
    );

    // Derive the validator's interop keypair
    let keypairs = generate_deterministic_keypairs(validator_index as usize + 1);
    let validator_keypair = &keypairs[validator_index as usize];
    info!(
        validator_index,
        pubkey = %validator_keypair.pk,
        "Using validator keypair for slashing"
    );

    match slashing_type {
        "proposer" => {
            inject_proposer_slashing::<E>(
                client,
                spec,
                &fork,
                genesis_validators_root,
                validator_index,
                validator_keypair,
                head_slot,
            )
            .await
        }
        "attester" => {
            inject_attester_slashing::<E>(
                client,
                spec,
                &fork,
                genesis_validators_root,
                validator_index,
                validator_keypair,
                head_slot,
                fork_name,
            )
            .await
        }
        other => Err(format!(
            "Unknown slashing type '{other}'. Use 'proposer' or 'attester'."
        )),
    }
}

/// Create and submit a proposer slashing.
///
/// Two block headers for the same slot with the same proposer_index but different state_root.
/// Both are signed with the validator's key → valid double-proposal equivocation.
async fn inject_proposer_slashing<E: EthSpec>(
    client: &BeaconNodeHttpClient,
    spec: &ChainSpec,
    fork: &types::Fork,
    genesis_validators_root: Hash256,
    validator_index: u64,
    validator_keypair: &types::Keypair,
    head_slot: Slot,
) -> Result<(), String> {
    // Get the actual head block header to use as the base
    let head_header_resp = client
        .get_beacon_headers_block_id(eth2::types::BlockId::Head)
        .await
        .map_err(|e| format!("Failed to get head header: {e:?}"))?
        .ok_or("Head header not available")?;

    let head_block_header = head_header_resp.data.header.message;

    // Create header_1: same as head but with our validator as proposer
    let header_1 = BeaconBlockHeader {
        slot: head_slot,
        proposer_index: validator_index,
        parent_root: head_block_header.parent_root,
        state_root: head_block_header.state_root,
        body_root: head_block_header.body_root,
    };

    // Create header_2: same as header_1 but with a different state_root → makes it a valid equivocation
    let mut header_2 = header_1.clone();
    header_2.state_root = Hash256::repeat_byte(0xDE);

    // Sign both headers with DOMAIN_BEACON_PROPOSER
    let signed_header_1 =
        header_1.sign::<E>(&validator_keypair.sk, fork, genesis_validators_root, spec);
    let signed_header_2 =
        header_2.sign::<E>(&validator_keypair.sk, fork, genesis_validators_root, spec);

    let proposer_slashing = ProposerSlashing {
        signed_header_1,
        signed_header_2,
    };

    info!(
        slot = head_slot.as_u64(),
        validator_index, "Submitting proposer slashing (double-proposal)..."
    );

    client
        .post_beacon_pool_proposer_slashings(&proposer_slashing)
        .await
        .map_err(|e| format!("Failed to submit proposer slashing: {e:?}"))?;

    info!(
        validator_index,
        slot = head_slot.as_u64(),
        "Proposer slashing submitted successfully! \
         Validator should be slashed when included in a block."
    );

    Ok(())
}

/// Create and submit an attester slashing (double-vote).
///
/// Two attestations with the same target epoch but different data.
/// Both signed by the same validator → valid double-vote equivocation.
#[allow(clippy::too_many_arguments)]
async fn inject_attester_slashing<E: EthSpec>(
    client: &BeaconNodeHttpClient,
    spec: &ChainSpec,
    fork: &types::Fork,
    genesis_validators_root: Hash256,
    validator_index: u64,
    validator_keypair: &types::Keypair,
    head_slot: Slot,
    fork_name: ForkName,
) -> Result<(), String> {
    // Create two conflicting attestation data — same target epoch, different index
    let target_epoch = head_slot.epoch(E::slots_per_epoch());

    let data_1 = AttestationData {
        slot: Slot::new(0),
        index: 0,
        beacon_block_root: Hash256::zero(),
        source: Checkpoint {
            epoch: Epoch::new(0),
            root: Hash256::zero(),
        },
        target: Checkpoint {
            epoch: target_epoch,
            root: Hash256::zero(),
        },
    };

    // data_2 differs only in `index` — same target epoch, same slot, different committee index
    // This is a double-vote (is_double_vote returns true because target epochs match but data differs)
    let data_2 = AttestationData { index: 1, ..data_1 };

    // Sign both attestations with DOMAIN_BEACON_ATTESTER
    let sign_att = |data: &AttestationData| {
        let domain = spec.get_domain(
            data.target.epoch,
            Domain::BeaconAttester,
            fork,
            genesis_validators_root,
        );
        let signing_root = data.signing_root(domain);
        let mut agg_sig = AggregateSignature::infinity();
        agg_sig.add_assign(&validator_keypair.sk.sign(signing_root));
        agg_sig
    };

    let sig_1 = sign_att(&data_1);
    let sig_2 = sign_att(&data_2);

    let attester_slashing = if fork_name.electra_enabled() {
        AttesterSlashing::Electra(AttesterSlashingElectra {
            attestation_1: IndexedAttestationElectra {
                attesting_indices: VariableList::new(vec![validator_index])
                    .map_err(|e| format!("Failed to create attesting_indices list: {e:?}"))?,
                data: data_1,
                signature: sig_1,
            },
            attestation_2: IndexedAttestationElectra {
                attesting_indices: VariableList::new(vec![validator_index])
                    .map_err(|e| format!("Failed to create attesting_indices list: {e:?}"))?,
                data: data_2,
                signature: sig_2,
            },
        })
    } else {
        AttesterSlashing::Base(AttesterSlashingBase {
            attestation_1: IndexedAttestationBase {
                attesting_indices: VariableList::new(vec![validator_index])
                    .map_err(|e| format!("Failed to create attesting_indices list: {e:?}"))?,
                data: data_1,
                signature: sig_1,
            },
            attestation_2: IndexedAttestationBase {
                attesting_indices: VariableList::new(vec![validator_index])
                    .map_err(|e| format!("Failed to create attesting_indices list: {e:?}"))?,
                data: data_2,
                signature: sig_2,
            },
        })
    };

    info!(
        validator_index,
        target_epoch = target_epoch.as_u64(),
        "Submitting attester slashing (double-vote)..."
    );

    // Use v1 API (available for all forks)
    client
        .post_beacon_pool_attester_slashings_v1::<E>(&attester_slashing)
        .await
        .map_err(|e| format!("Failed to submit attester slashing: {e:?}"))?;

    info!(
        validator_index,
        target_epoch = target_epoch.as_u64(),
        "Attester slashing submitted successfully! \
         Validator should be slashed when included in a block."
    );

    Ok(())
}
