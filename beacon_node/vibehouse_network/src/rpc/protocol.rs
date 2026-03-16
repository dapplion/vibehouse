use super::methods::*;
use crate::rpc::codec::SSZSnappyInboundCodec;
use futures::future::BoxFuture;
use futures::prelude::{AsyncRead, AsyncWrite};
use futures::{FutureExt, StreamExt};
use libp2p::core::{InboundUpgrade, UpgradeInfo};
use ssz::Encode;
use ssz_types::VariableList;
use std::io;
use std::marker::PhantomData;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use strum::{AsRefStr, Display, EnumString, IntoStaticStr};
use tokio_util::{
    codec::Framed,
    compat::{Compat, FuturesAsyncReadCompatExt},
};
use types::{
    BeaconBlock, BeaconBlockAltair, BeaconBlockBase, BlobSidecar, ChainSpec, DataColumnSidecar,
    EmptyBlock, Epoch, EthSpec, EthSpecId, ForkContext, ForkName, LightClientBootstrap,
    LightClientBootstrapAltair, LightClientFinalityUpdate, LightClientFinalityUpdateAltair,
    LightClientOptimisticUpdate, LightClientOptimisticUpdateAltair, LightClientUpdate,
    MainnetEthSpec, MinimalEthSpec, Signature, SignedBeaconBlock,
};

// Note: Hardcoding the `EthSpec` type for `SignedBeaconBlock` as min/max values is
// same across different `EthSpec` implementations.
pub static SIGNED_BEACON_BLOCK_BASE_MIN: LazyLock<usize> = LazyLock::new(|| {
    SignedBeaconBlock::<MainnetEthSpec>::from_block(
        BeaconBlock::Base(BeaconBlockBase::<MainnetEthSpec>::empty(
            &MainnetEthSpec::default_spec(),
        )),
        Signature::empty(),
    )
    .as_ssz_bytes()
    .len()
});
pub static SIGNED_BEACON_BLOCK_BASE_MAX: LazyLock<usize> = LazyLock::new(|| {
    SignedBeaconBlock::<MainnetEthSpec>::from_block(
        BeaconBlock::Base(BeaconBlockBase::full(&MainnetEthSpec::default_spec())),
        Signature::empty(),
    )
    .as_ssz_bytes()
    .len()
});

pub static SIGNED_BEACON_BLOCK_ALTAIR_MAX: LazyLock<usize> = LazyLock::new(|| {
    SignedBeaconBlock::<MainnetEthSpec>::from_block(
        BeaconBlock::Altair(BeaconBlockAltair::full(&MainnetEthSpec::default_spec())),
        Signature::empty(),
    )
    .as_ssz_bytes()
    .len()
});

/// The `BeaconBlockBellatrix` block has an `ExecutionPayload` field which has a max size ~16 GiB for future proofing.
/// We calculate the value from its fields instead of constructing the block and checking the length.
/// Note: This is only the theoretical upper bound. We further bound the max size we receive over the network
/// with `max_payload_size`.
pub static SIGNED_BEACON_BLOCK_BELLATRIX_MAX: LazyLock<usize> =
    LazyLock::new(||     // Size of a full altair block
    *SIGNED_BEACON_BLOCK_ALTAIR_MAX
    + types::ExecutionPayload::<MainnetEthSpec>::max_execution_payload_bellatrix_size() // adding max size of execution payload (~16gb)
    + ssz::BYTES_PER_LENGTH_OFFSET); // Adding the additional ssz offset for the `ExecutionPayload` field

pub static BLOB_SIDECAR_SIZE: LazyLock<usize> =
    LazyLock::new(BlobSidecar::<MainnetEthSpec>::max_size);

pub static SIGNED_EXECUTION_PAYLOAD_ENVELOPE_MAX: LazyLock<usize> = LazyLock::new(|| {
    // Use the same upper bound as bellatrix blocks since both contain a full execution payload.
    *SIGNED_BEACON_BLOCK_BELLATRIX_MAX
});

pub static BLOB_SIDECAR_SIZE_MINIMAL: LazyLock<usize> =
    LazyLock::new(BlobSidecar::<MinimalEthSpec>::max_size);

pub static ERROR_TYPE_MIN: LazyLock<usize> = LazyLock::new(|| {
    VariableList::<u8, MaxErrorLen>::from(Vec::<u8>::new())
        .as_ssz_bytes()
        .len()
});

pub static ERROR_TYPE_MAX: LazyLock<usize> = LazyLock::new(|| {
    VariableList::<u8, MaxErrorLen>::from(vec![0u8; MAX_ERROR_LEN as usize])
        .as_ssz_bytes()
        .len()
});

pub static LIGHT_CLIENT_FINALITY_UPDATE_CAPELLA_MAX: LazyLock<usize> = LazyLock::new(|| {
    LightClientFinalityUpdate::<MainnetEthSpec>::ssz_max_len_for_fork(ForkName::Capella)
});
pub static LIGHT_CLIENT_FINALITY_UPDATE_DENEB_MAX: LazyLock<usize> = LazyLock::new(|| {
    LightClientFinalityUpdate::<MainnetEthSpec>::ssz_max_len_for_fork(ForkName::Deneb)
});
pub static LIGHT_CLIENT_FINALITY_UPDATE_ELECTRA_MAX: LazyLock<usize> = LazyLock::new(|| {
    LightClientFinalityUpdate::<MainnetEthSpec>::ssz_max_len_for_fork(ForkName::Electra)
});
pub static LIGHT_CLIENT_OPTIMISTIC_UPDATE_CAPELLA_MAX: LazyLock<usize> = LazyLock::new(|| {
    LightClientOptimisticUpdate::<MainnetEthSpec>::ssz_max_len_for_fork(ForkName::Capella)
});
pub static LIGHT_CLIENT_OPTIMISTIC_UPDATE_DENEB_MAX: LazyLock<usize> = LazyLock::new(|| {
    LightClientOptimisticUpdate::<MainnetEthSpec>::ssz_max_len_for_fork(ForkName::Deneb)
});
pub static LIGHT_CLIENT_OPTIMISTIC_UPDATE_ELECTRA_MAX: LazyLock<usize> = LazyLock::new(|| {
    LightClientOptimisticUpdate::<MainnetEthSpec>::ssz_max_len_for_fork(ForkName::Electra)
});
pub static LIGHT_CLIENT_BOOTSTRAP_CAPELLA_MAX: LazyLock<usize> = LazyLock::new(|| {
    LightClientBootstrap::<MainnetEthSpec>::ssz_max_len_for_fork(ForkName::Capella)
});
pub static LIGHT_CLIENT_BOOTSTRAP_DENEB_MAX: LazyLock<usize> =
    LazyLock::new(|| LightClientBootstrap::<MainnetEthSpec>::ssz_max_len_for_fork(ForkName::Deneb));
pub static LIGHT_CLIENT_BOOTSTRAP_ELECTRA_MAX: LazyLock<usize> = LazyLock::new(|| {
    LightClientBootstrap::<MainnetEthSpec>::ssz_max_len_for_fork(ForkName::Electra)
});

pub static LIGHT_CLIENT_UPDATES_BY_RANGE_CAPELLA_MAX: LazyLock<usize> =
    LazyLock::new(|| LightClientUpdate::<MainnetEthSpec>::ssz_max_len_for_fork(ForkName::Capella));
pub static LIGHT_CLIENT_UPDATES_BY_RANGE_DENEB_MAX: LazyLock<usize> =
    LazyLock::new(|| LightClientUpdate::<MainnetEthSpec>::ssz_max_len_for_fork(ForkName::Deneb));
pub static LIGHT_CLIENT_UPDATES_BY_RANGE_ELECTRA_MAX: LazyLock<usize> =
    LazyLock::new(|| LightClientUpdate::<MainnetEthSpec>::ssz_max_len_for_fork(ForkName::Electra));

/// The protocol prefix the RPC protocol id.
const PROTOCOL_PREFIX: &str = "/eth2/beacon_chain/req";
/// The number of seconds to wait for the first bytes of a request once a protocol has been
/// established before the stream is terminated.
const REQUEST_TIMEOUT: u64 = 15;

/// Returns the rpc limits for beacon_block_by_range and beacon_block_by_root responses.
///
/// Note: This function should take care to return the min/max limits accounting for all
/// previous valid forks when adding a new fork variant.
pub fn rpc_block_limits_by_fork(current_fork: ForkName) -> RpcLimits {
    match &current_fork {
        ForkName::Base => {
            RpcLimits::new(*SIGNED_BEACON_BLOCK_BASE_MIN, *SIGNED_BEACON_BLOCK_BASE_MAX)
        }
        ForkName::Altair => RpcLimits::new(
            *SIGNED_BEACON_BLOCK_BASE_MIN, // Base block is smaller than altair blocks
            *SIGNED_BEACON_BLOCK_ALTAIR_MAX, // Altair block is larger than base blocks
        ),
        // After the merge the max SSZ size of a block is absurdly big. The size is actually
        // bound by other constants, so here we default to the bellatrix's max value
        _ => RpcLimits::new(
            *SIGNED_BEACON_BLOCK_BASE_MIN, // Base block is smaller than altair and bellatrix blocks
            *SIGNED_BEACON_BLOCK_BELLATRIX_MAX, // Bellatrix block is larger than base and altair blocks
        ),
    }
}

fn rpc_light_client_updates_by_range_limits_by_fork(current_fork: ForkName) -> RpcLimits {
    let altair_fixed_len = LightClientFinalityUpdateAltair::<MainnetEthSpec>::ssz_fixed_len();

    match &current_fork {
        ForkName::Base => RpcLimits::new(0, 0),
        ForkName::Altair | ForkName::Bellatrix => {
            RpcLimits::new(altair_fixed_len, altair_fixed_len)
        }
        ForkName::Capella => {
            RpcLimits::new(altair_fixed_len, *LIGHT_CLIENT_UPDATES_BY_RANGE_CAPELLA_MAX)
        }
        ForkName::Deneb => {
            RpcLimits::new(altair_fixed_len, *LIGHT_CLIENT_UPDATES_BY_RANGE_DENEB_MAX)
        }
        ForkName::Electra | ForkName::Fulu | ForkName::Gloas => {
            RpcLimits::new(altair_fixed_len, *LIGHT_CLIENT_UPDATES_BY_RANGE_ELECTRA_MAX)
        }
    }
}

