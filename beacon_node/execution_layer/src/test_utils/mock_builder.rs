use crate::test_utils::{DEFAULT_BUILDER_PAYLOAD_VALUE_WEI, DEFAULT_JWT_SECRET};
use crate::{Config, ExecutionLayer, PayloadAttributes, PayloadParameters};
use axum::Router;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use eth2::types::PublishBlockRequest;
use eth2::types::{
    BlobsBundle, BlockId, BroadcastValidation, EndpointVersion, EventKind, EventTopic,
    FullPayloadContents, ProposerData, StateId, ValidatorId,
};
use eth2::{
    BeaconNodeHttpClient, CONSENSUS_VERSION_HEADER, CONTENT_TYPE_HEADER, SSZ_CONTENT_TYPE_HEADER,
    Timeouts,
};
use fork_choice::ForkchoiceUpdateParameters;
use parking_lot::RwLock;
use sensitive_url::SensitiveUrl;
use ssz::Encode;
use std::collections::HashMap;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;
use task_executor::TaskExecutor;
use tempfile::NamedTempFile;
use tokio_stream::StreamExt;
use tracing::{debug, error, info, warn};
use tree_hash::TreeHash;
use types::builder_bid::{
    BuilderBid, BuilderBidBellatrix, BuilderBidCapella, BuilderBidDeneb, BuilderBidElectra,
    BuilderBidFulu, BuilderBidGloas, SignedBuilderBid,
};
use types::{
    Address, BeaconState, ChainSpec, Epoch, EthSpec, ExecPayload, ExecutionPayload,
    ExecutionPayloadHeaderRefMut, ExecutionRequests, ForkName, ForkVersionDecode,
    ForkVersionedResponse, Hash256, PublicKeyBytes, Signature, SignedBlindedBeaconBlock,
    SignedRoot, SignedValidatorRegistrationData, Slot, Uint256,
};
use types::{ExecutionBlockHash, SecretKey};

pub const DEFAULT_FEE_RECIPIENT: Address = Address::repeat_byte(42);
pub const DEFAULT_GAS_LIMIT: u64 = 60_000_000;
pub const DEFAULT_BUILDER_PRIVATE_KEY: &str =
    "607a11b45a7219cc61a3d9c5fd08c7eebd602a6a19a977f8d3771d5711a550f2";

#[derive(Clone)]
pub enum Operation {
    FeeRecipient(Address),
    GasLimit(usize),
    Value(Uint256),
    ParentHash(Hash256),
    PrevRandao(Hash256),
    BlockNumber(usize),
    Timestamp(usize),
    WithdrawalsRoot(Hash256),
}

impl Operation {
    fn apply<E: EthSpec, B: BidStuff<E>>(self, bid: &mut B) {
        match self {
            Operation::FeeRecipient(fee_recipient) => bid.set_fee_recipient(fee_recipient),
            Operation::GasLimit(gas_limit) => bid.set_gas_limit(gas_limit as u64),
            Operation::Value(value) => bid.set_value(value),
            Operation::ParentHash(parent_hash) => bid.set_parent_hash(parent_hash),
            Operation::PrevRandao(prev_randao) => bid.set_prev_randao(prev_randao),
            Operation::BlockNumber(block_number) => bid.set_block_number(block_number as u64),
            Operation::Timestamp(timestamp) => bid.set_timestamp(timestamp as u64),
            Operation::WithdrawalsRoot(root) => bid.set_withdrawals_root(root),
        }
    }
}

pub fn mock_builder_extra_data<E: EthSpec>() -> types::VariableList<u8, E::MaxExtraDataBytes> {
    "mock_builder".as_bytes().to_vec().into()
}

/// API error type for mock builder responses.
struct ApiError(axum::http::StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.0, self.1).into_response()
    }
}

impl From<String> for ApiError {
    fn from(s: String) -> Self {
        ApiError(axum::http::StatusCode::BAD_REQUEST, s)
    }
}

// contains functions we need for BuilderBids.. not sure what to call this
pub trait BidStuff<E: EthSpec> {
    fn set_fee_recipient(&mut self, fee_recipient_address: Address);
    fn set_gas_limit(&mut self, gas_limit: u64);
    fn set_value(&mut self, value: Uint256);
    fn set_parent_hash(&mut self, parent_hash: Hash256);
    fn set_prev_randao(&mut self, randao: Hash256);
    fn set_block_number(&mut self, block_number: u64);
    fn set_timestamp(&mut self, timestamp: u64);
    fn set_withdrawals_root(&mut self, withdrawals_root: Hash256);

    fn sign_builder_message(&mut self, sk: &SecretKey, spec: &ChainSpec) -> Signature;

    fn stamp_payload(&mut self);
}

impl<E: EthSpec> BidStuff<E> for BuilderBid<E> {
    fn set_fee_recipient(&mut self, fee_recipient: Address) {
        match self.to_mut().header_mut() {
            ExecutionPayloadHeaderRefMut::Bellatrix(header) => {
                header.fee_recipient = fee_recipient;
            }
            ExecutionPayloadHeaderRefMut::Capella(header) => {
                header.fee_recipient = fee_recipient;
            }
            ExecutionPayloadHeaderRefMut::Deneb(header) => {
                header.fee_recipient = fee_recipient;
            }
            ExecutionPayloadHeaderRefMut::Electra(header) => {
                header.fee_recipient = fee_recipient;
            }
            ExecutionPayloadHeaderRefMut::Fulu(header) => {
                header.fee_recipient = fee_recipient;
            }
            ExecutionPayloadHeaderRefMut::Gloas(header) => {
                header.fee_recipient = fee_recipient;
            }
        }
    }

