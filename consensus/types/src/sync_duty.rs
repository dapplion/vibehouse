use crate::{EthSpec, SyncCommittee, SyncSubnetId};
use bls::PublicKeyBytes;
use safe_arith::ArithError;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncDuty {
    pub pubkey: PublicKeyBytes,
    #[serde(with = "serde_utils::quoted_u64")]
    pub validator_index: u64,
    #[serde(with = "serde_utils::quoted_u64_vec")]
    pub validator_sync_committee_indices: Vec<u64>,
}

impl SyncDuty {
    /// Create a new `SyncDuty` from the list of validator indices in a sync committee.
    pub fn from_sync_committee_indices(
        validator_index: u64,
        pubkey: PublicKeyBytes,
        sync_committee_indices: &[usize],
    ) -> Option<Self> {
        // Positions of the `validator_index` within the committee.
        let validator_sync_committee_indices = sync_committee_indices
            .iter()
            .enumerate()
            .filter_map(|(i, &v)| {
                if validator_index == v as u64 {
                    Some(i as u64)
                } else {
                    None
                }
            })
            .collect();
        Self::new(validator_index, pubkey, validator_sync_committee_indices)
    }

    /// Create a new `SyncDuty` from a `SyncCommittee`, which contains the pubkeys but not the
    /// indices.
    pub fn from_sync_committee<E: EthSpec>(
        validator_index: u64,
        pubkey: PublicKeyBytes,
        sync_committee: &SyncCommittee<E>,
    ) -> Option<Self> {
        let validator_sync_committee_indices = sync_committee
            .pubkeys
            .iter()
            .enumerate()
            .filter_map(|(i, committee_pubkey)| {
                if &pubkey == committee_pubkey {
                    Some(i as u64)
                } else {
                    None
                }
            })
            .collect();
        Self::new(validator_index, pubkey, validator_sync_committee_indices)
    }

    /// Create a duty if the `validator_sync_committee_indices` is non-empty.
    fn new(
        validator_index: u64,
        pubkey: PublicKeyBytes,
        validator_sync_committee_indices: Vec<u64>,
    ) -> Option<Self> {
        if !validator_sync_committee_indices.is_empty() {
            Some(SyncDuty {
                pubkey,
                validator_index,
                validator_sync_committee_indices,
            })
        } else {
            None
        }
    }

