use crate::{Hash256, consts::altair::NUM_FLAG_INDICES, test_utils::TestRandom};
use safe_arith::{ArithError, SafeArith};
use serde::{Deserialize, Serialize};
use ssz::{Decode, DecodeError, Encode};
use test_random_derive::TestRandom;
use tree_hash::{PackedEncoding, TreeHash, TreeHashType};

#[derive(Debug, Default, Clone, Copy, PartialEq, Deserialize, Serialize, TestRandom)]
#[serde(transparent)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct ParticipationFlags {
    #[serde(with = "serde_utils::quoted_u8")]
    bits: u8,
}

impl ParticipationFlags {
    pub fn add_flag(&mut self, flag_index: usize) -> Result<(), ArithError> {
        if flag_index >= NUM_FLAG_INDICES {
            return Err(ArithError::Overflow);
        }
        self.bits |= 1u8.safe_shl(flag_index as u32)?;
        Ok(())
    }

    pub fn has_flag(&self, flag_index: usize) -> Result<bool, ArithError> {
        if flag_index >= NUM_FLAG_INDICES {
            return Err(ArithError::Overflow);
        }
        let mask = 1u8.safe_shl(flag_index as u32)?;
        Ok(self.bits & mask == mask)
    }

    pub fn into_u8(self) -> u8 {
        self.bits
    }
}

/// Decode implementation that transparently behaves like the inner `u8`.
impl Decode for ParticipationFlags {
    fn is_ssz_fixed_len() -> bool {
        <u8 as Decode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <u8 as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        u8::from_ssz_bytes(bytes).map(|bits| Self { bits })
    }
}

/// Encode implementation that transparently behaves like the inner `u8`.
impl Encode for ParticipationFlags {
    fn is_ssz_fixed_len() -> bool {
        <u8 as Encode>::is_ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        self.bits.ssz_append(buf);
    }

    fn ssz_fixed_len() -> usize {
        <u8 as Encode>::ssz_fixed_len()
    }

    fn ssz_bytes_len(&self) -> usize {
        self.bits.ssz_bytes_len()
    }

    fn as_ssz_bytes(&self) -> Vec<u8> {
        self.bits.as_ssz_bytes()
    }
}

impl TreeHash for ParticipationFlags {
    fn tree_hash_type() -> TreeHashType {
        u8::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> PackedEncoding {
        self.bits.tree_hash_packed_encoding()
    }

    fn tree_hash_packing_factor() -> usize {
        u8::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> Hash256 {
        self.bits.tree_hash_root()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_zero() {
        let flags = ParticipationFlags::default();
        assert_eq!(flags.into_u8(), 0);
    }

    #[test]
    fn add_flag_sets_bit() {
        let mut flags = ParticipationFlags::default();
        flags.add_flag(0).unwrap();
        assert_eq!(flags.into_u8(), 0b001);

        let mut flags = ParticipationFlags::default();
        flags.add_flag(1).unwrap();
        assert_eq!(flags.into_u8(), 0b010);

        let mut flags = ParticipationFlags::default();
        flags.add_flag(2).unwrap();
        assert_eq!(flags.into_u8(), 0b100);
    }

    #[test]
    fn add_multiple_flags() {
        let mut flags = ParticipationFlags::default();
        flags.add_flag(0).unwrap();
        flags.add_flag(1).unwrap();
        flags.add_flag(2).unwrap();
        assert_eq!(flags.into_u8(), 0b111);
    }

    #[test]
    fn add_flag_idempotent() {
        let mut flags = ParticipationFlags::default();
        flags.add_flag(1).unwrap();
        flags.add_flag(1).unwrap();
        assert_eq!(flags.into_u8(), 0b010);
    }

    #[test]
    fn add_flag_out_of_range() {
        let mut flags = ParticipationFlags::default();
        assert!(flags.add_flag(NUM_FLAG_INDICES).is_err());
        assert!(flags.add_flag(NUM_FLAG_INDICES + 1).is_err());
        assert!(flags.add_flag(255).is_err());
    }

    #[test]
    fn has_flag_empty() {
        let flags = ParticipationFlags::default();
        assert!(!flags.has_flag(0).unwrap());
        assert!(!flags.has_flag(1).unwrap());
        assert!(!flags.has_flag(2).unwrap());
    }

    #[test]
    fn has_flag_after_set() {
        let mut flags = ParticipationFlags::default();
        flags.add_flag(1).unwrap();
        assert!(!flags.has_flag(0).unwrap());
        assert!(flags.has_flag(1).unwrap());
        assert!(!flags.has_flag(2).unwrap());
    }

    #[test]
    fn has_flag_out_of_range() {
        let flags = ParticipationFlags::default();
        assert!(flags.has_flag(NUM_FLAG_INDICES).is_err());
        assert!(flags.has_flag(255).is_err());
    }

    #[test]
    fn ssz_round_trip() {
        let mut flags = ParticipationFlags::default();
        flags.add_flag(0).unwrap();
        flags.add_flag(2).unwrap();

        let encoded = flags.as_ssz_bytes();
        assert_eq!(encoded.len(), 1);

        let decoded = ParticipationFlags::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(flags, decoded);
    }

    #[test]
    fn ssz_fixed_len() {
        assert!(<ParticipationFlags as Encode>::is_ssz_fixed_len());
        assert_eq!(<ParticipationFlags as Encode>::ssz_fixed_len(), 1);
    }

    #[test]
    fn ssz_decode_specific_value() {
        // 0b101 = flags 0 and 2 set
        let decoded = ParticipationFlags::from_ssz_bytes(&[0b101]).unwrap();
        assert!(decoded.has_flag(0).unwrap());
        assert!(!decoded.has_flag(1).unwrap());
        assert!(decoded.has_flag(2).unwrap());
    }

    #[test]
    fn tree_hash_matches_u8() {
        let mut flags = ParticipationFlags::default();
        flags.add_flag(0).unwrap();
        flags.add_flag(2).unwrap();
        assert_eq!(flags.tree_hash_root(), flags.into_u8().tree_hash_root());
    }

    #[test]
    fn serde_round_trip() {
        let mut flags = ParticipationFlags::default();
        flags.add_flag(1).unwrap();
        let json = serde_json::to_string(&flags).unwrap();
        let decoded: ParticipationFlags = serde_json::from_str(&json).unwrap();
        assert_eq!(flags, decoded);
    }

    #[test]
    fn equality() {
        let mut a = ParticipationFlags::default();
        let mut b = ParticipationFlags::default();
        assert_eq!(a, b);

        a.add_flag(0).unwrap();
        assert_ne!(a, b);

        b.add_flag(0).unwrap();
        assert_eq!(a, b);
    }
}
