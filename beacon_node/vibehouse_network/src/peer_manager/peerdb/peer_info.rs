use super::client::Client;
use super::score::{PeerAction, Score, ScoreState};
use super::sync_status::SyncStatus;
use crate::discovery::Eth2Enr;
use crate::{rpc::MetaData, types::Subnet};
use PeerConnectionStatus::*;
use discv5::Enr;
use eth2::types::{PeerDirection, PeerState};
use libp2p::core::multiaddr::{Multiaddr, Protocol};
use serde::{
    Serialize,
    ser::{SerializeStruct, Serializer},
};
use std::collections::HashSet;
use std::net::IpAddr;
use std::time::Instant;
use strum::AsRefStr;
use types::{DataColumnSubnetId, EthSpec};

/// Information about a given connected peer.
#[derive(Clone, Debug, Serialize)]
#[serde(bound = "E: EthSpec")]
pub struct PeerInfo<E: EthSpec> {
    /// The peers reputation
    pub(crate) score: Score,
    /// Client managing this peer
    client: Client,
    /// Connection status of this peer
    connection_status: PeerConnectionStatus,
    /// The known listening addresses of this peer. This is given by identify and can be arbitrary
    /// (including local IPs).
    listening_addresses: Vec<Multiaddr>,
    /// These are the multiaddrs we have physically seen and is what we use for banning/un-banning
    /// peers.
    seen_multiaddrs: HashSet<Multiaddr>,
    /// The current syncing state of the peer. The state may be determined after it's initial
    /// connection.
    sync_status: SyncStatus,
    /// The ENR subnet bitfield of the peer. This may be determined after it's initial
    /// connection.
    meta_data: Option<MetaData<E>>,
    /// Subnets the peer is connected to.
    subnets: HashSet<Subnet>,
    /// This is computed from either metadata or the ENR, and contains the subnets that the peer
    /// is *assigned* to custody, rather than *connected* to (different to `self.subnets`).
    /// Note: Another reason to keep this separate to `self.subnets` is an upcoming change to
    /// decouple custody requirements from the actual subnets, i.e. changing this to `custody_groups`.
    custody_subnets: HashSet<DataColumnSubnetId>,
    /// The time we would like to retain this peer. After this time, the peer is no longer
    /// necessary.
    #[serde(skip)]
    min_ttl: Option<Instant>,
    /// Is the peer a trusted peer.
    pub(crate) is_trusted: bool,
    /// Direction of the first connection of the last (or current) connected session with this peer.
    /// None if this peer was never connected.
    connection_direction: Option<ConnectionDirection>,
    /// The enr of the peer, if known.
    enr: Option<Enr>,
}

impl<E: EthSpec> Default for PeerInfo<E> {
    fn default() -> PeerInfo<E> {
        PeerInfo {
            score: Score::default(),
            client: Client::default(),
            connection_status: Default::default(),
            listening_addresses: Vec::new(),
            seen_multiaddrs: HashSet::new(),
            subnets: HashSet::new(),
            custody_subnets: HashSet::new(),
            sync_status: SyncStatus::Unknown,
            meta_data: None,
            min_ttl: None,
            is_trusted: false,
            connection_direction: None,
            enr: None,
        }
    }
}

impl<E: EthSpec> PeerInfo<E> {
    /// Return a PeerInfo struct for a trusted peer.
    pub fn trusted_peer_info() -> Self {
        PeerInfo {
            score: Score::max_score(),
            is_trusted: true,
            ..Default::default()
        }
    }

    /// Returns if the peer is subscribed to a given `Subnet` from the metadata attnets/syncnets field.
    /// Also returns true if the peer is assigned to custody a given data column `Subnet` computed from the metadata `custody_group_count` field or ENR `cgc` field.
    pub fn on_subnet_metadata(&self, subnet: &Subnet) -> bool {
        if let Some(meta_data) = &self.meta_data {
            match subnet {
                Subnet::Attestation(id) => {
                    return meta_data.attnets().get(**id as usize).unwrap_or(false);
                }
                Subnet::SyncCommittee(id) => {
                    return meta_data
                        .syncnets()
                        .is_ok_and(|s| s.get(**id as usize).unwrap_or(false));
                }
                Subnet::DataColumn(subnet_id) => {
                    return self.is_assigned_to_custody_subnet(subnet_id);
                }
                // Execution proof subnets are not tracked in metadata.
                Subnet::ExecutionProof(_) => return false,
            }
        }
        false
    }

    /// Obtains the client of the peer.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Returns the listening addresses of the Peer.
    pub fn listening_addresses(&self) -> &Vec<Multiaddr> {
        &self.listening_addresses
    }

    /// Returns the connection direction for the peer.
    pub fn connection_direction(&self) -> Option<&ConnectionDirection> {
        self.connection_direction.as_ref()
    }