fn rpc_light_client_finality_update_limits_by_fork(current_fork: ForkName) -> RpcLimits {
    let altair_fixed_len = LightClientFinalityUpdateAltair::<MainnetEthSpec>::ssz_fixed_len();

    match &current_fork {
        ForkName::Base => RpcLimits::new(0, 0),
        ForkName::Altair | ForkName::Bellatrix => {
            RpcLimits::new(altair_fixed_len, altair_fixed_len)
        }
        ForkName::Capella => {
            RpcLimits::new(altair_fixed_len, *LIGHT_CLIENT_FINALITY_UPDATE_CAPELLA_MAX)
        }
        ForkName::Deneb => {
            RpcLimits::new(altair_fixed_len, *LIGHT_CLIENT_FINALITY_UPDATE_DENEB_MAX)
        }
        ForkName::Electra | ForkName::Fulu | ForkName::Gloas => {
            RpcLimits::new(altair_fixed_len, *LIGHT_CLIENT_FINALITY_UPDATE_ELECTRA_MAX)
        }
    }
}

fn rpc_light_client_optimistic_update_limits_by_fork(current_fork: ForkName) -> RpcLimits {
    let altair_fixed_len = LightClientOptimisticUpdateAltair::<MainnetEthSpec>::ssz_fixed_len();

    match &current_fork {
        ForkName::Base => RpcLimits::new(0, 0),
        ForkName::Altair | ForkName::Bellatrix => {
            RpcLimits::new(altair_fixed_len, altair_fixed_len)
        }
        ForkName::Capella => RpcLimits::new(
            altair_fixed_len,
            *LIGHT_CLIENT_OPTIMISTIC_UPDATE_CAPELLA_MAX,
        ),
        ForkName::Deneb => {
            RpcLimits::new(altair_fixed_len, *LIGHT_CLIENT_OPTIMISTIC_UPDATE_DENEB_MAX)
        }
        ForkName::Electra | ForkName::Fulu | ForkName::Gloas => RpcLimits::new(
            altair_fixed_len,
            *LIGHT_CLIENT_OPTIMISTIC_UPDATE_ELECTRA_MAX,
        ),
    }
}

fn rpc_light_client_bootstrap_limits_by_fork(current_fork: ForkName) -> RpcLimits {
    let altair_fixed_len = LightClientBootstrapAltair::<MainnetEthSpec>::ssz_fixed_len();

    match &current_fork {
        ForkName::Base => RpcLimits::new(0, 0),
        ForkName::Altair | ForkName::Bellatrix => {
            RpcLimits::new(altair_fixed_len, altair_fixed_len)
        }
        ForkName::Capella => RpcLimits::new(altair_fixed_len, *LIGHT_CLIENT_BOOTSTRAP_CAPELLA_MAX),
        ForkName::Deneb => RpcLimits::new(altair_fixed_len, *LIGHT_CLIENT_BOOTSTRAP_DENEB_MAX),
        ForkName::Electra | ForkName::Fulu | ForkName::Gloas => {
            RpcLimits::new(altair_fixed_len, *LIGHT_CLIENT_BOOTSTRAP_ELECTRA_MAX)
        }
    }
}

/// Protocol names to be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, AsRefStr, Display)]
#[strum(serialize_all = "snake_case")]
pub enum Protocol {
    /// The Status protocol name.
    Status,
    /// The Goodbye protocol name.
    Goodbye,
    /// The `BlocksByRange` protocol name.
    #[strum(serialize = "beacon_blocks_by_range")]
    BlocksByRange,
    /// The `BlocksByRoot` protocol name.
    #[strum(serialize = "beacon_blocks_by_root")]
    BlocksByRoot,
    /// The `BlobsByRange` protocol name.
    #[strum(serialize = "blob_sidecars_by_range")]
    BlobsByRange,
    /// The `BlobsByRoot` protocol name.
    #[strum(serialize = "blob_sidecars_by_root")]
    BlobsByRoot,
    /// The `DataColumnSidecarsByRoot` protocol name.
    #[strum(serialize = "data_column_sidecars_by_root")]
    DataColumnsByRoot,
    /// The `DataColumnSidecarsByRange` protocol name.
    #[strum(serialize = "data_column_sidecars_by_range")]
    DataColumnsByRange,
    /// The `Ping` protocol name.
    Ping,
    /// The `MetaData` protocol name.
    #[strum(serialize = "metadata")]
    MetaData,
    /// The `LightClientBootstrap` protocol name.
    #[strum(serialize = "light_client_bootstrap")]
    LightClientBootstrap,
    /// The `LightClientOptimisticUpdate` protocol name.
    #[strum(serialize = "light_client_optimistic_update")]
    LightClientOptimisticUpdate,
    /// The `LightClientFinalityUpdate` protocol name.
    #[strum(serialize = "light_client_finality_update")]
    LightClientFinalityUpdate,
    /// The `LightClientUpdatesByRange` protocol name
    #[strum(serialize = "light_client_updates_by_range")]
    LightClientUpdatesByRange,
    /// The `ExecutionPayloadEnvelopesByRoot` protocol name.
    #[strum(serialize = "execution_payload_envelopes_by_root")]
    ExecutionPayloadEnvelopesByRoot,
}

impl Protocol {
    pub(crate) fn terminator(self) -> Option<ResponseTermination> {
        match self {
            Protocol::Status => None,
            Protocol::Goodbye => None,
            Protocol::BlocksByRange => Some(ResponseTermination::BlocksByRange),
            Protocol::BlocksByRoot => Some(ResponseTermination::BlocksByRoot),
            Protocol::BlobsByRange => Some(ResponseTermination::BlobsByRange),
            Protocol::BlobsByRoot => Some(ResponseTermination::BlobsByRoot),
            Protocol::DataColumnsByRoot => Some(ResponseTermination::DataColumnsByRoot),
            Protocol::DataColumnsByRange => Some(ResponseTermination::DataColumnsByRange),
            Protocol::Ping => None,
            Protocol::MetaData => None,
            Protocol::LightClientBootstrap => None,
            Protocol::LightClientOptimisticUpdate => None,
            Protocol::LightClientFinalityUpdate => None,
            Protocol::LightClientUpdatesByRange => None,
            Protocol::ExecutionPayloadEnvelopesByRoot => {
                Some(ResponseTermination::ExecutionPayloadEnvelopesByRoot)
            }
        }
    }
}

/// RPC Encondings supported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Encoding {
    SSZSnappy,
}

/// All valid protocol name and version combinations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SupportedProtocol {
    StatusV1,
    StatusV2,
    GoodbyeV1,
    BlocksByRangeV1,
    BlocksByRangeV2,
    BlocksByRootV1,
    BlocksByRootV2,
    BlobsByRangeV1,
    BlobsByRootV1,
    DataColumnsByRootV1,
    DataColumnsByRangeV1,
    PingV1,
    MetaDataV1,
    MetaDataV2,
    MetaDataV3,
    LightClientBootstrapV1,
    LightClientOptimisticUpdateV1,
    LightClientFinalityUpdateV1,
    LightClientUpdatesByRangeV1,
    ExecutionPayloadEnvelopesByRootV1,
}

impl SupportedProtocol {
    pub fn version_string(&self) -> &'static str {
        match self {
            SupportedProtocol::StatusV1 => "1",
            SupportedProtocol::StatusV2 => "2",
            SupportedProtocol::GoodbyeV1 => "1",
            SupportedProtocol::BlocksByRangeV1 => "1",
            SupportedProtocol::BlocksByRangeV2 => "2",
            SupportedProtocol::BlocksByRootV1 => "1",
            SupportedProtocol::BlocksByRootV2 => "2",
            SupportedProtocol::BlobsByRangeV1 => "1",
            SupportedProtocol::BlobsByRootV1 => "1",
            SupportedProtocol::DataColumnsByRootV1 => "1",
            SupportedProtocol::DataColumnsByRangeV1 => "1",
            SupportedProtocol::PingV1 => "1",
            SupportedProtocol::MetaDataV1 => "1",
            SupportedProtocol::MetaDataV2 => "2",
            SupportedProtocol::MetaDataV3 => "3",
            SupportedProtocol::LightClientBootstrapV1 => "1",
            SupportedProtocol::LightClientOptimisticUpdateV1 => "1",
            SupportedProtocol::LightClientFinalityUpdateV1 => "1",
            SupportedProtocol::LightClientUpdatesByRangeV1 => "1",
            SupportedProtocol::ExecutionPayloadEnvelopesByRootV1 => "1",
        }
    }

    pub fn protocol(&self) -> Protocol {
        match self {
            SupportedProtocol::StatusV1 => Protocol::Status,
            SupportedProtocol::StatusV2 => Protocol::Status,
            SupportedProtocol::GoodbyeV1 => Protocol::Goodbye,
            SupportedProtocol::BlocksByRangeV1 => Protocol::BlocksByRange,
            SupportedProtocol::BlocksByRangeV2 => Protocol::BlocksByRange,
            SupportedProtocol::BlocksByRootV1 => Protocol::BlocksByRoot,
            SupportedProtocol::BlocksByRootV2 => Protocol::BlocksByRoot,
            SupportedProtocol::BlobsByRangeV1 => Protocol::BlobsByRange,
            SupportedProtocol::BlobsByRootV1 => Protocol::BlobsByRoot,
            SupportedProtocol::DataColumnsByRootV1 => Protocol::DataColumnsByRoot,
            SupportedProtocol::DataColumnsByRangeV1 => Protocol::DataColumnsByRange,
            SupportedProtocol::PingV1 => Protocol::Ping,
            SupportedProtocol::MetaDataV1 => Protocol::MetaData,
            SupportedProtocol::MetaDataV2 => Protocol::MetaData,
            SupportedProtocol::MetaDataV3 => Protocol::MetaData,
            SupportedProtocol::LightClientBootstrapV1 => Protocol::LightClientBootstrap,
            SupportedProtocol::LightClientOptimisticUpdateV1 => {
                Protocol::LightClientOptimisticUpdate
            }
            SupportedProtocol::LightClientFinalityUpdateV1 => Protocol::LightClientFinalityUpdate,
            SupportedProtocol::LightClientUpdatesByRangeV1 => Protocol::LightClientUpdatesByRange,
            SupportedProtocol::ExecutionPayloadEnvelopesByRootV1 => {
                Protocol::ExecutionPayloadEnvelopesByRoot
            }
        }
    }

    fn currently_supported(fork_context: &ForkContext) -> Vec<ProtocolId> {
        let mut supported = vec![
            ProtocolId::new(Self::StatusV2, Encoding::SSZSnappy),
            ProtocolId::new(Self::StatusV1, Encoding::SSZSnappy),
            ProtocolId::new(Self::GoodbyeV1, Encoding::SSZSnappy),
            // V2 variants have higher preference then V1
            ProtocolId::new(Self::BlocksByRangeV2, Encoding::SSZSnappy),
            ProtocolId::new(Self::BlocksByRangeV1, Encoding::SSZSnappy),
            ProtocolId::new(Self::BlocksByRootV2, Encoding::SSZSnappy),
            ProtocolId::new(Self::BlocksByRootV1, Encoding::SSZSnappy),
            ProtocolId::new(Self::PingV1, Encoding::SSZSnappy),
        ];
        if fork_context.spec.is_peer_das_scheduled() {
            supported.extend_from_slice(&[
                // V3 variants have higher preference for protocol negotation
                ProtocolId::new(Self::MetaDataV3, Encoding::SSZSnappy),
                ProtocolId::new(Self::MetaDataV2, Encoding::SSZSnappy),
                ProtocolId::new(Self::MetaDataV1, Encoding::SSZSnappy),
            ]);
        } else {
            supported.extend_from_slice(&[
                ProtocolId::new(Self::MetaDataV2, Encoding::SSZSnappy),
                ProtocolId::new(Self::MetaDataV1, Encoding::SSZSnappy),
            ]);
        }
        if fork_context.fork_exists(ForkName::Deneb) {
            supported.extend_from_slice(&[
                ProtocolId::new(SupportedProtocol::BlobsByRootV1, Encoding::SSZSnappy),
                ProtocolId::new(SupportedProtocol::BlobsByRangeV1, Encoding::SSZSnappy),
            ]);
        }
        if fork_context.spec.is_peer_das_scheduled() {
            supported.extend_from_slice(&[
                ProtocolId::new(SupportedProtocol::DataColumnsByRootV1, Encoding::SSZSnappy),
                ProtocolId::new(SupportedProtocol::DataColumnsByRangeV1, Encoding::SSZSnappy),
            ]);
        }
        if fork_context.fork_exists(ForkName::Gloas) {
            supported.push(ProtocolId::new(
                SupportedProtocol::ExecutionPayloadEnvelopesByRootV1,
                Encoding::SSZSnappy,
            ));
        }
        supported
    }
}

