//! Contains an implementation of `EngineAPI` using the JSON-RPC API via HTTP.

use super::{
    BlockByNumberQuery, ClientCode, ClientVersionV1, EngineCapabilities, Error, EthSpec,
    ExecutionBlock, ExecutionBlockHash, ExecutionPayload, ExecutionPayloadBodyV1, ForkName,
    ForkchoiceState, ForkchoiceUpdatedResponse, GetPayloadResponse, GetPayloadResponseBellatrix,
    Hash256, NewPayloadRequest, NewPayloadRequestDeneb, NewPayloadRequestElectra,
    NewPayloadRequestFulu, NewPayloadRequestGloas, PayloadAttributes, PayloadId, PayloadStatusV1,
    Uint256,
};
use crate::auth::Auth;
use crate::json_structures::{
    BlobAndProofV1, BlobAndProofV2, JsonClientVersionV1, JsonExecutionPayload,
    JsonExecutionPayloadBellatrix, JsonExecutionPayloadBodyV1, JsonForkchoiceStateV1,
    JsonForkchoiceUpdatedV1Response, JsonGetPayloadResponse, JsonGetPayloadResponseBellatrix,
    JsonGetPayloadResponseCapella, JsonGetPayloadResponseDeneb, JsonGetPayloadResponseElectra,
    JsonGetPayloadResponseFulu, JsonGetPayloadResponseGloas, JsonPayloadAttributes,
    JsonPayloadIdRequest, JsonPayloadStatusV1, JsonRequestBody, JsonResponseBody,
};
use reqwest::header::CONTENT_TYPE;
use sensitive_url::SensitiveUrl;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use std::collections::HashSet;
use std::sync::LazyLock;
use tokio::sync::Mutex;
use vibehouse_version::{COMMIT_PREFIX, VERSION};

use std::time::{Duration, Instant};

pub use reqwest::Client;

const STATIC_ID: u32 = 1;
pub(crate) const JSONRPC_VERSION: &str = "2.0";

const RETURN_FULL_TRANSACTION_OBJECTS: bool = false;

pub(crate) const ETH_GET_BLOCK_BY_NUMBER: &str = "eth_getBlockByNumber";
const ETH_GET_BLOCK_BY_NUMBER_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) const ETH_GET_BLOCK_BY_HASH: &str = "eth_getBlockByHash";
const ETH_GET_BLOCK_BY_HASH_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) const ETH_SYNCING: &str = "eth_syncing";
const ETH_SYNCING_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) const ENGINE_NEW_PAYLOAD_V1: &str = "engine_newPayloadV1";
pub const ENGINE_NEW_PAYLOAD_V2: &str = "engine_newPayloadV2";
pub const ENGINE_NEW_PAYLOAD_V3: &str = "engine_newPayloadV3";
pub const ENGINE_NEW_PAYLOAD_V4: &str = "engine_newPayloadV4";
pub(crate) const ENGINE_NEW_PAYLOAD_V5: &str = "engine_newPayloadV5";
const ENGINE_NEW_PAYLOAD_TIMEOUT: Duration = Duration::from_secs(8);

pub(crate) const ENGINE_GET_PAYLOAD_V1: &str = "engine_getPayloadV1";
pub const ENGINE_GET_PAYLOAD_V2: &str = "engine_getPayloadV2";
pub const ENGINE_GET_PAYLOAD_V3: &str = "engine_getPayloadV3";
pub const ENGINE_GET_PAYLOAD_V4: &str = "engine_getPayloadV4";
pub const ENGINE_GET_PAYLOAD_V5: &str = "engine_getPayloadV5";
const ENGINE_GET_PAYLOAD_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) const ENGINE_FORKCHOICE_UPDATED_V1: &str = "engine_forkchoiceUpdatedV1";
pub const ENGINE_FORKCHOICE_UPDATED_V2: &str = "engine_forkchoiceUpdatedV2";
pub const ENGINE_FORKCHOICE_UPDATED_V3: &str = "engine_forkchoiceUpdatedV3";
const ENGINE_FORKCHOICE_UPDATED_TIMEOUT: Duration = Duration::from_secs(8);

pub(crate) const ENGINE_GET_PAYLOAD_BODIES_BY_HASH_V1: &str = "engine_getPayloadBodiesByHashV1";
pub(crate) const ENGINE_GET_PAYLOAD_BODIES_BY_RANGE_V1: &str = "engine_getPayloadBodiesByRangeV1";
const ENGINE_GET_PAYLOAD_BODIES_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) const ENGINE_EXCHANGE_CAPABILITIES: &str = "engine_exchangeCapabilities";
const ENGINE_EXCHANGE_CAPABILITIES_TIMEOUT: Duration = Duration::from_secs(1);

pub const ENGINE_GET_CLIENT_VERSION_V1: &str = "engine_getClientVersionV1";
const ENGINE_GET_CLIENT_VERSION_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) const ENGINE_GET_BLOBS_V1: &str = "engine_getBlobsV1";
pub(crate) const ENGINE_GET_BLOBS_V2: &str = "engine_getBlobsV2";
const ENGINE_GET_BLOBS_TIMEOUT: Duration = Duration::from_secs(1);

/// This error is returned during a `chainId` call by Geth.
const EIP155_ERROR_STR: &str = "chain not synced beyond EIP-155 replay-protection fork block";
/// This code is returned by all clients when a method is not supported
/// (verified geth, nethermind, erigon, besu)
pub(crate) const METHOD_NOT_FOUND_CODE: i64 = -32601;

pub(crate) static VIBEHOUSE_CAPABILITIES: &[&str] = &[
    ENGINE_NEW_PAYLOAD_V1,
    ENGINE_NEW_PAYLOAD_V2,
    ENGINE_NEW_PAYLOAD_V3,
    ENGINE_NEW_PAYLOAD_V4,
    ENGINE_NEW_PAYLOAD_V5,
    ENGINE_GET_PAYLOAD_V1,
    ENGINE_GET_PAYLOAD_V2,
    ENGINE_GET_PAYLOAD_V3,
    ENGINE_GET_PAYLOAD_V4,
    ENGINE_GET_PAYLOAD_V5,
    ENGINE_FORKCHOICE_UPDATED_V1,
    ENGINE_FORKCHOICE_UPDATED_V2,
    ENGINE_FORKCHOICE_UPDATED_V3,
    ENGINE_GET_PAYLOAD_BODIES_BY_HASH_V1,
    ENGINE_GET_PAYLOAD_BODIES_BY_RANGE_V1,
    ENGINE_GET_CLIENT_VERSION_V1,
    ENGINE_GET_BLOBS_V1,
    ENGINE_GET_BLOBS_V2,
];

/// We opt to initialize the JsonClientVersionV1 rather than the ClientVersionV1
/// for two reasons:
/// 1. This saves the overhead of converting into Json for every engine call
/// 2. The Json version lacks error checking so we can avoid calling `unwrap()`
pub(crate) static VIBEHOUSE_JSON_CLIENT_VERSION: LazyLock<JsonClientVersionV1> =
    LazyLock::new(|| JsonClientVersionV1 {
        code: ClientCode::Vibehouse.to_string(),
        name: "Vibehouse".to_string(),
        version: VERSION.replace("Vibehouse/", ""),
        commit: COMMIT_PREFIX.to_string(),
    });

pub(crate) struct CachedResponse<T: Clone> {
    pub data: T,
    pub fetch_time: Instant,
}

impl<T: Clone> CachedResponse<T> {
    pub(crate) fn new(data: T) -> Self {
        Self {
            data,
            fetch_time: Instant::now(),
        }
    }

    pub(crate) fn data(&self) -> T {
        self.data.clone()
    }

    pub(crate) fn age(&self) -> Duration {
        Instant::now().duration_since(self.fetch_time)
    }

    /// returns `true` if the entry's age is >= age_limit
    pub(crate) fn older_than(&self, age_limit: Option<Duration>) -> bool {
        age_limit.is_some_and(|limit| self.age() >= limit)
    }
}

pub(crate) struct HttpJsonRpc {
    pub client: Client,
    pub url: SensitiveUrl,
    pub execution_timeout_multiplier: u32,
    pub engine_capabilities_cache: Mutex<Option<CachedResponse<EngineCapabilities>>>,
    pub engine_version_cache: Mutex<Option<CachedResponse<Vec<ClientVersionV1>>>>,
    auth: Option<Auth>,
}

impl HttpJsonRpc {
    #[cfg(test)]
    pub(crate) fn new(
        url: SensitiveUrl,
        execution_timeout_multiplier: Option<u32>,
    ) -> Result<Self, Error> {
        Ok(Self {
            client: Client::builder().build()?,
            url,
            execution_timeout_multiplier: execution_timeout_multiplier.unwrap_or(1),
            engine_capabilities_cache: Mutex::new(None),
            engine_version_cache: Mutex::new(None),
            auth: None,
        })
    }

    pub(crate) fn new_with_auth(
        url: SensitiveUrl,
        auth: Auth,
        execution_timeout_multiplier: Option<u32>,
    ) -> Result<Self, Error> {
        Ok(Self {
            client: Client::builder().build()?,
            url,
            execution_timeout_multiplier: execution_timeout_multiplier.unwrap_or(1),
            engine_capabilities_cache: Mutex::new(None),
            engine_version_cache: Mutex::new(None),
            auth: Some(auth),
        })
    }