    /// Returns true if this is an incoming ipv4 connection.
    pub fn is_incoming_ipv4_connection(&self) -> bool {
        self.seen_multiaddrs.iter().any(|multiaddr| {
            multiaddr
                .iter()
                .any(|protocol| matches!(protocol, libp2p::core::multiaddr::Protocol::Ip4(_)))
        })
    }

    /// Returns true if this is an incoming ipv6 connection.
    pub fn is_incoming_ipv6_connection(&self) -> bool {
        self.seen_multiaddrs.iter().any(|multiaddr| {
            multiaddr
                .iter()
                .any(|protocol| matches!(protocol, libp2p::core::multiaddr::Protocol::Ip6(_)))
        })
    }

    /// Returns the sync status of the peer.
    pub fn sync_status(&self) -> &SyncStatus {
        &self.sync_status
    }

    /// Returns the metadata for the peer if currently known.
    pub fn meta_data(&self) -> Option<&MetaData<E>> {
        self.meta_data.as_ref()
    }

    /// Returns whether the peer is a trusted peer or not.
    pub fn is_trusted(&self) -> bool {
        self.is_trusted
    }

    /// The time a peer is expected to be useful until for an attached validator. If this is set to
    /// None, the peer is not required for any upcoming duty.
    pub fn min_ttl(&self) -> Option<&Instant> {
        self.min_ttl.as_ref()
    }

    /// The ENR of the peer if it is known.
    pub fn enr(&self) -> Option<&Enr> {
        self.enr.as_ref()
    }

    /// An iterator over all the subnets this peer is subscribed to.
    pub fn subnets(&self) -> impl Iterator<Item = &Subnet> {
        self.subnets.iter()
    }

    /// Returns an iterator over the long-lived subnets if it has any.
    pub fn long_lived_subnets(&self) -> Vec<Subnet> {
        let mut long_lived_subnets = Vec::new();
        // Check the meta_data
        if let Some(meta_data) = self.meta_data.as_ref() {
            for subnet in 0..=meta_data.attnets().highest_set_bit().unwrap_or(0) {
                if meta_data.attnets().get(subnet).unwrap_or(false) {
                    long_lived_subnets.push(Subnet::Attestation((subnet as u64).into()));
                }
            }

            if let Ok(syncnet) = meta_data.syncnets() {
                for subnet in 0..=syncnet.highest_set_bit().unwrap_or(0) {
                    if syncnet.get(subnet).unwrap_or(false) {
                        long_lived_subnets.push(Subnet::SyncCommittee((subnet as u64).into()));
                    }
                }
            }
        } else if let Some(enr) = self.enr.as_ref() {
            if let Ok(attnets) = enr.attestation_bitfield::<E>() {
                for subnet in 0..=attnets.highest_set_bit().unwrap_or(0) {
                    if attnets.get(subnet).unwrap_or(false) {
                        long_lived_subnets.push(Subnet::Attestation((subnet as u64).into()));
                    }
                }
            }

            if let Ok(syncnets) = enr.sync_committee_bitfield::<E>() {
                for subnet in 0..=syncnets.highest_set_bit().unwrap_or(0) {
                    if syncnets.get(subnet).unwrap_or(false) {
                        long_lived_subnets.push(Subnet::SyncCommittee((subnet as u64).into()));
                    }
                }
            }
        }

        long_lived_subnets.extend(
            self.custody_subnets
                .iter()
                .map(|&id| Subnet::DataColumn(id)),
        );

        long_lived_subnets
    }

    /// Returns if the peer is subscribed to a given `Subnet` from the gossipsub subscriptions.
    pub fn on_subnet_gossipsub(&self, subnet: &Subnet) -> bool {
        self.subnets.contains(subnet)
    }

    /// Returns if the peer is assigned to a given `DataColumnSubnetId`.
    pub fn is_assigned_to_custody_subnet(&self, subnet: &DataColumnSubnetId) -> bool {
        self.custody_subnets.contains(subnet)
    }

    /// Returns an iterator on this peer's custody subnets
    pub fn custody_subnets_iter(&self) -> impl Iterator<Item = &DataColumnSubnetId> {
        self.custody_subnets.iter()
    }

    /// Returns the number of custody subnets this peer is assigned to.
    pub fn custody_subnet_count(&self) -> usize {
        self.custody_subnets.len()
    }

