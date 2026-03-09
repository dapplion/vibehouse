//! This crate contains a HTTP server which serves the endpoints listed here:
//!
//! <https://github.com/ethereum/beacon-APIs>
//!
//! There are also some additional, non-standard endpoints behind the `/lighthouse/` path which are
//! used for development.
// BeaconChainError is returned from closures throughout this crate.
// Its size (128+ bytes) triggers result_large_err but boxing it would
// change the error type API across the entire project.
#![allow(clippy::result_large_err)]

mod aggregate_attestation;
pub mod api_error;
mod attestation_performance;
mod attester_duties;
mod block_id;
mod block_packing_efficiency;
mod block_rewards;
mod build_block_contents;
mod builder_states;
mod custody;
mod database;
mod extractors;
mod light_client;
mod metrics;
mod peer;
mod produce_block;
mod proposer_duties;
mod ptc_duties;
mod publish_attestations;
mod publish_blocks;
mod standard_block_rewards;
mod state_id;
mod sync_committee_rewards;
mod sync_committees;
mod task_spawner;
pub mod test_utils;
mod ui;
mod validator;
mod validator_inclusion;
mod validators;
mod version;
use crate::api_error::ApiError;
use crate::extractors::{
    MultiKeyQuery, accept_header, consensus_version_header, json_body, json_body_or_default,
    optional_consensus_version_header, parse_endpoint_version,
};
use crate::light_client::{get_light_client_bootstrap, get_light_client_updates};
use crate::produce_block::{produce_blinded_block_v2, produce_block_v2, produce_block_v3};
use crate::version::beacon_response;
use beacon_chain::{
    AttestationError as AttnError, BeaconChain, BeaconChainError, BeaconChainTypes,
    WhenSlotSkipped, attestation_verification::VerifiedAttestation,
    observed_operations::ObservationOutcome, validator_monitor::timestamp_now,
};
use beacon_processor::BeaconProcessorSend;
pub use block_id::BlockId;
use builder_states::get_next_withdrawals;
use bytes::Bytes;
use directory::DEFAULT_ROOT_DIR;
use eth2::types::{
    self as api_types, BroadcastValidation, ContextDeserialize, ForkChoice, ForkChoiceNode,
    LightClientUpdatesQuery, PublishBlockRequest, StateId as CoreStateId,
    ValidatorBalancesRequestBody, ValidatorId, ValidatorIdentitiesRequestBody, ValidatorStatus,
    ValidatorsRequestBody,
};
use eth2::{CONTENT_TYPE_HEADER, SSZ_CONTENT_TYPE_HEADER};
use futures::StreamExt;
use health_metrics::observe::Observe;
use lighthouse_network::rpc::methods::MetaData;
use lighthouse_network::{Enr, NetworkGlobals, PeerId, PubsubMessage, types::SyncState};
use lighthouse_version::version_with_platform;
use logging::{SSELoggingComponents, crit};
use network::{NetworkMessage, NetworkSenders, ValidatorSubscriptionMessage};
use network_utils::enr_ext::EnrExt;
use operation_pool::ReceivedPreCapella;
use parking_lot::RwLock;
pub use publish_blocks::{
    ProvenancedBlock, publish_blinded_block, publish_block, reconstruct_block,
};
use serde::{Deserialize, Serialize};
use slot_clock::SlotClock;
use ssz::Encode;
pub use state_id::StateId;
use std::collections::HashSet;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use sysinfo::{System, SystemExt};
use system_health::{observe_nat, observe_system_health_bn};
use task_spawner::{Priority, TaskSpawner};
use tokio::sync::{
    mpsc::{Sender, UnboundedSender},
    oneshot,
};
use tokio_stream::wrappers::{BroadcastStream, errors::BroadcastStreamRecvError};
use tracing::{debug, error, info, warn};
use types::{
    Attestation, AttestationData, AttestationShufflingId, AttesterSlashing, BeaconStateError,
    ChainSpec, Checkpoint, CommitteeCache, ConfigAndPreset, Domain, Epoch, EthSpec, ExecutionProof,
    ForkName, Hash256, PayloadAttestationMessage, ProposerPreparationData, ProposerSlashing,
    RelativeEpoch, SignedAggregateAndProof, SignedBlindedBeaconBlock, SignedBlsToExecutionChange,
    SignedContributionAndProof, SignedExecutionPayloadBid, SignedExecutionPayloadEnvelope,
    SignedProposerPreferences, SignedRoot, SignedValidatorRegistrationData, SignedVoluntaryExit,
    SingleAttestation, Slot, SyncCommitteeMessage, SyncContributionData,
};
use validator::pubkey_to_validator_index;
use version::{
    ResponseIncludesVersion, V1, V2, V3, add_consensus_version_header, add_ssz_content_type_header,
    execution_optimistic_finalized_beacon_response, inconsistent_fork_rejection,
    unsupported_version_rejection,
};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;

const API_PREFIX: &str = "eth";

/// A custom type which allows for both unsecured and TLS-enabled HTTP servers.
type HttpServer = (SocketAddr, Box<dyn Future<Output = ()> + Send + Unpin>);

/// Alias for readability.
pub type ExecutionOptimistic = bool;

/// Configuration used when serving the HTTP server over TLS.
#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    pub cert: PathBuf,
    pub key: PathBuf,
}

/// A wrapper around all the items required to spawn the HTTP server.
///
/// The server will gracefully handle the case where any fields are `None`.
pub struct Context<T: BeaconChainTypes> {
    pub config: Config,
    pub chain: Option<Arc<BeaconChain<T>>>,
    pub network_senders: Option<NetworkSenders<T::EthSpec>>,
    pub network_globals: Option<Arc<NetworkGlobals<T::EthSpec>>>,
    pub beacon_processor_send: Option<BeaconProcessorSend<T::EthSpec>>,
    pub sse_logging_components: Option<SSELoggingComponents>,
}

mod serde_axum_status_code {
    use serde::{Deserialize, Serialize, de::Error};

    pub fn serialize<S>(status_code: &axum::http::StatusCode, ser: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        status_code.as_u16().serialize(ser)
    }

    pub fn deserialize<'de, D>(de: D) -> Result<axum::http::StatusCode, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let status_code = u16::deserialize(de)?;
        axum::http::StatusCode::from_u16(status_code).map_err(D::Error::custom)
    }
}

/// Configuration for the HTTP server.
#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub enabled: bool,
    pub listen_addr: IpAddr,
    pub listen_port: u16,
    pub allow_origin: Option<String>,
    pub tls_config: Option<TlsConfig>,
    pub data_dir: PathBuf,
    pub sse_capacity_multiplier: usize,
    pub enable_beacon_processor: bool,
    #[serde(with = "serde_axum_status_code")]
    pub duplicate_block_status_code: StatusCode,
    pub target_peers: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            listen_addr: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            listen_port: 5052,
            allow_origin: None,
            tls_config: None,
            data_dir: PathBuf::from(DEFAULT_ROOT_DIR),
            sse_capacity_multiplier: 1,
            enable_beacon_processor: true,
            duplicate_block_status_code: StatusCode::ACCEPTED,
            target_peers: 100,
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Other(String),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<String> for Error {
    fn from(e: String) -> Self {
        Error::Other(e)
    }
}

/// Shared application state for the beacon node HTTP API.
struct AppState<T: BeaconChainTypes> {
    chain: Option<Arc<BeaconChain<T>>>,
    network_tx: Option<UnboundedSender<NetworkMessage<T::EthSpec>>>,
    validator_subscription_tx: Option<Sender<ValidatorSubscriptionMessage>>,
    network_globals: Option<Arc<NetworkGlobals<T::EthSpec>>>,
    task_spawner: TaskSpawner<T::EthSpec>,
    data_dir: PathBuf,
    system_info: Arc<RwLock<System>>,
    app_start: std::time::Instant,
    sse_logging_components: Option<SSELoggingComponents>,
    duplicate_block_status_code: StatusCode,
}

type SharedState<T> = Arc<AppState<T>>;

impl<T: BeaconChainTypes> AppState<T> {
    fn chain(&self) -> Result<Arc<BeaconChain<T>>, ApiError> {
        self.chain
            .clone()
            .ok_or_else(|| ApiError::not_found("Beacon chain genesis has not yet been observed."))
    }

    fn network_tx(&self) -> Result<UnboundedSender<NetworkMessage<T::EthSpec>>, ApiError> {
        self.network_tx
            .clone()
            .ok_or_else(|| ApiError::not_found("The networking stack has not yet started."))
    }

    fn validator_subscription_tx(&self) -> Result<Sender<ValidatorSubscriptionMessage>, ApiError> {
        self.validator_subscription_tx
            .clone()
            .ok_or_else(|| ApiError::not_found("The networking stack has not yet started."))
    }

    fn network_globals(&self) -> Result<Arc<NetworkGlobals<T::EthSpec>>, ApiError> {
        self.network_globals
            .clone()
            .ok_or_else(|| ApiError::not_found("Network globals are not initialized."))
    }

    fn task_spawner(&self) -> TaskSpawner<T::EthSpec> {
        self.task_spawner.clone()
    }

    fn check_not_syncing(&self) -> Result<(), ApiError> {
        let network_globals = self.network_globals()?;
        let chain = self.chain()?;
        match *network_globals.sync_state.read() {
            SyncState::SyncingFinalized { .. } | SyncState::SyncingHead { .. } => {
                let head_slot = chain.canonical_head.cached_head().head_slot();
                let current_slot = chain
                    .slot_clock
                    .now_or_genesis()
                    .ok_or_else(|| ApiError::server_error("unable to read slot clock"))?;
                let tolerance = chain.config.sync_tolerance_epochs * T::EthSpec::slots_per_epoch();
                if head_slot + tolerance >= current_slot {
                    Ok(())
                } else {
                    Err(ApiError::service_unavailable(format!(
                        "head slot is {}, current slot is {}",
                        head_slot, current_slot
                    )))
                }
            }
            SyncState::SyncTransition
            | SyncState::BackFillSyncing { .. }
            | SyncState::CustodyBackFillSyncing { .. }
            | SyncState::Synced
            | SyncState::Stalled => Ok(()),
        }
    }

    fn check_light_client_server(&self) -> Result<(), ApiError> {
        let chain = self.chain()?;
        if chain.config.enable_light_client_server {
            Ok(())
        } else {
            Err(ApiError::not_found("Light client server is disabled"))
        }
    }
}

/// Axum middleware for prometheus metrics.
async fn prometheus_metrics_middleware(request: axum::extract::Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let method = request.method().to_string();
    let start = std::time::Instant::now();

    let response = next.run(request).await;

    let elapsed = start.elapsed();
    let status = response.status();

    // Bucket paths to avoid cardinality explosion.
    let bucketed_path = bucket_api_path(&path);

    metrics::inc_counter_vec(&metrics::HTTP_API_PATHS_TOTAL, &[bucketed_path]);
    metrics::inc_counter_vec(
        &metrics::HTTP_API_STATUS_CODES_TOTAL,
        &[&status.to_string()],
    );
    metrics::observe_timer_vec(&metrics::HTTP_API_PATHS_TIMES, &[bucketed_path], elapsed);

    if status.is_success() {
        debug!(
            elapsed_ms = %elapsed.as_secs_f64() * 1000.0,
            status = %status,
            path = %path,
            method = %method,
            "Processed HTTP API request"
        );
    } else {
        warn!(
            elapsed_ms = %elapsed.as_secs_f64() * 1000.0,
            status = %status,
            path = %path,
            method = %method,
            "Error processing HTTP API request"
        );
    }

    response
}

fn bucket_api_path(path: &str) -> &'static str {
    let equals = |s: &'static str| -> Option<&'static str> {
        if path == format!("/{}/{}", API_PREFIX, s) {
            Some(s)
        } else {
            None
        }
    };
    let starts_with = |s: &'static str| -> Option<&'static str> {
        if path.starts_with(&format!("/{}/{}", API_PREFIX, s)) {
            Some(s)
        } else {
            None
        }
    };

    equals("v1/beacon/blocks")
        .or_else(|| starts_with("v2/beacon/blocks"))
        .or_else(|| starts_with("v1/beacon/blob_sidecars"))
        .or_else(|| starts_with("v1/beacon/blobs"))
        .or_else(|| starts_with("v1/beacon/blocks/head/root"))
        .or_else(|| starts_with("v1/beacon/blinded_blocks"))
        .or_else(|| starts_with("v2/beacon/blinded_blocks"))
        .or_else(|| starts_with("v1/beacon/headers"))
        .or_else(|| starts_with("v1/beacon/light_client"))
        .or_else(|| starts_with("v1/beacon/pool/attestations"))
        .or_else(|| starts_with("v2/beacon/pool/attestations"))
        .or_else(|| starts_with("v1/beacon/pool/attester_slashings"))
        .or_else(|| starts_with("v1/beacon/pool/bls_to_execution_changes"))
        .or_else(|| starts_with("v1/beacon/pool/proposer_slashings"))
        .or_else(|| starts_with("v1/beacon/pool/sync_committees"))
        .or_else(|| starts_with("v1/beacon/pool/voluntary_exits"))
        .or_else(|| starts_with("v1/beacon/rewards/blocks"))
        .or_else(|| starts_with("v1/beacon/rewards/attestations"))
        .or_else(|| starts_with("v1/beacon/rewards/sync_committee"))
        .or_else(|| starts_with("v1/beacon/rewards"))
        .or_else(|| starts_with("v1/beacon/states"))
        .or_else(|| starts_with("v1/beacon/"))
        .or_else(|| starts_with("v2/beacon/"))
        .or_else(|| starts_with("v1/builder/bids"))
        .or_else(|| starts_with("v1/builder/states"))
        .or_else(|| starts_with("v1/config/deposit_contract"))
        .or_else(|| starts_with("v1/config/fork_schedule"))
        .or_else(|| starts_with("v1/config/spec"))
        .or_else(|| starts_with("v1/config/"))
        .or_else(|| starts_with("v1/debug/"))
        .or_else(|| starts_with("v2/debug/"))
        .or_else(|| starts_with("v1/events"))
        .or_else(|| starts_with("v1/events/"))
        .or_else(|| starts_with("v1/node/health"))
        .or_else(|| starts_with("v1/node/identity"))
        .or_else(|| starts_with("v1/node/peers"))
        .or_else(|| starts_with("v1/node/peer_count"))
        .or_else(|| starts_with("v1/node/syncing"))
        .or_else(|| starts_with("v1/node/version"))
        .or_else(|| starts_with("v1/node"))
        .or_else(|| starts_with("v1/validator/aggregate_and_proofs"))
        .or_else(|| starts_with("v2/validator/aggregate_and_proofs"))
        .or_else(|| starts_with("v1/validator/aggregate_attestation"))
        .or_else(|| starts_with("v2/validator/aggregate_attestation"))
        .or_else(|| starts_with("v1/validator/attestation_data"))
        .or_else(|| starts_with("v1/validator/beacon_committee_subscriptions"))
        .or_else(|| starts_with("v1/validator/blinded_blocks"))
        .or_else(|| starts_with("v2/validator/blinded_blocks"))
        .or_else(|| starts_with("v1/validator/blocks"))
        .or_else(|| starts_with("v2/validator/blocks"))
        .or_else(|| starts_with("v3/validator/blocks"))
        .or_else(|| starts_with("v1/validator/contribution_and_proofs"))
        .or_else(|| starts_with("v1/validator/duties/attester"))
        .or_else(|| starts_with("v1/validator/duties/proposer"))
        .or_else(|| starts_with("v1/validator/duties/sync"))
        .or_else(|| starts_with("v1/validator/liveness"))
        .or_else(|| starts_with("v1/validator/prepare_beacon_proposer"))
        .or_else(|| starts_with("v1/validator/register_validator"))
        .or_else(|| starts_with("v1/validator/sync_committee_contribution"))
        .or_else(|| starts_with("v1/validator/sync_committee_subscriptions"))
        .or_else(|| starts_with("v1/validator/"))
        .or_else(|| starts_with("v2/validator/"))
        .or_else(|| starts_with("v3/validator/"))
        .or_else(|| starts_with("lighthouse"))
        .unwrap_or("other")
}

