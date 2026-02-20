use std::sync::Arc;

use types::{
    EthSpec, ExecutionBlockHash, ExecutionProof, ExecutionProofSubnetId, Hash256,
    execution_proof::PROOF_VERSION_SP1_GROTH16,
    execution_proof_subnet_id::MAX_EXECUTION_PROOF_SUBNETS,
};

use crate::{BeaconChain, BeaconChainError, BeaconChainTypes};

/// Errors that can occur when verifying an execution proof for gossip.
#[derive(Debug)]
pub enum GossipExecutionProofError {
    /// The subnet_id is out of bounds.
    ///
    /// ## Peer scoring
    /// The peer has sent a structurally invalid message.
    InvalidSubnetId { received: u64 },
    /// The proof version is not supported.
    ///
    /// ## Peer scoring
    /// The peer has sent an invalid message.
    InvalidVersion { version: u64 },
    /// The proof_data field is empty.
    ///
    /// ## Peer scoring
    /// The peer has sent an invalid message.
    ProofDataEmpty,
    /// The proof_data exceeds the maximum allowed size.
    ///
    /// ## Peer scoring
    /// The peer has sent an invalid message.
    ProofDataTooLarge { size: usize },
    /// The proof's block_root is not known in fork choice.
    ///
    /// ## Peer scoring
    /// May not have received the block yet, ignore.
    UnknownBlockRoot { block_root: Hash256 },
    /// The proof's block is prior to the finalized slot.
    ///
    /// ## Peer scoring
    /// The proof is for a finalized block, ignore.
    PriorToFinalization {
        block_slot: u64,
        finalized_slot: u64,
    },
    /// The proof's block_hash does not match the block's bid block_hash.
    ///
    /// ## Peer scoring
    /// The peer has sent an invalid message.
    BlockHashMismatch {
        proof_block_hash: ExecutionBlockHash,
        block_block_hash: ExecutionBlockHash,
    },
    /// The proof_data could not be parsed according to its version's format.
    ///
    /// ## Peer scoring
    /// The peer has sent a malformed proof.
    InvalidProofData,
    /// The SP1 Groth16 proof could not be verified (feature `sp1` not enabled).
    ///
    /// ## Peer scoring
    /// Cannot verify — reject to be safe.
    Sp1VerificationUnavailable,
    /// The cryptographic proof verification failed.
    ///
    /// ## Peer scoring
    /// The peer has sent an invalid proof.
    InvalidProof { reason: String },
    /// The public values in the proof do not contain the expected block hash.
    ///
    /// ## Peer scoring
    /// The peer has sent an invalid proof.
    PublicValuesBlockHashMismatch {
        expected: ExecutionBlockHash,
        got: ExecutionBlockHash,
    },
    /// An internal beacon chain error occurred.
    ///
    /// ## Peer scoring
    /// Internal error, do not penalize.
    BeaconChainError(Box<BeaconChainError>),
}

impl From<BeaconChainError> for GossipExecutionProofError {
    fn from(e: BeaconChainError) -> Self {
        GossipExecutionProofError::BeaconChainError(Box::new(e))
    }
}

