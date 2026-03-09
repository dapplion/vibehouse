mod api_error;
mod api_secret;
mod create_signed_voluntary_exit;
mod create_validator;
mod graffiti;
mod keystores;
mod remotekeys;
mod tests;

pub mod test_utils;
pub use api_secret::PK_FILENAME;

use api_error::ApiError;
use graffiti::{delete_graffiti, get_graffiti, set_graffiti};

use create_signed_voluntary_exit::create_signed_voluntary_exit;
use graffiti_file::{GraffitiFile, determine_graffiti};
use validator_store::ValidatorStore;
use vibehouse_validator_store::VibehouseValidatorStore;

use account_utils::{
    mnemonic_from_phrase,
    validator_definitions::{SigningDefinition, ValidatorDefinition, Web3SignerDefinition},
};
pub use api_secret::ApiSecret;
use beacon_node_fallback::CandidateInfo;
use create_validator::{
    create_validators_mnemonic, create_validators_web3signer, get_voting_password_storage,
};
use directory::{DEFAULT_HARDCODED_NETWORK, DEFAULT_ROOT_DIR, DEFAULT_VALIDATOR_DIR};
use eth2::vibehouse_vc::{
    std_types::{AuthResponse, GetFeeRecipientResponse, GetGasLimitResponse},
    types::{
        self as api_types, GenericResponse, GetGraffitiResponse, Graffiti, PublicKey,
        PublicKeyBytes, SetGraffitiRequest, UpdateCandidatesRequest, UpdateCandidatesResponse,
    },
};
use health_metrics::observe::Observe;
use logging::SSELoggingComponents;
use logging::crit;
use parking_lot::RwLock;
use sensitive_url::SensitiveUrl;
use serde::{Deserialize, Serialize};
use slot_clock::SlotClock;
use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use sysinfo::{System, SystemExt};
use system_health::observe_system_health_vc;
use task_executor::TaskExecutor;
use tokio_stream::{StreamExt, wrappers::BroadcastStream};
use tracing::{info, warn};
use types::{ChainSpec, ConfigAndPreset, EthSpec};
use validator_dir::Builder as ValidatorDirBuilder;
use validator_services::block_service::BlockService;
use vibehouse_version::version_with_platform;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{delete, get, patch, post},
};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;

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

/// A wrapper around all the items required to spawn the HTTP server.
///
/// The server will gracefully handle the case where any fields are `None`.
pub struct Context<T: SlotClock, E> {
    pub task_executor: TaskExecutor,
    pub api_secret: ApiSecret,
    pub block_service: Option<BlockService<VibehouseValidatorStore<T, E>, T>>,
    pub validator_store: Option<Arc<VibehouseValidatorStore<T, E>>>,
    pub validator_dir: Option<PathBuf>,
    pub secrets_dir: Option<PathBuf>,
    pub graffiti_file: Option<GraffitiFile>,
    pub graffiti_flag: Option<Graffiti>,
    pub spec: Arc<ChainSpec>,
    pub config: Config,
    pub sse_logging_components: Option<SSELoggingComponents>,
    pub slot_clock: T,
}

/// Configuration for the HTTP server.
#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub enabled: bool,
    pub listen_addr: IpAddr,
    pub listen_port: u16,
    pub allow_origin: Option<String>,
    pub allow_keystore_export: bool,
    pub store_passwords_in_secrets_dir: bool,
    pub http_token_path: PathBuf,
    pub bn_long_timeouts: bool,
}

impl Default for Config {
    fn default() -> Self {
        // This value is always overridden when building config from CLI.
        let http_token_path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(DEFAULT_ROOT_DIR)
            .join(DEFAULT_HARDCODED_NETWORK)
            .join(DEFAULT_VALIDATOR_DIR)
            .join(PK_FILENAME);
        Self {
            enabled: false,
            listen_addr: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            listen_port: 5062,
            allow_origin: None,
            allow_keystore_export: false,
            store_passwords_in_secrets_dir: false,
            http_token_path,
            bn_long_timeouts: false,
        }
    }
}

/// Shared application state, wrapping Context with config-derived values.
struct AppState<T: SlotClock, E: EthSpec> {
    ctx: Arc<Context<T, E>>,
    system_info: Arc<RwLock<System>>,
    app_start: std::time::Instant,
    api_token_path: PathBuf,
    /// Pre-computed valid auth header values.
    auth_header_values: Vec<String>,
}

type SharedState<T, E> = Arc<AppState<T, E>>;