/// Creates a server that will serve requests using information from `ctx`.
///
/// The server will shut down gracefully when the `shutdown` future resolves.
pub fn serve<T: BeaconChainTypes>(
    ctx: Arc<Context<T>>,
    shutdown: impl Future<Output = ()> + Send + Sync + 'static,
) -> Result<HttpServer, Error> {
    let config = ctx.config.clone();

    // Sanity check.
    if !config.enabled {
        crit!("Cannot start disabled HTTP server");
        return Err(Error::Other(
            "A disabled server should not be started".to_string(),
        ));
    }

    if config.tls_config.is_some() {
        return Err(Error::Other(
            "TLS is not yet supported with the axum backend. Use a reverse proxy for TLS."
                .to_string(),
        ));
    }

    // Configure CORS.
    let cors_layer = build_cors_layer(
        config.allow_origin.as_deref(),
        config.listen_addr,
        config.listen_port,
    )?;

    let beacon_processor_send = ctx
        .beacon_processor_send
        .clone()
        .filter(|_| config.enable_beacon_processor);

    let system_info = Arc::new(RwLock::new(sysinfo::System::new()));
    {
        let mut system_info = system_info.write();
        system_info.refresh_disks_list();
        system_info.refresh_networks_list();
        system_info.refresh_cpu_specifics(sysinfo::CpuRefreshKind::everything());
        system_info.refresh_cpu();
    }

    let state: SharedState<T> = Arc::new(AppState {
        chain: ctx.chain.clone(),
        network_tx: ctx.network_senders.as_ref().map(|s| s.network_send()),
        validator_subscription_tx: ctx
            .network_senders
            .as_ref()
            .map(|s| s.validator_subscription_send()),
        network_globals: ctx.network_globals.clone(),
        task_spawner: TaskSpawner::new(beacon_processor_send),
        data_dir: config.data_dir.clone(),
        system_info,
        app_start: std::time::Instant::now(),
        sse_logging_components: ctx.sse_logging_components.clone(),
        duplicate_block_status_code: config.duplicate_block_status_code,
    });

    // Build the router with all API routes.
    let app = Router::new()
        // Beacon genesis
        .route("/eth/v1/beacon/genesis", get(get_beacon_genesis::<T>))
        // Beacon state routes
        .route(
            "/eth/v1/beacon/states/{state_id}/root",
            get(get_beacon_state_root::<T>),
        )
        .route(
            "/eth/v1/beacon/states/{state_id}/fork",
            get(get_beacon_state_fork::<T>),
        )
        .route(
            "/eth/v1/beacon/states/{state_id}/finality_checkpoints",
            get(get_beacon_state_finality_checkpoints::<T>),
        )
        .route(
            "/eth/v1/beacon/states/{state_id}/validator_balances",
            get(get_beacon_state_validator_balances::<T>)
                .post(post_beacon_state_validator_balances::<T>),
        )
        .route(
            "/eth/v1/beacon/states/{state_id}/validator_identities",
            post(post_beacon_state_validator_identities::<T>),
        )
        .route(
            "/eth/v1/beacon/states/{state_id}/validators",
            get(get_beacon_state_validators::<T>).post(post_beacon_state_validators::<T>),
        )
        .route(
            "/eth/v1/beacon/states/{state_id}/validators/{validator_id}",
            get(get_beacon_state_validators_id::<T>),
        )
        .route(
            "/eth/v1/beacon/states/{state_id}/committees",
            get(get_beacon_state_committees::<T>),
        )
        .route(
            "/eth/v1/beacon/states/{state_id}/sync_committees",
            get(get_beacon_state_sync_committees::<T>),
        )
        .route(
            "/eth/v1/beacon/states/{state_id}/randao",
            get(get_beacon_state_randao::<T>),
        )
        .route(
            "/eth/v1/beacon/states/{state_id}/pending_deposits",
            get(get_beacon_state_pending_deposits::<T>),
        )
        .route(
            "/eth/v1/beacon/states/{state_id}/pending_partial_withdrawals",
            get(get_beacon_state_pending_partial_withdrawals::<T>),
        )
        .route(
            "/eth/v1/beacon/states/{state_id}/pending_consolidations",
            get(get_beacon_state_pending_consolidations::<T>),
        )
        .route(
            "/eth/v1/beacon/states/{state_id}/proposer_lookahead",
            get(get_beacon_state_proposer_lookahead::<T>),
        )
        // Beacon headers
        .route("/eth/v1/beacon/headers", get(get_beacon_headers::<T>))
        .route(
            "/eth/v1/beacon/headers/{block_id}",
            get(get_beacon_headers_block_id::<T>),
        )
        // Block publishing
        .route("/eth/v1/beacon/blocks", post(post_beacon_blocks_v1::<T>))
        .route("/eth/v2/beacon/blocks", post(post_beacon_blocks_v2::<T>))
        .route(
            "/eth/v1/beacon/blinded_blocks",
            post(post_beacon_blinded_blocks_v1::<T>),
        )
        .route(
            "/eth/v2/beacon/blinded_blocks",
            post(post_beacon_blinded_blocks_v2::<T>),
        )
        // Block retrieval
        .route(
            "/eth/{version}/beacon/blocks/{block_id}",
            get(get_beacon_block::<T>),
        )
        .route(
            "/eth/v1/beacon/blocks/{block_id}/root",
            get(get_beacon_block_root::<T>),
        )
        .route(
            "/eth/{version}/beacon/blocks/{block_id}/attestations",
            get(get_beacon_block_attestations::<T>),
        )
        .route(
            "/eth/v1/beacon/blinded_blocks/{block_id}",
            get(get_beacon_blinded_block::<T>),
        )
        .route(
            "/eth/v1/beacon/blob_sidecars/{block_id}",
            get(get_blob_sidecars::<T>),
        )
        .route("/eth/v1/beacon/blobs/{block_id}", get(get_blobs::<T>))
        // Beacon pool routes
        .route(
            "/eth/v2/beacon/pool/attestations",
            get(get_beacon_pool_attestations_v2::<T>).post(post_beacon_pool_attestations_v2::<T>),
        )
        .route(
            "/eth/v1/beacon/pool/attestations",
            get(get_beacon_pool_attestations_v1::<T>),
        )
        .route(
            "/eth/{version}/beacon/pool/attester_slashings",
            get(get_beacon_pool_attester_slashings::<T>)
                .post(post_beacon_pool_attester_slashings::<T>),
        )
        .route(
            "/eth/v1/beacon/pool/proposer_slashings",
            get(get_beacon_pool_proposer_slashings::<T>)
                .post(post_beacon_pool_proposer_slashings::<T>),
        )
        .route(
            "/eth/v1/beacon/pool/voluntary_exits",
            get(get_beacon_pool_voluntary_exits::<T>).post(post_beacon_pool_voluntary_exits::<T>),
        )
        .route(
            "/eth/v1/beacon/pool/sync_committees",
            post(post_beacon_pool_sync_committees::<T>),
        )
        .route(
            "/eth/v1/beacon/pool/bls_to_execution_changes",
            get(get_beacon_pool_bls_to_execution_changes::<T>)
                .post(post_beacon_pool_bls_to_execution_changes::<T>),
        )
        .route(
            "/eth/v1/beacon/pool/payload_attestations",
            get(get_beacon_pool_payload_attestations::<T>)
                .post(post_beacon_pool_payload_attestations::<T>),
        )
        .route(
            "/eth/v1/beacon/pool/proposer_preferences",
            post(post_beacon_pool_proposer_preferences::<T>),
        )
        // Beacon rewards
        .route(
            "/eth/v1/beacon/rewards/blocks/{block_id}",
            get(get_beacon_rewards_blocks::<T>),
        )
        .route(
            "/eth/v1/beacon/rewards/attestations/{epoch}",
            post(post_beacon_rewards_attestations::<T>),
        )
        .route(
            "/eth/v1/beacon/rewards/sync_committee/{block_id}",
            post(post_beacon_rewards_sync_committee::<T>),
        )
        // Builder routes
        .route(
            "/eth/v1/builder/states/{state_id}/expected_withdrawals",
            get(get_expected_withdrawals_handler::<T>),
        )
        .route("/eth/v1/builder/bids", post(post_builder_bids::<T>))
        // Execution payload envelope
        .route(
            "/eth/v1/beacon/execution_payload_envelope/{block_id}",
            get(get_beacon_execution_payload_envelope::<T>),
        )
        .route(
            "/eth/v1/beacon/execution_payload_envelope",
            post(post_beacon_execution_payload_envelope::<T>),
        )
        // Light client
        .route(
            "/eth/v1/beacon/light_client/bootstrap/{block_root}",
            get(get_beacon_light_client_bootstrap_handler::<T>),
        )
        .route(
            "/eth/v1/beacon/light_client/optimistic_update",
            get(get_beacon_light_client_optimistic_update::<T>),
        )
        .route(
            "/eth/v1/beacon/light_client/finality_update",
            get(get_beacon_light_client_finality_update::<T>),
        )
        .route(
            "/eth/v1/beacon/light_client/updates",
            get(get_beacon_light_client_updates_handler::<T>),
        )
        // Config routes
        .route(
            "/eth/v1/config/fork_schedule",
            get(get_config_fork_schedule::<T>),
        )
        .route("/eth/v1/config/spec", get(get_config_spec::<T>))
        .route(
            "/eth/v1/config/deposit_contract",
            get(get_config_deposit_contract::<T>),
        )
        // Debug routes
        .route(
            "/eth/v1/debug/beacon/data_column_sidecars/{block_id}",
            get(get_debug_data_column_sidecars::<T>),
        )
        .route(
            "/eth/{version}/debug/beacon/states/{state_id}",
            get(get_debug_beacon_states::<T>),
        )
        .route(
            "/eth/{version}/debug/beacon/heads",
            get(get_debug_beacon_heads::<T>),
        )
        .route("/eth/v1/debug/fork_choice", get(get_debug_fork_choice::<T>))
        // Node routes
        .route("/eth/v1/node/identity", get(get_node_identity::<T>))
        .route("/eth/v1/node/version", get(get_node_version::<T>))
        .route("/eth/v1/node/syncing", get(get_node_syncing::<T>))
        .route("/eth/v1/node/health", get(get_node_health::<T>))
        .route(
            "/eth/v1/node/peers/{peer_id}",
            get(get_node_peers_by_id::<T>),
        )
        .route("/eth/v1/node/peers", get(get_node_peers::<T>))
        .route("/eth/v1/node/peer_count", get(get_node_peer_count::<T>))
        // Validator routes
        .route(
            "/eth/v1/validator/duties/proposer/{epoch}",
            get(get_validator_duties_proposer::<T>),
        )
        .route(
            "/eth/{version}/validator/blocks/{slot}",
            get(get_validator_blocks::<T>),
        )
        .route(
            "/eth/v1/validator/blinded_blocks/{slot}",
            get(get_validator_blinded_blocks::<T>),
        )
        .route(
            "/eth/v1/validator/attestation_data",
            get(get_validator_attestation_data::<T>),
        )
        .route(
            "/eth/v1/validator/payload_attestation_data",
            get(get_validator_payload_attestation_data::<T>),
        )
        .route(
            "/eth/{version}/validator/aggregate_attestation",
            get(get_validator_aggregate_attestation::<T>),
        )
        .route(
            "/eth/v1/validator/sync_committee_contribution",
            get(get_validator_sync_committee_contribution::<T>),
        )
        .route(
            "/eth/v1/validator/duties/attester/{epoch}",
            post(post_validator_duties_attester::<T>),
        )
        .route(
            "/eth/v1/validator/duties/ptc/{epoch}",
            post(post_validator_duties_ptc::<T>),
        )
        .route(
            "/eth/v1/validator/duties/sync/{epoch}",
            post(post_validator_duties_sync::<T>),
        )
        .route(
            "/eth/{version}/validator/aggregate_and_proofs",
            post(post_validator_aggregate_and_proofs::<T>),
        )
        .route(
            "/eth/v1/validator/contribution_and_proofs",
            post(post_validator_contribution_and_proofs::<T>),
        )
        .route(
            "/eth/v1/validator/beacon_committee_subscriptions",
            post(post_validator_beacon_committee_subscriptions::<T>),
        )
        .route(
            "/eth/v1/validator/prepare_beacon_proposer",
            post(post_validator_prepare_beacon_proposer::<T>),
        )
        .route(
            "/eth/v1/validator/register_validator",
            post(post_validator_register_validator::<T>),
        )
        .route(
            "/eth/v1/validator/sync_committee_subscriptions",
            post(post_validator_sync_committee_subscriptions::<T>),
        )
        .route(
            "/eth/v1/validator/liveness/{epoch}",
            post(post_validator_liveness_epoch::<T>),
        )
        // SSE events
        .route("/eth/v1/events", get(get_events::<T>))
        // Lighthouse routes
        .route("/lighthouse/health", get(get_lighthouse_health::<T>))
        .route("/lighthouse/ui/health", get(get_lighthouse_ui_health::<T>))
        .route(
            "/lighthouse/ui/validator_count",
            get(get_lighthouse_ui_validator_count::<T>),
        )
        .route("/lighthouse/syncing", get(get_lighthouse_syncing::<T>))
        .route("/lighthouse/nat", get(get_lighthouse_nat::<T>))
        .route("/lighthouse/peers", get(get_lighthouse_peers::<T>))
        .route(
            "/lighthouse/peers/connected",
            get(get_lighthouse_peers_connected::<T>),
        )
        .route(
            "/lighthouse/proto_array",
            get(get_lighthouse_proto_array::<T>),
        )
        .route(
            "/lighthouse/validator_inclusion/{epoch}/{validator_id}",
            get(get_lighthouse_validator_inclusion_global::<T>),
        )
        .route(
            "/lighthouse/validator_inclusion/{epoch}/global",
            get(get_lighthouse_validator_inclusion::<T>),
        )
        .route("/lighthouse/staking", get(get_lighthouse_staking::<T>))
        .route(
            "/lighthouse/database/info",
            get(get_lighthouse_database_info::<T>),
        )
        .route(
            "/lighthouse/custody/info",
            get(get_lighthouse_custody_info::<T>),
        )
        .route(
            "/lighthouse/analysis/block_rewards",
            get(get_lighthouse_block_rewards::<T>).post(post_lighthouse_block_rewards::<T>),
        )
        .route(
            "/lighthouse/analysis/attestation_performance/{target}",
            get(get_lighthouse_attestation_performance::<T>),
        )
        .route(
            "/lighthouse/analysis/block_packing_efficiency",
            get(get_lighthouse_block_packing_efficiency::<T>),
        )
        .route(
            "/lighthouse/merge_readiness",
            get(get_lighthouse_merge_readiness::<T>),
        )
        .route("/lighthouse/finalize", post(post_lighthouse_finalize::<T>))
        .route(
            "/lighthouse/compaction",
            post(post_lighthouse_compaction::<T>),
        )
        .route("/lighthouse/add_peer", post(post_lighthouse_add_peer::<T>))
        .route(
            "/lighthouse/remove_peer",
            post(post_lighthouse_remove_peer::<T>),
        )
        .route("/lighthouse/liveness", post(post_lighthouse_liveness::<T>))
        .route(
            "/lighthouse/ui/validator_metrics",
            post(post_lighthouse_ui_validator_metrics::<T>),
        )
        .route(
            "/lighthouse/ui/validator_info",
            post(post_lighthouse_ui_validator_info::<T>),
        )
        .route(
            "/lighthouse/database/reconstruct",
            post(post_lighthouse_database_reconstruct::<T>),
        )
        .route(
            "/lighthouse/custody/backfill",
            post(post_lighthouse_custody_backfill::<T>),
        )
        .route("/lighthouse/logs", get(get_lighthouse_logs::<T>))
        // Vibehouse routes
        .route(
            "/vibehouse/execution_proof_status/{block_id}",
            get(get_vibehouse_execution_proof_status::<T>),
        )
        .route(
            "/vibehouse/execution_proofs",
            post(post_vibehouse_execution_proofs::<T>),
        )
        // Middleware
        .layer(middleware::from_fn(prometheus_metrics_middleware))
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::header::SERVER,
            axum::http::HeaderValue::from_str(&version_with_platform())
                .unwrap_or_else(|_| axum::http::HeaderValue::from_static("vibehouse")),
        ))
        .layer(cors_layer)
        .with_state(state);

    let http_socket: SocketAddr = SocketAddr::new(config.listen_addr, config.listen_port);

    let listener = std::net::TcpListener::bind(http_socket).map_err(Error::Io)?;
    listener.set_nonblocking(true).map_err(Error::Io)?;
    let listening_socket = listener.local_addr().map_err(Error::Io)?;

    let server: Box<dyn Future<Output = ()> + Send + Unpin> = Box::new(Box::pin(async move {
        let listener = tokio::net::TcpListener::from_std(listener).expect("valid std listener");
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await;
    }));

    info!(
        listen_address = %listening_socket,
        "HTTP API started"
    );

    Ok((listening_socket, server))
}

// ── Handler functions ────────────────────────────────────────────────────────

// -- Beacon genesis --

async fn get_beacon_genesis<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let genesis_data = api_types::GenesisData {
                genesis_time: chain.genesis_time,
                genesis_validators_root: chain.genesis_validators_root,
                genesis_fork_version: chain.spec.genesis_fork_version,
            };
            Ok(api_types::GenericResponse::from(genesis_data))
        })
        .await
}

// -- Beacon state routes --

async fn get_beacon_state_root<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(state_id): Path<StateId>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let (root, execution_optimistic, finalized) = state_id.root(&chain)?;
            Ok(
                api_types::GenericResponse::from(api_types::RootData::from(root))
                    .add_execution_optimistic_finalized(execution_optimistic, finalized),
            )
        })
        .await
}

async fn get_beacon_state_fork<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(state_id): Path<StateId>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let (fork, execution_optimistic, finalized) =
                state_id.fork_and_execution_optimistic_and_finalized(&chain)?;
            Ok(api_types::ExecutionOptimisticFinalizedResponse {
                data: fork,
                execution_optimistic: Some(execution_optimistic),
                finalized: Some(finalized),
            })
        })
        .await
}