    fn set_gas_limit(&mut self, gas_limit: u64) {
        match self.to_mut().header_mut() {
            ExecutionPayloadHeaderRefMut::Bellatrix(header) => {
                header.gas_limit = gas_limit;
            }
            ExecutionPayloadHeaderRefMut::Capella(header) => {
                header.gas_limit = gas_limit;
            }
            ExecutionPayloadHeaderRefMut::Deneb(header) => {
                header.gas_limit = gas_limit;
            }
            ExecutionPayloadHeaderRefMut::Electra(header) => {
                header.gas_limit = gas_limit;
            }
            ExecutionPayloadHeaderRefMut::Fulu(header) => {
                header.gas_limit = gas_limit;
            }
            ExecutionPayloadHeaderRefMut::Gloas(header) => {
                header.gas_limit = gas_limit;
            }
        }
    }

    fn set_value(&mut self, value: Uint256) {
        *self.value_mut() = value;
    }

    fn set_parent_hash(&mut self, parent_hash: Hash256) {
        match self.to_mut().header_mut() {
            ExecutionPayloadHeaderRefMut::Bellatrix(header) => {
                header.parent_hash = ExecutionBlockHash::from_root(parent_hash);
            }
            ExecutionPayloadHeaderRefMut::Capella(header) => {
                header.parent_hash = ExecutionBlockHash::from_root(parent_hash);
            }
            ExecutionPayloadHeaderRefMut::Deneb(header) => {
                header.parent_hash = ExecutionBlockHash::from_root(parent_hash);
            }
            ExecutionPayloadHeaderRefMut::Electra(header) => {
                header.parent_hash = ExecutionBlockHash::from_root(parent_hash);
            }
            ExecutionPayloadHeaderRefMut::Fulu(header) => {
                header.parent_hash = ExecutionBlockHash::from_root(parent_hash);
            }
            ExecutionPayloadHeaderRefMut::Gloas(header) => {
                header.parent_hash = ExecutionBlockHash::from_root(parent_hash);
            }
        }
    }

    fn set_prev_randao(&mut self, prev_randao: Hash256) {
        match self.to_mut().header_mut() {
            ExecutionPayloadHeaderRefMut::Bellatrix(header) => {
                header.prev_randao = prev_randao;
            }
            ExecutionPayloadHeaderRefMut::Capella(header) => {
                header.prev_randao = prev_randao;
            }
            ExecutionPayloadHeaderRefMut::Deneb(header) => {
                header.prev_randao = prev_randao;
            }
            ExecutionPayloadHeaderRefMut::Electra(header) => {
                header.prev_randao = prev_randao;
            }
            ExecutionPayloadHeaderRefMut::Fulu(header) => {
                header.prev_randao = prev_randao;
            }
            ExecutionPayloadHeaderRefMut::Gloas(header) => {
                header.prev_randao = prev_randao;
            }
        }
    }

    fn set_block_number(&mut self, block_number: u64) {
        match self.to_mut().header_mut() {
            ExecutionPayloadHeaderRefMut::Bellatrix(header) => {
                header.block_number = block_number;
            }
            ExecutionPayloadHeaderRefMut::Capella(header) => {
                header.block_number = block_number;
            }
            ExecutionPayloadHeaderRefMut::Deneb(header) => {
                header.block_number = block_number;
            }
            ExecutionPayloadHeaderRefMut::Electra(header) => {
                header.block_number = block_number;
            }
            ExecutionPayloadHeaderRefMut::Fulu(header) => {
                header.block_number = block_number;
            }
            ExecutionPayloadHeaderRefMut::Gloas(header) => {
                header.block_number = block_number;
            }
        }
    }

    fn set_timestamp(&mut self, timestamp: u64) {
        match self.to_mut().header_mut() {
            ExecutionPayloadHeaderRefMut::Bellatrix(header) => {
                header.timestamp = timestamp;
            }
            ExecutionPayloadHeaderRefMut::Capella(header) => {
                header.timestamp = timestamp;
            }
            ExecutionPayloadHeaderRefMut::Deneb(header) => {
                header.timestamp = timestamp;
            }
            ExecutionPayloadHeaderRefMut::Electra(header) => {
                header.timestamp = timestamp;
            }
            ExecutionPayloadHeaderRefMut::Fulu(header) => {
                header.timestamp = timestamp;
            }
            ExecutionPayloadHeaderRefMut::Gloas(header) => {
                header.timestamp = timestamp;
            }
        }
    }

    fn set_withdrawals_root(&mut self, withdrawals_root: Hash256) {
        match self.to_mut().header_mut() {
            ExecutionPayloadHeaderRefMut::Bellatrix(_) => {
                panic!("no withdrawals before capella")
            }
            ExecutionPayloadHeaderRefMut::Capella(header) => {
                header.withdrawals_root = withdrawals_root;
            }
            ExecutionPayloadHeaderRefMut::Deneb(header) => {
                header.withdrawals_root = withdrawals_root;
            }
            ExecutionPayloadHeaderRefMut::Electra(header) => {
                header.withdrawals_root = withdrawals_root;
            }
            ExecutionPayloadHeaderRefMut::Fulu(header) => {
                header.withdrawals_root = withdrawals_root;
            }
            ExecutionPayloadHeaderRefMut::Gloas(header) => {
                header.withdrawals_root = withdrawals_root;
            }
        }
    }