/// Creates a server that will serve requests using information from `ctx`.
///
/// The server will shut down gracefully when the `shutdown` future resolves.
///
/// ## Returns
///
/// This function will bind the server to the provided address and then return a tuple of:
///
/// - `SocketAddr`: the address that the HTTP server will listen on.
/// - `Future`: the actual server future that will need to be awaited.
///
/// ## Errors
///
/// Returns an error if the server is unable to bind or there is another error during
/// configuration.
pub fn serve<T: 'static + SlotClock + Clone, E: EthSpec>(
    ctx: Arc<Context<T, E>>,
    shutdown: impl Future<Output = ()> + Send + Sync + 'static,
) -> Result<(SocketAddr, impl Future<Output = ()>), Error> {
    // Sanity check.
    if !ctx.config.enabled {
        crit!("Cannot start disabled metrics HTTP server");
        return Err(Error::Other(
            "A disabled metrics server should not be started".to_string(),
        ));
    }

    // Configure CORS.
    let cors_layer = build_cors_layer(
        ctx.config.allow_origin.as_deref(),
        ctx.config.listen_addr,
        ctx.config.listen_port,
    )?;

    let listen_addr = SocketAddr::new(ctx.config.listen_addr, ctx.config.listen_port);

    let mut api_token_path = ctx.api_secret.api_token_path();

    // Attempt to convert the path to an absolute path, but don't error if it fails.
    match api_token_path.canonicalize() {
        Ok(abs_path) => api_token_path = abs_path,
        Err(e) => {
            warn!(
                error = ?e,
                "Error canonicalizing token path"
            );
        }
    };

    let auth_header_values = ctx.api_secret.auth_header_values();

    let system_info = Arc::new(RwLock::new(sysinfo::System::new()));
    {
        let mut system_info = system_info.write();
        system_info.refresh_disks_list();
        system_info.refresh_networks_list();
    }

    let state: SharedState<T, E> = Arc::new(AppState {
        ctx,
        system_info,
        app_start: std::time::Instant::now(),
        api_token_path: api_token_path.clone(),
        auth_header_values,
    });

    // Build router. Routes requiring auth are nested under a middleware layer.
    let authed_routes = Router::new()
        // GET lighthouse/*
        .route("/lighthouse/version", get(get_node_version::<T, E>))
        .route("/lighthouse/health", get(get_lighthouse_health::<T, E>))
        .route("/lighthouse/spec", get(get_lighthouse_spec::<T, E>))
        .route(
            "/lighthouse/validators",
            get(get_lighthouse_validators::<T, E>),
        )
        .route(
            "/lighthouse/validators/{validator_pubkey}",
            get(get_lighthouse_validators_pubkey::<T, E>),
        )
        .route(
            "/lighthouse/ui/health",
            get(get_lighthouse_ui_health::<T, E>),
        )
        .route(
            "/lighthouse/ui/graffiti",
            get(get_lighthouse_ui_graffiti::<T, E>),
        )
        .route(
            "/lighthouse/beacon/health",
            get(get_lighthouse_beacon_health::<T, E>),
        )
        // POST lighthouse/*
        .route("/lighthouse/validators", post(post_validators::<T, E>))
        .route(
            "/lighthouse/validators/mnemonic",
            post(post_validators_mnemonic::<T, E>),
        )
        .route(
            "/lighthouse/validators/keystore",
            post(post_validators_keystore::<T, E>),
        )
        .route(
            "/lighthouse/validators/web3signer",
            post(post_validators_web3signer::<T, E>),
        )
        // PATCH lighthouse/*
        .route(
            "/lighthouse/validators/{validator_pubkey}",
            patch(patch_validators::<T, E>),
        )
        // DELETE lighthouse/*
        .route(
            "/lighthouse/keystores",
            delete(delete_lighthouse_keystores::<T, E>),
        )
        // POST lighthouse/beacon/update
        .route(
            "/lighthouse/beacon/update",
            post(post_lighthouse_beacon_update::<T, E>),
        )
        // Standard key-manager endpoints
        .route("/eth/v1/keystores", get(get_std_keystores::<T, E>))
        .route("/eth/v1/keystores", post(post_std_keystores::<T, E>))
        .route("/eth/v1/keystores", delete(delete_std_keystores::<T, E>))
        .route("/eth/v1/remotekeys", get(get_std_remotekeys::<T, E>))
        .route("/eth/v1/remotekeys", post(post_std_remotekeys::<T, E>))
        .route("/eth/v1/remotekeys", delete(delete_std_remotekeys::<T, E>))
        // Standard validator endpoints
        .route(
            "/eth/v1/validator/{validator_pubkey}/feerecipient",
            get(get_fee_recipient::<T, E>),
        )
        .route(
            "/eth/v1/validator/{validator_pubkey}/feerecipient",
            post(post_fee_recipient::<T, E>),
        )
        .route(
            "/eth/v1/validator/{validator_pubkey}/feerecipient",
            delete(delete_fee_recipient::<T, E>),
        )
        .route(
            "/eth/v1/validator/{validator_pubkey}/gas_limit",
            get(get_gas_limit::<T, E>),
        )
        .route(
            "/eth/v1/validator/{validator_pubkey}/gas_limit",
            post(post_gas_limit::<T, E>),
        )
        .route(
            "/eth/v1/validator/{validator_pubkey}/gas_limit",
            delete(delete_gas_limit::<T, E>),
        )
        .route(
            "/eth/v1/validator/{validator_pubkey}/voluntary_exit",
            post(post_validators_voluntary_exits::<T, E>),
        )
        .route(
            "/eth/v1/validator/{validator_pubkey}/graffiti",
            get(get_graffiti_endpoint::<T, E>),
        )
        .route(
            "/eth/v1/validator/{validator_pubkey}/graffiti",
            post(post_graffiti::<T, E>),
        )
        .route(
            "/eth/v1/validator/{validator_pubkey}/graffiti",
            delete(delete_graffiti_endpoint::<T, E>),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware::<T, E>,
        ));

    // Unauthenticated routes.
    let unauthed_routes = Router::new()
        .route("/lighthouse/auth", get(get_auth::<T, E>))
        .route("/lighthouse/logs", get(get_log_events::<T, E>));

    let app = Router::new()
        .merge(authed_routes)
        .merge(unauthed_routes)
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::header::SERVER,
            axum::http::HeaderValue::from_str(&version_with_platform())
                .unwrap_or_else(|_| axum::http::HeaderValue::from_static("vibehouse")),
        ))
        .layer(cors_layer)
        .with_state(state);

    let listener = std::net::TcpListener::bind(listen_addr).map_err(Error::Io)?;
    listener.set_nonblocking(true).map_err(Error::Io)?;
    let listening_socket = listener.local_addr().map_err(Error::Io)?;

    let server = async move {
        let listener = tokio::net::TcpListener::from_std(listener).expect("valid std listener");
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await;
    };

    info!(
        listen_address = listening_socket.to_string(),
        ?api_token_path,
        "HTTP API started"
    );

    Ok((listening_socket, server))
}