async fn get_beacon_state_finality_checkpoints<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(state_id): Path<StateId>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let (data, execution_optimistic, finalized) = state_id
                .map_state_and_execution_optimistic_and_finalized(
                    &chain,
                    |state, execution_optimistic, finalized| {
                        Ok((
                            api_types::FinalityCheckpointsData {
                                previous_justified: state.previous_justified_checkpoint(),
                                current_justified: state.current_justified_checkpoint(),
                                finalized: state.finalized_checkpoint(),
                            },
                            execution_optimistic,
                            finalized,
                        ))
                    },
                )?;
            Ok(api_types::ExecutionOptimisticFinalizedResponse {
                data,
                execution_optimistic: Some(execution_optimistic),
                finalized: Some(finalized),
            })
        })
        .await
}

async fn get_beacon_state_validator_balances<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(state_id): Path<StateId>,
    MultiKeyQuery(query): MultiKeyQuery<api_types::ValidatorBalancesQuery>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            crate::validators::get_beacon_state_validator_balances(
                state_id,
                chain,
                query.id.as_deref(),
            )
        })
        .await
}

async fn post_beacon_state_validator_balances<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(state_id): Path<StateId>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let query: ValidatorBalancesRequestBody = json_body_or_default(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            crate::validators::get_beacon_state_validator_balances(
                state_id,
                chain,
                Some(&query.ids),
            )
        })
        .await
}

async fn post_beacon_state_validator_identities<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(state_id): Path<StateId>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let query: ValidatorIdentitiesRequestBody = json_body_or_default(&headers, body).await?;
    let accept = accept_header(&headers);
    let priority = if let StateId(eth2::types::StateId::Head) = state_id {
        Priority::P0
    } else {
        Priority::P1
    };
    state
        .task_spawner()
        .blocking_response_task(priority, move || {
            let response = crate::validators::get_beacon_state_validator_identities(
                state_id,
                chain,
                Some(&query.ids),
            )?;
            match accept {
                Some(api_types::Accept::Ssz) => Response::builder()
                    .status(200)
                    .body(axum::body::Body::from(response.data.as_ssz_bytes()))
                    .map(add_ssz_content_type_header)
                    .map_err(|e| ApiError::server_error(format!("failed to create response: {e}"))),
                _ => Ok(Json(&response).into_response()),
            }
        })
        .await
}

async fn get_beacon_state_validators<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(state_id): Path<StateId>,
    MultiKeyQuery(query): MultiKeyQuery<api_types::ValidatorsQuery>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let priority = if let StateId(eth2::types::StateId::Head) = state_id {
        Priority::P0
    } else {
        Priority::P1
    };
    state
        .task_spawner()
        .blocking_json_task(priority, move || {
            crate::validators::get_beacon_state_validators(
                state_id,
                chain,
                &query.id,
                &query.status,
            )
        })
        .await
}

async fn post_beacon_state_validators<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(state_id): Path<StateId>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let query: ValidatorsRequestBody = json_body(&headers, body).await?;
    let priority = if let StateId(eth2::types::StateId::Head) = state_id {
        Priority::P0
    } else {
        Priority::P1
    };
    state
        .task_spawner()
        .blocking_json_task(priority, move || {
            crate::validators::get_beacon_state_validators(
                state_id,
                chain,
                &query.ids,
                &query.statuses,
            )
        })
        .await
}

async fn get_beacon_state_validators_id<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path((state_id, validator_id)): Path<(StateId, ValidatorId)>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let priority = if let StateId(eth2::types::StateId::Head) = state_id {
        Priority::P0
    } else {
        Priority::P1
    };
    state
        .task_spawner()
        .blocking_json_task(priority, move || {
            let (data, execution_optimistic, finalized) = state_id
                .map_state_and_execution_optimistic_and_finalized(
                    &chain,
                    |state, execution_optimistic, finalized| {
                        let index_opt = match &validator_id {
                            ValidatorId::PublicKey(pubkey) => {
                                pubkey_to_validator_index(&chain, state, pubkey).map_err(|e| {
                                    ApiError::not_found(format!(
                                        "unable to access pubkey cache: {e:?}",
                                    ))
                                })?
                            }
                            ValidatorId::Index(index) => Some(*index as usize),
                        };

                        Ok((
                            index_opt
                                .and_then(|index| {
                                    let validator = state.validators().get(index)?;
                                    let balance = *state.balances().get(index)?;
                                    let epoch = state.current_epoch();
                                    let far_future_epoch = chain.spec.far_future_epoch;

                                    Some(api_types::ValidatorData {
                                        index: index as u64,
                                        balance,
                                        status: api_types::ValidatorStatus::from_validator(
                                            validator,
                                            epoch,
                                            far_future_epoch,
                                        ),
                                        validator: validator.clone(),
                                    })
                                })
                                .ok_or_else(|| {
                                    ApiError::not_found(format!(
                                        "unknown validator: {}",
                                        validator_id
                                    ))
                                })?,
                            execution_optimistic,
                            finalized,
                        ))
                    },
                )?;
            Ok(api_types::ExecutionOptimisticFinalizedResponse {
                data,
                execution_optimistic: Some(execution_optimistic),
                finalized: Some(finalized),
            })
        })
        .await
}

async fn get_beacon_state_committees<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(state_id): Path<StateId>,
    Query(query): Query<api_types::CommitteesQuery>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let (data, execution_optimistic, finalized) = state_id
                .map_state_and_execution_optimistic_and_finalized(
                    &chain,
                    |state, execution_optimistic, finalized| {
                        let current_epoch = state.current_epoch();
                        let epoch = query.epoch.unwrap_or(current_epoch);

                        let decision_slot =
                            (epoch.saturating_sub(2u64)).end_slot(T::EthSpec::slots_per_epoch());
                        let shuffling_id = if let Ok(Some(shuffling_decision_block)) =
                            chain.block_root_at_slot(decision_slot, WhenSlotSkipped::Prev)
                        {
                            Some(AttestationShufflingId {
                                shuffling_epoch: epoch,
                                shuffling_decision_block,
                            })
                        } else {
                            None
                        };

                        let maybe_cached_shuffling =
                            if let Some(shuffling_id) = shuffling_id.as_ref() {
                                chain
                                    .shuffling_cache
                                    .try_write_for(std::time::Duration::from_secs(1))
                                    .and_then(|mut cache_write| cache_write.get(shuffling_id))
                                    .and_then(|cache_item| cache_item.wait().ok())
                            } else {
                                None
                            };

                        let committee_cache = if let Some(shuffling) = maybe_cached_shuffling {
                            shuffling
                        } else {
                            let possibly_built_cache =
                                match RelativeEpoch::from_epoch(current_epoch, epoch) {
                                    Ok(relative_epoch)
                                        if state.committee_cache_is_initialized(relative_epoch) =>
                                    {
                                        state.committee_cache(relative_epoch).cloned()
                                    }
                                    _ => CommitteeCache::initialized(state, epoch, &chain.spec),
                                }
                                .map_err(|e| match e {
                                    BeaconStateError::EpochOutOfBounds => {
                                        let max_sprp =
                                            T::EthSpec::slots_per_historical_root() as u64;
                                        let first_subsequent_restore_point_slot = ((epoch
                                            .start_slot(T::EthSpec::slots_per_epoch())
                                            / max_sprp)
                                            + 1)
                                            * max_sprp;
                                        if epoch < current_epoch {
                                            ApiError::bad_request(format!(
                                                "epoch out of bounds, \
                                                 try state at slot {}",
                                                first_subsequent_restore_point_slot,
                                            ))
                                        } else {
                                            ApiError::bad_request(
                                                "epoch out of bounds, too far in future",
                                            )
                                        }
                                    }
                                    _ => ApiError::unhandled_error(BeaconChainError::from(e)),
                                })?;

                            if chain.config.shuffling_cache_size
                                != beacon_chain::shuffling_cache::DEFAULT_CACHE_SIZE
                                && let Some(shuffling_id) = shuffling_id
                                && let Some(mut cache_write) = chain
                                    .shuffling_cache
                                    .try_write_for(std::time::Duration::from_secs(1))
                            {
                                cache_write
                                    .insert_committee_cache(shuffling_id, &possibly_built_cache);
                            }

                            possibly_built_cache
                        };

                        let slots = query.slot.map(|slot| vec![slot]).unwrap_or_else(|| {
                            epoch.slot_iter(T::EthSpec::slots_per_epoch()).collect()
                        });

                        let indices = query.index.map(|index| vec![index]).unwrap_or_else(|| {
                            (0..committee_cache.committees_per_slot()).collect()
                        });

                        let mut response = Vec::with_capacity(slots.len() * indices.len());

                        for slot in slots {
                            if slot.epoch(T::EthSpec::slots_per_epoch()) != epoch {
                                return Err(ApiError::bad_request(format!(
                                    "{} is not in epoch {}",
                                    slot, epoch
                                )));
                            }

                            for &index in &indices {
                                let committee = committee_cache
                                    .get_beacon_committee(slot, index)
                                    .ok_or_else(|| {
                                    ApiError::bad_request(format!(
                                        "committee index {} does not exist in epoch {}",
                                        index, epoch
                                    ))
                                })?;

                                response.push(api_types::CommitteeData {
                                    index,
                                    slot,
                                    validators: committee
                                        .committee
                                        .iter()
                                        .map(|i| *i as u64)
                                        .collect(),
                                });
                            }
                        }

                        Ok((response, execution_optimistic, finalized))
                    },
                )?;
            Ok(api_types::ExecutionOptimisticFinalizedResponse {
                data,
                execution_optimistic: Some(execution_optimistic),
                finalized: Some(finalized),
            })
        })
        .await
}

async fn get_beacon_state_sync_committees<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(state_id): Path<StateId>,
    Query(query): Query<api_types::SyncCommitteesQuery>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let (sync_committee, execution_optimistic, finalized) = state_id
                .map_state_and_execution_optimistic_and_finalized(
                    &chain,
                    |state, execution_optimistic, finalized| {
                        let current_epoch = state.current_epoch();
                        let epoch = query.epoch.unwrap_or(current_epoch);
                        Ok((
                            state
                                .get_built_sync_committee(epoch, &chain.spec)
                                .cloned()
                                .map_err(|e| match e {
                                    BeaconStateError::SyncCommitteeNotKnown { .. } => {
                                        ApiError::bad_request(format!(
                                            "state at epoch {} has no \
                                             sync committee for epoch {}",
                                            current_epoch, epoch
                                        ))
                                    }
                                    BeaconStateError::IncorrectStateVariant => {
                                        ApiError::bad_request(format!(
                                            "state at epoch {} is not activated for Altair",
                                            current_epoch,
                                        ))
                                    }
                                    e => ApiError::beacon_state_error(e),
                                })?,
                            execution_optimistic,
                            finalized,
                        ))
                    },
                )?;

            let validators = chain
                .validator_indices(sync_committee.pubkeys.iter())
                .map_err(ApiError::unhandled_error)?;

            let validator_aggregates = validators
                .chunks_exact(T::EthSpec::sync_subcommittee_size())
                .map(|indices| api_types::SyncSubcommittee {
                    indices: indices.to_vec(),
                })
                .collect();

            let response = api_types::SyncCommitteeByValidatorIndices {
                validators,
                validator_aggregates,
            };

            Ok(api_types::GenericResponse::from(response)
                .add_execution_optimistic_finalized(execution_optimistic, finalized))
        })
        .await
}

async fn get_beacon_state_randao<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(state_id): Path<StateId>,
    Query(query): Query<api_types::RandaoQuery>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let (randao, execution_optimistic, finalized) = state_id
                .map_state_and_execution_optimistic_and_finalized(
                    &chain,
                    |state, execution_optimistic, finalized| {
                        let epoch = query.epoch.unwrap_or_else(|| state.current_epoch());
                        let randao = *state.get_randao_mix(epoch).map_err(|e| {
                            ApiError::bad_request(format!("epoch out of range: {e:?}"))
                        })?;
                        Ok((randao, execution_optimistic, finalized))
                    },
                )?;
            Ok(
                api_types::GenericResponse::from(api_types::RandaoMix { randao })
                    .add_execution_optimistic_finalized(execution_optimistic, finalized),
            )
        })
        .await
}

async fn get_beacon_state_pending_deposits<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(state_id): Path<StateId>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let accept = accept_header(&headers);
    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            let (data, execution_optimistic, finalized, fork_name) = state_id
                .map_state_and_execution_optimistic_and_finalized(
                    &chain,
                    |state, execution_optimistic, finalized| {
                        let Ok(deposits) = state.pending_deposits() else {
                            return Err(ApiError::bad_request(
                                "Pending deposits not found".to_string(),
                            ));
                        };
                        Ok((
                            deposits.clone(),
                            execution_optimistic,
                            finalized,
                            state.fork_name_unchecked(),
                        ))
                    },
                )?;
            match accept {
                Some(api_types::Accept::Ssz) => Response::builder()
                    .status(200)
                    .body(axum::body::Body::from(data.as_ssz_bytes()))
                    .map(add_ssz_content_type_header)
                    .map(|resp| add_consensus_version_header(resp, fork_name))
                    .map_err(|e| ApiError::server_error(format!("failed to create response: {e}"))),
                _ => execution_optimistic_finalized_beacon_response(
                    ResponseIncludesVersion::Yes(fork_name),
                    execution_optimistic,
                    finalized,
                    data,
                )
                .map(|res| Json(res).into_response())
                .map(|resp| add_consensus_version_header(resp, fork_name)),
            }
        })
        .await
}

async fn get_beacon_state_pending_partial_withdrawals<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(state_id): Path<StateId>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let accept = accept_header(&headers);
    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            let (data, execution_optimistic, finalized, fork_name) = state_id
                .map_state_and_execution_optimistic_and_finalized(
                    &chain,
                    |state, execution_optimistic, finalized| {
                        let Ok(withdrawals) = state.pending_partial_withdrawals() else {
                            return Err(ApiError::bad_request(
                                "Pending withdrawals not found".to_string(),
                            ));
                        };
                        Ok((
                            withdrawals.clone(),
                            execution_optimistic,
                            finalized,
                            state.fork_name_unchecked(),
                        ))
                    },
                )?;
            match accept {
                Some(api_types::Accept::Ssz) => Response::builder()
                    .status(200)
                    .body(axum::body::Body::from(data.as_ssz_bytes()))
                    .map(add_ssz_content_type_header)
                    .map(|resp| add_consensus_version_header(resp, fork_name))
                    .map_err(|e| ApiError::server_error(format!("failed to create response: {e}"))),
                _ => execution_optimistic_finalized_beacon_response(
                    ResponseIncludesVersion::Yes(fork_name),
                    execution_optimistic,
                    finalized,
                    data,
                )
                .map(|res| Json(res).into_response())
                .map(|resp| add_consensus_version_header(resp, fork_name)),
            }
        })
        .await
}

async fn get_beacon_state_pending_consolidations<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(state_id): Path<StateId>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let accept = accept_header(&headers);
    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            let (data, execution_optimistic, finalized, fork_name) = state_id
                .map_state_and_execution_optimistic_and_finalized(
                    &chain,
                    |state, execution_optimistic, finalized| {
                        let Ok(consolidations) = state.pending_consolidations() else {
                            return Err(ApiError::bad_request(
                                "Pending consolidations not found".to_string(),
                            ));
                        };
                        Ok((
                            consolidations.clone(),
                            execution_optimistic,
                            finalized,
                            state.fork_name_unchecked(),
                        ))
                    },
                )?;
            match accept {
                Some(api_types::Accept::Ssz) => Response::builder()
                    .status(200)
                    .body(axum::body::Body::from(data.as_ssz_bytes()))
                    .map(add_ssz_content_type_header)
                    .map(|resp| add_consensus_version_header(resp, fork_name))
                    .map_err(|e| ApiError::server_error(format!("failed to create response: {e}"))),
                _ => execution_optimistic_finalized_beacon_response(
                    ResponseIncludesVersion::Yes(fork_name),
                    execution_optimistic,
                    finalized,
                    data,
                )
                .map(|res| Json(res).into_response())
                .map(|resp| add_consensus_version_header(resp, fork_name)),
            }
        })
        .await
}

async fn get_beacon_state_proposer_lookahead<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(state_id): Path<StateId>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let accept = accept_header(&headers);
    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            let (data, execution_optimistic, finalized, fork_name) = state_id
                .map_state_and_execution_optimistic_and_finalized(
                    &chain,
                    |state, execution_optimistic, finalized| {
                        let Ok(lookahead) = state.proposer_lookahead() else {
                            return Err(ApiError::bad_request(
                                "Proposer lookahead is not available before Fulu".to_string(),
                            ));
                        };
                        Ok((
                            lookahead.clone(),
                            execution_optimistic,
                            finalized,
                            state.fork_name_unchecked(),
                        ))
                    },
                )?;
            match accept {
                Some(api_types::Accept::Ssz) => Response::builder()
                    .status(200)
                    .body(axum::body::Body::from(data.as_ssz_bytes()))
                    .map(add_ssz_content_type_header)
                    .map(|resp| add_consensus_version_header(resp, fork_name))
                    .map_err(|e| ApiError::server_error(format!("failed to create response: {e}"))),
                _ => execution_optimistic_finalized_beacon_response(
                    ResponseIncludesVersion::No,
                    execution_optimistic,
                    finalized,
                    data,
                )
                .map(|res| Json(res).into_response())
                .map(|resp| add_consensus_version_header(resp, fork_name)),
            }
        })
        .await
}

