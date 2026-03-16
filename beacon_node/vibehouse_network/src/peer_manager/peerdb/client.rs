//! Known Ethereum 2.0 clients and their fingerprints.
//!
//! Currently using identify to fingerprint.

use libp2p::identify::Info as IdentifyInfo;
use serde::Serialize;
use strum::{AsRefStr, EnumIter, IntoStaticStr};

/// Various client and protocol information related to a node.
#[derive(Clone, Debug, Serialize)]
pub struct Client {
    /// The client's name (Ex: vibehouse, prism, nimbus, etc)
    pub kind: ClientKind,
    /// The client's version.
    pub version: String,
    /// The OS version of the client.
    pub os_version: String,
    /// The libp2p protocol version.
    pub protocol_version: String,
    /// Identify agent string
    pub agent_string: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, AsRefStr, IntoStaticStr, EnumIter)]
pub enum ClientKind {
    /// A vibehouse node.
    Vibehouse,
    /// A Lighthouse node.
    Lighthouse,
    /// A Nimbus node.
    Nimbus,
    /// A Teku node.
    Teku,
    /// A Prysm node.
    Prysm,
    /// A lodestar node.
    Lodestar,
    /// A Caplin node.
    Caplin,
    /// An unknown client.
    Unknown,
}

impl Default for Client {
    fn default() -> Self {
        Client {
            kind: ClientKind::Unknown,
            version: "unknown".into(),
            os_version: "unknown".into(),
            protocol_version: "unknown".into(),
            agent_string: None,
        }
    }
}

impl Client {
    /// Builds a `Client` from `IdentifyInfo`.
    pub fn from_identify_info(info: &IdentifyInfo) -> Self {
        let (kind, version, os_version) = client_from_agent_version(&info.agent_version);

        Client {
            kind,
            version,
            os_version,
            protocol_version: info.protocol_version.clone(),
            agent_string: Some(info.agent_version.clone()),
        }
    }
}

impl std::fmt::Display for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            ClientKind::Vibehouse => write!(
                f,
                "vibehouse: version: {}, os_version: {}",
                self.version, self.os_version
            ),
            ClientKind::Lighthouse => write!(
                f,
                "Lighthouse: version: {}, os_version: {}",
                self.version, self.os_version
            ),
            ClientKind::Teku => write!(
                f,
                "Teku: version: {}, os_version: {}",
                self.version, self.os_version
            ),
            ClientKind::Nimbus => write!(
                f,
                "Nimbus: version: {}, os_version: {}",
                self.version, self.os_version
            ),
            ClientKind::Prysm => write!(
                f,
                "Prysm: version: {}, os_version: {}",
                self.version, self.os_version
            ),
            ClientKind::Lodestar => write!(f, "Lodestar: version: {}", self.version),
            ClientKind::Caplin => write!(f, "Caplin"),
            ClientKind::Unknown => {
                if let Some(agent_string) = &self.agent_string {
                    write!(f, "Unknown: {}", agent_string)
                } else {
                    write!(f, "Unknown")
                }
            }
        }
    }
}

impl std::fmt::Display for ClientKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_ref())
    }
}

