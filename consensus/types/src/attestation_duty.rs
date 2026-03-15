use crate::*;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Debug, PartialEq, Clone, Copy, Default, Serialize, Deserialize)]
pub struct AttestationDuty {
    /// The slot during which the attester must attest.
    pub slot: Slot,
    /// The index of this committee within the committees in `slot`.
    pub index: CommitteeIndex,
    /// The position of the attester within the committee.
    pub committee_position: usize,
    /// The total number of attesters in the committee.
    pub committee_len: usize,
    /// The committee count at `attestation_slot`.
    #[serde(with = "serde_utils::quoted_u64")]
    pub committees_at_slot: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let duty = AttestationDuty::default();
        assert_eq!(duty.slot, Slot::new(0));
        assert_eq!(duty.index, 0);
        assert_eq!(duty.committee_position, 0);
        assert_eq!(duty.committee_len, 0);
        assert_eq!(duty.committees_at_slot, 0);
    }

    #[test]
    fn clone_and_eq() {
        let duty = AttestationDuty {
            slot: Slot::new(10),
            index: 3,
            committee_position: 5,
            committee_len: 128,
            committees_at_slot: 64,
        };
        assert_eq!(duty, duty.clone());
    }

    #[test]
    fn copy_semantics() {
        let duty = AttestationDuty {
            slot: Slot::new(42),
            index: 1,
            committee_position: 7,
            committee_len: 64,
            committees_at_slot: 32,
        };
        let copied = duty;
        assert_eq!(duty.slot, copied.slot);
    }

    #[test]
    fn serde_round_trip() {
        let duty = AttestationDuty {
            slot: Slot::new(100),
            index: 5,
            committee_position: 10,
            committee_len: 256,
            committees_at_slot: 16,
        };
        let json = serde_json::to_string(&duty).unwrap();
        let decoded: AttestationDuty = serde_json::from_str(&json).unwrap();
        assert_eq!(duty, decoded);
    }

    #[test]
    fn serde_committees_at_slot_quoted() {
        let duty = AttestationDuty {
            slot: Slot::new(1),
            index: 0,
            committee_position: 0,
            committee_len: 0,
            committees_at_slot: 999,
        };
        let json = serde_json::to_string(&duty).unwrap();
        // committees_at_slot should be quoted as a string
        assert!(json.contains("\"999\""));
    }

    #[test]
    fn debug_format() {
        let duty = AttestationDuty {
            slot: Slot::new(7),
            index: 2,
            committee_position: 3,
            committee_len: 64,
            committees_at_slot: 8,
        };
        let debug = format!("{:?}", duty);
        assert!(debug.contains("AttestationDuty"));
    }

    #[test]
    fn inequality() {
        let duty1 = AttestationDuty {
            slot: Slot::new(1),
            index: 0,
            committee_position: 0,
            committee_len: 64,
            committees_at_slot: 4,
        };
        let duty2 = AttestationDuty {
            slot: Slot::new(2),
            index: 0,
            committee_position: 0,
            committee_len: 64,
            committees_at_slot: 4,
        };
        assert_ne!(duty1, duty2);
    }
}