// ── Auth middleware ──────────────────────────────────────────────────────────

async fn auth_middleware<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    match headers.get("Authorization").map(|v| v.to_str()) {
        Some(Ok(header)) => {
            if state.auth_header_values.iter().any(|v| v == header) {
                next.run(request).await
            } else {
                ApiError::Forbidden(format!("Invalid auth token: {}", header)).into_response()
            }
        }
        _ => ApiError::Unauthorized("missing Authorization header".to_string()).into_response(),
    }
}

// ── Helper: run blocking task and return JSON ───────────────────────────────

async fn blocking_json<F, T>(func: F) -> Result<Json<T>, ApiError>
where
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
    T: serde::Serialize + Send + 'static,
{
    tokio::task::spawn_blocking(func)
        .await
        .map_err(|_| ApiError::ServerError("task panicked".to_string()))?
        .map(Json)
}

// ── Helper: extract validated services ──────────────────────────────────────

fn get_validator_store<T: SlotClock, E: EthSpec>(
    state: &AppState<T, E>,
) -> Result<Arc<VibehouseValidatorStore<T, E>>, ApiError> {
    state
        .ctx
        .validator_store
        .clone()
        .ok_or_else(|| ApiError::NotFound("validator store is not initialized.".to_string()))
}

fn get_validator_dir<T: SlotClock, E: EthSpec>(
    state: &AppState<T, E>,
) -> Result<PathBuf, ApiError> {
    state.ctx.validator_dir.clone().ok_or_else(|| {
        ApiError::NotFound("validator_dir directory is not initialized.".to_string())
    })
}

fn get_secrets_dir<T: SlotClock, E: EthSpec>(state: &AppState<T, E>) -> Result<PathBuf, ApiError> {
    state
        .ctx
        .secrets_dir
        .clone()
        .ok_or_else(|| ApiError::NotFound("secrets_dir directory is not initialized.".to_string()))
}

fn get_block_service<T: SlotClock + Clone, E: EthSpec>(
    state: &AppState<T, E>,
) -> Result<BlockService<VibehouseValidatorStore<T, E>, T>, ApiError> {
    state
        .ctx
        .block_service
        .clone()
        .ok_or_else(|| ApiError::NotFound("block service is not initialized.".to_string()))
}

// ── GET handlers ────────────────────────────────────────────────────────────

async fn get_node_version<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(_state): State<SharedState<T, E>>,
) -> Result<Json<GenericResponse<api_types::VersionData>>, ApiError> {
    blocking_json(move || {
        Ok(GenericResponse::from(api_types::VersionData {
            version: version_with_platform(),
        }))
    })
    .await
}

async fn get_lighthouse_health<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(_state): State<SharedState<T, E>>,
) -> Result<impl IntoResponse, ApiError> {
    blocking_json(move || {
        eth2::vibehouse::Health::observe()
            .map(GenericResponse::from)
            .map_err(|e| ApiError::BadRequest(e.to_string()))
    })
    .await
}

async fn get_lighthouse_spec<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
) -> Result<impl IntoResponse, ApiError> {
    let spec = state.ctx.spec.clone();
    blocking_json(move || {
        let config = ConfigAndPreset::from_chain_spec::<E>(&spec);
        Ok(GenericResponse::from(config))
    })
    .await
}

async fn get_lighthouse_validators<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    blocking_json(move || {
        let validators = validator_store
            .initialized_validators()
            .read()
            .validator_definitions()
            .iter()
            .map(|def| api_types::ValidatorData {
                enabled: def.enabled,
                description: def.description.clone(),
                voting_pubkey: PublicKeyBytes::from(&def.voting_public_key),
            })
            .collect::<Vec<_>>();

        Ok(GenericResponse::from(validators))
    })
    .await
}