    /// Returns true if the peer is connected to a long-lived subnet.
    pub fn has_long_lived_subnet(&self) -> bool {
        // Check the meta_data
        if let Some(meta_data) = self.meta_data.as_ref() {
            if !meta_data.attnets().is_zero() && !self.subnets.is_empty() {
                return true;
            }
            if let Ok(sync) = meta_data.syncnets()
                && !sync.is_zero()
            {
                return true;
            }
        }

        // We may not have the metadata but may have an ENR. Lets check that
        if let Some(enr) = self.enr.as_ref()
            && let Ok(attnets) = enr.attestation_bitfield::<E>()
            && !attnets.is_zero()
            && !self.subnets.is_empty()
        {
            return true;
        }

        // Check if the peer has custody subnets populated and the peer is subscribed to any of
        // its custody subnets
        let subscribed_to_any_custody_subnets = self
            .custody_subnets
            .iter()
            .any(|subnet_id| self.subnets.contains(&Subnet::DataColumn(*subnet_id)));
        if subscribed_to_any_custody_subnets {
            return true;
        }

        false
    }

    /// Returns the seen addresses of the peer.
    pub fn seen_multiaddrs(&self) -> impl Iterator<Item = &Multiaddr> + '_ {
        self.seen_multiaddrs.iter()
    }

    /// Returns a list of seen IP addresses for the peer.
    pub fn seen_ip_addresses(&self) -> impl Iterator<Item = IpAddr> + '_ {
        self.seen_multiaddrs.iter().filter_map(|multiaddr| {
            multiaddr.iter().find_map(|protocol| {
                match protocol {
                    Protocol::Ip4(ip) => Some(ip.into()),
                    Protocol::Ip6(ip) => Some(ip.into()),
                    _ => None, // Only care for IP addresses
                }
            })
        })
    }

    /// Returns the connection status of the peer.
    pub fn connection_status(&self) -> &PeerConnectionStatus {
        &self.connection_status
    }

    /// Reports if this peer has some future validator duty in which case it is valuable to keep it.
    pub fn has_future_duty(&self) -> bool {
        self.min_ttl.is_some_and(|i| i >= Instant::now())
    }

    /// Returns score of the peer.
    pub fn score(&self) -> &Score {
        &self.score
    }

    /// Returns the state of the peer based on the score.
    pub(crate) fn score_state(&self) -> ScoreState {
        self.score.state()
    }

    /// Returns true if the gossipsub score is sufficient.
    pub fn is_good_gossipsub_peer(&self) -> bool {
        self.score.is_good_gossipsub_peer()
    }

    /* Peer connection status API */

    /// Checks if the status is connected.
    pub fn is_connected(&self) -> bool {
        matches!(
            self.connection_status,
            PeerConnectionStatus::Connected { .. }
        )
    }

    /// Checks if the peer is synced or advanced.
    pub fn is_synced_or_advanced(&self) -> bool {
        matches!(
            self.sync_status,
            SyncStatus::Synced { .. } | SyncStatus::Advanced { .. }
        )
    }

    /// Checks if the status is connected.
    pub fn is_dialing(&self) -> bool {
        matches!(self.connection_status, PeerConnectionStatus::Dialing { .. })
    }

    /// The peer is either connected or in the process of being dialed.
    pub fn is_connected_or_dialing(&self) -> bool {
        self.is_connected() || self.is_dialing()
    }

    /// Checks if the connection status is banned. This can lag behind the score state
    /// temporarily.
    pub fn is_banned(&self) -> bool {
        matches!(self.connection_status, PeerConnectionStatus::Banned { .. })
    }

    /// Checks if the peer's score is banned.
    pub fn score_is_banned(&self) -> bool {
        matches!(self.score.state(), ScoreState::Banned)
    }

    /// Checks if the status is disconnected.
    pub fn is_disconnected(&self) -> bool {
        matches!(self.connection_status, Disconnected { .. })
    }

    /// Checks if the peer is outbound-only
    pub fn is_outbound_only(&self) -> bool {
        matches!(self.connection_status, Connected {n_in, n_out, ..} if n_in == 0 && n_out > 0)
    }

    /// Returns the number of connections with this peer.
    pub fn connections(&self) -> (u8, u8) {
        match self.connection_status {
            Connected { n_in, n_out, .. } => (n_in, n_out),
            _ => (0, 0),
        }
    }

    /* Mutable Functions */

    /// Updates the sync status. Returns true if the status was changed.
    // VISIBILITY: Both the peer manager the network sync is able to update the sync state of a peer
    pub fn update_sync_status(&mut self, sync_status: SyncStatus) -> bool {
        self.sync_status.update(sync_status)
    }

    /// Sets the client of the peer.
    // VISIBILITY: The peer manager is able to set the client
    pub(in crate::peer_manager) fn set_client(&mut self, client: Client) {
        self.client = client
    }

    /// Replaces the current listening addresses with those specified, returning the current
    /// listening addresses.
    // VISIBILITY: The peer manager is able to set the listening addresses
    pub(in crate::peer_manager) fn set_listening_addresses(
        &mut self,
        listening_addresses: Vec<Multiaddr>,
    ) -> Vec<Multiaddr> {
        std::mem::replace(&mut self.listening_addresses, listening_addresses)
    }

    /// Sets an explicit value for the meta data.
    // VISIBILITY: The peer manager is able to adjust the meta_data
    pub(in crate::peer_manager) fn set_meta_data(&mut self, meta_data: MetaData<E>) {
        self.meta_data = Some(meta_data);
    }

    /// Sets the connection status of the peer.
    pub(super) fn set_connection_status(&mut self, connection_status: PeerConnectionStatus) {
        self.connection_status = connection_status
    }

    pub(in crate::peer_manager) fn set_custody_subnets(
        &mut self,
        custody_subnets: HashSet<DataColumnSubnetId>,
    ) {
        self.custody_subnets = custody_subnets
    }

    /// Sets the ENR of the peer if one is known.
    pub(super) fn set_enr(&mut self, enr: Enr) {
        self.enr = Some(enr)
    }

    /// Sets the time that the peer is expected to be needed until for an attached validator duty.
    pub(super) fn set_min_ttl(&mut self, min_ttl: Instant) {
        self.min_ttl = Some(min_ttl)
    }

    /// Adds a known subnet for the peer.
    pub(super) fn insert_subnet(&mut self, subnet: Subnet) {
        self.subnets.insert(subnet);
    }

    /// Removes a subnet from the peer.
    pub(super) fn remove_subnet(&mut self, subnet: &Subnet) {
        self.subnets.remove(subnet);
    }

    /// Removes all subnets from the peer.
    pub(super) fn clear_subnets(&mut self) {
        self.subnets.clear()
    }

    /// Applies decay rates to a non-trusted peer's score.
    pub(super) fn score_update(&mut self) {
        if !self.is_trusted {
            self.score.update()
        }
    }

    /// Apply peer action to a non-trusted peer's score.
    // VISIBILITY: The peer manager is able to modify the score of a peer.
    pub(in crate::peer_manager) fn apply_peer_action_to_score(&mut self, peer_action: PeerAction) {
        if !self.is_trusted {
            self.score.apply_peer_action(peer_action)
        }
    }

    /// Updates the gossipsub score with a new score. Optionally ignore the gossipsub score.
    pub(super) fn update_gossipsub_score(&mut self, new_score: f64, ignore: bool) {
        self.score.update_gossipsub_score(new_score, ignore);
    }

    #[cfg(test)]
    /// Resets the peers score.
    pub fn reset_score(&mut self) {
        self.score.test_reset();
    }

    /// Modifies the status to Dialing
    /// Returns an error if the current state is unexpected.
    pub(super) fn set_dialing_peer(&mut self) -> Result<(), &'static str> {
        match &mut self.connection_status {
            Connected { .. } => return Err("Dialing connected peer"),
            Dialing { .. } => return Err("Dialing an already dialing peer"),
            Disconnecting { .. } => return Err("Dialing a disconnecting peer"),
            Disconnected { .. } | Banned { .. } | Unknown => {}
        }
        self.connection_status = Dialing {
            since: Instant::now(),
        };
        Ok(())
    }

    /// Modifies the status to Connected and increases the number of ingoing
    /// connections by one
    pub(super) fn connect_ingoing(&mut self, multiaddr: Multiaddr) {
        self.seen_multiaddrs.insert(multiaddr.clone());

        match &mut self.connection_status {
            Connected { n_in, .. } => *n_in += 1,
            Disconnected { .. }
            | Banned { .. }
            | Dialing { .. }
            | Disconnecting { .. }
            | Unknown => {
                self.connection_status = Connected {
                    n_in: 1,
                    n_out: 0,
                    multiaddr,
                };
                self.connection_direction = Some(ConnectionDirection::Incoming);
            }
        }
    }

    /// Modifies the status to Connected and increases the number of outgoing
    /// connections by one
    pub(super) fn connect_outgoing(&mut self, multiaddr: Multiaddr) {
        self.seen_multiaddrs.insert(multiaddr.clone());
        match &mut self.connection_status {
            Connected { n_out, .. } => *n_out += 1,
            Disconnected { .. }
            | Banned { .. }
            | Dialing { .. }
            | Disconnecting { .. }
            | Unknown => {
                self.connection_status = Connected {
                    n_in: 0,
                    n_out: 1,
                    multiaddr,
                };
                self.connection_direction = Some(ConnectionDirection::Outgoing);
            }
        }
    }

    #[cfg(test)]
    /// Add an f64 to a non-trusted peer's score abiding by the limits.
    pub fn add_to_score(&mut self, score: f64) {
        if !self.is_trusted {
            self.score.test_add(score)
        }
    }

    #[cfg(test)]
    pub fn set_gossipsub_score(&mut self, score: f64) {
        self.score.set_gossipsub_score(score);
    }
}

