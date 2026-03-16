use crate::engines::ForkchoiceState;
use crate::http::{
    ENGINE_FORKCHOICE_UPDATED_V1, ENGINE_FORKCHOICE_UPDATED_V2, ENGINE_FORKCHOICE_UPDATED_V3,
    ENGINE_GET_BLOBS_V1, ENGINE_GET_BLOBS_V2, ENGINE_GET_CLIENT_VERSION_V1,
    ENGINE_GET_PAYLOAD_BODIES_BY_HASH_V1, ENGINE_GET_PAYLOAD_BODIES_BY_RANGE_V1,
    ENGINE_GET_PAYLOAD_V1, ENGINE_GET_PAYLOAD_V2, ENGINE_GET_PAYLOAD_V3, ENGINE_GET_PAYLOAD_V4,
    ENGINE_GET_PAYLOAD_V5, ENGINE_NEW_PAYLOAD_V1, ENGINE_NEW_PAYLOAD_V2, ENGINE_NEW_PAYLOAD_V3,
    ENGINE_NEW_PAYLOAD_V4, ENGINE_NEW_PAYLOAD_V5,
};
use eth2::types::{
    BlobsBundle, SsePayloadAttributes, SsePayloadAttributesV1, SsePayloadAttributesV2,
    SsePayloadAttributesV3,
};
pub use json_structures::{JsonWithdrawal, TransitionConfigurationV1};
use pretty_reqwest_error::PrettyReqwestError;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;
use superstruct::superstruct;
pub use types::{
    Address, BeaconBlockRef, ConsolidationRequest, EthSpec, ExecutionBlockHash, ExecutionPayload,
    ExecutionPayloadHeader, ExecutionPayloadRef, FixedVector, ForkName, Hash256, Transactions,
    Uint256, VariableList, Withdrawal, Withdrawals,
};
use types::{
    ExecutionPayloadBellatrix, ExecutionPayloadCapella, ExecutionPayloadDeneb,
    ExecutionPayloadElectra, ExecutionPayloadFulu, ExecutionPayloadGloas, ExecutionRequests,
    KzgProofs,
};
use types::{GRAFFITI_BYTES_LEN, Graffiti};

pub mod auth;
pub mod http;
pub mod json_structures;
mod new_payload_request;

pub use new_payload_request::{
    NewPayloadRequest, NewPayloadRequestBellatrix, NewPayloadRequestCapella,
    NewPayloadRequestDeneb, NewPayloadRequestElectra, NewPayloadRequestFulu,
    NewPayloadRequestGloas,
};

pub const LATEST_TAG: &str = "latest";

pub type PayloadId = [u8; 8];