async fn get_lighthouse_validators_pubkey<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Path(validator_pubkey): Path<PublicKey>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    blocking_json(move || {
        let validator = validator_store
            .initialized_validators()
            .read()
            .validator_definitions()
            .iter()
            .find(|def| def.voting_public_key == validator_pubkey)
            .map(|def| api_types::ValidatorData {
                enabled: def.enabled,
                description: def.description.clone(),
                voting_pubkey: PublicKeyBytes::from(&def.voting_public_key),
            })
            .ok_or_else(|| {
                ApiError::NotFound(format!("no validator for {:?}", validator_pubkey))
            })?;

        Ok(GenericResponse::from(validator))
    })
    .await
}

async fn get_lighthouse_ui_health<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
) -> Result<impl IntoResponse, ApiError> {
    let val_dir = get_validator_dir(&state)?;
    let sysinfo = state.system_info.clone();
    let app_start = state.app_start;
    blocking_json(move || {
        {
            let mut sysinfo_lock = sysinfo.write();
            sysinfo_lock.refresh_memory();
            sysinfo_lock.refresh_cpu_specifics(sysinfo::CpuRefreshKind::everything());
            sysinfo_lock.refresh_cpu();
            sysinfo_lock.refresh_system();
            sysinfo_lock.refresh_networks();
            sysinfo_lock.refresh_disks();
        }
        let app_uptime = app_start.elapsed().as_secs();
        Ok(GenericResponse::from(observe_system_health_vc(
            sysinfo, val_dir, app_uptime,
        )))
    })
    .await
}

async fn get_lighthouse_ui_graffiti<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    let graffiti_file = state.ctx.graffiti_file.clone();
    let graffiti_flag = state.ctx.graffiti_flag;
    blocking_json(move || {
        let mut result = HashMap::new();
        for (key, graffiti_definition) in validator_store
            .initialized_validators()
            .read()
            .get_all_validators_graffiti()
        {
            let graffiti = determine_graffiti(
                key,
                graffiti_file.clone(),
                graffiti_definition,
                graffiti_flag,
            );
            result.insert(key.to_string(), graffiti.map(|g| g.as_utf8_lossy()));
        }
        Ok(GenericResponse::from(result))
    })
    .await
}

async fn get_lighthouse_beacon_health<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
) -> Result<impl IntoResponse, ApiError> {
    let block_service = get_block_service(&state)?;
    let mut result: HashMap<String, Vec<CandidateInfo>> = HashMap::new();

    let mut beacon_nodes = Vec::new();
    for node in &*block_service.beacon_nodes.candidates.read().await {
        beacon_nodes.push(CandidateInfo {
            index: node.index,
            endpoint: node.beacon_node.to_string(),
            health: *node.health.read().await,
        });
    }
    result.insert("beacon_nodes".to_string(), beacon_nodes);

    if let Some(proposer_nodes_list) = &block_service.proposer_nodes {
        let mut proposer_nodes = Vec::new();
        for node in &*proposer_nodes_list.candidates.read().await {
            proposer_nodes.push(CandidateInfo {
                index: node.index,
                endpoint: node.beacon_node.to_string(),
                health: *node.health.read().await,
            });
        }
        result.insert("proposer_nodes".to_string(), proposer_nodes);
    }

    Ok(Json(GenericResponse::from(result)))
}

async fn get_fee_recipient<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Path(validator_pubkey): Path<PublicKey>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    blocking_json(move || {
        if validator_store
            .initialized_validators()
            .read()
            .is_enabled(&validator_pubkey)
            .is_none()
        {
            return Err(ApiError::NotFound(format!(
                "no validator found with pubkey {:?}",
                validator_pubkey
            )));
        }
        validator_store
            .get_fee_recipient(&PublicKeyBytes::from(&validator_pubkey))
            .map(|fee_recipient| {
                GenericResponse::from(GetFeeRecipientResponse {
                    pubkey: PublicKeyBytes::from(validator_pubkey.clone()),
                    ethaddress: fee_recipient,
                })
            })
            .ok_or_else(|| ApiError::ServerError("no fee recipient set".to_string()))
    })
    .await
}

async fn get_gas_limit<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Path(validator_pubkey): Path<PublicKey>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    blocking_json(move || {
        if validator_store
            .initialized_validators()
            .read()
            .is_enabled(&validator_pubkey)
            .is_none()
        {
            return Err(ApiError::NotFound(format!(
                "no validator found with pubkey {:?}",
                validator_pubkey
            )));
        }
        Ok(GenericResponse::from(GetGasLimitResponse {
            pubkey: PublicKeyBytes::from(validator_pubkey.clone()),
            gas_limit: validator_store.get_gas_limit(&PublicKeyBytes::from(&validator_pubkey)),
        }))
    })
    .await
}