    fn sign_builder_message(&mut self, sk: &SecretKey, spec: &ChainSpec) -> Signature {
        let domain = spec.get_builder_domain();
        let message = self.signing_root(domain);
        sk.sign(message)
    }

    // this helps differentiate a builder block from a regular block
    fn stamp_payload(&mut self) {
        let extra_data = mock_builder_extra_data::<E>();
        match self.to_mut().header_mut() {
            ExecutionPayloadHeaderRefMut::Bellatrix(header) => {
                header.extra_data = extra_data;
                header.block_hash = ExecutionBlockHash::from_root(header.tree_hash_root());
            }
            ExecutionPayloadHeaderRefMut::Capella(header) => {
                header.extra_data = extra_data;
                header.block_hash = ExecutionBlockHash::from_root(header.tree_hash_root());
            }
            ExecutionPayloadHeaderRefMut::Deneb(header) => {
                header.extra_data = extra_data;
                header.block_hash = ExecutionBlockHash::from_root(header.tree_hash_root());
            }
            ExecutionPayloadHeaderRefMut::Electra(header) => {
                header.extra_data = extra_data;
                header.block_hash = ExecutionBlockHash::from_root(header.tree_hash_root());
            }
            ExecutionPayloadHeaderRefMut::Fulu(header) => {
                header.extra_data = extra_data;
                header.block_hash = ExecutionBlockHash::from_root(header.tree_hash_root());
            }
            ExecutionPayloadHeaderRefMut::Gloas(header) => {
                header.extra_data = extra_data;
                header.block_hash = ExecutionBlockHash::from_root(header.tree_hash_root());
            }
        }
    }
}

// Non referenced version of `PayloadParameters`
#[derive(Clone)]
pub struct PayloadParametersCloned {
    pub parent_hash: ExecutionBlockHash,
    pub parent_gas_limit: u64,
    pub proposer_gas_limit: Option<u64>,
    pub payload_attributes: PayloadAttributes,
    pub forkchoice_update_params: ForkchoiceUpdateParameters,
    pub current_fork: ForkName,
}

#[derive(Clone)]
pub struct MockBuilder<E: EthSpec> {
    el: ExecutionLayer<E>,
    beacon_client: BeaconNodeHttpClient,
    spec: Arc<ChainSpec>,
    val_registration_cache: Arc<RwLock<HashMap<PublicKeyBytes, SignedValidatorRegistrationData>>>,
    builder_sk: SecretKey,
    operations: Arc<RwLock<Vec<Operation>>>,
    invalidate_signatures: Arc<RwLock<bool>>,
    genesis_time: Option<u64>,
    /// Only returns bids for registered validators if set to true. `true` by default.
    validate_pubkey: bool,
    /// Do not apply any operations if set to `false`.
    /// Applying operations might modify the cached header in the execution layer.
    /// Use this if you want get_header to return a valid bid that can be eventually submitted as
    /// a valid block.
    apply_operations: bool,
    payload_id_cache: Arc<RwLock<HashMap<ExecutionBlockHash, PayloadParametersCloned>>>,
    /// If set to `true`, sets the bid returned by `get_header` to Uint256::MAX
    max_bid: bool,
    /// Broadcast the full block with payload to the attached beacon node (simulating the relay).
    ///
    /// Turning this off is useful for testing.
    broadcast_to_bn: bool,
    /// A cache that stores the proposers index for a given epoch
    proposers_cache: Arc<RwLock<HashMap<Epoch, Vec<ProposerData>>>>,
}

impl<E: EthSpec> MockBuilder<E> {
    pub fn new_for_testing(
        mock_el_url: SensitiveUrl,
        beacon_url: SensitiveUrl,
        validate_pubkey: bool,
        apply_operations: bool,
        broadcast_to_bn: bool,
        spec: Arc<ChainSpec>,
        executor: TaskExecutor,
    ) -> (Self, (SocketAddr, impl Future<Output = ()>)) {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().into();
        std::fs::write(&path, hex::encode(DEFAULT_JWT_SECRET)).unwrap();

        // This EL should not talk to a builder
        let config = Config {
            execution_endpoint: Some(mock_el_url),
            secret_file: Some(path),
            suggested_fee_recipient: None,
            ..Default::default()
        };

        let el = ExecutionLayer::from_config(config, executor.clone()).unwrap();

        let max_bid = false;

        let builder = MockBuilder::new(
            el,
            BeaconNodeHttpClient::new(beacon_url, Timeouts::set_all(Duration::from_secs(1))),
            validate_pubkey,
            apply_operations,
            broadcast_to_bn,
            max_bid,
            spec,
            None,
        );
        let host: Ipv4Addr = Ipv4Addr::LOCALHOST;
        let port = 0;
        let server = serve(host, port, builder.clone()).expect("mock builder server should start");
        (builder, server)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        el: ExecutionLayer<E>,
        beacon_client: BeaconNodeHttpClient,
        validate_pubkey: bool,
        apply_operations: bool,
        broadcast_to_bn: bool,
        max_bid: bool,
        spec: Arc<ChainSpec>,
        sk: Option<&[u8]>,
    ) -> Self {
        let builder_sk = if let Some(sk_bytes) = sk {
            match SecretKey::deserialize(sk_bytes) {
                Ok(sk) => sk,
                Err(_) => {
                    error!("Invalid sk_bytes provided, generating random secret key");
                    SecretKey::random()
                }
            }
        } else {
            SecretKey::deserialize(&hex::decode(DEFAULT_BUILDER_PRIVATE_KEY).unwrap()).unwrap()
        };
        Self {
            el,
            beacon_client,
            // Should keep spec and context consistent somehow
            spec,
            val_registration_cache: Arc::new(RwLock::new(HashMap::new())),
            builder_sk,
            validate_pubkey,
            operations: Arc::new(RwLock::new(vec![])),
            invalidate_signatures: Arc::new(RwLock::new(false)),
            payload_id_cache: Arc::new(RwLock::new(HashMap::new())),
            proposers_cache: Arc::new(RwLock::new(HashMap::new())),
            apply_operations,
            max_bid,
            broadcast_to_bn,
            genesis_time: None,
        }
    }

