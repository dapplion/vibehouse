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

/// Proof version for stub proofs (test/development).
pub const PROOF_VERSION_STUB: u64 = 1;

/// Proof version for SP1 Groth16 proofs.
pub const PROOF_VERSION_SP1_GROTH16: u64 = 2;

/// Minimum size of proof_data for SP1 Groth16 proofs.
///
/// Layout: vkey_hash (32 bytes) + groth16_proof_length (4 bytes) + at least 1 byte of groth16
/// proof + public_values (96 bytes).
pub const SP1_GROTH16_MIN_PROOF_DATA_SIZE: usize = 32 + 4 + 1 + EXECUTION_PROOF_PUBLIC_VALUES_SIZE;

/// Parsed SP1 Groth16 proof components extracted from proof_data.
pub struct Sp1Groth16ProofData<'a> {
    /// The SP1 program verification key hash (32 bytes).
    pub vkey_hash: &'a [u8; 32],
    /// The raw Groth16 proof bytes.
    pub groth16_proof: &'a [u8],
    /// The SP1 public values (contains the proven block hash).
    pub public_values: &'a [u8],
}

/// The public values committed by the vibehouse SP1 guest program.
///
/// The guest program re-executes an Ethereum block inside the SP1 zkVM,
/// then commits these values as the proof's public output. The CL verifier
/// checks that `block_hash` matches the proof's claimed execution block hash.
///
/// Layout: 96 bytes total, simple concatenation of three 32-byte hashes.
///
/// ```text
/// [0..32]   block_hash   — keccak256 of the RLP-encoded block header
/// [32..64]  parent_hash  — parent block's hash (chain continuity)
/// [64..96]  state_root   — post-execution state root (proven correct)
/// ```
///
/// This struct is used by both the guest program (to commit) and the CL
/// verifier (to parse and cross-check).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionProofPublicValues {
    /// The execution block hash (keccak256 of RLP-encoded header).
    pub block_hash: [u8; 32],
    /// The parent block hash.
    pub parent_hash: [u8; 32],
    /// The post-execution state root, proven correct by re-execution.
    pub state_root: [u8; 32],
}

/// Size of serialized `ExecutionProofPublicValues` in bytes.
pub const EXECUTION_PROOF_PUBLIC_VALUES_SIZE: usize = 96;

impl ExecutionProofPublicValues {
    /// Serialize to a fixed-size byte array.
    pub fn to_bytes(&self) -> [u8; EXECUTION_PROOF_PUBLIC_VALUES_SIZE] {
        let mut buf = [0u8; EXECUTION_PROOF_PUBLIC_VALUES_SIZE];
        buf[..32].copy_from_slice(&self.block_hash);
        buf[32..64].copy_from_slice(&self.parent_hash);
        buf[64..96].copy_from_slice(&self.state_root);
        buf
    }