async fn get_graffiti_endpoint<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Path(pubkey): Path<PublicKey>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    let graffiti_flag = state.ctx.graffiti_flag;
    blocking_json(move || {
        let graffiti = get_graffiti(pubkey.clone(), validator_store, graffiti_flag)?;
        Ok(GenericResponse::from(GetGraffitiResponse {
            pubkey: pubkey.into(),
            graffiti,
        }))
    })
    .await
}

async fn get_std_keystores<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    blocking_json(move || Ok(keystores::list(validator_store))).await
}

async fn get_std_remotekeys<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    blocking_json(move || Ok(remotekeys::list(validator_store))).await
}

async fn get_auth<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
) -> Result<impl IntoResponse, ApiError> {
    let token_path = state.api_token_path.clone();
    blocking_json(move || {
        Ok(AuthResponse {
            token_path: token_path.display().to_string(),
        })
    })
    .await
}

// ── POST handlers ───────────────────────────────────────────────────────────

async fn post_validators<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Json(body): Json<Vec<api_types::ValidatorRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_dir = get_validator_dir(&state)?;
    let secrets_dir = get_secrets_dir(&state)?;
    let validator_store = get_validator_store(&state)?;
    let spec = state.ctx.spec.clone();
    let task_executor = state.ctx.task_executor.clone();
    let store_passwords_in_secrets_dir = state.ctx.config.store_passwords_in_secrets_dir;
    blocking_json(move || {
        let secrets_dir = store_passwords_in_secrets_dir.then_some(secrets_dir);
        if let Some(handle) = task_executor.handle() {
            let (validators, mnemonic) = handle.block_on(create_validators_mnemonic::<_, _, E>(
                None,
                None,
                &body,
                &validator_dir,
                secrets_dir,
                &validator_store,
                &spec,
            ))?;
            let response = api_types::PostValidatorsResponseData {
                mnemonic: mnemonic.into_phrase().into(),
                validators,
            };
            Ok(GenericResponse::from(response))
        } else {
            Err(ApiError::ServerError("vibehouse shutting down".into()))
        }
    })
    .await
}

async fn post_validators_mnemonic<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Json(body): Json<api_types::CreateValidatorsMnemonicRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_dir = get_validator_dir(&state)?;
    let secrets_dir = get_secrets_dir(&state)?;
    let validator_store = get_validator_store(&state)?;
    let spec = state.ctx.spec.clone();
    let task_executor = state.ctx.task_executor.clone();
    let store_passwords_in_secrets_dir = state.ctx.config.store_passwords_in_secrets_dir;
    blocking_json(move || {
        let secrets_dir = store_passwords_in_secrets_dir.then_some(secrets_dir);
        if let Some(handle) = task_executor.handle() {
            let mnemonic = mnemonic_from_phrase(body.mnemonic.as_str())
                .map_err(|e| ApiError::BadRequest(format!("invalid mnemonic: {:?}", e)))?;
            let (validators, _mnemonic) =
                handle.block_on(create_validators_mnemonic::<_, _, E>(
                    Some(mnemonic),
                    Some(body.key_derivation_path_offset),
                    &body.validators,
                    &validator_dir,
                    secrets_dir,
                    &validator_store,
                    &spec,
                ))?;
            Ok(GenericResponse::from(validators))
        } else {
            Err(ApiError::ServerError("vibehouse shutting down".into()))
        }
    })
    .await
}

async fn post_validators_keystore<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Json(body): Json<api_types::KeystoreValidatorsPostRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_dir = get_validator_dir(&state)?;
    let secrets_dir = get_secrets_dir(&state)?;
    let validator_store = get_validator_store(&state)?;
    let task_executor = state.ctx.task_executor.clone();
    let store_passwords_in_secrets_dir = state.ctx.config.store_passwords_in_secrets_dir;
    blocking_json(move || {
        // Check to ensure the password is correct.
        let keypair = body
            .keystore
            .decrypt_keypair(body.password.as_ref())
            .map_err(|e| ApiError::BadRequest(format!("invalid keystore: {:?}", e)))?;

        let secrets_dir = store_passwords_in_secrets_dir.then_some(secrets_dir);
        let password_storage =
            get_voting_password_storage(&secrets_dir, &body.keystore, &body.password)?;

        let validator_dir = ValidatorDirBuilder::new(validator_dir.clone())
            .password_dir_opt(secrets_dir)
            .voting_keystore(body.keystore.clone(), body.password.as_ref())
            .store_withdrawal_keystore(false)
            .build()
            .map_err(|e| {
                ApiError::ServerError(format!("failed to build validator directory: {:?}", e))
            })?;

        // Drop validator dir so that `add_validator_keystore` can re-lock the keystore.
        let voting_keystore_path = validator_dir.voting_keystore_path();
        drop(validator_dir);
        let graffiti = body.graffiti.clone();
        let suggested_fee_recipient = body.suggested_fee_recipient;
        let gas_limit = body.gas_limit;
        let builder_proposals = body.builder_proposals;
        let builder_boost_factor = body.builder_boost_factor;
        let prefer_builder_proposals = body.prefer_builder_proposals;

        let validator_def = {
            if let Some(handle) = task_executor.handle() {
                handle
                    .block_on(validator_store.add_validator_keystore(
                        voting_keystore_path,
                        password_storage,
                        body.enable,
                        graffiti,
                        suggested_fee_recipient,
                        gas_limit,
                        builder_proposals,
                        builder_boost_factor,
                        prefer_builder_proposals,
                    ))
                    .map_err(|e| {
                        ApiError::ServerError(format!("failed to initialize validator: {:?}", e))
                    })?
            } else {
                return Err(ApiError::ServerError("vibehouse shutting down".into()));
            }
        };

        Ok(GenericResponse::from(api_types::ValidatorData {
            enabled: body.enable,
            description: validator_def.description,
            voting_pubkey: keypair.pk.into(),
        }))
    })
    .await
}

