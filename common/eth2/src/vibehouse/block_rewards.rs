use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use types::{AttestationData, Hash256, Slot};

/// Details about the rewards paid to a block proposer for proposing a block.
///
/// All rewards in GWei.
///
/// Presently this only counts attestation rewards, but in future should be expanded
/// to include information on slashings and sync committee aggregates too.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct BlockReward {
    /// Sum of all reward components.
    pub total: u64,
    /// Block root of the block that these rewards are for.
    pub block_root: Hash256,
    /// Metadata about the block, particularly reward-relevant metadata.
    pub meta: BlockRewardMeta,
    /// Rewards due to attestations.
    pub attestation_rewards: AttestationRewards,
    /// Sum of rewards due to sync committee signatures.
    pub sync_committee_rewards: u64,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct BlockRewardMeta {
    pub slot: Slot,
    pub parent_slot: Slot,
    pub proposer_index: u64,
    pub graffiti: String,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct AttestationRewards {
    /// Total block reward from attestations included.
    pub total: u64,
    /// Total rewards from previous epoch attestations.
    pub prev_epoch_total: u64,
    /// Total rewards from current epoch attestations.
    pub curr_epoch_total: u64,
    /// Vec of attestation rewards for each attestation included.
    ///
    /// Each element of the vec is a map from validator index to reward.
    pub per_attestation_rewards: Vec<HashMap<u64, u64>>,
    /// The attestations themselves (optional).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attestations: Vec<AttestationData>,
}

/// Query parameters for the `/vibehouse/block_rewards` endpoint.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct BlockRewardsQuery {
    /// Lower slot limit for block rewards returned (inclusive).
    pub start_slot: Slot,
    /// Upper slot limit for block rewards returned (inclusive).
    pub end_slot: Slot,
    /// Include the full attestations themselves?
    #[serde(default)]
    pub include_attestations: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_reward_serde_roundtrip() {
        let reward = BlockReward {
            total: 1000,
            block_root: Hash256::repeat_byte(0x01),
            meta: BlockRewardMeta {
                slot: Slot::new(42),
                parent_slot: Slot::new(41),
                proposer_index: 5,
                graffiti: "test".into(),
            },
            attestation_rewards: AttestationRewards {
                total: 800,
                prev_epoch_total: 500,
                curr_epoch_total: 300,
                per_attestation_rewards: vec![],
                attestations: vec![],
            },
            sync_committee_rewards: 200,
        };
        let json = serde_json::to_string(&reward).unwrap();
        let decoded: BlockReward = serde_json::from_str(&json).unwrap();
        assert_eq!(reward, decoded);
    }

    #[test]
    fn attestation_rewards_with_per_attestation_data() {
        let mut map = HashMap::new();
        map.insert(1u64, 100u64);
        map.insert(2, 200);

        let rewards = AttestationRewards {
            total: 300,
            prev_epoch_total: 300,
            curr_epoch_total: 0,
            per_attestation_rewards: vec![map],
            attestations: vec![],
        };
        let json = serde_json::to_string(&rewards).unwrap();
        let decoded: AttestationRewards = serde_json::from_str(&json).unwrap();
        assert_eq!(rewards, decoded);
    }

    #[test]
    fn empty_attestations_skipped_in_serialization() {
        let rewards = AttestationRewards {
            total: 0,
            prev_epoch_total: 0,
            curr_epoch_total: 0,
            per_attestation_rewards: vec![],
            attestations: vec![],
        };
        let json = serde_json::to_string(&rewards).unwrap();
        assert!(!json.contains("attestations"));
    }

    #[test]
    fn block_rewards_query_serde() {
        let q = BlockRewardsQuery {
            start_slot: Slot::new(100),
            end_slot: Slot::new(200),
            include_attestations: true,
        };
        let json = serde_json::to_string(&q).unwrap();
        let decoded: BlockRewardsQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(q, decoded);
    }

    #[test]
    fn block_rewards_query_include_attestations_defaults_false() {
        let json = r#"{"start_slot":"100","end_slot":"200"}"#;
        let q: BlockRewardsQuery = serde_json::from_str(json).unwrap();
        assert!(!q.include_attestations);
    }

    #[test]
    fn block_reward_meta_clone_eq() {
        let meta = BlockRewardMeta {
            slot: Slot::new(1),
            parent_slot: Slot::new(0),
            proposer_index: 42,
            graffiti: "vibehouse".into(),
        };
        assert_eq!(meta.clone(), meta);
    }
}
