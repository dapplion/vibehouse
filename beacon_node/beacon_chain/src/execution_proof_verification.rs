use std::sync::Arc;

use types::{
    EthSpec, ExecutionBlockHash, ExecutionProof, ExecutionProofSubnetId, Hash256,
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
    /// The cryptographic proof verification failed.
    ///
    /// ## Peer scoring
    /// The peer has sent an invalid proof.
    InvalidProof,
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
/// Cryptographic verification is currently stubbed — real ZK verification will
/// be added in a later task.
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
/// 7. Cryptographic verification (currently stubbed)
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

    // Check 7: Cryptographic verification (stubbed).
    // Real ZK verification will be added in a later task.
    // For now, accept all proofs that pass structural checks.

    Ok(VerifiedExecutionProof {
        proof,
        block_root,
        _phantom: std::marker::PhantomData,
    })
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

        let err = GossipExecutionProofError::InvalidProof;
        let _ = format!("{:?}", err);
    }
}