    pub fn add_operation(&self, op: Operation) {
        // Insert operations at the front of the vec to make sure `apply_operations` applies them
        // in the order they are added.
        self.operations.write().insert(0, op);
    }

    pub fn invalid_signatures(&self) {
        *self.invalidate_signatures.write() = true;
    }

    pub fn valid_signatures(&mut self) {
        *self.invalidate_signatures.write() = false;
    }

    fn apply_operations<B: BidStuff<E>>(&self, bid: &mut B) {
        let mut guard = self.operations.write();
        while let Some(op) = guard.pop() {
            op.apply(bid);
        }
        bid.stamp_payload();
    }

    /// Return the public key of the builder
    pub fn public_key(&self) -> PublicKeyBytes {
        self.builder_sk.public_key().compress()
    }

    pub async fn register_validators(
        &self,
        registrations: Vec<SignedValidatorRegistrationData>,
    ) -> Result<(), String> {
        info!(count = registrations.len(), "Registering validators");
        for registration in registrations {
            if !registration.verify_signature(&self.spec) {
                error!(
                    error = "invalid signature",
                    validator = %registration.message.pubkey,
                    "Failed to register validator"
                );
                return Err("invalid signature".to_string());
            }
            self.val_registration_cache
                .write()
                .insert(registration.message.pubkey, registration);
        }
        Ok(())
    }

    pub async fn submit_blinded_block(
        &self,
        block: SignedBlindedBeaconBlock<E>,
    ) -> Result<FullPayloadContents<E>, String> {
        let root = match &block {
            SignedBlindedBeaconBlock::Base(_) | types::SignedBeaconBlock::Altair(_) => {
                return Err("invalid fork".to_string());
            }
            SignedBlindedBeaconBlock::Bellatrix(block) => {
                block.message.body.execution_payload.tree_hash_root()
            }
            SignedBlindedBeaconBlock::Capella(block) => {
                block.message.body.execution_payload.tree_hash_root()
            }
            SignedBlindedBeaconBlock::Deneb(block) => {
                block.message.body.execution_payload.tree_hash_root()
            }
            SignedBlindedBeaconBlock::Electra(block) => {
                block.message.body.execution_payload.tree_hash_root()
            }
            SignedBlindedBeaconBlock::Fulu(block) => {
                block.message.body.execution_payload.tree_hash_root()
            }
            SignedBlindedBeaconBlock::Gloas(_) => {
                // Gloas (ePBS): no traditional blinded block unblinding — builders reveal
                // payloads via execution payload envelopes, not via this endpoint.
                return Err("invalid fork".to_string());
            }
        };
        let block_hash = block
            .message()
            .body()
            .execution_payload()
            .unwrap()
            .block_hash();
        info!(
            execution_payload_root = %root,
            ?block_hash,
            "Submitting blinded beacon block to builder"
        );
        let payload = self.el.get_payload_by_root(&root).ok_or_else(|| {
            format!("missing payload for root: {root:?}, block_hash: {block_hash:?}",)
        })?;

        let (payload, blobs) = payload.deconstruct();
        let full_block = block
            .try_into_full_block(Some(payload.clone()))
            .ok_or("Internal error, just provided a payload")?;
        debug!(
            txs_count = payload.transactions().len(),
            blob_count = blobs.as_ref().map(|b| b.commitments.len()),
            "Got full payload"
        );
        if self.broadcast_to_bn {
            debug!(
                block_hash = ?payload.block_hash(),
                "Broadcasting builder block to BN"
            );
            let publish_block_request = PublishBlockRequest::new(
                Arc::new(full_block),
                blobs.clone().map(|b| (b.proofs, b.blobs)),
            );
            self.beacon_client
                .post_beacon_blocks_v2(
                    &publish_block_request,
                    Some(BroadcastValidation::ConsensusAndEquivocation),
                )
                .await
                .map_err(|e| format!("Failed to post blinded block {e:?}"))?;
        }
        Ok(FullPayloadContents::new(payload, blobs))
    }

