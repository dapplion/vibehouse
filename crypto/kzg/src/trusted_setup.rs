use crate::{Error, PeerDASTrustedSetup};
use serde::{
    de::{self, Deserializer, Visitor},
    Deserialize, Serialize,
};

// Number of bytes per G1 point.
const BYTES_PER_G1_POINT: usize = 48;
// Number of bytes per G2 point.
const BYTES_PER_G2_POINT: usize = 96;

pub const TRUSTED_SETUP_BYTES: &[u8] = include_bytes!("../trusted_setup.json");

pub fn get_trusted_setup() -> Vec<u8> {
    TRUSTED_SETUP_BYTES.into()
}

/// Wrapper over a BLS G1 point's byte representation.
#[derive(Debug, Clone, PartialEq)]
struct G1Point([u8; BYTES_PER_G1_POINT]);

/// Wrapper over a BLS G2 point's byte representation.
#[derive(Debug, Clone, PartialEq)]
struct G2Point([u8; BYTES_PER_G2_POINT]);

/// Contains the trusted setup parameters that are required to instantiate a
/// `c_kzg::KzgSettings` object.
///
/// The serialize/deserialize implementations are written according to
/// the format specified in the ethereum consensus specs trusted setup files.
///
/// See <https://github.com/ethereum/consensus-specs/blob/dev/presets/mainnet/trusted_setups/trusted_setup_4096.json>
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrustedSetup {
    g1_monomial: Vec<G1Point>,
    g1_lagrange: Vec<G1Point>,
    g2_monomial: Vec<G2Point>,
}

impl TrustedSetup {
    pub fn g1_monomial(&self) -> Vec<u8> {
        self.g1_monomial.iter().flat_map(|p| p.0).collect()
    }

    pub fn g1_lagrange(&self) -> Vec<u8> {
        self.g1_lagrange.iter().flat_map(|p| p.0).collect()
    }

    pub fn g2_monomial(&self) -> Vec<u8> {
        self.g2_monomial.iter().flat_map(|p| p.0).collect()
    }

    pub fn g1_len(&self) -> usize {
        self.g1_lagrange.len()
    }
}

impl Serialize for G1Point {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let point = hex::encode(self.0);
        serializer.serialize_str(&point)
    }
}

impl Serialize for G2Point {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let point = hex::encode(self.0);
        serializer.serialize_str(&point)
    }
}

impl<'de> Deserialize<'de> for G1Point {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct G1PointVisitor;

        impl Visitor<'_> for G1PointVisitor {
            type Value = G1Point;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("A 48 byte hex encoded string")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let point = hex::decode(strip_prefix(v))
                    .map_err(|e| de::Error::custom(format!("Failed to decode G1 point: {}", e)))?;
                if point.len() != BYTES_PER_G1_POINT {
                    return Err(de::Error::custom(format!(
                        "G1 point has invalid length. Expected {} got {}",
                        BYTES_PER_G1_POINT,
                        point.len()
                    )));
                }
                let mut res = [0; BYTES_PER_G1_POINT];
                res.copy_from_slice(&point);
                Ok(G1Point(res))
            }
        }

        deserializer.deserialize_str(G1PointVisitor)
    }
}

impl<'de> Deserialize<'de> for G2Point {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct G2PointVisitor;

        impl Visitor<'_> for G2PointVisitor {
            type Value = G2Point;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("A 96 byte hex encoded string")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let point = hex::decode(strip_prefix(v))
                    .map_err(|e| de::Error::custom(format!("Failed to decode G2 point: {}", e)))?;
                if point.len() != BYTES_PER_G2_POINT {
                    return Err(de::Error::custom(format!(
                        "G2 point has invalid length. Expected {} got {}",
                        BYTES_PER_G2_POINT,
                        point.len()
                    )));
                }
                let mut res = [0; BYTES_PER_G2_POINT];
                res.copy_from_slice(&point);
                Ok(G2Point(res))
            }
        }

        deserializer.deserialize_str(G2PointVisitor)
    }
}

