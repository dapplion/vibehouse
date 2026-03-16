//! ENR extension trait to support libp2p integration.

use discv5::enr::{CombinedKey, CombinedPublicKey};
use libp2p_identity::{KeyType, Keypair, PublicKey, ed25519, secp256k1};
use multiaddr::{Multiaddr, PeerId, Protocol};
use tiny_keccak::{Hasher, Keccak};

type Enr = discv5::enr::Enr<CombinedKey>;

pub const QUIC_ENR_KEY: &str = "quic";
pub const QUIC6_ENR_KEY: &str = "quic6";

/// Extend ENR for libp2p types.
pub trait EnrExt {
    /// The libp2p `PeerId` for the record.
    fn peer_id(&self) -> PeerId;

    /// Returns a list of multiaddrs if the ENR has an `ip` and one of \[`tcp`,`udp`,`quic`\] key **or** an `ip6` and one of \[`tcp6`,`udp6`,`quic6`\].
    /// The vector remains empty if these fields are not defined.
    fn multiaddr(&self) -> Vec<Multiaddr>;

    /// Returns a list of multiaddrs with the `PeerId` prepended.
    fn multiaddr_p2p(&self) -> Vec<Multiaddr>;

    /// Returns any multiaddrs that contain the TCP protocol with the `PeerId` prepended.
    fn multiaddr_p2p_tcp(&self) -> Vec<Multiaddr>;

    /// Returns any multiaddrs that contain the UDP protocol with the `PeerId` prepended.
    fn multiaddr_p2p_udp(&self) -> Vec<Multiaddr>;

    /// Returns any multiaddrs that contain the TCP protocol.
    fn multiaddr_tcp(&self) -> Vec<Multiaddr>;

    /// Returns any QUIC multiaddrs that are registered in this ENR.
    fn multiaddr_quic(&self) -> Vec<Multiaddr>;

    /// Returns the quic port if one is set.
    fn quic4(&self) -> Option<u16>;

    /// Returns the quic6 port if one is set.
    fn quic6(&self) -> Option<u16>;
}

/// Extend ENR CombinedPublicKey for libp2p types.
pub trait CombinedKeyPublicExt {
    /// Converts the publickey into a peer id, without consuming the key.
    fn as_peer_id(&self) -> PeerId;
}

/// Extend ENR CombinedKey for conversion to libp2p keys.
pub trait CombinedKeyExt {
    /// Converts a libp2p key into an ENR combined key.
    fn from_libp2p(key: Keypair) -> Result<CombinedKey, &'static str>;

    /// Converts a [`secp256k1::Keypair`] into and Enr [`CombinedKey`].
    fn from_secp256k1(key: &secp256k1::Keypair) -> CombinedKey;
}

impl EnrExt for Enr {
    /// The libp2p `PeerId` for the record.
    fn peer_id(&self) -> PeerId {
        self.public_key().as_peer_id()
    }

    /// Returns the quic port if one is set.
    fn quic4(&self) -> Option<u16> {
        self.get_decodable(QUIC_ENR_KEY).and_then(Result::ok)
    }

    /// Returns the quic6 port if one is set.
    fn quic6(&self) -> Option<u16> {
        self.get_decodable(QUIC6_ENR_KEY).and_then(Result::ok)
    }

    /// Returns a list of multiaddrs if the ENR has an `ip` and either a `tcp`, `quic` or `udp` key **or** an `ip6` and either a `tcp6` `quic6` or `udp6`.
    /// The vector remains empty if these fields are not defined.
    fn multiaddr(&self) -> Vec<Multiaddr> {
        let mut multiaddrs: Vec<Multiaddr> = Vec::new();
        if let Some(ip) = self.ip4() {
            if let Some(udp) = self.udp4() {
                let mut multiaddr: Multiaddr = ip.into();
                multiaddr.push(Protocol::Udp(udp));
                multiaddrs.push(multiaddr);
            }
            if let Some(quic) = self.quic4() {
                let mut multiaddr: Multiaddr = ip.into();
                multiaddr.push(Protocol::Udp(quic));
                multiaddr.push(Protocol::QuicV1);
                multiaddrs.push(multiaddr);
            }

            if let Some(tcp) = self.tcp4() {
                let mut multiaddr: Multiaddr = ip.into();
                multiaddr.push(Protocol::Tcp(tcp));
                multiaddrs.push(multiaddr);
            }
        }
        if let Some(ip6) = self.ip6() {
            if let Some(udp6) = self.udp6() {
                let mut multiaddr: Multiaddr = ip6.into();
                multiaddr.push(Protocol::Udp(udp6));
                multiaddrs.push(multiaddr);
            }

            if let Some(quic6) = self.quic6() {
                let mut multiaddr: Multiaddr = ip6.into();
                multiaddr.push(Protocol::Udp(quic6));
                multiaddr.push(Protocol::QuicV1);
                multiaddrs.push(multiaddr);
            }

            if let Some(tcp6) = self.tcp6() {
                let mut multiaddr: Multiaddr = ip6.into();
                multiaddr.push(Protocol::Tcp(tcp6));
                multiaddrs.push(multiaddr);
            }
        }
        multiaddrs
    }

