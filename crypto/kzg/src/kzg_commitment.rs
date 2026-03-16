use c_kzg::BYTES_PER_COMMITMENT;
use educe::Educe;
use ethereum_hashing::hash_fixed;
use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};
use ssz_derive::{Decode, Encode};
use std::fmt;
use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;
use tree_hash::{Hash256, PackedEncoding, TreeHash};

pub const VERSIONED_HASH_VERSION_KZG: u8 = 0x01;

#[derive(Educe, Clone, Copy, Encode, Decode)]
#[educe(PartialEq, Eq, Hash)]
#[ssz(struct_behaviour = "transparent")]
pub struct KzgCommitment(pub [u8; c_kzg::BYTES_PER_COMMITMENT]);

impl KzgCommitment {
    pub fn calculate_versioned_hash(&self) -> Hash256 {
        let mut versioned_hash = hash_fixed(&self.0);
        versioned_hash[0] = VERSIONED_HASH_VERSION_KZG;
        Hash256::from(versioned_hash)
    }

    pub fn empty_for_testing() -> Self {
        KzgCommitment([0; c_kzg::BYTES_PER_COMMITMENT])
    }
}

impl From<KzgCommitment> for c_kzg::Bytes48 {
    fn from(value: KzgCommitment) -> Self {
        value.0.into()
    }
}

impl Display for KzgCommitment {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "0x")?;
        for i in &self.0[0..2] {
            write!(f, "{:02x}", i)?;
        }
        write!(f, "…")?;
        for i in &self.0[BYTES_PER_COMMITMENT - 2..BYTES_PER_COMMITMENT] {
            write!(f, "{:02x}", i)?;
        }
        Ok(())
    }
}

impl TreeHash for KzgCommitment {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        <[u8; BYTES_PER_COMMITMENT] as TreeHash>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> PackedEncoding {
        self.0.tree_hash_packed_encoding()
    }

    fn tree_hash_packing_factor() -> usize {
        <[u8; BYTES_PER_COMMITMENT] as TreeHash>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> tree_hash::Hash256 {
        self.0.tree_hash_root()
    }
}

impl Serialize for KzgCommitment {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{:?}", self))
    }
}

impl<'de> Deserialize<'de> for KzgCommitment {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let string = String::deserialize(deserializer)?;
        Self::from_str(&string).map_err(serde::de::Error::custom)
    }
}

impl FromStr for KzgCommitment {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(stripped) = s.strip_prefix("0x") {
            let bytes = hex::decode(stripped).map_err(|e| e.to_string())?;
            if bytes.len() == BYTES_PER_COMMITMENT {
                let mut kzg_commitment_bytes = [0; BYTES_PER_COMMITMENT];
                kzg_commitment_bytes[..].copy_from_slice(&bytes);
                Ok(Self(kzg_commitment_bytes))
            } else {
                Err(format!(
                    "InvalidByteLength: got {}, expected {}",
                    bytes.len(),
                    BYTES_PER_COMMITMENT
                ))
            }
        } else {
            Err("must start with 0x".to_string())
        }
    }
}

impl Debug for KzgCommitment {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", serde_utils::hex::encode(self.0))
    }
}

