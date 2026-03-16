//! Contains the types required to make JSON requests to Web3Signer servers.

use super::Error;
use serde::{Deserialize, Serialize};
use types::*;

#[derive(Debug, PartialEq, Copy, Clone, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MessageType {
    AggregationSlot,
    AggregateAndProof,
    Attestation,
    BlockV2,
    Deposit,
    RandaoReveal,
    VoluntaryExit,
    SyncCommitteeMessage,
    SyncCommitteeSelectionProof,
    SyncCommitteeContributionAndProof,
    ValidatorRegistration,
}

#[derive(Debug, PartialEq, Copy, Clone, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ForkName {
    Phase0,
    Altair,
    Bellatrix,
    Capella,
    Deneb,
    Electra,
    Fulu,
    Gloas,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct ForkInfo {
    pub fork: Fork,
    pub genesis_validators_root: Hash256,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(bound = "E: EthSpec", rename_all = "snake_case")]
pub enum Web3SignerObject<'a, E: EthSpec, Payload: AbstractExecPayload<E>> {
    AggregationSlot {
        slot: Slot,
    },
    AggregateAndProof(AggregateAndProofRef<'a, E>),
    Attestation(&'a AttestationData),
    BeaconBlock {
        version: ForkName,
        #[serde(skip_serializing_if = "Option::is_none")]
        block: Option<&'a BeaconBlock<E, Payload>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        block_header: Option<BeaconBlockHeader>,
    },
    #[allow(dead_code)]
    Deposit {
        pubkey: PublicKeyBytes,
        withdrawal_credentials: Hash256,
        #[serde(with = "serde_utils::quoted_u64")]
        amount: u64,
        #[serde(with = "serde_utils::bytes_4_hex")]
        genesis_fork_version: [u8; 4],
    },
    RandaoReveal {
        epoch: Epoch,
    },
    VoluntaryExit(&'a VoluntaryExit),
    SyncCommitteeMessage {
        beacon_block_root: Hash256,
        slot: Slot,
    },
    SyncAggregatorSelectionData(&'a SyncAggregatorSelectionData),
    ContributionAndProof(&'a ContributionAndProof<E>),
    ValidatorRegistration(&'a ValidatorRegistrationData),
}

impl<'a, E: EthSpec, Payload: AbstractExecPayload<E>> Web3SignerObject<'a, E, Payload> {
    pub fn beacon_block(block: &'a BeaconBlock<E, Payload>) -> Result<Self, Error> {
        match block {
            BeaconBlock::Base(_) => Ok(Web3SignerObject::BeaconBlock {
                version: ForkName::Phase0,
                block: Some(block),
                block_header: None,
            }),
            BeaconBlock::Altair(_) => Ok(Web3SignerObject::BeaconBlock {
                version: ForkName::Altair,
                block: Some(block),
                block_header: None,
            }),
            BeaconBlock::Bellatrix(_) => Ok(Web3SignerObject::BeaconBlock {
                version: ForkName::Bellatrix,
                block: None,
                block_header: Some(block.block_header()),
            }),
            BeaconBlock::Capella(_) => Ok(Web3SignerObject::BeaconBlock {
                version: ForkName::Capella,
                block: None,
                block_header: Some(block.block_header()),
            }),
            BeaconBlock::Deneb(_) => Ok(Web3SignerObject::BeaconBlock {
                version: ForkName::Deneb,
                block: None,
                block_header: Some(block.block_header()),
            }),
            BeaconBlock::Electra(_) => Ok(Web3SignerObject::BeaconBlock {
                version: ForkName::Electra,
                block: None,
                block_header: Some(block.block_header()),
            }),
            BeaconBlock::Fulu(_) => Ok(Web3SignerObject::BeaconBlock {
                version: ForkName::Fulu,
                block: None,
                block_header: Some(block.block_header()),
            }),
            BeaconBlock::Gloas(_) => Ok(Web3SignerObject::BeaconBlock {
                version: ForkName::Gloas,
                block: None,
                block_header: Some(block.block_header()),
            }),
        }
    }

    pub fn message_type(&self) -> MessageType {
        match self {
            Web3SignerObject::AggregationSlot { .. } => MessageType::AggregationSlot,
            Web3SignerObject::AggregateAndProof(_) => MessageType::AggregateAndProof,
            Web3SignerObject::Attestation(_) => MessageType::Attestation,
            Web3SignerObject::BeaconBlock { .. } => MessageType::BlockV2,
            Web3SignerObject::Deposit { .. } => MessageType::Deposit,
            Web3SignerObject::RandaoReveal { .. } => MessageType::RandaoReveal,
            Web3SignerObject::VoluntaryExit(_) => MessageType::VoluntaryExit,
            Web3SignerObject::SyncCommitteeMessage { .. } => MessageType::SyncCommitteeMessage,
            Web3SignerObject::SyncAggregatorSelectionData(_) => {
                MessageType::SyncCommitteeSelectionProof
            }
            Web3SignerObject::ContributionAndProof(_) => {
                MessageType::SyncCommitteeContributionAndProof
            }
            Web3SignerObject::ValidatorRegistration(_) => MessageType::ValidatorRegistration,
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(bound = "E: EthSpec")]
pub struct SigningRequest<'a, E: EthSpec, Payload: AbstractExecPayload<E>> {
    #[serde(rename = "type")]
    pub message_type: MessageType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fork_info: Option<ForkInfo>,
    #[serde(rename = "signingRoot")]
    pub signing_root: Hash256,
    #[serde(flatten)]
    pub object: Web3SignerObject<'a, E, Payload>,
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct SigningResponse {
    pub signature: Signature,
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::MinimalEthSpec;

    type E = MinimalEthSpec;

    #[test]
    fn message_type_serde_screaming_snake() {
        assert_eq!(
            serde_json::to_string(&MessageType::AggregationSlot).unwrap(),
            r#""AGGREGATION_SLOT""#
        );
        assert_eq!(
            serde_json::to_string(&MessageType::AggregateAndProof).unwrap(),
            r#""AGGREGATE_AND_PROOF""#
        );
        assert_eq!(
            serde_json::to_string(&MessageType::Attestation).unwrap(),
            r#""ATTESTATION""#
        );
        assert_eq!(
            serde_json::to_string(&MessageType::BlockV2).unwrap(),
            r#""BLOCK_V2""#
        );
        assert_eq!(
            serde_json::to_string(&MessageType::Deposit).unwrap(),
            r#""DEPOSIT""#
        );
        assert_eq!(
            serde_json::to_string(&MessageType::RandaoReveal).unwrap(),
            r#""RANDAO_REVEAL""#
        );
        assert_eq!(
            serde_json::to_string(&MessageType::VoluntaryExit).unwrap(),
            r#""VOLUNTARY_EXIT""#
        );
        assert_eq!(
            serde_json::to_string(&MessageType::SyncCommitteeMessage).unwrap(),
            r#""SYNC_COMMITTEE_MESSAGE""#
        );
        assert_eq!(
            serde_json::to_string(&MessageType::SyncCommitteeSelectionProof).unwrap(),
            r#""SYNC_COMMITTEE_SELECTION_PROOF""#
        );
        assert_eq!(
            serde_json::to_string(&MessageType::SyncCommitteeContributionAndProof).unwrap(),
            r#""SYNC_COMMITTEE_CONTRIBUTION_AND_PROOF""#
        );
        assert_eq!(
            serde_json::to_string(&MessageType::ValidatorRegistration).unwrap(),
            r#""VALIDATOR_REGISTRATION""#
        );
    }

    #[test]
    fn fork_name_serde_screaming_snake() {
        assert_eq!(
            serde_json::to_string(&ForkName::Phase0).unwrap(),
            r#""PHASE0""#
        );
        assert_eq!(
            serde_json::to_string(&ForkName::Altair).unwrap(),
            r#""ALTAIR""#
        );
        assert_eq!(
            serde_json::to_string(&ForkName::Bellatrix).unwrap(),
            r#""BELLATRIX""#
        );
        assert_eq!(
            serde_json::to_string(&ForkName::Capella).unwrap(),
            r#""CAPELLA""#
        );
        assert_eq!(
            serde_json::to_string(&ForkName::Deneb).unwrap(),
            r#""DENEB""#
        );
        assert_eq!(
            serde_json::to_string(&ForkName::Electra).unwrap(),
            r#""ELECTRA""#
        );
        assert_eq!(serde_json::to_string(&ForkName::Fulu).unwrap(), r#""FULU""#);
        assert_eq!(
            serde_json::to_string(&ForkName::Gloas).unwrap(),
            r#""GLOAS""#
        );
    }

    #[test]
    fn fork_name_eq_and_copy() {
        let f = ForkName::Gloas;
        let f2 = f; // Copy
        assert_eq!(f, f2);
        assert_ne!(ForkName::Phase0, ForkName::Altair);
    }

    #[test]
    fn message_type_eq_and_copy() {
        let m = MessageType::Attestation;
        let m2 = m; // Copy
        assert_eq!(m, m2);
        assert_ne!(MessageType::Attestation, MessageType::Deposit);
    }

    #[test]
    fn message_type_aggregation_slot() {
        let obj: Web3SignerObject<E, FullPayload<E>> = Web3SignerObject::AggregationSlot {
            slot: Slot::new(42),
        };
        assert_eq!(obj.message_type(), MessageType::AggregationSlot);
    }

    #[test]
    fn message_type_attestation() {
        let data = AttestationData::default();
        let obj: Web3SignerObject<E, FullPayload<E>> = Web3SignerObject::Attestation(&data);
        assert_eq!(obj.message_type(), MessageType::Attestation);
    }

    #[test]
    fn message_type_randao_reveal() {
        let obj: Web3SignerObject<E, FullPayload<E>> = Web3SignerObject::RandaoReveal {
            epoch: Epoch::new(1),
        };
        assert_eq!(obj.message_type(), MessageType::RandaoReveal);
    }

    #[test]
    fn message_type_voluntary_exit() {
        let exit = VoluntaryExit {
            epoch: Epoch::new(1),
            validator_index: 0,
        };
        let obj: Web3SignerObject<E, FullPayload<E>> = Web3SignerObject::VoluntaryExit(&exit);
        assert_eq!(obj.message_type(), MessageType::VoluntaryExit);
    }

    #[test]
    fn message_type_sync_committee_message() {
        let obj: Web3SignerObject<E, FullPayload<E>> = Web3SignerObject::SyncCommitteeMessage {
            beacon_block_root: Hash256::zero(),
            slot: Slot::new(0),
        };
        assert_eq!(obj.message_type(), MessageType::SyncCommitteeMessage);
    }

    #[test]
    fn fork_info_serde() {
        let fi = ForkInfo {
            fork: Fork {
                previous_version: [0; 4],
                current_version: [1; 4],
                epoch: Epoch::new(10),
            },
            genesis_validators_root: Hash256::zero(),
        };
        let json = serde_json::to_string(&fi).unwrap();
        assert!(json.contains("previous_version"));
        assert!(json.contains("genesis_validators_root"));
    }

    #[test]
    fn aggregation_slot_serde() {
        let obj: Web3SignerObject<E, FullPayload<E>> = Web3SignerObject::AggregationSlot {
            slot: Slot::new(99),
        };
        let json = serde_json::to_string(&obj).unwrap();
        assert!(json.contains("99"));
    }
}
