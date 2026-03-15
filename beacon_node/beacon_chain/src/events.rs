pub use eth2::types::{
    EventKind, SseBlock, SseExecutionBid, SseExecutionPayload, SseExecutionProof,
    SseFinalizedCheckpoint, SseHead, SsePayloadAttestation,
};
use tokio::sync::broadcast;
use tokio::sync::broadcast::{Receiver, Sender, error::SendError};
use tracing::trace;
use types::EthSpec;

const DEFAULT_CHANNEL_CAPACITY: usize = 16;

pub struct ServerSentEventHandler<E: EthSpec> {
    attestation_tx: Sender<EventKind<E>>,
    single_attestation_tx: Sender<EventKind<E>>,
    block_tx: Sender<EventKind<E>>,
    blob_sidecar_tx: Sender<EventKind<E>>,
    data_column_sidecar_tx: Sender<EventKind<E>>,
    finalized_tx: Sender<EventKind<E>>,
    head_tx: Sender<EventKind<E>>,
    exit_tx: Sender<EventKind<E>>,
    chain_reorg_tx: Sender<EventKind<E>>,
    contribution_tx: Sender<EventKind<E>>,
    payload_attributes_tx: Sender<EventKind<E>>,
    late_head: Sender<EventKind<E>>,
    light_client_finality_update_tx: Sender<EventKind<E>>,
    light_client_optimistic_update_tx: Sender<EventKind<E>>,
    block_reward_tx: Sender<EventKind<E>>,
    proposer_slashing_tx: Sender<EventKind<E>>,
    attester_slashing_tx: Sender<EventKind<E>>,
    bls_to_execution_change_tx: Sender<EventKind<E>>,
    block_gossip_tx: Sender<EventKind<E>>,
    execution_bid_tx: Sender<EventKind<E>>,
    execution_payload_tx: Sender<EventKind<E>>,
    payload_attestation_tx: Sender<EventKind<E>>,
    execution_proof_received_tx: Sender<EventKind<E>>,
}

impl<E: EthSpec> ServerSentEventHandler<E> {
    pub fn new(capacity_multiplier: usize) -> Self {
        Self::new_with_capacity(capacity_multiplier.saturating_mul(DEFAULT_CHANNEL_CAPACITY))
    }

    pub fn new_with_capacity(capacity: usize) -> Self {
        let (attestation_tx, _) = broadcast::channel(capacity);
        let (single_attestation_tx, _) = broadcast::channel(capacity);
        let (block_tx, _) = broadcast::channel(capacity);
        let (blob_sidecar_tx, _) = broadcast::channel(capacity);
        let (data_column_sidecar_tx, _) = broadcast::channel(capacity);
        let (finalized_tx, _) = broadcast::channel(capacity);
        let (head_tx, _) = broadcast::channel(capacity);
        let (exit_tx, _) = broadcast::channel(capacity);
        let (chain_reorg_tx, _) = broadcast::channel(capacity);
        let (contribution_tx, _) = broadcast::channel(capacity);
        let (payload_attributes_tx, _) = broadcast::channel(capacity);
        let (late_head, _) = broadcast::channel(capacity);
        let (light_client_finality_update_tx, _) = broadcast::channel(capacity);
        let (light_client_optimistic_update_tx, _) = broadcast::channel(capacity);
        let (block_reward_tx, _) = broadcast::channel(capacity);
        let (proposer_slashing_tx, _) = broadcast::channel(capacity);
        let (attester_slashing_tx, _) = broadcast::channel(capacity);
        let (bls_to_execution_change_tx, _) = broadcast::channel(capacity);
        let (block_gossip_tx, _) = broadcast::channel(capacity);
        let (execution_bid_tx, _) = broadcast::channel(capacity);
        let (execution_payload_tx, _) = broadcast::channel(capacity);
        let (payload_attestation_tx, _) = broadcast::channel(capacity);
        let (execution_proof_received_tx, _) = broadcast::channel(capacity);

        Self {
            attestation_tx,
            single_attestation_tx,
            block_tx,
            blob_sidecar_tx,
            data_column_sidecar_tx,
            finalized_tx,
            head_tx,
            exit_tx,
            chain_reorg_tx,
            contribution_tx,
            payload_attributes_tx,
            late_head,
            light_client_finality_update_tx,
            light_client_optimistic_update_tx,
            block_reward_tx,
            proposer_slashing_tx,
            attester_slashing_tx,
            bls_to_execution_change_tx,
            block_gossip_tx,
            execution_bid_tx,
            execution_payload_tx,
            payload_attestation_tx,
            execution_proof_received_tx,
        }
    }