// -- Beacon headers --

async fn get_beacon_headers<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Query(query): Query<api_types::HeadersQuery>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let (root, block, execution_optimistic, finalized) =
                match (query.slot, query.parent_root) {
                    (None, None) => {
                        let (cached_head, execution_status) = chain
                            .canonical_head
                            .head_and_execution_status()
                            .map_err(ApiError::unhandled_error)?;
                        (
                            cached_head.head_block_root(),
                            cached_head.snapshot.beacon_block.clone_as_blinded(),
                            execution_status.is_optimistic_or_invalid(),
                            false,
                        )
                    }
                    (None, Some(parent_root)) => {
                        let (parent, execution_optimistic, _parent_finalized) =
                            BlockId::from_root(parent_root).blinded_block(&chain)?;
                        let (root, _slot) = chain
                            .forwards_iter_block_roots(parent.slot())
                            .map_err(ApiError::unhandled_error)?
                            .find(|res| res.as_ref().is_ok_and(|(root, _)| *root != parent_root))
                            .transpose()
                            .map_err(ApiError::unhandled_error)?
                            .ok_or_else(|| {
                                ApiError::not_found(format!(
                                    "child of block with root {}",
                                    parent_root
                                ))
                            })?;
                        BlockId::from_root(root).blinded_block(&chain).map(
                            |(block, _execution_optimistic, finalized)| {
                                (root, block, execution_optimistic, finalized)
                            },
                        )?
                    }
                    (Some(slot), parent_root_opt) => {
                        let (root, execution_optimistic, finalized) =
                            BlockId::from_slot(slot).root(&chain)?;
                        let (block, _execution_optimistic, _finalized) =
                            BlockId::from_root(root).blinded_block(&chain)?;
                        if let Some(parent_root) = parent_root_opt
                            && block.parent_root() != parent_root
                        {
                            return Err(ApiError::not_found(format!(
                                "no canonical block at slot {} with parent root {}",
                                slot, parent_root
                            )));
                        }
                        (root, block, execution_optimistic, finalized)
                    }
                };

            let data = api_types::BlockHeaderData {
                root,
                canonical: true,
                header: api_types::BlockHeaderAndSignature {
                    message: block.message().block_header(),
                    signature: block.signature().clone().into(),
                },
            };

            Ok(api_types::GenericResponse::from(vec![data])
                .add_execution_optimistic_finalized(execution_optimistic, finalized))
        })
        .await
}

async fn get_beacon_headers_block_id<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(block_id): Path<BlockId>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let (root, execution_optimistic, finalized) = block_id.root(&chain)?;
            let (block, _execution_optimistic, _finalized) =
                BlockId::from_root(root).blinded_block(&chain)?;

            let canonical = chain
                .block_root_at_slot(block.slot(), WhenSlotSkipped::None)
                .map_err(ApiError::unhandled_error)?
                .is_some_and(|canonical| root == canonical);

            let data = api_types::BlockHeaderData {
                root,
                canonical,
                header: api_types::BlockHeaderAndSignature {
                    message: block.message().block_header(),
                    signature: block.signature().clone().into(),
                },
            };

            Ok(api_types::ExecutionOptimisticFinalizedResponse {
                execution_optimistic: Some(execution_optimistic),
                finalized: Some(finalized),
                data,
            })
        })
        .await
}

// -- Block publishing --

async fn post_beacon_blocks_v1<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let network_tx = state.network_tx()?;
    let duplicate_status_code = state.duplicate_block_status_code;
    let consensus_version = consensus_version_header(&headers)?;
    let is_ssz = headers
        .get(CONTENT_TYPE_HEADER)
        .is_some_and(|ct| ct.as_bytes() == SSZ_CONTENT_TYPE_HEADER.as_bytes());

    state
        .task_spawner()
        .spawn_async_with_rejection(Priority::P0, async move {
            let request = if is_ssz {
                PublishBlockRequest::<T::EthSpec>::from_ssz_bytes(&body, consensus_version)
                    .map_err(|e| ApiError::bad_request(format!("invalid SSZ: {e:?}")))?
            } else {
                let value: serde_json::Value = serde_json::from_slice(&body)
                    .map_err(|e| ApiError::bad_request(format!("invalid JSON: {e:?}")))?;
                PublishBlockRequest::<T::EthSpec>::context_deserialize(&value, consensus_version)
                    .map_err(|e| ApiError::bad_request(format!("invalid JSON: {e:?}")))?
            };
            let (provenanced_block, signed_envelope) =
                ProvenancedBlock::local_from_publish_request(request);
            publish_blocks::publish_block(
                None,
                provenanced_block,
                signed_envelope,
                chain,
                &network_tx,
                BroadcastValidation::default(),
                duplicate_status_code,
            )
            .await
        })
        .await
}

async fn post_beacon_blocks_v2<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Query(validation_level): Query<api_types::BroadcastValidationQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let network_tx = state.network_tx()?;
    let duplicate_status_code = state.duplicate_block_status_code;
    let consensus_version = consensus_version_header(&headers)?;
    let is_ssz = headers
        .get(CONTENT_TYPE_HEADER)
        .is_some_and(|ct| ct.as_bytes() == SSZ_CONTENT_TYPE_HEADER.as_bytes());

    state
        .task_spawner()
        .spawn_async_with_rejection(Priority::P0, async move {
            let request = if is_ssz {
                PublishBlockRequest::<T::EthSpec>::from_ssz_bytes(&body, consensus_version)
                    .map_err(|e| ApiError::bad_request(format!("invalid SSZ: {e:?}")))?
            } else {
                let value: serde_json::Value = serde_json::from_slice(&body)
                    .map_err(|e| ApiError::bad_request(format!("invalid JSON: {e:?}")))?;
                PublishBlockRequest::<T::EthSpec>::context_deserialize(&value, consensus_version)
                    .map_err(|e| ApiError::bad_request(format!("invalid JSON: {e:?}")))?
            };
            let (provenanced_block, signed_envelope) =
                ProvenancedBlock::local_from_publish_request(request);
            publish_blocks::publish_block(
                None,
                provenanced_block,
                signed_envelope,
                chain,
                &network_tx,
                validation_level.broadcast_validation,
                duplicate_status_code,
            )
            .await
        })
        .await
}

async fn post_beacon_blinded_blocks_v1<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let network_tx = state.network_tx()?;
    let duplicate_status_code = state.duplicate_block_status_code;
    let is_ssz = headers
        .get(CONTENT_TYPE_HEADER)
        .is_some_and(|ct| ct.as_bytes() == SSZ_CONTENT_TYPE_HEADER.as_bytes());

    state
        .task_spawner()
        .spawn_async_with_rejection(Priority::P0, async move {
            let block = if is_ssz {
                SignedBlindedBeaconBlock::<T::EthSpec>::from_ssz_bytes(&body, &chain.spec)
                    .map(Arc::new)
                    .map_err(|e| ApiError::bad_request(format!("invalid SSZ: {e:?}")))?
            } else {
                let block: SignedBlindedBeaconBlock<T::EthSpec> = serde_json::from_slice(&body)
                    .map_err(|e| ApiError::bad_request(format!("body deserialize error: {e:?}")))?;
                Arc::new(block)
            };
            publish_blocks::publish_blinded_block(
                block,
                chain,
                &network_tx,
                BroadcastValidation::default(),
                duplicate_status_code,
            )
            .await
        })
        .await
}

async fn post_beacon_blinded_blocks_v2<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Query(validation_level): Query<api_types::BroadcastValidationQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let network_tx = state.network_tx()?;
    let duplicate_status_code = state.duplicate_block_status_code;
    let consensus_version = consensus_version_header(&headers)?;
    let is_ssz = headers
        .get(CONTENT_TYPE_HEADER)
        .is_some_and(|ct| ct.as_bytes() == SSZ_CONTENT_TYPE_HEADER.as_bytes());

    state
        .task_spawner()
        .spawn_async_with_rejection(Priority::P0, async move {
            let block = if is_ssz {
                SignedBlindedBeaconBlock::<T::EthSpec>::from_ssz_bytes(&body, &chain.spec)
                    .map(Arc::new)
                    .map_err(|e| ApiError::bad_request(format!("invalid SSZ: {e:?}")))?
            } else {
                let value: serde_json::Value = serde_json::from_slice(&body)
                    .map_err(|e| ApiError::bad_request(format!("invalid JSON: {e:?}")))?;
                SignedBlindedBeaconBlock::<T::EthSpec>::context_deserialize(
                    &value,
                    consensus_version,
                )
                .map(Arc::new)
                .map_err(|e| ApiError::bad_request(format!("invalid JSON: {e:?}")))?
            };
            publish_blocks::publish_blinded_block(
                block,
                chain,
                &network_tx,
                validation_level.broadcast_validation,
                duplicate_status_code,
            )
            .await
        })
        .await
}

// -- Block retrieval --

async fn get_beacon_block<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path((version_str, block_id)): Path<(String, BlockId)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let endpoint_version = parse_endpoint_version(&version_str)?;
    let accept = accept_header(&headers);
    let chain = state.chain()?;

    state
        .task_spawner()
        .spawn_async_with_rejection(Priority::P1, async move {
            let (block, execution_optimistic, finalized) = block_id.full_block(&chain).await?;
            let fork_name = block
                .fork_name(&chain.spec)
                .map_err(inconsistent_fork_rejection)?;

            let require_version = match endpoint_version {
                V1 => ResponseIncludesVersion::No,
                V2 => ResponseIncludesVersion::Yes(fork_name),
                _ => return Err(unsupported_version_rejection(endpoint_version)),
            };

            match accept {
                Some(api_types::Accept::Ssz) => axum::http::Response::builder()
                    .status(200)
                    .body(axum::body::Body::from(block.as_ssz_bytes()))
                    .map(|res| add_ssz_content_type_header(res.into_response()))
                    .map_err(|e| {
                        ApiError::server_error(format!("failed to create response: {}", e))
                    }),
                _ => execution_optimistic_finalized_beacon_response(
                    require_version,
                    execution_optimistic,
                    finalized,
                    block,
                )
                .map(|res| Json(res).into_response()),
            }
            .map(|resp| add_consensus_version_header(resp, fork_name))
        })
        .await
}

async fn get_beacon_block_root<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(block_id): Path<BlockId>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let priority = if let BlockId(eth2::types::BlockId::Head) = block_id {
        Priority::P0
    } else {
        Priority::P1
    };
    state
        .task_spawner()
        .blocking_json_task(priority, move || {
            let (block_root, execution_optimistic, finalized) = block_id.root(&chain)?;
            Ok(
                api_types::GenericResponse::from(api_types::RootData::from(block_root))
                    .add_execution_optimistic_finalized(execution_optimistic, finalized),
            )
        })
        .await
}

async fn get_beacon_block_attestations<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path((version_str, block_id)): Path<(String, BlockId)>,
) -> Result<Response, ApiError> {
    let endpoint_version = parse_endpoint_version(&version_str)?;
    let chain = state.chain()?;

    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            let (block, execution_optimistic, finalized) = block_id.blinded_block(&chain)?;
            let fork_name = block
                .fork_name(&chain.spec)
                .map_err(inconsistent_fork_rejection)?;
            let atts = block
                .message()
                .body()
                .attestations()
                .map(|att| att.clone_as_attestation())
                .collect::<Vec<_>>();

            let require_version = match endpoint_version {
                V1 => ResponseIncludesVersion::No,
                V2 => ResponseIncludesVersion::Yes(fork_name),
                _ => return Err(unsupported_version_rejection(endpoint_version)),
            };

            let res = execution_optimistic_finalized_beacon_response(
                require_version,
                execution_optimistic,
                finalized,
                &atts,
            )?;
            Ok(add_consensus_version_header(
                Json(res).into_response(),
                fork_name,
            ))
        })
        .await
}

async fn get_beacon_blinded_block<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(block_id): Path<BlockId>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let accept = accept_header(&headers);
    let chain = state.chain()?;

    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            let (block, execution_optimistic, finalized) = block_id.blinded_block(&chain)?;
            let fork_name = block
                .fork_name(&chain.spec)
                .map_err(inconsistent_fork_rejection)?;

            match accept {
                Some(api_types::Accept::Ssz) => axum::http::Response::builder()
                    .status(200)
                    .body(axum::body::Body::from(block.as_ssz_bytes()))
                    .map(|res| add_ssz_content_type_header(res.into_response()))
                    .map_err(|e| {
                        ApiError::server_error(format!("failed to create response: {}", e))
                    }),
                _ => execution_optimistic_finalized_beacon_response(
                    ResponseIncludesVersion::Yes(fork_name),
                    execution_optimistic,
                    finalized,
                    block,
                )
                .map(|res| Json(res).into_response()),
            }
            .map(|resp| add_consensus_version_header(resp, fork_name))
        })
        .await
}

async fn get_blob_sidecars<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(block_id): Path<BlockId>,
    MultiKeyQuery(indices): MultiKeyQuery<api_types::BlobIndicesQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let accept = accept_header(&headers);
    let chain = state.chain()?;

    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            let (block, blob_sidecar_list_filtered, execution_optimistic, finalized) =
                block_id.get_blinded_block_and_blob_list_filtered(indices, &chain)?;
            let fork_name = block
                .fork_name(&chain.spec)
                .map_err(inconsistent_fork_rejection)?;

            match accept {
                Some(api_types::Accept::Ssz) => axum::http::Response::builder()
                    .status(200)
                    .body(axum::body::Body::from(
                        blob_sidecar_list_filtered.as_ssz_bytes(),
                    ))
                    .map(|res| add_ssz_content_type_header(res.into_response()))
                    .map_err(|e| {
                        ApiError::server_error(format!("failed to create response: {}", e))
                    }),
                _ => {
                    let res = execution_optimistic_finalized_beacon_response(
                        ResponseIncludesVersion::Yes(fork_name),
                        execution_optimistic,
                        finalized,
                        &blob_sidecar_list_filtered,
                    )?;
                    Ok(Json(res).into_response())
                }
            }
            .map(|resp| add_consensus_version_header(resp, fork_name))
        })
        .await
}

async fn get_blobs<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(block_id): Path<BlockId>,
    MultiKeyQuery(versioned_hashes): MultiKeyQuery<api_types::BlobsVersionedHashesQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let accept = accept_header(&headers);
    let chain = state.chain()?;

    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            let response = block_id.get_blobs_by_versioned_hashes(versioned_hashes, &chain)?;

            match accept {
                Some(api_types::Accept::Ssz) => axum::http::Response::builder()
                    .status(200)
                    .body(axum::body::Body::from(response.data.as_ssz_bytes()))
                    .map(|res| add_ssz_content_type_header(res.into_response()))
                    .map_err(|e| {
                        ApiError::server_error(format!("failed to create response: {}", e))
                    }),
                _ => {
                    let res = execution_optimistic_finalized_beacon_response(
                        ResponseIncludesVersion::No,
                        response.metadata.execution_optimistic.unwrap_or(false),
                        response.metadata.finalized.unwrap_or(false),
                        response.data,
                    )?;
                    Ok(Json(res).into_response())
                }
            }
        })
        .await
}

// The rest of the handler functions will be added in the next write operation.
// This file is being built incrementally due to size constraints.

// ── CORS ─────────────────────────────────────────────────────────────────────

// -- Pool routes --

async fn post_beacon_pool_attestations_v2<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let network_tx = state.network_tx()?;
    let attestations: Vec<SingleAttestation> = json_body(&headers, body).await?;
    let _fork_name = optional_consensus_version_header(&headers);
    let task_spawner = state.task_spawner();
    let result = crate::publish_attestations::publish_attestations(
        task_spawner,
        chain,
        attestations,
        network_tx,
        true,
    )
    .await;
    result.map(|()| Json(()).into_response())
}

async fn get_beacon_pool_attestations_v1<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Query(query): Query<api_types::AttestationPoolQuery>,
) -> Result<Response, ApiError> {
    get_beacon_pool_attestations_inner::<T>(state, V1, query).await
}

async fn get_beacon_pool_attestations_v2<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Query(query): Query<api_types::AttestationPoolQuery>,
) -> Result<Response, ApiError> {
    get_beacon_pool_attestations_inner::<T>(state, V2, query).await
}

