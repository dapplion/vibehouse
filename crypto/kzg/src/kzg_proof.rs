use c_kzg::BYTES_PER_PROOF;
use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};
use ssz_derive::{Decode, Encode};
use std::fmt;
use std::fmt::Debug;
use std::str::FromStr;
use tree_hash::{PackedEncoding, TreeHash};

#[derive(PartialEq, Hash, Clone, Copy, Encode, Decode)]
#[ssz(struct_behaviour = "transparent")]
pub struct KzgProof(pub [u8; BYTES_PER_PROOF]);

impl From<KzgProof> for c_kzg::Bytes48 {
    fn from(value: KzgProof) -> Self {
        value.0.into()
    }
}

impl KzgProof {
    /// Creates a valid proof using `G1_POINT_AT_INFINITY`.
    pub fn empty() -> Self {
        let mut bytes = [0; BYTES_PER_PROOF];
        bytes[0] = 0xc0;
        Self(bytes)
    }
}

impl fmt::Display for KzgProof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", serde_utils::hex::encode(self.0))
    }
}

impl From<[u8; BYTES_PER_PROOF]> for KzgProof {
    fn from(bytes: [u8; BYTES_PER_PROOF]) -> Self {
        Self(bytes)
    }
}

impl From<KzgProof> for [u8; BYTES_PER_PROOF] {
    fn from(from: KzgProof) -> [u8; BYTES_PER_PROOF] {
        from.0
    }
}

impl TreeHash for KzgProof {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        <[u8; BYTES_PER_PROOF]>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> PackedEncoding {
        self.0.tree_hash_packed_encoding()
    }

    fn tree_hash_packing_factor() -> usize {
        <[u8; BYTES_PER_PROOF]>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> tree_hash::Hash256 {
        self.0.tree_hash_root()
    }
}

impl Serialize for KzgProof {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for KzgProof {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let string = String::deserialize(deserializer)?;
        Self::from_str(&string).map_err(serde::de::Error::custom)
    }
}

impl FromStr for KzgProof {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(stripped) = s.strip_prefix("0x") {
            let bytes = hex::decode(stripped).map_err(|e| e.to_string())?;
            if bytes.len() == BYTES_PER_PROOF {
                let mut kzg_proof_bytes = [0; BYTES_PER_PROOF];
                kzg_proof_bytes[..].copy_from_slice(&bytes);
                Ok(Self(kzg_proof_bytes))
            } else {
                Err(format!(
                    "InvalidByteLength: got {}, expected {}",
                    bytes.len(),
                    BYTES_PER_PROOF
                ))
            }
        } else {
            Err("must start with 0x".to_string())
        }
    }
}

impl Debug for KzgProof {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", serde_utils::hex::encode(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssz::{Decode, Encode};

    fn sample_hex() -> &'static str {
        "0xb4021b0de10f743893d4f71e1bf830c8e7f4c01c67456f1783b09a1c6d5e0cd0a4a36e52c68700e461024b0e21e580e0"
    }

    fn sample_proof() -> KzgProof {
        KzgProof::from_str(sample_hex()).unwrap()
    }

    #[test]
    fn from_str_valid() {
        let p = sample_proof();
        assert_eq!(p.0[0], 0xb4);
        assert_eq!(p.0[47], 0xe0);
    }

    #[test]
    fn from_str_no_prefix() {
        let result = KzgProof::from_str(
            "b4021b0de10f743893d4f71e1bf830c8e7f4c01c67456f1783b09a1c6d5e0cd0a4a36e52c68700e461024b0e21e580e0",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must start with 0x"));
    }

    #[test]
    fn from_str_wrong_length() {
        let result = KzgProof::from_str("0xaabb");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("InvalidByteLength"));
    }

    #[test]
    fn from_str_invalid_hex() {
        let result = KzgProof::from_str("0xZZZZ");
        assert!(result.is_err());
    }

    #[test]
    fn from_str_empty_hex() {
        let result = KzgProof::from_str("0x");
        assert!(result.is_err());
    }

    #[test]
    fn display_format() {
        let p = sample_proof();
        let display = p.to_string();
        assert!(display.starts_with("0x"));
        assert_eq!(display, sample_hex());
    }

    #[test]
    fn debug_format() {
        let p = sample_proof();
        let debug = format!("{:?}", p);
        assert_eq!(debug, sample_hex());
    }

    #[test]
    fn ssz_roundtrip() {
        let p = sample_proof();
        let bytes = p.as_ssz_bytes();
        assert_eq!(bytes.len(), BYTES_PER_PROOF);
        let decoded = KzgProof::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(p, decoded);
    }

    #[test]
    fn serde_roundtrip() {
        let p = sample_proof();
        let json = serde_json::to_string(&p).unwrap();
        let decoded: KzgProof = serde_json::from_str(&json).unwrap();
        assert_eq!(p, decoded);
    }

    #[test]
    fn serde_json_format_is_hex_string() {
        let p = sample_proof();
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.starts_with('"'));
        assert!(json.contains("0x"));
    }

    #[test]
    fn empty_proof() {
        let p = KzgProof::empty();
        assert_eq!(p.0[0], 0xc0);
        for &b in &p.0[1..] {
            assert_eq!(b, 0);
        }
    }

    #[test]
    fn equality() {
        assert_eq!(sample_proof(), sample_proof());
    }

    #[test]
    fn inequality() {
        let a = sample_proof();
        let mut b_bytes = a.0;
        b_bytes[0] ^= 0xFF;
        assert_ne!(a, KzgProof(b_bytes));
    }

    #[test]
    fn hash_consistent() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h1 = DefaultHasher::new();
        let mut h2 = DefaultHasher::new();
        sample_proof().hash(&mut h1);
        sample_proof().hash(&mut h2);
        assert_eq!(h1.finish(), h2.finish());
    }

    #[test]
    fn into_bytes48() {
        let p = sample_proof();
        let bytes48: c_kzg::Bytes48 = p.into();
        assert_eq!(bytes48.as_ref(), &p.0[..]);
    }

    #[test]
    fn from_byte_array() {
        let bytes = [0xAB; BYTES_PER_PROOF];
        let p = KzgProof::from(bytes);
        assert_eq!(p.0, bytes);
    }

    #[test]
    fn into_byte_array() {
        let p = sample_proof();
        let bytes: [u8; BYTES_PER_PROOF] = p.into();
        assert_eq!(bytes, p.0);
    }

    #[test]
    fn tree_hash_deterministic() {
        let p = sample_proof();
        assert_eq!(p.tree_hash_root(), p.tree_hash_root());
    }

    #[test]
    fn tree_hash_different_for_different_proofs() {
        let a = sample_proof();
        let b = KzgProof([0xFF; BYTES_PER_PROOF]);
        assert_ne!(a.tree_hash_root(), b.tree_hash_root());
    }

    #[test]
    fn clone_preserves_value() {
        let p = sample_proof();
        let cloned = p;
        assert_eq!(p, cloned);
    }
}

#[cfg(feature = "arbitrary")]
impl arbitrary::Arbitrary<'_> for KzgProof {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        let mut bytes = [0u8; BYTES_PER_PROOF];
        u.fill_buffer(&mut bytes)?;
        Ok(KzgProof(bytes))
    }
}