fn strip_prefix(s: &str) -> &str {
    if let Some(stripped) = s.strip_prefix("0x") {
        stripped
    } else {
        s
    }
}

/// Loads the trusted setup from JSON.
///
/// ## Note:
/// Currently we load both c-kzg and rust-eth-kzg trusted setup structs, because c-kzg is still being
/// used for 4844. Longer term we're planning to switch all KZG operations to the rust-eth-kzg
/// crate, and we'll be able to maintain a single trusted setup struct.
pub(crate) fn load_trusted_setup(
    trusted_setup: &[u8],
) -> Result<(TrustedSetup, PeerDASTrustedSetup), Error> {
    let ckzg_trusted_setup: TrustedSetup = serde_json::from_slice(trusted_setup)
        .map_err(|e| Error::TrustedSetupError(format!("{e:?}")))?;
    let trusted_setup_json = std::str::from_utf8(trusted_setup)
        .map_err(|e| Error::TrustedSetupError(format!("{e:?}")))?;
    let rkzg_trusted_setup = PeerDASTrustedSetup::from_json(trusted_setup_json);
    Ok((ckzg_trusted_setup, rkzg_trusted_setup))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_prefix_removes_0x() {
        assert_eq!(strip_prefix("0xabcd"), "abcd");
    }

    #[test]
    fn strip_prefix_no_prefix_unchanged() {
        assert_eq!(strip_prefix("abcd"), "abcd");
    }

    #[test]
    fn strip_prefix_empty_string() {
        assert_eq!(strip_prefix(""), "");
    }

    #[test]
    fn strip_prefix_only_0x() {
        assert_eq!(strip_prefix("0x"), "");
    }

    // --- G1Point serde ---

    #[test]
    fn g1_point_roundtrip() {
        let bytes = [42u8; BYTES_PER_G1_POINT];
        let point = G1Point(bytes);
        let json = serde_json::to_string(&point).unwrap();
        let deserialized: G1Point = serde_json::from_str(&json).unwrap();
        assert_eq!(point, deserialized);
    }

    #[test]
    fn g1_point_deserialize_with_0x_prefix() {
        let hex_str = format!("\"0x{}\"", "aa".repeat(BYTES_PER_G1_POINT));
        let point: G1Point = serde_json::from_str(&hex_str).unwrap();
        assert_eq!(point.0, [0xaa; BYTES_PER_G1_POINT]);
    }

    #[test]
    fn g1_point_deserialize_without_prefix() {
        let hex_str = format!("\"{}\"", "bb".repeat(BYTES_PER_G1_POINT));
        let point: G1Point = serde_json::from_str(&hex_str).unwrap();
        assert_eq!(point.0, [0xbb; BYTES_PER_G1_POINT]);
    }

    #[test]
    fn g1_point_wrong_length_fails() {
        let hex_str = format!("\"{}\"", "aa".repeat(BYTES_PER_G1_POINT - 1));
        let result = serde_json::from_str::<G1Point>(&hex_str);
        assert!(result.is_err());
    }

    #[test]
    fn g1_point_invalid_hex_fails() {
        let result = serde_json::from_str::<G1Point>("\"not_valid_hex\"");
        assert!(result.is_err());
    }

    // --- G2Point serde ---

    #[test]
    fn g2_point_roundtrip() {
        let bytes = [99u8; BYTES_PER_G2_POINT];
        let point = G2Point(bytes);
        let json = serde_json::to_string(&point).unwrap();
        let deserialized: G2Point = serde_json::from_str(&json).unwrap();
        assert_eq!(point, deserialized);
    }

    #[test]
    fn g2_point_deserialize_with_0x_prefix() {
        let hex_str = format!("\"0x{}\"", "cc".repeat(BYTES_PER_G2_POINT));
        let point: G2Point = serde_json::from_str(&hex_str).unwrap();
        assert_eq!(point.0, [0xcc; BYTES_PER_G2_POINT]);
    }

    #[test]
    fn g2_point_wrong_length_fails() {
        let hex_str = format!("\"{}\"", "aa".repeat(BYTES_PER_G2_POINT + 1));
        let result = serde_json::from_str::<G2Point>(&hex_str);
        assert!(result.is_err());
    }

    // --- TrustedSetup methods ---

    #[test]
    fn trusted_setup_g1_monomial_flattens_correctly() {
        let ts = TrustedSetup {
            g1_monomial: vec![
                G1Point([1; BYTES_PER_G1_POINT]),
                G1Point([2; BYTES_PER_G1_POINT]),
            ],
            g1_lagrange: vec![],
            g2_monomial: vec![],
        };
        let bytes = ts.g1_monomial();
        assert_eq!(bytes.len(), 2 * BYTES_PER_G1_POINT);
        assert!(bytes[..BYTES_PER_G1_POINT].iter().all(|&b| b == 1));
        assert!(bytes[BYTES_PER_G1_POINT..].iter().all(|&b| b == 2));
    }

    #[test]
    fn trusted_setup_g1_lagrange_flattens_correctly() {
        let ts = TrustedSetup {
            g1_monomial: vec![],
            g1_lagrange: vec![G1Point([3; BYTES_PER_G1_POINT])],
            g2_monomial: vec![],
        };
        let bytes = ts.g1_lagrange();
        assert_eq!(bytes.len(), BYTES_PER_G1_POINT);
        assert!(bytes.iter().all(|&b| b == 3));
    }

    #[test]
    fn trusted_setup_g2_monomial_flattens_correctly() {
        let ts = TrustedSetup {
            g1_monomial: vec![],
            g1_lagrange: vec![],
            g2_monomial: vec![G2Point([4; BYTES_PER_G2_POINT])],
        };
        let bytes = ts.g2_monomial();
        assert_eq!(bytes.len(), BYTES_PER_G2_POINT);
        assert!(bytes.iter().all(|&b| b == 4));
    }

    #[test]
    fn trusted_setup_g1_len() {
        let ts = TrustedSetup {
            g1_monomial: vec![G1Point([0; BYTES_PER_G1_POINT]); 5],
            g1_lagrange: vec![G1Point([0; BYTES_PER_G1_POINT]); 3],
            g2_monomial: vec![],
        };
        assert_eq!(ts.g1_len(), 3);
    }

    #[test]
    fn trusted_setup_empty() {
        let ts = TrustedSetup {
            g1_monomial: vec![],
            g1_lagrange: vec![],
            g2_monomial: vec![],
        };
        assert_eq!(ts.g1_monomial().len(), 0);
        assert_eq!(ts.g1_lagrange().len(), 0);
        assert_eq!(ts.g2_monomial().len(), 0);
        assert_eq!(ts.g1_len(), 0);
    }

    // --- TrustedSetup full JSON roundtrip ---

    #[test]
    fn trusted_setup_json_roundtrip() {
        let ts = TrustedSetup {
            g1_monomial: vec![G1Point([0xab; BYTES_PER_G1_POINT])],
            g1_lagrange: vec![G1Point([0xcd; BYTES_PER_G1_POINT])],
            g2_monomial: vec![G2Point([0xef; BYTES_PER_G2_POINT])],
        };
        let json = serde_json::to_string(&ts).unwrap();
        let deserialized: TrustedSetup = serde_json::from_str(&json).unwrap();
        assert_eq!(ts, deserialized);
    }

    // --- get_trusted_setup ---

    #[test]
    fn get_trusted_setup_returns_nonempty_bytes() {
        let bytes = get_trusted_setup();
        assert!(!bytes.is_empty());
    }

    // --- load_trusted_setup ---

    #[test]
    fn load_trusted_setup_from_embedded_succeeds() {
        let result = load_trusted_setup(TRUSTED_SETUP_BYTES);
        assert!(result.is_ok());
        let (ts, _) = result.unwrap();
        assert!(ts.g1_len() > 0);
    }

    #[test]
    fn load_trusted_setup_invalid_json_fails() {
        let result = load_trusted_setup(b"not json");
        assert!(result.is_err());
    }
}