async fn get_beacon_pool_attestations_inner<T: BeaconChainTypes>(
    state: SharedState<T>,
    endpoint_version: api_types::EndpointVersion,
    query: api_types::AttestationPoolQuery,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            let query_filter = |data: &AttestationData, committee_indices: HashSet<u64>| {
                let dominated_by_slot = query.slot.is_none_or(|slot| data.slot == slot);
                let dominated_by_index = query
                    .committee_index
                    .is_none_or(|index| committee_indices.contains(&index));
                dominated_by_slot && dominated_by_index
            };

            let mut attestations = chain.op_pool.get_filtered_attestations(query_filter);
            attestations.extend(
                chain
                    .naive_aggregation_pool
                    .read()
                    .iter()
                    .filter(|att| {
                        let dominated_by_slot =
                            query.slot.is_none_or(|slot| att.data().slot == slot);
                        let dominated_by_index = query
                            .committee_index
                            .is_none_or(|index| att.get_committee_indices_map().contains(&index));
                        dominated_by_slot && dominated_by_index
                    })
                    .cloned(),
            );
            let current_slot = chain
                .slot_clock
                .now()
                .ok_or(ApiError::server_error("unable to read slot clock"))?;
            let fork_name = chain.spec.fork_name_at_slot::<T::EthSpec>(current_slot);
            let attestations = attestations
                .into_iter()
                .filter(|att| {
                    (fork_name.electra_enabled() && matches!(att, Attestation::Electra(_)))
                        || (!fork_name.electra_enabled() && matches!(att, Attestation::Base(_)))
                })
                .collect::<Vec<_>>();

            let require_version = match endpoint_version {
                V1 => ResponseIncludesVersion::No,
                V2 => ResponseIncludesVersion::Yes(fork_name),
                _ => return Err(unsupported_version_rejection(endpoint_version)),
            };

            let res = beacon_response(require_version, &attestations);
            Ok(add_consensus_version_header(
                Json(res).into_response(),
                fork_name,
            ))
        })
        .await
}

async fn post_beacon_pool_attester_slashings<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(_version_str): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let network_tx = state.network_tx()?;
    let slashing: AttesterSlashing<T::EthSpec> = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            let outcome = chain
                .verify_attester_slashing_for_gossip(slashing.clone())
                .map_err(|e| {
                    ApiError::object_invalid(format!("gossip verification failed: {:?}", e))
                })?;

            chain
                .validator_monitor
                .read()
                .register_api_attester_slashing(slashing.to_ref());

            if let ObservationOutcome::New(slashing) = outcome {
                publish_pubsub_message(
                    &network_tx,
                    PubsubMessage::AttesterSlashing(Box::new(slashing.clone().into_inner())),
                )?;
                chain.import_attester_slashing(slashing);
            }

            Ok(())
        })
        .await
}

async fn get_beacon_pool_attester_slashings<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(version_str): Path<String>,
) -> Result<Response, ApiError> {
    let endpoint_version = parse_endpoint_version(&version_str)?;
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            let slashings = chain.op_pool.get_all_attester_slashings();

            let current_slot = chain
                .slot_clock
                .now()
                .ok_or(ApiError::server_error("unable to read slot clock"))?;
            let fork_name = chain.spec.fork_name_at_slot::<T::EthSpec>(current_slot);
            let slashings = slashings
                .into_iter()
                .filter(|slashing| {
                    (fork_name.electra_enabled()
                        && matches!(slashing, AttesterSlashing::Electra(_)))
                        || (!fork_name.electra_enabled()
                            && matches!(slashing, AttesterSlashing::Base(_)))
                })
                .collect::<Vec<_>>();

            let require_version = match endpoint_version {
                V1 => ResponseIncludesVersion::No,
                V2 => ResponseIncludesVersion::Yes(fork_name),
                _ => return Err(unsupported_version_rejection(endpoint_version)),
            };

            let res = beacon_response(require_version, &slashings);
            Ok(add_consensus_version_header(
                Json(res).into_response(),
                fork_name,
            ))
        })
        .await
}

async fn get_beacon_pool_proposer_slashings<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let slashings = chain.op_pool.get_all_proposer_slashings();
            Ok(api_types::GenericResponse::from(slashings))
        })
        .await
}

async fn post_beacon_pool_proposer_slashings<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let network_tx = state.network_tx()?;
    let slashing: ProposerSlashing = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            let outcome = chain
                .verify_proposer_slashing_for_gossip(slashing.clone())
                .map_err(|e| {
                    ApiError::object_invalid(format!("gossip verification failed: {:?}", e))
                })?;

            chain
                .validator_monitor
                .read()
                .register_api_proposer_slashing(&slashing);

            if let ObservationOutcome::New(slashing) = outcome {
                publish_pubsub_message(
                    &network_tx,
                    PubsubMessage::ProposerSlashing(Box::new(slashing.clone().into_inner())),
                )?;
                chain.import_proposer_slashing(slashing);
            }

            Ok(())
        })
        .await
}

async fn get_beacon_pool_voluntary_exits<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let exits = chain.op_pool.get_all_voluntary_exits();
            Ok(api_types::GenericResponse::from(exits))
        })
        .await
}

async fn post_beacon_pool_voluntary_exits<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let network_tx = state.network_tx()?;
    let exit: SignedVoluntaryExit = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            let outcome = chain
                .verify_voluntary_exit_for_gossip(exit.clone())
                .map_err(|e| {
                    ApiError::object_invalid(format!("gossip verification failed: {:?}", e))
                })?;

            chain
                .validator_monitor
                .read()
                .register_api_voluntary_exit(&exit.message);

            if let ObservationOutcome::New(exit) = outcome {
                publish_pubsub_message(
                    &network_tx,
                    PubsubMessage::VoluntaryExit(Box::new(exit.clone().into_inner())),
                )?;
                chain.import_voluntary_exit(exit);
            }

            Ok(())
        })
        .await
}

async fn post_beacon_pool_sync_committees<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let network_tx = state.network_tx()?;
    let signatures: Vec<SyncCommitteeMessage> = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            sync_committees::process_sync_committee_signatures(signatures, network_tx, &chain)?;
            Ok(api_types::GenericResponse::from(()))
        })
        .await
}

async fn get_beacon_pool_bls_to_execution_changes<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let address_changes = chain.op_pool.get_all_bls_to_execution_changes();
            Ok(api_types::GenericResponse::from(address_changes))
        })
        .await
}

async fn post_beacon_pool_bls_to_execution_changes<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let network_tx = state.network_tx()?;
    let address_changes: Vec<SignedBlsToExecutionChange> = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            let mut failures = vec![];

            for (index, address_change) in address_changes.into_iter().enumerate() {
                let validator_index = address_change.message.validator_index;

                match chain.verify_bls_to_execution_change_for_http_api(address_change) {
                    Ok(ObservationOutcome::New(verified_address_change)) => {
                        let validator_index =
                            verified_address_change.as_inner().message.validator_index;
                        let address = verified_address_change
                            .as_inner()
                            .message
                            .to_execution_address;

                        let received_pre_capella =
                            if chain.current_slot_is_post_capella().unwrap_or(false) {
                                ReceivedPreCapella::No
                            } else {
                                ReceivedPreCapella::Yes
                            };
                        if matches!(received_pre_capella, ReceivedPreCapella::No) {
                            publish_pubsub_message(
                                &network_tx,
                                PubsubMessage::BlsToExecutionChange(Box::new(
                                    verified_address_change.as_inner().clone(),
                                )),
                            )?;
                        }

                        let imported = chain.import_bls_to_execution_change(
                            verified_address_change,
                            received_pre_capella,
                        );

                        info!(
                            %validator_index,
                            ?address,
                            published =
                                matches!(received_pre_capella, ReceivedPreCapella::No),
                            imported,
                            "Processed BLS to execution change"
                        );
                    }
                    Ok(ObservationOutcome::AlreadyKnown) => {
                        debug!(%validator_index, "BLS to execution change already known");
                    }
                    Err(e) => {
                        warn!(
                            validator_index,
                            reason = ?e,
                            source = "HTTP",
                            "Invalid BLS to execution change"
                        );
                        failures.push(api_types::Failure::new(index, format!("invalid: {e:?}")));
                    }
                }
            }

            if failures.is_empty() {
                Ok(())
            } else {
                Err(ApiError::IndexedBadRequest {
                    message: "some BLS to execution changes failed to verify".into(),
                    failures,
                })
            }
        })
        .await
}

async fn post_beacon_pool_payload_attestations<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let network_tx = state.network_tx()?;
    let messages: Vec<PayloadAttestationMessage> = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            let mut failures = vec![];

            for (index, message) in messages.into_iter().enumerate() {
                match chain.import_payload_attestation_message(message.clone()) {
                    Ok(_attestation) => {
                        publish_pubsub_message(
                            &network_tx,
                            PubsubMessage::PayloadAttestation(Box::new(message)),
                        )?;
                    }
                    Err(e) => {
                        failures.push(api_types::Failure::new(index, format!("invalid: {:?}", e)));
                    }
                }
            }

            if failures.is_empty() {
                Ok(())
            } else {
                Err(ApiError::IndexedBadRequest {
                    message: "some payload attestations failed to import".into(),
                    failures,
                })
            }
        })
        .await
}

async fn get_beacon_pool_payload_attestations<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Query(query): Query<api_types::PayloadAttestationPoolQuery>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let attestations = chain.get_all_payload_attestations(query.slot);
            Ok(api_types::GenericResponse::from(attestations))
        })
        .await
}

async fn post_beacon_pool_proposer_preferences<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let network_tx = state.network_tx()?;
    let signed_preferences: SignedProposerPreferences = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            if !chain.spec.is_gloas_scheduled() {
                return Err(ApiError::bad_request("Gloas is not scheduled"));
            }

            let preferences = &signed_preferences.message;
            let proposal_slot = Slot::new(preferences.proposal_slot);
            let validator_index = preferences.validator_index;

            let head_snapshot = chain.canonical_head.cached_head();
            let head_state = &head_snapshot.snapshot.beacon_state;
            let pubkey = head_state
                .validators()
                .get(validator_index as usize)
                .ok_or_else(|| {
                    ApiError::bad_request(format!("Unknown validator index {validator_index}"))
                })?
                .pubkey
                .decompress()
                .map_err(|_| ApiError::bad_request("Invalid validator pubkey"))?;

            let proposal_epoch = proposal_slot.epoch(T::EthSpec::slots_per_epoch());
            let domain = chain.spec.get_domain(
                proposal_epoch,
                Domain::ProposerPreferences,
                &chain.spec.fork_at_epoch(proposal_epoch),
                head_state.genesis_validators_root(),
            );
            let signing_root = preferences.signing_root(domain);
            if !signed_preferences.signature.verify(&pubkey, signing_root) {
                return Err(ApiError::bad_request(
                    "Invalid proposer preferences signature",
                ));
            }

            let inserted = chain.insert_proposer_preferences(signed_preferences.clone());
            if inserted {
                debug!(
                    %proposal_slot,
                    %validator_index,
                    "Inserted proposer preferences via HTTP"
                );
                publish_pubsub_message(
                    &network_tx,
                    PubsubMessage::ProposerPreferences(Box::new(signed_preferences)),
                )?;
            } else {
                debug!(
                    %proposal_slot,
                    %validator_index,
                    "Proposer preferences already known for this slot"
                );
            }

            Ok(())
        })
        .await
}

// -- Beacon rewards routes --

async fn get_beacon_rewards_blocks<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(block_id): Path<BlockId>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let (rewards, execution_optimistic, finalized) =
                standard_block_rewards::compute_beacon_block_rewards(chain, block_id)?;
            Ok(api_types::GenericResponse::from(rewards)
                .add_execution_optimistic_finalized(execution_optimistic, finalized))
        })
        .await
}

async fn post_beacon_rewards_attestations<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(epoch): Path<Epoch>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let validators: Vec<ValidatorId> = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let attestation_rewards = chain
                .compute_attestation_rewards(epoch, validators)
                .map_err(|e| match e {
                    BeaconChainError::MissingBeaconState(root) => {
                        ApiError::not_found(format!("missing state {root:?}"))
                    }
                    BeaconChainError::NoStateForSlot(slot) => {
                        ApiError::not_found(format!("missing state at slot {slot}"))
                    }
                    BeaconChainError::BeaconStateError(BeaconStateError::UnknownValidator(
                        validator_index,
                    )) => ApiError::bad_request(format!("validator is unknown: {validator_index}")),
                    BeaconChainError::ValidatorPubkeyUnknown(pubkey) => {
                        ApiError::bad_request(format!("validator pubkey is unknown: {pubkey:?}"))
                    }
                    e => ApiError::server_error(format!("unexpected error: {:?}", e)),
                })?;
            let execution_optimistic = chain.is_optimistic_or_invalid_head().unwrap_or_default();

            Ok(api_types::GenericResponse::from(attestation_rewards)
                .add_execution_optimistic(execution_optimistic))
        })
        .await
}

async fn post_beacon_rewards_sync_committee<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(block_id): Path<BlockId>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let validators: Vec<ValidatorId> = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let (rewards, execution_optimistic, finalized) =
                sync_committee_rewards::compute_sync_committee_rewards(
                    chain, block_id, validators,
                )?;

            Ok(api_types::GenericResponse::from(rewards)
                .add_execution_optimistic_finalized(execution_optimistic, finalized))
        })
        .await
}

// -- Builder routes --

async fn get_expected_withdrawals_handler<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(state_id): Path<StateId>,
    Query(query): Query<api_types::ExpectedWithdrawalsQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let accept = accept_header(&headers);
    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            let (beacon_state, execution_optimistic, finalized) = state_id.state(&chain)?;
            let proposal_slot = query.proposal_slot.unwrap_or(beacon_state.slot() + 1);
            let withdrawals =
                get_next_withdrawals::<T>(&chain, beacon_state, state_id, proposal_slot)?;

            match accept {
                Some(api_types::Accept::Ssz) => Response::builder()
                    .status(200)
                    .body(withdrawals.as_ssz_bytes().into())
                    .map(add_ssz_content_type_header)
                    .map_err(|e| {
                        ApiError::server_error(format!("failed to create response: {}", e))
                    }),
                _ => Ok(Json(api_types::ExecutionOptimisticFinalizedResponse {
                    data: withdrawals,
                    execution_optimistic: Some(execution_optimistic),
                    finalized: Some(finalized),
                })
                .into_response()),
            }
        })
        .await
}

async fn post_builder_bids<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let network_tx = state.network_tx()?;
    let bid: SignedExecutionPayloadBid<T::EthSpec> = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            use beacon_chain::gloas_verification::ExecutionBidError;

            if !chain.spec.is_gloas_scheduled() {
                return Err(ApiError::bad_request("Gloas is not scheduled"));
            }

            let builder_index = bid.message.builder_index;

            let verified_bid = match chain.verify_execution_bid_for_gossip(bid) {
                Ok(verified) => verified,
                Err(ExecutionBidError::DuplicateBid { .. }) => {
                    debug!(builder_index, "Duplicate execution bid submitted via HTTP");
                    return Ok(());
                }
                Err(e) => {
                    return Err(ApiError::bad_request(format!(
                        "invalid execution bid: {e:?}"
                    )));
                }
            };

            chain.import_execution_bid(&verified_bid);

            publish_pubsub_message(
                &network_tx,
                PubsubMessage::ExecutionBid(Box::new(verified_bid.into_inner())),
            )?;

            Ok(())
        })
        .await
}

// -- Execution payload envelope routes --

async fn get_beacon_execution_payload_envelope<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(block_id): Path<BlockId>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let accept = accept_header(&headers);
    state
        .task_spawner()
        .spawn_async_with_rejection(Priority::P1, async move {
            let (block_root, execution_optimistic, finalized) = block_id.root(&chain)?;

            let envelope = chain
                .get_payload_envelope(&block_root)
                .map_err(ApiError::unhandled_error)?
                .ok_or_else(|| {
                    ApiError::not_found(format!("payload envelope for block {block_root:?}"))
                })?;

            match accept {
                Some(api_types::Accept::Ssz) => Response::builder()
                    .status(200)
                    .body(envelope.as_ssz_bytes().into())
                    .map(add_ssz_content_type_header)
                    .map(|resp| add_consensus_version_header(resp, ForkName::Gloas))
                    .map_err(|e| ApiError::server_error(format!("failed to create response: {e}"))),
                _ => execution_optimistic_finalized_beacon_response(
                    ResponseIncludesVersion::Yes(ForkName::Gloas),
                    execution_optimistic,
                    finalized,
                    envelope,
                )
                .map(|res| Json(res).into_response()),
            }
        })
        .await
}

async fn post_beacon_execution_payload_envelope<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let network_tx = state.network_tx()?;
    let envelope: SignedExecutionPayloadEnvelope<T::EthSpec> = json_body(&headers, body).await?;
    state
        .task_spawner()
        .spawn_async_with_rejection(Priority::P0, async move {
            use beacon_chain::gloas_verification::PayloadEnvelopeError;

            if !chain.spec.is_gloas_scheduled() {
                return Err(ApiError::bad_request("Gloas is not scheduled"));
            }

            let builder_index = envelope.message.builder_index;

            let verified_envelope =
                match chain.verify_payload_envelope_for_gossip(Arc::new(envelope)) {
                    Ok(verified) => verified,
                    Err(PayloadEnvelopeError::PriorToFinalization { .. }) => {
                        debug!(
                            builder_index,
                            "Stale execution payload envelope submitted via HTTP"
                        );
                        return Ok(StatusCode::OK.into_response());
                    }
                    Err(e) => {
                        return Err(ApiError::bad_request(format!(
                            "invalid execution payload envelope: {e:?}"
                        )));
                    }
                };

            publish_pubsub_message(
                &network_tx,
                PubsubMessage::ExecutionPayload(Box::new(verified_envelope.envelope().clone())),
            )?;

            match chain.process_payload_envelope(&verified_envelope).await {
                Ok(el_valid) => {
                    let beacon_block_root = verified_envelope.beacon_block_root();

                    if let Err(e) = chain.apply_payload_envelope_to_fork_choice(&verified_envelope)
                    {
                        warn!(
                            ?beacon_block_root,
                            builder_index,
                            error = ?e,
                            "Failed to import payload envelope to fork choice"
                        );
                    } else if el_valid
                        && let Err(e) = chain
                            .canonical_head
                            .fork_choice_write_lock()
                            .on_valid_execution_payload(beacon_block_root)
                    {
                        warn!(
                            ?beacon_block_root,
                            error = ?e,
                            "Failed to mark envelope payload as valid in fork choice"
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        builder_index,
                        error = ?e,
                        "Failed to process execution payload envelope"
                    );
                }
            }

            Ok(StatusCode::OK.into_response())
        })
        .await
}

