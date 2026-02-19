//! Stub execution proof generation for ZK stateless validation.
//!
//! When `generate_execution_proofs` is enabled, this module creates execution proofs
//! after the EL validates a payload via `engine_newPayload`. In ePBS, this happens
//! when the builder reveals an `ExecutionPayloadEnvelope`.
//!
//! Currently a stub — generates a placeholder proof. Real ZK prover integration
//! (SP1, RISC Zero, etc.) is Task 20.

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{debug, warn};
use types::{
    ExecutionBlockHash, ExecutionProof, ExecutionProofSubnetId, Hash256,
    execution_proof_subnet_id::MAX_EXECUTION_PROOF_SUBNETS,
};

/// Stub proof data marker. Real provers will replace this with actual ZK proof bytes.
const STUB_PROOF_MARKER: &[u8] = b"vibehouse-stub-proof-v1";

/// Receiver end for generated proofs. Task 13 (proof broadcaster) will consume from this.
pub type ProofReceiver = mpsc::UnboundedReceiver<Arc<ExecutionProof>>;

/// Generates execution proofs for validated payloads.
///
/// In ePBS, the natural proof generation trigger is after a builder's
/// `ExecutionPayloadEnvelope` passes `engine_newPayload`. The builder
/// (or any node with `--generate-execution-proofs`) generates proofs
/// and publishes them to execution proof gossip subnets.
pub struct ExecutionProofGenerator {
    /// Channel to send generated proofs for broadcasting.
    proof_tx: mpsc::UnboundedSender<Arc<ExecutionProof>>,
}

impl ExecutionProofGenerator {
    /// Create a new proof generator and its receiving channel.
    ///
    /// The returned `ProofReceiver` should be consumed by a broadcaster service
    /// (Task 13) to publish proofs to gossip subnets.
    pub fn new() -> (Self, ProofReceiver) {
        let (proof_tx, proof_rx) = mpsc::unbounded_channel();
        (Self { proof_tx }, proof_rx)
    }

    /// Generate an execution proof for a validated payload.
    ///
    /// Called after `engine_newPayload` returns VALID/SYNCING/ACCEPTED for a
    /// Gloas `ExecutionPayloadEnvelope`. Generates one proof per subnet
    /// (currently just subnet 0).
    ///
    /// This is a synchronous stub — real ZK proof generation will be async and
    /// computationally expensive (seconds to minutes on GPU).
    pub fn generate_proof(&self, block_root: Hash256, block_hash: ExecutionBlockHash) {
        for subnet in 0..MAX_EXECUTION_PROOF_SUBNETS {
            let Ok(subnet_id) = ExecutionProofSubnetId::new(subnet) else {
                continue;
            };

            let proof = Arc::new(ExecutionProof::new(
                block_root,
                block_hash,
                subnet_id,
                1, // version
                STUB_PROOF_MARKER.to_vec(),
            ));

            debug!(
                %block_root,
                %block_hash,
                subnet = subnet,
                "Generated stub execution proof"
            );

            if self.proof_tx.send(proof).is_err() {
                warn!(
                    %block_root,
                    "Failed to send generated execution proof — receiver dropped"
                );
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_stub_proof_for_each_subnet() {
        let (generator, mut rx) = ExecutionProofGenerator::new();
        let block_root = Hash256::random();
        let block_hash = ExecutionBlockHash::from(Hash256::random());

        generator.generate_proof(block_root, block_hash);

        let mut count = 0;
        while let Ok(proof) = rx.try_recv() {
            assert_eq!(proof.block_root, block_root);
            assert_eq!(proof.block_hash, block_hash);
            assert_eq!(proof.version, 1);
            assert_eq!(proof.proof_data, STUB_PROOF_MARKER);
            assert!(proof.is_structurally_valid());
            count += 1;
        }
        assert_eq!(count, MAX_EXECUTION_PROOF_SUBNETS);
    }

    #[test]
    fn proof_receiver_dropped_does_not_panic() {
        let (generator, rx) = ExecutionProofGenerator::new();
        drop(rx);

        // Should not panic, just warn
        generator.generate_proof(
            Hash256::random(),
            ExecutionBlockHash::from(Hash256::random()),
        );
    }
}