    pub async fn get_header(
        &self,
        slot: Slot,
        parent_hash: ExecutionBlockHash,
        pubkey: PublicKeyBytes,
    ) -> Result<SignedBuilderBid<E>, String> {
        info!("In get_header");
        // Check if the pubkey has registered with the builder if required
        if self.validate_pubkey && !self.val_registration_cache.read().contains_key(&pubkey) {
            return Err("validator not registered with builder".to_string());
        }
        let payload_parameters = {
            let mut guard = self.payload_id_cache.write();
            guard.remove(&parent_hash)
        };

        let payload_parameters = match payload_parameters {
            Some(params) => params,
            None => {
                warn!("Payload params not cached for parent_hash {}", parent_hash);
                self.get_payload_params(slot, None, pubkey, None).await?
            }
        };

        info!("Got payload params");

        let fork = self.fork_name_at_slot(slot);

        let payload_response_type = self
            .el
            .get_full_payload_with(
                PayloadParameters {
                    parent_hash: payload_parameters.parent_hash,
                    parent_gas_limit: payload_parameters.parent_gas_limit,
                    proposer_gas_limit: payload_parameters.proposer_gas_limit,
                    payload_attributes: &payload_parameters.payload_attributes,
                    forkchoice_update_params: &payload_parameters.forkchoice_update_params,
                    current_fork: payload_parameters.current_fork,
                },
                // If apply_operations is set, do NOT cache the payload at this point, we are about
                // to mutate it and it would be incorrect to cache the unmutated payload.
                //
                // This is a flaw in apply_operations generally, if you want the mock builder to
                // actually return payloads then this option should be turned off.
                if self.apply_operations {
                    |_, _| None
                } else {
                    ExecutionLayer::cache_payload
                },
            )
            .await
            .map_err(|e| format!("couldn't get payload {:?}", e))?;

        info!("Got payload message, fork {}", fork);

        let mut message = match payload_response_type {
            crate::GetPayloadResponseType::Full(payload_response) => {
                #[allow(clippy::type_complexity)]
                let (payload, value, maybe_blobs_bundle, maybe_requests): (
                    ExecutionPayload<E>,
                    Uint256,
                    Option<BlobsBundle<E>>,
                    Option<ExecutionRequests<E>>,
                ) = payload_response.into();

                match fork {
                    ForkName::Gloas => BuilderBid::Gloas(BuilderBidGloas {
                        header: payload
                            .as_gloas()
                            .map_err(|_| "incorrect payload variant".to_string())?
                            .into(),
                        blob_kzg_commitments: maybe_blobs_bundle
                            .map(|b| b.commitments.clone())
                            .unwrap_or_default(),
                        value: self.get_bid_value(value),
                        pubkey: self.builder_sk.public_key().compress(),
                        execution_requests: maybe_requests.unwrap_or_default(),
                    }),
                    ForkName::Fulu => BuilderBid::Fulu(BuilderBidFulu {
                        header: payload
                            .as_fulu()
                            .map_err(|_| "incorrect payload variant".to_string())?
                            .into(),
                        blob_kzg_commitments: maybe_blobs_bundle
                            .map(|b| b.commitments.clone())
                            .unwrap_or_default(),
                        value: self.get_bid_value(value),
                        pubkey: self.builder_sk.public_key().compress(),
                        execution_requests: maybe_requests.unwrap_or_default(),
                    }),
                    ForkName::Electra => BuilderBid::Electra(BuilderBidElectra {
                        header: payload
                            .as_electra()
                            .map_err(|_| "incorrect payload variant".to_string())?
                            .into(),
                        blob_kzg_commitments: maybe_blobs_bundle
                            .map(|b| b.commitments.clone())
                            .unwrap_or_default(),
                        value: self.get_bid_value(value),
                        pubkey: self.builder_sk.public_key().compress(),
                        execution_requests: maybe_requests.unwrap_or_default(),
                    }),
                    ForkName::Deneb => BuilderBid::Deneb(BuilderBidDeneb {
                        header: payload
                            .as_deneb()
                            .map_err(|_| "incorrect payload variant".to_string())?
                            .into(),
                        blob_kzg_commitments: maybe_blobs_bundle
                            .map(|b| b.commitments.clone())
                            .unwrap_or_default(),
                        value: self.get_bid_value(value),
                        pubkey: self.builder_sk.public_key().compress(),
                    }),
                    ForkName::Capella => BuilderBid::Capella(BuilderBidCapella {
                        header: payload
                            .as_capella()
                            .map_err(|_| "incorrect payload variant".to_string())?
                            .into(),
                        value: self.get_bid_value(value),
                        pubkey: self.builder_sk.public_key().compress(),
                    }),
                    ForkName::Bellatrix => BuilderBid::Bellatrix(BuilderBidBellatrix {
                        header: payload
                            .as_bellatrix()
                            .map_err(|_| "incorrect payload variant".to_string())?
                            .into(),
                        value: self.get_bid_value(value),
                        pubkey: self.builder_sk.public_key().compress(),
                    }),
                    ForkName::Base | ForkName::Altair => return Err("invalid fork".to_string()),
                }
            }
            _ => panic!("just requested full payload, cannot get blinded"),
        };

        if self.apply_operations {
            info!("Applying operations");
            self.apply_operations(&mut message);
        }
        info!("Signing builder message");

        let mut signature = message.sign_builder_message(&self.builder_sk, &self.spec);

        if *self.invalidate_signatures.read() {
            signature = Signature::empty();
        };
        let signed_bid = SignedBuilderBid { message, signature };
        info!("Builder bid {:?}", &signed_bid.message.value());
        Ok(signed_bid)
    }

    fn fork_name_at_slot(&self, slot: Slot) -> ForkName {
        self.spec.fork_name_at_slot::<E>(slot)
    }

    fn get_bid_value(&self, value: Uint256) -> Uint256 {
        if self.max_bid {
            Uint256::MAX
        } else if !self.apply_operations {
            value
        } else {
            Uint256::from(DEFAULT_BUILDER_PAYLOAD_VALUE_WEI)
        }
    }