    pub(crate) async fn rpc_request<D: DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<D, Error> {
        let body = JsonRequestBody {
            jsonrpc: JSONRPC_VERSION,
            method,
            params,
            id: json!(STATIC_ID),
        };

        let mut request = self
            .client
            .post(self.url.full.clone())
            .timeout(timeout)
            .header(CONTENT_TYPE, "application/json")
            .json(&body);

        // Generate and add a jwt token to the header if auth is defined.
        if let Some(auth) = &self.auth {
            request = request.bearer_auth(auth.generate_token()?);
        }

        let body: JsonResponseBody = request.send().await?.error_for_status()?.json().await?;

        match (body.result, body.error) {
            (result, None) => serde_json::from_value(result).map_err(Into::into),
            (_, Some(error)) => {
                if error.message.contains(EIP155_ERROR_STR) {
                    Err(Error::Eip155Failure)
                } else {
                    Err(Error::ServerMessage {
                        code: error.code,
                        message: error.message,
                    })
                }
            }
        }
    }
}

impl std::fmt::Display for HttpJsonRpc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}, auth={}", self.url, self.auth.is_some())
    }
}

impl HttpJsonRpc {
    pub(crate) async fn upcheck(&self) -> Result<(), Error> {
        let result: serde_json::Value = self
            .rpc_request(
                ETH_SYNCING,
                json!([]),
                ETH_SYNCING_TIMEOUT * self.execution_timeout_multiplier,
            )
            .await?;

        match result.as_bool() {
            Some(false) => Ok(()),
            _ => Err(Error::IsSyncing),
        }
    }

    pub(crate) async fn get_blobs_v1<E: EthSpec>(
        &self,
        versioned_hashes: Vec<Hash256>,
    ) -> Result<Vec<Option<BlobAndProofV1<E>>>, Error> {
        let params = json!([versioned_hashes]);

        self.rpc_request(
            ENGINE_GET_BLOBS_V1,
            params,
            ENGINE_GET_BLOBS_TIMEOUT * self.execution_timeout_multiplier,
        )
        .await
    }

    pub(crate) async fn get_blobs_v2<E: EthSpec>(
        &self,
        versioned_hashes: Vec<Hash256>,
    ) -> Result<Option<Vec<BlobAndProofV2<E>>>, Error> {
        let params = json!([versioned_hashes]);

        self.rpc_request(
            ENGINE_GET_BLOBS_V2,
            params,
            ENGINE_GET_BLOBS_TIMEOUT * self.execution_timeout_multiplier,
        )
        .await
    }

    pub(crate) async fn get_block_by_number(
        &self,
        query: BlockByNumberQuery<'_>,
    ) -> Result<Option<ExecutionBlock>, Error> {
        let params = json!([query, RETURN_FULL_TRANSACTION_OBJECTS]);

        self.rpc_request(
            ETH_GET_BLOCK_BY_NUMBER,
            params,
            ETH_GET_BLOCK_BY_NUMBER_TIMEOUT * self.execution_timeout_multiplier,
        )
        .await
    }

    pub(crate) async fn get_block_by_hash(
        &self,
        block_hash: ExecutionBlockHash,
    ) -> Result<Option<ExecutionBlock>, Error> {
        let params = json!([block_hash, RETURN_FULL_TRANSACTION_OBJECTS]);

        self.rpc_request(
            ETH_GET_BLOCK_BY_HASH,
            params,
            ETH_GET_BLOCK_BY_HASH_TIMEOUT * self.execution_timeout_multiplier,
        )
        .await
    }

    pub(crate) async fn new_payload_v1<E: EthSpec>(
        &self,
        execution_payload: ExecutionPayload<E>,
    ) -> Result<PayloadStatusV1, Error> {
        let params = json!([JsonExecutionPayload::from(execution_payload)]);

        let response: JsonPayloadStatusV1 = self
            .rpc_request(
                ENGINE_NEW_PAYLOAD_V1,
                params,
                ENGINE_NEW_PAYLOAD_TIMEOUT * self.execution_timeout_multiplier,
            )
            .await?;

        Ok(response.into())
    }

    pub(crate) async fn new_payload_v2<E: EthSpec>(
        &self,
        execution_payload: ExecutionPayload<E>,
    ) -> Result<PayloadStatusV1, Error> {
        let params = json!([JsonExecutionPayload::from(execution_payload)]);

        let response: JsonPayloadStatusV1 = self
            .rpc_request(
                ENGINE_NEW_PAYLOAD_V2,
                params,
                ENGINE_NEW_PAYLOAD_TIMEOUT * self.execution_timeout_multiplier,
            )
            .await?;

        Ok(response.into())
    }

    pub(crate) async fn new_payload_v3_deneb<E: EthSpec>(
        &self,
        new_payload_request_deneb: NewPayloadRequestDeneb<'_, E>,
    ) -> Result<PayloadStatusV1, Error> {
        let params = json!([
            JsonExecutionPayload::Deneb(new_payload_request_deneb.execution_payload.clone().into()),
            new_payload_request_deneb.versioned_hashes,
            new_payload_request_deneb.parent_beacon_block_root,
        ]);

        let response: JsonPayloadStatusV1 = self
            .rpc_request(
                ENGINE_NEW_PAYLOAD_V3,
                params,
                ENGINE_NEW_PAYLOAD_TIMEOUT * self.execution_timeout_multiplier,
            )
            .await?;

        Ok(response.into())
    }

    pub(crate) async fn new_payload_v4_electra<E: EthSpec>(
        &self,
        new_payload_request_electra: NewPayloadRequestElectra<'_, E>,
    ) -> Result<PayloadStatusV1, Error> {
        let params = json!([
            JsonExecutionPayload::Electra(
                new_payload_request_electra.execution_payload.clone().into()
            ),
            new_payload_request_electra.versioned_hashes,
            new_payload_request_electra.parent_beacon_block_root,
            new_payload_request_electra
                .execution_requests
                .get_execution_requests_list(),
        ]);

        let response: JsonPayloadStatusV1 = self
            .rpc_request(
                ENGINE_NEW_PAYLOAD_V4,
                params,
                ENGINE_NEW_PAYLOAD_TIMEOUT * self.execution_timeout_multiplier,
            )
            .await?;

        Ok(response.into())
    }

    pub(crate) async fn new_payload_v4_fulu<E: EthSpec>(
        &self,
        new_payload_request_fulu: NewPayloadRequestFulu<'_, E>,
    ) -> Result<PayloadStatusV1, Error> {
        let params = json!([
            JsonExecutionPayload::Fulu(new_payload_request_fulu.execution_payload.clone().into()),
            new_payload_request_fulu.versioned_hashes,
            new_payload_request_fulu.parent_beacon_block_root,
            new_payload_request_fulu
                .execution_requests
                .get_execution_requests_list(),
        ]);

        let response: JsonPayloadStatusV1 = self
            .rpc_request(
                ENGINE_NEW_PAYLOAD_V4,
                params,
                ENGINE_NEW_PAYLOAD_TIMEOUT * self.execution_timeout_multiplier,
            )
            .await?;

        Ok(response.into())
    }

    pub(crate) async fn new_payload_v5_gloas<E: EthSpec>(
        &self,
        new_payload_request_gloas: NewPayloadRequestGloas<'_, E>,
    ) -> Result<PayloadStatusV1, Error> {
        let params = json!([
            JsonExecutionPayload::Gloas(new_payload_request_gloas.execution_payload.clone().into()),
            new_payload_request_gloas.versioned_hashes,
            new_payload_request_gloas.parent_beacon_block_root,
            new_payload_request_gloas
                .execution_requests
                .get_execution_requests_list(),
        ]);

        let response: JsonPayloadStatusV1 = self
            .rpc_request(
                ENGINE_NEW_PAYLOAD_V5,
                params,
                ENGINE_NEW_PAYLOAD_TIMEOUT * self.execution_timeout_multiplier,
            )
            .await?;

        Ok(response.into())
    }

    pub(crate) async fn get_payload_v1<E: EthSpec>(
        &self,
        payload_id: PayloadId,
    ) -> Result<GetPayloadResponse<E>, Error> {
        let params = json!([JsonPayloadIdRequest::from(payload_id)]);

        let payload_v1: JsonExecutionPayloadBellatrix<E> = self
            .rpc_request(
                ENGINE_GET_PAYLOAD_V1,
                params,
                ENGINE_GET_PAYLOAD_TIMEOUT * self.execution_timeout_multiplier,
            )
            .await?;

        Ok(GetPayloadResponse::Bellatrix(GetPayloadResponseBellatrix {
            execution_payload: payload_v1.into(),
            // Set the V1 payload values from the EE to be zero. This simulates
            // the pre-block-value functionality of always choosing the builder
            // block.
            block_value: Uint256::ZERO,
        }))
    }

    pub(crate) async fn get_payload_v2<E: EthSpec>(
        &self,
        fork_name: ForkName,
        payload_id: PayloadId,
    ) -> Result<GetPayloadResponse<E>, Error> {
        let params = json!([JsonPayloadIdRequest::from(payload_id)]);

        match fork_name {
            ForkName::Bellatrix => {
                let response: JsonGetPayloadResponseBellatrix<E> = self
                    .rpc_request(
                        ENGINE_GET_PAYLOAD_V2,
                        params,
                        ENGINE_GET_PAYLOAD_TIMEOUT * self.execution_timeout_multiplier,
                    )
                    .await?;
                JsonGetPayloadResponse::Bellatrix(response)
                    .try_into()
                    .map_err(Error::BadResponse)
            }
            ForkName::Capella => {
                let response: JsonGetPayloadResponseCapella<E> = self
                    .rpc_request(
                        ENGINE_GET_PAYLOAD_V2,
                        params,
                        ENGINE_GET_PAYLOAD_TIMEOUT * self.execution_timeout_multiplier,
                    )
                    .await?;
                JsonGetPayloadResponse::Capella(response)
                    .try_into()
                    .map_err(Error::BadResponse)
            }
            _ => Err(Error::UnsupportedForkVariant(format!(
                "called get_payload_v2 with {fork_name}"
            ))),
        }
    }

