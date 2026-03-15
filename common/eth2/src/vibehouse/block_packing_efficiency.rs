use serde::{Deserialize, Serialize};
use types::{Epoch, Hash256, Slot};

type CommitteePosition = usize;
type Committee = u64;
type ValidatorIndex = u64;

#[derive(Debug, Default, PartialEq, Eq, Hash, Clone, Serialize, Deserialize)]
pub struct UniqueAttestation {
    pub slot: Slot,
    pub committee_index: Committee,
    pub committee_position: CommitteePosition,
}
#[derive(Debug, Default, PartialEq, Clone, Serialize, Deserialize)]
pub struct ProposerInfo {
    pub validator_index: ValidatorIndex,
    pub graffiti: String,
}

#[derive(Debug, Default, PartialEq, Clone, Serialize, Deserialize)]
pub struct BlockPackingEfficiency {
    pub slot: Slot,
    pub block_hash: Hash256,
    pub proposer_info: ProposerInfo,
    pub available_attestations: usize,
    pub included_attestations: usize,
    pub prior_skip_slots: u64,
}

#[derive(Debug, Default, PartialEq, Clone, Serialize, Deserialize)]
pub struct BlockPackingEfficiencyQuery {
    pub start_epoch: Epoch,
    pub end_epoch: Epoch,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_attestation_default() {
        let att = UniqueAttestation::default();
        assert_eq!(att.slot, Slot::new(0));
        assert_eq!(att.committee_index, 0);
        assert_eq!(att.committee_position, 0);
    }

    #[test]
    fn unique_attestation_serde_roundtrip() {
        let att = UniqueAttestation {
            slot: Slot::new(42),
            committee_index: 3,
            committee_position: 7,
        };
        let json = serde_json::to_string(&att).unwrap();
        let decoded: UniqueAttestation = serde_json::from_str(&json).unwrap();
        assert_eq!(att, decoded);
    }

    #[test]
    fn unique_attestation_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(UniqueAttestation {
            slot: Slot::new(1),
            committee_index: 0,
            committee_position: 0,
        });
        set.insert(UniqueAttestation {
            slot: Slot::new(1),
            committee_index: 0,
            committee_position: 0,
        });
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn proposer_info_serde_roundtrip() {
        let pi = ProposerInfo {
            validator_index: 99,
            graffiti: "hello".into(),
        };
        let json = serde_json::to_string(&pi).unwrap();
        let decoded: ProposerInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(pi, decoded);
    }

    #[test]
    fn block_packing_efficiency_serde_roundtrip() {
        let bpe = BlockPackingEfficiency {
            slot: Slot::new(100),
            block_hash: Hash256::repeat_byte(0xab),
            proposer_info: ProposerInfo {
                validator_index: 5,
                graffiti: "test".into(),
            },
            available_attestations: 128,
            included_attestations: 64,
            prior_skip_slots: 2,
        };
        let json = serde_json::to_string(&bpe).unwrap();
        let decoded: BlockPackingEfficiency = serde_json::from_str(&json).unwrap();
        assert_eq!(bpe, decoded);
    }

    #[test]
    fn query_serde_roundtrip() {
        let q = BlockPackingEfficiencyQuery {
            start_epoch: Epoch::new(10),
            end_epoch: Epoch::new(20),
        };
        let json = serde_json::to_string(&q).unwrap();
        let decoded: BlockPackingEfficiencyQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(q, decoded);
    }

    #[test]
    fn clone_preserves_values() {
        let bpe = BlockPackingEfficiency {
            slot: Slot::new(50),
            block_hash: Hash256::repeat_byte(0x01),
            proposer_info: ProposerInfo {
                validator_index: 1,
                graffiti: "g".into(),
            },
            available_attestations: 10,
            included_attestations: 5,
            prior_skip_slots: 1,
        };
        assert_eq!(bpe.clone(), bpe);
    }
}
