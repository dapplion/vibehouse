use crate::proto_array::ProposerBoost;
use crate::{
    Error, JustifiedBalances,
    proto_array::{ProtoArray, ProtoNode},
    proto_array_fork_choice::{ElasticList, ProtoArrayForkChoice, VoteTracker},
};
use ssz_derive::{Decode, Encode};
use std::collections::HashMap;
use types::{Checkpoint, Hash256};

#[derive(Encode, Decode, Clone)]
pub struct SszContainer {
    pub votes: Vec<VoteTracker>,
    pub prune_threshold: usize,
    pub justified_checkpoint: Checkpoint,
    pub finalized_checkpoint: Checkpoint,
    pub nodes: Vec<ProtoNode>,
    pub indices: Vec<(Hash256, usize)>,
    pub previous_proposer_boost: ProposerBoost,
}

impl From<&ProtoArrayForkChoice> for SszContainer {
    fn from(from: &ProtoArrayForkChoice) -> Self {
        let proto_array = &from.proto_array;

        Self {
            votes: from.votes.0.clone(),
            prune_threshold: proto_array.prune_threshold,
            justified_checkpoint: proto_array.justified_checkpoint,
            finalized_checkpoint: proto_array.finalized_checkpoint,
            nodes: proto_array.nodes.clone(),
            indices: proto_array.indices.iter().map(|(k, v)| (*k, *v)).collect(),
            previous_proposer_boost: proto_array.previous_proposer_boost,
        }
    }
}

impl TryFrom<(SszContainer, JustifiedBalances)> for ProtoArrayForkChoice {
    type Error = Error;

    fn try_from((from, balances): (SszContainer, JustifiedBalances)) -> Result<Self, Error> {
        let proto_array = ProtoArray {
            prune_threshold: from.prune_threshold,
            justified_checkpoint: from.justified_checkpoint,
            finalized_checkpoint: from.finalized_checkpoint,
            nodes: from.nodes,
            indices: from.indices.into_iter().collect::<HashMap<_, _>>(),
            previous_proposer_boost: from.previous_proposer_boost,
        };

        Ok(Self {
            proto_array,
            votes: ElasticList(from.votes),
            balances,
            gloas_head_payload_status: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExecutionStatus;
    use ssz::{Decode, Encode};
    use types::{
        AttestationShufflingId, Epoch, ExecutionBlockHash, FixedBytesExtended, MainnetEthSpec, Slot,
    };

    /// Build a ProtoArrayForkChoice with one Gloas block (non-trivial Gloas fields)
    /// and one vote with Gloas payload_present tracking.
    fn make_gloas_fork_choice() -> ProtoArrayForkChoice {
        let genesis_checkpoint = Checkpoint {
            epoch: Epoch::new(0),
            root: Hash256::from_low_u64_be(1),
        };
        let shuffling_id = AttestationShufflingId::from_components(Epoch::new(0), Hash256::zero());

        let mut fc = ProtoArrayForkChoice::new::<MainnetEthSpec>(
            Slot::new(0),
            Slot::new(0),
            Hash256::zero(),
            genesis_checkpoint,
            genesis_checkpoint,
            shuffling_id,
            shuffling_id,
            ExecutionStatus::irrelevant(),
        )
        .unwrap();

        // Insert a Gloas block with populated fields
        let block_root = Hash256::from_low_u64_be(2);
        fc.proto_array
            .on_block::<MainnetEthSpec>(
                crate::Block {
                    slot: Slot::new(1),
                    root: block_root,
                    parent_root: Some(Hash256::from_low_u64_be(1)),
                    state_root: Hash256::from_low_u64_be(3),
                    target_root: Hash256::from_low_u64_be(1),
                    current_epoch_shuffling_id: shuffling_id,
                    next_epoch_shuffling_id: shuffling_id,
                    justified_checkpoint: genesis_checkpoint,
                    finalized_checkpoint: genesis_checkpoint,
                    execution_status: ExecutionStatus::Optimistic(ExecutionBlockHash::from_root(
                        Hash256::from_low_u64_be(500),
                    )),
                    unrealized_justified_checkpoint: Some(genesis_checkpoint),
                    unrealized_finalized_checkpoint: Some(genesis_checkpoint),
                    builder_index: Some(42),
                    payload_revealed: true,
                    ptc_weight: 300,
                    ptc_blob_data_available_weight: 257,
                    payload_data_available: true,
                    bid_block_hash: Some(ExecutionBlockHash::from_root(Hash256::from_low_u64_be(
                        500,
                    ))),
                    bid_parent_block_hash: Some(ExecutionBlockHash::from_root(
                        Hash256::from_low_u64_be(499),
                    )),
                    proposer_index: 77,
                    ptc_timely: true,
                    envelope_received: true,
                },
                Slot::new(1),
            )
            .unwrap();

        // Add a vote with Gloas fields
        fc.process_attestation(0, block_root, Epoch::new(0), Slot::new(1), true)
            .unwrap();

        fc
    }

    #[test]
    fn ssz_container_round_trip_preserves_gloas_fields() {
        let original = make_gloas_fork_choice();

        // Encode via SszContainer
        let container = SszContainer::from(&original);
        let encoded = container.as_ssz_bytes();
        let decoded_container =
            SszContainer::from_ssz_bytes(&encoded).expect("SSZ container decode failed");

        // Reconstruct the fork choice
        let balances = original.balances.clone();
        let restored =
            ProtoArrayForkChoice::try_from((decoded_container, balances)).expect("restore failed");

        // Verify the Gloas block fields survived the round-trip
        let gloas_node = &restored.proto_array.nodes[1]; // index 1 = the block we inserted
        assert_eq!(gloas_node.builder_index, Some(42));
        assert!(gloas_node.payload_revealed);
        assert_eq!(gloas_node.ptc_weight, 300);
        assert_eq!(gloas_node.ptc_blob_data_available_weight, 257);
        assert!(gloas_node.payload_data_available);
        assert_eq!(
            gloas_node.bid_block_hash,
            Some(ExecutionBlockHash::from_root(Hash256::from_low_u64_be(500)))
        );
        assert_eq!(
            gloas_node.bid_parent_block_hash,
            Some(ExecutionBlockHash::from_root(Hash256::from_low_u64_be(499)))
        );
        assert_eq!(gloas_node.proposer_index, 77);
        assert!(gloas_node.ptc_timely);
        assert!(gloas_node.envelope_received);

        // Verify the vote survived the round-trip (fields are private, use equality)
        let original_vote = &original.votes.0[0];
        let restored_vote = &restored.votes.0[0];
        assert_eq!(*original_vote, *restored_vote);
    }

    #[test]
    fn vote_tracker_ssz_round_trip_byte_equality() {
        // VoteTracker fields are private, so we verify round-trip via byte equality.
        // The make_gloas_fork_choice helper sets a vote with payload_present=true,
        // which exercises the Gloas-specific fields.
        let fc = make_gloas_fork_choice();
        let vote = &fc.votes.0[0];

        let encoded = vote.as_ssz_bytes();
        let decoded = VoteTracker::from_ssz_bytes(&encoded).expect("VoteTracker decode failed");

        // Byte-level equality guarantees all fields (including Gloas slot/payload_present)
        // survived the round-trip
        assert_eq!(decoded.as_ssz_bytes(), encoded);
        assert_eq!(*vote, decoded);
    }
}
