use crate::*;
use serde::{Deserialize, Serialize};

/// A proposer preparation, created when a validator prepares the beacon node for potential proposers
/// by supplying information required when proposing blocks for the given validators.
#[derive(PartialEq, Debug, Serialize, Deserialize, Clone)]
pub struct ProposerPreparationData {
    /// The validators index.
    #[serde(with = "serde_utils::quoted_u64")]
    pub validator_index: u64,
    /// The fee-recipient address.
    #[serde(with = "serde_utils::address_hex")]
    pub fee_recipient: Address,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_and_eq() {
        let data = ProposerPreparationData {
            validator_index: 42,
            fee_recipient: Address::repeat_byte(0xab),
        };
        assert_eq!(data, data.clone());
    }

    #[test]
    fn serde_round_trip() {
        let data = ProposerPreparationData {
            validator_index: 100,
            fee_recipient: Address::repeat_byte(0x01),
        };
        let json = serde_json::to_string(&data).unwrap();
        let decoded: ProposerPreparationData = serde_json::from_str(&json).unwrap();
        assert_eq!(data, decoded);
    }

    #[test]
    fn serde_validator_index_quoted() {
        let data = ProposerPreparationData {
            validator_index: 12345,
            fee_recipient: Address::zero(),
        };
        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("\"12345\""));
    }

    #[test]
    fn serde_fee_recipient_hex() {
        let data = ProposerPreparationData {
            validator_index: 0,
            fee_recipient: Address::repeat_byte(0xff),
        };
        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("0xffffffffffffffffffffffffffffffffffffffff"));
    }

    #[test]
    fn inequality() {
        let data1 = ProposerPreparationData {
            validator_index: 1,
            fee_recipient: Address::zero(),
        };
        let data2 = ProposerPreparationData {
            validator_index: 2,
            fee_recipient: Address::zero(),
        };
        assert_ne!(data1, data2);
    }

    #[test]
    fn debug_format() {
        let data = ProposerPreparationData {
            validator_index: 7,
            fee_recipient: Address::zero(),
        };
        let debug = format!("{:?}", data);
        assert!(debug.contains("ProposerPreparationData"));
    }
}