    /// Returns a list of multiaddrs if the ENR has an `ip` and either a `tcp` or `udp` key **or** an `ip6` and either a `tcp6` or `udp6`.
    /// The vector remains empty if these fields are not defined.
    ///
    /// This also prepends the `PeerId` into each multiaddr with the `P2p` protocol.
    fn multiaddr_p2p(&self) -> Vec<Multiaddr> {
        let peer_id = self.peer_id();
        let mut multiaddrs: Vec<Multiaddr> = Vec::new();
        if let Some(ip) = self.ip4() {
            if let Some(udp) = self.udp4() {
                let mut multiaddr: Multiaddr = ip.into();
                multiaddr.push(Protocol::Udp(udp));
                multiaddr.push(Protocol::P2p(peer_id));
                multiaddrs.push(multiaddr);
            }

            if let Some(tcp) = self.tcp4() {
                let mut multiaddr: Multiaddr = ip.into();
                multiaddr.push(Protocol::Tcp(tcp));
                multiaddr.push(Protocol::P2p(peer_id));
                multiaddrs.push(multiaddr);
            }
        }
        if let Some(ip6) = self.ip6() {
            if let Some(udp6) = self.udp6() {
                let mut multiaddr: Multiaddr = ip6.into();
                multiaddr.push(Protocol::Udp(udp6));
                multiaddr.push(Protocol::P2p(peer_id));
                multiaddrs.push(multiaddr);
            }

            if let Some(tcp6) = self.tcp6() {
                let mut multiaddr: Multiaddr = ip6.into();
                multiaddr.push(Protocol::Tcp(tcp6));
                multiaddr.push(Protocol::P2p(peer_id));
                multiaddrs.push(multiaddr);
            }
        }
        multiaddrs
    }

    /// Returns a list of multiaddrs if the ENR has an `ip` and a `tcp` key **or** an `ip6` and a `tcp6`.
    /// The vector remains empty if these fields are not defined.
    ///
    /// This also prepends the `PeerId` into each multiaddr with the `P2p` protocol.
    fn multiaddr_p2p_tcp(&self) -> Vec<Multiaddr> {
        let peer_id = self.peer_id();
        let mut multiaddrs: Vec<Multiaddr> = Vec::new();
        if let Some(ip) = self.ip4()
            && let Some(tcp) = self.tcp4()
        {
            let mut multiaddr: Multiaddr = ip.into();
            multiaddr.push(Protocol::Tcp(tcp));
            multiaddr.push(Protocol::P2p(peer_id));
            multiaddrs.push(multiaddr);
        }
        if let Some(ip6) = self.ip6()
            && let Some(tcp6) = self.tcp6()
        {
            let mut multiaddr: Multiaddr = ip6.into();
            multiaddr.push(Protocol::Tcp(tcp6));
            multiaddr.push(Protocol::P2p(peer_id));
            multiaddrs.push(multiaddr);
        }
        multiaddrs
    }

    /// Returns a list of multiaddrs if the ENR has an `ip` and a `udp` key **or** an `ip6` and a `udp6`.
    /// The vector remains empty if these fields are not defined.
    ///
    /// This also prepends the `PeerId` into each multiaddr with the `P2p` protocol.
    fn multiaddr_p2p_udp(&self) -> Vec<Multiaddr> {
        let peer_id = self.peer_id();
        let mut multiaddrs: Vec<Multiaddr> = Vec::new();
        if let Some(ip) = self.ip4()
            && let Some(udp) = self.udp4()
        {
            let mut multiaddr: Multiaddr = ip.into();
            multiaddr.push(Protocol::Udp(udp));
            multiaddr.push(Protocol::P2p(peer_id));
            multiaddrs.push(multiaddr);
        }
        if let Some(ip6) = self.ip6()
            && let Some(udp6) = self.udp6()
        {
            let mut multiaddr: Multiaddr = ip6.into();
            multiaddr.push(Protocol::Udp(udp6));
            multiaddr.push(Protocol::P2p(peer_id));
            multiaddrs.push(multiaddr);
        }
        multiaddrs
    }

