use crate::*;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};

/// A validator subscription, created when a validator subscribes to a slot to perform optional aggregation
/// duties.
#[derive(PartialEq, Debug, Serialize, Deserialize, Clone, Encode, Decode, Eq, PartialOrd, Ord)]
pub struct ValidatorSubscription {
    /// The index of the committee within `slot` of which the validator is a member. Used by the
    /// beacon node to quickly evaluate the associated `SubnetId`.
    pub attestation_committee_index: CommitteeIndex,
    /// The slot in which to subscribe.
    pub slot: Slot,
    /// Committee count at slot to subscribe.
    pub committee_count_at_slot: u64,
    /// If true, the validator is an aggregator and the beacon node should aggregate attestations
    /// for this slot.
    pub is_aggregator: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sub(slot: u64, index: u64, is_agg: bool) -> ValidatorSubscription {
        ValidatorSubscription {
            attestation_committee_index: index,
            slot: Slot::new(slot),
            committee_count_at_slot: 64,
            is_aggregator: is_agg,
        }
    }

    #[test]
    fn clone_and_eq() {
        let sub = make_sub(10, 3, true);
        assert_eq!(sub, sub.clone());
    }

    #[test]
    fn serde_round_trip() {
        let sub = make_sub(100, 5, false);
        let json = serde_json::to_string(&sub).unwrap();
        let decoded: ValidatorSubscription = serde_json::from_str(&json).unwrap();
        assert_eq!(sub, decoded);
    }

    #[test]
    fn ssz_round_trip() {
        let sub = make_sub(42, 7, true);
        let encoded = ssz::Encode::as_ssz_bytes(&sub);
        let decoded = <ValidatorSubscription as ssz::Decode>::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(sub, decoded);
    }

    #[test]
    fn ordering() {
        let sub1 = make_sub(1, 0, false);
        let sub2 = make_sub(2, 0, false);
        let sub3 = make_sub(1, 1, false);
        let mut subs = [sub2.clone(), sub3.clone(), sub1.clone()];
        subs.sort();
        // Ord derives field order: attestation_committee_index first, then slot
        assert_eq!(subs[0].attestation_committee_index, 0);
        assert_eq!(subs[2].attestation_committee_index, 1);
    }

    #[test]
    fn debug_format() {
        let sub = make_sub(5, 2, true);
        let debug = format!("{:?}", sub);
        assert!(debug.contains("ValidatorSubscription"));
    }

    #[test]
    fn inequality_by_slot() {
        let sub1 = make_sub(1, 0, false);
        let sub2 = make_sub(2, 0, false);
        assert_ne!(sub1, sub2);
    }

    #[test]
    fn inequality_by_aggregator() {
        let sub1 = make_sub(1, 0, false);
        let sub2 = make_sub(1, 0, true);
        assert_ne!(sub1, sub2);
    }
}