    /// Prepare the execution layer for payload creation every slot for the correct
    /// proposer index
    pub async fn prepare_execution_layer(&self) -> Result<(), String> {
        info!("Starting a task to prepare the execution layer");
        let mut head_event_stream = self
            .beacon_client
            .get_events::<E>(&[EventTopic::Head])
            .await
            .map_err(|e| format!("Failed to get head event {:?}", e))?;

        while let Some(Ok(event)) = head_event_stream.next().await {
            match event {
                EventKind::Head(head) => {
                    debug!(
                        block_hash = %head.block,
                        "Got a new head event"
                    );
                    let next_slot = head.slot + 1;
                    // Find the next proposer index from the cached data or through a beacon api call
                    let epoch = next_slot.epoch(E::slots_per_epoch());
                    let position_in_slot = next_slot.as_u64() % E::slots_per_epoch();
                    let proposer_data = {
                        let proposers_opt = {
                            let proposers_cache = self.proposers_cache.read();
                            proposers_cache.get(&epoch).cloned()
                        };
                        match proposers_opt {
                            Some(proposers) => proposers
                                .get(position_in_slot as usize)
                                .expect("position in slot is max epoch size")
                                .clone(),
                            None => {
                                // make a call to the beacon api and populate the cache
                                let duties: Vec<_> = self
                                    .beacon_client
                                    .get_validator_duties_proposer(epoch)
                                    .await
                                    .map_err(|e| {
                                        format!(
                                            "Failed to get proposer duties for epoch: {}, {:?}",
                                            epoch, e
                                        )
                                    })?
                                    .data;
                                let proposer_data = duties
                                    .get(position_in_slot as usize)
                                    .expect("position in slot is max epoch size")
                                    .clone();
                                self.proposers_cache.write().insert(epoch, duties);
                                proposer_data
                            }
                        }
                    };
                    self.prepare_execution_layer_internal(
                        head.slot,
                        head.block,
                        proposer_data.validator_index,
                        proposer_data.pubkey,
                    )
                    .await?;
                }
                e => {
                    warn!(
                        event = %e.topic_name(),
                        "Got an unexpected event"
                    );
                }
            }
        }
        Ok(())
    }

    async fn prepare_execution_layer_internal(
        &self,
        current_slot: Slot,
        head_block_root: Hash256,
        validator_index: u64,
        pubkey: PublicKeyBytes,
    ) -> Result<(), String> {
        let next_slot = current_slot + 1;
        let payload_parameters = self
            .get_payload_params(
                next_slot,
                Some(head_block_root),
                pubkey,
                Some(validator_index),
            )
            .await?;

        self.payload_id_cache
            .write()
            .insert(payload_parameters.parent_hash, payload_parameters);
        Ok(())
    }