/// Connection Direction of connection.
#[derive(Debug, Clone, Copy, Serialize, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum ConnectionDirection {
    /// The connection was established by a peer dialing us.
    Incoming,
    /// The connection was established by us dialing a peer.
    Outgoing,
}

impl From<ConnectionDirection> for PeerDirection {
    fn from(direction: ConnectionDirection) -> Self {
        match direction {
            ConnectionDirection::Incoming => PeerDirection::Inbound,
            ConnectionDirection::Outgoing => PeerDirection::Outbound,
        }
    }
}

/// Connection Status of the peer.
#[derive(Debug, Clone, Default)]
pub enum PeerConnectionStatus {
    /// The peer is connected.
    Connected {
        /// The multiaddr that we are connected via.
        multiaddr: Multiaddr,
        /// number of ingoing connections.
        n_in: u8,
        /// number of outgoing connections.
        n_out: u8,
    },
    /// The peer is being disconnected.
    Disconnecting {
        // After the disconnection the peer will be considered banned.
        to_ban: bool,
    },
    /// The peer has disconnected.
    Disconnected {
        /// last time the peer was connected or discovered.
        since: Instant,
    },
    /// The peer has been banned and is disconnected.
    Banned {
        /// moment when the peer was banned.
        since: Instant,
    },
    /// We are currently dialing this peer.
    Dialing {
        /// time since we last communicated with the peer.
        since: Instant,
    },
    /// The connection status has not been specified.
    #[default]
    Unknown,
}