async fn post_validators_web3signer<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Json(body): Json<Vec<api_types::Web3SignerValidatorRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    let task_executor = state.ctx.task_executor.clone();
    blocking_json(move || {
        if let Some(handle) = task_executor.handle() {
            let web3signers: Vec<ValidatorDefinition> = body
                .into_iter()
                .map(|web3signer| ValidatorDefinition {
                    enabled: web3signer.enable,
                    voting_public_key: web3signer.voting_public_key,
                    graffiti: web3signer.graffiti,
                    suggested_fee_recipient: web3signer.suggested_fee_recipient,
                    gas_limit: web3signer.gas_limit,
                    builder_proposals: web3signer.builder_proposals,
                    builder_boost_factor: web3signer.builder_boost_factor,
                    prefer_builder_proposals: web3signer.prefer_builder_proposals,
                    description: web3signer.description,
                    signing_definition: SigningDefinition::Web3Signer(Web3SignerDefinition {
                        url: web3signer.url,
                        root_certificate_path: web3signer.root_certificate_path,
                        request_timeout_ms: web3signer.request_timeout_ms,
                        client_identity_path: web3signer.client_identity_path,
                        client_identity_password: web3signer.client_identity_password,
                    }),
                })
                .collect();
            handle.block_on(create_validators_web3signer::<_, E>(
                web3signers,
                &validator_store,
            ))?;
            Ok(())
        } else {
            Err(ApiError::ServerError("vibehouse shutting down".into()))
        }
    })
    .await
}

async fn post_fee_recipient<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Path(validator_pubkey): Path<PublicKey>,
    Json(request): Json<api_types::UpdateFeeRecipientRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    blocking_json(move || {
        if validator_store
            .initialized_validators()
            .read()
            .is_enabled(&validator_pubkey)
            .is_none()
        {
            return Err(ApiError::NotFound(format!(
                "no validator found with pubkey {:?}",
                validator_pubkey
            )));
        }
        validator_store
            .initialized_validators()
            .write()
            .set_validator_fee_recipient(&validator_pubkey, request.ethaddress)
            .map_err(|e| ApiError::ServerError(format!("Error persisting fee recipient: {:?}", e)))
    })
    .await
    .map(|reply| (StatusCode::ACCEPTED, reply))
}

async fn post_gas_limit<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Path(validator_pubkey): Path<PublicKey>,
    Json(request): Json<api_types::UpdateGasLimitRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    blocking_json(move || {
        if validator_store
            .initialized_validators()
            .read()
            .is_enabled(&validator_pubkey)
            .is_none()
        {
            return Err(ApiError::NotFound(format!(
                "no validator found with pubkey {:?}",
                validator_pubkey
            )));
        }
        validator_store
            .initialized_validators()
            .write()
            .set_validator_gas_limit(&validator_pubkey, request.gas_limit)
            .map_err(|e| ApiError::ServerError(format!("Error persisting gas limit: {:?}", e)))
    })
    .await
    .map(|reply| (StatusCode::ACCEPTED, reply))
}

async fn post_validators_voluntary_exits<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Path(pubkey): Path<PublicKey>,
    Query(query): Query<api_types::VoluntaryExitQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    let slot_clock = state.ctx.slot_clock.clone();
    let task_executor = state.ctx.task_executor.clone();
    blocking_json(move || {
        if let Some(handle) = task_executor.handle() {
            let signed_voluntary_exit = handle.block_on(create_signed_voluntary_exit::<T, E>(
                pubkey,
                query.epoch,
                validator_store,
                slot_clock,
            ))?;
            Ok(signed_voluntary_exit)
        } else {
            Err(ApiError::ServerError("vibehouse shutting down".into()))
        }
    })
    .await
}

async fn post_graffiti<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Path(pubkey): Path<PublicKey>,
    Json(query): Json<SetGraffitiRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    let graffiti_file = state.ctx.graffiti_file.clone();
    blocking_json(move || {
        if graffiti_file.is_some() {
            return Err(ApiError::Forbidden(
                "Unable to update graffiti as the \"--graffiti-file\" flag is set".to_string(),
            ));
        }
        set_graffiti(pubkey.clone(), query.graffiti, validator_store)
    })
    .await
    .map(|reply| (StatusCode::ACCEPTED, reply))
}