    pub(crate) async fn get_payload_v3<E: EthSpec>(
        &self,
        fork_name: ForkName,
        payload_id: PayloadId,
    ) -> Result<GetPayloadResponse<E>, Error> {
        let params = json!([JsonPayloadIdRequest::from(payload_id)]);

        match fork_name {
            ForkName::Deneb => {
                let response: JsonGetPayloadResponseDeneb<E> = self
                    .rpc_request(
                        ENGINE_GET_PAYLOAD_V3,
                        params,
                        ENGINE_GET_PAYLOAD_TIMEOUT * self.execution_timeout_multiplier,
                    )
                    .await?;
                JsonGetPayloadResponse::Deneb(response)
                    .try_into()
                    .map_err(Error::BadResponse)
            }
            _ => Err(Error::UnsupportedForkVariant(format!(
                "called get_payload_v3 with {fork_name}"
            ))),
        }
    }

    pub(crate) async fn get_payload_v4<E: EthSpec>(
        &self,
        fork_name: ForkName,
        payload_id: PayloadId,
    ) -> Result<GetPayloadResponse<E>, Error> {
        let params = json!([JsonPayloadIdRequest::from(payload_id)]);

        match fork_name {
            ForkName::Electra => {
                let response: JsonGetPayloadResponseElectra<E> = self
                    .rpc_request(
                        ENGINE_GET_PAYLOAD_V4,
                        params,
                        ENGINE_GET_PAYLOAD_TIMEOUT * self.execution_timeout_multiplier,
                    )
                    .await?;
                JsonGetPayloadResponse::Electra(response)
                    .try_into()
                    .map_err(Error::BadResponse)
            }
            _ => Err(Error::UnsupportedForkVariant(format!(
                "called get_payload_v4 with {fork_name}"
            ))),
        }
    }

    pub(crate) async fn get_payload_v5<E: EthSpec>(
        &self,
        fork_name: ForkName,
        payload_id: PayloadId,
    ) -> Result<GetPayloadResponse<E>, Error> {
        let params = json!([JsonPayloadIdRequest::from(payload_id)]);

        match fork_name {
            ForkName::Fulu => {
                let response: JsonGetPayloadResponseFulu<E> = self
                    .rpc_request(
                        ENGINE_GET_PAYLOAD_V5,
                        params,
                        ENGINE_GET_PAYLOAD_TIMEOUT * self.execution_timeout_multiplier,
                    )
                    .await?;
                JsonGetPayloadResponse::Fulu(response)
                    .try_into()
                    .map_err(Error::BadResponse)
            }
            ForkName::Gloas => {
                let response: JsonGetPayloadResponseGloas<E> = self
                    .rpc_request(
                        ENGINE_GET_PAYLOAD_V5,
                        params,
                        ENGINE_GET_PAYLOAD_TIMEOUT * self.execution_timeout_multiplier,
                    )
                    .await?;
                JsonGetPayloadResponse::Gloas(response)
                    .try_into()
                    .map_err(Error::BadResponse)
            }
            _ => Err(Error::UnsupportedForkVariant(format!(
                "called get_payload_v5 with {fork_name}"
            ))),
        }
    }

    pub(crate) async fn forkchoice_updated_v1(
        &self,
        forkchoice_state: ForkchoiceState,
        payload_attributes: Option<PayloadAttributes>,
    ) -> Result<ForkchoiceUpdatedResponse, Error> {
        let params = json!([
            JsonForkchoiceStateV1::from(forkchoice_state),
            payload_attributes.map(JsonPayloadAttributes::from)
        ]);

        let response: JsonForkchoiceUpdatedV1Response = self
            .rpc_request(
                ENGINE_FORKCHOICE_UPDATED_V1,
                params,
                ENGINE_FORKCHOICE_UPDATED_TIMEOUT * self.execution_timeout_multiplier,
            )
            .await?;

        Ok(response.into())
    }

    pub(crate) async fn forkchoice_updated_v2(
        &self,
        forkchoice_state: ForkchoiceState,
        payload_attributes: Option<PayloadAttributes>,
    ) -> Result<ForkchoiceUpdatedResponse, Error> {
        let params = json!([
            JsonForkchoiceStateV1::from(forkchoice_state),
            payload_attributes.map(JsonPayloadAttributes::from)
        ]);

        let response: JsonForkchoiceUpdatedV1Response = self
            .rpc_request(
                ENGINE_FORKCHOICE_UPDATED_V2,
                params,
                ENGINE_FORKCHOICE_UPDATED_TIMEOUT * self.execution_timeout_multiplier,
            )
            .await?;

        Ok(response.into())
    }

    pub(crate) async fn forkchoice_updated_v3(
        &self,
        forkchoice_state: ForkchoiceState,
        payload_attributes: Option<PayloadAttributes>,
    ) -> Result<ForkchoiceUpdatedResponse, Error> {
        let params = json!([
            JsonForkchoiceStateV1::from(forkchoice_state),
            payload_attributes.map(JsonPayloadAttributes::from)
        ]);

        let response: JsonForkchoiceUpdatedV1Response = self
            .rpc_request(
                ENGINE_FORKCHOICE_UPDATED_V3,
                params,
                ENGINE_FORKCHOICE_UPDATED_TIMEOUT * self.execution_timeout_multiplier,
            )
            .await?;

        Ok(response.into())
    }

    pub(crate) async fn get_payload_bodies_by_hash_v1<E: EthSpec>(
        &self,
        block_hashes: Vec<ExecutionBlockHash>,
    ) -> Result<Vec<Option<ExecutionPayloadBodyV1<E>>>, Error> {
        let params = json!([block_hashes]);

        let response: Vec<Option<JsonExecutionPayloadBodyV1<E>>> = self
            .rpc_request(
                ENGINE_GET_PAYLOAD_BODIES_BY_HASH_V1,
                params,
                ENGINE_GET_PAYLOAD_BODIES_TIMEOUT * self.execution_timeout_multiplier,
            )
            .await?;

        Ok(response
            .into_iter()
            .map(|opt_json| opt_json.map(From::from))
            .collect())
    }

    pub(crate) async fn get_payload_bodies_by_range_v1<E: EthSpec>(
        &self,
        start: u64,
        count: u64,
    ) -> Result<Vec<Option<ExecutionPayloadBodyV1<E>>>, Error> {
        #[derive(Serialize)]
        #[serde(transparent)]
        struct Quantity(#[serde(with = "serde_utils::u64_hex_be")] u64);

        let params = json!([Quantity(start), Quantity(count)]);
        let response: Vec<Option<JsonExecutionPayloadBodyV1<E>>> = self
            .rpc_request(
                ENGINE_GET_PAYLOAD_BODIES_BY_RANGE_V1,
                params,
                ENGINE_GET_PAYLOAD_BODIES_TIMEOUT * self.execution_timeout_multiplier,
            )
            .await?;

        Ok(response
            .into_iter()
            .map(|opt_json| opt_json.map(From::from))
            .collect())
    }

    pub(crate) async fn exchange_capabilities(&self) -> Result<EngineCapabilities, Error> {
        let params = json!([VIBEHOUSE_CAPABILITIES]);

        let capabilities: HashSet<String> = self
            .rpc_request(
                ENGINE_EXCHANGE_CAPABILITIES,
                params,
                ENGINE_EXCHANGE_CAPABILITIES_TIMEOUT * self.execution_timeout_multiplier,
            )
            .await?;

        Ok(EngineCapabilities {
            new_payload_v1: capabilities.contains(ENGINE_NEW_PAYLOAD_V1),
            new_payload_v2: capabilities.contains(ENGINE_NEW_PAYLOAD_V2),
            new_payload_v3: capabilities.contains(ENGINE_NEW_PAYLOAD_V3),
            new_payload_v4: capabilities.contains(ENGINE_NEW_PAYLOAD_V4),
            new_payload_v5: capabilities.contains(ENGINE_NEW_PAYLOAD_V5),
            forkchoice_updated_v1: capabilities.contains(ENGINE_FORKCHOICE_UPDATED_V1),
            forkchoice_updated_v2: capabilities.contains(ENGINE_FORKCHOICE_UPDATED_V2),
            forkchoice_updated_v3: capabilities.contains(ENGINE_FORKCHOICE_UPDATED_V3),
            get_payload_bodies_by_hash_v1: capabilities
                .contains(ENGINE_GET_PAYLOAD_BODIES_BY_HASH_V1),
            get_payload_bodies_by_range_v1: capabilities
                .contains(ENGINE_GET_PAYLOAD_BODIES_BY_RANGE_V1),
            get_payload_v1: capabilities.contains(ENGINE_GET_PAYLOAD_V1),
            get_payload_v2: capabilities.contains(ENGINE_GET_PAYLOAD_V2),
            get_payload_v3: capabilities.contains(ENGINE_GET_PAYLOAD_V3),
            get_payload_v4: capabilities.contains(ENGINE_GET_PAYLOAD_V4),
            get_payload_v5: capabilities.contains(ENGINE_GET_PAYLOAD_V5),
            get_client_version_v1: capabilities.contains(ENGINE_GET_CLIENT_VERSION_V1),
            get_blobs_v1: capabilities.contains(ENGINE_GET_BLOBS_V1),
            get_blobs_v2: capabilities.contains(ENGINE_GET_BLOBS_V2),
        })
    }

    pub(crate) async fn clear_exchange_capabilties_cache(&self) {
        *self.engine_capabilities_cache.lock().await = None;
    }

    /// Returns the execution engine capabilities resulting from a call to
    /// engine_exchangeCapabilities. If the capabilities cache is not populated,
    /// or if it is populated with a cached result of age >= `age_limit`, this
    /// method will fetch the result from the execution engine and populate the
    /// cache before returning it. Otherwise it will return a cached result from
    /// a previous call.
    ///
    /// Set `age_limit` to `None` to always return the cached result
    /// Set `age_limit` to `Some(Duration::ZERO)` to force fetching from EE
    pub(crate) async fn get_engine_capabilities(
        &self,
        age_limit: Option<Duration>,
    ) -> Result<EngineCapabilities, Error> {
        let mut lock = self.engine_capabilities_cache.lock().await;

        if let Some(lock) = lock
            .as_ref()
            .filter(|cached_response| !cached_response.older_than(age_limit))
        {
            Ok(lock.data())
        } else {
            let engine_capabilities = self.exchange_capabilities().await?;
            *lock = Some(CachedResponse::new(engine_capabilities));
            Ok(engine_capabilities)
        }
    }

    /// This method fetches the response from the engine without checking
    /// any caches or storing the result in the cache. It is better to use
    /// `get_engine_version(Some(Duration::ZERO))` if you want to force
    /// fetching from the EE as this will cache the result.
    pub(crate) async fn get_client_version_v1(&self) -> Result<Vec<ClientVersionV1>, Error> {
        let params = json!([*VIBEHOUSE_JSON_CLIENT_VERSION]);

        let response: Vec<JsonClientVersionV1> = self
            .rpc_request(
                ENGINE_GET_CLIENT_VERSION_V1,
                params,
                ENGINE_GET_CLIENT_VERSION_TIMEOUT * self.execution_timeout_multiplier,
            )
            .await?;

        response
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Error::InvalidClientVersion)
    }