/// Serialization for http requests.
impl Serialize for PeerConnectionStatus {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("connection_status", 6)?;
        match self {
            Connected {
                n_in,
                n_out,
                multiaddr,
            } => {
                s.serialize_field("multiaddr", multiaddr)?;
                s.serialize_field("status", "connected")?;
                s.serialize_field("connections_in", n_in)?;
                s.serialize_field("connections_out", n_out)?;
                s.serialize_field("last_seen", &0)?;
                s.end()
            }
            Disconnecting { .. } => {
                s.serialize_field("status", "disconnecting")?;
                s.serialize_field("connections_in", &0)?;
                s.serialize_field("connections_out", &0)?;
                s.serialize_field("last_seen", &0)?;
                s.end()
            }
            Disconnected { since } => {
                s.serialize_field("status", "disconnected")?;
                s.serialize_field("connections_in", &0)?;
                s.serialize_field("connections_out", &0)?;
                s.serialize_field("last_seen", &since.elapsed().as_secs())?;
                s.serialize_field("banned_ips", &Vec::<IpAddr>::new())?;
                s.end()
            }
            Banned { since } => {
                s.serialize_field("status", "banned")?;
                s.serialize_field("connections_in", &0)?;
                s.serialize_field("connections_out", &0)?;
                s.serialize_field("last_seen", &since.elapsed().as_secs())?;
                s.end()
            }
            Dialing { since } => {
                s.serialize_field("status", "dialing")?;
                s.serialize_field("connections_in", &0)?;
                s.serialize_field("connections_out", &0)?;
                s.serialize_field("last_seen", &since.elapsed().as_secs())?;
                s.end()
            }
            Unknown => {
                s.serialize_field("status", "unknown")?;
                s.serialize_field("connections_in", &0)?;
                s.serialize_field("connections_out", &0)?;
                s.serialize_field("last_seen", &0)?;
                s.end()
            }
        }
    }
}

