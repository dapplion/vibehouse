use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use multiaddr::{Multiaddr, Protocol};
use serde::{Deserialize, Serialize};

/// A listening address composed by an Ip, an UDP port and a TCP port.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListenAddr<Ip> {
    /// The IP address we will listen on.
    pub addr: Ip,
    /// The UDP port that discovery will listen on.
    pub disc_port: u16,
    /// The UDP port that QUIC will listen on.
    pub quic_port: u16,
    /// The TCP port that libp2p will listen on.
    pub tcp_port: u16,
}

impl<Ip: Into<IpAddr> + Clone> ListenAddr<Ip> {
    pub fn discovery_socket_addr(&self) -> SocketAddr {
        (self.addr.clone().into(), self.disc_port).into()
    }

    pub fn quic_socket_addr(&self) -> SocketAddr {
        (self.addr.clone().into(), self.quic_port).into()
    }

    pub fn tcp_socket_addr(&self) -> SocketAddr {
        (self.addr.clone().into(), self.tcp_port).into()
    }
}

/// Types of listening addresses Vibehouse can accept.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ListenAddress {
    V4(ListenAddr<Ipv4Addr>),
    V6(ListenAddr<Ipv6Addr>),
    DualStack(ListenAddr<Ipv4Addr>, ListenAddr<Ipv6Addr>),
}

impl ListenAddress {
    /// Return the listening address over IpV4 if any.
    pub fn v4(&self) -> Option<&ListenAddr<Ipv4Addr>> {
        match self {
            ListenAddress::V4(v4_addr) | ListenAddress::DualStack(v4_addr, _) => Some(v4_addr),
            ListenAddress::V6(_) => None,
        }
    }

    /// Return the listening address over IpV6 if any.
    pub fn v6(&self) -> Option<&ListenAddr<Ipv6Addr>> {
        match self {
            ListenAddress::V6(v6_addr) | ListenAddress::DualStack(_, v6_addr) => Some(v6_addr),
            ListenAddress::V4(_) => None,
        }
    }

    /// Returns the addresses the Swarm will listen on, given the setup.
    pub fn libp2p_addresses(&self) -> impl Iterator<Item = Multiaddr> {
        let v4_tcp_multiaddr = self
            .v4()
            .map(|v4_addr| Multiaddr::from(v4_addr.addr).with(Protocol::Tcp(v4_addr.tcp_port)));

        let v4_quic_multiaddr = self.v4().map(|v4_addr| {
            Multiaddr::from(v4_addr.addr)
                .with(Protocol::Udp(v4_addr.quic_port))
                .with(Protocol::QuicV1)
        });

        let v6_quic_multiaddr = self.v6().map(|v6_addr| {
            Multiaddr::from(v6_addr.addr)
                .with(Protocol::Udp(v6_addr.quic_port))
                .with(Protocol::QuicV1)
        });

        let v6_tcp_multiaddr = self
            .v6()
            .map(|v6_addr| Multiaddr::from(v6_addr.addr).with(Protocol::Tcp(v6_addr.tcp_port)));

        v4_tcp_multiaddr
            .into_iter()
            .chain(v4_quic_multiaddr)
            .chain(v6_quic_multiaddr)
            .chain(v6_tcp_multiaddr)
    }

    pub fn unused_v4_ports() -> Self {
        ListenAddress::V4(ListenAddr {
            addr: Ipv4Addr::UNSPECIFIED,
            disc_port: crate::unused_port::unused_udp4_port().unwrap(),
            quic_port: crate::unused_port::unused_udp4_port().unwrap(),
            tcp_port: crate::unused_port::unused_tcp4_port().unwrap(),
        })
    }

