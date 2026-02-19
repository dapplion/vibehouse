//! Execution proof message for gossip.
//!
//! An execution proof cryptographically attests that an execution payload is valid,
//! enabling stateless validators to verify blocks without a local Execution Layer client.
//! Multiple proof types (one per subnet) can exist for a single execution payload.

use crate::execution_proof_subnet_id::ExecutionProofSubnetId;
use crate::{ExecutionBlockHash, Hash256};
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};

/// Maximum size of proof_data in bytes (1 MB).
///
/// Current zkEVM proofs (SP1, RISC Zero) produce proofs in the 100KB-500KB range.
/// 1MB provides headroom for future proof systems without allowing gossip abuse.
pub const MAX_EXECUTION_PROOF_SIZE: usize = 1_048_576;

/// A proof attesting to the validity of an execution payload.
///
/// If verified, this proof is equivalent to the EL returning `VALID` for `engine_newPayload`.
/// In ePBS, builders are the natural proof generators — they have the payload earliest and
/// can prove while other nodes are still waiting for the envelope reveal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
pub struct ExecutionProof {
    /// The beacon block root this proof is for.
    pub block_root: Hash256,
    /// The execution block hash this proof attests to.
    pub block_hash: ExecutionBlockHash,
    /// The subnet ID (also identifies the proof type).
    pub subnet_id: ExecutionProofSubnetId,
    /// Version of the proof format. Each subnet can upgrade independently.
    pub version: u64,
    /// Opaque proof data. Structure depends on subnet_id and version.
    #[serde(with = "serde_utils::hex_vec")]
    pub proof_data: Vec<u8>,
}

impl ExecutionProof {
    pub fn new(
        block_root: Hash256,
        block_hash: ExecutionBlockHash,
        subnet_id: ExecutionProofSubnetId,
        version: u64,
        proof_data: Vec<u8>,
    ) -> Self {
        Self {
            block_root,
            block_hash,
            subnet_id,
            version,
            proof_data,
        }
    }

    /// Check if this proof version is supported.
    pub fn is_version_supported(&self) -> bool {
        // Each subnet can upgrade its version independently.
        // For now, only version 1 is supported across all subnets.
        matches!(self.version, 1)
    }

    /// Validate basic structure: non-empty, supported version, within size limit.
    pub fn is_structurally_valid(&self) -> bool {
        !self.proof_data.is_empty()
            && self.proof_data.len() <= MAX_EXECUTION_PROOF_SIZE
            && self.is_version_supported()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_proof(version: u64, data: Vec<u8>) -> ExecutionProof {
        ExecutionProof::new(
            Hash256::random(),
            ExecutionBlockHash::from(Hash256::random()),
            ExecutionProofSubnetId::new(0).unwrap(),
            version,
            data,
        )
    }

    #[test]
    fn valid_proof() {
        let proof = make_proof(1, vec![1, 2, 3]);
        assert!(proof.is_version_supported());
        assert!(proof.is_structurally_valid());
    }

    #[test]
    fn invalid_version() {
        let proof = make_proof(99, vec![1, 2, 3]);
        assert!(!proof.is_version_supported());
        assert!(!proof.is_structurally_valid());
    }

    #[test]
    fn empty_proof_data() {
        let proof = make_proof(1, vec![]);
        assert!(proof.is_version_supported());
        assert!(!proof.is_structurally_valid());
    }

    #[test]
    fn oversized_proof_data() {
        let proof = make_proof(1, vec![0u8; MAX_EXECUTION_PROOF_SIZE + 1]);
        assert!(!proof.is_structurally_valid());
    }

    #[test]
    fn ssz_roundtrip() {
        use ssz::{Decode, Encode};

        let original = make_proof(1, vec![10, 20, 30, 40, 50]);
        let encoded = original.as_ssz_bytes();
        let decoded = ExecutionProof::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(original, decoded);
    }
}