impl std::fmt::Display for Encoding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let repr = match self {
            Encoding::SSZSnappy => "ssz_snappy",
        };
        f.write_str(repr)
    }
}

#[derive(Debug, Clone)]
pub struct RPCProtocol<E: EthSpec> {
    pub fork_context: Arc<ForkContext>,
    pub max_rpc_size: usize,
    pub enable_light_client_server: bool,
    pub phantom: PhantomData<E>,
}

impl<E: EthSpec> UpgradeInfo for RPCProtocol<E> {
    type Info = ProtocolId;
    type InfoIter = Vec<Self::Info>;

    /// The list of supported RPC protocols for Vibehouse.
    fn protocol_info(&self) -> Self::InfoIter {
        let mut supported_protocols = SupportedProtocol::currently_supported(&self.fork_context);
        if self.enable_light_client_server {
            supported_protocols.push(ProtocolId::new(
                SupportedProtocol::LightClientBootstrapV1,
                Encoding::SSZSnappy,
            ));
            supported_protocols.push(ProtocolId::new(
                SupportedProtocol::LightClientOptimisticUpdateV1,
                Encoding::SSZSnappy,
            ));
            supported_protocols.push(ProtocolId::new(
                SupportedProtocol::LightClientFinalityUpdateV1,
                Encoding::SSZSnappy,
            ));
        }
        supported_protocols
    }
}

/// Represents the ssz length bounds for RPC messages.
#[derive(Debug, PartialEq)]
pub struct RpcLimits {
    pub min: usize,
    pub max: usize,
}

impl RpcLimits {
    pub fn new(min: usize, max: usize) -> Self {
        Self { min, max }
    }

    /// Returns true if the given length is greater than `max_rpc_size` or out of
    /// bounds for the given ssz type, returns false otherwise.
    pub fn is_out_of_bounds(&self, length: usize, max_rpc_size: usize) -> bool {
        length > std::cmp::min(self.max, max_rpc_size) || length < self.min
    }
}

/// Tracks the types in a protocol id.
#[derive(Clone, Debug)]
pub struct ProtocolId {
    /// The protocol name and version
    pub versioned_protocol: SupportedProtocol,

    /// The encoding of the RPC.
    pub encoding: Encoding,

    /// The protocol id that is formed from the above fields.
    protocol_id: String,
}

impl AsRef<str> for ProtocolId {
    fn as_ref(&self) -> &str {
        self.protocol_id.as_ref()
    }
}

impl ProtocolId {
    /// Returns min and max size for messages of given protocol id requests.
    pub fn rpc_request_limits<E: EthSpec>(&self, spec: &ChainSpec) -> RpcLimits {
        match self.versioned_protocol.protocol() {
            Protocol::Status => RpcLimits::new(
                <StatusMessageV1 as Encode>::ssz_fixed_len(),
                <StatusMessageV2 as Encode>::ssz_fixed_len(),
            ),
            Protocol::Goodbye => RpcLimits::new(
                <GoodbyeReason as Encode>::ssz_fixed_len(),
                <GoodbyeReason as Encode>::ssz_fixed_len(),
            ),
            // V1 and V2 requests are the same
            Protocol::BlocksByRange => RpcLimits::new(
                <OldBlocksByRangeRequestV2 as Encode>::ssz_fixed_len(),
                <OldBlocksByRangeRequestV2 as Encode>::ssz_fixed_len(),
            ),
            Protocol::BlocksByRoot => RpcLimits::new(0, spec.max_blocks_by_root_request),
            Protocol::BlobsByRange => RpcLimits::new(
                <BlobsByRangeRequest as Encode>::ssz_fixed_len(),
                <BlobsByRangeRequest as Encode>::ssz_fixed_len(),
            ),
            Protocol::BlobsByRoot => RpcLimits::new(0, spec.max_blobs_by_root_request),
            Protocol::DataColumnsByRoot => RpcLimits::new(0, spec.max_data_columns_by_root_request),
            Protocol::DataColumnsByRange => RpcLimits::new(
                DataColumnsByRangeRequest::ssz_min_len(),
                DataColumnsByRangeRequest::ssz_max_len::<E>(),
            ),
            Protocol::Ping => RpcLimits::new(
                <Ping as Encode>::ssz_fixed_len(),
                <Ping as Encode>::ssz_fixed_len(),
            ),
            Protocol::LightClientBootstrap => RpcLimits::new(
                <LightClientBootstrapRequest as Encode>::ssz_fixed_len(),
                <LightClientBootstrapRequest as Encode>::ssz_fixed_len(),
            ),
            Protocol::LightClientOptimisticUpdate => RpcLimits::new(0, 0),
            Protocol::LightClientFinalityUpdate => RpcLimits::new(0, 0),
            Protocol::LightClientUpdatesByRange => RpcLimits::new(
                LightClientUpdatesByRangeRequest::ssz_min_len(),
                LightClientUpdatesByRangeRequest::ssz_max_len(),
            ),
            Protocol::MetaData => RpcLimits::new(0, 0), // Metadata requests are empty
            Protocol::ExecutionPayloadEnvelopesByRoot => {
                RpcLimits::new(0, spec.max_execution_payload_envelopes_by_root_request)
            }
        }
    }

    /// Returns min and max size for messages of given protocol id responses.
    pub fn rpc_response_limits<E: EthSpec>(&self, fork_context: &ForkContext) -> RpcLimits {
        match self.versioned_protocol.protocol() {
            Protocol::Status => RpcLimits::new(
                <StatusMessageV1 as Encode>::ssz_fixed_len(),
                <StatusMessageV2 as Encode>::ssz_fixed_len(),
            ),
            Protocol::Goodbye => RpcLimits::new(0, 0), // Goodbye request has no response
            Protocol::BlocksByRange => rpc_block_limits_by_fork(fork_context.current_fork_name()),
            Protocol::BlocksByRoot => rpc_block_limits_by_fork(fork_context.current_fork_name()),
            Protocol::BlobsByRange => rpc_blob_limits::<E>(),
            Protocol::BlobsByRoot => rpc_blob_limits::<E>(),
            Protocol::DataColumnsByRoot => {
                rpc_data_column_limits::<E>(fork_context.current_fork_epoch(), &fork_context.spec)
            }
            Protocol::DataColumnsByRange => {
                rpc_data_column_limits::<E>(fork_context.current_fork_epoch(), &fork_context.spec)
            }
            Protocol::Ping => RpcLimits::new(
                <Ping as Encode>::ssz_fixed_len(),
                <Ping as Encode>::ssz_fixed_len(),
            ),
            Protocol::MetaData => RpcLimits::new(
                <MetaDataV1<E> as Encode>::ssz_fixed_len(),
                <MetaDataV3<E> as Encode>::ssz_fixed_len(),
            ),
            Protocol::LightClientBootstrap => {
                rpc_light_client_bootstrap_limits_by_fork(fork_context.current_fork_name())
            }
            Protocol::LightClientOptimisticUpdate => {
                rpc_light_client_optimistic_update_limits_by_fork(fork_context.current_fork_name())
            }
            Protocol::LightClientFinalityUpdate => {
                rpc_light_client_finality_update_limits_by_fork(fork_context.current_fork_name())
            }
            Protocol::LightClientUpdatesByRange => {
                rpc_light_client_updates_by_range_limits_by_fork(fork_context.current_fork_name())
            }
            Protocol::ExecutionPayloadEnvelopesByRoot => {
                RpcLimits::new(0, *SIGNED_EXECUTION_PAYLOAD_ENVELOPE_MAX)
            }
        }
    }

    /// Returns `true` if the given `ProtocolId` should expect `context_bytes` in the
    /// beginning of the stream, else returns `false`.
    pub fn has_context_bytes(&self) -> bool {
        match self.versioned_protocol {
            SupportedProtocol::BlocksByRangeV2
            | SupportedProtocol::BlocksByRootV2
            | SupportedProtocol::BlobsByRangeV1
            | SupportedProtocol::BlobsByRootV1
            | SupportedProtocol::DataColumnsByRootV1
            | SupportedProtocol::DataColumnsByRangeV1
            | SupportedProtocol::LightClientBootstrapV1
            | SupportedProtocol::LightClientOptimisticUpdateV1
            | SupportedProtocol::LightClientFinalityUpdateV1
            | SupportedProtocol::LightClientUpdatesByRangeV1
            | SupportedProtocol::ExecutionPayloadEnvelopesByRootV1 => true,
            SupportedProtocol::StatusV1
            | SupportedProtocol::StatusV2
            | SupportedProtocol::BlocksByRootV1
            | SupportedProtocol::BlocksByRangeV1
            | SupportedProtocol::PingV1
            | SupportedProtocol::MetaDataV1
            | SupportedProtocol::MetaDataV2
            | SupportedProtocol::MetaDataV3
            | SupportedProtocol::GoodbyeV1 => false,
        }
    }
}