// -- Light client routes --

async fn get_beacon_light_client_bootstrap_handler<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(block_root): Path<Hash256>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    state.check_light_client_server()?;
    let chain = state.chain()?;
    let accept = accept_header(&headers);
    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            get_light_client_bootstrap::<T>(chain, &block_root, accept)
        })
        .await
}

async fn get_beacon_light_client_optimistic_update<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    state.check_light_client_server()?;
    let chain = state.chain()?;
    let accept = accept_header(&headers);
    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            let update = chain
                .light_client_server_cache
                .get_latest_optimistic_update()
                .ok_or_else(|| {
                    ApiError::not_found("No LightClientOptimisticUpdate is available")
                })?;

            let fork_name = chain
                .spec
                .fork_name_at_slot::<T::EthSpec>(update.get_slot());
            match accept {
                Some(api_types::Accept::Ssz) => Response::builder()
                    .status(200)
                    .body(update.as_ssz_bytes().into())
                    .map(add_ssz_content_type_header)
                    .map_err(|e| {
                        ApiError::server_error(format!("failed to create response: {}", e))
                    }),
                _ => Ok(Json(beacon_response(
                    ResponseIncludesVersion::Yes(fork_name),
                    update,
                ))
                .into_response()),
            }
            .map(|resp| add_consensus_version_header(resp, fork_name))
        })
        .await
}

async fn get_beacon_light_client_finality_update<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    state.check_light_client_server()?;
    let chain = state.chain()?;
    let accept = accept_header(&headers);
    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            let update = chain
                .light_client_server_cache
                .get_latest_finality_update()
                .ok_or_else(|| ApiError::not_found("No LightClientFinalityUpdate is available"))?;

            let fork_name = chain
                .spec
                .fork_name_at_slot::<T::EthSpec>(update.signature_slot());
            match accept {
                Some(api_types::Accept::Ssz) => Response::builder()
                    .status(200)
                    .body(update.as_ssz_bytes().into())
                    .map(add_ssz_content_type_header)
                    .map_err(|e| {
                        ApiError::server_error(format!("failed to create response: {}", e))
                    }),
                _ => Ok(Json(beacon_response(
                    ResponseIncludesVersion::Yes(fork_name),
                    update,
                ))
                .into_response()),
            }
            .map(|resp| add_consensus_version_header(resp, fork_name))
        })
        .await
}

async fn get_beacon_light_client_updates_handler<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Query(query): Query<LightClientUpdatesQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    state.check_light_client_server()?;
    let chain = state.chain()?;
    let accept = accept_header(&headers);
    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            get_light_client_updates::<T>(chain, query, accept)
        })
        .await
}

// -- Config routes --

async fn get_config_fork_schedule<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let forks = ForkName::list_all()
                .into_iter()
                .filter_map(|fork_name| chain.spec.fork_for_name(fork_name))
                .collect::<Vec<_>>();
            Ok(api_types::GenericResponse::from(forks))
        })
        .await
}

async fn get_config_spec<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            let config_and_preset = ConfigAndPreset::from_chain_spec::<T::EthSpec>(&chain.spec);
            Ok(api_types::GenericResponse::from(config_and_preset))
        })
        .await
}

async fn get_config_deposit_contract<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            Ok(api_types::GenericResponse::from(
                api_types::DepositContractData {
                    address: chain.spec.deposit_contract_address,
                    chain_id: chain.spec.deposit_chain_id,
                },
            ))
        })
        .await
}

// -- Debug routes --

async fn get_debug_data_column_sidecars<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(block_id): Path<BlockId>,
    MultiKeyQuery(indices): MultiKeyQuery<api_types::DataColumnIndicesQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let accept = accept_header(&headers);
    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            let (data_columns, fork_name, execution_optimistic, finalized) =
                block_id.get_data_columns(indices, &chain)?;

            match accept {
                Some(api_types::Accept::Ssz) => Response::builder()
                    .status(200)
                    .body(data_columns.as_ssz_bytes().into())
                    .map(add_ssz_content_type_header)
                    .map_err(|e| {
                        ApiError::server_error(format!("failed to create response: {}", e))
                    }),
                _ => {
                    let res = execution_optimistic_finalized_beacon_response(
                        ResponseIncludesVersion::Yes(fork_name),
                        execution_optimistic,
                        finalized,
                        &data_columns,
                    )?;
                    Ok(Json(res).into_response())
                }
            }
            .map(|resp| add_consensus_version_header(resp, fork_name))
        })
        .await
}

async fn get_debug_beacon_states<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path((_version_str, state_id)): Path<(String, StateId)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let accept = accept_header(&headers);
    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || match accept {
            Some(api_types::Accept::Ssz) => {
                let t = std::time::Instant::now();
                let (beacon_state, _execution_optimistic, _finalized) = state_id.state(&chain)?;
                let fork_name = beacon_state
                    .fork_name(&chain.spec)
                    .map_err(inconsistent_fork_rejection)?;
                let timer = metrics::start_timer(&metrics::HTTP_API_STATE_SSZ_ENCODE_TIMES);
                let response_bytes = beacon_state.as_ssz_bytes();
                drop(timer);
                debug!(
                    total_time_ms = t.elapsed().as_millis(),
                    target_slot = %beacon_state.slot(),
                    "HTTP state load"
                );

                Response::builder()
                    .status(200)
                    .body(response_bytes.into())
                    .map(add_ssz_content_type_header)
                    .map(|resp| add_consensus_version_header(resp, fork_name))
                    .map_err(|e| {
                        ApiError::server_error(format!("failed to create response: {}", e))
                    })
            }
            _ => state_id.map_state_and_execution_optimistic_and_finalized(
                &chain,
                |beacon_state, execution_optimistic, finalized| {
                    let fork_name = beacon_state
                        .fork_name(&chain.spec)
                        .map_err(inconsistent_fork_rejection)?;
                    let res = execution_optimistic_finalized_beacon_response(
                        ResponseIncludesVersion::Yes(fork_name),
                        execution_optimistic,
                        finalized,
                        &beacon_state,
                    )?;
                    Ok(add_consensus_version_header(
                        Json(res).into_response(),
                        fork_name,
                    ))
                },
            ),
        })
        .await
}

async fn get_debug_beacon_heads<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(version_str): Path<String>,
) -> Result<Response, ApiError> {
    let endpoint_version = parse_endpoint_version(&version_str)?;
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let heads = chain
                .heads()
                .into_iter()
                .map(|(root, slot)| {
                    let execution_optimistic = if endpoint_version == V1 {
                        None
                    } else if endpoint_version == V2 {
                        chain
                            .canonical_head
                            .fork_choice_read_lock()
                            .is_optimistic_or_invalid_block(&root)
                            .ok()
                    } else {
                        return Err(unsupported_version_rejection(endpoint_version));
                    };
                    Ok(api_types::ChainHeadData {
                        slot,
                        root,
                        execution_optimistic,
                    })
                })
                .collect::<Result<Vec<_>, ApiError>>();
            Ok(api_types::GenericResponse::from(heads?))
        })
        .await
}

async fn get_debug_fork_choice<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let beacon_fork_choice = chain.canonical_head.fork_choice_read_lock();

            let proto_array = beacon_fork_choice.proto_array().core_proto_array();

            let fork_choice_nodes = proto_array
                .nodes
                .iter()
                .map(|node| {
                    let execution_status = if node.execution_status.is_execution_enabled() {
                        Some(node.execution_status.to_string())
                    } else {
                        None
                    };

                    ForkChoiceNode {
                        slot: node.slot,
                        block_root: node.root,
                        parent_root: node
                            .parent
                            .and_then(|index| proto_array.nodes.get(index))
                            .map(|parent| parent.root),
                        justified_epoch: node.justified_checkpoint.epoch,
                        finalized_epoch: node.finalized_checkpoint.epoch,
                        weight: node.weight,
                        validity: execution_status,
                        execution_block_hash: node
                            .execution_status
                            .block_hash()
                            .map(|block_hash| block_hash.into_root()),
                    }
                })
                .collect::<Vec<_>>();
            Ok(ForkChoice {
                justified_checkpoint: proto_array.justified_checkpoint,
                finalized_checkpoint: proto_array.finalized_checkpoint,
                fork_choice_nodes,
            })
        })
        .await
}

// -- Node routes --

async fn get_node_identity<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let network_globals = state.network_globals()?;
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let enr = network_globals.local_enr();
            let p2p_addresses = enr.multiaddr_p2p_tcp();
            let discovery_addresses = enr.multiaddr_p2p_udp();
            Ok(api_types::GenericResponse::from(api_types::IdentityData {
                peer_id: network_globals.local_peer_id().to_base58(),
                enr,
                p2p_addresses,
                discovery_addresses,
                metadata: from_meta_data::<T::EthSpec>(
                    &network_globals.local_metadata,
                    &chain.spec,
                ),
            }))
        })
        .await
}

async fn get_node_version<T: BeaconChainTypes>(State(_state): State<SharedState<T>>) -> Response {
    Json(api_types::GenericResponse::from(api_types::VersionData {
        version: version_with_platform(),
    }))
    .into_response()
}

async fn get_node_syncing<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let network_globals = state.network_globals()?;

    let el_offline = if let Some(el) = &chain.execution_layer {
        el.is_offline_or_erroring().await
    } else {
        true
    };

    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            let (head, head_execution_status) = chain
                .canonical_head
                .head_and_execution_status()
                .map_err(ApiError::unhandled_error)?;
            let head_slot = head.head_slot();
            let current_slot = chain
                .slot_clock
                .now_or_genesis()
                .ok_or_else(|| ApiError::server_error("Unable to read slot clock"))?;

            let sync_distance = current_slot - head_slot;
            let is_optimistic = head_execution_status.is_optimistic_or_invalid();

            let sync_state = network_globals.sync_state.read();
            let is_synced = sync_state.is_synced()
                || (sync_state.is_stalled() && network_globals.config.target_peers == 0);
            drop(sync_state);

            let syncing_data = api_types::SyncingData {
                is_syncing: !is_synced,
                is_optimistic,
                el_offline,
                head_slot,
                sync_distance,
            };

            Ok(api_types::GenericResponse::from(syncing_data))
        })
        .await
}

async fn get_node_health<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let network_globals = state.network_globals()?;

    let el_offline = if let Some(el) = &chain.execution_layer {
        el.is_offline_or_erroring().await
    } else {
        true
    };

    state
        .task_spawner()
        .blocking_response_task(Priority::P0, move || {
            let is_optimistic = chain
                .is_optimistic_or_invalid_head()
                .map_err(ApiError::unhandled_error)?;

            let is_syncing = !network_globals.sync_state.read().is_synced();

            if el_offline {
                Err(ApiError::service_unavailable("execution layer is offline"))
            } else if is_syncing || is_optimistic {
                Ok(StatusCode::PARTIAL_CONTENT.into_response())
            } else {
                Ok(StatusCode::OK.into_response())
            }
        })
        .await
}

async fn get_node_peers_by_id<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(requested_peer_id): Path<String>,
) -> Result<Response, ApiError> {
    let network_globals = state.network_globals()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let peer_id = PeerId::from_bytes(
                &bs58::decode(requested_peer_id.as_str())
                    .into_vec()
                    .map_err(|e| ApiError::bad_request(format!("invalid peer id: {}", e)))?,
            )
            .map_err(|_| ApiError::bad_request("invalid peer id."))?;

            if let Some(peer_info) = network_globals.peers.read().peer_info(&peer_id) {
                let address = if let Some(multiaddr) = peer_info.seen_multiaddrs().next() {
                    multiaddr.to_string()
                } else if let Some(addr) = peer_info.listening_addresses().first() {
                    addr.to_string()
                } else {
                    String::new()
                };

                if let Some(&dir) = peer_info.connection_direction() {
                    return Ok(api_types::GenericResponse::from(api_types::PeerData {
                        peer_id: peer_id.to_string(),
                        enr: peer_info.enr().map(|enr| enr.to_base64()),
                        last_seen_p2p_address: address,
                        direction: dir.into(),
                        state: peer_info.connection_status().clone().into(),
                    }));
                }
            }
            Err(ApiError::not_found("peer not found."))
        })
        .await
}

async fn get_node_peers<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    MultiKeyQuery(query): MultiKeyQuery<api_types::PeersQuery>,
) -> Result<Response, ApiError> {
    let network_globals = state.network_globals()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let mut peers: Vec<api_types::PeerData> = Vec::new();
            network_globals
                .peers
                .read()
                .peers()
                .for_each(|(peer_id, peer_info)| {
                    let address = if let Some(multiaddr) = peer_info.seen_multiaddrs().next() {
                        multiaddr.to_string()
                    } else if let Some(addr) = peer_info.listening_addresses().first() {
                        addr.to_string()
                    } else {
                        String::new()
                    };

                    if let Some(&dir) = peer_info.connection_direction() {
                        let direction = dir.into();
                        let state = peer_info.connection_status().clone().into();

                        let state_matches = query
                            .state
                            .as_ref()
                            .is_none_or(|states| states.contains(&state));
                        let direction_matches = query
                            .direction
                            .as_ref()
                            .is_none_or(|directions| directions.contains(&direction));

                        if state_matches && direction_matches {
                            peers.push(api_types::PeerData {
                                peer_id: peer_id.to_string(),
                                enr: peer_info.enr().map(|enr| enr.to_base64()),
                                last_seen_p2p_address: address,
                                direction,
                                state,
                            });
                        }
                    }
                });
            Ok(api_types::PeersData {
                meta: api_types::PeersMetaData {
                    count: peers.len() as u64,
                },
                data: peers,
            })
        })
        .await
}

async fn get_node_peer_count<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let network_globals = state.network_globals()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let mut connected: u64 = 0;
            let mut connecting: u64 = 0;
            let mut disconnected: u64 = 0;
            let mut disconnecting: u64 = 0;

            network_globals
                .peers
                .read()
                .peers()
                .for_each(|(_, peer_info)| {
                    let state = api_types::PeerState::from(peer_info.connection_status().clone());
                    match state {
                        api_types::PeerState::Connected => connected += 1,
                        api_types::PeerState::Connecting => connecting += 1,
                        api_types::PeerState::Disconnected => disconnected += 1,
                        api_types::PeerState::Disconnecting => disconnecting += 1,
                    }
                });

            Ok(api_types::GenericResponse::from(api_types::PeerCount {
                connected,
                connecting,
                disconnected,
                disconnecting,
            }))
        })
        .await
}

// -- Validator routes --

async fn get_validator_duties_proposer<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(epoch): Path<Epoch>,
) -> Result<Response, ApiError> {
    state.check_not_syncing()?;
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            proposer_duties::proposer_duties(epoch, &chain)
        })
        .await
}

async fn get_validator_blocks<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path((version_str, slot)): Path<(String, Slot)>,
    Query(query): Query<api_types::ValidatorBlocksQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let endpoint_version = parse_endpoint_version(&version_str)?;
    state.check_not_syncing()?;
    let chain = state.chain()?;
    let accept = accept_header(&headers);
    state
        .task_spawner()
        .spawn_async_with_rejection(Priority::P0, async move {
            debug!(?slot, "Block production request from HTTP API");

            if endpoint_version == V3 {
                produce_block_v3(accept, chain, slot, query).await
            } else {
                produce_block_v2(accept, chain, slot, query).await
            }
        })
        .await
}

async fn get_validator_blinded_blocks<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(slot): Path<Slot>,
    Query(query): Query<api_types::ValidatorBlocksQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    state.check_not_syncing()?;
    let chain = state.chain()?;
    let accept = accept_header(&headers);
    state
        .task_spawner()
        .spawn_async_with_rejection(Priority::P0, async move {
            produce_blinded_block_v2(accept, chain, slot, query).await
        })
        .await
}