#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
pub enum Error {
    HttpClient(PrettyReqwestError),
    Auth(auth::Error),
    BadResponse(String),
    Json(serde_json::Error),
    ServerMessage { code: i64, message: String },
    Eip155Failure,
    IsSyncing,
    ExecutionBlockNotFound(ExecutionBlockHash),
    ExecutionHeadBlockNotFound,
    PayloadIdUnavailable,
    SszError(ssz_types::Error),
    BuilderApi(builder_client::Error),
    IncorrectStateVariant,
    RequiredMethodUnsupported(&'static str),
    UnsupportedForkVariant(String),
    InvalidClientVersion(String),
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        if matches!(
            e.status(),
            Some(StatusCode::UNAUTHORIZED) | Some(StatusCode::FORBIDDEN)
        ) {
            Error::Auth(auth::Error::InvalidToken)
        } else {
            Error::HttpClient(e.into())
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}

impl From<auth::Error> for Error {
    fn from(e: auth::Error) -> Self {
        Error::Auth(e)
    }
}

impl From<builder_client::Error> for Error {
    fn from(e: builder_client::Error) -> Self {
        Error::BuilderApi(e)
    }
}

impl From<ssz_types::Error> for Error {
    fn from(e: ssz_types::Error) -> Self {
        Error::SszError(e)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum PayloadStatusV1Status {
    Valid,
    Invalid,
    Syncing,
    Accepted,
    InvalidBlockHash,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PayloadStatusV1 {
    pub status: PayloadStatusV1Status,
    pub latest_valid_hash: Option<ExecutionBlockHash>,
    pub validation_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum BlockByNumberQuery<'a> {
    Tag(&'a str),
}

/// Representation of an exection block with enough detail to determine the terminal PoW block.
///
/// See `get_pow_block_hash_at_total_difficulty`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionBlock {
    #[serde(rename = "hash")]
    pub block_hash: ExecutionBlockHash,
    #[serde(rename = "number", with = "serde_utils::u64_hex_be")]
    pub block_number: u64,

    pub parent_hash: ExecutionBlockHash,
    pub total_difficulty: Option<Uint256>,
    #[serde(with = "serde_utils::u64_hex_be")]
    pub timestamp: u64,
}

impl ExecutionBlock {
    pub fn terminal_total_difficulty_reached(&self, terminal_total_difficulty: Uint256) -> bool {
        self.total_difficulty
            .is_none_or(|td| td >= terminal_total_difficulty)
    }
}

#[superstruct(
    variants(V1, V2, V3),
    variant_attributes(derive(Clone, Debug, Eq, Hash, PartialEq),),
    cast_error(ty = "Error", expr = "Error::IncorrectStateVariant"),
    partial_getter_error(ty = "Error", expr = "Error::IncorrectStateVariant")
)]
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PayloadAttributes {
    #[superstruct(getter(copy))]
    pub timestamp: u64,
    #[superstruct(getter(copy))]
    pub prev_randao: Hash256,
    #[superstruct(getter(copy))]
    pub suggested_fee_recipient: Address,
    #[superstruct(only(V2, V3))]
    pub withdrawals: Vec<Withdrawal>,
    #[superstruct(only(V3), partial_getter(copy))]
    pub parent_beacon_block_root: Hash256,
}

impl PayloadAttributes {
    pub fn new(
        timestamp: u64,
        prev_randao: Hash256,
        suggested_fee_recipient: Address,
        withdrawals: Option<Vec<Withdrawal>>,
        parent_beacon_block_root: Option<Hash256>,
    ) -> Self {
        match withdrawals {
            Some(withdrawals) => match parent_beacon_block_root {
                Some(parent_beacon_block_root) => PayloadAttributes::V3(PayloadAttributesV3 {
                    timestamp,
                    prev_randao,
                    suggested_fee_recipient,
                    withdrawals,
                    parent_beacon_block_root,
                }),
                None => PayloadAttributes::V2(PayloadAttributesV2 {
                    timestamp,
                    prev_randao,
                    suggested_fee_recipient,
                    withdrawals,
                }),
            },
            None => PayloadAttributes::V1(PayloadAttributesV1 {
                timestamp,
                prev_randao,
                suggested_fee_recipient,
            }),
        }
    }
}

impl From<PayloadAttributes> for SsePayloadAttributes {
    fn from(pa: PayloadAttributes) -> Self {
        match pa {
            PayloadAttributes::V1(PayloadAttributesV1 {
                timestamp,
                prev_randao,
                suggested_fee_recipient,
            }) => Self::V1(SsePayloadAttributesV1 {
                timestamp,
                prev_randao,
                suggested_fee_recipient,
            }),
            PayloadAttributes::V2(PayloadAttributesV2 {
                timestamp,
                prev_randao,
                suggested_fee_recipient,
                withdrawals,
            }) => Self::V2(SsePayloadAttributesV2 {
                timestamp,
                prev_randao,
                suggested_fee_recipient,
                withdrawals,
            }),
            PayloadAttributes::V3(PayloadAttributesV3 {
                timestamp,
                prev_randao,
                suggested_fee_recipient,
                withdrawals,
                parent_beacon_block_root,
            }) => Self::V3(SsePayloadAttributesV3 {
                timestamp,
                prev_randao,
                suggested_fee_recipient,
                withdrawals,
                parent_beacon_block_root,
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ForkchoiceUpdatedResponse {
    pub payload_status: PayloadStatusV1,
    pub payload_id: Option<PayloadId>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProposeBlindedBlockResponseStatus {
    Valid,
    Invalid,
    Syncing,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProposeBlindedBlockResponse {
    pub status: ProposeBlindedBlockResponseStatus,
    pub latest_valid_hash: Option<Hash256>,
    pub validation_error: Option<String>,
}

#[superstruct(
    variants(Bellatrix, Capella, Deneb, Electra, Fulu, Gloas),
    variant_attributes(derive(Clone, Debug, PartialEq),),
    map_into(ExecutionPayload),
    map_ref_into(ExecutionPayloadRef),
    cast_error(ty = "Error", expr = "Error::IncorrectStateVariant"),
    partial_getter_error(ty = "Error", expr = "Error::IncorrectStateVariant")
)]
#[derive(Clone, Debug, PartialEq)]
pub struct GetPayloadResponse<E: EthSpec> {
    #[superstruct(
        only(Bellatrix),
        partial_getter(rename = "execution_payload_bellatrix")
    )]
    pub execution_payload: ExecutionPayloadBellatrix<E>,
    #[superstruct(only(Capella), partial_getter(rename = "execution_payload_capella"))]
    pub execution_payload: ExecutionPayloadCapella<E>,
    #[superstruct(only(Deneb), partial_getter(rename = "execution_payload_deneb"))]
    pub execution_payload: ExecutionPayloadDeneb<E>,
    #[superstruct(only(Electra), partial_getter(rename = "execution_payload_electra"))]
    pub execution_payload: ExecutionPayloadElectra<E>,
    #[superstruct(only(Fulu), partial_getter(rename = "execution_payload_fulu"))]
    pub execution_payload: ExecutionPayloadFulu<E>,
    #[superstruct(only(Gloas), partial_getter(rename = "execution_payload_gloas"))]
    pub execution_payload: ExecutionPayloadGloas<E>,
    pub block_value: Uint256,
    #[superstruct(only(Deneb, Electra, Fulu, Gloas))]
    pub blobs_bundle: BlobsBundle<E>,
    #[superstruct(only(Deneb, Electra, Fulu, Gloas), partial_getter(copy))]
    pub should_override_builder: bool,
    #[superstruct(only(Electra, Fulu, Gloas))]
    pub requests: ExecutionRequests<E>,
}

impl<E: EthSpec> GetPayloadResponse<E> {
    pub fn fee_recipient(&self) -> Address {
        ExecutionPayloadRef::from(self.to_ref()).fee_recipient()
    }

    pub fn block_hash(&self) -> ExecutionBlockHash {
        ExecutionPayloadRef::from(self.to_ref()).block_hash()
    }

    pub fn block_number(&self) -> u64 {
        ExecutionPayloadRef::from(self.to_ref()).block_number()
    }
}

impl<'a, E: EthSpec> From<GetPayloadResponseRef<'a, E>> for ExecutionPayloadRef<'a, E> {
    fn from(response: GetPayloadResponseRef<'a, E>) -> Self {
        map_get_payload_response_ref_into_execution_payload_ref!(&'a _, response, |inner, cons| {
            cons(&inner.execution_payload)
        })
    }
}

impl<E: EthSpec> From<GetPayloadResponse<E>> for ExecutionPayload<E> {
    fn from(response: GetPayloadResponse<E>) -> Self {
        map_get_payload_response_into_execution_payload!(response, |inner, cons| {
            cons(inner.execution_payload)
        })
    }
}

impl<E: EthSpec> From<GetPayloadResponse<E>>
    for (
        ExecutionPayload<E>,
        Uint256,
        Option<BlobsBundle<E>>,
        Option<ExecutionRequests<E>>,
    )
{
    fn from(response: GetPayloadResponse<E>) -> Self {
        match response {
            GetPayloadResponse::Bellatrix(inner) => (
                ExecutionPayload::Bellatrix(inner.execution_payload),
                inner.block_value,
                None,
                None,
            ),
            GetPayloadResponse::Capella(inner) => (
                ExecutionPayload::Capella(inner.execution_payload),
                inner.block_value,
                None,
                None,
            ),
            GetPayloadResponse::Deneb(inner) => (
                ExecutionPayload::Deneb(inner.execution_payload),
                inner.block_value,
                Some(inner.blobs_bundle),
                None,
            ),
            GetPayloadResponse::Electra(inner) => (
                ExecutionPayload::Electra(inner.execution_payload),
                inner.block_value,
                Some(inner.blobs_bundle),
                Some(inner.requests),
            ),
            GetPayloadResponse::Fulu(inner) => (
                ExecutionPayload::Fulu(inner.execution_payload),
                inner.block_value,
                Some(inner.blobs_bundle),
                Some(inner.requests),
            ),
            GetPayloadResponse::Gloas(inner) => (
                ExecutionPayload::Gloas(inner.execution_payload),
                inner.block_value,
                Some(inner.blobs_bundle),
                Some(inner.requests),
            ),
        }
    }
}

pub enum GetPayloadResponseType<E: EthSpec> {
    Full(GetPayloadResponse<E>),
    Blinded(GetPayloadResponse<E>),
}

impl<E: EthSpec> GetPayloadResponse<E> {
    pub fn execution_payload_ref(&self) -> ExecutionPayloadRef<'_, E> {
        self.to_ref().into()
    }
}

#[derive(Clone, Debug)]
pub struct ExecutionPayloadBodyV1<E: EthSpec> {
    pub transactions: Transactions<E>,
    pub withdrawals: Option<Withdrawals<E>>,
}

impl<E: EthSpec> ExecutionPayloadBodyV1<E> {
    pub fn to_payload(
        self,
        header: ExecutionPayloadHeader<E>,
    ) -> Result<ExecutionPayload<E>, String> {
        match header {
            ExecutionPayloadHeader::Bellatrix(header) => {
                if self.withdrawals.is_some() {
                    return Err(format!(
                        "block {} is bellatrix but payload body has withdrawals",
                        header.block_hash
                    ));
                }
                Ok(ExecutionPayload::Bellatrix(ExecutionPayloadBellatrix {
                    parent_hash: header.parent_hash,
                    fee_recipient: header.fee_recipient,
                    state_root: header.state_root,
                    receipts_root: header.receipts_root,
                    logs_bloom: header.logs_bloom,
                    prev_randao: header.prev_randao,
                    block_number: header.block_number,
                    gas_limit: header.gas_limit,
                    gas_used: header.gas_used,
                    timestamp: header.timestamp,
                    extra_data: header.extra_data,
                    base_fee_per_gas: header.base_fee_per_gas,
                    block_hash: header.block_hash,
                    transactions: self.transactions,
                }))
            }
            ExecutionPayloadHeader::Capella(header) => {
                if let Some(withdrawals) = self.withdrawals {
                    Ok(ExecutionPayload::Capella(ExecutionPayloadCapella {
                        parent_hash: header.parent_hash,
                        fee_recipient: header.fee_recipient,
                        state_root: header.state_root,
                        receipts_root: header.receipts_root,
                        logs_bloom: header.logs_bloom,
                        prev_randao: header.prev_randao,
                        block_number: header.block_number,
                        gas_limit: header.gas_limit,
                        gas_used: header.gas_used,
                        timestamp: header.timestamp,
                        extra_data: header.extra_data,
                        base_fee_per_gas: header.base_fee_per_gas,
                        block_hash: header.block_hash,
                        transactions: self.transactions,
                        withdrawals,
                    }))
                } else {
                    Err(format!(
                        "block {} is capella but payload body doesn't have withdrawals",
                        header.block_hash
                    ))
                }
            }
            ExecutionPayloadHeader::Deneb(header) => {
                if let Some(withdrawals) = self.withdrawals {
                    Ok(ExecutionPayload::Deneb(ExecutionPayloadDeneb {
                        parent_hash: header.parent_hash,
                        fee_recipient: header.fee_recipient,
                        state_root: header.state_root,
                        receipts_root: header.receipts_root,
                        logs_bloom: header.logs_bloom,
                        prev_randao: header.prev_randao,
                        block_number: header.block_number,
                        gas_limit: header.gas_limit,
                        gas_used: header.gas_used,
                        timestamp: header.timestamp,
                        extra_data: header.extra_data,
                        base_fee_per_gas: header.base_fee_per_gas,
                        block_hash: header.block_hash,
                        transactions: self.transactions,
                        withdrawals,
                        blob_gas_used: header.blob_gas_used,
                        excess_blob_gas: header.excess_blob_gas,
                    }))
                } else {
                    Err(format!(
                        "block {} is post capella but payload body doesn't have withdrawals",
                        header.block_hash
                    ))
                }
            }
            ExecutionPayloadHeader::Electra(header) => {
                if let Some(withdrawals) = self.withdrawals {
                    Ok(ExecutionPayload::Electra(ExecutionPayloadElectra {
                        parent_hash: header.parent_hash,
                        fee_recipient: header.fee_recipient,
                        state_root: header.state_root,
                        receipts_root: header.receipts_root,
                        logs_bloom: header.logs_bloom,
                        prev_randao: header.prev_randao,
                        block_number: header.block_number,
                        gas_limit: header.gas_limit,
                        gas_used: header.gas_used,
                        timestamp: header.timestamp,
                        extra_data: header.extra_data,
                        base_fee_per_gas: header.base_fee_per_gas,
                        block_hash: header.block_hash,
                        transactions: self.transactions,
                        withdrawals,
                        blob_gas_used: header.blob_gas_used,
                        excess_blob_gas: header.excess_blob_gas,
                    }))
                } else {
                    Err(format!(
                        "block {} is post capella but payload body doesn't have withdrawals",
                        header.block_hash
                    ))
                }
            }
            ExecutionPayloadHeader::Fulu(header) => {
                if let Some(withdrawals) = self.withdrawals {
                    Ok(ExecutionPayload::Fulu(ExecutionPayloadFulu {
                        parent_hash: header.parent_hash,
                        fee_recipient: header.fee_recipient,
                        state_root: header.state_root,
                        receipts_root: header.receipts_root,
                        logs_bloom: header.logs_bloom,
                        prev_randao: header.prev_randao,
                        block_number: header.block_number,
                        gas_limit: header.gas_limit,
                        gas_used: header.gas_used,
                        timestamp: header.timestamp,
                        extra_data: header.extra_data,
                        base_fee_per_gas: header.base_fee_per_gas,
                        block_hash: header.block_hash,
                        transactions: self.transactions,
                        withdrawals,
                        blob_gas_used: header.blob_gas_used,
                        excess_blob_gas: header.excess_blob_gas,
                    }))
                } else {
                    Err(format!(
                        "block {} is post capella but payload body doesn't have withdrawals",
                        header.block_hash
                    ))
                }
            }
            ExecutionPayloadHeader::Gloas(header) => {
                if let Some(withdrawals) = self.withdrawals {
                    Ok(ExecutionPayload::Gloas(ExecutionPayloadGloas {
                        parent_hash: header.parent_hash,
                        fee_recipient: header.fee_recipient,
                        state_root: header.state_root,
                        receipts_root: header.receipts_root,
                        logs_bloom: header.logs_bloom,
                        prev_randao: header.prev_randao,
                        block_number: header.block_number,
                        gas_limit: header.gas_limit,
                        gas_used: header.gas_used,
                        timestamp: header.timestamp,
                        extra_data: header.extra_data,
                        base_fee_per_gas: header.base_fee_per_gas,
                        block_hash: header.block_hash,
                        transactions: self.transactions,
                        withdrawals,
                        blob_gas_used: header.blob_gas_used,
                        excess_blob_gas: header.excess_blob_gas,
                    }))
                } else {
                    Err(format!(
                        "block {} is post capella but payload body doesn't have withdrawals",
                        header.block_hash
                    ))
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EngineCapabilities {
    pub new_payload_v1: bool,
    pub new_payload_v2: bool,
    pub new_payload_v3: bool,
    pub new_payload_v4: bool,
    pub new_payload_v5: bool,
    pub forkchoice_updated_v1: bool,
    pub forkchoice_updated_v2: bool,
    pub forkchoice_updated_v3: bool,
    pub get_payload_bodies_by_hash_v1: bool,
    pub get_payload_bodies_by_range_v1: bool,
    pub get_payload_v1: bool,
    pub get_payload_v2: bool,
    pub get_payload_v3: bool,
    pub get_payload_v4: bool,
    pub get_payload_v5: bool,
    pub get_client_version_v1: bool,
    pub get_blobs_v1: bool,
    pub get_blobs_v2: bool,
}

impl EngineCapabilities {
    pub fn to_response(&self) -> Vec<&str> {
        let mut response = Vec::new();
        if self.new_payload_v1 {
            response.push(ENGINE_NEW_PAYLOAD_V1);
        }
        if self.new_payload_v2 {
            response.push(ENGINE_NEW_PAYLOAD_V2);
        }
        if self.new_payload_v3 {
            response.push(ENGINE_NEW_PAYLOAD_V3);
        }
        if self.new_payload_v4 {
            response.push(ENGINE_NEW_PAYLOAD_V4);
        }
        if self.new_payload_v5 {
            response.push(ENGINE_NEW_PAYLOAD_V5);
        }
        if self.forkchoice_updated_v1 {
            response.push(ENGINE_FORKCHOICE_UPDATED_V1);
        }
        if self.forkchoice_updated_v2 {
            response.push(ENGINE_FORKCHOICE_UPDATED_V2);
        }
        if self.forkchoice_updated_v3 {
            response.push(ENGINE_FORKCHOICE_UPDATED_V3);
        }
        if self.get_payload_bodies_by_hash_v1 {
            response.push(ENGINE_GET_PAYLOAD_BODIES_BY_HASH_V1);
        }
        if self.get_payload_bodies_by_range_v1 {
            response.push(ENGINE_GET_PAYLOAD_BODIES_BY_RANGE_V1);
        }
        if self.get_payload_v1 {
            response.push(ENGINE_GET_PAYLOAD_V1);
        }
        if self.get_payload_v2 {
            response.push(ENGINE_GET_PAYLOAD_V2);
        }
        if self.get_payload_v3 {
            response.push(ENGINE_GET_PAYLOAD_V3);
        }
        if self.get_payload_v4 {
            response.push(ENGINE_GET_PAYLOAD_V4);
        }
        if self.get_payload_v5 {
            response.push(ENGINE_GET_PAYLOAD_V5);
        }
        if self.get_client_version_v1 {
            response.push(ENGINE_GET_CLIENT_VERSION_V1);
        }
        if self.get_blobs_v1 {
            response.push(ENGINE_GET_BLOBS_V1);
        }
        if self.get_blobs_v2 {
            response.push(ENGINE_GET_BLOBS_V2);
        }

        response
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClientCode {
    Besu,
    EtherumJS,
    Erigon,
    GoEthereum,
    Grandine,
    Lighthouse,
    Vibehouse,
    Lodestar,
    Nethermind,
    Nimbus,
    TrinExecution,
    Teku,
    Prysm,
    Reth,
    Unknown(String),
}

impl std::fmt::Display for ClientCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ClientCode::Besu => "BU",
            ClientCode::EtherumJS => "EJ",
            ClientCode::Erigon => "EG",
            ClientCode::GoEthereum => "GE",
            ClientCode::Grandine => "GR",
            ClientCode::Lighthouse => "LH",
            ClientCode::Vibehouse => "VH",
            ClientCode::Lodestar => "LS",
            ClientCode::Nethermind => "NM",
            ClientCode::Nimbus => "NB",
            ClientCode::TrinExecution => "TE",
            ClientCode::Teku => "TK",
            ClientCode::Prysm => "PM",
            ClientCode::Reth => "RH",
            ClientCode::Unknown(code) => code,
        };
        write!(f, "{}", s)
    }
}

impl TryFrom<String> for ClientCode {
    type Error = String;

    fn try_from(code: String) -> Result<Self, Self::Error> {
        match code.as_str() {
            "BU" => Ok(Self::Besu),
            "EJ" => Ok(Self::EtherumJS),
            "EG" => Ok(Self::Erigon),
            "GE" => Ok(Self::GoEthereum),
            "GR" => Ok(Self::Grandine),
            "LH" => Ok(Self::Lighthouse),
            "VH" => Ok(Self::Vibehouse),
            "LS" => Ok(Self::Lodestar),
            "NM" => Ok(Self::Nethermind),
            "NB" => Ok(Self::Nimbus),
            "TE" => Ok(Self::TrinExecution),
            "TK" => Ok(Self::Teku),
            "PM" => Ok(Self::Prysm),
            "RH" => Ok(Self::Reth),
            string => {
                if string.len() == 2 {
                    Ok(Self::Unknown(code))
                } else {
                    Err(format!("Invalid client code: {}", code))
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct CommitPrefix(pub String);

impl TryFrom<String> for CommitPrefix {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        // Check if the input starts with '0x' and strip it if it does
        let commit_prefix = value.strip_prefix("0x").unwrap_or(&value);

        // Ensure length is exactly 8 characters after '0x' removal
        if commit_prefix.len() != 8 {
            return Err(
                "Input must be exactly 8 characters long (excluding any '0x' prefix)".to_string(),
            );
        }

        // Ensure all characters are valid hex digits
        if commit_prefix.chars().all(|c| c.is_ascii_hexdigit()) {
            Ok(CommitPrefix(commit_prefix.to_lowercase()))
        } else {
            Err("Input must contain only hexadecimal characters".to_string())
        }
    }
}

impl std::fmt::Display for CommitPrefix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug)]
pub struct ClientVersionV1 {
    pub code: ClientCode,
    pub name: String,
    pub version: String,
    pub commit: CommitPrefix,
}

impl ClientVersionV1 {
    pub fn calculate_graffiti(&self, vibehouse_commit_prefix: CommitPrefix) -> Graffiti {
        let graffiti_string = format!(
            "{}{}LH{}",
            self.code,
            self.commit
                .0
                .get(..4)
                .unwrap_or(self.commit.0.as_str())
                .to_lowercase(),
            vibehouse_commit_prefix
                .0
                .get(..4)
                .unwrap_or("0000")
                .to_lowercase(),
        );
        let mut graffiti_bytes = [0u8; GRAFFITI_BYTES_LEN];
        let bytes_to_copy = std::cmp::min(graffiti_string.len(), GRAFFITI_BYTES_LEN);
        graffiti_bytes[..bytes_to_copy]
            .copy_from_slice(&graffiti_string.as_bytes()[..bytes_to_copy]);

        Graffiti::from(graffiti_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{FixedBytesExtended, MinimalEthSpec};

    type E = MinimalEthSpec;

    // --- PayloadStatusV1Status ---

    #[test]
    fn payload_status_v1_status_into_static_str() {
        let s: &str = PayloadStatusV1Status::Valid.into();
        assert_eq!(s, "valid");
        let s: &str = PayloadStatusV1Status::Invalid.into();
        assert_eq!(s, "invalid");
        let s: &str = PayloadStatusV1Status::Syncing.into();
        assert_eq!(s, "syncing");
        let s: &str = PayloadStatusV1Status::Accepted.into();
        assert_eq!(s, "accepted");
        let s: &str = PayloadStatusV1Status::InvalidBlockHash.into();
        assert_eq!(s, "invalid_block_hash");
    }

    #[test]
    fn payload_status_v1_status_clone_copy_eq() {
        let s = PayloadStatusV1Status::Valid;
        let s2 = s;
        assert_eq!(s, s2);
    }

    // --- ExecutionBlock ---

    #[test]
    fn execution_block_td_reached_above() {
        let block = ExecutionBlock {
            block_hash: ExecutionBlockHash::zero(),
            block_number: 1,
            parent_hash: ExecutionBlockHash::zero(),
            total_difficulty: Some(Uint256::from(100u64)),
            timestamp: 0,
        };
        assert!(block.terminal_total_difficulty_reached(Uint256::from(50u64)));
    }

    #[test]
    fn execution_block_td_reached_equal() {
        let block = ExecutionBlock {
            block_hash: ExecutionBlockHash::zero(),
            block_number: 1,
            parent_hash: ExecutionBlockHash::zero(),
            total_difficulty: Some(Uint256::from(100u64)),
            timestamp: 0,
        };
        assert!(block.terminal_total_difficulty_reached(Uint256::from(100u64)));
    }

    #[test]
    fn execution_block_td_not_reached() {
        let block = ExecutionBlock {
            block_hash: ExecutionBlockHash::zero(),
            block_number: 1,
            parent_hash: ExecutionBlockHash::zero(),
            total_difficulty: Some(Uint256::from(50u64)),
            timestamp: 0,
        };
        assert!(!block.terminal_total_difficulty_reached(Uint256::from(100u64)));
    }

    #[test]
    fn execution_block_td_none_is_reached() {
        let block = ExecutionBlock {
            block_hash: ExecutionBlockHash::zero(),
            block_number: 1,
            parent_hash: ExecutionBlockHash::zero(),
            total_difficulty: None,
            timestamp: 0,
        };
        // None means post-merge, always reached
        assert!(block.terminal_total_difficulty_reached(Uint256::from(100u64)));
    }

    // --- PayloadAttributes ---

    #[test]
    fn payload_attributes_new_v1() {
        let pa = PayloadAttributes::new(100, Hash256::zero(), Address::ZERO, None, None);
        assert!(matches!(pa, PayloadAttributes::V1(_)));
        assert_eq!(pa.timestamp(), 100);
        assert_eq!(pa.prev_randao(), Hash256::zero());
        assert_eq!(pa.suggested_fee_recipient(), Address::ZERO);
    }

    #[test]
    fn payload_attributes_new_v2() {
        let pa = PayloadAttributes::new(200, Hash256::zero(), Address::ZERO, Some(vec![]), None);
        assert!(matches!(pa, PayloadAttributes::V2(_)));
        assert_eq!(pa.timestamp(), 200);
    }

    #[test]
    fn payload_attributes_new_v3() {
        let pa = PayloadAttributes::new(
            300,
            Hash256::zero(),
            Address::ZERO,
            Some(vec![]),
            Some(Hash256::repeat_byte(0xaa)),
        );
        assert!(matches!(pa, PayloadAttributes::V3(_)));
        assert_eq!(pa.timestamp(), 300);
        assert_eq!(
            pa.parent_beacon_block_root().unwrap(),
            Hash256::repeat_byte(0xaa)
        );
    }

    #[test]
    fn payload_attributes_v1_parent_beacon_block_root_err() {
        let pa = PayloadAttributes::new(100, Hash256::zero(), Address::zero(), None, None);
        assert!(pa.parent_beacon_block_root().is_err());
    }

    // --- PayloadAttributes -> SsePayloadAttributes ---

    #[test]
    fn payload_attributes_to_sse_v1() {
        let pa = PayloadAttributes::new(100, Hash256::zero(), Address::zero(), None, None);
        let sse: SsePayloadAttributes = pa.into();
        assert!(matches!(sse, SsePayloadAttributes::V1(_)));
    }

    #[test]
    fn payload_attributes_to_sse_v2() {
        let pa = PayloadAttributes::new(200, Hash256::zero(), Address::ZERO, Some(vec![]), None);
        let sse: SsePayloadAttributes = pa.into();
        assert!(matches!(sse, SsePayloadAttributes::V2(_)));
    }

    #[test]
    fn payload_attributes_to_sse_v3() {
        let pa = PayloadAttributes::new(
            300,
            Hash256::zero(),
            Address::ZERO,
            Some(vec![]),
            Some(Hash256::zero()),
        );
        let sse: SsePayloadAttributes = pa.into();
        assert!(matches!(sse, SsePayloadAttributes::V3(_)));
    }

    // --- ExecutionPayloadBodyV1::to_payload ---

    fn make_body(with_withdrawals: bool) -> ExecutionPayloadBodyV1<E> {
        ExecutionPayloadBodyV1 {
            transactions: <_>::default(),
            withdrawals: if with_withdrawals {
                Some(<_>::default())
            } else {
                None
            },
        }
    }

    #[test]
    fn body_to_payload_bellatrix_ok() {
        let body = make_body(false);
        let header = ExecutionPayloadHeader::Bellatrix(<_>::default());
        assert!(body.to_payload(header).is_ok());
    }

    #[test]
    fn body_to_payload_bellatrix_with_withdrawals_err() {
        let body = make_body(true);
        let header = ExecutionPayloadHeader::Bellatrix(<_>::default());
        let result = body.to_payload(header);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("bellatrix"));
    }

    #[test]
    fn body_to_payload_capella_ok() {
        let body = make_body(true);
        let header = ExecutionPayloadHeader::Capella(<_>::default());
        assert!(body.to_payload(header).is_ok());
    }

    #[test]
    fn body_to_payload_capella_no_withdrawals_err() {
        let body = make_body(false);
        let header = ExecutionPayloadHeader::Capella(<_>::default());
        assert!(body.to_payload(header).is_err());
    }

    #[test]
    fn body_to_payload_deneb_ok() {
        let body = make_body(true);
        let header = ExecutionPayloadHeader::Deneb(<_>::default());
        assert!(body.to_payload(header).is_ok());
    }

    #[test]
    fn body_to_payload_deneb_no_withdrawals_err() {
        let body = make_body(false);
        let header = ExecutionPayloadHeader::Deneb(<_>::default());
        assert!(body.to_payload(header).is_err());
    }

    #[test]
    fn body_to_payload_electra_ok() {
        let body = make_body(true);
        let header = ExecutionPayloadHeader::Electra(<_>::default());
        assert!(body.to_payload(header).is_ok());
    }

    #[test]
    fn body_to_payload_fulu_ok() {
        let body = make_body(true);
        let header = ExecutionPayloadHeader::Fulu(<_>::default());
        assert!(body.to_payload(header).is_ok());
    }

    #[test]
    fn body_to_payload_gloas_ok() {
        let body = make_body(true);
        let header = ExecutionPayloadHeader::Gloas(<_>::default());
        assert!(body.to_payload(header).is_ok());
    }

    #[test]
    fn body_to_payload_gloas_no_withdrawals_err() {
        let body = make_body(false);
        let header = ExecutionPayloadHeader::Gloas(<_>::default());
        assert!(body.to_payload(header).is_err());
    }

    #[test]
    fn body_to_payload_preserves_header_fields() {
        let body = make_body(false);
        let header = types::ExecutionPayloadHeaderBellatrix::<E> {
            block_number: 42,
            gas_limit: 1000,
            timestamp: 9999,
            ..Default::default()
        };
        let payload = body
            .to_payload(ExecutionPayloadHeader::Bellatrix(header))
            .unwrap();
        assert_eq!(payload.block_number(), 42);
        assert_eq!(payload.gas_limit(), 1000);
        assert_eq!(payload.timestamp(), 9999);
    }

    // --- EngineCapabilities ---

    #[test]
    fn engine_capabilities_all_false_empty_response() {
        let caps = EngineCapabilities {
            new_payload_v1: false,
            new_payload_v2: false,
            new_payload_v3: false,
            new_payload_v4: false,
            new_payload_v5: false,
            forkchoice_updated_v1: false,
            forkchoice_updated_v2: false,
            forkchoice_updated_v3: false,
            get_payload_bodies_by_hash_v1: false,
            get_payload_bodies_by_range_v1: false,
            get_payload_v1: false,
            get_payload_v2: false,
            get_payload_v3: false,
            get_payload_v4: false,
            get_payload_v5: false,
            get_client_version_v1: false,
            get_blobs_v1: false,
            get_blobs_v2: false,
        };
        assert!(caps.to_response().is_empty());
    }

    #[test]
    fn engine_capabilities_all_true_full_response() {
        let caps = EngineCapabilities {
            new_payload_v1: true,
            new_payload_v2: true,
            new_payload_v3: true,
            new_payload_v4: true,
            new_payload_v5: true,
            forkchoice_updated_v1: true,
            forkchoice_updated_v2: true,
            forkchoice_updated_v3: true,
            get_payload_bodies_by_hash_v1: true,
            get_payload_bodies_by_range_v1: true,
            get_payload_v1: true,
            get_payload_v2: true,
            get_payload_v3: true,
            get_payload_v4: true,
            get_payload_v5: true,
            get_client_version_v1: true,
            get_blobs_v1: true,
            get_blobs_v2: true,
        };
        let response = caps.to_response();
        assert_eq!(response.len(), 18);
        assert!(response.contains(&"engine_newPayloadV1"));
        assert!(response.contains(&"engine_newPayloadV5"));
        assert!(response.contains(&"engine_forkchoiceUpdatedV3"));
        assert!(response.contains(&"engine_getPayloadV5"));
        assert!(response.contains(&"engine_getClientVersionV1"));
        assert!(response.contains(&"engine_getBlobsV2"));
    }

    #[test]
    fn engine_capabilities_partial() {
        let caps = EngineCapabilities {
            new_payload_v1: false,
            new_payload_v2: false,
            new_payload_v3: true,
            new_payload_v4: false,
            new_payload_v5: false,
            forkchoice_updated_v1: false,
            forkchoice_updated_v2: false,
            forkchoice_updated_v3: true,
            get_payload_bodies_by_hash_v1: false,
            get_payload_bodies_by_range_v1: false,
            get_payload_v1: false,
            get_payload_v2: false,
            get_payload_v3: true,
            get_payload_v4: false,
            get_payload_v5: false,
            get_client_version_v1: false,
            get_blobs_v1: false,
            get_blobs_v2: false,
        };
        let response = caps.to_response();
        assert_eq!(response.len(), 3);
        assert!(response.contains(&"engine_newPayloadV3"));
        assert!(response.contains(&"engine_forkchoiceUpdatedV3"));
        assert!(response.contains(&"engine_getPayloadV3"));
    }

    // --- ClientCode ---

    #[test]
    fn client_code_try_from_all_known() {
        let cases = vec![
            ("BU", ClientCode::Besu),
            ("EJ", ClientCode::EtherumJS),
            ("EG", ClientCode::Erigon),
            ("GE", ClientCode::GoEthereum),
            ("GR", ClientCode::Grandine),
            ("LH", ClientCode::Lighthouse),
            ("VH", ClientCode::Vibehouse),
            ("LS", ClientCode::Lodestar),
            ("NM", ClientCode::Nethermind),
            ("NB", ClientCode::Nimbus),
            ("TE", ClientCode::TrinExecution),
            ("TK", ClientCode::Teku),
            ("PM", ClientCode::Prysm),
            ("RH", ClientCode::Reth),
        ];
        for (code, expected) in cases {
            let result = ClientCode::try_from(code.to_string()).unwrap();
            assert_eq!(result, expected, "Failed for code {}", code);
        }
    }

    #[test]
    fn client_code_unknown_two_char() {
        let result = ClientCode::try_from("ZZ".to_string()).unwrap();
        assert_eq!(result, ClientCode::Unknown("ZZ".to_string()));
    }

    #[test]
    fn client_code_invalid_length() {
        assert!(ClientCode::try_from("ABC".to_string()).is_err());
        assert!(ClientCode::try_from("A".to_string()).is_err());
        assert!(ClientCode::try_from("".to_string()).is_err());
    }

    #[test]
    fn client_code_display_roundtrip() {
        let cases = vec![
            (ClientCode::Besu, "BU"),
            (ClientCode::EtherumJS, "EJ"),
            (ClientCode::Erigon, "EG"),
            (ClientCode::GoEthereum, "GE"),
            (ClientCode::Grandine, "GR"),
            (ClientCode::Lighthouse, "LH"),
            (ClientCode::Vibehouse, "VH"),
            (ClientCode::Lodestar, "LS"),
            (ClientCode::Nethermind, "NM"),
            (ClientCode::Nimbus, "NB"),
            (ClientCode::TrinExecution, "TE"),
            (ClientCode::Teku, "TK"),
            (ClientCode::Prysm, "PM"),
            (ClientCode::Reth, "RH"),
            (ClientCode::Unknown("XY".to_string()), "XY"),
        ];
        for (code, expected) in cases {
            assert_eq!(format!("{}", code), expected);
        }
    }

    // --- CommitPrefix ---

    #[test]
    fn commit_prefix_valid_hex() {
        let cp = CommitPrefix::try_from("abcdef01".to_string()).unwrap();
        assert_eq!(cp.0, "abcdef01");
    }

    #[test]
    fn commit_prefix_with_0x_prefix() {
        let cp = CommitPrefix::try_from("0xABCDEF01".to_string()).unwrap();
        assert_eq!(cp.0, "abcdef01"); // lowercased
    }

    #[test]
    fn commit_prefix_wrong_length() {
        assert!(CommitPrefix::try_from("abc".to_string()).is_err());
        assert!(CommitPrefix::try_from("abcdef0123".to_string()).is_err());
    }

    #[test]
    fn commit_prefix_non_hex_chars() {
        assert!(CommitPrefix::try_from("abcdefgh".to_string()).is_err());
    }

    #[test]
    fn commit_prefix_display() {
        let cp = CommitPrefix::try_from("0xDEADBEEF".to_string()).unwrap();
        assert_eq!(format!("{}", cp), "deadbeef");
    }

    #[test]
    fn commit_prefix_0x_with_short_remaining() {
        // "0x" + 6 chars = 8 total, but after stripping 0x only 6 chars
        assert!(CommitPrefix::try_from("0xabcdef".to_string()).is_err());
    }

    // --- ClientVersionV1 ---

    #[test]
    fn client_version_calculate_graffiti() {
        let cv = ClientVersionV1 {
            code: ClientCode::Reth,
            name: "reth".to_string(),
            version: "1.0.0".to_string(),
            commit: CommitPrefix::try_from("aabbccdd".to_string()).unwrap(),
        };
        let vh_commit = CommitPrefix::try_from("11223344".to_string()).unwrap();
        let graffiti = cv.calculate_graffiti(vh_commit);
        let text = graffiti.as_utf8_lossy();
        // Expected: "RHaabbLH1122"
        assert!(text.starts_with("RHaabb"), "Got: {}", text);
        assert!(text.contains("LH1122"), "Got: {}", text);
    }

    #[test]
    fn client_version_graffiti_uses_first_4_chars_of_commit() {
        let cv = ClientVersionV1 {
            code: ClientCode::GoEthereum,
            name: "geth".to_string(),
            version: "1.0.0".to_string(),
            commit: CommitPrefix::try_from("deadbeef".to_string()).unwrap(),
        };
        let vh_commit = CommitPrefix::try_from("cafebabe".to_string()).unwrap();
        let graffiti = cv.calculate_graffiti(vh_commit);
        let text = graffiti.as_utf8_lossy();
        // "GEdeadLHcafe"
        assert!(text.starts_with("GEdead"), "Got: {}", text);
        assert!(text.contains("LHcafe"), "Got: {}", text);
    }

    // --- GetPayloadResponse conversions ---

    type PayloadTuple = (
        ExecutionPayload<E>,
        Uint256,
        Option<BlobsBundle<E>>,
        Option<ExecutionRequests<E>>,
    );

    #[test]
    fn get_payload_response_bellatrix_into_tuple() {
        let response = GetPayloadResponse::<E>::Bellatrix(GetPayloadResponseBellatrix {
            execution_payload: <_>::default(),
            block_value: Uint256::from(42u64),
        });
        let (payload, value, blobs, requests): PayloadTuple = response.into();
        assert!(matches!(payload, ExecutionPayload::Bellatrix(_)));
        assert_eq!(value, Uint256::from(42u64));
        assert!(blobs.is_none());
        assert!(requests.is_none());
    }

    #[test]
    fn get_payload_response_deneb_into_tuple() {
        let response = GetPayloadResponse::<E>::Deneb(GetPayloadResponseDeneb {
            execution_payload: <_>::default(),
            block_value: Uint256::from(99u64),
            blobs_bundle: <_>::default(),
            should_override_builder: true,
        });
        let (payload, value, blobs, requests): PayloadTuple = response.into();
        assert!(matches!(payload, ExecutionPayload::Deneb(_)));
        assert_eq!(value, Uint256::from(99u64));
        assert!(blobs.is_some());
        assert!(requests.is_none());
    }

    #[test]
    fn get_payload_response_gloas_into_tuple() {
        let response = GetPayloadResponse::<E>::Gloas(GetPayloadResponseGloas {
            execution_payload: <_>::default(),
            block_value: Uint256::from(7u64),
            blobs_bundle: <_>::default(),
            should_override_builder: false,
            requests: <_>::default(),
        });
        let (payload, value, blobs, requests): PayloadTuple = response.into();
        assert!(matches!(payload, ExecutionPayload::Gloas(_)));
        assert_eq!(value, Uint256::from(7u64));
        assert!(blobs.is_some());
        assert!(requests.is_some());
    }

    #[test]
    fn get_payload_response_accessors() {
        let response = GetPayloadResponse::<E>::Bellatrix(GetPayloadResponseBellatrix {
            execution_payload: <_>::default(),
            block_value: Uint256::from(0u64),
        });
        assert_eq!(response.block_number(), 0);
        assert_eq!(response.fee_recipient(), Address::ZERO);
        assert_eq!(response.block_hash(), ExecutionBlockHash::zero());
    }

    #[test]
    fn get_payload_response_into_execution_payload() {
        let response = GetPayloadResponse::<E>::Capella(GetPayloadResponseCapella {
            execution_payload: <_>::default(),
            block_value: Uint256::from(1u64),
        });
        let payload: ExecutionPayload<E> = response.into();
        assert!(matches!(payload, ExecutionPayload::Capella(_)));
    }

    // --- ForkchoiceUpdatedResponse ---

    #[test]
    fn forkchoice_updated_response_with_payload_id() {
        let response = ForkchoiceUpdatedResponse {
            payload_status: PayloadStatusV1 {
                status: PayloadStatusV1Status::Valid,
                latest_valid_hash: Some(ExecutionBlockHash::zero()),
                validation_error: None,
            },
            payload_id: Some([1u8; 8]),
        };
        assert_eq!(response.payload_status.status, PayloadStatusV1Status::Valid);
        assert_eq!(response.payload_id, Some([1u8; 8]));
    }

    #[test]
    fn forkchoice_updated_response_no_payload_id() {
        let response = ForkchoiceUpdatedResponse {
            payload_status: PayloadStatusV1 {
                status: PayloadStatusV1Status::Syncing,
                latest_valid_hash: None,
                validation_error: None,
            },
            payload_id: None,
        };
        assert_eq!(
            response.payload_status.status,
            PayloadStatusV1Status::Syncing
        );
        assert!(response.payload_id.is_none());
    }

    // --- Error From conversions ---

    #[test]
    fn error_from_serde_json() {
        let json_err: serde_json::Error =
            serde_json::from_str::<String>("invalid json").unwrap_err();
        let err = Error::from(json_err);
        assert!(matches!(err, Error::Json(_)));
    }

    #[test]
    fn error_from_ssz_types() {
        let ssz_err = ssz_types::Error::OutOfBounds { i: 10, len: 5 };
        let err = Error::from(ssz_err);
        assert!(matches!(err, Error::SszError(_)));
    }

    #[test]
    fn error_from_auth() {
        let auth_err = auth::Error::InvalidToken;
        let err = Error::from(auth_err);
        assert!(matches!(err, Error::Auth(_)));
    }

    // --- ProposeBlindedBlockResponseStatus ---

    #[test]
    fn propose_blinded_block_response_status_variants() {
        let valid = ProposeBlindedBlockResponseStatus::Valid;
        let invalid = ProposeBlindedBlockResponseStatus::Invalid;
        let syncing = ProposeBlindedBlockResponseStatus::Syncing;
        assert_ne!(valid, invalid);
        assert_ne!(valid, syncing);
        assert_ne!(invalid, syncing);
    }

    // --- PayloadStatusV1 ---

    #[test]
    fn payload_status_v1_clone_eq() {
        let status = PayloadStatusV1 {
            status: PayloadStatusV1Status::Invalid,
            latest_valid_hash: Some(ExecutionBlockHash::repeat_byte(0xff)),
            validation_error: Some("bad block".to_string()),
        };
        let cloned = status.clone();
        assert_eq!(status, cloned);
    }

    // --- ExecutionBlock serde ---

    #[test]
    fn execution_block_serde_roundtrip() {
        let block = ExecutionBlock {
            block_hash: ExecutionBlockHash::repeat_byte(0x11),
            block_number: 123456,
            parent_hash: ExecutionBlockHash::repeat_byte(0x22),
            total_difficulty: Some(Uint256::from(999u64)),
            timestamp: 1700000000,
        };
        let json = serde_json::to_string(&block).unwrap();
        let decoded: ExecutionBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, decoded);
    }

    #[test]
    fn execution_block_serde_no_total_difficulty() {
        let block = ExecutionBlock {
            block_hash: ExecutionBlockHash::zero(),
            block_number: 0,
            parent_hash: ExecutionBlockHash::zero(),
            total_difficulty: None,
            timestamp: 0,
        };
        let json = serde_json::to_string(&block).unwrap();
        let decoded: ExecutionBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block.total_difficulty, decoded.total_difficulty);
    }

    // --- GetPayloadResponseType ---

    #[test]
    fn get_payload_response_type_full_and_blinded() {
        let full = GetPayloadResponseType::Full(GetPayloadResponse::<E>::Bellatrix(
            GetPayloadResponseBellatrix {
                execution_payload: <_>::default(),
                block_value: Uint256::from(0u64),
            },
        ));
        assert!(matches!(full, GetPayloadResponseType::Full(_)));

        let blinded = GetPayloadResponseType::Blinded(GetPayloadResponse::<E>::Bellatrix(
            GetPayloadResponseBellatrix {
                execution_payload: <_>::default(),
                block_value: Uint256::from(0u64),
            },
        ));
        assert!(matches!(blinded, GetPayloadResponseType::Blinded(_)));
    }

    // --- ExecutionPayloadBodyV1 ---

    #[test]
    fn execution_payload_body_clone() {
        let body = make_body(true);
        let cloned = body.clone();
        assert_eq!(cloned.withdrawals.is_some(), body.withdrawals.is_some());
    }

    #[test]
    fn body_to_payload_electra_no_withdrawals_err() {
        let body = make_body(false);
        let header = ExecutionPayloadHeader::Electra(<_>::default());
        assert!(body.to_payload(header).is_err());
    }

    #[test]
    fn body_to_payload_fulu_no_withdrawals_err() {
        let body = make_body(false);
        let header = ExecutionPayloadHeader::Fulu(<_>::default());
        assert!(body.to_payload(header).is_err());
    }
}
