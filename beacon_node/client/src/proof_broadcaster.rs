//! Background service that publishes generated execution proofs to gossip subnets.
//!
//! Consumes proofs from the `ExecutionProofGenerator` channel and publishes each
//! as a `PubsubMessage::ExecutionProof` via the network sender.

use beacon_chain::execution_proof_generation::ProofReceiver;
use lighthouse_network::PubsubMessage;
use network::NetworkMessage;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, warn};
use types::EthSpec;

/// Runs the proof broadcaster loop until the receiver is closed.
///
/// Each proof received from the generator is wrapped in a `PubsubMessage::ExecutionProof`
/// and sent to the network for gossip publication.
pub async fn run_proof_broadcaster<E: EthSpec>(
    mut proof_rx: ProofReceiver,
    network_tx: UnboundedSender<NetworkMessage<E>>,
) {
    while let Some(proof) = proof_rx.recv().await {
        let subnet_id = proof.subnet_id;
        let block_root = proof.block_root;

        debug!(
            %block_root,
            subnet = %subnet_id,
            "Broadcasting execution proof to gossip"
        );

        let message = PubsubMessage::ExecutionProof(Box::new((subnet_id, proof)));
        if network_tx
            .send(NetworkMessage::Publish {
                messages: vec![message],
            })
            .is_err()
        {
            warn!("Proof broadcaster: network sender dropped, stopping");
            return;
        }
    }

    debug!("Proof broadcaster: generator channel closed, stopping");
}