    /// Get the `PayloadParameters` for requesting an ExecutionPayload for `slot`
    /// for the given `validator_index` and `pubkey`.
    async fn get_payload_params(
        &self,
        slot: Slot,
        head_block_root: Option<Hash256>,
        pubkey: PublicKeyBytes,
        validator_index: Option<u64>,
    ) -> Result<PayloadParametersCloned, String> {
        let fork = self.fork_name_at_slot(slot);

        let block_id = match head_block_root {
            Some(block_root) => BlockId::Root(block_root),
            None => BlockId::Head,
        };
        let head = self
            .beacon_client
            .get_beacon_blocks::<E>(block_id)
            .await
            .map_err(|_| "couldn't get head".to_string())?
            .ok_or_else(|| "missing head block".to_string())?
            .into_data();

        let head_block_root = head_block_root.unwrap_or(head.canonical_root());

        let head_execution_payload = head
            .message()
            .body()
            .execution_payload()
            .map_err(|_| "pre-merge block".to_string())?;
        let head_execution_hash = head_execution_payload.block_hash();
        let head_gas_limit = head_execution_payload.gas_limit();

        let finalized_execution_hash = self
            .beacon_client
            .get_beacon_blocks::<E>(BlockId::Finalized)
            .await
            .map_err(|e| format!("couldn't get finalized block: {e:?}"))?
            .ok_or_else(|| "missing finalized block".to_string())?
            .data()
            .message()
            .body()
            .execution_payload()
            .map_err(|_| "pre-merge block".to_string())?
            .block_hash();

        let justified_execution_hash = self
            .beacon_client
            .get_beacon_blocks::<E>(BlockId::Justified)
            .await
            .map_err(|e| format!("couldn't get justified block: {e:?}"))?
            .ok_or_else(|| "missing justified block".to_string())?
            .data()
            .message()
            .body()
            .execution_payload()
            .map_err(|_| "pre-merge block".to_string())?
            .block_hash();

        let (fee_recipient, proposer_gas_limit) =
            match self.val_registration_cache.read().get(&pubkey) {
                Some(cached_data) => (
                    cached_data.message.fee_recipient,
                    cached_data.message.gas_limit,
                ),
                None => {
                    warn!(
                        "Validator not registered {}, using default fee recipient and gas limits",
                        pubkey
                    );
                    (DEFAULT_FEE_RECIPIENT, DEFAULT_GAS_LIMIT)
                }
            };
        let slots_since_genesis = slot.as_u64() - self.spec.genesis_slot.as_u64();

        let genesis_time = if let Some(genesis_time) = self.genesis_time {
            genesis_time
        } else {
            self.beacon_client
                .get_beacon_genesis()
                .await
                .map_err(|_| "couldn't get beacon genesis".to_string())?
                .data
                .genesis_time
        };
        let timestamp = (slots_since_genesis * self.spec.seconds_per_slot) + genesis_time;

        let head_state: BeaconState<E> = self
            .beacon_client
            .get_debug_beacon_states(StateId::Head)
            .await
            .map_err(|_| "couldn't get state".to_string())?
            .ok_or_else(|| "missing state".to_string())?
            .into_data();

        let prev_randao = head_state
            .get_randao_mix(head_state.current_epoch())
            .map_err(|_| "couldn't get prev randao".to_string())?;

        let expected_withdrawals = if fork.capella_enabled() {
            Some(
                self.beacon_client
                    .get_expected_withdrawals(&StateId::Head)
                    .await
                    .map_err(|e| format!("Failed to get expected withdrawals: {:?}", e))?
                    .data,
            )
        } else {
            None
        };

        let payload_attributes = match fork {
            // the withdrawals root is filled in by operations, but we supply the valid withdrawals
            // first to avoid polluting the execution block generator with invalid payload attributes
            // NOTE: this was part of an effort to add payload attribute uniqueness checks,
            // which was abandoned because it broke too many tests in subtle ways.
            ForkName::Bellatrix | ForkName::Capella => PayloadAttributes::new(
                timestamp,
                *prev_randao,
                fee_recipient,
                expected_withdrawals,
                None,
            ),
            ForkName::Deneb | ForkName::Electra | ForkName::Fulu | ForkName::Gloas => {
                PayloadAttributes::new(
                    timestamp,
                    *prev_randao,
                    fee_recipient,
                    expected_withdrawals,
                    Some(head_block_root),
                )
            }
            ForkName::Base | ForkName::Altair => {
                return Err("invalid fork".to_string());
            }
        };

        // Tells the execution layer that the `validator_index` is expected to propose
        // a block on top of `head_block_root` for the given slot
        let val_index = validator_index.unwrap_or(
            self.beacon_client
                .get_beacon_states_validator_id(StateId::Head, &ValidatorId::PublicKey(pubkey))
                .await
                .map_err(|_| "couldn't get validator".to_string())?
                .ok_or_else(|| "missing validator".to_string())?
                .data
                .index,
        );

        self.el
            .insert_proposer(slot, head_block_root, val_index, payload_attributes.clone())
            .await;

        let forkchoice_update_params = ForkchoiceUpdateParameters {
            head_hash: Some(head_execution_hash),
            finalized_hash: Some(finalized_execution_hash),
            justified_hash: Some(justified_execution_hash),
            head_root: head_block_root,
        };

        let _status = self
            .el
            .notify_forkchoice_updated(
                head_execution_hash,
                justified_execution_hash,
                finalized_execution_hash,
                slot - 1,
                head_block_root,
            )
            .await
            .map_err(|e| format!("fcu call failed : {:?}", e))?;

        let payload_parameters = PayloadParametersCloned {
            parent_hash: head_execution_hash,
            parent_gas_limit: head_gas_limit,
            proposer_gas_limit: Some(proposer_gas_limit),
            payload_attributes,
            forkchoice_update_params,
            current_fork: fork,
        };
        Ok(payload_parameters)
    }
}

/// POST /eth/v1/builder/validators
async fn validators_handler<E: EthSpec>(
    State(builder): State<MockBuilder<E>>,
    axum::Json(registrations): axum::Json<Vec<SignedValidatorRegistrationData>>,
) -> Result<impl IntoResponse, ApiError> {
    builder
        .register_validators(registrations)
        .await
        .map_err(ApiError::from)?;
    Ok(axum::http::StatusCode::OK)
}

/// GET /eth/v1/builder/status
async fn status_handler() -> impl IntoResponse {
    (
        axum::http::StatusCode::OK,
        [("Server", "vibehouse-mock-builder-server")],
    )
}

/// GET /eth/v1/builder/header/:slot/:parent_hash/:pubkey
async fn header_handler<E: EthSpec>(
    State(builder): State<MockBuilder<E>>,
    Path((slot, parent_hash, pubkey)): Path<(Slot, ExecutionBlockHash, PublicKeyBytes)>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let fork_name = builder.fork_name_at_slot(slot);
    let signed_bid = builder
        .get_header(slot, parent_hash, pubkey)
        .await
        .map_err(ApiError::from)?;

    let accept_header: eth2::types::Accept = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(eth2::types::Accept::Any);

    match accept_header {
        eth2::types::Accept::Ssz => Ok((
            axum::http::StatusCode::OK,
            [
                (CONTENT_TYPE_HEADER, SSZ_CONTENT_TYPE_HEADER.to_string()),
                (CONSENSUS_VERSION_HEADER, fork_name.to_string()),
                ("Server", "vibehouse-mock-builder-server".to_string()),
            ],
            signed_bid.as_ssz_bytes(),
        )
            .into_response()),
        eth2::types::Accept::Json | eth2::types::Accept::Any => {
            let resp: ForkVersionedResponse<_> = ForkVersionedResponse {
                version: fork_name,
                metadata: Default::default(),
                data: signed_bid,
            };
            Ok((
                axum::http::StatusCode::OK,
                [("Server", "vibehouse-mock-builder-server".to_string())],
                axum::Json(resp),
            )
                .into_response())
        }
    }
}

