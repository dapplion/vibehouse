//! This crate provides a HTTP server that is solely dedicated to serving the `/metrics` endpoint.
//!
//! For other endpoints, see the `http_api` crate.
mod metrics;

use beacon_chain::{BeaconChain, BeaconChainTypes};
use lighthouse_network::prometheus_client::registry::Registry;
use logging::crit;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;
use vibehouse_version::version_with_platform;

use axum::{Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;

#[derive(Debug)]
pub enum Error {
    Io(#[allow(dead_code)] std::io::Error),
    Other(#[allow(dead_code)] String),
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
pub struct Context<T: BeaconChainTypes> {
    pub config: Config,
    pub chain: Option<Arc<BeaconChain<T>>>,
    pub db_path: Option<PathBuf>,
    pub freezer_db_path: Option<PathBuf>,
    pub gossipsub_registry: Option<std::sync::Mutex<Registry>>,
}

/// Configuration for the HTTP server.
#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub enabled: bool,
    pub listen_addr: IpAddr,
    pub listen_port: u16,
    pub allow_origin: Option<String>,
    pub allocator_metrics_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            listen_addr: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            listen_port: 5054,
            allow_origin: None,
            allocator_metrics_enabled: true,
        }
    }
}

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
pub fn serve<T: BeaconChainTypes>(
    ctx: Arc<Context<T>>,
    shutdown: impl Future<Output = ()> + Send + Sync + 'static,
) -> Result<(SocketAddr, impl Future<Output = ()>), Error> {
    let config = &ctx.config;

    // Sanity check.
    if !config.enabled {
        crit!("Cannot start disabled metrics HTTP server");
        return Err(Error::Other(
            "A disabled metrics server should not be started".to_string(),
        ));
    }

    // Configure CORS.
    let cors_layer = build_cors_layer(
        config.allow_origin.as_deref(),
        config.listen_addr,
        config.listen_port,
    )?;

    let app = Router::new()
        .route("/metrics", get(metrics_handler::<T>))
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::header::SERVER,
            axum::http::HeaderValue::from_str(&version_with_platform())
                .unwrap_or_else(|_| axum::http::HeaderValue::from_static("vibehouse")),
        ))
        .layer(cors_layer)
        .with_state(ctx.clone());

    let listen_addr = SocketAddr::new(config.listen_addr, config.listen_port);
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
        "Metrics HTTP server started"
    );

    Ok((listening_socket, server))
}

async fn metrics_handler<T: BeaconChainTypes>(
    State(ctx): State<Arc<Context<T>>>,
) -> impl IntoResponse {
    match metrics::gather_prometheus_metrics(&ctx) {
        Ok(body) => (StatusCode::OK, [("Content-Type", "text/plain")], body),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [("Content-Type", "text/plain")],
            format!("Unable to gather metrics: {:?}", e),
        ),
    }
}

fn build_cors_layer(
    allow_origin: Option<&str>,
    listen_addr: IpAddr,
    listen_port: u16,
) -> Result<CorsLayer, String> {
    let layer = CorsLayer::new()
        .allow_methods([axum::http::Method::GET])
        .allow_headers([axum::http::header::CONTENT_TYPE]);

    if let Some(allow_origin) = allow_origin {
        let origins: Vec<&str> = allow_origin.split(',').collect();
        if origins.contains(&"*") {
            Ok(layer.allow_origin(AllowOrigin::any()))
        } else {
            let parsed: Result<Vec<axum::http::HeaderValue>, _> = origins
                .iter()
                .map(|o| o.trim().parse::<axum::http::HeaderValue>())
                .collect();
            let parsed = parsed.map_err(|e| format!("Invalid CORS origin: {e}"))?;
            Ok(layer.allow_origin(parsed))
        }
    } else {
        let origin = match listen_addr {
            IpAddr::V4(_) => format!("http://{}:{}", listen_addr, listen_port),
            IpAddr::V6(_) => format!("http://[{}]:{}", listen_addr, listen_port),
        };
        let header_value: axum::http::HeaderValue = origin
            .parse()
            .map_err(|e| format!("Invalid default origin: {e}"))?;
        Ok(layer.allow_origin(header_value))
    }
}
