use crate::context_deserialize;
use crate::test_utils::TestRandom;
use crate::{EthSpec, FixedVector, ForkName, SyncSubnetId};
use bls::PublicKeyBytes;
use safe_arith::{ArithError, SafeArith};
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use std::collections::HashMap;
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

#[derive(Debug, PartialEq)]
pub enum Error {
    ArithError(ArithError),
    InvalidSubcommitteeRange {
        start_subcommittee_index: usize,
        end_subcommittee_index: usize,
        subcommittee_index: usize,
    },
}

impl From<ArithError> for Error {
    fn from(e: ArithError) -> Error {
        Error::ArithError(e)
    }
}

#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary),
    arbitrary(bound = "E: EthSpec")
)]
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize, Encode, Decode, TreeHash, TestRandom)]
#[serde(bound = "E: EthSpec")]
#[context_deserialize(ForkName)]
pub struct SyncCommittee<E: EthSpec> {
    pub pubkeys: FixedVector<PublicKeyBytes, E::SyncCommitteeSize>,
    pub aggregate_pubkey: PublicKeyBytes,
}

impl<E: EthSpec> SyncCommittee<E> {
    /// Create a temporary sync committee that should *never* be included in a legitimate consensus object.
    pub fn temporary() -> Self {
        Self {
            pubkeys: FixedVector::from_elem(PublicKeyBytes::empty()),
            aggregate_pubkey: PublicKeyBytes::empty(),
        }
    }

    /// Return the pubkeys in this `SyncCommittee` for the given `subcommittee_index`.
    pub fn get_subcommittee_pubkeys(
        &self,
        subcommittee_index: usize,
    ) -> Result<&[PublicKeyBytes], Error> {
        let start_subcommittee_index = subcommittee_index.safe_mul(E::sync_subcommittee_size())?;
        let end_subcommittee_index =
            start_subcommittee_index.safe_add(E::sync_subcommittee_size())?;
        self.pubkeys
            .get(start_subcommittee_index..end_subcommittee_index)
            .ok_or(Error::InvalidSubcommitteeRange {
                start_subcommittee_index,
                end_subcommittee_index,
                subcommittee_index,
            })
    }

    /// For a given `pubkey`, finds all subcommittees that it is included in, and maps the
    /// subcommittee index (typed as `SyncSubnetId`) to all positions this `pubkey` is associated
    /// with within the subcommittee.
    pub fn subcommittee_positions_for_public_key(
        &self,
        pubkey: &PublicKeyBytes,
    ) -> Result<HashMap<SyncSubnetId, Vec<usize>>, Error> {
        let mut subnet_positions = HashMap::new();
        for (committee_index, validator_pubkey) in self.pubkeys.iter().enumerate() {
            if pubkey == validator_pubkey {
                let subcommittee_index = committee_index.safe_div(E::sync_subcommittee_size())?;
                let position_in_subcommittee =
                    committee_index.safe_rem(E::sync_subcommittee_size())?;
                subnet_positions
                    .entry(SyncSubnetId::new(subcommittee_index as u64))
                    .or_insert_with(Vec::new)
                    .push(position_in_subcommittee);
            }
        }
        Ok(subnet_positions)
    }

    /// Returns `true` if the pubkey exists in the `SyncCommittee`.
    pub fn contains(&self, pubkey: &PublicKeyBytes) -> bool {
        self.pubkeys.contains(pubkey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MinimalEthSpec;

    type E = MinimalEthSpec;

    fn make_pubkey(byte: u8) -> PublicKeyBytes {
        let mut bytes = [0u8; 48];
        bytes[0] = byte;
        PublicKeyBytes::deserialize(&bytes).unwrap()
    }

    fn make_committee(fill: u8) -> SyncCommittee<E> {
        let pubkeys = FixedVector::from_elem(make_pubkey(fill));
        SyncCommittee {
            pubkeys,
            aggregate_pubkey: make_pubkey(fill),
        }
    }

    #[test]
    fn temporary_uses_empty_pubkeys() {
        let committee = SyncCommittee::<E>::temporary();
        assert!(
            committee
                .pubkeys
                .iter()
                .all(|pk| *pk == PublicKeyBytes::empty())
        );
        assert_eq!(committee.aggregate_pubkey, PublicKeyBytes::empty());
    }

    #[test]
    fn contains_present_pubkey() {
        let pk = make_pubkey(42);
        let mut committee = SyncCommittee::<E>::temporary();
        committee.pubkeys[0] = pk;
        assert!(committee.contains(&pk));
    }

    #[test]
    fn contains_absent_pubkey() {
        let committee = SyncCommittee::<E>::temporary();
        let pk = make_pubkey(99);
        assert!(!committee.contains(&pk));
    }

    #[test]
    fn get_subcommittee_pubkeys_valid() {
        let committee = make_committee(1);
        let subcommittee_size = E::sync_subcommittee_size();
        let sub = committee.get_subcommittee_pubkeys(0).unwrap();
        assert_eq!(sub.len(), subcommittee_size);
    }

    #[test]
    fn get_subcommittee_pubkeys_out_of_range() {
        let committee = make_committee(1);
        // There are SYNC_COMMITTEE_SUBNET_COUNT subcommittees
        let subnet_count = crate::consts::altair::SYNC_COMMITTEE_SUBNET_COUNT as usize;
        assert!(committee.get_subcommittee_pubkeys(subnet_count).is_err());
    }

    #[test]
    fn subcommittee_positions_empty_for_absent_key() {
        let committee = make_committee(1);
        let pk = make_pubkey(99);
        let positions = committee
            .subcommittee_positions_for_public_key(&pk)
            .unwrap();
        assert!(positions.is_empty());
    }

    #[test]
    fn subcommittee_positions_for_all_same_key() {
        // All positions filled with same key
        let pk = make_pubkey(7);
        let committee = make_committee(7);
        let positions = committee
            .subcommittee_positions_for_public_key(&pk)
            .unwrap();
        // Should be present in all subcommittees
        let subnet_count = crate::consts::altair::SYNC_COMMITTEE_SUBNET_COUNT as usize;
        assert_eq!(positions.len(), subnet_count);
        let mut total_positions = 0usize;
        for v in positions.values() {
            total_positions += v.len();
        }
        assert_eq!(total_positions, E::sync_committee_size());
    }

    #[test]
    fn subcommittee_positions_single_occurrence() {
        let mut committee = SyncCommittee::<E>::temporary();
        let pk = make_pubkey(42);
        // Place at position 0 (subcommittee 0, position 0)
        committee.pubkeys[0] = pk;
        let positions = committee
            .subcommittee_positions_for_public_key(&pk)
            .unwrap();
        assert_eq!(positions.len(), 1);
        let sub0 = positions.get(&SyncSubnetId::new(0)).unwrap();
        assert_eq!(sub0, &vec![0]);
    }
}