    /// Get the set of subnet IDs for this duty.
    pub fn subnet_ids<E: EthSpec>(&self) -> Result<HashSet<SyncSubnetId>, ArithError> {
        SyncSubnetId::compute_subnets_for_sync_committee::<E>(
            &self.validator_sync_committee_indices,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FixedVector, MinimalEthSpec};

    type E = MinimalEthSpec;

    fn pk(byte: u8) -> PublicKeyBytes {
        let mut bytes = [0u8; 48];
        bytes[0] = byte;
        PublicKeyBytes::deserialize(&bytes).unwrap()
    }

    #[test]
    fn from_sync_committee_indices_not_in_committee() {
        // Validator 99 is not in the committee indices
        let indices: Vec<usize> = vec![0, 1, 2, 3];
        let result = SyncDuty::from_sync_committee_indices(99, pk(1), &indices);
        assert!(result.is_none());
    }

    #[test]
    fn from_sync_committee_indices_present_once() {
        let indices: Vec<usize> = vec![5, 10, 15, 20];
        let duty = SyncDuty::from_sync_committee_indices(10, pk(2), &indices).unwrap();
        assert_eq!(duty.validator_index, 10);
        assert_eq!(duty.validator_sync_committee_indices, vec![1]); // position 1
    }

    #[test]
    fn from_sync_committee_indices_present_multiple() {
        // Validator 7 appears at positions 0, 2, 4
        let indices: Vec<usize> = vec![7, 3, 7, 5, 7];
        let duty = SyncDuty::from_sync_committee_indices(7, pk(3), &indices).unwrap();
        assert_eq!(duty.validator_sync_committee_indices, vec![0, 2, 4]);
    }

    #[test]
    fn from_sync_committee_indices_empty() {
        let indices: Vec<usize> = vec![];
        let result = SyncDuty::from_sync_committee_indices(1, pk(4), &indices);
        assert!(result.is_none());
    }

    #[test]
    fn from_sync_committee_pubkey_not_found() {
        let target_pk = pk(99);
        let other_pk = pk(1);
        let pubkeys_vec = vec![other_pk; 32]; // MinimalEthSpec SyncCommitteeSize = 32
        // None match target_pk
        let pubkeys = FixedVector::new(pubkeys_vec).unwrap();
        let committee = SyncCommittee::<E> {
            pubkeys,
            aggregate_pubkey: PublicKeyBytes::empty(),
        };
        let result = SyncDuty::from_sync_committee(42, target_pk, &committee);
        assert!(result.is_none());
    }

    #[test]
    fn from_sync_committee_pubkey_found_once() {
        let target_pk = pk(42);
        let other_pk = pk(1);
        let mut pubkeys_vec = vec![other_pk; 32];
        pubkeys_vec[5] = target_pk;
        let pubkeys = FixedVector::new(pubkeys_vec).unwrap();
        let committee = SyncCommittee::<E> {
            pubkeys,
            aggregate_pubkey: PublicKeyBytes::empty(),
        };
        let duty = SyncDuty::from_sync_committee(10, target_pk, &committee).unwrap();
        assert_eq!(duty.validator_index, 10);
        assert_eq!(duty.validator_sync_committee_indices, vec![5]);
    }

    #[test]
    fn from_sync_committee_pubkey_found_multiple() {
        let target_pk = pk(42);
        let other_pk = pk(1);
        let mut pubkeys_vec = vec![other_pk; 32];
        pubkeys_vec[0] = target_pk;
        pubkeys_vec[10] = target_pk;
        pubkeys_vec[31] = target_pk;
        let pubkeys = FixedVector::new(pubkeys_vec).unwrap();
        let committee = SyncCommittee::<E> {
            pubkeys,
            aggregate_pubkey: PublicKeyBytes::empty(),
        };
        let duty = SyncDuty::from_sync_committee(7, target_pk, &committee).unwrap();
        assert_eq!(duty.validator_sync_committee_indices, vec![0, 10, 31]);
    }

    #[test]
    fn subnet_ids_single_index() {
        let duty = SyncDuty {
            pubkey: pk(1),
            validator_index: 0,
            validator_sync_committee_indices: vec![0], // subcommittee 0
        };
        let subnets = duty.subnet_ids::<E>().unwrap();
        assert_eq!(subnets.len(), 1);
        assert!(subnets.contains(&SyncSubnetId::new(0)));
    }

    #[test]
    fn subnet_ids_multiple_same_subcommittee() {
        // MinimalEthSpec: SyncSubcommitteeSize = 32/4 = 8
        // indices 0..7 all map to subcommittee 0
        let duty = SyncDuty {
            pubkey: pk(1),
            validator_index: 0,
            validator_sync_committee_indices: vec![0, 3, 7],
        };
        let subnets = duty.subnet_ids::<E>().unwrap();
        assert_eq!(subnets.len(), 1);
        assert!(subnets.contains(&SyncSubnetId::new(0)));
    }

    #[test]
    fn subnet_ids_multiple_different_subcommittees() {
        // index 0 → subnet 0, index 8 → subnet 1, index 16 → subnet 2
        let duty = SyncDuty {
            pubkey: pk(1),
            validator_index: 0,
            validator_sync_committee_indices: vec![0, 8, 16],
        };
        let subnets = duty.subnet_ids::<E>().unwrap();
        assert_eq!(subnets.len(), 3);
        assert!(subnets.contains(&SyncSubnetId::new(0)));
        assert!(subnets.contains(&SyncSubnetId::new(1)));
        assert!(subnets.contains(&SyncSubnetId::new(2)));
    }

    #[test]
    fn subnet_ids_empty_indices() {
        let duty = SyncDuty {
            pubkey: pk(1),
            validator_index: 0,
            validator_sync_committee_indices: vec![],
        };
        let subnets = duty.subnet_ids::<E>().unwrap();
        assert!(subnets.is_empty());
    }

    #[test]
    fn new_returns_none_for_empty_indices() {
        let result = SyncDuty::from_sync_committee_indices(1, pk(1), &[]);
        assert!(result.is_none());
    }

    #[test]
    fn duty_preserves_pubkey() {
        let expected_pk = pk(55);
        let indices: Vec<usize> = vec![5, 5]; // validator 5 at positions 0 and 1
        let duty = SyncDuty::from_sync_committee_indices(5, expected_pk, &indices).unwrap();
        assert_eq!(duty.pubkey, expected_pk);
    }
}