    pub fn register(&self, kind: EventKind<E>) {
        let log_count = |name, count| {
            trace!(
                kind = name,
                receiver_count = count,
                "Registering server-sent event"
            );
        };
        let result = match &kind {
            EventKind::Attestation(_) => self
                .attestation_tx
                .send(kind)
                .map(|count| log_count("attestation", count)),
            EventKind::SingleAttestation(_) => self
                .single_attestation_tx
                .send(kind)
                .map(|count| log_count("single_attestation", count)),
            EventKind::Block(_) => self
                .block_tx
                .send(kind)
                .map(|count| log_count("block", count)),
            EventKind::BlobSidecar(_) => self
                .blob_sidecar_tx
                .send(kind)
                .map(|count| log_count("blob sidecar", count)),
            EventKind::DataColumnSidecar(_) => self
                .data_column_sidecar_tx
                .send(kind)
                .map(|count| log_count("data_column_sidecar", count)),
            EventKind::FinalizedCheckpoint(_) => self
                .finalized_tx
                .send(kind)
                .map(|count| log_count("finalized checkpoint", count)),
            EventKind::Head(_) => self
                .head_tx
                .send(kind)
                .map(|count| log_count("head", count)),
            EventKind::VoluntaryExit(_) => self
                .exit_tx
                .send(kind)
                .map(|count| log_count("exit", count)),
            EventKind::ChainReorg(_) => self
                .chain_reorg_tx
                .send(kind)
                .map(|count| log_count("chain reorg", count)),
            EventKind::ContributionAndProof(_) => self
                .contribution_tx
                .send(kind)
                .map(|count| log_count("contribution and proof", count)),
            EventKind::PayloadAttributes(_) => self
                .payload_attributes_tx
                .send(kind)
                .map(|count| log_count("payload attributes", count)),
            EventKind::LateHead(_) => self
                .late_head
                .send(kind)
                .map(|count| log_count("late head", count)),
            EventKind::LightClientFinalityUpdate(_) => self
                .light_client_finality_update_tx
                .send(kind)
                .map(|count| log_count("light client finality update", count)),
            EventKind::LightClientOptimisticUpdate(_) => self
                .light_client_optimistic_update_tx
                .send(kind)
                .map(|count| log_count("light client optimistic update", count)),
            EventKind::BlockReward(_) => self
                .block_reward_tx
                .send(kind)
                .map(|count| log_count("block reward", count)),
            EventKind::ProposerSlashing(_) => self
                .proposer_slashing_tx
                .send(kind)
                .map(|count| log_count("proposer slashing", count)),
            EventKind::AttesterSlashing(_) => self
                .attester_slashing_tx
                .send(kind)
                .map(|count| log_count("attester slashing", count)),
            EventKind::BlsToExecutionChange(_) => self
                .bls_to_execution_change_tx
                .send(kind)
                .map(|count| log_count("bls to execution change", count)),
            EventKind::BlockGossip(_) => self
                .block_gossip_tx
                .send(kind)
                .map(|count| log_count("block gossip", count)),
            EventKind::ExecutionBid(_) => self
                .execution_bid_tx
                .send(kind)
                .map(|count| log_count("execution bid", count)),
            EventKind::ExecutionPayload(_) => self
                .execution_payload_tx
                .send(kind)
                .map(|count| log_count("execution payload", count)),
            EventKind::PayloadAttestation(_) => self
                .payload_attestation_tx
                .send(kind)
                .map(|count| log_count("payload attestation", count)),
            EventKind::ExecutionProofReceived(_) => self
                .execution_proof_received_tx
                .send(kind)
                .map(|count| log_count("execution proof received", count)),
        };
        if let Err(SendError(event)) = result {
            trace!(?event, "No receivers registered to listen for event");
        }
    }

    pub fn subscribe_attestation(&self) -> Receiver<EventKind<E>> {
        self.attestation_tx.subscribe()
    }

    pub fn subscribe_single_attestation(&self) -> Receiver<EventKind<E>> {
        self.single_attestation_tx.subscribe()
    }

    pub fn subscribe_block(&self) -> Receiver<EventKind<E>> {
        self.block_tx.subscribe()
    }

    pub fn subscribe_blob_sidecar(&self) -> Receiver<EventKind<E>> {
        self.blob_sidecar_tx.subscribe()
    }