impl From<PeerConnectionStatus> for PeerState {
    fn from(status: PeerConnectionStatus) -> Self {
        match status {
            Connected { .. } => PeerState::Connected,
            Dialing { .. } => PeerState::Connecting,
            Disconnecting { .. } => PeerState::Disconnecting,
            Disconnected { .. } | Banned { .. } | Unknown => PeerState::Disconnected,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Subnet;
    use types::{DataColumnSubnetId, MainnetEthSpec};

    type E = MainnetEthSpec;

    fn create_test_peer_info() -> PeerInfo<E> {
        PeerInfo::default()
    }

    #[test]
    fn test_has_long_lived_subnet_empty_custody_subnets() {
        let peer_info = create_test_peer_info();
        // peer has no custody subnets or subscribed to any subnets hence return false
        assert!(!peer_info.has_long_lived_subnet());
    }

    #[test]
    fn test_has_long_lived_subnet_empty_subnets_with_custody_subnets() {
        let mut peer_info = create_test_peer_info();
        peer_info.custody_subnets.insert(DataColumnSubnetId::new(1));
        peer_info.custody_subnets.insert(DataColumnSubnetId::new(2));
        // Peer has custody subnets but isn't subscribed to any hence return false
        assert!(!peer_info.has_long_lived_subnet());
    }

    #[test]
    fn test_has_long_lived_subnet_subscribed_to_custody_subnets() {
        let mut peer_info = create_test_peer_info();
        peer_info.custody_subnets.insert(DataColumnSubnetId::new(1));
        peer_info.custody_subnets.insert(DataColumnSubnetId::new(2));
        peer_info.custody_subnets.insert(DataColumnSubnetId::new(3));

        peer_info
            .subnets
            .insert(Subnet::DataColumn(DataColumnSubnetId::new(1)));
        peer_info
            .subnets
            .insert(Subnet::DataColumn(DataColumnSubnetId::new(2)));
        // Missing DataColumnSubnetId::new(3) - but peer is subscribed to some custody subnets
        // Peer is subscribed to any custody subnets - return true
        assert!(peer_info.has_long_lived_subnet());
    }

    // --- Default and trusted peer ---

    #[test]
    fn default_peer_info() {
        let peer: PeerInfo<E> = PeerInfo::default();
        assert!(!peer.is_trusted());
        assert!(!peer.is_connected());
        assert!(!peer.is_dialing());
        assert!(!peer.is_banned());
        assert!(!peer.is_connected_or_dialing());
        assert!(peer.connection_direction().is_none());
        assert!(peer.enr().is_none());
        assert!(peer.meta_data().is_none());
        assert!(peer.min_ttl().is_none());
        assert_eq!(peer.custody_subnet_count(), 0);
    }

    #[test]
    fn trusted_peer_has_max_score() {
        let peer: PeerInfo<E> = PeerInfo::trusted_peer_info();
        assert!(peer.is_trusted());
        assert!(peer.score().score().is_infinite());
    }

    #[test]
    fn trusted_peer_score_update_is_noop() {
        let mut peer: PeerInfo<E> = PeerInfo::trusted_peer_info();
        let before = peer.score().score();
        peer.score_update();
        assert_eq!(peer.score().score(), before);
    }

    #[test]
    fn trusted_peer_action_is_noop() {
        let mut peer: PeerInfo<E> = PeerInfo::trusted_peer_info();
        peer.apply_peer_action_to_score(PeerAction::Fatal);
        assert!(peer.score().score().is_infinite());
    }

    // --- Connection status ---

    #[test]
    fn connect_ingoing_sets_connected() {
        let mut peer = create_test_peer_info();
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/9000".parse().unwrap();
        peer.connect_ingoing(addr);
        assert!(peer.is_connected());
        assert!(!peer.is_outbound_only());
        let (n_in, n_out) = peer.connections();
        assert_eq!(n_in, 1);
        assert_eq!(n_out, 0);
        assert!(matches!(
            peer.connection_direction(),
            Some(ConnectionDirection::Incoming)
        ));
    }

    #[test]
    fn connect_outgoing_sets_connected() {
        let mut peer = create_test_peer_info();
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/9000".parse().unwrap();
        peer.connect_outgoing(addr);
        assert!(peer.is_connected());
        assert!(peer.is_outbound_only());
        let (n_in, n_out) = peer.connections();
        assert_eq!(n_in, 0);
        assert_eq!(n_out, 1);
        assert!(matches!(
            peer.connection_direction(),
            Some(ConnectionDirection::Outgoing)
        ));
    }

    #[test]
    fn multiple_ingoing_connections_increment() {
        let mut peer = create_test_peer_info();
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/9000".parse().unwrap();
        peer.connect_ingoing(addr.clone());
        peer.connect_ingoing(addr);
        let (n_in, n_out) = peer.connections();
        assert_eq!(n_in, 2);
        assert_eq!(n_out, 0);
    }

    #[test]
    fn multiple_outgoing_connections_increment() {
        let mut peer = create_test_peer_info();
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/9000".parse().unwrap();
        peer.connect_outgoing(addr.clone());
        peer.connect_outgoing(addr);
        let (n_in, n_out) = peer.connections();
        assert_eq!(n_in, 0);
        assert_eq!(n_out, 2);
    }

    #[test]
    fn mixed_connections_not_outbound_only() {
        let mut peer = create_test_peer_info();
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/9000".parse().unwrap();
        peer.connect_outgoing(addr.clone());
        peer.connect_ingoing(addr);
        assert!(!peer.is_outbound_only());
    }

    #[test]
    fn disconnected_peer_connections_are_zero() {
        let mut peer = create_test_peer_info();
        peer.set_connection_status(PeerConnectionStatus::Disconnected {
            since: Instant::now(),
        });
        assert!(peer.is_disconnected());
        let (n_in, n_out) = peer.connections();
        assert_eq!(n_in, 0);
        assert_eq!(n_out, 0);
    }

    // --- set_dialing_peer ---

    #[test]
    fn set_dialing_from_unknown() {
        let mut peer = create_test_peer_info();
        assert!(peer.set_dialing_peer().is_ok());
        assert!(peer.is_dialing());
        assert!(peer.is_connected_or_dialing());
    }

    #[test]
    fn set_dialing_from_disconnected() {
        let mut peer = create_test_peer_info();
        peer.set_connection_status(PeerConnectionStatus::Disconnected {
            since: Instant::now(),
        });
        assert!(peer.set_dialing_peer().is_ok());
        assert!(peer.is_dialing());
    }

    #[test]
    fn set_dialing_from_banned() {
        let mut peer = create_test_peer_info();
        peer.set_connection_status(PeerConnectionStatus::Banned {
            since: Instant::now(),
        });
        assert!(peer.set_dialing_peer().is_ok());
    }

    #[test]
    fn set_dialing_from_connected_fails() {
        let mut peer = create_test_peer_info();
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/9000".parse().unwrap();
        peer.connect_ingoing(addr);
        assert!(peer.set_dialing_peer().is_err());
    }

    #[test]
    fn set_dialing_from_dialing_fails() {
        let mut peer = create_test_peer_info();
        peer.set_dialing_peer().unwrap();
        assert!(peer.set_dialing_peer().is_err());
    }

    #[test]
    fn set_dialing_from_disconnecting_fails() {
        let mut peer = create_test_peer_info();
        peer.set_connection_status(PeerConnectionStatus::Disconnecting { to_ban: false });
        assert!(peer.set_dialing_peer().is_err());
    }

    // --- IP address detection ---

    #[test]
    fn is_incoming_ipv4_connection() {
        let mut peer = create_test_peer_info();
        let addr: Multiaddr = "/ip4/192.168.1.1/tcp/9000".parse().unwrap();
        peer.connect_ingoing(addr);
        assert!(peer.is_incoming_ipv4_connection());
        assert!(!peer.is_incoming_ipv6_connection());
    }

    #[test]
    fn is_incoming_ipv6_connection() {
        let mut peer = create_test_peer_info();
        let addr: Multiaddr = "/ip6/::1/tcp/9000".parse().unwrap();
        peer.connect_ingoing(addr);
        assert!(peer.is_incoming_ipv6_connection());
        assert!(!peer.is_incoming_ipv4_connection());
    }

    #[test]
    fn seen_ip_addresses_extracts_ips() {
        let mut peer = create_test_peer_info();
        let addr4: Multiaddr = "/ip4/192.168.1.1/tcp/9000".parse().unwrap();
        let addr6: Multiaddr = "/ip6/::1/tcp/9001".parse().unwrap();
        peer.connect_ingoing(addr4);
        peer.connect_outgoing(addr6);
        let ips: Vec<IpAddr> = peer.seen_ip_addresses().collect();
        assert_eq!(ips.len(), 2);
    }

    #[test]
    fn no_seen_multiaddrs_no_ips() {
        let peer = create_test_peer_info();
        assert_eq!(peer.seen_ip_addresses().count(), 0);
        assert!(!peer.is_incoming_ipv4_connection());
        assert!(!peer.is_incoming_ipv6_connection());
    }

    // --- Sync status ---

    #[test]
    fn update_sync_status_returns_true_on_change() {
        let mut peer = create_test_peer_info();
        let info = super::super::sync_status::SyncInfo {
            head_slot: types::Slot::new(100),
            head_root: types::Hash256::ZERO,
            finalized_epoch: types::Epoch::new(3),
            finalized_root: types::Hash256::ZERO,
            earliest_available_slot: None,
        };
        assert!(peer.update_sync_status(SyncStatus::Synced { info }));
    }

    #[test]
    fn is_synced_or_advanced() {
        let mut peer = create_test_peer_info();
        assert!(!peer.is_synced_or_advanced());

        let info = super::super::sync_status::SyncInfo {
            head_slot: types::Slot::new(100),
            head_root: types::Hash256::ZERO,
            finalized_epoch: types::Epoch::new(3),
            finalized_root: types::Hash256::ZERO,
            earliest_available_slot: None,
        };
        peer.update_sync_status(SyncStatus::Synced { info: info.clone() });
        assert!(peer.is_synced_or_advanced());

        peer.update_sync_status(SyncStatus::Advanced { info });
        assert!(peer.is_synced_or_advanced());
    }

    // --- Subnet operations ---

    #[test]
    fn insert_and_check_subnet_gossipsub() {
        let mut peer = create_test_peer_info();
        let subnet = Subnet::DataColumn(DataColumnSubnetId::new(5));
        assert!(!peer.on_subnet_gossipsub(&subnet));
        peer.insert_subnet(subnet);
        assert!(peer.on_subnet_gossipsub(&subnet));
    }

    #[test]
    fn remove_subnet() {
        let mut peer = create_test_peer_info();
        let subnet = Subnet::DataColumn(DataColumnSubnetId::new(5));
        peer.insert_subnet(subnet);
        peer.remove_subnet(&subnet);
        assert!(!peer.on_subnet_gossipsub(&subnet));
    }

    #[test]
    fn clear_subnets() {
        let mut peer = create_test_peer_info();
        peer.insert_subnet(Subnet::DataColumn(DataColumnSubnetId::new(1)));
        peer.insert_subnet(Subnet::DataColumn(DataColumnSubnetId::new(2)));
        peer.clear_subnets();
        assert_eq!(peer.subnets().count(), 0);
    }

    // --- Custody subnets ---

    #[test]
    fn custody_subnet_operations() {
        let mut peer = create_test_peer_info();
        let mut custody = HashSet::new();
        custody.insert(DataColumnSubnetId::new(1));
        custody.insert(DataColumnSubnetId::new(3));
        peer.set_custody_subnets(custody);
        assert_eq!(peer.custody_subnet_count(), 2);
        assert!(peer.is_assigned_to_custody_subnet(&DataColumnSubnetId::new(1)));
        assert!(!peer.is_assigned_to_custody_subnet(&DataColumnSubnetId::new(2)));
    }

    // --- ConnectionDirection conversion ---

    #[test]
    fn connection_direction_to_peer_direction() {
        assert_eq!(
            PeerDirection::from(ConnectionDirection::Incoming),
            PeerDirection::Inbound
        );
        assert_eq!(
            PeerDirection::from(ConnectionDirection::Outgoing),
            PeerDirection::Outbound
        );
    }

    // --- PeerConnectionStatus to PeerState ---

    #[test]
    fn connection_status_to_peer_state() {
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/9000".parse().unwrap();
        assert_eq!(
            PeerState::from(PeerConnectionStatus::Connected {
                n_in: 1,
                n_out: 0,
                multiaddr: addr
            }),
            PeerState::Connected
        );
        assert_eq!(
            PeerState::from(PeerConnectionStatus::Dialing {
                since: Instant::now()
            }),
            PeerState::Connecting
        );
        assert_eq!(
            PeerState::from(PeerConnectionStatus::Disconnecting { to_ban: false }),
            PeerState::Disconnecting
        );
        assert_eq!(
            PeerState::from(PeerConnectionStatus::Disconnected {
                since: Instant::now()
            }),
            PeerState::Disconnected
        );
        assert_eq!(
            PeerState::from(PeerConnectionStatus::Banned {
                since: Instant::now()
            }),
            PeerState::Disconnected
        );
        assert_eq!(
            PeerState::from(PeerConnectionStatus::Unknown),
            PeerState::Disconnected
        );
    }

    // --- has_future_duty ---

    #[test]
    fn no_future_duty_by_default() {
        let peer = create_test_peer_info();
        assert!(!peer.has_future_duty());
    }

    #[test]
    fn has_future_duty_with_future_ttl() {
        let mut peer = create_test_peer_info();
        peer.set_min_ttl(Instant::now() + std::time::Duration::from_secs(60));
        assert!(peer.has_future_duty());
    }

    // --- Listening addresses ---

    #[test]
    fn set_listening_addresses_returns_old() {
        let mut peer = create_test_peer_info();
        let addr1: Multiaddr = "/ip4/127.0.0.1/tcp/9000".parse().unwrap();
        let addr2: Multiaddr = "/ip4/192.168.1.1/tcp/9000".parse().unwrap();

        let old = peer.set_listening_addresses(vec![addr1.clone()]);
        assert!(old.is_empty());
        assert_eq!(peer.listening_addresses().len(), 1);

        let old = peer.set_listening_addresses(vec![addr2]);
        assert_eq!(old.len(), 1);
        assert_eq!(old[0], addr1);
    }

    // --- ConnectionDirection as_ref ---

    #[test]
    fn connection_direction_as_ref_str() {
        assert_eq!(ConnectionDirection::Incoming.as_ref(), "incoming");
        assert_eq!(ConnectionDirection::Outgoing.as_ref(), "outgoing");
    }

    // --- Score state from peer_info ---

    #[test]
    fn score_state_reflects_score() {
        let mut peer = create_test_peer_info();
        assert_eq!(peer.score_state(), ScoreState::Healthy);
        peer.add_to_score(-60.0);
        assert_eq!(peer.score_state(), ScoreState::Banned);
        assert!(peer.score_is_banned());
    }

    // --- Gossipsub score ---

    #[test]
    fn gossipsub_score_affects_peer() {
        let mut peer = create_test_peer_info();
        assert!(peer.is_good_gossipsub_peer());
        peer.set_gossipsub_score(-10.0);
        assert!(!peer.is_good_gossipsub_peer());
    }
}