// helper function to identify clients from their agent_version. Returns the client
// kind and it's associated version and the OS kind.
fn client_from_agent_version(agent_version: &str) -> (ClientKind, String, String) {
    let mut agent_split = agent_version.split('/');
    let mut version = String::from("unknown");
    let mut os_version = String::from("unknown");
    match agent_split.next() {
        Some("vibehouse") => {
            let kind = ClientKind::Vibehouse;
            if let Some(agent_version) = agent_split.next() {
                version = agent_version.into();
                if let Some(agent_os_version) = agent_split.next() {
                    os_version = agent_os_version.into();
                }
            }
            (kind, version, os_version)
        }
        Some("Lighthouse") => {
            let kind = ClientKind::Lighthouse;
            if let Some(agent_version) = agent_split.next() {
                version = agent_version.into();
                if let Some(agent_os_version) = agent_split.next() {
                    os_version = agent_os_version.into();
                }
            }
            (kind, version, os_version)
        }
        Some("teku") => {
            let kind = ClientKind::Teku;
            if agent_split.next().is_some()
                && let Some(agent_version) = agent_split.next()
            {
                version = agent_version.into();
                if let Some(agent_os_version) = agent_split.next() {
                    os_version = agent_os_version.into();
                }
            }
            (kind, version, os_version)
        }
        Some("github.com") => {
            let kind = ClientKind::Prysm;
            (kind, version, os_version)
        }
        Some("Prysm") => {
            let kind = ClientKind::Prysm;
            if agent_split.next().is_some()
                && let Some(agent_version) = agent_split.next()
            {
                version = agent_version.into();
                if let Some(agent_os_version) = agent_split.next() {
                    os_version = agent_os_version.into();
                }
            }
            (kind, version, os_version)
        }
        Some("nimbus") => {
            let kind = ClientKind::Nimbus;
            if agent_split.next().is_some()
                && let Some(agent_version) = agent_split.next()
            {
                version = agent_version.into();
                if let Some(agent_os_version) = agent_split.next() {
                    os_version = agent_os_version.into();
                }
            }
            (kind, version, os_version)
        }
        Some("nim-libp2p") => {
            let kind = ClientKind::Nimbus;
            if let Some(agent_version) = agent_split.next() {
                version = agent_version.into();
                if let Some(agent_os_version) = agent_split.next() {
                    os_version = agent_os_version.into();
                }
            }
            (kind, version, os_version)
        }
        Some("js-libp2p") | Some("lodestar") => {
            let kind = ClientKind::Lodestar;
            if let Some(agent_version) = agent_split.next() {
                version = agent_version.into();
                if let Some(agent_os_version) = agent_split.next() {
                    os_version = agent_os_version.into();
                }
            }
            (kind, version, os_version)
        }
        Some("erigon") => {
            let client_kind = if let Some("caplin") = agent_split.next() {
                ClientKind::Caplin
            } else {
                ClientKind::Unknown
            };
            (client_kind, version, os_version)
        }
        _ => {
            let unknown = String::from("unknown");
            (ClientKind::Unknown, unknown.clone(), unknown)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    #[test]
    fn client_default() {
        let client = Client::default();
        assert_eq!(client.kind, ClientKind::Unknown);
        assert_eq!(client.version, "unknown");
        assert_eq!(client.os_version, "unknown");
        assert_eq!(client.protocol_version, "unknown");
        assert!(client.agent_string.is_none());
    }

    #[test]
    fn parse_vibehouse_agent() {
        let (kind, version, os) = client_from_agent_version("vibehouse/v1.0.0/linux");
        assert_eq!(kind, ClientKind::Vibehouse);
        assert_eq!(version, "v1.0.0");
        assert_eq!(os, "linux");
    }

    #[test]
    fn parse_lighthouse_agent() {
        let (kind, version, os) = client_from_agent_version("Lighthouse/v5.3.0/x86_64-linux");
        assert_eq!(kind, ClientKind::Lighthouse);
        assert_eq!(version, "v5.3.0");
        assert_eq!(os, "x86_64-linux");
    }

    #[test]
    fn parse_teku_agent() {
        let (kind, version, os) = client_from_agent_version("teku/teku/v24.1.0/linux-x86_64");
        assert_eq!(kind, ClientKind::Teku);
        assert_eq!(version, "v24.1.0");
        assert_eq!(os, "linux-x86_64");
    }

    #[test]
    fn parse_prysm_github_agent() {
        let (kind, version, _) = client_from_agent_version("github.com/prysmaticlabs/prysm");
        assert_eq!(kind, ClientKind::Prysm);
        assert_eq!(version, "unknown");
    }

    #[test]
    fn parse_prysm_named_agent() {
        let (kind, version, os) = client_from_agent_version("Prysm/unused/v5.0.0/linux");
        assert_eq!(kind, ClientKind::Prysm);
        assert_eq!(version, "v5.0.0");
        assert_eq!(os, "linux");
    }

    #[test]
    fn parse_nimbus_agent() {
        let (kind, version, os) = client_from_agent_version("nimbus/unused/v24.1.0/linux");
        assert_eq!(kind, ClientKind::Nimbus);
        assert_eq!(version, "v24.1.0");
        assert_eq!(os, "linux");
    }

    #[test]
    fn parse_nim_libp2p_agent() {
        let (kind, version, os) = client_from_agent_version("nim-libp2p/v1.0.0/linux");
        assert_eq!(kind, ClientKind::Nimbus);
        assert_eq!(version, "v1.0.0");
        assert_eq!(os, "linux");
    }

    #[test]
    fn parse_lodestar_js_agent() {
        let (kind, version, _) = client_from_agent_version("js-libp2p/v0.45.0");
        assert_eq!(kind, ClientKind::Lodestar);
        assert_eq!(version, "v0.45.0");
    }

    #[test]
    fn parse_lodestar_named_agent() {
        let (kind, version, _) = client_from_agent_version("lodestar/v1.12.0");
        assert_eq!(kind, ClientKind::Lodestar);
        assert_eq!(version, "v1.12.0");
    }

    #[test]
    fn parse_caplin_agent() {
        let (kind, _, _) = client_from_agent_version("erigon/caplin");
        assert_eq!(kind, ClientKind::Caplin);
    }

    #[test]
    fn parse_erigon_non_caplin() {
        let (kind, _, _) = client_from_agent_version("erigon/other");
        assert_eq!(kind, ClientKind::Unknown);
    }

    #[test]
    fn parse_unknown_agent() {
        let (kind, version, os) = client_from_agent_version("something_random");
        assert_eq!(kind, ClientKind::Unknown);
        assert_eq!(version, "unknown");
        assert_eq!(os, "unknown");
    }

    #[test]
    fn parse_empty_agent() {
        let (kind, _, _) = client_from_agent_version("");
        assert_eq!(kind, ClientKind::Unknown);
    }

    #[test]
    fn parse_vibehouse_no_version() {
        let (kind, version, _) = client_from_agent_version("vibehouse");
        assert_eq!(kind, ClientKind::Vibehouse);
        assert_eq!(version, "unknown");
    }

    #[test]
    fn client_display_vibehouse() {
        let client = Client {
            kind: ClientKind::Vibehouse,
            version: "v1.0.0".into(),
            os_version: "linux".into(),
            protocol_version: "unknown".into(),
            agent_string: None,
        };
        let display = format!("{}", client);
        assert!(display.contains("vibehouse"));
        assert!(display.contains("v1.0.0"));
        assert!(display.contains("linux"));
    }

    #[test]
    fn client_display_unknown_with_agent() {
        let client = Client {
            kind: ClientKind::Unknown,
            version: "unknown".into(),
            os_version: "unknown".into(),
            protocol_version: "unknown".into(),
            agent_string: Some("mystery/v1".into()),
        };
        assert!(format!("{}", client).contains("mystery/v1"));
    }

    #[test]
    fn client_display_unknown_without_agent() {
        let client = Client {
            kind: ClientKind::Unknown,
            version: "unknown".into(),
            os_version: "unknown".into(),
            protocol_version: "unknown".into(),
            agent_string: None,
        };
        assert_eq!(format!("{}", client), "Unknown");
    }

    #[test]
    fn client_kind_display() {
        assert_eq!(format!("{}", ClientKind::Vibehouse), "Vibehouse");
        assert_eq!(format!("{}", ClientKind::Lighthouse), "Lighthouse");
        assert_eq!(format!("{}", ClientKind::Unknown), "Unknown");
    }

    #[test]
    fn client_kind_enum_iter() {
        let kinds: Vec<ClientKind> = ClientKind::iter().collect();
        assert!(kinds.contains(&ClientKind::Vibehouse));
        assert!(kinds.contains(&ClientKind::Lighthouse));
        assert!(kinds.contains(&ClientKind::Nimbus));
        assert!(kinds.contains(&ClientKind::Teku));
        assert!(kinds.contains(&ClientKind::Prysm));
        assert!(kinds.contains(&ClientKind::Lodestar));
        assert!(kinds.contains(&ClientKind::Caplin));
        assert!(kinds.contains(&ClientKind::Unknown));
        assert_eq!(kinds.len(), 8);
    }

    #[test]
    fn client_kind_as_ref_str() {
        assert_eq!(ClientKind::Vibehouse.as_ref(), "Vibehouse");
        assert_eq!(ClientKind::Lighthouse.as_ref(), "Lighthouse");
    }

    #[test]
    fn client_clone() {
        let client = Client {
            kind: ClientKind::Teku,
            version: "v24.1.0".into(),
            os_version: "linux".into(),
            protocol_version: "ipfs/0.1.0".into(),
            agent_string: Some("teku/teku/v24.1.0/linux".into()),
        };
        let cloned = client.clone();
        assert_eq!(cloned.kind, client.kind);
        assert_eq!(cloned.version, client.version);
        assert_eq!(cloned.agent_string, client.agent_string);
    }
}
