use crate::*;

#[derive(Default, Clone, Debug, PartialEq)]
pub struct BeaconCommittee<'a> {
    pub slot: Slot,
    pub index: CommitteeIndex,
    pub committee: &'a [usize],
}

impl BeaconCommittee<'_> {
    pub fn into_owned(self) -> OwnedBeaconCommittee {
        OwnedBeaconCommittee {
            slot: self.slot,
            index: self.index,
            committee: self.committee.to_vec(),
        }
    }
}

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Default, Clone, Debug, PartialEq)]
pub struct OwnedBeaconCommittee {
    pub slot: Slot,
    pub index: CommitteeIndex,
    pub committee: Vec<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_beacon_committee() {
        let bc = BeaconCommittee::default();
        assert_eq!(bc.slot, Slot::new(0));
        assert_eq!(bc.index, 0);
        assert!(bc.committee.is_empty());
    }

    #[test]
    fn into_owned_preserves_fields() {
        let committee_data = vec![1, 2, 3, 4, 5];
        let bc = BeaconCommittee {
            slot: Slot::new(42),
            index: 7,
            committee: &committee_data,
        };
        let owned = bc.into_owned();
        assert_eq!(owned.slot, Slot::new(42));
        assert_eq!(owned.index, 7);
        assert_eq!(owned.committee, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn into_owned_empty_committee() {
        let committee_data: Vec<usize> = vec![];
        let bc = BeaconCommittee {
            slot: Slot::new(0),
            index: 0,
            committee: &committee_data,
        };
        let owned = bc.into_owned();
        assert!(owned.committee.is_empty());
    }

    #[test]
    fn owned_default() {
        let owned = OwnedBeaconCommittee::default();
        assert_eq!(owned.slot, Slot::new(0));
        assert_eq!(owned.index, 0);
        assert!(owned.committee.is_empty());
    }

    #[test]
    fn beacon_committee_equality() {
        let data1 = vec![1, 2, 3];
        let data2 = vec![1, 2, 3];
        let data3 = vec![1, 2, 4];
        let bc1 = BeaconCommittee {
            slot: Slot::new(1),
            index: 0,
            committee: &data1,
        };
        let bc2 = BeaconCommittee {
            slot: Slot::new(1),
            index: 0,
            committee: &data2,
        };
        let bc3 = BeaconCommittee {
            slot: Slot::new(1),
            index: 0,
            committee: &data3,
        };
        assert_eq!(bc1, bc2);
        assert_ne!(bc1, bc3);
    }
}