/// Route blinded_blocks requests to SSZ or JSON handler based on content-type.
async fn blinded_blocks_dispatch<E: EthSpec>(
    State(builder): State<MockBuilder<E>>,
    Path(version_str): Path<String>,
    req: axum::extract::Request,
) -> axum::response::Response {
    let endpoint_version: EndpointVersion = match version_str.parse() {
        Ok(v) => v,
        Err(_) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                "Invalid EndpointVersion",
            )
                .into_response();
        }
    };
    let headers = req.headers().clone();
    let body = match axum::body::to_bytes(req.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                format!("Failed to read body: {e}"),
            )
                .into_response();
        }
    };
    let content_type = headers
        .get(CONTENT_TYPE_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if endpoint_version != EndpointVersion(1) && endpoint_version != EndpointVersion(2) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            format!("Unsupported version: {endpoint_version}"),
        )
            .into_response();
    }

    let fork_name: ForkName = match headers
        .get(CONSENSUS_VERSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
    {
        Some(f) => f,
        None => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                "Missing or invalid consensus version header",
            )
                .into_response();
        }
    };

    if content_type == SSZ_CONTENT_TYPE_HEADER {
        let block = match SignedBlindedBeaconBlock::<E>::from_ssz_bytes_by_fork(&body, fork_name) {
            Ok(b) => b,
            Err(e) => {
                return (axum::http::StatusCode::BAD_REQUEST, format!("{:?}", e)).into_response();
            }
        };
        let payload = match builder.submit_blinded_block(block).await {
            Ok(p) => p,
            Err(e) => return (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
        };

        if endpoint_version == EndpointVersion(1) {
            (
                axum::http::StatusCode::OK,
                [
                    (CONTENT_TYPE_HEADER, SSZ_CONTENT_TYPE_HEADER.to_string()),
                    (CONSENSUS_VERSION_HEADER, fork_name.to_string()),
                    ("Server", "vibehouse-mock-builder-server".to_string()),
                ],
                payload.as_ssz_bytes(),
            )
                .into_response()
        } else {
            (
                axum::http::StatusCode::ACCEPTED,
                [
                    (CONSENSUS_VERSION_HEADER, fork_name.to_string()),
                    ("Server", "vibehouse-mock-builder-server".to_string()),
                ],
                Vec::<u8>::new(),
            )
                .into_response()
        }
    } else {
        let block: SignedBlindedBeaconBlock<E> = match serde_json::from_slice(&body) {
            Ok(b) => b,
            Err(e) => {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    format!("Invalid JSON: {e}"),
                )
                    .into_response();
            }
        };
        let payload = match builder.submit_blinded_block(block).await {
            Ok(p) => p,
            Err(e) => return (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
        };
        let resp: ForkVersionedResponse<_> = ForkVersionedResponse {
            version: fork_name,
            metadata: Default::default(),
            data: payload,
        };

        let Ok(json_payload) = serde_json::to_string(&resp) else {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "couldn't serialize response",
            )
                .into_response();
        };

        if endpoint_version == EndpointVersion(1) {
            let Ok(body_str) = serde_json::to_string(&json_payload) else {
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "invalid JSON",
                )
                    .into_response();
            };
            (
                axum::http::StatusCode::OK,
                [
                    (CONSENSUS_VERSION_HEADER, fork_name.to_string()),
                    ("Server", "vibehouse-mock-builder-server".to_string()),
                ],
                body_str,
            )
                .into_response()
        } else {
            (
                axum::http::StatusCode::ACCEPTED,
                [
                    (CONSENSUS_VERSION_HEADER, fork_name.to_string()),
                    ("Server", "vibehouse-mock-builder-server".to_string()),
                ],
                String::new(),
            )
                .into_response()
        }
    }
}

/// Serve the builder API using axum.
pub fn serve<E: EthSpec>(
    listen_addr: Ipv4Addr,
    listen_port: u16,
    builder: MockBuilder<E>,
) -> Result<(SocketAddr, impl Future<Output = ()>), crate::test_utils::Error> {
    let app = Router::<MockBuilder<E>>::new()
        .route(
            "/eth/v1/builder/validators",
            post(
                |state: State<MockBuilder<E>>,
                 body: axum::Json<Vec<SignedValidatorRegistrationData>>| async move {
                    validators_handler::<E>(state, body).await
                },
            ),
        )
        .route(
            "/eth/{version}/builder/blinded_blocks",
            post(blinded_blocks_dispatch::<E>),
        )
        .route("/eth/v1/builder/status", get(status_handler))
        .route(
            "/eth/v1/builder/header/{slot}/{parent_hash}/{pubkey}",
            get(
                |state: State<MockBuilder<E>>,
                 path: Path<(Slot, ExecutionBlockHash, PublicKeyBytes)>,
                 headers: HeaderMap| async move {
                    header_handler::<E>(state, path, headers).await
                },
            ),
        )
        .with_state(builder);

    let addr = SocketAddrV4::new(listen_addr, listen_port);
    let listener = std::net::TcpListener::bind(addr).expect("mock builder server should bind");
    listener
        .set_nonblocking(true)
        .expect("mock builder should set nonblocking");
    let listener =
        tokio::net::TcpListener::from_std(listener).expect("mock builder tokio listener");
    let listening_socket = listener
        .local_addr()
        .expect("mock builder should have local addr");

    let server = async move {
        axum::serve(listener, app).await.ok();
    };

    Ok((listening_socket, server))
}