async fn post_std_keystores<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Json(request): Json<eth2::vibehouse_vc::std_types::ImportKeystoresRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_dir = get_validator_dir(&state)?;
    let secrets_dir = get_secrets_dir(&state)?;
    let validator_store = get_validator_store(&state)?;
    let task_executor = state.ctx.task_executor.clone();
    let store_passwords_in_secrets_dir = state.ctx.config.store_passwords_in_secrets_dir;
    let secrets_dir = store_passwords_in_secrets_dir.then_some(secrets_dir);
    blocking_json(move || {
        keystores::import::<_, E>(
            request,
            validator_dir,
            secrets_dir,
            validator_store,
            task_executor,
        )
    })
    .await
}

async fn post_std_remotekeys<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Json(request): Json<eth2::vibehouse_vc::std_types::ImportRemotekeysRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    let task_executor = state.ctx.task_executor.clone();
    blocking_json(move || remotekeys::import::<_, E>(request, validator_store, task_executor)).await
}

async fn post_lighthouse_beacon_update<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Json(request): Json<UpdateCandidatesRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let block_service = get_block_service(&state)?;
    let use_long_timeouts = state.ctx.config.bn_long_timeouts;

    let beacons: Vec<SensitiveUrl> = request
        .beacon_nodes
        .iter()
        .map(|url| SensitiveUrl::parse(url).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()
        .map_err(|_| ApiError::BadRequest("one or more urls could not be parsed".to_string()))?;

    let beacons = block_service
        .beacon_nodes
        .update_candidates_list(beacons, use_long_timeouts)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let response = UpdateCandidatesResponse {
        new_beacon_nodes_list: beacons.iter().map(|surl| surl.to_string()).collect(),
    };

    Ok(Json(GenericResponse::from(response)))
}

// ── PATCH handlers ──────────────────────────────────────────────────────────

async fn patch_validators<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Path(validator_pubkey): Path<PublicKey>,
    Json(body): Json<api_types::ValidatorPatchRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    let graffiti_file = state.ctx.graffiti_file.clone();
    let task_executor = state.ctx.task_executor.clone();
    blocking_json(move || {
        if body.graffiti.is_some() && graffiti_file.is_some() {
            return Err(ApiError::BadRequest(
                "Unable to update graffiti as the \"--graffiti-file\" flag is set".to_string(),
            ));
        }

        let maybe_graffiti = body.graffiti.clone().map(Into::into);
        let initialized_validators_rw_lock = validator_store.initialized_validators();
        let initialized_validators = initialized_validators_rw_lock.upgradable_read();

        // Do not make any changes if all fields are identical or unchanged.
        fn equal_or_none<T: PartialEq>(current_value: Option<T>, new_value: Option<T>) -> bool {
            new_value.is_none() || current_value == new_value
        }

        match (
            initialized_validators.is_enabled(&validator_pubkey),
            initialized_validators.validator(&validator_pubkey.compress()),
        ) {
            (None, _) => Err(ApiError::NotFound(format!(
                "no validator for {:?}",
                validator_pubkey
            ))),
            (Some(is_enabled), Some(initialized_validator))
                if equal_or_none(Some(is_enabled), body.enabled)
                    && equal_or_none(initialized_validator.get_gas_limit(), body.gas_limit)
                    && equal_or_none(
                        initialized_validator.get_builder_boost_factor(),
                        body.builder_boost_factor,
                    )
                    && equal_or_none(
                        initialized_validator.get_builder_proposals(),
                        body.builder_proposals,
                    )
                    && equal_or_none(
                        initialized_validator.get_prefer_builder_proposals(),
                        body.prefer_builder_proposals,
                    )
                    && equal_or_none(initialized_validator.get_graffiti(), maybe_graffiti) =>
            {
                Ok(())
            }
            (Some(false), None)
                if body.enabled.is_none_or(|enabled| !enabled)
                    && body.gas_limit.is_none()
                    && body.builder_boost_factor.is_none()
                    && body.builder_proposals.is_none()
                    && body.prefer_builder_proposals.is_none()
                    && maybe_graffiti.is_none() =>
            {
                Ok(())
            }
            (Some(_), _) => {
                let mut initialized_validators_write =
                    parking_lot::RwLockUpgradableReadGuard::upgrade(initialized_validators);
                if let Some(handle) = task_executor.handle() {
                    handle
                        .block_on(
                            initialized_validators_write.set_validator_definition_fields(
                                &validator_pubkey,
                                body.enabled,
                                body.gas_limit,
                                body.builder_proposals,
                                body.builder_boost_factor,
                                body.prefer_builder_proposals,
                                body.graffiti,
                            ),
                        )
                        .map_err(|e| {
                            ApiError::ServerError(format!(
                                "unable to set validator status: {:?}",
                                e
                            ))
                        })?;
                    Ok(())
                } else {
                    Err(ApiError::ServerError("vibehouse shutting down".into()))
                }
            }
        }
    })
    .await
}

// ── DELETE handlers ─────────────────────────────────────────────────────────