    pub fn subscribe_data_column_sidecar(&self) -> Receiver<EventKind<E>> {
        self.data_column_sidecar_tx.subscribe()
    }

    pub fn subscribe_finalized(&self) -> Receiver<EventKind<E>> {
        self.finalized_tx.subscribe()
    }

    pub fn subscribe_head(&self) -> Receiver<EventKind<E>> {
        self.head_tx.subscribe()
    }

    pub fn subscribe_exit(&self) -> Receiver<EventKind<E>> {
        self.exit_tx.subscribe()
    }

    pub fn subscribe_reorgs(&self) -> Receiver<EventKind<E>> {
        self.chain_reorg_tx.subscribe()
    }

    pub fn subscribe_contributions(&self) -> Receiver<EventKind<E>> {
        self.contribution_tx.subscribe()
    }

    pub fn subscribe_payload_attributes(&self) -> Receiver<EventKind<E>> {
        self.payload_attributes_tx.subscribe()
    }

    pub fn subscribe_late_head(&self) -> Receiver<EventKind<E>> {
        self.late_head.subscribe()
    }

    pub fn subscribe_light_client_finality_update(&self) -> Receiver<EventKind<E>> {
        self.light_client_finality_update_tx.subscribe()
    }

    pub fn subscribe_light_client_optimistic_update(&self) -> Receiver<EventKind<E>> {
        self.light_client_optimistic_update_tx.subscribe()
    }

    pub fn subscribe_block_reward(&self) -> Receiver<EventKind<E>> {
        self.block_reward_tx.subscribe()
    }

    pub fn subscribe_attester_slashing(&self) -> Receiver<EventKind<E>> {
        self.attester_slashing_tx.subscribe()
    }

    pub fn subscribe_proposer_slashing(&self) -> Receiver<EventKind<E>> {
        self.proposer_slashing_tx.subscribe()
    }

    pub fn subscribe_bls_to_execution_change(&self) -> Receiver<EventKind<E>> {
        self.bls_to_execution_change_tx.subscribe()
    }

    pub fn subscribe_block_gossip(&self) -> Receiver<EventKind<E>> {
        self.block_gossip_tx.subscribe()
    }

    pub fn subscribe_execution_bid(&self) -> Receiver<EventKind<E>> {
        self.execution_bid_tx.subscribe()
    }

    pub fn subscribe_execution_payload(&self) -> Receiver<EventKind<E>> {
        self.execution_payload_tx.subscribe()
    }

    pub fn subscribe_payload_attestation(&self) -> Receiver<EventKind<E>> {
        self.payload_attestation_tx.subscribe()
    }

    pub fn has_attestation_subscribers(&self) -> bool {
        self.attestation_tx.receiver_count() > 0
    }

    pub fn has_single_attestation_subscribers(&self) -> bool {
        self.single_attestation_tx.receiver_count() > 0
    }

    pub fn has_block_subscribers(&self) -> bool {
        self.block_tx.receiver_count() > 0
    }

    pub fn has_blob_sidecar_subscribers(&self) -> bool {
        self.blob_sidecar_tx.receiver_count() > 0
    }

    pub fn has_data_column_sidecar_subscribers(&self) -> bool {
        self.data_column_sidecar_tx.receiver_count() > 0
    }

    pub fn has_finalized_subscribers(&self) -> bool {
        self.finalized_tx.receiver_count() > 0
    }

    pub fn has_head_subscribers(&self) -> bool {
        self.head_tx.receiver_count() > 0
    }

    pub fn has_exit_subscribers(&self) -> bool {
        self.exit_tx.receiver_count() > 0
    }

    pub fn has_reorg_subscribers(&self) -> bool {
        self.chain_reorg_tx.receiver_count() > 0
    }

    pub fn has_contribution_subscribers(&self) -> bool {
        self.contribution_tx.receiver_count() > 0
    }

    pub fn has_payload_attributes_subscribers(&self) -> bool {
        self.payload_attributes_tx.receiver_count() > 0
    }

    pub fn has_late_head_subscribers(&self) -> bool {
        self.late_head.receiver_count() > 0
    }

    pub fn has_block_reward_subscribers(&self) -> bool {
        self.block_reward_tx.receiver_count() > 0
    }

    pub fn has_proposer_slashing_subscribers(&self) -> bool {
        self.proposer_slashing_tx.receiver_count() > 0
    }

    pub fn has_attester_slashing_subscribers(&self) -> bool {
        self.attester_slashing_tx.receiver_count() > 0
    }