async fn get_validator_attestation_data<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Query(query): Query<api_types::ValidatorAttestationDataQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    state.check_not_syncing()?;
    let chain = state.chain()?;
    let accept = accept_header(&headers);
    state
        .task_spawner()
        .blocking_response_task(Priority::P0, move || {
            let current_slot = chain.slot().map_err(ApiError::unhandled_error)?;

            if query.slot > current_slot + 1 {
                return Err(ApiError::bad_request(format!(
                    "request slot {} is more than one slot past the current slot {}",
                    query.slot, current_slot
                )));
            }

            let committee_index = if chain
                .spec
                .fork_name_at_slot::<T::EthSpec>(query.slot)
                .electra_enabled()
            {
                0
            } else {
                query.committee_index
            };

            let attestation_data = chain
                .produce_unaggregated_attestation(query.slot, committee_index)
                .map(|attestation| attestation.data().clone())
                .map_err(ApiError::unhandled_error)?;

            match accept {
                Some(api_types::Accept::Ssz) => Response::builder()
                    .status(200)
                    .body(attestation_data.as_ssz_bytes().into())
                    .map(add_ssz_content_type_header)
                    .map_err(|e| ApiError::server_error(format!("failed to create response: {e}"))),
                _ => Ok(Json(api_types::GenericResponse::from(attestation_data)).into_response()),
            }
        })
        .await
}

async fn get_validator_payload_attestation_data<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Query(query): Query<api_types::ValidatorPayloadAttestationDataQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    state.check_not_syncing()?;
    let chain = state.chain()?;
    let accept = accept_header(&headers);
    state
        .task_spawner()
        .blocking_response_task(Priority::P0, move || {
            let data = chain
                .get_payload_attestation_data(query.slot)
                .map_err(ApiError::unhandled_error)?;

            match accept {
                Some(api_types::Accept::Ssz) => Response::builder()
                    .status(200)
                    .body(data.as_ssz_bytes().into())
                    .map(add_ssz_content_type_header)
                    .map_err(|e| ApiError::server_error(format!("failed to create response: {e}"))),
                _ => Ok(Json(api_types::GenericResponse::from(data)).into_response()),
            }
        })
        .await
}

async fn get_validator_aggregate_attestation<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(version_str): Path<String>,
    Query(query): Query<api_types::ValidatorAggregateAttestationQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let endpoint_version = parse_endpoint_version(&version_str)?;
    state.check_not_syncing()?;
    let chain = state.chain()?;
    let accept = accept_header(&headers);
    state
        .task_spawner()
        .blocking_response_task(Priority::P0, move || {
            crate::aggregate_attestation::get_aggregate_attestation(
                query.slot,
                &query.attestation_data_root,
                query.committee_index,
                endpoint_version,
                accept,
                chain,
            )
        })
        .await
}

async fn get_validator_sync_committee_contribution<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Query(sync_committee_data): Query<SyncContributionData>,
) -> Result<Response, ApiError> {
    state.check_not_syncing()?;
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            chain
                .get_aggregated_sync_committee_contribution(&sync_committee_data)
                .map_err(|e| {
                    ApiError::bad_request(format!("unable to fetch sync contribution: {:?}", e))
                })?
                .map(api_types::GenericResponse::from)
                .ok_or_else(|| ApiError::not_found("no matching sync contribution found"))
        })
        .await
}

async fn post_validator_duties_attester<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(epoch): Path<Epoch>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    state.check_not_syncing()?;
    let chain = state.chain()?;
    let indices: api_types::ValidatorIndexData = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            attester_duties::attester_duties(epoch, &indices.0, &chain)
        })
        .await
}

async fn post_validator_duties_ptc<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(epoch): Path<Epoch>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    state.check_not_syncing()?;
    let chain = state.chain()?;
    let indices: api_types::ValidatorIndexData = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            ptc_duties::ptc_duties(epoch, &indices.0, &chain)
        })
        .await
}

async fn post_validator_duties_sync<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(epoch): Path<Epoch>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    state.check_not_syncing()?;
    let chain = state.chain()?;
    let indices: api_types::ValidatorIndexData = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            sync_committees::sync_committee_duties(epoch, &indices.0, &chain)
        })
        .await
}

async fn post_validator_aggregate_and_proofs<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(_version_str): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    state.check_not_syncing()?;
    let chain = state.chain()?;
    let network_tx = state.network_tx()?;
    let aggregates: Vec<SignedAggregateAndProof<T::EthSpec>> = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            let seen_timestamp = timestamp_now();
            let mut verified_aggregates = Vec::with_capacity(aggregates.len());
            let mut messages = Vec::with_capacity(aggregates.len());
            let mut failures = Vec::new();

            for (index, aggregate) in aggregates.iter().enumerate() {
                match chain.verify_aggregated_attestation_for_gossip(aggregate) {
                    Ok(verified_aggregate) => {
                        messages.push(PubsubMessage::AggregateAndProofAttestation(Box::new(
                            verified_aggregate.aggregate().clone(),
                        )));

                        chain
                            .validator_monitor
                            .read()
                            .register_api_aggregated_attestation(
                                seen_timestamp,
                                verified_aggregate.aggregate(),
                                verified_aggregate.indexed_attestation(),
                                &chain.slot_clock,
                            );

                        verified_aggregates.push((index, verified_aggregate));
                    }
                    Err(AttnError::AttestationSupersetKnown(_)) => continue,
                    Err(AttnError::AggregatorAlreadyKnown(_)) => continue,
                    Err(e) => {
                        error!(
                            error = ?e,
                            request_index = index,
                            aggregator_index = aggregate.message().aggregator_index(),
                            attestation_index = aggregate.message().aggregate().committee_index(),
                            attestation_slot = %aggregate.message().aggregate().data().slot,
                            "Failure verifying aggregate and proofs"
                        );
                        failures.push(api_types::Failure::new(index, format!("Verification: {:?}", e)));
                    }
                }
            }

            if !messages.is_empty() {
                publish_network_message(&network_tx, NetworkMessage::Publish { messages })?;
            }

            for (index, verified_aggregate) in verified_aggregates {
                if let Err(e) = chain.apply_attestation_to_fork_choice(&verified_aggregate) {
                    error!(
                        error = ?e,
                        request_index = index,
                        aggregator_index = verified_aggregate.aggregate().message().aggregator_index(),
                        attestation_index = verified_aggregate.attestation().committee_index(),
                        attestation_slot = %verified_aggregate.attestation().data().slot,
                        "Failure applying verified aggregate attestation to fork choice"
                    );
                    failures.push(api_types::Failure::new(index, format!("Fork choice: {:?}", e)));
                }
                if let Err(e) = chain.add_to_block_inclusion_pool(verified_aggregate) {
                    warn!(
                        error = ?e,
                        request_index = index,
                        "Could not add verified aggregate attestation to the inclusion pool"
                    );
                    failures.push(api_types::Failure::new(index, format!("Op pool: {:?}", e)));
                }
            }

            if !failures.is_empty() {
                Err(ApiError::IndexedBadRequest {
                    message: "error processing aggregate and proofs".into(),
                    failures,
                })
            } else {
                Ok(())
            }
        })
        .await
}

async fn post_validator_contribution_and_proofs<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    state.check_not_syncing()?;
    let chain = state.chain()?;
    let network_tx = state.network_tx()?;
    let contributions: Vec<SignedContributionAndProof<T::EthSpec>> =
        json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            sync_committees::process_signed_contribution_and_proofs(
                contributions,
                network_tx,
                &chain,
            )?;
            Ok(api_types::GenericResponse::from(()))
        })
        .await
}

async fn post_validator_beacon_committee_subscriptions<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let validator_subscription_tx = state.validator_subscription_tx()?;
    let committee_subscriptions: Vec<api_types::BeaconCommitteeSubscription> =
        json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            let subscriptions: std::collections::BTreeSet<_> = committee_subscriptions
                .iter()
                .map(|subscription| {
                    chain
                        .validator_monitor
                        .write()
                        .auto_register_local_validator(subscription.validator_index);
                    api_types::ValidatorSubscription {
                        attestation_committee_index: subscription.committee_index,
                        slot: subscription.slot,
                        committee_count_at_slot: subscription.committees_at_slot,
                        is_aggregator: subscription.is_aggregator,
                    }
                })
                .collect();

            let message = ValidatorSubscriptionMessage::AttestationSubscribe { subscriptions };
            if let Err(e) = validator_subscription_tx.try_send(message) {
                warn!(
                    info = "the host may be overloaded or resource-constrained",
                    error = ?e,
                    "Unable to process committee subscriptions"
                );
                return Err(ApiError::server_error(
                    "unable to queue subscription, host may be overloaded or shutting down",
                ));
            }
            Ok(())
        })
        .await
}

async fn post_validator_prepare_beacon_proposer<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    state.check_not_syncing()?;
    let chain = state.chain()?;
    let network_tx = state.network_tx()?;
    let preparation_data: Vec<ProposerPreparationData> = json_body(&headers, body).await?;
    state
        .task_spawner()
        .spawn_async_with_rejection(Priority::P0, async move {
            let execution_layer = chain
                .execution_layer
                .as_ref()
                .ok_or(BeaconChainError::ExecutionLayerMissing)
                .map_err(ApiError::unhandled_error)?;

            let current_slot = chain
                .slot_clock
                .now_or_genesis()
                .ok_or(BeaconChainError::UnableToReadSlot)
                .map_err(ApiError::unhandled_error)?;
            let current_epoch = current_slot.epoch(T::EthSpec::slots_per_epoch());

            debug!(
                count = preparation_data.len(),
                "Received proposer preparation data"
            );

            execution_layer
                .update_proposer_preparation(
                    current_epoch,
                    preparation_data.iter().map(|data| (data, &None)),
                )
                .await;

            chain
                .prepare_beacon_proposer(current_slot)
                .await
                .map_err(|e| {
                    ApiError::bad_request(format!("error updating proposer preparations: {:?}", e))
                })?;

            if chain.spec.is_peer_das_scheduled() {
                let (finalized_beacon_state, _, _) =
                    StateId(CoreStateId::Finalized).state(&chain)?;
                let validators_and_balances = preparation_data
                    .iter()
                    .filter_map(|preparation| {
                        if let Ok(effective_balance) = finalized_beacon_state
                            .get_effective_balance(preparation.validator_index as usize)
                        {
                            Some((preparation.validator_index as usize, effective_balance))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();

                let current_slot = chain.slot().map_err(ApiError::unhandled_error)?;
                if let Some(cgc_change) = chain
                    .data_availability_checker
                    .custody_context()
                    .register_validators(validators_and_balances, current_slot, &chain.spec)
                {
                    chain.update_data_column_custody_info(Some(
                        cgc_change
                            .effective_epoch
                            .start_slot(T::EthSpec::slots_per_epoch()),
                    ));

                    network_tx
                        .send(NetworkMessage::CustodyCountChanged {
                            new_custody_group_count: cgc_change.new_custody_group_count,
                            sampling_count: cgc_change.sampling_count,
                        })
                        .unwrap_or_else(|e| {
                            debug!(error = %e, "Could not send message to the network service. \
                        Likely shutdown")
                        });
                }
            }

            Ok::<_, ApiError>(Json(()).into_response())
        })
        .await
}

async fn post_validator_register_validator<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let register_val_data: Vec<SignedValidatorRegistrationData> = json_body(&headers, body).await?;

    let (tx, rx) = oneshot::channel();

    let result = state
        .task_spawner()
        .spawn_async_with_rejection(Priority::P0, async move {
            let execution_layer = chain
                .execution_layer
                .as_ref()
                .ok_or(BeaconChainError::ExecutionLayerMissing)
                .map_err(ApiError::unhandled_error)?;
            let current_slot = chain
                .slot_clock
                .now_or_genesis()
                .ok_or(BeaconChainError::UnableToReadSlot)
                .map_err(ApiError::unhandled_error)?;
            let current_epoch = current_slot.epoch(T::EthSpec::slots_per_epoch());

            debug!(
                count = register_val_data.len(),
                "Received register validator request"
            );

            let head_snapshot = chain.head_snapshot();
            let spec = &chain.spec;

            let (preparation_data, filtered_registration_data): (
                Vec<(ProposerPreparationData, Option<u64>)>,
                Vec<SignedValidatorRegistrationData>,
            ) = register_val_data
                .into_iter()
                .filter_map(|register_data| {
                    chain
                        .validator_index(&register_data.message.pubkey)
                        .ok()
                        .flatten()
                        .and_then(|validator_index| {
                            let validator = head_snapshot
                                .beacon_state
                                .get_validator(validator_index)
                                .ok()?;
                            let validator_status = ValidatorStatus::from_validator(
                                validator,
                                current_epoch,
                                spec.far_future_epoch,
                            )
                            .superstatus();
                            let is_active_or_pending =
                                matches!(validator_status, ValidatorStatus::Pending)
                                    || matches!(validator_status, ValidatorStatus::Active);

                            is_active_or_pending.then_some((
                                (
                                    ProposerPreparationData {
                                        validator_index: validator_index as u64,
                                        fee_recipient: register_data.message.fee_recipient,
                                    },
                                    Some(register_data.message.gas_limit),
                                ),
                                register_data,
                            ))
                        })
                })
                .unzip();

            execution_layer
                .update_proposer_preparation(
                    current_epoch,
                    preparation_data.iter().map(|(data, limit)| (data, limit)),
                )
                .await;

            chain
                .prepare_beacon_proposer(current_slot)
                .await
                .map_err(|e| {
                    ApiError::bad_request(format!("error updating proposer preparations: {:?}", e))
                })?;

            info!(
                count = filtered_registration_data.len(),
                "Forwarding register validator request to connected builder"
            );

            let chain_clone = chain.clone();
            let builder_future = async move {
                let arc_builder = chain_clone
                    .execution_layer
                    .as_ref()
                    .ok_or(BeaconChainError::ExecutionLayerMissing)
                    .map_err(ApiError::unhandled_error)?
                    .builder();
                let builder = arc_builder
                    .as_ref()
                    .ok_or(BeaconChainError::BuilderMissing)
                    .map_err(ApiError::unhandled_error)?;
                builder
                    .post_builder_validators(&filtered_registration_data)
                    .await
                    .map(|resp| Json(resp).into_response())
                    .map_err(|e| {
                        warn!(
                            num_registrations = filtered_registration_data.len(),
                            error = ?e,
                            "Relay error when registering validator(s)"
                        );
                        if let eth2::Error::ServerMessage(message) = e {
                            if message.code == StatusCode::BAD_REQUEST.as_u16() {
                                return ApiError::bad_request(message.message);
                            } else {
                                return ApiError::server_error(message.message);
                            }
                        }
                        ApiError::server_error(format!("{e:?}"))
                    })
            };
            tokio::task::spawn(async move { tx.send(builder_future.await) });

            Ok(StatusCode::OK.into_response())
        })
        .await;

    result?;

    rx.await
        .unwrap_or_else(|_| Err(ApiError::server_error("No response from builder channel")))
}

async fn post_validator_sync_committee_subscriptions<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let validator_subscription_tx = state.validator_subscription_tx()?;
    let subscriptions: Vec<types::SyncCommitteeSubscription> = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            for subscription in subscriptions {
                chain
                    .validator_monitor
                    .write()
                    .auto_register_local_validator(subscription.validator_index);

                let message = ValidatorSubscriptionMessage::SyncCommitteeSubscribe {
                    subscriptions: vec![subscription],
                };
                if let Err(e) = validator_subscription_tx.try_send(message) {
                    warn!(
                        info = "the host may be overloaded or resource-constrained",
                        error = ?e,
                        "Unable to process sync subscriptions"
                    );
                    return Err(ApiError::server_error(
                        "unable to queue subscription, host may be overloaded or shutting down",
                    ));
                }
            }

            Ok(())
        })
        .await
}

async fn post_validator_liveness_epoch<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(epoch): Path<Epoch>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let indices: api_types::ValidatorIndexData = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            let current_epoch = chain.epoch().map_err(ApiError::unhandled_error)?;
            let prev_epoch = current_epoch.saturating_sub(Epoch::new(1));
            let next_epoch = current_epoch.saturating_add(Epoch::new(1));

            if epoch < prev_epoch || epoch > next_epoch {
                return Err(ApiError::bad_request(format!(
                    "request epoch {} is more than one epoch from the current epoch {}",
                    epoch, current_epoch
                )));
            }

            let liveness: Vec<api_types::StandardLivenessResponseData> = indices
                .0
                .iter()
                .cloned()
                .map(|index| {
                    let is_live = chain.validator_seen_at_epoch(index as usize, epoch);
                    api_types::StandardLivenessResponseData { index, is_live }
                })
                .collect();

            Ok(api_types::GenericResponse::from(liveness))
        })
        .await
}

// -- SSE events --