    /// Returns a list of multiaddrs if the ENR has an `ip` and a `quic` key **or** an `ip6` and a `quic6`.
    fn multiaddr_quic(&self) -> Vec<Multiaddr> {
        let mut multiaddrs: Vec<Multiaddr> = Vec::new();
        if let Some(quic_port) = self.quic4()
            && let Some(ip) = self.ip4()
        {
            let mut multiaddr: Multiaddr = ip.into();
            multiaddr.push(Protocol::Udp(quic_port));
            multiaddr.push(Protocol::QuicV1);
            multiaddrs.push(multiaddr);
        }

        if let Some(quic6_port) = self.quic6()
            && let Some(ip6) = self.ip6()
        {
            let mut multiaddr: Multiaddr = ip6.into();
            multiaddr.push(Protocol::Udp(quic6_port));
            multiaddr.push(Protocol::QuicV1);
            multiaddrs.push(multiaddr);
        }
        multiaddrs
    }

    /// Returns a list of multiaddrs if the ENR has an `ip` and either a `tcp` or `udp` key **or** an `ip6` and either a `tcp6` or `udp6`.
    fn multiaddr_tcp(&self) -> Vec<Multiaddr> {
        let mut multiaddrs: Vec<Multiaddr> = Vec::new();
        if let Some(ip) = self.ip4()
            && let Some(tcp) = self.tcp4()
        {
            let mut multiaddr: Multiaddr = ip.into();
            multiaddr.push(Protocol::Tcp(tcp));
            multiaddrs.push(multiaddr);
        }
        if let Some(ip6) = self.ip6()
            && let Some(tcp6) = self.tcp6()
        {
            let mut multiaddr: Multiaddr = ip6.into();
            multiaddr.push(Protocol::Tcp(tcp6));
            multiaddrs.push(multiaddr);
        }
        multiaddrs
    }
}

impl CombinedKeyPublicExt for CombinedPublicKey {
    /// Converts the publickey into a peer id, without consuming the key.
    ///
    /// This is only available with the `libp2p` feature flag.
    fn as_peer_id(&self) -> PeerId {
        match self {
            Self::Secp256k1(pk) => {
                let pk_bytes = pk.to_sec1_bytes();
                let libp2p_pk: PublicKey = secp256k1::PublicKey::try_from_bytes(&pk_bytes)
                    .expect("valid public key")
                    .into();
                PeerId::from_public_key(&libp2p_pk)
            }
            Self::Ed25519(pk) => {
                let pk_bytes = pk.to_bytes();
                let libp2p_pk: PublicKey = ed25519::PublicKey::try_from_bytes(&pk_bytes)
                    .expect("valid public key")
                    .into();
                PeerId::from_public_key(&libp2p_pk)
            }
        }
    }
}

impl CombinedKeyExt for CombinedKey {
    fn from_libp2p(key: Keypair) -> Result<CombinedKey, &'static str> {
        match key.key_type() {
            KeyType::Secp256k1 => {
                let key = key.try_into_secp256k1().expect("right key type");
                let secret =
                    discv5::enr::k256::ecdsa::SigningKey::from_slice(&key.secret().to_bytes())
                        .expect("libp2p key must be valid");
                Ok(CombinedKey::Secp256k1(secret))
            }
            KeyType::Ed25519 => {
                let key = key.try_into_ed25519().expect("right key type");
                let ed_keypair = discv5::enr::ed25519_dalek::SigningKey::from_bytes(
                    &(key.to_bytes()[..32])
                        .try_into()
                        .expect("libp2p key must be valid"),
                );
                Ok(CombinedKey::from(ed_keypair))
            }
            _ => Err("Unsupported keypair kind"),
        }
    }
    fn from_secp256k1(key: &secp256k1::Keypair) -> Self {
        let secret = discv5::enr::k256::ecdsa::SigningKey::from_slice(&key.secret().to_bytes())
            .expect("libp2p key must be valid");
        CombinedKey::Secp256k1(secret)
    }
}