    pub fn has_bls_to_execution_change_subscribers(&self) -> bool {
        self.bls_to_execution_change_tx.receiver_count() > 0
    }

    pub fn has_block_gossip_subscribers(&self) -> bool {
        self.block_gossip_tx.receiver_count() > 0
    }

    pub fn has_execution_bid_subscribers(&self) -> bool {
        self.execution_bid_tx.receiver_count() > 0
    }

    pub fn has_execution_payload_subscribers(&self) -> bool {
        self.execution_payload_tx.receiver_count() > 0
    }

    pub fn has_payload_attestation_subscribers(&self) -> bool {
        self.payload_attestation_tx.receiver_count() > 0
    }

    pub fn subscribe_execution_proof_received(&self) -> Receiver<EventKind<E>> {
        self.execution_proof_received_tx.subscribe()
    }

    pub fn has_execution_proof_received_subscribers(&self) -> bool {
        self.execution_proof_received_tx.receiver_count() > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eth2::types::{SseBlock, SseFinalizedCheckpoint, SseHead};
    use types::{Epoch, FixedBytesExtended, Hash256, MinimalEthSpec, Slot};

    type E = MinimalEthSpec;

    #[test]
    fn no_subscribers_initially() {
        let handler = ServerSentEventHandler::<E>::new(1);
        assert!(!handler.has_block_subscribers());
        assert!(!handler.has_head_subscribers());
        assert!(!handler.has_finalized_subscribers());
        assert!(!handler.has_attestation_subscribers());
        assert!(!handler.has_single_attestation_subscribers());
        assert!(!handler.has_exit_subscribers());
        assert!(!handler.has_reorg_subscribers());
        assert!(!handler.has_contribution_subscribers());
        assert!(!handler.has_payload_attributes_subscribers());
        assert!(!handler.has_late_head_subscribers());
        assert!(!handler.has_block_reward_subscribers());
        assert!(!handler.has_proposer_slashing_subscribers());
        assert!(!handler.has_attester_slashing_subscribers());
        assert!(!handler.has_bls_to_execution_change_subscribers());
        assert!(!handler.has_block_gossip_subscribers());
        assert!(!handler.has_blob_sidecar_subscribers());
        assert!(!handler.has_data_column_sidecar_subscribers());
        assert!(!handler.has_execution_bid_subscribers());
        assert!(!handler.has_execution_payload_subscribers());
        assert!(!handler.has_payload_attestation_subscribers());
        assert!(!handler.has_execution_proof_received_subscribers());
    }

    #[test]
    fn subscribe_block_shows_subscriber() {
        let handler = ServerSentEventHandler::<E>::new(1);
        let _rx = handler.subscribe_block();
        assert!(handler.has_block_subscribers());
    }

    #[test]
    fn subscribe_head_shows_subscriber() {
        let handler = ServerSentEventHandler::<E>::new(1);
        let _rx = handler.subscribe_head();
        assert!(handler.has_head_subscribers());
    }

    #[test]
    fn subscribe_finalized_shows_subscriber() {
        let handler = ServerSentEventHandler::<E>::new(1);
        let _rx = handler.subscribe_finalized();
        assert!(handler.has_finalized_subscribers());
    }

    #[test]
    fn subscribe_execution_bid_shows_subscriber() {
        let handler = ServerSentEventHandler::<E>::new(1);
        let _rx = handler.subscribe_execution_bid();
        assert!(handler.has_execution_bid_subscribers());
    }

    #[test]
    fn subscribe_execution_payload_shows_subscriber() {
        let handler = ServerSentEventHandler::<E>::new(1);
        let _rx = handler.subscribe_execution_payload();
        assert!(handler.has_execution_payload_subscribers());
    }

    #[test]
    fn subscribe_payload_attestation_shows_subscriber() {
        let handler = ServerSentEventHandler::<E>::new(1);
        let _rx = handler.subscribe_payload_attestation();
        assert!(handler.has_payload_attestation_subscribers());
    }

    #[test]
    fn subscribe_execution_proof_received_shows_subscriber() {
        let handler = ServerSentEventHandler::<E>::new(1);
        let _rx = handler.subscribe_execution_proof_received();
        assert!(handler.has_execution_proof_received_subscribers());
    }

    #[test]
    fn drop_receiver_removes_subscriber() {
        let handler = ServerSentEventHandler::<E>::new(1);
        let rx = handler.subscribe_block();
        assert!(handler.has_block_subscribers());
        drop(rx);
        assert!(!handler.has_block_subscribers());
    }

    #[test]
    fn register_block_event_received_by_subscriber() {
        let handler = ServerSentEventHandler::<E>::new(1);
        let mut rx = handler.subscribe_block();

        let event = EventKind::Block(SseBlock {
            slot: Slot::new(42),
            block: Hash256::zero(),
            execution_optimistic: false,
        });
        handler.register(event);

        let received = rx.try_recv().expect("should receive block event");
        match received {
            EventKind::Block(sse_block) => assert_eq!(sse_block.slot, Slot::new(42)),
            _ => panic!("expected Block event"),
        }
    }

    #[test]
    fn register_head_event_received_by_subscriber() {
        let handler = ServerSentEventHandler::<E>::new(1);
        let mut rx = handler.subscribe_head();

        let event = EventKind::Head(SseHead {
            slot: Slot::new(10),
            block: Hash256::zero(),
            state: Hash256::zero(),
            epoch_transition: false,
            current_duty_dependent_root: Hash256::zero(),
            previous_duty_dependent_root: Hash256::zero(),
            execution_optimistic: false,
        });
        handler.register(event);

        let received = rx.try_recv().expect("should receive head event");
        match received {
            EventKind::Head(sse_head) => assert_eq!(sse_head.slot, Slot::new(10)),
            _ => panic!("expected Head event"),
        }
    }

    #[test]
    fn register_finalized_event_received_by_subscriber() {
        let handler = ServerSentEventHandler::<E>::new(1);
        let mut rx = handler.subscribe_finalized();

        let event = EventKind::FinalizedCheckpoint(SseFinalizedCheckpoint {
            block: Hash256::zero(),
            state: Hash256::zero(),
            epoch: Epoch::new(3),
            execution_optimistic: false,
        });
        handler.register(event);

        let received = rx.try_recv().expect("should receive finalized event");
        match received {
            EventKind::FinalizedCheckpoint(cp) => assert_eq!(cp.epoch, Epoch::new(3)),
            _ => panic!("expected FinalizedCheckpoint event"),
        }
    }

    #[test]
    fn register_without_subscribers_does_not_panic() {
        let handler = ServerSentEventHandler::<E>::new(1);
        // No subscribers — register should silently drop.
        handler.register(EventKind::Block(SseBlock {
            slot: Slot::new(1),
            block: Hash256::zero(),
            execution_optimistic: false,
        }));
    }

    #[test]
    fn multiple_subscribers_all_receive() {
        let handler = ServerSentEventHandler::<E>::new(1);
        let mut rx1 = handler.subscribe_block();
        let mut rx2 = handler.subscribe_block();

        handler.register(EventKind::Block(SseBlock {
            slot: Slot::new(7),
            block: Hash256::zero(),
            execution_optimistic: false,
        }));

        let r1 = rx1.try_recv().expect("rx1 should receive");
        let r2 = rx2.try_recv().expect("rx2 should receive");

        match (r1, r2) {
            (EventKind::Block(b1), EventKind::Block(b2)) => {
                assert_eq!(b1.slot, Slot::new(7));
                assert_eq!(b2.slot, Slot::new(7));
            }
            _ => panic!("expected Block events"),
        }
    }

    #[test]
    fn capacity_multiplier_scales_channel_size() {
        // With multiplier=2, capacity = 2 * 16 = 32.
        let handler = ServerSentEventHandler::<E>::new(2);
        let mut rx = handler.subscribe_block();

        // Send 32 events without receiving — all should fit.
        for i in 0..32 {
            handler.register(EventKind::Block(SseBlock {
                slot: Slot::new(i),
                block: Hash256::zero(),
                execution_optimistic: false,
            }));
        }

        // All 32 should be receivable.
        for i in 0..32 {
            let received = rx.try_recv().expect("should receive event");
            match received {
                EventKind::Block(b) => assert_eq!(b.slot, Slot::new(i)),
                _ => panic!("expected Block event"),
            }
        }
    }

    #[test]
    fn event_routing_independence() {
        let handler = ServerSentEventHandler::<E>::new(1);
        let mut block_rx = handler.subscribe_block();
        let mut head_rx = handler.subscribe_head();

        // Register a block event — head subscriber should not receive it.
        handler.register(EventKind::Block(SseBlock {
            slot: Slot::new(1),
            block: Hash256::zero(),
            execution_optimistic: false,
        }));

        assert!(block_rx.try_recv().is_ok());
        assert!(
            head_rx.try_recv().is_err(),
            "head should not receive block event"
        );
    }
}