/// An RPC protocol ID.
impl ProtocolId {
    pub fn new(versioned_protocol: SupportedProtocol, encoding: Encoding) -> Self {
        let protocol_id = format!(
            "{}/{}/{}/{}",
            PROTOCOL_PREFIX,
            versioned_protocol.protocol(),
            versioned_protocol.version_string(),
            encoding
        );

        ProtocolId {
            versioned_protocol,
            encoding,
            protocol_id,
        }
    }
}

pub fn rpc_blob_limits<E: EthSpec>() -> RpcLimits {
    match E::spec_name() {
        EthSpecId::Minimal => {
            RpcLimits::new(*BLOB_SIDECAR_SIZE_MINIMAL, *BLOB_SIDECAR_SIZE_MINIMAL)
        }
        EthSpecId::Mainnet | EthSpecId::Gnosis => {
            RpcLimits::new(*BLOB_SIDECAR_SIZE, *BLOB_SIDECAR_SIZE)
        }
    }
}

pub fn rpc_data_column_limits<E: EthSpec>(
    current_digest_epoch: Epoch,
    spec: &ChainSpec,
) -> RpcLimits {
    RpcLimits::new(
        DataColumnSidecar::<E>::min_size(),
        DataColumnSidecar::<E>::max_size(spec.max_blobs_per_block(current_digest_epoch) as usize),
    )
}

/* Inbound upgrade */

// The inbound protocol reads the request, decodes it and returns the stream to the protocol
// handler to respond to once ready.

pub type InboundOutput<TSocket, E> = (RequestType<E>, InboundFramed<TSocket, E>);
pub type InboundFramed<TSocket, E> =
    Framed<std::pin::Pin<Box<Compat<TSocket>>>, SSZSnappyInboundCodec<E>>;

impl<TSocket, E> InboundUpgrade<TSocket> for RPCProtocol<E>
where
    TSocket: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    E: EthSpec,
{
    type Output = InboundOutput<TSocket, E>;
    type Error = (Protocol, RPCError);
    type Future = BoxFuture<'static, Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, socket: TSocket, protocol: ProtocolId) -> Self::Future {
        async move {
            let versioned_protocol = protocol.versioned_protocol;
            // convert the socket to tokio compatible socket
            let socket = socket.compat();
            let codec = match protocol.encoding {
                Encoding::SSZSnappy => SSZSnappyInboundCodec::new(
                    protocol,
                    self.max_rpc_size,
                    self.fork_context.clone(),
                ),
            };

            let socket = Framed::new(Box::pin(socket), codec);

            // MetaData requests should be empty, return the stream
            match versioned_protocol {
                SupportedProtocol::MetaDataV1 => {
                    Ok((RequestType::MetaData(MetadataRequest::new_v1()), socket))
                }
                SupportedProtocol::MetaDataV2 => {
                    Ok((RequestType::MetaData(MetadataRequest::new_v2()), socket))
                }
                SupportedProtocol::MetaDataV3 => {
                    Ok((RequestType::MetaData(MetadataRequest::new_v3()), socket))
                }
                SupportedProtocol::LightClientOptimisticUpdateV1 => {
                    Ok((RequestType::LightClientOptimisticUpdate, socket))
                }
                SupportedProtocol::LightClientFinalityUpdateV1 => {
                    Ok((RequestType::LightClientFinalityUpdate, socket))
                }
                _ => {
                    match tokio::time::timeout(
                        Duration::from_secs(REQUEST_TIMEOUT),
                        socket.into_future(),
                    )
                    .await
                    {
                        Err(e) => Err((versioned_protocol.protocol(), RPCError::from(e))),
                        Ok((Some(Ok(request)), stream)) => Ok((request, stream)),
                        Ok((Some(Err(e)), _)) => Err((versioned_protocol.protocol(), e)),
                        Ok((None, _)) => {
                            Err((versioned_protocol.protocol(), RPCError::IncompleteStream))
                        }
                    }
                }
            }
        }
        .boxed()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequestType<E: EthSpec> {
    Status(StatusMessage),
    Goodbye(GoodbyeReason),
    BlocksByRange(OldBlocksByRangeRequest),
    BlocksByRoot(BlocksByRootRequest),
    BlobsByRange(BlobsByRangeRequest),
    BlobsByRoot(BlobsByRootRequest),
    DataColumnsByRoot(DataColumnsByRootRequest<E>),
    DataColumnsByRange(DataColumnsByRangeRequest),
    LightClientBootstrap(LightClientBootstrapRequest),
    LightClientOptimisticUpdate,
    LightClientFinalityUpdate,
    LightClientUpdatesByRange(LightClientUpdatesByRangeRequest),
    ExecutionPayloadEnvelopesByRoot(ExecutionPayloadEnvelopesByRootRequest),
    Ping(Ping),
    MetaData(MetadataRequest<E>),
}

/// Implements the encoding per supported protocol for `RPCRequest`.
impl<E: EthSpec> RequestType<E> {
    /* These functions are used in the handler for stream management */

    /// Maximum number of responses expected for this request.
    pub fn max_responses(&self, digest_epoch: Epoch, spec: &ChainSpec) -> u64 {
        match self {
            RequestType::Status(_) => 1,
            RequestType::Goodbye(_) => 0,
            RequestType::BlocksByRange(req) => *req.count(),
            RequestType::BlocksByRoot(req) => req.block_roots().len() as u64,
            RequestType::BlobsByRange(req) => req.max_blobs_requested(digest_epoch, spec),
            RequestType::BlobsByRoot(req) => req.blob_ids.len() as u64,
            RequestType::DataColumnsByRoot(req) => req.max_requested() as u64,
            RequestType::DataColumnsByRange(req) => req.max_requested::<E>(),
            RequestType::Ping(_) => 1,
            RequestType::MetaData(_) => 1,
            RequestType::LightClientBootstrap(_) => 1,
            RequestType::LightClientOptimisticUpdate => 1,
            RequestType::LightClientFinalityUpdate => 1,
            RequestType::LightClientUpdatesByRange(req) => req.count,
            RequestType::ExecutionPayloadEnvelopesByRoot(req) => req.block_roots.len() as u64,
        }
    }

    /// Gives the corresponding `SupportedProtocol` to this request.
    pub fn versioned_protocol(&self) -> SupportedProtocol {
        match self {
            RequestType::Status(req) => match req {
                StatusMessage::V1(_) => SupportedProtocol::StatusV1,
                StatusMessage::V2(_) => SupportedProtocol::StatusV2,
            },
            RequestType::Goodbye(_) => SupportedProtocol::GoodbyeV1,
            RequestType::BlocksByRange(req) => match req {
                OldBlocksByRangeRequest::V1(_) => SupportedProtocol::BlocksByRangeV1,
                OldBlocksByRangeRequest::V2(_) => SupportedProtocol::BlocksByRangeV2,
            },
            RequestType::BlocksByRoot(req) => match req {
                BlocksByRootRequest::V1(_) => SupportedProtocol::BlocksByRootV1,
                BlocksByRootRequest::V2(_) => SupportedProtocol::BlocksByRootV2,
            },
            RequestType::BlobsByRange(_) => SupportedProtocol::BlobsByRangeV1,
            RequestType::BlobsByRoot(_) => SupportedProtocol::BlobsByRootV1,
            RequestType::DataColumnsByRoot(_) => SupportedProtocol::DataColumnsByRootV1,
            RequestType::DataColumnsByRange(_) => SupportedProtocol::DataColumnsByRangeV1,
            RequestType::Ping(_) => SupportedProtocol::PingV1,
            RequestType::MetaData(req) => match req {
                MetadataRequest::V1(_) => SupportedProtocol::MetaDataV1,
                MetadataRequest::V2(_) => SupportedProtocol::MetaDataV2,
                MetadataRequest::V3(_) => SupportedProtocol::MetaDataV3,
            },
            RequestType::LightClientBootstrap(_) => SupportedProtocol::LightClientBootstrapV1,
            RequestType::LightClientOptimisticUpdate => {
                SupportedProtocol::LightClientOptimisticUpdateV1
            }
            RequestType::LightClientFinalityUpdate => {
                SupportedProtocol::LightClientFinalityUpdateV1
            }
            RequestType::LightClientUpdatesByRange(_) => {
                SupportedProtocol::LightClientUpdatesByRangeV1
            }
            RequestType::ExecutionPayloadEnvelopesByRoot(_) => {
                SupportedProtocol::ExecutionPayloadEnvelopesByRootV1
            }
        }
    }

    /// Returns the `ResponseTermination` type associated with the request if a stream gets
    /// terminated.
    pub fn stream_termination(&self) -> ResponseTermination {
        match self {
            // this only gets called after `multiple_responses()` returns true. Therefore, only
            // variants that have `multiple_responses()` can have values.
            RequestType::BlocksByRange(_) => ResponseTermination::BlocksByRange,
            RequestType::BlocksByRoot(_) => ResponseTermination::BlocksByRoot,
            RequestType::BlobsByRange(_) => ResponseTermination::BlobsByRange,
            RequestType::BlobsByRoot(_) => ResponseTermination::BlobsByRoot,
            RequestType::DataColumnsByRoot(_) => ResponseTermination::DataColumnsByRoot,
            RequestType::DataColumnsByRange(_) => ResponseTermination::DataColumnsByRange,
            RequestType::ExecutionPayloadEnvelopesByRoot(_) => {
                ResponseTermination::ExecutionPayloadEnvelopesByRoot
            }
            RequestType::Status(_) => unreachable!(),
            RequestType::Goodbye(_) => unreachable!(),
            RequestType::Ping(_) => unreachable!(),
            RequestType::MetaData(_) => unreachable!(),
            RequestType::LightClientBootstrap(_) => unreachable!(),
            RequestType::LightClientFinalityUpdate => unreachable!(),
            RequestType::LightClientOptimisticUpdate => unreachable!(),
            RequestType::LightClientUpdatesByRange(_) => unreachable!(),
        }
    }

    pub fn supported_protocols(&self) -> Vec<ProtocolId> {
        match self {
            // add more protocols when versions/encodings are supported
            RequestType::Status(_) => vec![
                ProtocolId::new(SupportedProtocol::StatusV2, Encoding::SSZSnappy),
                ProtocolId::new(SupportedProtocol::StatusV1, Encoding::SSZSnappy),
            ],
            RequestType::Goodbye(_) => vec![ProtocolId::new(
                SupportedProtocol::GoodbyeV1,
                Encoding::SSZSnappy,
            )],
            RequestType::BlocksByRange(_) => vec![
                ProtocolId::new(SupportedProtocol::BlocksByRangeV2, Encoding::SSZSnappy),
                ProtocolId::new(SupportedProtocol::BlocksByRangeV1, Encoding::SSZSnappy),
            ],
            RequestType::BlocksByRoot(_) => vec![
                ProtocolId::new(SupportedProtocol::BlocksByRootV2, Encoding::SSZSnappy),
                ProtocolId::new(SupportedProtocol::BlocksByRootV1, Encoding::SSZSnappy),
            ],
            RequestType::BlobsByRange(_) => vec![ProtocolId::new(
                SupportedProtocol::BlobsByRangeV1,
                Encoding::SSZSnappy,
            )],
            RequestType::BlobsByRoot(_) => vec![ProtocolId::new(
                SupportedProtocol::BlobsByRootV1,
                Encoding::SSZSnappy,
            )],
            RequestType::DataColumnsByRoot(_) => vec![ProtocolId::new(
                SupportedProtocol::DataColumnsByRootV1,
                Encoding::SSZSnappy,
            )],
            RequestType::DataColumnsByRange(_) => vec![ProtocolId::new(
                SupportedProtocol::DataColumnsByRangeV1,
                Encoding::SSZSnappy,
            )],
            RequestType::Ping(_) => vec![ProtocolId::new(
                SupportedProtocol::PingV1,
                Encoding::SSZSnappy,
            )],
            RequestType::MetaData(_) => vec![
                ProtocolId::new(SupportedProtocol::MetaDataV3, Encoding::SSZSnappy),
                ProtocolId::new(SupportedProtocol::MetaDataV2, Encoding::SSZSnappy),
                ProtocolId::new(SupportedProtocol::MetaDataV1, Encoding::SSZSnappy),
            ],
            RequestType::LightClientBootstrap(_) => vec![ProtocolId::new(
                SupportedProtocol::LightClientBootstrapV1,
                Encoding::SSZSnappy,
            )],
            RequestType::LightClientOptimisticUpdate => vec![ProtocolId::new(
                SupportedProtocol::LightClientOptimisticUpdateV1,
                Encoding::SSZSnappy,
            )],
            RequestType::LightClientFinalityUpdate => vec![ProtocolId::new(
                SupportedProtocol::LightClientFinalityUpdateV1,
                Encoding::SSZSnappy,
            )],
            RequestType::LightClientUpdatesByRange(_) => vec![ProtocolId::new(
                SupportedProtocol::LightClientUpdatesByRangeV1,
                Encoding::SSZSnappy,
            )],
            RequestType::ExecutionPayloadEnvelopesByRoot(_) => vec![ProtocolId::new(
                SupportedProtocol::ExecutionPayloadEnvelopesByRootV1,
                Encoding::SSZSnappy,
            )],
        }
    }

    pub fn expect_exactly_one_response(&self) -> bool {
        match self {
            RequestType::Status(_) => true,
            RequestType::Goodbye(_) => false,
            RequestType::BlocksByRange(_) => false,
            RequestType::BlocksByRoot(_) => false,
            RequestType::BlobsByRange(_) => false,
            RequestType::BlobsByRoot(_) => false,
            RequestType::DataColumnsByRoot(_) => false,
            RequestType::DataColumnsByRange(_) => false,
            RequestType::Ping(_) => true,
            RequestType::MetaData(_) => true,
            RequestType::LightClientBootstrap(_) => true,
            RequestType::LightClientOptimisticUpdate => true,
            RequestType::LightClientFinalityUpdate => true,
            RequestType::LightClientUpdatesByRange(_) => true,
            RequestType::ExecutionPayloadEnvelopesByRoot(_) => false,
        }
    }
}

/// Error in RPC Encoding/Decoding.
#[derive(Debug, Clone, PartialEq, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum RPCError {
    /// Error when decoding the raw buffer from ssz.
    // NOTE: in the future a ssz::DecodeError should map to an InvalidData error
    #[strum(serialize = "decode_error")]
    SSZDecodeError(ssz::DecodeError),
    /// IO Error.
    IoError(String),
    /// The peer returned a valid response but the response indicated an error.
    ErrorResponse(RpcErrorResponse, String),
    /// Timed out waiting for a response.
    StreamTimeout,
    /// Peer does not support the protocol.
    UnsupportedProtocol,
    /// Stream ended unexpectedly.
    IncompleteStream,
    /// Peer sent invalid data.
    InvalidData(String),
    /// An error occurred due to internal reasons. Ex: timer failure.
    InternalError(&'static str),
    /// Negotiation with this peer timed out.
    NegotiationTimeout,
    /// Handler rejected this request.
    HandlerRejected,
    /// We have intentionally disconnected.
    Disconnected,
}

impl From<ssz::DecodeError> for RPCError {
    #[inline]
    fn from(err: ssz::DecodeError) -> Self {
        RPCError::SSZDecodeError(err)
    }
}
impl From<tokio::time::error::Elapsed> for RPCError {
    fn from(_: tokio::time::error::Elapsed) -> Self {
        RPCError::StreamTimeout
    }
}

impl From<io::Error> for RPCError {
    fn from(err: io::Error) -> Self {
        RPCError::IoError(err.to_string())
    }
}

// Error trait is required for `ProtocolsHandler`
impl std::fmt::Display for RPCError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            RPCError::SSZDecodeError(ref err) => write!(f, "Error while decoding ssz: {:?}", err),
            RPCError::InvalidData(ref err) => write!(f, "Peer sent unexpected data: {}", err),
            RPCError::IoError(ref err) => write!(f, "IO Error: {}", err),
            RPCError::ErrorResponse(ref code, ref reason) => write!(
                f,
                "RPC response was an error: {} with reason: {}",
                code, reason
            ),
            RPCError::StreamTimeout => write!(f, "Stream Timeout"),
            RPCError::UnsupportedProtocol => write!(f, "Peer does not support the protocol"),
            RPCError::IncompleteStream => write!(f, "Stream ended unexpectedly"),
            RPCError::InternalError(ref err) => write!(f, "Internal error: {}", err),
            RPCError::NegotiationTimeout => write!(f, "Negotiation timeout"),
            RPCError::HandlerRejected => write!(f, "Handler rejected the request"),
            RPCError::Disconnected => write!(f, "Gracefully Disconnected"),
        }
    }
}