// helper function to convert a peer_id to a node_id. This is only possible for secp256k1/ed25519 libp2p
// peer_ids
pub fn peer_id_to_node_id(peer_id: &PeerId) -> Result<discv5::enr::NodeId, String> {
    // A libp2p peer id byte representation should be 2 length bytes + 4 protobuf bytes + compressed pk bytes
    // if generated from a PublicKey with Identity multihash.
    let pk_bytes = &peer_id.to_bytes()[2..];

    let public_key = PublicKey::try_decode_protobuf(pk_bytes).map_err(|e| {
        format!(
            " Cannot parse libp2p public key public key from peer id: {}",
            e
        )
    })?;

    match public_key.key_type() {
        KeyType::Secp256k1 => {
            let pk = public_key
                .clone()
                .try_into_secp256k1()
                .expect("right key type");
            let uncompressed_key_bytes = &pk.to_bytes_uncompressed()[1..];
            let mut output = [0_u8; 32];
            let mut hasher = Keccak::v256();
            hasher.update(uncompressed_key_bytes);
            hasher.finalize(&mut output);
            Ok(discv5::enr::NodeId::parse(&output).expect("Must be correct length"))
        }
        KeyType::Ed25519 => {
            let pk = public_key
                .clone()
                .try_into_ed25519()
                .expect("right key type");
            let uncompressed_key_bytes = pk.to_bytes();
            let mut output = [0_u8; 32];
            let mut hasher = Keccak::v256();
            hasher.update(&uncompressed_key_bytes);
            hasher.finalize(&mut output);
            Ok(discv5::enr::NodeId::parse(&output).expect("Must be correct length"))
        }

        _ => Err(format!("Unsupported public key from peer {}", peer_id)),
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    // ── Helper: generate CombinedKey for ENR building ───────────────

    fn secp256k1_combined_key() -> CombinedKey {
        let sk_hex = "df94a73d528434ce2309abb19c16aedb535322797dbd59c157b1e04095900f48";
        let sk_bytes = hex::decode(sk_hex).unwrap();
        let secret = discv5::enr::k256::ecdsa::SigningKey::from_slice(&sk_bytes).unwrap();
        CombinedKey::Secp256k1(secret)
    }

    fn ed25519_combined_key() -> CombinedKey {
        let sk_hex = "4dea8a5072119927e9d243a7d953f2f4bc95b70f110978e2f9bc7a9000e4b261";
        let sk_bytes = hex::decode(sk_hex).unwrap();
        let secret =
            discv5::enr::ed25519_dalek::SigningKey::from_bytes(&sk_bytes.try_into().unwrap());
        CombinedKey::from(secret)
    }

    /// Build a secp256k1 libp2p Keypair from the same key material.
    fn secp256k1_libp2p_keypair() -> Keypair {
        let sk_hex = "df94a73d528434ce2309abb19c16aedb535322797dbd59c157b1e04095900f48";
        let sk_bytes = hex::decode(sk_hex).unwrap();
        let libp2p_sk = secp256k1::SecretKey::try_from_bytes(sk_bytes).unwrap();
        let kp: secp256k1::Keypair = libp2p_sk.into();
        kp.into()
    }

    /// Build an ed25519 libp2p Keypair from the same key material.
    fn ed25519_libp2p_keypair() -> Keypair {
        let sk_hex = "4dea8a5072119927e9d243a7d953f2f4bc95b70f110978e2f9bc7a9000e4b261";
        let sk_bytes = hex::decode(sk_hex).unwrap();
        let libp2p_sk = ed25519::SecretKey::try_from_bytes(sk_bytes).unwrap();
        let kp: ed25519::Keypair = libp2p_sk.into();
        kp.into()
    }

    // ── Existing peer_id / node_id conversion tests ─────────────────

    #[test]
    fn test_secp256k1_peer_id_conversion() {
        let key = secp256k1_combined_key();
        let libp2p_kp = secp256k1_libp2p_keypair();
        let peer_id = libp2p_kp.public().to_peer_id();

        let enr = discv5::enr::Enr::builder().build(&key).unwrap();
        let node_id = peer_id_to_node_id(&peer_id).unwrap();

        assert_eq!(enr.node_id(), node_id);
    }

    #[test]
    fn test_ed25519_peer_conversion() {
        let key = ed25519_combined_key();
        let libp2p_kp = ed25519_libp2p_keypair();
        let peer_id = libp2p_kp.public().to_peer_id();

        let enr = discv5::enr::Enr::builder().build(&key).unwrap();
        let node_id = peer_id_to_node_id(&peer_id).unwrap();

        assert_eq!(enr.node_id(), node_id);
    }

    // ── peer_id() ───────────────────────────────────────────────────

    #[test]
    fn peer_id_deterministic_secp256k1() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder().build(&key).unwrap();
        let pid1 = enr.peer_id();
        let pid2 = enr.peer_id();
        assert_eq!(pid1, pid2, "peer_id should be deterministic");
    }

    #[test]
    fn peer_id_deterministic_ed25519() {
        let key = ed25519_combined_key();
        let enr = discv5::enr::Enr::builder().build(&key).unwrap();
        let pid1 = enr.peer_id();
        let pid2 = enr.peer_id();
        assert_eq!(pid1, pid2, "peer_id should be deterministic");
    }

    // ── quic4 / quic6 ──────────────────────────────────────────────

    #[test]
    fn quic4_none_when_not_set() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder().build(&key).unwrap();
        assert_eq!(enr.quic4(), None);
    }

    #[test]
    fn quic4_returns_port_when_set() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .add_value(QUIC_ENR_KEY, &9001u16)
            .build(&key)
            .unwrap();
        assert_eq!(enr.quic4(), Some(9001));
    }

    #[test]
    fn quic6_none_when_not_set() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder().build(&key).unwrap();
        assert_eq!(enr.quic6(), None);
    }

    #[test]
    fn quic6_returns_port_when_set() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .add_value(QUIC6_ENR_KEY, &9101u16)
            .build(&key)
            .unwrap();
        assert_eq!(enr.quic6(), Some(9101));
    }

    // ── multiaddr() — no addresses set ──────────────────────────────

    #[test]
    fn multiaddr_empty_when_no_addresses() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder().build(&key).unwrap();
        assert!(enr.multiaddr().is_empty());
    }

    // ── multiaddr() — IPv4 only ─────────────────────────────────────

    #[test]
    fn multiaddr_ipv4_udp_only() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .udp4(9000)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].to_string(), "/ip4/127.0.0.1/udp/9000");
    }

    #[test]
    fn multiaddr_ipv4_tcp_only() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .tcp4(9002)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].to_string(), "/ip4/127.0.0.1/tcp/9002");
    }

    #[test]
    fn multiaddr_ipv4_quic_only() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .add_value(QUIC_ENR_KEY, &9001u16)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].to_string(), "/ip4/127.0.0.1/udp/9001/quic-v1");
    }

    #[test]
    fn multiaddr_ipv4_all_protocols() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .udp4(9000)
            .tcp4(9002)
            .add_value(QUIC_ENR_KEY, &9001u16)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr();
        // UDP, QUIC, TCP — in that order per the implementation
        assert_eq!(addrs.len(), 3);
        assert_eq!(addrs[0].to_string(), "/ip4/127.0.0.1/udp/9000");
        assert_eq!(addrs[1].to_string(), "/ip4/127.0.0.1/udp/9001/quic-v1");
        assert_eq!(addrs[2].to_string(), "/ip4/127.0.0.1/tcp/9002");
    }

    // ── multiaddr() — IPv6 only ─────────────────────────────────────

    #[test]
    fn multiaddr_ipv6_udp_only() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip6(Ipv6Addr::LOCALHOST)
            .udp6(9100)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].to_string(), "/ip6/::1/udp/9100");
    }

    #[test]
    fn multiaddr_ipv6_tcp_only() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip6(Ipv6Addr::LOCALHOST)
            .tcp6(9102)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].to_string(), "/ip6/::1/tcp/9102");
    }

    #[test]
    fn multiaddr_ipv6_quic_only() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip6(Ipv6Addr::LOCALHOST)
            .add_value(QUIC6_ENR_KEY, &9101u16)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].to_string(), "/ip6/::1/udp/9101/quic-v1");
    }

    #[test]
    fn multiaddr_ipv6_all_protocols() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip6(Ipv6Addr::LOCALHOST)
            .udp6(9100)
            .tcp6(9102)
            .add_value(QUIC6_ENR_KEY, &9101u16)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr();
        assert_eq!(addrs.len(), 3);
        assert_eq!(addrs[0].to_string(), "/ip6/::1/udp/9100");
        assert_eq!(addrs[1].to_string(), "/ip6/::1/udp/9101/quic-v1");
        assert_eq!(addrs[2].to_string(), "/ip6/::1/tcp/9102");
    }

    // ── multiaddr() — dual-stack ────────────────────────────────────

    #[test]
    fn multiaddr_dual_stack() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .udp4(9000)
            .tcp4(9002)
            .ip6(Ipv6Addr::LOCALHOST)
            .udp6(9100)
            .tcp6(9102)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr();
        assert_eq!(addrs.len(), 4);
        // IPv4 first, then IPv6
        assert!(addrs[0].to_string().contains("/ip4/"));
        assert!(addrs[1].to_string().contains("/ip4/"));
        assert!(addrs[2].to_string().contains("/ip6/"));
        assert!(addrs[3].to_string().contains("/ip6/"));
    }

    // ── multiaddr() — port without IP returns nothing ───────────────

    #[test]
    fn multiaddr_udp_without_ip_returns_empty() {
        let key = secp256k1_combined_key();
        // Setting just UDP without IP — cannot form a multiaddr
        let enr = discv5::enr::Enr::builder().udp4(9000).build(&key).unwrap();
        assert!(enr.multiaddr().is_empty());
    }

    // ── multiaddr_p2p() ─────────────────────────────────────────────

    #[test]
    fn multiaddr_p2p_empty_when_no_addresses() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder().build(&key).unwrap();
        assert!(enr.multiaddr_p2p().is_empty());
    }

    #[test]
    fn multiaddr_p2p_includes_peer_id() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .udp4(9000)
            .build(&key)
            .unwrap();
        let peer_id = enr.peer_id();
        let addrs = enr.multiaddr_p2p();
        assert_eq!(addrs.len(), 1);
        let addr_str = addrs[0].to_string();
        assert!(
            addr_str.contains(&format!("/p2p/{peer_id}")),
            "should contain peer_id: {addr_str}"
        );
    }

    #[test]
    fn multiaddr_p2p_ipv4_udp_and_tcp() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .udp4(9000)
            .tcp4(9002)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr_p2p();
        assert_eq!(addrs.len(), 2);
        // UDP first, then TCP
        assert!(addrs[0].to_string().contains("/udp/9000/p2p/"));
        assert!(addrs[1].to_string().contains("/tcp/9002/p2p/"));
    }

    #[test]
    fn multiaddr_p2p_ipv6() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip6(Ipv6Addr::LOCALHOST)
            .tcp6(9102)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr_p2p();
        assert_eq!(addrs.len(), 1);
        assert!(addrs[0].to_string().contains("/ip6/::1/tcp/9102/p2p/"));
    }

    // ── multiaddr_p2p_tcp() ─────────────────────────────────────────

    #[test]
    fn multiaddr_p2p_tcp_empty_when_no_tcp() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .udp4(9000)
            .build(&key)
            .unwrap();
        assert!(enr.multiaddr_p2p_tcp().is_empty());
    }

    #[test]
    fn multiaddr_p2p_tcp_ipv4() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .tcp4(9002)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr_p2p_tcp();
        assert_eq!(addrs.len(), 1);
        assert!(
            addrs[0]
                .to_string()
                .contains("/ip4/127.0.0.1/tcp/9002/p2p/")
        );
    }

    #[test]
    fn multiaddr_p2p_tcp_ipv6() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip6(Ipv6Addr::LOCALHOST)
            .tcp6(9102)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr_p2p_tcp();
        assert_eq!(addrs.len(), 1);
        assert!(addrs[0].to_string().contains("/ip6/::1/tcp/9102/p2p/"));
    }

    #[test]
    fn multiaddr_p2p_tcp_dual_stack() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .tcp4(9002)
            .ip6(Ipv6Addr::LOCALHOST)
            .tcp6(9102)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr_p2p_tcp();
        assert_eq!(addrs.len(), 2);
        assert!(addrs[0].to_string().contains("/ip4/"));
        assert!(addrs[1].to_string().contains("/ip6/"));
    }

    // ── multiaddr_p2p_udp() ─────────────────────────────────────────

    #[test]
    fn multiaddr_p2p_udp_empty_when_no_udp() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .tcp4(9002)
            .build(&key)
            .unwrap();
        assert!(enr.multiaddr_p2p_udp().is_empty());
    }

    #[test]
    fn multiaddr_p2p_udp_ipv4() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .udp4(9000)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr_p2p_udp();
        assert_eq!(addrs.len(), 1);
        assert!(
            addrs[0]
                .to_string()
                .contains("/ip4/127.0.0.1/udp/9000/p2p/")
        );
    }

    #[test]
    fn multiaddr_p2p_udp_ipv6() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip6(Ipv6Addr::LOCALHOST)
            .udp6(9100)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr_p2p_udp();
        assert_eq!(addrs.len(), 1);
        assert!(addrs[0].to_string().contains("/ip6/::1/udp/9100/p2p/"));
    }

    #[test]
    fn multiaddr_p2p_udp_dual_stack() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .udp4(9000)
            .ip6(Ipv6Addr::LOCALHOST)
            .udp6(9100)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr_p2p_udp();
        assert_eq!(addrs.len(), 2);
        assert!(addrs[0].to_string().contains("/ip4/"));
        assert!(addrs[1].to_string().contains("/ip6/"));
    }

    // ── multiaddr_tcp() ─────────────────────────────────────────────

    #[test]
    fn multiaddr_tcp_empty_when_no_tcp() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .udp4(9000)
            .build(&key)
            .unwrap();
        assert!(enr.multiaddr_tcp().is_empty());
    }

    #[test]
    fn multiaddr_tcp_ipv4() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .tcp4(9002)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr_tcp();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].to_string(), "/ip4/127.0.0.1/tcp/9002");
    }

    #[test]
    fn multiaddr_tcp_does_not_include_peer_id() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .tcp4(9002)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr_tcp();
        assert!(!addrs[0].to_string().contains("/p2p/"));
    }

    // ── multiaddr_quic() ────────────────────────────────────────────

    #[test]
    fn multiaddr_quic_empty_when_no_quic() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .tcp4(9002)
            .build(&key)
            .unwrap();
        assert!(enr.multiaddr_quic().is_empty());
    }

    #[test]
    fn multiaddr_quic_ipv4() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .add_value(QUIC_ENR_KEY, &9001u16)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr_quic();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].to_string(), "/ip4/127.0.0.1/udp/9001/quic-v1");
    }

    #[test]
    fn multiaddr_quic_ipv6() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip6(Ipv6Addr::LOCALHOST)
            .add_value(QUIC6_ENR_KEY, &9101u16)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr_quic();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].to_string(), "/ip6/::1/udp/9101/quic-v1");
    }

    #[test]
    fn multiaddr_quic_dual_stack() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .add_value(QUIC_ENR_KEY, &9001u16)
            .ip6(Ipv6Addr::LOCALHOST)
            .add_value(QUIC6_ENR_KEY, &9101u16)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr_quic();
        assert_eq!(addrs.len(), 2);
        assert!(addrs[0].to_string().contains("/ip4/"));
        assert!(addrs[1].to_string().contains("/ip6/"));
    }

    #[test]
    fn multiaddr_quic_without_ip_returns_empty() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .add_value(QUIC_ENR_KEY, &9001u16)
            .build(&key)
            .unwrap();
        assert!(enr.multiaddr_quic().is_empty());
    }

    // ── CombinedKeyExt ──────────────────────────────────────────────

    #[test]
    fn from_libp2p_secp256k1_roundtrip() {
        let libp2p_kp = secp256k1_libp2p_keypair();
        let combined = CombinedKey::from_libp2p(libp2p_kp.clone()).unwrap();
        let enr = discv5::enr::Enr::builder().build(&combined).unwrap();
        assert_eq!(
            enr.peer_id(),
            libp2p_kp.public().to_peer_id(),
            "ENR peer_id should match libp2p peer_id"
        );
    }

    #[test]
    fn from_libp2p_ed25519_roundtrip() {
        let libp2p_kp = ed25519_libp2p_keypair();
        let combined = CombinedKey::from_libp2p(libp2p_kp.clone()).unwrap();
        let enr = discv5::enr::Enr::builder().build(&combined).unwrap();
        assert_eq!(
            enr.peer_id(),
            libp2p_kp.public().to_peer_id(),
            "ENR peer_id should match libp2p peer_id"
        );
    }

    #[test]
    fn from_secp256k1_keypair() {
        let libp2p_kp = secp256k1_libp2p_keypair();
        let secp_kp = libp2p_kp
            .clone()
            .try_into_secp256k1()
            .expect("should be secp256k1");
        let combined = CombinedKey::from_secp256k1(&secp_kp);
        let enr = discv5::enr::Enr::builder().build(&combined).unwrap();
        assert_eq!(enr.peer_id(), libp2p_kp.public().to_peer_id());
    }

    // ── CombinedKeyPublicExt ────────────────────────────────────────

    #[test]
    fn as_peer_id_secp256k1_matches_libp2p() {
        let libp2p_kp = secp256k1_libp2p_keypair();
        let secp_kp = libp2p_kp
            .clone()
            .try_into_secp256k1()
            .expect("should be secp256k1");
        let combined = CombinedKey::from_secp256k1(&secp_kp);
        let enr = discv5::enr::Enr::builder().build(&combined).unwrap();
        let pub_key = enr.public_key();
        assert_eq!(pub_key.as_peer_id(), libp2p_kp.public().to_peer_id());
    }

    #[test]
    fn as_peer_id_ed25519_matches_libp2p() {
        let libp2p_kp = ed25519_libp2p_keypair();
        let combined = CombinedKey::from_libp2p(libp2p_kp.clone()).unwrap();
        let enr = discv5::enr::Enr::builder().build(&combined).unwrap();
        let pub_key = enr.public_key();
        assert_eq!(pub_key.as_peer_id(), libp2p_kp.public().to_peer_id());
    }

    // ── peer_id_to_node_id edge cases ───────────────────────────────

    #[test]
    fn peer_id_to_node_id_secp256k1_consistency() {
        let libp2p_kp = secp256k1_libp2p_keypair();
        let peer_id = libp2p_kp.public().to_peer_id();
        let node_id_1 = peer_id_to_node_id(&peer_id).unwrap();
        let node_id_2 = peer_id_to_node_id(&peer_id).unwrap();
        assert_eq!(node_id_1, node_id_2, "should be deterministic");
    }

    #[test]
    fn peer_id_to_node_id_ed25519_consistency() {
        let libp2p_kp = ed25519_libp2p_keypair();
        let peer_id = libp2p_kp.public().to_peer_id();
        let node_id_1 = peer_id_to_node_id(&peer_id).unwrap();
        let node_id_2 = peer_id_to_node_id(&peer_id).unwrap();
        assert_eq!(node_id_1, node_id_2, "should be deterministic");
    }

    #[test]
    fn different_keys_produce_different_node_ids() {
        // Use two different keys by deriving from different hex values
        let sk1_hex = "df94a73d528434ce2309abb19c16aedb535322797dbd59c157b1e04095900f48";
        let sk2_hex = "1111111111111111111111111111111111111111111111111111111111111111";
        let sk1_bytes = hex::decode(sk1_hex).unwrap();
        let sk2_bytes = hex::decode(sk2_hex).unwrap();
        let kp1: Keypair = {
            let sk = secp256k1::SecretKey::try_from_bytes(sk1_bytes).unwrap();
            let kp: secp256k1::Keypair = sk.into();
            kp.into()
        };
        let kp2: Keypair = {
            let sk = secp256k1::SecretKey::try_from_bytes(sk2_bytes).unwrap();
            let kp: secp256k1::Keypair = sk.into();
            kp.into()
        };
        let nid1 = peer_id_to_node_id(&kp1.public().to_peer_id()).unwrap();
        let nid2 = peer_id_to_node_id(&kp2.public().to_peer_id()).unwrap();
        assert_ne!(
            nid1, nid2,
            "different keys should produce different node IDs"
        );
    }

    // ── multiaddr with non-localhost IP ─────────────────────────────

    #[test]
    fn multiaddr_with_public_ipv4() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip4(Ipv4Addr::new(192, 168, 1, 100))
            .tcp4(30303)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].to_string(), "/ip4/192.168.1.100/tcp/30303");
    }

    #[test]
    fn multiaddr_with_public_ipv6() {
        let key = secp256k1_combined_key();
        let enr = discv5::enr::Enr::builder()
            .ip6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1))
            .tcp6(30303)
            .build(&key)
            .unwrap();
        let addrs = enr.multiaddr();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].to_string(), "/ip6/2001:db8::1/tcp/30303");
    }
}