    pub fn unused_v6_ports() -> Self {
        ListenAddress::V6(ListenAddr {
            addr: Ipv6Addr::UNSPECIFIED,
            disc_port: crate::unused_port::unused_udp6_port().unwrap(),
            quic_port: crate::unused_port::unused_udp6_port().unwrap(),
            tcp_port: crate::unused_port::unused_tcp6_port().unwrap(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4_addr() -> ListenAddr<Ipv4Addr> {
        ListenAddr {
            addr: Ipv4Addr::new(127, 0, 0, 1),
            disc_port: 9000,
            quic_port: 9001,
            tcp_port: 9002,
        }
    }

    fn v6_addr() -> ListenAddr<Ipv6Addr> {
        ListenAddr {
            addr: Ipv6Addr::LOCALHOST,
            disc_port: 9100,
            quic_port: 9101,
            tcp_port: 9102,
        }
    }

    // ── Socket address methods ───────────────────────────────────

    #[test]
    fn v4_discovery_socket_addr() {
        let addr = v4_addr();
        let sa = addr.discovery_socket_addr();
        assert_eq!(
            sa,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 9000)
        );
    }

    #[test]
    fn v4_quic_socket_addr() {
        let addr = v4_addr();
        let sa = addr.quic_socket_addr();
        assert_eq!(
            sa,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 9001)
        );
    }

    #[test]
    fn v4_tcp_socket_addr() {
        let addr = v4_addr();
        let sa = addr.tcp_socket_addr();
        assert_eq!(
            sa,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 9002)
        );
    }

    #[test]
    fn v6_discovery_socket_addr() {
        let addr = v6_addr();
        let sa = addr.discovery_socket_addr();
        assert_eq!(sa, SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 9100));
    }

    #[test]
    fn v6_quic_socket_addr() {
        let addr = v6_addr();
        let sa = addr.quic_socket_addr();
        assert_eq!(sa, SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 9101));
    }

    #[test]
    fn v6_tcp_socket_addr() {
        let addr = v6_addr();
        let sa = addr.tcp_socket_addr();
        assert_eq!(sa, SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 9102));
    }

    // ── ListenAddress v4/v6 selectors ────────────────────────────

    #[test]
    fn listen_address_v4_returns_some_for_v4() {
        let la = ListenAddress::V4(v4_addr());
        assert!(la.v4().is_some());
        assert!(la.v6().is_none());
    }

    #[test]
    fn listen_address_v6_returns_some_for_v6() {
        let la = ListenAddress::V6(v6_addr());
        assert!(la.v6().is_some());
        assert!(la.v4().is_none());
    }

    #[test]
    fn listen_address_dual_stack_returns_both() {
        let la = ListenAddress::DualStack(v4_addr(), v6_addr());
        assert!(la.v4().is_some());
        assert!(la.v6().is_some());
    }

    // ── libp2p_addresses ─────────────────────────────────────────

    #[test]
    fn libp2p_addresses_v4_produces_two_addrs() {
        let la = ListenAddress::V4(v4_addr());
        let addrs: Vec<_> = la.libp2p_addresses().collect();
        // TCP + QUIC
        assert_eq!(addrs.len(), 2);

        let tcp_str = addrs[0].to_string();
        assert!(
            tcp_str.contains("/ip4/127.0.0.1/tcp/9002"),
            "got: {tcp_str}"
        );

        let quic_str = addrs[1].to_string();
        assert!(
            quic_str.contains("/ip4/127.0.0.1/udp/9001/quic-v1"),
            "got: {quic_str}"
        );
    }

    #[test]
    fn libp2p_addresses_v6_produces_two_addrs() {
        let la = ListenAddress::V6(v6_addr());
        let addrs: Vec<_> = la.libp2p_addresses().collect();
        assert_eq!(addrs.len(), 2);

        let quic_str = addrs[0].to_string();
        assert!(quic_str.contains("quic-v1"), "got: {quic_str}");

        let tcp_str = addrs[1].to_string();
        assert!(tcp_str.contains("tcp/9102"), "got: {tcp_str}");
    }

    #[test]
    fn libp2p_addresses_dual_stack_produces_four_addrs() {
        let la = ListenAddress::DualStack(v4_addr(), v6_addr());
        let addrs: Vec<_> = la.libp2p_addresses().collect();
        assert_eq!(addrs.len(), 4);
    }
}
