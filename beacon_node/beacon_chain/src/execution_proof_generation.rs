//! Execution proof generation for ZK stateless validation.
//!
//! When `generate_execution_proofs` is enabled, this module creates execution proofs
//! after the EL validates a payload via `engine_newPayload`. In ePBS, this happens
//! when the builder reveals an `ExecutionPayloadEnvelope`.
//!
//! Proof generation is asynchronous — proofs are generated in background tasks and
//! sent to the broadcaster service for gossip publication when ready.
//!
//! Currently generates stub proofs (version 1). The `generate_proof` method spawns
//! proof generation on the task executor, ready for integration with real SP1 proving
//! via the `zkvm/host` binary or `sp1-sdk`.

use std::sync::Arc;

use task_executor::TaskExecutor;
use tokio::sync::mpsc;
use tracing::{debug, warn};
use types::{
    ExecutionBlockHash, ExecutionProof, ExecutionProofSubnetId, Hash256,
    execution_proof::PROOF_VERSION_STUB, execution_proof_subnet_id::MAX_EXECUTION_PROOF_SUBNETS,
};

/// Stub proof data marker. Real provers will replace this with actual ZK proof bytes.
const STUB_PROOF_MARKER: &[u8] = b"vibehouse-stub-proof-v1";

/// Receiver end for generated proofs. The proof broadcaster service consumes from this.
pub type ProofReceiver = mpsc::UnboundedReceiver<Arc<ExecutionProof>>;

/// Generates execution proofs for validated payloads.
///
/// In ePBS, the natural proof generation trigger is after a builder's
/// `ExecutionPayloadEnvelope` passes `engine_newPayload`. The builder
/// (or any node with `--generate-execution-proofs`) generates proofs
/// and publishes them to execution proof gossip subnets.
///
/// Proof generation is spawned as a background task on the `TaskExecutor`.
/// For stub proofs this completes instantly; for real ZK proofs (SP1 Groth16)
/// this may take seconds to minutes depending on the proving backend.
pub struct ExecutionProofGenerator {
    /// Channel to send generated proofs for broadcasting.
    proof_tx: mpsc::UnboundedSender<Arc<ExecutionProof>>,
    /// Task executor for spawning async proof generation work.
    task_executor: TaskExecutor,
}

impl ExecutionProofGenerator {
    /// Create a new proof generator and its receiving channel.
    ///
    /// The returned `ProofReceiver` should be consumed by the proof broadcaster
    /// service to publish proofs to gossip subnets.
    pub fn new(task_executor: TaskExecutor) -> (Self, ProofReceiver) {
        let (proof_tx, proof_rx) = mpsc::unbounded_channel();
        (
            Self {
                proof_tx,
                task_executor,
            },
            proof_rx,
        )
    }

    /// Generate execution proofs for a validated payload.
    ///
    /// Called after `engine_newPayload` returns VALID/SYNCING/ACCEPTED for a
    /// Gloas `ExecutionPayloadEnvelope`. Spawns proof generation as a background
    /// task — the caller does not wait for the proof to be generated.
    ///
    /// Currently generates stub proofs (version 1, instant). When integrated
    /// with a real SP1 prover, this will spawn a long-running background task
    /// that calls the prover and sends the result when ready.
    pub fn generate_proof(&self, block_root: Hash256, block_hash: ExecutionBlockHash) {
        let proof_tx = self.proof_tx.clone();

        self.task_executor.spawn(
            async move {
                generate_stub_proofs(proof_tx, block_root, block_hash);
            },
            "execution_proof_generation",
        );
    }
}

/// Generate stub proofs (version 1) for all subnets and send them to the channel.
///
/// This is the default proof generation backend. It creates placeholder proofs
/// that pass structural validation but contain no cryptographic content.
/// Stateless nodes configured with `--stateless-min-proofs-required 1` accept
/// these proofs for testing and development.
///
/// To replace with real SP1 Groth16 proof generation, this function should:
/// 1. Call the `vibehouse-sp1-host` binary as a subprocess, or
/// 2. Use `sp1-sdk` directly (requires adding sp1-sdk as a dependency), or
/// 3. Call an external proving service via HTTP
fn generate_stub_proofs(
    proof_tx: mpsc::UnboundedSender<Arc<ExecutionProof>>,
    block_root: Hash256,
    block_hash: ExecutionBlockHash,
) {
    for subnet in 0..MAX_EXECUTION_PROOF_SUBNETS {
        let Ok(subnet_id) = ExecutionProofSubnetId::new(subnet) else {
            continue;
        };

        let proof = Arc::new(ExecutionProof::new(
            block_root,
            block_hash,
            subnet_id,
            PROOF_VERSION_STUB,
            STUB_PROOF_MARKER.to_vec(),
        ));

        debug!(
            %block_root,
            %block_hash,
            subnet = subnet,
            "Generated stub execution proof"
        );

        if proof_tx.send(proof).is_err() {
            warn!(
                %block_root,
                "Failed to send generated execution proof — receiver dropped"
            );
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use task_executor::test_utils::TestRuntime;

    #[test]
    fn generates_stub_proof_for_each_subnet() {
        let block_root = Hash256::random();
        let block_hash = ExecutionBlockHash::from(Hash256::random());

        // Test the stub generation function directly (no task executor needed).
        let (proof_tx, mut proof_rx) = mpsc::unbounded_channel();
        generate_stub_proofs(proof_tx, block_root, block_hash);

        let mut count = 0;
        while let Ok(proof) = proof_rx.try_recv() {
            assert_eq!(proof.block_root, block_root);
            assert_eq!(proof.block_hash, block_hash);
            assert_eq!(proof.version, PROOF_VERSION_STUB);
            assert_eq!(proof.proof_data, STUB_PROOF_MARKER);
            assert!(proof.is_structurally_valid());
            count += 1;
        }
        assert_eq!(count, MAX_EXECUTION_PROOF_SUBNETS);
    }

    #[test]
    fn proof_receiver_dropped_does_not_panic() {
        let (proof_tx, proof_rx) = mpsc::unbounded_channel();
        drop(proof_rx);

        // Should not panic, just warn.
        generate_stub_proofs(
            proof_tx,
            Hash256::random(),
            ExecutionBlockHash::from(Hash256::random()),
        );
    }

    #[tokio::test]
    async fn async_proof_generation() {
        let test_runtime = TestRuntime::default();
        let (generator, mut rx) = ExecutionProofGenerator::new(test_runtime.task_executor.clone());
        let block_root = Hash256::random();
        let block_hash = ExecutionBlockHash::from(Hash256::random());

        generator.generate_proof(block_root, block_hash);

        // Allow the spawned task to complete.
        tokio::task::yield_now().await;

        let mut count = 0;
        while let Ok(proof) = rx.try_recv() {
            assert_eq!(proof.block_root, block_root);
            assert_eq!(proof.block_hash, block_hash);
            assert_eq!(proof.version, PROOF_VERSION_STUB);
            assert!(proof.is_structurally_valid());
            count += 1;
        }
        assert_eq!(count, MAX_EXECUTION_PROOF_SUBNETS);
    }
}
