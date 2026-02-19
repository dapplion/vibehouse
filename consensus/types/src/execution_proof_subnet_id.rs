//! Identifies each execution proof subnet by an integer identifier.
//!
//! Each subnet corresponds to one type of execution proof (e.g., SP1, RISC Zero, Jolt).
//! Nodes subscribe to subnets for proof types they accept or generate.
use serde::{Deserialize, Serialize};
use ssz::{Decode, DecodeError, Encode};
use std::fmt::{self, Display};
use std::ops::{Deref, DerefMut};

/// Maximum number of execution proof subnets allowed by the protocol.
///
/// Each subnet corresponds to a distinct proof system (e.g., SP1, RISC Zero).
/// Set to 1 for initial rollout; will expand as more provers come online.
pub const MAX_EXECUTION_PROOF_SUBNETS: u64 = 1;

/// Identifies a specific execution proof subnet.
///
/// Also serves as the proof type identifier — one proof type per subnet.
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExecutionProofSubnetId(#[serde(with = "serde_utils::quoted_u64")] u64);

impl ExecutionProofSubnetId {
    pub fn new(id: u64) -> Result<Self, InvalidSubnetId> {
        if id >= MAX_EXECUTION_PROOF_SUBNETS {
            return Err(InvalidSubnetId(id));
        }
        Ok(Self(id))
    }
}

impl Display for ExecutionProofSubnetId {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.0)
    }
}

impl Deref for ExecutionProofSubnetId {
    type Target = u64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ExecutionProofSubnetId {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<ExecutionProofSubnetId> for u64 {
    fn from(val: ExecutionProofSubnetId) -> Self {
        val.0
    }
}

impl From<&ExecutionProofSubnetId> for u64 {
    fn from(val: &ExecutionProofSubnetId) -> Self {
        val.0
    }
}

#[derive(Debug)]
pub struct InvalidSubnetId(pub u64);

impl Display for InvalidSubnetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid execution proof subnet id: {}, must be < {}",
            self.0, MAX_EXECUTION_PROOF_SUBNETS
        )
    }
}

impl std::error::Error for InvalidSubnetId {}

impl Encode for ExecutionProofSubnetId {
    fn is_ssz_fixed_len() -> bool {
        <u64 as Encode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <u64 as Encode>::ssz_fixed_len()
    }

    fn ssz_bytes_len(&self) -> usize {
        self.0.ssz_bytes_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        self.0.ssz_append(buf)
    }
}

impl Decode for ExecutionProofSubnetId {
    fn is_ssz_fixed_len() -> bool {
        <u64 as Decode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <u64 as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        u64::from_ssz_bytes(bytes).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_subnet_ids() {
        for id in 0..MAX_EXECUTION_PROOF_SUBNETS {
            let subnet_id = ExecutionProofSubnetId::new(id).unwrap();
            assert_eq!(*subnet_id, id);
        }
    }

    #[test]
    fn invalid_subnet_ids() {
        assert!(ExecutionProofSubnetId::new(MAX_EXECUTION_PROOF_SUBNETS).is_err());
        assert!(ExecutionProofSubnetId::new(u64::MAX).is_err());
    }

    #[test]
    fn ssz_roundtrip() {
        let subnet_id = ExecutionProofSubnetId::new(0).unwrap();
        let encoded = subnet_id.as_ssz_bytes();
        let decoded = ExecutionProofSubnetId::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(subnet_id, decoded);
    }
}