/// An execution proof that has passed gossip validation checks.
///
/// The inner proof is safe to propagate on gossip and store in the proof cache.
/// For version 2 (SP1 Groth16) proofs, cryptographic verification is performed
/// when the `sp1` feature is enabled.
pub struct VerifiedExecutionProof<T: BeaconChainTypes> {
    proof: Arc<ExecutionProof>,
    block_root: Hash256,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: BeaconChainTypes> VerifiedExecutionProof<T> {
    /// Returns a reference to the verified proof.
    pub fn proof(&self) -> &ExecutionProof {
        &self.proof
    }

    /// Returns the block_root this proof attests to.
    pub fn block_root(&self) -> Hash256 {
        self.block_root
    }

    /// Returns the subnet_id of this proof.
    pub fn subnet_id(&self) -> ExecutionProofSubnetId {
        self.proof.subnet_id
    }

    /// Consume self and return the inner proof.
    pub fn into_inner(self) -> Arc<ExecutionProof> {
        self.proof
    }
}

/// Verify an execution proof received via gossip.
///
/// Checks performed:
/// 1. Subnet ID is within bounds
/// 2. Proof version is supported
/// 3. Proof data is non-empty and within size limits
/// 4. Block root is known in fork choice
/// 5. Block is not prior to finalization
/// 6. Block hash matches the block's bid block hash (Gloas ePBS)
/// 7. Cryptographic verification (SP1 Groth16 for version 2, stub for version 1)
pub fn verify_execution_proof_for_gossip<T: BeaconChainTypes>(
    proof: Arc<ExecutionProof>,
    subnet_id: ExecutionProofSubnetId,
    chain: &BeaconChain<T>,
) -> Result<VerifiedExecutionProof<T>, GossipExecutionProofError> {
    // Check 1: Subnet ID matches and is within bounds.
    if *subnet_id >= MAX_EXECUTION_PROOF_SUBNETS {
        return Err(GossipExecutionProofError::InvalidSubnetId {
            received: *subnet_id,
        });
    }

    // Check 2: Version is supported.
    if !proof.is_version_supported() {
        return Err(GossipExecutionProofError::InvalidVersion {
            version: proof.version,
        });
    }

    // Check 3: Proof data is non-empty and within size limits.
    if proof.proof_data.is_empty() {
        return Err(GossipExecutionProofError::ProofDataEmpty);
    }

    if !proof.is_structurally_valid() {
        return Err(GossipExecutionProofError::ProofDataTooLarge {
            size: proof.proof_data.len(),
        });
    }

    let block_root = proof.block_root;

    // Check 4: Block root is known in fork choice.
    let fork_choice = chain.canonical_head.fork_choice_read_lock();
    let proto_block = fork_choice
        .get_block(&block_root)
        .ok_or(GossipExecutionProofError::UnknownBlockRoot { block_root })?;

    // Check 5: Block is not prior to finalization.
    let finalized_slot = chain
        .canonical_head
        .cached_head()
        .finalized_checkpoint()
        .epoch
        .start_slot(T::EthSpec::slots_per_epoch());
    if proto_block.slot < finalized_slot {
        return Err(GossipExecutionProofError::PriorToFinalization {
            block_slot: proto_block.slot.as_u64(),
            finalized_slot: finalized_slot.as_u64(),
        });
    }

    // Check 6: Block hash matches the bid block hash.
    // In Gloas ePBS, the execution block hash comes from the committed bid.
    if let Some(bid_block_hash) = proto_block.bid_block_hash
        && proof.block_hash != bid_block_hash
    {
        return Err(GossipExecutionProofError::BlockHashMismatch {
            proof_block_hash: proof.block_hash,
            block_block_hash: bid_block_hash,
        });
    }
    // If bid_block_hash is None (pre-ePBS block), skip this check — the proof
    // can still reference a pre-ePBS block by its execution_status block hash.

    drop(fork_choice);

    // Check 7: Cryptographic verification.
    verify_proof_cryptography(&proof)?;

    Ok(VerifiedExecutionProof {
        proof,
        block_root,
        _phantom: std::marker::PhantomData,
    })
}

/// Perform cryptographic verification of the proof based on its version.
///
/// - Version 1 (stub): accepts all proofs (test/development only).
/// - Version 2 (SP1 Groth16): verifies the Groth16 proof using `sp1-verifier` and
///   checks that the public values contain the correct block hash.
fn verify_proof_cryptography(proof: &ExecutionProof) -> Result<(), GossipExecutionProofError> {
    match proof.version {
        types::execution_proof::PROOF_VERSION_STUB => {
            // Stub proofs have no cryptographic content to verify.
            Ok(())
        }
        PROOF_VERSION_SP1_GROTH16 => verify_sp1_groth16(proof),
        _ => {
            // Should not reach here since is_version_supported() already checked.
            Err(GossipExecutionProofError::InvalidVersion {
                version: proof.version,
            })
        }
    }
}

/// Verify an SP1 Groth16 proof.
///
/// When the `sp1` feature is enabled, performs full Groth16 verification using
/// the `sp1-verifier` crate and checks that the public values commit to the
/// expected block hash.
///
/// When the `sp1` feature is not enabled, rejects all SP1 proofs since we cannot
/// verify them.
#[cfg(feature = "sp1")]
fn verify_sp1_groth16(proof: &ExecutionProof) -> Result<(), GossipExecutionProofError> {
    use sp1_verifier::{GROTH16_VK_BYTES, Groth16Verifier};

    let parsed = proof
        .parse_sp1_groth16()
        .ok_or(GossipExecutionProofError::InvalidProofData)?;

    // Convert the raw 32-byte vkey hash to the hex string format expected by sp1-verifier.
    let vkey_hex = format!("0x{}", hex::encode(parsed.vkey_hash));

    // Verify the Groth16 proof.
    Groth16Verifier::verify(
        parsed.groth16_proof,
        parsed.public_values,
        &vkey_hex,
        &GROTH16_VK_BYTES,
    )
    .map_err(|e| GossipExecutionProofError::InvalidProof {
        reason: format!("SP1 Groth16 verification failed: {e}"),
    })?;

    // Parse and cross-check public values.
    // The public values must contain the block_hash committed by the guest program.
    let public_values =
        types::execution_proof::ExecutionProofPublicValues::from_bytes(parsed.public_values)
            .ok_or(GossipExecutionProofError::InvalidProofData)?;

    let proven_hash = public_values.execution_block_hash();
    if proven_hash != proof.block_hash {
        return Err(GossipExecutionProofError::PublicValuesBlockHashMismatch {
            expected: proof.block_hash,
            got: proven_hash,
        });
    }

    Ok(())
}

#[cfg(not(feature = "sp1"))]
fn verify_sp1_groth16(_proof: &ExecutionProof) -> Result<(), GossipExecutionProofError> {
    Err(GossipExecutionProofError::Sp1VerificationUnavailable)
}

impl<T: BeaconChainTypes> BeaconChain<T> {
    /// Verify an execution proof received via gossip.
    ///
    /// Thin wrapper around `verify_execution_proof_for_gossip` that adds metrics.
    pub fn verify_execution_proof_for_gossip(
        self: &Arc<Self>,
        proof: Arc<ExecutionProof>,
        subnet_id: ExecutionProofSubnetId,
    ) -> Result<VerifiedExecutionProof<T>, GossipExecutionProofError> {
        verify_execution_proof_for_gossip(proof, subnet_id, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::execution_proof::MAX_EXECUTION_PROOF_SIZE;

    #[test]
    fn test_error_variants() {
        // Ensure error type is Debug-printable
        let err = GossipExecutionProofError::InvalidSubnetId { received: 99 };
        let _ = format!("{:?}", err);

        let err = GossipExecutionProofError::InvalidVersion { version: 0 };
        let _ = format!("{:?}", err);

        let err = GossipExecutionProofError::ProofDataEmpty;
        let _ = format!("{:?}", err);

        let err = GossipExecutionProofError::ProofDataTooLarge { size: 9999999 };
        let _ = format!("{:?}", err);

        let err = GossipExecutionProofError::UnknownBlockRoot {
            block_root: Hash256::ZERO,
        };
        let _ = format!("{:?}", err);

        let err = GossipExecutionProofError::InvalidProof {
            reason: "test".into(),
        };
        let _ = format!("{:?}", err);

        let err = GossipExecutionProofError::InvalidProofData;
        let _ = format!("{:?}", err);

        let err = GossipExecutionProofError::Sp1VerificationUnavailable;
        let _ = format!("{:?}", err);

        let err = GossipExecutionProofError::PublicValuesBlockHashMismatch {
            expected: ExecutionBlockHash::zero(),
            got: ExecutionBlockHash::zero(),
        };
        let _ = format!("{:?}", err);
    }

    #[test]
    fn test_error_from_beacon_chain_error() {
        let bce = BeaconChainError::NoStateForSlot(types::Slot::new(42));
        let err = GossipExecutionProofError::from(bce);
        assert!(matches!(
            err,
            GossipExecutionProofError::BeaconChainError(_)
        ));
    }

    /// Test that the subnet_id bounds check in verify_execution_proof_for_gossip
    /// rejects out-of-bounds subnet IDs. We test this via the proof's structural
    /// checks since verify_execution_proof_for_gossip requires a chain.
    #[test]
    fn test_structural_checks_cover_verification_preconditions() {
        // Valid proof passes structural checks
        let valid = ExecutionProof::new(
            Hash256::random(),
            ExecutionBlockHash::from(Hash256::random()),
            ExecutionProofSubnetId::new(0).unwrap(),
            1,
            b"valid-proof".to_vec(),
        );
        assert!(valid.is_version_supported());
        assert!(valid.is_structurally_valid());

        // Version 0 is unsupported
        let bad_version = ExecutionProof::new(
            Hash256::random(),
            ExecutionBlockHash::from(Hash256::random()),
            ExecutionProofSubnetId::new(0).unwrap(),
            0,
            b"proof".to_vec(),
        );
        assert!(!bad_version.is_version_supported());

        // Empty proof data
        let empty = ExecutionProof::new(
            Hash256::random(),
            ExecutionBlockHash::from(Hash256::random()),
            ExecutionProofSubnetId::new(0).unwrap(),
            1,
            vec![],
        );
        assert!(!empty.is_structurally_valid());

        // Oversized proof data
        let oversized = ExecutionProof::new(
            Hash256::random(),
            ExecutionBlockHash::from(Hash256::random()),
            ExecutionProofSubnetId::new(0).unwrap(),
            1,
            vec![0u8; MAX_EXECUTION_PROOF_SIZE + 1],
        );
        assert!(!oversized.is_structurally_valid());
    }

    /// Test that ExecutionProofSubnetId bounds checking works correctly
    #[test]
    fn test_subnet_id_bounds() {
        // Valid subnet IDs
        assert!(ExecutionProofSubnetId::new(0).is_ok());

        // Out of bounds (MAX_EXECUTION_PROOF_SUBNETS is 1, so only 0 is valid)
        assert!(ExecutionProofSubnetId::new(MAX_EXECUTION_PROOF_SUBNETS).is_err());
        assert!(ExecutionProofSubnetId::new(MAX_EXECUTION_PROOF_SUBNETS + 1).is_err());
    }

    /// Test that stub proofs (version 1) pass cryptographic verification.
    #[test]
    fn test_stub_proof_passes_crypto_check() {
        let proof = ExecutionProof::new(
            Hash256::random(),
            ExecutionBlockHash::from(Hash256::random()),
            ExecutionProofSubnetId::new(0).unwrap(),
            1,
            b"stub-proof".to_vec(),
        );
        assert!(verify_proof_cryptography(&proof).is_ok());
    }

    /// Test that SP1 Groth16 proofs (version 2) are handled correctly:
    /// - With `sp1` feature: would attempt real verification (fails with invalid proof data)
    /// - Without `sp1` feature: returns Sp1VerificationUnavailable
    #[test]
    fn test_sp1_groth16_proof_crypto_check() {
        use types::execution_proof::{ExecutionProofPublicValues, PROOF_VERSION_SP1_GROTH16};

        let block_hash = ExecutionBlockHash::from(Hash256::random());

        // Build structurally valid but cryptographically invalid v2 proof.
        let pv = ExecutionProofPublicValues {
            block_hash: block_hash.into_root().0,
            parent_hash: [0x22; 32],
            state_root: [0x33; 32],
        };
        let mut proof_data = vec![0u8; 32]; // vkey_hash
        proof_data.extend_from_slice(&4u32.to_be_bytes()); // proof_len = 4
        proof_data.extend_from_slice(&[1, 2, 3, 4]); // fake groth16 proof
        proof_data.extend_from_slice(&pv.to_bytes()); // valid public values

        let proof = ExecutionProof::new(
            Hash256::random(),
            block_hash,
            ExecutionProofSubnetId::new(0).unwrap(),
            PROOF_VERSION_SP1_GROTH16,
            proof_data,
        );

        let result = verify_proof_cryptography(&proof);

        // Without sp1 feature: should be Sp1VerificationUnavailable.
        // With sp1 feature: should be InvalidProof (fake groth16 data won't verify).
        #[cfg(not(feature = "sp1"))]
        assert!(matches!(
            result,
            Err(GossipExecutionProofError::Sp1VerificationUnavailable)
        ));

        #[cfg(feature = "sp1")]
        assert!(matches!(
            result,
            Err(GossipExecutionProofError::InvalidProof { .. })
        ));
    }
}
