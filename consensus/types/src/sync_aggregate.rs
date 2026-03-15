use crate::consts::altair::SYNC_COMMITTEE_SUBNET_COUNT;
use crate::context_deserialize;
use crate::test_utils::TestRandom;
use crate::{AggregateSignature, BitVector, EthSpec, ForkName, SyncCommitteeContribution};
use educe::Educe;
use safe_arith::{ArithError, SafeArith};
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

#[derive(Debug, PartialEq)]
pub enum Error {
    SszTypesError(ssz_types::Error),
    BitfieldError(ssz::BitfieldError),
    ArithError(ArithError),
}

impl From<ArithError> for Error {
    fn from(e: ArithError) -> Error {
        Error::ArithError(e)
    }
}
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary),
    arbitrary(bound(E: EthSpec))
)]
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, TreeHash, TestRandom, Educe)]
#[educe(PartialEq, Hash(bound(E: EthSpec)))]
#[serde(bound = "E: EthSpec")]
#[context_deserialize(ForkName)]
pub struct SyncAggregate<E: EthSpec> {
    pub sync_committee_bits: BitVector<E::SyncCommitteeSize>,
    pub sync_committee_signature: AggregateSignature,
}

impl<E: EthSpec> SyncAggregate<E> {
    /// New aggregate to be used as the seed for aggregating other signatures.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            sync_committee_bits: BitVector::default(),
            sync_committee_signature: AggregateSignature::infinity(),
        }
    }

    /// Create a `SyncAggregate` from a slice of `SyncCommitteeContribution`s.
    ///
    /// Equivalent to `process_sync_committee_contributions` from the spec.
    pub fn from_contributions(
        contributions: &[SyncCommitteeContribution<E>],
    ) -> Result<SyncAggregate<E>, Error> {
        let mut sync_aggregate = Self::new();
        let sync_subcommittee_size =
            E::sync_committee_size().safe_div(SYNC_COMMITTEE_SUBNET_COUNT as usize)?;
        for contribution in contributions {
            for (index, participated) in contribution.aggregation_bits.iter().enumerate() {
                if participated {
                    let participant_index = sync_subcommittee_size
                        .safe_mul(contribution.subcommittee_index as usize)?
                        .safe_add(index)?;
                    sync_aggregate
                        .sync_committee_bits
                        .set(participant_index, true)
                        .map_err(Error::BitfieldError)?;
                }
            }
            sync_aggregate
                .sync_committee_signature
                .add_assign_aggregate(&contribution.signature);
        }
        Ok(sync_aggregate)
    }

    /// Empty aggregate to be used at genesis.
    ///
    /// Contains an empty signature and should *not* be used as the starting point for aggregation,
    /// use `new` instead.
    pub fn empty() -> Self {
        Self {
            sync_committee_bits: BitVector::default(),
            sync_committee_signature: AggregateSignature::empty(),
        }
    }

    /// Returns how many bits are `true` in `self.sync_committee_bits`.
    pub fn num_set_bits(&self) -> usize {
        self.sync_committee_bits.num_set_bits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FixedBytesExtended, MinimalEthSpec};

    type E = MinimalEthSpec;

    #[test]
    fn new_has_zero_bits_set() {
        let agg = SyncAggregate::<E>::new();
        assert_eq!(agg.num_set_bits(), 0);
    }

    #[test]
    fn empty_has_zero_bits_set() {
        let agg = SyncAggregate::<E>::empty();
        assert_eq!(agg.num_set_bits(), 0);
    }

    #[test]
    fn from_contributions_empty_list() {
        let agg = SyncAggregate::<E>::from_contributions(&[]).unwrap();
        assert_eq!(agg.num_set_bits(), 0);
    }

    #[test]
    fn from_contributions_single_contribution_one_bit() {
        let mut contribution = SyncCommitteeContribution::<E> {
            slot: crate::Slot::new(0),
            beacon_block_root: crate::Hash256::zero(),
            subcommittee_index: 0,
            aggregation_bits: BitVector::new(),
            signature: AggregateSignature::infinity(),
        };
        contribution.aggregation_bits.set(2, true).unwrap();

        let agg = SyncAggregate::<E>::from_contributions(&[contribution]).unwrap();
        assert_eq!(agg.num_set_bits(), 1);
        // subcommittee 0, index 2 → global index 2
        assert!(agg.sync_committee_bits.get(2).unwrap());
    }

    #[test]
    fn from_contributions_different_subcommittees() {
        // MinimalEthSpec: SyncCommitteeSize=32, SYNC_COMMITTEE_SUBNET_COUNT=4
        // subcommittee_size = 32/4 = 8
        let mut c0 = SyncCommitteeContribution::<E> {
            slot: crate::Slot::new(0),
            beacon_block_root: crate::Hash256::zero(),
            subcommittee_index: 0,
            aggregation_bits: BitVector::new(),
            signature: AggregateSignature::infinity(),
        };
        c0.aggregation_bits.set(0, true).unwrap();

        let mut c1 = SyncCommitteeContribution::<E> {
            slot: crate::Slot::new(0),
            beacon_block_root: crate::Hash256::zero(),
            subcommittee_index: 1,
            aggregation_bits: BitVector::new(),
            signature: AggregateSignature::infinity(),
        };
        c1.aggregation_bits.set(3, true).unwrap();

        let agg = SyncAggregate::<E>::from_contributions(&[c0, c1]).unwrap();
        assert_eq!(agg.num_set_bits(), 2);
        // subcommittee 0, index 0 → global 0
        assert!(agg.sync_committee_bits.get(0).unwrap());
        // subcommittee 1, index 3 → global 8 + 3 = 11
        assert!(agg.sync_committee_bits.get(11).unwrap());
    }

    #[test]
    fn from_contributions_multiple_bits_same_subcommittee() {
        let mut c = SyncCommitteeContribution::<E> {
            slot: crate::Slot::new(0),
            beacon_block_root: crate::Hash256::zero(),
            subcommittee_index: 2,
            aggregation_bits: BitVector::new(),
            signature: AggregateSignature::infinity(),
        };
        c.aggregation_bits.set(0, true).unwrap();
        c.aggregation_bits.set(1, true).unwrap();
        c.aggregation_bits.set(7, true).unwrap();

        let agg = SyncAggregate::<E>::from_contributions(&[c]).unwrap();
        assert_eq!(agg.num_set_bits(), 3);
        // subcommittee 2: global offset = 2 * 8 = 16
        assert!(agg.sync_committee_bits.get(16).unwrap());
        assert!(agg.sync_committee_bits.get(17).unwrap());
        assert!(agg.sync_committee_bits.get(23).unwrap());
    }

    #[test]
    fn from_contributions_all_subcommittees_full() {
        // Fill all 4 subcommittees completely (8 bits each = 32 total)
        let contributions: Vec<_> = (0..4u64)
            .map(|sub_idx| {
                let mut c = SyncCommitteeContribution::<E> {
                    slot: crate::Slot::new(0),
                    beacon_block_root: crate::Hash256::zero(),
                    subcommittee_index: sub_idx,
                    aggregation_bits: BitVector::new(),
                    signature: AggregateSignature::infinity(),
                };
                for i in 0..8 {
                    c.aggregation_bits.set(i, true).unwrap();
                }
                c
            })
            .collect();

        let agg = SyncAggregate::<E>::from_contributions(&contributions).unwrap();
        assert_eq!(agg.num_set_bits(), 32);
    }

    #[test]
    fn from_contributions_overlapping_bits_idempotent() {
        // Two contributions for same subcommittee, same bit
        let make_contribution = || {
            let mut c = SyncCommitteeContribution::<E> {
                slot: crate::Slot::new(0),
                beacon_block_root: crate::Hash256::zero(),
                subcommittee_index: 0,
                aggregation_bits: BitVector::new(),
                signature: AggregateSignature::infinity(),
            };
            c.aggregation_bits.set(0, true).unwrap();
            c
        };

        let agg =
            SyncAggregate::<E>::from_contributions(&[make_contribution(), make_contribution()])
                .unwrap();
        // Setting same bit twice still results in 1 bit set
        assert_eq!(agg.num_set_bits(), 1);
    }

    #[test]
    fn num_set_bits_zero_on_default_bits() {
        let agg = SyncAggregate::<E> {
            sync_committee_bits: BitVector::default(),
            sync_committee_signature: AggregateSignature::infinity(),
        };
        assert_eq!(agg.num_set_bits(), 0);
    }
}