impl std::error::Error for RPCError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            // NOTE: this does have a source
            RPCError::SSZDecodeError(_) => None,
            RPCError::IoError(_) => None,
            RPCError::StreamTimeout => None,
            RPCError::UnsupportedProtocol => None,
            RPCError::IncompleteStream => None,
            RPCError::InvalidData(_) => None,
            RPCError::InternalError(_) => None,
            RPCError::ErrorResponse(_, _) => None,
            RPCError::NegotiationTimeout => None,
            RPCError::HandlerRejected => None,
            RPCError::Disconnected => None,
        }
    }
}

impl<E: EthSpec> std::fmt::Display for RequestType<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestType::Status(status) => write!(f, "Status Message: {}", status),
            RequestType::Goodbye(reason) => write!(f, "Goodbye: {}", reason),
            RequestType::BlocksByRange(req) => write!(f, "Blocks by range: {}", req),
            RequestType::BlocksByRoot(req) => write!(f, "Blocks by root: {:?}", req),
            RequestType::BlobsByRange(req) => write!(f, "Blobs by range: {:?}", req),
            RequestType::BlobsByRoot(req) => write!(f, "Blobs by root: {:?}", req),
            RequestType::DataColumnsByRoot(req) => write!(f, "Data columns by root: {:?}", req),
            RequestType::DataColumnsByRange(req) => {
                write!(f, "Data columns by range: {:?}", req)
            }
            RequestType::Ping(ping) => write!(f, "Ping: {}", ping.data),
            RequestType::MetaData(_) => write!(f, "MetaData request"),
            RequestType::LightClientBootstrap(bootstrap) => {
                write!(f, "Light client boostrap: {}", bootstrap.root)
            }
            RequestType::LightClientOptimisticUpdate => {
                write!(f, "Light client optimistic update request")
            }
            RequestType::LightClientFinalityUpdate => {
                write!(f, "Light client finality update request")
            }
            RequestType::LightClientUpdatesByRange(_) => {
                write!(f, "Light client updates by range request")
            }
            RequestType::ExecutionPayloadEnvelopesByRoot(req) => write!(
                f,
                "Execution payload envelopes by root: {} roots",
                req.block_roots.len()
            ),
        }
    }
}