#[cfg(feature = "arbitrary")]
impl arbitrary::Arbitrary<'_> for KzgCommitment {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        let mut bytes = [0u8; BYTES_PER_COMMITMENT];
        u.fill_buffer(&mut bytes)?;
        Ok(KzgCommitment(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssz::{Decode, Encode};

    fn sample_hex() -> &'static str {
        "0x53fa09af35d1d1a9e76f65e16112a9064ce30d1e4e2df98583f0f5dc2e7dd13a4f421a9c89f518fafd952df76f23adac"
    }

    fn sample_commitment() -> KzgCommitment {
        KzgCommitment::from_str(sample_hex()).unwrap()
    }

    #[test]
    fn from_str_valid() {
        let c = sample_commitment();
        assert_eq!(c.0[0], 0x53);
        assert_eq!(c.0[47], 0xac);
    }

    #[test]
    fn from_str_no_prefix() {
        let result = KzgCommitment::from_str(
            "53fa09af35d1d1a9e76f65e16112a9064ce30d1e4e2df98583f0f5dc2e7dd13a4f421a9c89f518fafd952df76f23adac",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must start with 0x"));
    }

    #[test]
    fn from_str_wrong_length() {
        let result = KzgCommitment::from_str("0xaabb");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("InvalidByteLength"));
    }

    #[test]
    fn from_str_invalid_hex() {
        let result = KzgCommitment::from_str("0xZZZZ");
        assert!(result.is_err());
    }

    #[test]
    fn from_str_empty_hex() {
        let result = KzgCommitment::from_str("0x");
        assert!(result.is_err());
    }

    #[test]
    fn display_format() {
        let c = sample_commitment();
        let display = c.to_string();
        assert_eq!(display, "0x53fa…adac");
    }

    #[test]
    fn debug_format() {
        let c = sample_commitment();
        let debug = format!("{:?}", c);
        assert_eq!(debug, sample_hex());
    }

    #[test]
    fn ssz_roundtrip() {
        let c = sample_commitment();
        let bytes = c.as_ssz_bytes();
        assert_eq!(bytes.len(), BYTES_PER_COMMITMENT);
        let decoded = KzgCommitment::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(c, decoded);
    }

    #[test]
    fn serde_roundtrip() {
        let c = sample_commitment();
        let json = serde_json::to_string(&c).unwrap();
        let decoded: KzgCommitment = serde_json::from_str(&json).unwrap();
        assert_eq!(c, decoded);
    }

    #[test]
    fn calculate_versioned_hash_prefix() {
        let c = sample_commitment();
        let vh = c.calculate_versioned_hash();
        assert_eq!(vh[0], VERSIONED_HASH_VERSION_KZG);
    }

    #[test]
    fn versioned_hash_different_commitments() {
        let a = sample_commitment();
        let b = KzgCommitment([0xFF; BYTES_PER_COMMITMENT]);
        assert_ne!(a.calculate_versioned_hash(), b.calculate_versioned_hash());
    }

    #[test]
    fn versioned_hash_deterministic() {
        let c = sample_commitment();
        assert_eq!(c.calculate_versioned_hash(), c.calculate_versioned_hash());
    }

    #[test]
    fn empty_for_testing() {
        let c = KzgCommitment::empty_for_testing();
        assert_eq!(c.0, [0u8; BYTES_PER_COMMITMENT]);
    }

    #[test]
    fn equality() {
        assert_eq!(sample_commitment(), sample_commitment());
    }

    #[test]
    fn inequality() {
        let a = sample_commitment();
        let mut b_bytes = a.0;
        b_bytes[0] ^= 0xFF;
        assert_ne!(a, KzgCommitment(b_bytes));
    }

    #[test]
    fn hash_usable_in_hashset() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(sample_commitment());
        set.insert(sample_commitment());
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn into_bytes48() {
        let c = sample_commitment();
        let bytes48: c_kzg::Bytes48 = c.into();
        assert_eq!(bytes48.as_ref(), &c.0[..]);
    }

    #[test]
    fn tree_hash_deterministic() {
        let c = sample_commitment();
        assert_eq!(c.tree_hash_root(), c.tree_hash_root());
    }

    #[test]
    fn tree_hash_different_for_different_commitments() {
        let a = sample_commitment();
        let b = KzgCommitment([0xFF; BYTES_PER_COMMITMENT]);
        assert_ne!(a.tree_hash_root(), b.tree_hash_root());
    }

    #[test]
    fn serde_json_format_is_hex_string() {
        let c = sample_commitment();
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.starts_with('"'));
        assert!(json.contains("0x"));
    }

    #[test]
    fn clone_preserves_value() {
        let c = sample_commitment();
        let cloned = c;
        assert_eq!(c, cloned);
    }
}
