use serde::Serialize;
use types::EthSpec;
use vibehouse_network::PeerInfo;

/// Information returned by `peers` and `connected_peers`.
#[derive(Debug, Clone, Serialize)]
#[serde(bound = "E: EthSpec")]
pub(crate) struct Peer<E: EthSpec> {
    /// The Peer's ID
    pub peer_id: String,
    /// The PeerInfo associated with the peer.
    pub peer_info: PeerInfo<E>,
}
