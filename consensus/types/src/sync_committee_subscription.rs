use crate::Epoch;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};

/// A sync committee subscription created when a validator subscribes to sync committee subnets to perform
/// sync committee duties.
#[derive(PartialEq, Debug, Serialize, Deserialize, Clone, Encode, Decode)]
pub struct SyncCommitteeSubscription {
    /// The validators index.
    #[serde(with = "serde_utils::quoted_u64")]
    pub validator_index: u64,
    /// The sync committee indices.
    #[serde(with = "serde_utils::quoted_u64_vec")]
    pub sync_committee_indices: Vec<u64>,
    /// Epoch until which this subscription is required.
    pub until_epoch: Epoch,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_and_eq() {
        let sub = SyncCommitteeSubscription {
            validator_index: 42,
            sync_committee_indices: vec![0, 1, 2],
            until_epoch: Epoch::new(10),
        };
        assert_eq!(sub, sub.clone());
    }

    #[test]
    fn serde_round_trip() {
        let sub = SyncCommitteeSubscription {
            validator_index: 100,
            sync_committee_indices: vec![5, 10, 15],
            until_epoch: Epoch::new(256),
        };
        let json = serde_json::to_string(&sub).unwrap();
        let decoded: SyncCommitteeSubscription = serde_json::from_str(&json).unwrap();
        assert_eq!(sub, decoded);
    }

    #[test]
    fn serde_validator_index_quoted() {
        let sub = SyncCommitteeSubscription {
            validator_index: 9999,
            sync_committee_indices: vec![],
            until_epoch: Epoch::new(0),
        };
        let json = serde_json::to_string(&sub).unwrap();
        assert!(json.contains("\"9999\""));
    }

    #[test]
    fn serde_indices_quoted() {
        let sub = SyncCommitteeSubscription {
            validator_index: 0,
            sync_committee_indices: vec![42],
            until_epoch: Epoch::new(0),
        };
        let json = serde_json::to_string(&sub).unwrap();
        assert!(json.contains("\"42\""));
    }

    #[test]
    fn ssz_round_trip() {
        let sub = SyncCommitteeSubscription {
            validator_index: 7,
            sync_committee_indices: vec![1, 2, 3],
            until_epoch: Epoch::new(50),
        };
        let encoded = ssz::Encode::as_ssz_bytes(&sub);
        let decoded = <SyncCommitteeSubscription as ssz::Decode>::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(sub, decoded);
    }

    #[test]
    fn empty_indices() {
        let sub = SyncCommitteeSubscription {
            validator_index: 0,
            sync_committee_indices: vec![],
            until_epoch: Epoch::new(0),
        };
        let json = serde_json::to_string(&sub).unwrap();
        let decoded: SyncCommitteeSubscription = serde_json::from_str(&json).unwrap();
        assert_eq!(sub, decoded);
        assert!(decoded.sync_committee_indices.is_empty());
    }

    #[test]
    fn debug_format() {
        let sub = SyncCommitteeSubscription {
            validator_index: 1,
            sync_committee_indices: vec![0],
            until_epoch: Epoch::new(5),
        };
        let debug = format!("{:?}", sub);
        assert!(debug.contains("SyncCommitteeSubscription"));
    }
}