async fn get_events<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    MultiKeyQuery(topics): MultiKeyQuery<api_types::EventQuery>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;

    let event_handler = chain
        .event_handler
        .as_ref()
        .ok_or_else(|| ApiError::server_error("event handler was not initialized"))?;

    let mut receivers = Vec::with_capacity(topics.topics.len());

    for topic in topics.topics {
        let receiver = match topic {
            api_types::EventTopic::Head => event_handler.subscribe_head(),
            api_types::EventTopic::Block => event_handler.subscribe_block(),
            api_types::EventTopic::BlobSidecar => event_handler.subscribe_blob_sidecar(),
            api_types::EventTopic::DataColumnSidecar => {
                event_handler.subscribe_data_column_sidecar()
            }
            api_types::EventTopic::Attestation => event_handler.subscribe_attestation(),
            api_types::EventTopic::SingleAttestation => {
                event_handler.subscribe_single_attestation()
            }
            api_types::EventTopic::VoluntaryExit => event_handler.subscribe_exit(),
            api_types::EventTopic::FinalizedCheckpoint => event_handler.subscribe_finalized(),
            api_types::EventTopic::ChainReorg => event_handler.subscribe_reorgs(),
            api_types::EventTopic::ContributionAndProof => event_handler.subscribe_contributions(),
            api_types::EventTopic::PayloadAttributes => {
                event_handler.subscribe_payload_attributes()
            }
            api_types::EventTopic::LateHead => event_handler.subscribe_late_head(),
            api_types::EventTopic::LightClientFinalityUpdate => {
                event_handler.subscribe_light_client_finality_update()
            }
            api_types::EventTopic::LightClientOptimisticUpdate => {
                event_handler.subscribe_light_client_optimistic_update()
            }
            api_types::EventTopic::BlockReward => event_handler.subscribe_block_reward(),
            api_types::EventTopic::AttesterSlashing => event_handler.subscribe_attester_slashing(),
            api_types::EventTopic::ProposerSlashing => event_handler.subscribe_proposer_slashing(),
            api_types::EventTopic::BlsToExecutionChange => {
                event_handler.subscribe_bls_to_execution_change()
            }
            api_types::EventTopic::BlockGossip => event_handler.subscribe_block_gossip(),
            api_types::EventTopic::ExecutionBid => event_handler.subscribe_execution_bid(),
            api_types::EventTopic::ExecutionPayload => event_handler.subscribe_execution_payload(),
            api_types::EventTopic::PayloadAttestation => {
                event_handler.subscribe_payload_attestation()
            }
            api_types::EventTopic::ExecutionProofReceived => {
                event_handler.subscribe_execution_proof_received()
            }
        };

        receivers.push(
            BroadcastStream::new(receiver)
                .map(|msg| match msg {
                    Ok(data) => Event::default()
                        .event(data.topic_name())
                        .json_data(data)
                        .unwrap_or_else(|e| {
                            Event::default().comment(format!("error - bad json: {e:?}"))
                        }),
                    Err(BroadcastStreamRecvError::Lagged(n)) => {
                        Event::default().comment(format!("error - dropped {n} messages"))
                    }
                })
                .map(Ok::<_, std::convert::Infallible>),
        );
    }

    let s = futures::stream::select_all(receivers);
    Ok(Sse::new(s).keep_alive(KeepAlive::default()).into_response())
}

// -- Lighthouse routes --

async fn get_lighthouse_health<T: BeaconChainTypes>(
    State(_state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    // No task_spawner needed for this simple check.
    eth2::lighthouse::Health::observe()
        .map(|h| Json(api_types::GenericResponse::from(h)).into_response())
        .map_err(ApiError::bad_request)
}

async fn get_lighthouse_ui_health<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let network_globals = state.network_globals()?;
    let system_info = state.system_info.clone();
    let app_start = state.app_start;
    let data_dir = state.data_dir.clone();
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            let app_uptime = app_start.elapsed().as_secs();
            Ok(api_types::GenericResponse::from(observe_system_health_bn(
                system_info,
                data_dir,
                app_uptime,
                network_globals,
            )))
        })
        .await
}

async fn get_lighthouse_ui_validator_count<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            ui::get_validator_count(chain).map(api_types::GenericResponse::from)
        })
        .await
}

async fn get_lighthouse_syncing<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let network_globals = state.network_globals()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            Ok(api_types::GenericResponse::from(
                network_globals.sync_state(),
            ))
        })
        .await
}

async fn get_lighthouse_nat<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            Ok(api_types::GenericResponse::from(observe_nat()))
        })
        .await
}

async fn get_lighthouse_peers<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let network_globals = state.network_globals()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            Ok(network_globals
                .peers
                .read()
                .peers()
                .map(|(peer_id, peer_info)| peer::Peer {
                    peer_id: peer_id.to_string(),
                    peer_info: peer_info.clone(),
                })
                .collect::<Vec<_>>())
        })
        .await
}

async fn get_lighthouse_peers_connected<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let network_globals = state.network_globals()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let mut peers = vec![];
            for (peer_id, peer_info) in network_globals.peers.read().connected_peers() {
                peers.push(peer::Peer {
                    peer_id: peer_id.to_string(),
                    peer_info: peer_info.clone(),
                });
            }
            Ok(peers)
        })
        .await
}

async fn get_lighthouse_proto_array<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            Ok(Json(api_types::GenericResponseRef::from(
                chain
                    .canonical_head
                    .fork_choice_read_lock()
                    .proto_array()
                    .core_proto_array(),
            ))
            .into_response())
        })
        .await
}

async fn get_lighthouse_validator_inclusion_global<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path((epoch, validator_id)): Path<(Epoch, ValidatorId)>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            validator_inclusion::validator_inclusion_data(epoch, &validator_id, &chain)
                .map(api_types::GenericResponse::from)
        })
        .await
}

async fn get_lighthouse_validator_inclusion<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(epoch): Path<Epoch>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            validator_inclusion::global_validator_inclusion_data(epoch, &chain)
                .map(api_types::GenericResponse::from)
        })
        .await
}

async fn get_lighthouse_staking<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || Ok(()))
        .await
}

async fn get_lighthouse_database_info<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || database::info(chain))
        .await
}

async fn get_lighthouse_custody_info<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || custody::info(chain))
        .await
}

async fn get_lighthouse_block_rewards<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Query(query): Query<eth2::lighthouse::BlockRewardsQuery>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            block_rewards::get_block_rewards(query, chain)
        })
        .await
}

async fn post_lighthouse_block_rewards<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let blocks = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            block_rewards::compute_block_rewards(blocks, chain)
        })
        .await
}

async fn get_lighthouse_attestation_performance<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(target): Path<String>,
    Query(query): Query<eth2::lighthouse::AttestationPerformanceQuery>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            attestation_performance::get_attestation_performance(target, query, chain)
        })
        .await
}

async fn get_lighthouse_block_packing_efficiency<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Query(query): Query<eth2::lighthouse::BlockPackingEfficiencyQuery>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            block_packing_efficiency::get_block_packing_efficiency(query, chain)
        })
        .await
}

async fn get_lighthouse_merge_readiness<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .spawn_async_with_rejection(Priority::P1, async move {
            let current_slot = chain.slot_clock.now_or_genesis().unwrap_or(Slot::new(0));
            let merge_readiness = chain.check_bellatrix_readiness(current_slot).await;
            Ok::<_, ApiError>(
                Json(api_types::GenericResponse::from(merge_readiness)).into_response(),
            )
        })
        .await
}

async fn post_lighthouse_finalize<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let request_data: api_types::ManualFinalizationRequestData = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            let checkpoint = Checkpoint {
                epoch: request_data.epoch,
                root: request_data.block_root,
            };

            chain
                .manually_finalize_state(request_data.state_root, checkpoint)
                .map(|_| api_types::GenericResponse::from(request_data))
                .map_err(|e| {
                    ApiError::bad_request(format!("Failed to finalize state due to error: {e:?}"))
                })
        })
        .await
}

async fn post_lighthouse_compaction<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            chain.manually_compact_database();
            Ok(api_types::GenericResponse::from(String::from(
                "Triggered manual compaction",
            )))
        })
        .await
}

async fn post_lighthouse_add_peer<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let network_globals = state.network_globals()?;
    let network_tx = state.network_tx()?;
    let request_data: api_types::AdminPeer = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            let enr = Enr::from_str(&request_data.enr)
                .map_err(|e| ApiError::bad_request(format!("invalid enr error {}", e)))?;
            info!(
                peer_id = %enr.peer_id(),
                multiaddr = ?enr.multiaddr(),
                "Adding trusted peer"
            );
            network_globals.add_trusted_peer(enr.clone());
            publish_network_message(&network_tx, NetworkMessage::ConnectTrustedPeer(enr))?;
            Ok(())
        })
        .await
}

async fn post_lighthouse_remove_peer<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let network_globals = state.network_globals()?;
    let network_tx = state.network_tx()?;
    let request_data: api_types::AdminPeer = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            let enr = Enr::from_str(&request_data.enr)
                .map_err(|e| ApiError::bad_request(format!("invalid enr error {}", e)))?;
            info!(
                peer_id = %enr.peer_id(),
                multiaddr = ?enr.multiaddr(),
                "Removing trusted peer"
            );
            network_globals.remove_trusted_peer(enr.clone());
            publish_network_message(&network_tx, NetworkMessage::DisconnectTrustedPeer(enr))?;
            Ok(())
        })
        .await
}

async fn post_lighthouse_liveness<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let request_data: api_types::LivenessRequestData = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P0, move || {
            let current_epoch = chain.epoch().map_err(ApiError::unhandled_error)?;
            let prev_epoch = current_epoch.saturating_sub(Epoch::new(1));
            let next_epoch = current_epoch.saturating_add(Epoch::new(1));

            if request_data.epoch < prev_epoch || request_data.epoch > next_epoch {
                return Err(ApiError::bad_request(format!(
                    "request epoch {} is more than one epoch from the current epoch {}",
                    request_data.epoch, current_epoch
                )));
            }

            let liveness: Vec<api_types::LivenessResponseData> = request_data
                .indices
                .iter()
                .cloned()
                .map(|index| {
                    let is_live = chain.validator_seen_at_epoch(index as usize, request_data.epoch);
                    api_types::LivenessResponseData {
                        index,
                        epoch: request_data.epoch,
                        is_live,
                    }
                })
                .collect();

            Ok(api_types::GenericResponse::from(liveness))
        })
        .await
}

async fn post_lighthouse_ui_validator_metrics<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let request_data: ui::ValidatorMetricsRequestData = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            ui::post_validator_monitor_metrics(request_data, chain)
                .map(api_types::GenericResponse::from)
        })
        .await
}

async fn post_lighthouse_ui_validator_info<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let request_data: ui::ValidatorInfoRequestData = json_body(&headers, body).await?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            ui::get_validator_info(request_data, chain).map(api_types::GenericResponse::from)
        })
        .await
}

async fn post_lighthouse_database_reconstruct<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    state.check_not_syncing()?;
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            chain.store_migrator.process_reconstruction();
            Ok("success")
        })
        .await
}

async fn post_lighthouse_custody_backfill<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let effective_epoch = chain
                .canonical_head
                .cached_head()
                .head_slot()
                .epoch(T::EthSpec::slots_per_epoch())
                + 1;
            let custody_context = chain.data_availability_checker.custody_context();
            custody_context.reset_validator_custody_requirements(effective_epoch);
            chain.update_data_column_custody_info(Some(
                effective_epoch.start_slot(T::EthSpec::slots_per_epoch()),
            ));
            Ok(())
        })
        .await
}

async fn get_lighthouse_logs<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
) -> Result<Response, ApiError> {
    let sse_component = state.sse_logging_components.clone();
    state
        .task_spawner()
        .blocking_response_task(Priority::P1, move || {
            if let Some(logging_components) = sse_component {
                let s =
                    BroadcastStream::new(logging_components.sender.subscribe()).map(
                        |msg| match msg {
                            Ok(data) => match serde_json::to_string(&data) {
                                Ok(json) => {
                                    Ok::<_, std::convert::Infallible>(Event::default().data(json))
                                }
                                Err(e) => Ok::<_, std::convert::Infallible>(
                                    Event::default().comment(format!("error - bad json: {e:?}")),
                                ),
                            },
                            Err(e) => Ok::<_, std::convert::Infallible>(
                                Event::default().comment(format!("error - receive: {e}")),
                            ),
                        },
                    );

                Ok(Sse::new(s).keep_alive(KeepAlive::default()).into_response())
            } else {
                Err(ApiError::server_error("SSE Logging is not enabled"))
            }
        })
        .await
}

// -- Vibehouse routes --

async fn get_vibehouse_execution_proof_status<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    Path(block_id): Path<BlockId>,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    state
        .task_spawner()
        .blocking_json_task(Priority::P1, move || {
            let (block_root, execution_optimistic, finalized) = block_id.root(&chain)?;

            let received_subnet_ids = chain
                .data_availability_checker
                .cached_execution_proof_subnet_ids(&block_root)
                .unwrap_or_default();

            let required_proofs = if chain.config.stateless_validation {
                chain.config.stateless_min_proofs_required as u64
            } else {
                0
            };

            let is_fully_proven = if required_proofs == 0 {
                true
            } else {
                received_subnet_ids.len() as u64 >= required_proofs
            };

            let status = api_types::ExecutionProofStatus {
                block_root,
                received_proof_subnet_ids: received_subnet_ids.into_iter().map(|id| *id).collect(),
                required_proofs,
                is_fully_proven,
            };

            Ok(api_types::GenericResponse::from(status)
                .add_execution_optimistic_finalized(execution_optimistic, finalized))
        })
        .await
}

async fn post_vibehouse_execution_proofs<T: BeaconChainTypes>(
    State(state): State<SharedState<T>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let chain = state.chain()?;
    let proof: ExecutionProof = json_body(&headers, body).await?;
    state
        .task_spawner()
        .spawn_async_with_rejection(Priority::P1, async move {
            let subnet_id = proof.subnet_id;
            let proof = Arc::new(proof);

            let verified = chain
                .verify_execution_proof_for_gossip(proof, subnet_id)
                .map_err(|e| ApiError::bad_request(format!("proof verification failed: {e:?}")))?;

            let block_root = verified.block_root();
            let slot = chain
                .canonical_head
                .fork_choice_read_lock()
                .get_block(&block_root)
                .map(|b| b.slot)
                .ok_or_else(|| ApiError::bad_request("block root not found in fork choice"))?;

            chain
                .check_gossip_execution_proof_availability_and_import(slot, block_root, verified)
                .await
                .map_err(|e| ApiError::bad_request(format!("proof import failed: {e:?}")))?;

            Ok::<_, ApiError>(Json(api_types::GenericResponse::from(())).into_response())
        })
        .await
}

fn build_cors_layer(
    allow_origin: Option<&str>,
    listen_addr: IpAddr,
    listen_port: u16,
) -> Result<CorsLayer, Error> {
    let cors = CorsLayer::new()
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([axum::http::header::CONTENT_TYPE]);

    let cors = match allow_origin {
        Some("*") => cors.allow_origin(AllowOrigin::any()),
        Some(origin) => cors.allow_origin(
            origin
                .parse::<axum::http::HeaderValue>()
                .map_err(|e| Error::Other(format!("invalid CORS origin: {e}")))?,
        ),
        None => {
            let len = if listen_addr.is_loopback() { 2 } else { 1 };
            let mut origins = Vec::with_capacity(len);
            origins.push(
                format!("http://{}:{}", listen_addr, listen_port)
                    .parse::<axum::http::HeaderValue>()
                    .map_err(|e| Error::Other(format!("invalid CORS origin: {e}")))?,
            );
            if listen_addr.is_loopback() {
                origins.push(
                    format!("http://localhost:{}", listen_port)
                        .parse::<axum::http::HeaderValue>()
                        .map_err(|e| Error::Other(format!("invalid CORS origin: {e}")))?,
                );
            }
            cors.allow_origin(origins)
        }
    };

    Ok(cors)
}

// ── Utility functions ────────────────────────────────────────────────────────

fn from_meta_data<E: EthSpec>(
    meta_data: &RwLock<MetaData<E>>,
    spec: &ChainSpec,
) -> api_types::MetaData {
    let meta_data = meta_data.read();
    let format_hex = |bytes: &[u8]| format!("0x{}", hex::encode(bytes));

    let seq_number = *meta_data.seq_number();
    let attnets = format_hex(&meta_data.attnets().clone().into_bytes());
    let syncnets = format_hex(
        &meta_data
            .syncnets()
            .cloned()
            .unwrap_or_default()
            .into_bytes(),
    );

    if spec.is_peer_das_scheduled() {
        api_types::MetaData::V3(api_types::MetaDataV3 {
            seq_number,
            attnets,
            syncnets,
            custody_group_count: meta_data.custody_group_count().cloned().unwrap_or_default(),
        })
    } else {
        api_types::MetaData::V2(api_types::MetaDataV2 {
            seq_number,
            attnets,
            syncnets,
        })
    }
}

/// Publish a message to the libp2p pubsub network.
fn publish_pubsub_message<E: EthSpec>(
    network_tx: &UnboundedSender<NetworkMessage<E>>,
    message: PubsubMessage<E>,
) -> Result<(), ApiError> {
    publish_network_message(
        network_tx,
        NetworkMessage::Publish {
            messages: vec![message],
        },
    )
}

/// Publish a message to the libp2p pubsub network.
fn publish_pubsub_messages<E: EthSpec>(
    network_tx: &UnboundedSender<NetworkMessage<E>>,
    messages: Vec<PubsubMessage<E>>,
) -> Result<(), ApiError> {
    publish_network_message(network_tx, NetworkMessage::Publish { messages })
}

/// Publish a message to the libp2p network.
fn publish_network_message<E: EthSpec>(
    network_tx: &UnboundedSender<NetworkMessage<E>>,
    message: NetworkMessage<E>,
) -> Result<(), ApiError> {
    network_tx
        .send(message)
        .map_err(|e| ApiError::server_error(format!("unable to publish to network channel: {}", e)))
}