async fn delete_lighthouse_keystores<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Json(request): Json<eth2::vibehouse_vc::std_types::DeleteKeystoresRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    let task_executor = state.ctx.task_executor.clone();
    let allow_keystore_export = state.ctx.config.allow_keystore_export;
    blocking_json(move || {
        if allow_keystore_export {
            keystores::export(request, validator_store, task_executor)
        } else {
            Err(ApiError::BadRequest(
                "keystore export is disabled".to_string(),
            ))
        }
    })
    .await
}

async fn delete_fee_recipient<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Path(validator_pubkey): Path<PublicKey>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    blocking_json(move || {
        if validator_store
            .initialized_validators()
            .read()
            .is_enabled(&validator_pubkey)
            .is_none()
        {
            return Err(ApiError::NotFound(format!(
                "no validator found with pubkey {:?}",
                validator_pubkey
            )));
        }
        validator_store
            .initialized_validators()
            .write()
            .delete_validator_fee_recipient(&validator_pubkey)
            .map_err(|e| {
                ApiError::ServerError(format!("Error persisting fee recipient removal: {:?}", e))
            })
    })
    .await
    .map(|reply| (StatusCode::NO_CONTENT, reply))
}

async fn delete_gas_limit<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Path(validator_pubkey): Path<PublicKey>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    blocking_json(move || {
        if validator_store
            .initialized_validators()
            .read()
            .is_enabled(&validator_pubkey)
            .is_none()
        {
            return Err(ApiError::NotFound(format!(
                "no validator found with pubkey {:?}",
                validator_pubkey
            )));
        }
        validator_store
            .initialized_validators()
            .write()
            .delete_validator_gas_limit(&validator_pubkey)
            .map_err(|e| {
                ApiError::ServerError(format!("Error persisting gas limit removal: {:?}", e))
            })
    })
    .await
    .map(|reply| (StatusCode::NO_CONTENT, reply))
}

async fn delete_graffiti_endpoint<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Path(pubkey): Path<PublicKey>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    let graffiti_file = state.ctx.graffiti_file.clone();
    blocking_json(move || {
        if graffiti_file.is_some() {
            return Err(ApiError::Forbidden(
                "Unable to delete graffiti as the \"--graffiti-file\" flag is set".to_string(),
            ));
        }
        delete_graffiti(pubkey.clone(), validator_store)
    })
    .await
    .map(|reply| (StatusCode::NO_CONTENT, reply))
}

async fn delete_std_keystores<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Json(request): Json<eth2::vibehouse_vc::std_types::DeleteKeystoresRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    let task_executor = state.ctx.task_executor.clone();
    blocking_json(move || keystores::delete(request, validator_store, task_executor)).await
}

async fn delete_std_remotekeys<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
    Json(request): Json<eth2::vibehouse_vc::std_types::DeleteRemotekeysRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let validator_store = get_validator_store(&state)?;
    let task_executor = state.ctx.task_executor.clone();
    blocking_json(move || remotekeys::delete(request, validator_store, task_executor)).await
}

// ── SSE log events ──────────────────────────────────────────────────────────

async fn get_log_events<T: 'static + SlotClock + Clone, E: EthSpec>(
    State(state): State<SharedState<T, E>>,
) -> Result<Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let logging_components = state
        .ctx
        .sse_logging_components
        .clone()
        .ok_or_else(|| ApiError::ServerError("SSE Logging is not enabled".to_string()))?;

    let stream = BroadcastStream::new(logging_components.sender.subscribe()).map(|msg| match msg {
        Ok(data) => match serde_json::to_string(&data) {
            Ok(json) => Ok(Event::default().data(json)),
            Err(e) => Ok(Event::default().data(format!("serialization error: {:?}", e))),
        },
        Err(e) => Ok(Event::default().data(format!("receive error: {:?}", e))),
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

// ── CORS helper ─────────────────────────────────────────────────────────────

fn build_cors_layer(
    allow_origin: Option<&str>,
    listen_addr: IpAddr,
    listen_port: u16,
) -> Result<CorsLayer, Error> {
    let layer = CorsLayer::new()
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PATCH,
            axum::http::Method::DELETE,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
        ]);

    if let Some(allow_origin) = allow_origin {
        let origins: Vec<&str> = allow_origin.split(',').collect();
        if origins.contains(&"*") {
            Ok(layer.allow_origin(AllowOrigin::any()))
        } else {
            let parsed: Result<Vec<axum::http::HeaderValue>, _> = origins
                .iter()
                .map(|o| o.trim().parse::<axum::http::HeaderValue>())
                .collect();
            let parsed = parsed.map_err(|e| Error::Other(format!("Invalid CORS origin: {e}")))?;
            Ok(layer.allow_origin(parsed))
        }
    } else {
        let origin = match listen_addr {
            IpAddr::V4(_) => format!("http://{}:{}", listen_addr, listen_port),
            IpAddr::V6(_) => format!("http://[{}]:{}", listen_addr, listen_port),
        };
        let header_value: axum::http::HeaderValue = origin
            .parse()
            .map_err(|e| Error::Other(format!("Invalid default origin: {e}")))?;
        Ok(layer.allow_origin(header_value))
    }
}