    pub(crate) async fn clear_engine_version_cache(&self) {
        *self.engine_version_cache.lock().await = None;
    }

    /// Returns the execution engine version resulting from a call to
    /// engine_getClientVersionV1. If the version cache is not populated, or if it
    /// is populated with a cached result of age >= `age_limit`, this method will
    /// fetch the result from the execution engine and populate the cache before
    /// returning it. Otherwise it will return the cached result from an earlier
    /// call.
    ///
    /// Set `age_limit` to `None` to always return the cached result
    /// Set `age_limit` to `Some(Duration::ZERO)` to force fetching from EE
    pub(crate) async fn get_engine_version(
        &self,
        age_limit: Option<Duration>,
    ) -> Result<Vec<ClientVersionV1>, Error> {
        // check engine capabilities first (avoids holding two locks at once)
        let engine_capabilities = self.get_engine_capabilities(None).await?;
        if !engine_capabilities.get_client_version_v1 {
            // We choose an empty vec to denote that this method is not
            // supported instead of an error since this method is optional
            // & we don't want to log a warning and concern the user
            return Ok(vec![]);
        }
        let mut lock = self.engine_version_cache.lock().await;
        if let Some(lock) = lock
            .as_ref()
            .filter(|cached_response| !cached_response.older_than(age_limit))
        {
            Ok(lock.data())
        } else {
            let engine_version = self.get_client_version_v1().await?;
            *lock = Some(CachedResponse::new(engine_version.clone()));
            if !engine_version.is_empty() {
                // reset metric gauge when there's a fresh fetch
                crate::metrics::reset_execution_layer_info_gauge();
            }
            Ok(engine_version)
        }
    }

    // automatically selects the latest version of
    // new_payload that the execution engine supports
    pub(crate) async fn new_payload<E: EthSpec>(
        &self,
        new_payload_request: NewPayloadRequest<'_, E>,
    ) -> Result<PayloadStatusV1, Error> {
        let engine_capabilities = self.get_engine_capabilities(None).await?;
        match new_payload_request {
            NewPayloadRequest::Bellatrix(_) | NewPayloadRequest::Capella(_) => {
                if engine_capabilities.new_payload_v2 {
                    self.new_payload_v2(new_payload_request.into_execution_payload())
                        .await
                } else if engine_capabilities.new_payload_v1 {
                    self.new_payload_v1(new_payload_request.into_execution_payload())
                        .await
                } else {
                    Err(Error::RequiredMethodUnsupported("engine_newPayload"))
                }
            }
            NewPayloadRequest::Deneb(new_payload_request_deneb) => {
                if engine_capabilities.new_payload_v3 {
                    self.new_payload_v3_deneb(new_payload_request_deneb).await
                } else {
                    Err(Error::RequiredMethodUnsupported("engine_newPayloadV3"))
                }
            }
            NewPayloadRequest::Electra(new_payload_request_electra) => {
                if engine_capabilities.new_payload_v4 {
                    self.new_payload_v4_electra(new_payload_request_electra)
                        .await
                } else {
                    Err(Error::RequiredMethodUnsupported("engine_newPayloadV4"))
                }
            }
            NewPayloadRequest::Fulu(new_payload_request_fulu) => {
                if engine_capabilities.new_payload_v4 {
                    self.new_payload_v4_fulu(new_payload_request_fulu).await
                } else {
                    Err(Error::RequiredMethodUnsupported("engine_newPayloadV4"))
                }
            }
            NewPayloadRequest::Gloas(new_payload_request_gloas) => {
                if engine_capabilities.new_payload_v5 {
                    self.new_payload_v5_gloas(new_payload_request_gloas).await
                } else {
                    Err(Error::RequiredMethodUnsupported("engine_newPayloadV5"))
                }
            }
            NewPayloadRequest::Heze(new_payload_request_heze) => {
                if engine_capabilities.new_payload_v5 {
                    let as_gloas = NewPayloadRequestGloas {
                        execution_payload: new_payload_request_heze.execution_payload,
                        versioned_hashes: new_payload_request_heze.versioned_hashes,
                        parent_beacon_block_root: new_payload_request_heze.parent_beacon_block_root,
                        execution_requests: new_payload_request_heze.execution_requests,
                    };
                    self.new_payload_v5_gloas(as_gloas).await
                } else {
                    Err(Error::RequiredMethodUnsupported("engine_newPayloadV5"))
                }
            }
        }
    }

    // automatically selects the latest version of
    // get_payload that the execution engine supports
    pub(crate) async fn get_payload<E: EthSpec>(
        &self,
        fork_name: ForkName,
        payload_id: PayloadId,
    ) -> Result<GetPayloadResponse<E>, Error> {
        let engine_capabilities = self.get_engine_capabilities(None).await?;
        match fork_name {
            ForkName::Bellatrix | ForkName::Capella => {
                if engine_capabilities.get_payload_v2 {
                    self.get_payload_v2(fork_name, payload_id).await
                } else if engine_capabilities.get_payload_v1 {
                    self.get_payload_v1(payload_id).await
                } else {
                    Err(Error::RequiredMethodUnsupported("engine_getPayload"))
                }
            }
            ForkName::Deneb => {
                if engine_capabilities.get_payload_v3 {
                    self.get_payload_v3(fork_name, payload_id).await
                } else {
                    Err(Error::RequiredMethodUnsupported("engine_getPayloadv3"))
                }
            }
            ForkName::Electra => {
                if engine_capabilities.get_payload_v4 {
                    self.get_payload_v4(fork_name, payload_id).await
                } else {
                    Err(Error::RequiredMethodUnsupported("engine_getPayloadv4"))
                }
            }
            ForkName::Fulu | ForkName::Gloas | ForkName::Heze => {
                if engine_capabilities.get_payload_v5 {
                    self.get_payload_v5(fork_name, payload_id).await
                } else {
                    Err(Error::RequiredMethodUnsupported("engine_getPayloadv5"))
                }
            }
            ForkName::Base | ForkName::Altair => Err(Error::UnsupportedForkVariant(format!(
                "called get_payload with {fork_name}"
            ))),
        }
    }