impl RPCError {
    /// Get a `str` representation of the error.
    /// Used for metrics.
    pub fn as_static_str(&self) -> &'static str {
        match self {
            RPCError::ErrorResponse(code, ..) => code.into(),
            e => e.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssz::Encode;
    use std::str::FromStr;
    use types::{FixedBytesExtended, Hash256, MinimalEthSpec, Slot};

    type E = MinimalEthSpec;

    fn make_fork_context(fork: ForkName) -> Arc<ForkContext> {
        let spec = {
            let mut s = E::default_spec();
            // Enable all forks at epoch 0 up to the requested fork
            s.altair_fork_epoch = Some(Epoch::new(0));
            s.bellatrix_fork_epoch = Some(Epoch::new(0));
            s.capella_fork_epoch = Some(Epoch::new(0));
            s.deneb_fork_epoch = Some(Epoch::new(0));
            s.electra_fork_epoch = Some(Epoch::new(0));
            s.fulu_fork_epoch = Some(Epoch::new(0));
            // Only enable Gloas if requested
            if fork == ForkName::Gloas {
                s.gloas_fork_epoch = Some(Epoch::new(0));
            } else {
                s.gloas_fork_epoch = None;
            }
            s
        };
        Arc::new(ForkContext::new::<E>(Slot::new(0), Hash256::zero(), &spec))
    }

    // ── Protocol enum ───────────────────────────────────────────

    #[test]
    fn protocol_strum_serialization() {
        assert_eq!(Protocol::Status.to_string(), "status");
        assert_eq!(Protocol::Goodbye.to_string(), "goodbye");
        assert_eq!(
            Protocol::BlocksByRange.to_string(),
            "beacon_blocks_by_range"
        );
        assert_eq!(Protocol::BlocksByRoot.to_string(), "beacon_blocks_by_root");
        assert_eq!(Protocol::BlobsByRange.to_string(), "blob_sidecars_by_range");
        assert_eq!(Protocol::BlobsByRoot.to_string(), "blob_sidecars_by_root");
        assert_eq!(
            Protocol::DataColumnsByRoot.to_string(),
            "data_column_sidecars_by_root"
        );
        assert_eq!(
            Protocol::DataColumnsByRange.to_string(),
            "data_column_sidecars_by_range"
        );
        assert_eq!(Protocol::Ping.to_string(), "ping");
        assert_eq!(Protocol::MetaData.to_string(), "metadata");
        assert_eq!(
            Protocol::LightClientBootstrap.to_string(),
            "light_client_bootstrap"
        );
        assert_eq!(
            Protocol::LightClientOptimisticUpdate.to_string(),
            "light_client_optimistic_update"
        );
        assert_eq!(
            Protocol::LightClientFinalityUpdate.to_string(),
            "light_client_finality_update"
        );
        assert_eq!(
            Protocol::LightClientUpdatesByRange.to_string(),
            "light_client_updates_by_range"
        );
        assert_eq!(
            Protocol::ExecutionPayloadEnvelopesByRoot.to_string(),
            "execution_payload_envelopes_by_root"
        );
    }

    #[test]
    fn protocol_from_str_roundtrip() {
        assert_eq!(Protocol::from_str("status").unwrap(), Protocol::Status);
        assert_eq!(Protocol::from_str("goodbye").unwrap(), Protocol::Goodbye);
        assert_eq!(
            Protocol::from_str("beacon_blocks_by_range").unwrap(),
            Protocol::BlocksByRange
        );
        assert_eq!(Protocol::from_str("ping").unwrap(), Protocol::Ping);
        assert_eq!(Protocol::from_str("metadata").unwrap(), Protocol::MetaData);
        assert!(Protocol::from_str("nonexistent").is_err());
    }

    #[test]
    fn protocol_terminator_some_for_streaming() {
        assert_eq!(
            Protocol::BlocksByRange.terminator(),
            Some(ResponseTermination::BlocksByRange)
        );
        assert_eq!(
            Protocol::BlocksByRoot.terminator(),
            Some(ResponseTermination::BlocksByRoot)
        );
        assert_eq!(
            Protocol::BlobsByRange.terminator(),
            Some(ResponseTermination::BlobsByRange)
        );
        assert_eq!(
            Protocol::BlobsByRoot.terminator(),
            Some(ResponseTermination::BlobsByRoot)
        );
        assert_eq!(
            Protocol::DataColumnsByRoot.terminator(),
            Some(ResponseTermination::DataColumnsByRoot)
        );
        assert_eq!(
            Protocol::DataColumnsByRange.terminator(),
            Some(ResponseTermination::DataColumnsByRange)
        );
        assert_eq!(
            Protocol::ExecutionPayloadEnvelopesByRoot.terminator(),
            Some(ResponseTermination::ExecutionPayloadEnvelopesByRoot)
        );
    }

    #[test]
    fn protocol_terminator_none_for_single_response() {
        assert_eq!(Protocol::Status.terminator(), None);
        assert_eq!(Protocol::Goodbye.terminator(), None);
        assert_eq!(Protocol::Ping.terminator(), None);
        assert_eq!(Protocol::MetaData.terminator(), None);
        assert_eq!(Protocol::LightClientBootstrap.terminator(), None);
        assert_eq!(Protocol::LightClientOptimisticUpdate.terminator(), None);
        assert_eq!(Protocol::LightClientFinalityUpdate.terminator(), None);
        assert_eq!(Protocol::LightClientUpdatesByRange.terminator(), None);
    }

    // ── Encoding ────────────────────────────────────────────────

    #[test]
    fn encoding_display() {
        assert_eq!(Encoding::SSZSnappy.to_string(), "ssz_snappy");
    }

    // ── RpcLimits ───────────────────────────────────────────────

    #[test]
    fn rpc_limits_in_bounds() {
        let limits = RpcLimits::new(10, 100);
        assert!(!limits.is_out_of_bounds(50, 200));
        assert!(!limits.is_out_of_bounds(10, 200)); // exactly min
        assert!(!limits.is_out_of_bounds(100, 200)); // exactly max
    }

    #[test]
    fn rpc_limits_below_min() {
        let limits = RpcLimits::new(10, 100);
        assert!(limits.is_out_of_bounds(9, 200));
        assert!(limits.is_out_of_bounds(0, 200));
    }

    #[test]
    fn rpc_limits_above_max() {
        let limits = RpcLimits::new(10, 100);
        assert!(limits.is_out_of_bounds(101, 200));
    }

    #[test]
    fn rpc_limits_clamped_by_max_rpc_size() {
        let limits = RpcLimits::new(10, 100);
        // max_rpc_size=50 clamps effective max to 50
        assert!(!limits.is_out_of_bounds(50, 50));
        assert!(limits.is_out_of_bounds(51, 50));
    }

    #[test]
    fn rpc_limits_zero_min_max() {
        let limits = RpcLimits::new(0, 0);
        assert!(!limits.is_out_of_bounds(0, 100));
        assert!(limits.is_out_of_bounds(1, 100));
    }

    // ── SupportedProtocol ───────────────────────────────────────

    #[test]
    fn supported_protocol_version_strings() {
        assert_eq!(SupportedProtocol::StatusV1.version_string(), "1");
        assert_eq!(SupportedProtocol::StatusV2.version_string(), "2");
        assert_eq!(SupportedProtocol::BlocksByRangeV1.version_string(), "1");
        assert_eq!(SupportedProtocol::BlocksByRangeV2.version_string(), "2");
        assert_eq!(SupportedProtocol::MetaDataV1.version_string(), "1");
        assert_eq!(SupportedProtocol::MetaDataV2.version_string(), "2");
        assert_eq!(SupportedProtocol::MetaDataV3.version_string(), "3");
        assert_eq!(
            SupportedProtocol::ExecutionPayloadEnvelopesByRootV1.version_string(),
            "1"
        );
    }

    #[test]
    fn supported_protocol_to_protocol_mapping() {
        assert_eq!(SupportedProtocol::StatusV1.protocol(), Protocol::Status);
        assert_eq!(SupportedProtocol::StatusV2.protocol(), Protocol::Status);
        assert_eq!(SupportedProtocol::GoodbyeV1.protocol(), Protocol::Goodbye);
        assert_eq!(
            SupportedProtocol::BlocksByRangeV1.protocol(),
            Protocol::BlocksByRange
        );
        assert_eq!(
            SupportedProtocol::BlocksByRangeV2.protocol(),
            Protocol::BlocksByRange
        );
        assert_eq!(
            SupportedProtocol::BlocksByRootV1.protocol(),
            Protocol::BlocksByRoot
        );
        assert_eq!(
            SupportedProtocol::BlocksByRootV2.protocol(),
            Protocol::BlocksByRoot
        );
        assert_eq!(
            SupportedProtocol::BlobsByRangeV1.protocol(),
            Protocol::BlobsByRange
        );
        assert_eq!(
            SupportedProtocol::BlobsByRootV1.protocol(),
            Protocol::BlobsByRoot
        );
        assert_eq!(
            SupportedProtocol::DataColumnsByRootV1.protocol(),
            Protocol::DataColumnsByRoot
        );
        assert_eq!(
            SupportedProtocol::DataColumnsByRangeV1.protocol(),
            Protocol::DataColumnsByRange
        );
        assert_eq!(SupportedProtocol::PingV1.protocol(), Protocol::Ping);
        assert_eq!(SupportedProtocol::MetaDataV1.protocol(), Protocol::MetaData);
        assert_eq!(SupportedProtocol::MetaDataV2.protocol(), Protocol::MetaData);
        assert_eq!(SupportedProtocol::MetaDataV3.protocol(), Protocol::MetaData);
        assert_eq!(
            SupportedProtocol::ExecutionPayloadEnvelopesByRootV1.protocol(),
            Protocol::ExecutionPayloadEnvelopesByRoot
        );
    }

    #[test]
    fn currently_supported_includes_envelope_for_gloas() {
        let fc = make_fork_context(ForkName::Gloas);
        let protocols = SupportedProtocol::currently_supported(&fc);
        let has_envelope = protocols
            .iter()
            .any(|p| p.versioned_protocol == SupportedProtocol::ExecutionPayloadEnvelopesByRootV1);
        assert!(
            has_envelope,
            "Gloas fork context should include ExecutionPayloadEnvelopesByRootV1"
        );
    }

    #[test]
    fn currently_supported_excludes_envelope_pre_gloas() {
        let fc = make_fork_context(ForkName::Fulu);
        let protocols = SupportedProtocol::currently_supported(&fc);
        let has_envelope = protocols
            .iter()
            .any(|p| p.versioned_protocol == SupportedProtocol::ExecutionPayloadEnvelopesByRootV1);
        assert!(
            !has_envelope,
            "Pre-Gloas fork context should not include ExecutionPayloadEnvelopesByRootV1"
        );
    }

    #[test]
    fn currently_supported_always_has_core_protocols() {
        let fc = make_fork_context(ForkName::Fulu);
        let protocols = SupportedProtocol::currently_supported(&fc);
        let has = |sp: SupportedProtocol| protocols.iter().any(|p| p.versioned_protocol == sp);
        assert!(has(SupportedProtocol::StatusV1));
        assert!(has(SupportedProtocol::StatusV2));
        assert!(has(SupportedProtocol::GoodbyeV1));
        assert!(has(SupportedProtocol::BlocksByRangeV1));
        assert!(has(SupportedProtocol::BlocksByRangeV2));
        assert!(has(SupportedProtocol::BlocksByRootV1));
        assert!(has(SupportedProtocol::BlocksByRootV2));
        assert!(has(SupportedProtocol::PingV1));
    }

    #[test]
    fn currently_supported_includes_blobs_for_deneb_plus() {
        let fc = make_fork_context(ForkName::Fulu);
        let protocols = SupportedProtocol::currently_supported(&fc);
        let has = |sp: SupportedProtocol| protocols.iter().any(|p| p.versioned_protocol == sp);
        assert!(has(SupportedProtocol::BlobsByRangeV1));
        assert!(has(SupportedProtocol::BlobsByRootV1));
    }

    #[test]
    fn currently_supported_includes_data_columns_when_peerdas_scheduled() {
        let fc = make_fork_context(ForkName::Fulu);
        let protocols = SupportedProtocol::currently_supported(&fc);
        let has = |sp: SupportedProtocol| protocols.iter().any(|p| p.versioned_protocol == sp);
        assert!(has(SupportedProtocol::DataColumnsByRootV1));
        assert!(has(SupportedProtocol::DataColumnsByRangeV1));
    }

    #[test]
    fn currently_supported_metadata_v3_when_peerdas() {
        let fc = make_fork_context(ForkName::Fulu);
        let protocols = SupportedProtocol::currently_supported(&fc);
        let has = |sp: SupportedProtocol| protocols.iter().any(|p| p.versioned_protocol == sp);
        assert!(has(SupportedProtocol::MetaDataV3));
        assert!(has(SupportedProtocol::MetaDataV2));
        assert!(has(SupportedProtocol::MetaDataV1));
    }

    // ── ProtocolId ──────────────────────────────────────────────

    #[test]
    fn protocol_id_format() {
        let pid = ProtocolId::new(SupportedProtocol::StatusV1, Encoding::SSZSnappy);
        assert_eq!(pid.as_ref(), "/eth2/beacon_chain/req/status/1/ssz_snappy");
    }

    #[test]
    fn protocol_id_format_blocks_by_range_v2() {
        let pid = ProtocolId::new(SupportedProtocol::BlocksByRangeV2, Encoding::SSZSnappy);
        assert_eq!(
            pid.as_ref(),
            "/eth2/beacon_chain/req/beacon_blocks_by_range/2/ssz_snappy"
        );
    }

    #[test]
    fn protocol_id_format_envelope() {
        let pid = ProtocolId::new(
            SupportedProtocol::ExecutionPayloadEnvelopesByRootV1,
            Encoding::SSZSnappy,
        );
        assert_eq!(
            pid.as_ref(),
            "/eth2/beacon_chain/req/execution_payload_envelopes_by_root/1/ssz_snappy"
        );
    }

    #[test]
    fn protocol_id_has_context_bytes_true() {
        let context_protocols = [
            SupportedProtocol::BlocksByRangeV2,
            SupportedProtocol::BlocksByRootV2,
            SupportedProtocol::BlobsByRangeV1,
            SupportedProtocol::BlobsByRootV1,
            SupportedProtocol::DataColumnsByRootV1,
            SupportedProtocol::DataColumnsByRangeV1,
            SupportedProtocol::LightClientBootstrapV1,
            SupportedProtocol::LightClientOptimisticUpdateV1,
            SupportedProtocol::LightClientFinalityUpdateV1,
            SupportedProtocol::LightClientUpdatesByRangeV1,
            SupportedProtocol::ExecutionPayloadEnvelopesByRootV1,
        ];
        for sp in context_protocols {
            let pid = ProtocolId::new(sp, Encoding::SSZSnappy);
            assert!(
                pid.has_context_bytes(),
                "{:?} should have context bytes",
                sp
            );
        }
    }

    #[test]
    fn protocol_id_has_context_bytes_false() {
        let no_context_protocols = [
            SupportedProtocol::StatusV1,
            SupportedProtocol::StatusV2,
            SupportedProtocol::BlocksByRootV1,
            SupportedProtocol::BlocksByRangeV1,
            SupportedProtocol::PingV1,
            SupportedProtocol::MetaDataV1,
            SupportedProtocol::MetaDataV2,
            SupportedProtocol::MetaDataV3,
            SupportedProtocol::GoodbyeV1,
        ];
        for sp in no_context_protocols {
            let pid = ProtocolId::new(sp, Encoding::SSZSnappy);
            assert!(
                !pid.has_context_bytes(),
                "{:?} should NOT have context bytes",
                sp
            );
        }
    }

    // ── rpc_block_limits_by_fork ────────────────────────────────

    #[test]
    fn block_limits_base_fork() {
        let limits = rpc_block_limits_by_fork(ForkName::Base);
        assert_eq!(limits.min, *SIGNED_BEACON_BLOCK_BASE_MIN);
        assert_eq!(limits.max, *SIGNED_BEACON_BLOCK_BASE_MAX);
    }

    #[test]
    fn block_limits_altair_fork() {
        let limits = rpc_block_limits_by_fork(ForkName::Altair);
        assert_eq!(limits.min, *SIGNED_BEACON_BLOCK_BASE_MIN);
        assert_eq!(limits.max, *SIGNED_BEACON_BLOCK_ALTAIR_MAX);
    }

    #[test]
    fn block_limits_post_merge_uses_bellatrix_max() {
        for fork in [
            ForkName::Bellatrix,
            ForkName::Capella,
            ForkName::Deneb,
            ForkName::Electra,
            ForkName::Fulu,
            ForkName::Gloas,
        ] {
            let limits = rpc_block_limits_by_fork(fork);
            assert_eq!(limits.min, *SIGNED_BEACON_BLOCK_BASE_MIN);
            assert_eq!(limits.max, *SIGNED_BEACON_BLOCK_BELLATRIX_MAX);
        }
    }

    #[test]
    fn block_limits_min_less_than_max() {
        for fork in ForkName::list_all() {
            let limits = rpc_block_limits_by_fork(fork);
            assert!(limits.min <= limits.max, "min > max for fork {:?}", fork);
        }
    }

    // ── rpc_request_limits ──────────────────────────────────────

    #[test]
    fn request_limits_status_fixed_size() {
        let spec = E::default_spec();
        let pid = ProtocolId::new(SupportedProtocol::StatusV1, Encoding::SSZSnappy);
        let limits = pid.rpc_request_limits::<E>(&spec);
        assert_eq!(limits.min, <StatusMessageV1 as Encode>::ssz_fixed_len());
        assert_eq!(limits.max, <StatusMessageV2 as Encode>::ssz_fixed_len());
    }

    #[test]
    fn request_limits_goodbye_fixed_size() {
        let spec = E::default_spec();
        let pid = ProtocolId::new(SupportedProtocol::GoodbyeV1, Encoding::SSZSnappy);
        let limits = pid.rpc_request_limits::<E>(&spec);
        let goodbye_len = <GoodbyeReason as Encode>::ssz_fixed_len();
        assert_eq!(limits.min, goodbye_len);
        assert_eq!(limits.max, goodbye_len);
    }

    #[test]
    fn request_limits_ping_fixed_size() {
        let spec = E::default_spec();
        let pid = ProtocolId::new(SupportedProtocol::PingV1, Encoding::SSZSnappy);
        let limits = pid.rpc_request_limits::<E>(&spec);
        let ping_len = <Ping as Encode>::ssz_fixed_len();
        assert_eq!(limits.min, ping_len);
        assert_eq!(limits.max, ping_len);
    }

    #[test]
    fn request_limits_metadata_is_empty() {
        let spec = E::default_spec();
        let pid = ProtocolId::new(SupportedProtocol::MetaDataV1, Encoding::SSZSnappy);
        let limits = pid.rpc_request_limits::<E>(&spec);
        assert_eq!(limits.min, 0);
        assert_eq!(limits.max, 0);
    }

    #[test]
    fn request_limits_blocks_by_root_variable() {
        let spec = E::default_spec();
        let pid = ProtocolId::new(SupportedProtocol::BlocksByRootV2, Encoding::SSZSnappy);
        let limits = pid.rpc_request_limits::<E>(&spec);
        assert_eq!(limits.min, 0);
        assert_eq!(limits.max, spec.max_blocks_by_root_request);
    }

    #[test]
    fn request_limits_envelope_by_root_variable() {
        let spec = E::default_spec();
        let pid = ProtocolId::new(
            SupportedProtocol::ExecutionPayloadEnvelopesByRootV1,
            Encoding::SSZSnappy,
        );
        let limits = pid.rpc_request_limits::<E>(&spec);
        assert_eq!(limits.min, 0);
        assert_eq!(
            limits.max,
            spec.max_execution_payload_envelopes_by_root_request
        );
    }

    // ── rpc_response_limits ─────────────────────────────────────

    #[test]
    fn response_limits_goodbye_is_zero() {
        let fc = make_fork_context(ForkName::Fulu);
        let pid = ProtocolId::new(SupportedProtocol::GoodbyeV1, Encoding::SSZSnappy);
        let limits = pid.rpc_response_limits::<E>(&fc);
        assert_eq!(limits.min, 0);
        assert_eq!(limits.max, 0);
    }

    #[test]
    fn response_limits_ping_fixed() {
        let fc = make_fork_context(ForkName::Fulu);
        let pid = ProtocolId::new(SupportedProtocol::PingV1, Encoding::SSZSnappy);
        let limits = pid.rpc_response_limits::<E>(&fc);
        let ping_len = <Ping as Encode>::ssz_fixed_len();
        assert_eq!(limits.min, ping_len);
        assert_eq!(limits.max, ping_len);
    }

    #[test]
    fn response_limits_envelope_uses_bellatrix_max() {
        let fc = make_fork_context(ForkName::Gloas);
        let pid = ProtocolId::new(
            SupportedProtocol::ExecutionPayloadEnvelopesByRootV1,
            Encoding::SSZSnappy,
        );
        let limits = pid.rpc_response_limits::<E>(&fc);
        assert_eq!(limits.min, 0);
        assert_eq!(limits.max, *SIGNED_EXECUTION_PAYLOAD_ENVELOPE_MAX);
    }

    #[test]
    fn response_limits_metadata_spans_v1_to_v3() {
        let fc = make_fork_context(ForkName::Fulu);
        let pid = ProtocolId::new(SupportedProtocol::MetaDataV1, Encoding::SSZSnappy);
        let limits = pid.rpc_response_limits::<E>(&fc);
        assert_eq!(limits.min, <MetaDataV1<E> as Encode>::ssz_fixed_len());
        assert_eq!(limits.max, <MetaDataV3<E> as Encode>::ssz_fixed_len());
    }

    // ── Light client limits ─────────────────────────────────────

    #[test]
    fn light_client_limits_base_is_zero() {
        let base_limits_fns: Vec<fn(ForkName) -> RpcLimits> = vec![
            rpc_light_client_bootstrap_limits_by_fork,
            rpc_light_client_finality_update_limits_by_fork,
            rpc_light_client_optimistic_update_limits_by_fork,
            rpc_light_client_updates_by_range_limits_by_fork,
        ];
        for f in base_limits_fns {
            let limits = f(ForkName::Base);
            assert_eq!(limits.min, 0);
            assert_eq!(limits.max, 0);
        }
    }

    #[test]
    fn light_client_limits_altair_bellatrix_are_fixed() {
        for fork in [ForkName::Altair, ForkName::Bellatrix] {
            let limits = rpc_light_client_bootstrap_limits_by_fork(fork);
            assert_eq!(
                limits.min, limits.max,
                "Altair/Bellatrix bootstrap should be fixed-size"
            );
        }
    }

    #[test]
    fn light_client_limits_grow_with_forks() {
        let capella = rpc_light_client_bootstrap_limits_by_fork(ForkName::Capella);
        let deneb = rpc_light_client_bootstrap_limits_by_fork(ForkName::Deneb);
        let electra = rpc_light_client_bootstrap_limits_by_fork(ForkName::Electra);
        // All share the same altair min
        assert_eq!(capella.min, deneb.min);
        assert_eq!(deneb.min, electra.min);
        // Max should be non-zero
        assert!(capella.max > 0);
        assert!(deneb.max > 0);
        assert!(electra.max > 0);
    }

    // ── RPCError ────────────────────────────────────────────────

    #[test]
    fn rpc_error_display_variants() {
        let err = RPCError::StreamTimeout;
        assert_eq!(err.to_string(), "Stream Timeout");

        let err = RPCError::UnsupportedProtocol;
        assert_eq!(err.to_string(), "Peer does not support the protocol");

        let err = RPCError::IncompleteStream;
        assert_eq!(err.to_string(), "Stream ended unexpectedly");

        let err = RPCError::NegotiationTimeout;
        assert_eq!(err.to_string(), "Negotiation timeout");

        let err = RPCError::HandlerRejected;
        assert_eq!(err.to_string(), "Handler rejected the request");

        let err = RPCError::Disconnected;
        assert_eq!(err.to_string(), "Gracefully Disconnected");
    }

    #[test]
    fn rpc_error_display_with_data() {
        let err = RPCError::IoError("connection reset".to_string());
        assert!(err.to_string().contains("connection reset"));

        let err = RPCError::InvalidData("bad payload".to_string());
        assert!(err.to_string().contains("bad payload"));

        let err = RPCError::InternalError("timer failure");
        assert!(err.to_string().contains("timer failure"));

        let err = RPCError::ErrorResponse(RpcErrorResponse::ServerError, "overloaded".to_string());
        assert!(err.to_string().contains("overloaded"));
    }

    #[test]
    fn rpc_error_from_io_error() {
        let io_err = io::Error::new(io::ErrorKind::ConnectionReset, "reset");
        let rpc_err: RPCError = io_err.into();
        match rpc_err {
            RPCError::IoError(msg) => assert!(msg.contains("reset")),
            other => panic!("expected IoError, got {:?}", other),
        }
    }

    #[test]
    fn rpc_error_from_ssz_decode_error() {
        let ssz_err = ssz::DecodeError::InvalidByteLength {
            len: 5,
            expected: 8,
        };
        let rpc_err: RPCError = ssz_err.clone().into();
        assert_eq!(rpc_err, RPCError::SSZDecodeError(ssz_err));
    }

    #[test]
    fn rpc_error_as_static_str_for_error_response() {
        let err = RPCError::ErrorResponse(RpcErrorResponse::InvalidRequest, "bad".to_string());
        // as_static_str delegates to the RpcErrorResponse code
        let s = err.as_static_str();
        assert!(!s.is_empty());
    }

    #[test]
    fn rpc_error_as_static_str_for_non_error_response() {
        let err = RPCError::StreamTimeout;
        let s = err.as_static_str();
        assert_eq!(s, "stream_timeout");
    }

    #[test]
    fn rpc_error_strum_into_static_str() {
        let err = RPCError::Disconnected;
        let s: &'static str = (&err).into();
        assert_eq!(s, "disconnected");
    }

    // ── RequestType ─────────────────────────────────────────────

    #[test]
    fn request_type_expect_exactly_one_response() {
        // Single-response requests
        assert!(
            RequestType::<E>::Status(StatusMessage::V1(StatusMessageV1 {
                fork_digest: [0; 4],
                finalized_root: Hash256::zero(),
                finalized_epoch: Epoch::new(0),
                head_root: Hash256::zero(),
                head_slot: Slot::new(0),
            }))
            .expect_exactly_one_response()
        );

        assert!(RequestType::<E>::Ping(Ping { data: 0 }).expect_exactly_one_response());

        assert!(
            RequestType::<E>::MetaData(MetadataRequest::new_v1()).expect_exactly_one_response()
        );

        assert!(RequestType::<E>::LightClientOptimisticUpdate.expect_exactly_one_response());
        assert!(RequestType::<E>::LightClientFinalityUpdate.expect_exactly_one_response());

        // Multi-response requests
        assert!(
            !RequestType::<E>::BlocksByRange(OldBlocksByRangeRequest::V2(
                OldBlocksByRangeRequestV2 {
                    start_slot: 0,
                    count: 10,
                    step: 1,
                }
            ))
            .expect_exactly_one_response()
        );

        assert!(
            !RequestType::<E>::Goodbye(GoodbyeReason::ClientShutdown).expect_exactly_one_response()
        );
    }

    #[test]
    fn request_type_versioned_protocol_mapping() {
        assert_eq!(
            RequestType::<E>::Ping(Ping { data: 1 }).versioned_protocol(),
            SupportedProtocol::PingV1
        );
        assert_eq!(
            RequestType::<E>::Goodbye(GoodbyeReason::ClientShutdown).versioned_protocol(),
            SupportedProtocol::GoodbyeV1
        );
        assert_eq!(
            RequestType::<E>::LightClientOptimisticUpdate.versioned_protocol(),
            SupportedProtocol::LightClientOptimisticUpdateV1
        );
        assert_eq!(
            RequestType::<E>::LightClientFinalityUpdate.versioned_protocol(),
            SupportedProtocol::LightClientFinalityUpdateV1
        );
    }

    #[test]
    fn request_type_status_v1_v2_versioned_protocol() {
        let v1 = RequestType::<E>::Status(StatusMessage::V1(StatusMessageV1 {
            fork_digest: [0; 4],
            finalized_root: Hash256::zero(),
            finalized_epoch: Epoch::new(0),
            head_root: Hash256::zero(),
            head_slot: Slot::new(0),
        }));
        assert_eq!(v1.versioned_protocol(), SupportedProtocol::StatusV1);
    }

    #[test]
    fn request_type_metadata_v1_v2_v3_versioned_protocol() {
        assert_eq!(
            RequestType::<E>::MetaData(MetadataRequest::new_v1()).versioned_protocol(),
            SupportedProtocol::MetaDataV1
        );
        assert_eq!(
            RequestType::<E>::MetaData(MetadataRequest::new_v2()).versioned_protocol(),
            SupportedProtocol::MetaDataV2
        );
        assert_eq!(
            RequestType::<E>::MetaData(MetadataRequest::new_v3()).versioned_protocol(),
            SupportedProtocol::MetaDataV3
        );
    }

    #[test]
    fn request_type_max_responses_single() {
        let spec = E::default_spec();
        let epoch = Epoch::new(0);
        let status = RequestType::<E>::Status(StatusMessage::V1(StatusMessageV1 {
            fork_digest: [0; 4],
            finalized_root: Hash256::zero(),
            finalized_epoch: Epoch::new(0),
            head_root: Hash256::zero(),
            head_slot: Slot::new(0),
        }));
        assert_eq!(status.max_responses(epoch, &spec), 1);

        let ping = RequestType::<E>::Ping(Ping { data: 42 });
        assert_eq!(ping.max_responses(epoch, &spec), 1);

        let metadata = RequestType::<E>::MetaData(MetadataRequest::new_v1());
        assert_eq!(metadata.max_responses(epoch, &spec), 1);
    }

    #[test]
    fn request_type_max_responses_goodbye_is_zero() {
        let spec = E::default_spec();
        let goodbye = RequestType::<E>::Goodbye(GoodbyeReason::ClientShutdown);
        assert_eq!(goodbye.max_responses(Epoch::new(0), &spec), 0);
    }

    #[test]
    fn request_type_max_responses_blocks_by_range() {
        let spec = E::default_spec();
        let req = RequestType::<E>::BlocksByRange(OldBlocksByRangeRequest::V2(
            OldBlocksByRangeRequestV2 {
                start_slot: 0,
                count: 64,
                step: 1,
            },
        ));
        assert_eq!(req.max_responses(Epoch::new(0), &spec), 64);
    }

    #[test]
    fn request_type_supported_protocols_status_has_v1_and_v2() {
        let req = RequestType::<E>::Status(StatusMessage::V1(StatusMessageV1 {
            fork_digest: [0; 4],
            finalized_root: Hash256::zero(),
            finalized_epoch: Epoch::new(0),
            head_root: Hash256::zero(),
            head_slot: Slot::new(0),
        }));
        let protos = req.supported_protocols();
        assert_eq!(protos.len(), 2);
        assert_eq!(protos[0].versioned_protocol, SupportedProtocol::StatusV2);
        assert_eq!(protos[1].versioned_protocol, SupportedProtocol::StatusV1);
    }

    #[test]
    fn request_type_supported_protocols_metadata_has_all_versions() {
        let req = RequestType::<E>::MetaData(MetadataRequest::new_v1());
        let protos = req.supported_protocols();
        assert_eq!(protos.len(), 3);
    }

    #[test]
    fn request_type_display_formatting() {
        let ping = RequestType::<E>::Ping(Ping { data: 42 });
        assert!(ping.to_string().contains("42"));

        let metadata = RequestType::<E>::MetaData(MetadataRequest::new_v1());
        assert!(metadata.to_string().contains("MetaData"));

        let lc_opt = RequestType::<E>::LightClientOptimisticUpdate;
        assert!(lc_opt.to_string().contains("optimistic"));

        let lc_fin = RequestType::<E>::LightClientFinalityUpdate;
        assert!(lc_fin.to_string().contains("finality"));
    }

    // ── Static size constants ───────────────────────────────────

    #[test]
    fn static_block_sizes_are_positive() {
        assert!(*SIGNED_BEACON_BLOCK_BASE_MIN > 0);
        assert!(*SIGNED_BEACON_BLOCK_BASE_MAX > 0);
        assert!(*SIGNED_BEACON_BLOCK_ALTAIR_MAX > 0);
        assert!(*SIGNED_BEACON_BLOCK_BELLATRIX_MAX > 0);
    }

    #[test]
    fn static_block_sizes_monotonic() {
        assert!(*SIGNED_BEACON_BLOCK_BASE_MIN <= *SIGNED_BEACON_BLOCK_BASE_MAX);
        assert!(*SIGNED_BEACON_BLOCK_BASE_MAX <= *SIGNED_BEACON_BLOCK_ALTAIR_MAX);
        assert!(*SIGNED_BEACON_BLOCK_ALTAIR_MAX <= *SIGNED_BEACON_BLOCK_BELLATRIX_MAX);
    }

    #[test]
    fn blob_sidecar_size_positive() {
        assert!(*BLOB_SIDECAR_SIZE > 0);
        assert!(*BLOB_SIDECAR_SIZE_MINIMAL > 0);
    }

    #[test]
    fn error_type_min_max() {
        assert!(*ERROR_TYPE_MIN <= *ERROR_TYPE_MAX);
        assert!(*ERROR_TYPE_MAX > 0);
    }

    #[test]
    fn envelope_max_equals_bellatrix_max() {
        assert_eq!(
            *SIGNED_EXECUTION_PAYLOAD_ENVELOPE_MAX,
            *SIGNED_BEACON_BLOCK_BELLATRIX_MAX
        );
    }

    // ── rpc_blob_limits ─────────────────────────────────────────

    #[test]
    fn blob_limits_minimal_spec() {
        let limits = rpc_blob_limits::<E>();
        assert_eq!(limits.min, *BLOB_SIDECAR_SIZE_MINIMAL);
        assert_eq!(limits.max, *BLOB_SIDECAR_SIZE_MINIMAL);
    }

    #[test]
    fn blob_limits_mainnet_spec() {
        let limits = rpc_blob_limits::<MainnetEthSpec>();
        assert_eq!(limits.min, *BLOB_SIDECAR_SIZE);
        assert_eq!(limits.max, *BLOB_SIDECAR_SIZE);
    }
}