    /// Deserialize from bytes. Returns `None` if the slice is too short.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < EXECUTION_PROOF_PUBLIC_VALUES_SIZE {
            return None;
        }
        Some(Self {
            block_hash: bytes.get(..32)?.try_into().ok()?,
            parent_hash: bytes.get(32..64)?.try_into().ok()?,
            state_root: bytes.get(64..96)?.try_into().ok()?,
        })
    }

    /// Extract the block hash as an `ExecutionBlockHash`.
    pub fn execution_block_hash(&self) -> ExecutionBlockHash {
        ExecutionBlockHash::from_root(Hash256::from(self.block_hash))
    }
}

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
        matches!(self.version, PROOF_VERSION_STUB | PROOF_VERSION_SP1_GROTH16)
    }

    /// Validate basic structure: non-empty, supported version, within size limit.
    pub fn is_structurally_valid(&self) -> bool {
        if !self.is_version_supported() || self.proof_data.is_empty() {
            return false;
        }
        if self.proof_data.len() > MAX_EXECUTION_PROOF_SIZE {
            return false;
        }
        // Version 2 (SP1 Groth16) has additional structural requirements.
        if self.version == PROOF_VERSION_SP1_GROTH16 {
            return self.proof_data.len() >= SP1_GROTH16_MIN_PROOF_DATA_SIZE;
        }
        true
    }

    /// Parse SP1 Groth16 proof_data into its components.
    ///
    /// proof_data layout (version 2):
    ///   [0..32]                         vkey_hash (32 bytes, raw)
    ///   [32..36]                        groth16_proof_length (u32 big-endian)
    ///   [36..36+proof_len]              groth16_proof_bytes
    ///   [36+proof_len..]               sp1_public_values
    ///
    /// Returns `None` if the proof data is too short or the length field is invalid.
    pub fn parse_sp1_groth16(&self) -> Option<Sp1Groth16ProofData<'_>> {
        if self.version != PROOF_VERSION_SP1_GROTH16 {
            return None;
        }
        if self.proof_data.len() < SP1_GROTH16_MIN_PROOF_DATA_SIZE {
            return None;
        }

        let vkey_hash: &[u8; 32] = self.proof_data.get(..32)?.try_into().ok()?;
        let proof_len_bytes: [u8; 4] = self.proof_data.get(32..36)?.try_into().ok()?;
        let proof_len = u32::from_be_bytes(proof_len_bytes) as usize;

        let proof_end = 36_usize.checked_add(proof_len)?;
        if proof_end > self.proof_data.len() {
            return None;
        }

        let groth16_proof = self.proof_data.get(36..proof_end)?;
        let public_values = self.proof_data.get(proof_end..)?;

        Some(Sp1Groth16ProofData {
            vkey_hash,
            groth16_proof,
            public_values,
        })
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

    /// Build a valid SP1 Groth16 proof_data blob.
    fn make_sp1_groth16_data(
        vkey_hash: [u8; 32],
        groth16_proof: &[u8],
        public_values: &[u8],
    ) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&vkey_hash);
        data.extend_from_slice(&(groth16_proof.len() as u32).to_be_bytes());
        data.extend_from_slice(groth16_proof);
        data.extend_from_slice(public_values);
        data
    }

    #[test]
    fn valid_stub_proof() {
        let proof = make_proof(1, vec![1, 2, 3]);
        assert!(proof.is_version_supported());
        assert!(proof.is_structurally_valid());
    }

    #[test]
    fn valid_sp1_groth16_proof() {
        let pv = ExecutionProofPublicValues {
            block_hash: [0x11; 32],
            parent_hash: [0x22; 32],
            state_root: [0x33; 32],
        };
        let data = make_sp1_groth16_data([0xaa; 32], &[1, 2, 3], &pv.to_bytes());
        let proof = make_proof(PROOF_VERSION_SP1_GROTH16, data);
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
    fn sp1_groth16_too_short() {
        // Version 2 proof with data shorter than the minimum size.
        // SP1_GROTH16_MIN_PROOF_DATA_SIZE = 32 + 4 + 1 + 96 = 133
        let proof = make_proof(PROOF_VERSION_SP1_GROTH16, vec![0u8; 132]);
        assert!(proof.is_version_supported());
        assert!(!proof.is_structurally_valid());
    }

    #[test]
    fn sp1_groth16_parse_valid() {
        let vkey = [0xbb; 32];
        let groth16_bytes = vec![10, 20, 30, 40];
        let pv = ExecutionProofPublicValues {
            block_hash: [0x11; 32],
            parent_hash: [0x22; 32],
            state_root: [0x33; 32],
        };
        let pv_bytes = pv.to_bytes();
        let data = make_sp1_groth16_data(vkey, &groth16_bytes, &pv_bytes);

        let proof = make_proof(PROOF_VERSION_SP1_GROTH16, data);
        let parsed = proof.parse_sp1_groth16().expect("should parse");

        assert_eq!(parsed.vkey_hash, &vkey);
        assert_eq!(parsed.groth16_proof, &groth16_bytes);
        assert_eq!(parsed.public_values, &pv_bytes);

        // Parse public values from the proof.
        let parsed_pv =
            ExecutionProofPublicValues::from_bytes(parsed.public_values).expect("should parse pv");
        assert_eq!(parsed_pv, pv);
    }

    #[test]
    fn sp1_groth16_parse_truncated_proof_len() {
        // proof_len says 100 bytes but proof_data only has a few bytes after header.
        let mut data = vec![0u8; 32]; // vkey_hash
        data.extend_from_slice(&100u32.to_be_bytes()); // claims 100 bytes of proof
        data.extend_from_slice(&[1, 2, 3]); // only 3 bytes available

        let proof = make_proof(PROOF_VERSION_SP1_GROTH16, data);
        assert!(proof.parse_sp1_groth16().is_none());
    }

    #[test]
    fn sp1_groth16_parse_wrong_version() {
        let pv = ExecutionProofPublicValues {
            block_hash: [0; 32],
            parent_hash: [0; 32],
            state_root: [0; 32],
        };
        let data = make_sp1_groth16_data([0; 32], &[1, 2, 3], &pv.to_bytes());
        let proof = make_proof(PROOF_VERSION_STUB, data);
        assert!(proof.parse_sp1_groth16().is_none());
    }

    #[test]
    fn ssz_roundtrip() {
        use ssz::{Decode, Encode};

        let original = make_proof(1, vec![10, 20, 30, 40, 50]);
        let encoded = original.as_ssz_bytes();
        let decoded = ExecutionProof::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn ssz_roundtrip_sp1_groth16() {
        use ssz::{Decode, Encode};

        let pv = ExecutionProofPublicValues {
            block_hash: [0xaa; 32],
            parent_hash: [0xbb; 32],
            state_root: [0xcc; 32],
        };
        let data = make_sp1_groth16_data([0xdd; 32], &[1; 256], &pv.to_bytes());
        let original = make_proof(PROOF_VERSION_SP1_GROTH16, data);
        let encoded = original.as_ssz_bytes();
        let decoded = ExecutionProof::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(original, decoded);

        // Verify parsed components survive roundtrip.
        let parsed = decoded
            .parse_sp1_groth16()
            .expect("should parse after SSZ roundtrip");
        assert_eq!(parsed.vkey_hash, &[0xdd; 32]);
        assert_eq!(parsed.groth16_proof.len(), 256);
        assert_eq!(
            parsed.public_values.len(),
            EXECUTION_PROOF_PUBLIC_VALUES_SIZE
        );
    }

    #[test]
    fn public_values_roundtrip() {
        let pv = ExecutionProofPublicValues {
            block_hash: [0x11; 32],
            parent_hash: [0x22; 32],
            state_root: [0x33; 32],
        };
        let bytes = pv.to_bytes();
        assert_eq!(bytes.len(), EXECUTION_PROOF_PUBLIC_VALUES_SIZE);

        let parsed = ExecutionProofPublicValues::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, pv);
    }

    #[test]
    fn public_values_from_bytes_too_short() {
        assert!(ExecutionProofPublicValues::from_bytes(&[0u8; 95]).is_none());
    }

    #[test]
    fn public_values_execution_block_hash() {
        let block_hash_bytes = [0x42; 32];
        let pv = ExecutionProofPublicValues {
            block_hash: block_hash_bytes,
            parent_hash: [0; 32],
            state_root: [0; 32],
        };
        let ebh = pv.execution_block_hash();
        assert_eq!(
            ebh,
            ExecutionBlockHash::from_root(Hash256::from(block_hash_bytes))
        );
    }

    #[test]
    fn public_values_extra_bytes_ignored() {
        // from_bytes should work with more than 96 bytes (ignoring extras).
        let mut bytes = [0u8; 128];
        bytes[..32].copy_from_slice(&[0xaa; 32]);
        bytes[32..64].copy_from_slice(&[0xbb; 32]);
        bytes[64..96].copy_from_slice(&[0xcc; 32]);

        let pv = ExecutionProofPublicValues::from_bytes(&bytes).unwrap();
        assert_eq!(pv.block_hash, [0xaa; 32]);
        assert_eq!(pv.parent_hash, [0xbb; 32]);
        assert_eq!(pv.state_root, [0xcc; 32]);
    }
}