    // automatically selects the latest version of
    // forkchoice_updated that the execution engine supports
    pub(crate) async fn forkchoice_updated(
        &self,
        forkchoice_state: ForkchoiceState,
        maybe_payload_attributes: Option<PayloadAttributes>,
    ) -> Result<ForkchoiceUpdatedResponse, Error> {
        let engine_capabilities = self.get_engine_capabilities(None).await?;
        if let Some(payload_attributes) = maybe_payload_attributes.as_ref() {
            match payload_attributes {
                PayloadAttributes::V1(_) | PayloadAttributes::V2(_) => {
                    if engine_capabilities.forkchoice_updated_v2 {
                        self.forkchoice_updated_v2(forkchoice_state, maybe_payload_attributes)
                            .await
                    } else if engine_capabilities.forkchoice_updated_v1 {
                        self.forkchoice_updated_v1(forkchoice_state, maybe_payload_attributes)
                            .await
                    } else {
                        Err(Error::RequiredMethodUnsupported("engine_forkchoiceUpdated"))
                    }
                }
                PayloadAttributes::V3(_) => {
                    if engine_capabilities.forkchoice_updated_v3 {
                        self.forkchoice_updated_v3(forkchoice_state, maybe_payload_attributes)
                            .await
                    } else {
                        Err(Error::RequiredMethodUnsupported(
                            "engine_forkchoiceUpdatedV3",
                        ))
                    }
                }
            }
        } else if engine_capabilities.forkchoice_updated_v3 {
            self.forkchoice_updated_v3(forkchoice_state, maybe_payload_attributes)
                .await
        } else if engine_capabilities.forkchoice_updated_v2 {
            self.forkchoice_updated_v2(forkchoice_state, maybe_payload_attributes)
                .await
        } else if engine_capabilities.forkchoice_updated_v1 {
            self.forkchoice_updated_v1(forkchoice_state, maybe_payload_attributes)
                .await
        } else {
            Err(Error::RequiredMethodUnsupported("engine_forkchoiceUpdated"))
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::auth::JwtKey;
    use crate::engine_api::{LATEST_TAG, PayloadAttributesV1, PayloadStatusV1Status};
    use crate::json_structures::TransparentJsonPayloadId;
    use crate::test_utils::{DEFAULT_JWT_SECRET, MockServer};
    use std::future::Future;
    use std::str::FromStr;
    use std::sync::Arc;
    use types::{
        Address, ExecutionPayloadBellatrix, FixedBytesExtended, MainnetEthSpec, Transactions,
        Unsigned, VariableList,
    };

    struct Tester {
        server: MockServer<MainnetEthSpec>,
        rpc_client: Arc<HttpJsonRpc>,
        echo_client: Arc<HttpJsonRpc>,
    }

    impl Tester {
        pub(crate) fn new(with_auth: bool) -> Self {
            let spec = Arc::new(MainnetEthSpec::default_spec());
            let server = MockServer::unit_testing(spec);

            let rpc_url = SensitiveUrl::parse(&server.url()).unwrap();
            let echo_url = SensitiveUrl::parse(&format!("{}/echo", server.url())).unwrap();
            // Create rpc clients that include JWT auth headers if `with_auth` is true.
            let (rpc_client, echo_client) = if with_auth {
                let rpc_auth =
                    Auth::new(JwtKey::from_slice(&DEFAULT_JWT_SECRET).unwrap(), None, None);
                let echo_auth =
                    Auth::new(JwtKey::from_slice(&DEFAULT_JWT_SECRET).unwrap(), None, None);
                (
                    Arc::new(HttpJsonRpc::new_with_auth(rpc_url, rpc_auth, None).unwrap()),
                    Arc::new(HttpJsonRpc::new_with_auth(echo_url, echo_auth, None).unwrap()),
                )
            } else {
                (
                    Arc::new(HttpJsonRpc::new(rpc_url, None).unwrap()),
                    Arc::new(HttpJsonRpc::new(echo_url, None).unwrap()),
                )
            };

            Self {
                server,
                rpc_client,
                echo_client,
            }
        }

        pub(crate) async fn assert_request_equals<R, F>(
            self,
            request_func: R,
            expected_json: serde_json::Value,
        ) -> Self
        where
            R: Fn(Arc<HttpJsonRpc>) -> F,
            F: Future<Output = ()>,
        {
            request_func(self.echo_client.clone()).await;
            let request_bytes = self.server.last_echo_request();
            let request_json: serde_json::Value =
                serde_json::from_slice(&request_bytes).expect("request was not valid json");
            assert!(
                request_json == expected_json,
                "json mismatch!\n\nobserved: {request_json}\n\nexpected: {expected_json}\n\n",
            );
            self
        }

        pub(crate) async fn assert_auth_failure<R, F, T>(self, request_func: R) -> Self
        where
            R: Fn(Arc<HttpJsonRpc>) -> F,
            F: Future<Output = Result<T, Error>>,
            T: std::fmt::Debug,
        {
            let res = request_func(self.echo_client.clone()).await;
            assert!(
                matches!(res, Err(Error::Auth(_))),
                "No authentication provided, rpc call should have failed.\nResult: {res:?}"
            );
            self
        }

        pub(crate) async fn with_preloaded_responses<R, F>(
            self,
            preloaded_responses: Vec<serde_json::Value>,
            request_func: R,
        ) -> Self
        where
            R: Fn(Arc<HttpJsonRpc>) -> F,
            F: Future<Output = ()>,
        {
            for response in preloaded_responses {
                self.server.push_preloaded_response(response);
            }
            request_func(self.rpc_client.clone()).await;
            self
        }
    }

    const HASH_00: &str = "0x0000000000000000000000000000000000000000000000000000000000000000";
    const HASH_01: &str = "0x0101010101010101010101010101010101010101010101010101010101010101";

    const ADDRESS_00: &str = "0x0000000000000000000000000000000000000000";
    const ADDRESS_01: &str = "0x0101010101010101010101010101010101010101";

    const JSON_NULL: serde_json::Value = serde_json::Value::Null;
    const LOGS_BLOOM_00: &str = "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
    const LOGS_BLOOM_01: &str = "0x01010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101";

    fn encode_transactions<E: EthSpec>(
        transactions: Transactions<E>,
    ) -> Result<serde_json::Value, serde_json::Error> {
        let ep: JsonExecutionPayload<E> =
            JsonExecutionPayload::Bellatrix(JsonExecutionPayloadBellatrix {
                transactions,
                ..<_>::default()
            });
        let json = serde_json::to_value(ep)?;
        Ok(json.get("transactions").unwrap().clone())
    }

    fn decode_transactions<E: EthSpec>(
        transactions: serde_json::Value,
    ) -> Result<Transactions<E>, serde_json::Error> {
        let mut json = json!({
            "parentHash": HASH_00,
            "feeRecipient": ADDRESS_01,
            "stateRoot": HASH_01,
            "receiptsRoot": HASH_00,
            "logsBloom": LOGS_BLOOM_01,
            "prevRandao": HASH_01,
            "blockNumber": "0x0",
            "gasLimit": "0x1",
            "gasUsed": "0x2",
            "timestamp": "0x2a",
            "extraData": "0x",
            "baseFeePerGas": "0x1",
            "blockHash": HASH_01,
        });
        // Take advantage of the fact that we own `transactions` and don't need to reserialize it.
        json.as_object_mut()
            .unwrap()
            .insert("transactions".into(), transactions);
        let ep: JsonExecutionPayload<E> = serde_json::from_value(json)?;
        Ok(ep.transactions().clone())
    }

    fn assert_transactions_serde<E: EthSpec>(
        name: &str,
        as_obj: Transactions<E>,
        as_json: serde_json::Value,
    ) {
        assert_eq!(
            encode_transactions::<E>(as_obj.clone()).unwrap(),
            as_json,
            "encoding for {name}"
        );
        assert_eq!(
            decode_transactions::<E>(as_json).unwrap(),
            as_obj,
            "decoding for {name}"
        );
    }

    /// Example: if `spec == &[1, 1]`, then two one-byte transactions will be created.
    fn generate_transactions<E: EthSpec>(spec: &[usize]) -> Transactions<E> {
        let mut txs = VariableList::default();

        for &num_bytes in spec {
            let mut tx = VariableList::default();
            for _ in 0..num_bytes {
                tx.push(0).unwrap();
            }
            txs.push(tx).unwrap();
        }

        txs
    }

    #[test]
    fn transaction_serde() {
        assert_transactions_serde::<MainnetEthSpec>(
            "empty",
            generate_transactions::<MainnetEthSpec>(&[]),
            json!([]),
        );
        assert_transactions_serde::<MainnetEthSpec>(
            "one empty tx",
            generate_transactions::<MainnetEthSpec>(&[0]),
            json!(["0x"]),
        );
        assert_transactions_serde::<MainnetEthSpec>(
            "two empty txs",
            generate_transactions::<MainnetEthSpec>(&[0, 0]),
            json!(["0x", "0x"]),
        );
        assert_transactions_serde::<MainnetEthSpec>(
            "one one-byte tx",
            generate_transactions::<MainnetEthSpec>(&[1]),
            json!(["0x00"]),
        );
        assert_transactions_serde::<MainnetEthSpec>(
            "two one-byte txs",
            generate_transactions::<MainnetEthSpec>(&[1, 1]),
            json!(["0x00", "0x00"]),
        );
        assert_transactions_serde::<MainnetEthSpec>(
            "mixed bag",
            generate_transactions::<MainnetEthSpec>(&[0, 1, 3, 0]),
            json!(["0x", "0x00", "0x000000", "0x"]),
        );

        /*
         * Check for too many transactions
         */

        let num_max_txs = <MainnetEthSpec as EthSpec>::MaxTransactionsPerPayload::to_usize();
        let max_txs = (0..num_max_txs).map(|_| "0x00").collect::<Vec<_>>();
        let too_many_txs = (0..=num_max_txs).map(|_| "0x00").collect::<Vec<_>>();

        decode_transactions::<MainnetEthSpec>(serde_json::to_value(max_txs).unwrap()).unwrap();
        assert!(
            decode_transactions::<MainnetEthSpec>(serde_json::to_value(too_many_txs).unwrap())
                .is_err()
        );
    }

    #[tokio::test]
    async fn get_block_by_number_request() {
        Tester::new(true)
            .assert_request_equals(
                |client| async move {
                    let _ = client
                        .get_block_by_number(BlockByNumberQuery::Tag(LATEST_TAG))
                        .await;
                },
                json!({
                    "id": STATIC_ID,
                    "jsonrpc": JSONRPC_VERSION,
                    "method": ETH_GET_BLOCK_BY_NUMBER,
                    "params": ["latest", false]
                }),
            )
            .await;

        Tester::new(false)
            .assert_auth_failure(|client| async move {
                client
                    .get_block_by_number(BlockByNumberQuery::Tag(LATEST_TAG))
                    .await
            })
            .await;
    }

    #[tokio::test]
    async fn get_block_by_hash_request() {
        Tester::new(true)
            .assert_request_equals(
                |client| async move {
                    let _ = client
                        .get_block_by_hash(ExecutionBlockHash::repeat_byte(1))
                        .await;
                },
                json!({
                    "id": STATIC_ID,
                    "jsonrpc": JSONRPC_VERSION,
                    "method": ETH_GET_BLOCK_BY_HASH,
                    "params": [HASH_01, false]
                }),
            )
            .await;

        Tester::new(false)
            .assert_auth_failure(|client| async move {
                client
                    .get_block_by_hash(ExecutionBlockHash::repeat_byte(1))
                    .await
            })
            .await;
    }

    #[tokio::test]
    async fn forkchoice_updated_v1_with_payload_attributes_request() {
        Tester::new(true)
            .assert_request_equals(
                |client| async move {
                    let _ = client
                        .forkchoice_updated_v1(
                            ForkchoiceState {
                                head_block_hash: ExecutionBlockHash::repeat_byte(1),
                                safe_block_hash: ExecutionBlockHash::repeat_byte(1),
                                finalized_block_hash: ExecutionBlockHash::zero(),
                            },
                            Some(PayloadAttributes::V1(PayloadAttributesV1 {
                                timestamp: 5,
                                prev_randao: Hash256::zero(),
                                suggested_fee_recipient: Address::repeat_byte(0),
                            })),
                        )
                        .await;
                },
                json!({
                    "id": STATIC_ID,
                    "jsonrpc": JSONRPC_VERSION,
                    "method": ENGINE_FORKCHOICE_UPDATED_V1,
                    "params": [{
                        "headBlockHash": HASH_01,
                        "safeBlockHash": HASH_01,
                        "finalizedBlockHash": HASH_00,
                    },
                    {
                        "timestamp":"0x5",
                        "prevRandao": HASH_00,
                        "suggestedFeeRecipient": ADDRESS_00
                    }]
                }),
            )
            .await;

        Tester::new(false)
            .assert_auth_failure(|client| async move {
                client
                    .forkchoice_updated_v1(
                        ForkchoiceState {
                            head_block_hash: ExecutionBlockHash::repeat_byte(1),
                            safe_block_hash: ExecutionBlockHash::repeat_byte(1),
                            finalized_block_hash: ExecutionBlockHash::zero(),
                        },
                        Some(PayloadAttributes::V1(PayloadAttributesV1 {
                            timestamp: 5,
                            prev_randao: Hash256::zero(),
                            suggested_fee_recipient: Address::repeat_byte(0),
                        })),
                    )
                    .await
            })
            .await;
    }

    #[tokio::test]
    async fn get_payload_v1_request() {
        Tester::new(true)
            .assert_request_equals(
                |client| async move {
                    let _ = client.get_payload_v1::<MainnetEthSpec>([42; 8]).await;
                },
                json!({
                    "id": STATIC_ID,
                    "jsonrpc": JSONRPC_VERSION,
                    "method": ENGINE_GET_PAYLOAD_V1,
                    "params": ["0x2a2a2a2a2a2a2a2a"]
                }),
            )
            .await;

        Tester::new(false)
            .assert_auth_failure(|client| async move {
                client.get_payload_v1::<MainnetEthSpec>([42; 8]).await
            })
            .await;
    }

    #[tokio::test]
    async fn new_payload_v1_request() {
        Tester::new(true)
            .assert_request_equals(
                |client| async move {
                    let _ = client
                        .new_payload_v1::<MainnetEthSpec>(ExecutionPayload::Bellatrix(
                            ExecutionPayloadBellatrix {
                                parent_hash: ExecutionBlockHash::repeat_byte(0),
                                fee_recipient: Address::repeat_byte(1),
                                state_root: Hash256::repeat_byte(1),
                                receipts_root: Hash256::repeat_byte(0),
                                logs_bloom: vec![1; 256].try_into().unwrap(),
                                prev_randao: Hash256::repeat_byte(1),
                                block_number: 0,
                                gas_limit: 1,
                                gas_used: 2,
                                timestamp: 42,
                                extra_data: vec![].try_into().unwrap(),
                                base_fee_per_gas: Uint256::from(1),
                                block_hash: ExecutionBlockHash::repeat_byte(1),
                                transactions: vec![].try_into().unwrap(),
                            },
                        ))
                        .await;
                },
                json!({
                    "id": STATIC_ID,
                    "jsonrpc": JSONRPC_VERSION,
                    "method": ENGINE_NEW_PAYLOAD_V1,
                    "params": [{
                        "parentHash": HASH_00,
                        "feeRecipient": ADDRESS_01,
                        "stateRoot": HASH_01,
                        "receiptsRoot": HASH_00,
                        "logsBloom": LOGS_BLOOM_01,
                        "prevRandao": HASH_01,
                        "blockNumber": "0x0",
                        "gasLimit": "0x1",
                        "gasUsed": "0x2",
                        "timestamp": "0x2a",
                        "extraData": "0x",
                        "baseFeePerGas": "0x1",
                        "blockHash": HASH_01,
                        "transactions": [],
                    }]
                }),
            )
            .await;

        Tester::new(false)
            .assert_auth_failure(|client| async move {
                client
                    .new_payload_v1::<MainnetEthSpec>(ExecutionPayload::Bellatrix(
                        ExecutionPayloadBellatrix {
                            parent_hash: ExecutionBlockHash::repeat_byte(0),
                            fee_recipient: Address::repeat_byte(1),
                            state_root: Hash256::repeat_byte(1),
                            receipts_root: Hash256::repeat_byte(0),
                            logs_bloom: vec![1; 256].try_into().unwrap(),
                            prev_randao: Hash256::repeat_byte(1),
                            block_number: 0,
                            gas_limit: 1,
                            gas_used: 2,
                            timestamp: 42,
                            extra_data: vec![].try_into().unwrap(),
                            base_fee_per_gas: Uint256::from(1),
                            block_hash: ExecutionBlockHash::repeat_byte(1),
                            transactions: vec![].try_into().unwrap(),
                        },
                    ))
                    .await
            })
            .await;
    }

    #[tokio::test]
    async fn forkchoice_updated_v1_request() {
        Tester::new(true)
            .assert_request_equals(
                |client| async move {
                    let _ = client
                        .forkchoice_updated_v1(
                            ForkchoiceState {
                                head_block_hash: ExecutionBlockHash::repeat_byte(0),
                                safe_block_hash: ExecutionBlockHash::repeat_byte(0),
                                finalized_block_hash: ExecutionBlockHash::repeat_byte(1),
                            },
                            None,
                        )
                        .await;
                },
                json!({
                    "id": STATIC_ID,
                    "jsonrpc": JSONRPC_VERSION,
                    "method": ENGINE_FORKCHOICE_UPDATED_V1,
                    "params": [{
                        "headBlockHash": HASH_00,
                        "safeBlockHash": HASH_00,
                        "finalizedBlockHash": HASH_01,
                    }, JSON_NULL]
                }),
            )
            .await;

        Tester::new(false)
            .assert_auth_failure(|client| async move {
                client
                    .forkchoice_updated_v1(
                        ForkchoiceState {
                            head_block_hash: ExecutionBlockHash::repeat_byte(0),
                            safe_block_hash: ExecutionBlockHash::repeat_byte(0),
                            finalized_block_hash: ExecutionBlockHash::repeat_byte(1),
                        },
                        None,
                    )
                    .await
            })
            .await;
    }

    fn str_to_payload_id(s: &str) -> PayloadId {
        serde_json::from_str::<TransparentJsonPayloadId>(&format!("\"{s}\""))
            .unwrap()
            .into()
    }

    #[test]
    fn str_payload_id() {
        assert_eq!(
            str_to_payload_id("0x002a2a2a2a2a2a01"),
            [0, 42, 42, 42, 42, 42, 42, 1]
        );
    }

    /// Test vectors provided by Geth:
    ///
    /// <https://notes.ethereum.org/@9AeMAlpyQYaAAyuj47BzRw/rkwW3ceVY>
    ///
    /// The `id` field has been modified on these vectors to match the one we use.
    #[tokio::test]
    async fn geth_test_vectors() {
        Tester::new(true)
            .assert_request_equals(
                // engine_forkchoiceUpdatedV1 (prepare payload) REQUEST validation
                |client| async move {
                    let _ = client
                        .forkchoice_updated_v1(
                            ForkchoiceState {
                                head_block_hash: ExecutionBlockHash::from_str("0x3b8fb240d288781d4aac94d3fd16809ee413bc99294a085798a589dae51ddd4a").unwrap(),
                                safe_block_hash: ExecutionBlockHash::from_str("0x3b8fb240d288781d4aac94d3fd16809ee413bc99294a085798a589dae51ddd4a").unwrap(),
                                finalized_block_hash: ExecutionBlockHash::zero(),
                            },
                            Some(PayloadAttributes::V1(PayloadAttributesV1 {
                                timestamp: 5,
                                prev_randao: Hash256::zero(),
                                suggested_fee_recipient: Address::from_str("0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b").unwrap(),
                            }))
                        )
                        .await;
                },
                json!({
                    "id": STATIC_ID,
                    "jsonrpc": JSONRPC_VERSION,
                    "method": ENGINE_FORKCHOICE_UPDATED_V1,
                    "params": [{
                        "headBlockHash": "0x3b8fb240d288781d4aac94d3fd16809ee413bc99294a085798a589dae51ddd4a",
                        "safeBlockHash": "0x3b8fb240d288781d4aac94d3fd16809ee413bc99294a085798a589dae51ddd4a",
                        "finalizedBlockHash": HASH_00,
                    },
                    {
                        "timestamp":"0x5",
                        "prevRandao": HASH_00,
                        "suggestedFeeRecipient":"0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b"
                    }]
                })
            )
            .await
            .with_preloaded_responses(
                // engine_forkchoiceUpdatedV1 (prepare payload) RESPONSE validation
                vec![json!({
                    "id": STATIC_ID,
                    "jsonrpc": JSONRPC_VERSION,
                    "result": {
                        "payloadStatus": {
                            "status": "VALID",
                            "latestValidHash": HASH_00,
                            "validationError": ""
                        },
                        "payloadId": "0xa247243752eb10b4"
                    }
                })],
                |client| async move {
                    let response = client
                        .forkchoice_updated_v1(
                            ForkchoiceState {
                                head_block_hash: ExecutionBlockHash::from_str("0x3b8fb240d288781d4aac94d3fd16809ee413bc99294a085798a589dae51ddd4a").unwrap(),
                                safe_block_hash: ExecutionBlockHash::from_str("0x3b8fb240d288781d4aac94d3fd16809ee413bc99294a085798a589dae51ddd4a").unwrap(),
                                finalized_block_hash: ExecutionBlockHash::zero(),
                            },
                            Some(PayloadAttributes::V1(PayloadAttributesV1 {
                                timestamp: 5,
                                prev_randao: Hash256::zero(),
                                suggested_fee_recipient: Address::from_str("0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b").unwrap(),
                            }))
                        )
                        .await
                        .unwrap();
                    assert_eq!(response, ForkchoiceUpdatedResponse {
                        payload_status: PayloadStatusV1 {
                            status: PayloadStatusV1Status::Valid,
                            latest_valid_hash: Some(ExecutionBlockHash::zero()),
                            validation_error: Some(String::new()),
                        },
                        payload_id:
                            Some(str_to_payload_id("0xa247243752eb10b4")),
                    });
                },
            )
            .await
            .assert_request_equals(
                // engine_getPayloadV1 REQUEST validation
                |client| async move {
                    let _ = client
                        .get_payload_v1::<MainnetEthSpec>(str_to_payload_id("0xa247243752eb10b4"))
                        .await;
                },
                json!({
                    "id": STATIC_ID,
                    "jsonrpc": JSONRPC_VERSION,
                    "method": ENGINE_GET_PAYLOAD_V1,
                    "params": ["0xa247243752eb10b4"]
                })
            )
            .await
            .with_preloaded_responses(
                // engine_getPayloadV1 RESPONSE validation
                vec![json!({
                    "jsonrpc":JSONRPC_VERSION,
                    "id":STATIC_ID,
                    "result":{
                        "parentHash":"0x3b8fb240d288781d4aac94d3fd16809ee413bc99294a085798a589dae51ddd4a",
                        "feeRecipient":"0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b",
                        "stateRoot":"0xca3149fa9e37db08d1cd49c9061db1002ef1cd58db2210f2115c8c989b2bdf45",
                        "receiptsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
                        "logsBloom": LOGS_BLOOM_00,
                        "prevRandao": HASH_00,
                        "blockNumber":"0x1",
                        "gasLimit":"0x1c95111",
                        "gasUsed":"0x0",
                        "timestamp":"0x5",
                        "extraData":"0x",
                        "baseFeePerGas":"0x7",
                        "blockHash":"0x6359b8381a370e2f54072a5784ddd78b6ed024991558c511d4452eb4f6ac898c",
                        "transactions":[],
                    }
                })],
                |client| async move {
                    let payload: ExecutionPayload<_> = client
                        .get_payload_v1::<MainnetEthSpec>(str_to_payload_id("0xa247243752eb10b4"))
                        .await
                        .unwrap()
                        .into();

                    let expected = ExecutionPayload::Bellatrix(ExecutionPayloadBellatrix {
                            parent_hash: ExecutionBlockHash::from_str("0x3b8fb240d288781d4aac94d3fd16809ee413bc99294a085798a589dae51ddd4a").unwrap(),
                            fee_recipient: Address::from_str("0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b").unwrap(),
                            state_root: Hash256::from_str("0xca3149fa9e37db08d1cd49c9061db1002ef1cd58db2210f2115c8c989b2bdf45").unwrap(),
                            receipts_root: Hash256::from_str("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421").unwrap(),
                            logs_bloom: vec![0; 256].try_into().unwrap(),
                            prev_randao: Hash256::zero(),
                            block_number: 1,
                            gas_limit: u64::from_str_radix("1c95111",16).unwrap(),
                            gas_used: 0,
                            timestamp: 5,
                            extra_data: vec![].try_into().unwrap(),
                            base_fee_per_gas: Uint256::from(7),
                            block_hash: ExecutionBlockHash::from_str("0x6359b8381a370e2f54072a5784ddd78b6ed024991558c511d4452eb4f6ac898c").unwrap(),
                            transactions: vec![].try_into().unwrap(),
                        });

                    assert_eq!(payload, expected);
                },
            )
            .await
            .assert_request_equals(
                // engine_newPayloadV1 REQUEST validation
                |client| async move {
                    let _ = client
                        .new_payload_v1::<MainnetEthSpec>(ExecutionPayload::Bellatrix(ExecutionPayloadBellatrix{
                            parent_hash: ExecutionBlockHash::from_str("0x3b8fb240d288781d4aac94d3fd16809ee413bc99294a085798a589dae51ddd4a").unwrap(),
                            fee_recipient: Address::from_str("0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b").unwrap(),
                            state_root: Hash256::from_str("0xca3149fa9e37db08d1cd49c9061db1002ef1cd58db2210f2115c8c989b2bdf45").unwrap(),
                            receipts_root: Hash256::from_str("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421").unwrap(),
                            logs_bloom: vec![0; 256].try_into().unwrap(),
                            prev_randao: Hash256::zero(),
                            block_number: 1,
                            gas_limit: u64::from_str_radix("1c9c380",16).unwrap(),
                            gas_used: 0,
                            timestamp: 5,
                            extra_data: vec![].try_into().unwrap(),
                            base_fee_per_gas: Uint256::from(7),
                            block_hash: ExecutionBlockHash::from_str("0x3559e851470f6e7bbed1db474980683e8c315bfce99b2a6ef47c057c04de7858").unwrap(),
                            transactions: vec![].try_into().unwrap(),
                        }))
                        .await;
                },
                json!({
                    "id": STATIC_ID,
                    "jsonrpc": JSONRPC_VERSION,
                    "method": ENGINE_NEW_PAYLOAD_V1,
                    "params": [{
                        "parentHash":"0x3b8fb240d288781d4aac94d3fd16809ee413bc99294a085798a589dae51ddd4a",
                        "feeRecipient":"0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b",
                        "stateRoot":"0xca3149fa9e37db08d1cd49c9061db1002ef1cd58db2210f2115c8c989b2bdf45",
                        "receiptsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
                        "logsBloom": LOGS_BLOOM_00,
                        "prevRandao": HASH_00,
                        "blockNumber":"0x1",
                        "gasLimit":"0x1c9c380",
                        "gasUsed":"0x0",
                        "timestamp":"0x5",
                        "extraData":"0x",
                        "baseFeePerGas":"0x7",
                        "blockHash":"0x3559e851470f6e7bbed1db474980683e8c315bfce99b2a6ef47c057c04de7858",
                        "transactions":[],
                    }],
                })
            )
            .await
            .with_preloaded_responses(
                // engine_newPayloadV1 RESPONSE validation
                vec![json!({
                    "jsonrpc": JSONRPC_VERSION,
                    "id": STATIC_ID,
                    "result":{
                        "status":"VALID",
                        "latestValidHash":"0x3559e851470f6e7bbed1db474980683e8c315bfce99b2a6ef47c057c04de7858",
                        "validationError":"",
                    }
                })],
                |client| async move {
                    let response = client
                        .new_payload_v1::<MainnetEthSpec>(ExecutionPayload::Bellatrix(ExecutionPayloadBellatrix::default()))
                        .await
                        .unwrap();

                    assert_eq!(response,
                               PayloadStatusV1 {
                            status: PayloadStatusV1Status::Valid,
                            latest_valid_hash: Some(ExecutionBlockHash::from_str("0x3559e851470f6e7bbed1db474980683e8c315bfce99b2a6ef47c057c04de7858").unwrap()),
                            validation_error: Some(String::new()),
                        }
                    );
                },
            )
            .await
            .assert_request_equals(
                // engine_forkchoiceUpdatedV1 REQUEST validation
                |client| async move {
                    let _ = client
                        .forkchoice_updated_v1(
                            ForkchoiceState {
                                head_block_hash: ExecutionBlockHash::from_str("0x3559e851470f6e7bbed1db474980683e8c315bfce99b2a6ef47c057c04de7858").unwrap(),
                                safe_block_hash: ExecutionBlockHash::from_str("0x3559e851470f6e7bbed1db474980683e8c315bfce99b2a6ef47c057c04de7858").unwrap(),
                                finalized_block_hash: ExecutionBlockHash::from_str("0x3b8fb240d288781d4aac94d3fd16809ee413bc99294a085798a589dae51ddd4a").unwrap(),
                            },
                            None,
                        )
                        .await;
                },
                json!({
                    "jsonrpc": JSONRPC_VERSION,
                    "method": ENGINE_FORKCHOICE_UPDATED_V1,
                    "params": [
                        {
                            "headBlockHash":"0x3559e851470f6e7bbed1db474980683e8c315bfce99b2a6ef47c057c04de7858",
                            "safeBlockHash":"0x3559e851470f6e7bbed1db474980683e8c315bfce99b2a6ef47c057c04de7858",
                            "finalizedBlockHash":"0x3b8fb240d288781d4aac94d3fd16809ee413bc99294a085798a589dae51ddd4a"
                        }, JSON_NULL],
                    "id": STATIC_ID
                })
            )
            .await
            .with_preloaded_responses(
                // engine_forkchoiceUpdatedV1 RESPONSE validation
                vec![json!({
                    "jsonrpc": JSONRPC_VERSION,
                    "id": STATIC_ID,
                    "result": {
                        "payloadStatus": {
                            "status": "VALID",
                            "latestValidHash": HASH_00,
                            "validationError": ""
                        },
                        "payloadId": JSON_NULL,
                    }
                })],
                |client| async move {
                    let response = client
                        .forkchoice_updated_v1(
                            ForkchoiceState {
                                head_block_hash: ExecutionBlockHash::from_str("0x3559e851470f6e7bbed1db474980683e8c315bfce99b2a6ef47c057c04de7858").unwrap(),
                                safe_block_hash: ExecutionBlockHash::from_str("0x3559e851470f6e7bbed1db474980683e8c315bfce99b2a6ef47c057c04de7858").unwrap(),
                                finalized_block_hash: ExecutionBlockHash::from_str("0x3b8fb240d288781d4aac94d3fd16809ee413bc99294a085798a589dae51ddd4a").unwrap(),
                            },
                            None,
                        )
                        .await
                        .unwrap();
                    assert_eq!(response, ForkchoiceUpdatedResponse {
                        payload_status: PayloadStatusV1 {
                            status: PayloadStatusV1Status::Valid,
                            latest_valid_hash: Some(ExecutionBlockHash::zero()),
                            validation_error: Some(String::new()),
                        },
                        payload_id: None,
                    });
                },
            )
            .await;
    }

    #[tokio::test]
    async fn new_payload_v5_gloas_request() {
        Tester::new(true)
            .assert_request_equals(
                |client| async move {
                    let payload = types::ExecutionPayloadGloas::<MainnetEthSpec> {
                        parent_hash: ExecutionBlockHash::repeat_byte(0),
                        fee_recipient: Address::repeat_byte(1),
                        state_root: Hash256::repeat_byte(1),
                        receipts_root: Hash256::repeat_byte(0),
                        logs_bloom: vec![1; 256].try_into().unwrap(),
                        prev_randao: Hash256::repeat_byte(1),
                        block_number: 0,
                        gas_limit: 1,
                        gas_used: 2,
                        timestamp: 42,
                        extra_data: vec![].try_into().unwrap(),
                        base_fee_per_gas: Uint256::from(1),
                        block_hash: ExecutionBlockHash::repeat_byte(1),
                        transactions: vec![].try_into().unwrap(),
                        withdrawals: vec![].try_into().unwrap(),
                        blob_gas_used: 0,
                        excess_blob_gas: 0,
                    };
                    let execution_requests: types::ExecutionRequests<MainnetEthSpec> =
                        types::ExecutionRequests::default();
                    let _ = client
                        .new_payload_v5_gloas(NewPayloadRequestGloas {
                            execution_payload: &payload,
                            versioned_hashes: vec![],
                            parent_beacon_block_root: Hash256::repeat_byte(0),
                            execution_requests: &execution_requests,
                        })
                        .await;
                },
                json!({
                    "id": STATIC_ID,
                    "jsonrpc": JSONRPC_VERSION,
                    "method": ENGINE_NEW_PAYLOAD_V5,
                    "params": [{
                        "parentHash": HASH_00,
                        "feeRecipient": ADDRESS_01,
                        "stateRoot": HASH_01,
                        "receiptsRoot": HASH_00,
                        "logsBloom": LOGS_BLOOM_01,
                        "prevRandao": HASH_01,
                        "blockNumber": "0x0",
                        "gasLimit": "0x1",
                        "gasUsed": "0x2",
                        "timestamp": "0x2a",
                        "extraData": "0x",
                        "baseFeePerGas": "0x1",
                        "blockHash": HASH_01,
                        "transactions": [],
                        "withdrawals": [],
                        "blobGasUsed": "0x0",
                        "excessBlobGas": "0x0",
                    },
                    [],
                    HASH_00,
                    []
                    ]
                }),
            )
            .await;

        Tester::new(false)
            .assert_auth_failure(|client| async move {
                let payload = types::ExecutionPayloadGloas::<MainnetEthSpec>::default();
                let execution_requests: types::ExecutionRequests<MainnetEthSpec> =
                    types::ExecutionRequests::default();
                client
                    .new_payload_v5_gloas(NewPayloadRequestGloas {
                        execution_payload: &payload,
                        versioned_hashes: vec![],
                        parent_beacon_block_root: Hash256::zero(),
                        execution_requests: &execution_requests,
                    })
                    .await
            })
            .await;
    }

    #[tokio::test]
    async fn get_payload_v5_gloas_request() {
        Tester::new(true)
            .assert_request_equals(
                |client| async move {
                    let _ = client
                        .get_payload_v5::<MainnetEthSpec>(ForkName::Gloas, [42; 8])
                        .await;
                },
                json!({
                    "id": STATIC_ID,
                    "jsonrpc": JSONRPC_VERSION,
                    "method": ENGINE_GET_PAYLOAD_V5,
                    "params": ["0x2a2a2a2a2a2a2a2a"]
                }),
            )
            .await;

        Tester::new(false)
            .assert_auth_failure(|client| async move {
                client
                    .get_payload_v5::<MainnetEthSpec>(ForkName::Gloas, [42; 8])
                    .await
            })
            .await;
    }

    #[tokio::test]
    async fn get_payload_v5_gloas_response() {
        Tester::new(true)
            .with_preloaded_responses(
                vec![json!({
                    "jsonrpc": JSONRPC_VERSION,
                    "id": STATIC_ID,
                    "result": {
                        "executionPayload": {
                            "parentHash": HASH_00,
                            "feeRecipient": ADDRESS_01,
                            "stateRoot": HASH_01,
                            "receiptsRoot": HASH_00,
                            "logsBloom": LOGS_BLOOM_01,
                            "prevRandao": HASH_01,
                            "blockNumber": "0x1",
                            "gasLimit": "0x1c95111",
                            "gasUsed": "0x0",
                            "timestamp": "0x5",
                            "extraData": "0x",
                            "baseFeePerGas": "0x7",
                            "blockHash": HASH_01,
                            "transactions": [],
                            "withdrawals": [],
                            "blobGasUsed": "0x0",
                            "excessBlobGas": "0x0"
                        },
                        "blockValue": "0x2a",
                        "blobsBundle": {
                            "commitments": [],
                            "proofs": [],
                            "blobs": []
                        },
                        "shouldOverrideBuilder": false,
                        "executionRequests": []
                    }
                })],
                |client| async move {
                    let response = client
                        .get_payload_v5::<MainnetEthSpec>(ForkName::Gloas, [42; 8])
                        .await
                        .unwrap();

                    let expected_payload = types::ExecutionPayloadGloas::<MainnetEthSpec> {
                        parent_hash: ExecutionBlockHash::repeat_byte(0),
                        fee_recipient: Address::repeat_byte(1),
                        state_root: Hash256::repeat_byte(1),
                        receipts_root: Hash256::repeat_byte(0),
                        logs_bloom: vec![1; 256].try_into().unwrap(),
                        prev_randao: Hash256::repeat_byte(1),
                        block_number: 1,
                        gas_limit: u64::from_str_radix("1c95111", 16).unwrap(),
                        gas_used: 0,
                        timestamp: 5,
                        extra_data: vec![].try_into().unwrap(),
                        base_fee_per_gas: Uint256::from(7),
                        block_hash: ExecutionBlockHash::repeat_byte(1),
                        transactions: vec![].try_into().unwrap(),
                        withdrawals: vec![].try_into().unwrap(),
                        blob_gas_used: 0,
                        excess_blob_gas: 0,
                    };

                    let payload: ExecutionPayload<MainnetEthSpec> = response.clone().into();
                    assert_eq!(payload, ExecutionPayload::Gloas(expected_payload),);

                    assert_eq!(*response.block_value(), Uint256::from(42));
                    assert!(!response.should_override_builder().unwrap());
                },
            )
            .await;
    }
}
