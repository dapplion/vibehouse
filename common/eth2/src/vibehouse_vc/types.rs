pub use crate::types::{GenericResponse, VersionData};
pub use crate::vibehouse::Health;
pub use crate::vibehouse_vc::std_types::*;
use eth2_keystore::Keystore;
use graffiti::GraffitiString;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
pub use types::*;
use zeroize::Zeroizing;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidatorData {
    pub enabled: bool,
    pub description: String,
    pub voting_pubkey: PublicKeyBytes,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidatorRequest {
    pub enable: bool,
    pub description: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graffiti: Option<GraffitiString>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_fee_recipient: Option<Address>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub builder_proposals: Option<bool>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub builder_boost_factor: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefer_builder_proposals: Option<bool>,
    #[serde(with = "serde_utils::quoted_u64")]
    pub deposit_gwei: u64,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateValidatorsMnemonicRequest {
    pub mnemonic: Zeroizing<String>,
    #[serde(with = "serde_utils::quoted_u32")]
    pub key_derivation_path_offset: u32,
    pub validators: Vec<ValidatorRequest>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreatedValidator {
    pub enabled: bool,
    pub description: String,
    pub voting_pubkey: PublicKeyBytes,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graffiti: Option<GraffitiString>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_fee_recipient: Option<Address>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub builder_proposals: Option<bool>,
    pub eth1_deposit_tx_data: String,
    #[serde(with = "serde_utils::quoted_u64")]
    pub deposit_gwei: u64,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct PostValidatorsResponseData {
    pub mnemonic: Zeroizing<String>,
    pub validators: Vec<CreatedValidator>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidatorPatchRequest {
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub builder_proposals: Option<bool>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graffiti: Option<GraffitiString>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub builder_boost_factor: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefer_builder_proposals: Option<bool>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct KeystoreValidatorsPostRequest {
    pub password: Zeroizing<String>,
    pub enable: bool,
    pub keystore: Keystore,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graffiti: Option<GraffitiString>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_fee_recipient: Option<Address>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub builder_proposals: Option<bool>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub builder_boost_factor: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefer_builder_proposals: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Web3SignerValidatorRequest {
    pub enable: bool,
    pub description: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graffiti: Option<GraffitiString>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_fee_recipient: Option<Address>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub builder_proposals: Option<bool>,
    pub voting_public_key: PublicKey,
    pub url: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_certificate_path: Option<PathBuf>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_identity_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_identity_password: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub builder_boost_factor: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefer_builder_proposals: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct UpdateFeeRecipientRequest {
    #[serde(with = "serde_utils::address_hex")]
    pub ethaddress: Address,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct UpdateGasLimitRequest {
    #[serde(with = "serde_utils::quoted_u64")]
    pub gas_limit: u64,
}

#[derive(Deserialize)]
pub struct VoluntaryExitQuery {
    pub epoch: Option<Epoch>,
}

#[derive(Deserialize, Serialize)]
pub struct ExportKeystoresResponse {
    pub data: Vec<SingleExportKeystoresResponse>,
    #[serde(with = "serde_utils::json_str")]
    pub slashing_protection: Interchange,
}

#[derive(Deserialize, Serialize)]
pub struct SingleExportKeystoresResponse {
    pub status: Status<DeleteKeystoreStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validating_keystore: Option<KeystoreJsonStr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validating_keystore_password: Option<Zeroizing<String>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SetGraffitiRequest {
    pub graffiti: GraffitiString,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct UpdateCandidatesRequest {
    pub beacon_nodes: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct UpdateCandidatesResponse {
    pub new_beacon_nodes_list: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validator_data_serde_roundtrip() {
        let data = ValidatorData {
            enabled: true,
            description: "test validator".to_string(),
            voting_pubkey: PublicKeyBytes::empty(),
        };
        let json = serde_json::to_string(&data).unwrap();
        let decoded: ValidatorData = serde_json::from_str(&json).unwrap();
        assert!(decoded.enabled);
        assert_eq!(decoded.description, "test validator");
    }

    #[test]
    fn validator_request_serde_roundtrip() {
        let req = ValidatorRequest {
            enable: true,
            description: "my validator".to_string(),
            graffiti: None,
            suggested_fee_recipient: None,
            gas_limit: Some(30_000_000),
            builder_proposals: Some(true),
            builder_boost_factor: None,
            prefer_builder_proposals: None,
            deposit_gwei: 32_000_000_000,
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: ValidatorRequest = serde_json::from_str(&json).unwrap();
        assert!(decoded.enable);
        assert_eq!(decoded.gas_limit, Some(30_000_000));
        assert_eq!(decoded.deposit_gwei, 32_000_000_000);
    }

    #[test]
    fn validator_request_optional_fields_absent() {
        let json = r#"{"enable":true,"description":"test","deposit_gwei":"1000"}"#;
        let req: ValidatorRequest = serde_json::from_str(json).unwrap();
        assert!(req.graffiti.is_none());
        assert!(req.suggested_fee_recipient.is_none());
        assert!(req.gas_limit.is_none());
        assert!(req.builder_proposals.is_none());
    }

    #[test]
    fn validator_patch_request_all_none() {
        let patch = ValidatorPatchRequest {
            enabled: None,
            gas_limit: None,
            builder_proposals: None,
            graffiti: None,
            builder_boost_factor: None,
            prefer_builder_proposals: None,
        };
        let json = serde_json::to_string(&patch).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn validator_patch_request_some_fields() {
        let patch = ValidatorPatchRequest {
            enabled: Some(false),
            gas_limit: Some(25_000_000),
            builder_proposals: None,
            graffiti: None,
            builder_boost_factor: Some(100),
            prefer_builder_proposals: Some(true),
        };
        let json = serde_json::to_string(&patch).unwrap();
        let decoded: ValidatorPatchRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.enabled, Some(false));
        assert_eq!(decoded.gas_limit, Some(25_000_000));
        assert!(decoded.builder_proposals.is_none());
        assert_eq!(decoded.builder_boost_factor, Some(100));
        assert_eq!(decoded.prefer_builder_proposals, Some(true));
    }

    #[test]
    fn created_validator_serde_roundtrip() {
        let cv = CreatedValidator {
            enabled: true,
            description: "created".to_string(),
            voting_pubkey: PublicKeyBytes::empty(),
            graffiti: None,
            suggested_fee_recipient: None,
            gas_limit: None,
            builder_proposals: None,
            eth1_deposit_tx_data: "0x1234".to_string(),
            deposit_gwei: 32_000_000_000,
        };
        let json = serde_json::to_string(&cv).unwrap();
        let decoded: CreatedValidator = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.eth1_deposit_tx_data, "0x1234");
        assert_eq!(decoded.deposit_gwei, 32_000_000_000);
    }

    #[test]
    fn update_gas_limit_request_quoted_u64() {
        let json = r#"{"gas_limit":"30000000"}"#;
        let req: UpdateGasLimitRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.gas_limit, 30_000_000);
        let out = serde_json::to_string(&req).unwrap();
        assert!(out.contains("\"30000000\""));
    }

    #[test]
    fn voluntary_exit_query_none_epoch() {
        let json = "{}";
        let q: VoluntaryExitQuery = serde_json::from_str(json).unwrap();
        assert!(q.epoch.is_none());
    }

    #[test]
    fn set_graffiti_request_serde() {
        let req = SetGraffitiRequest {
            graffiti: GraffitiString::default(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: SetGraffitiRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.graffiti, req.graffiti);
    }

    #[test]
    fn update_candidates_request_serde() {
        let req = UpdateCandidatesRequest {
            beacon_nodes: vec!["http://localhost:5052".to_string()],
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: UpdateCandidatesRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.beacon_nodes.len(), 1);
    }

    #[test]
    fn update_candidates_response_serde() {
        let resp = UpdateCandidatesResponse {
            new_beacon_nodes_list: vec!["http://node1".to_string(), "http://node2".to_string()],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let decoded: UpdateCandidatesResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.new_beacon_nodes_list.len(), 2);
    }

    #[test]
    fn web3signer_validator_request_serde() {
        let json = serde_json::json!({
            "enable": true,
            "description": "web3signer",
            "voting_public_key": "0x933ad9491b62059dd065b560d256d8957a8c402cc6e8d8ee7290ae11e8f7329267a8811c397529dac52ae1342ba58c95",
            "url": "http://localhost:9000"
        });
        let req: Web3SignerValidatorRequest = serde_json::from_value(json).unwrap();
        assert!(req.enable);
        assert_eq!(req.description, "web3signer");
        assert_eq!(req.url, "http://localhost:9000");
        assert!(req.root_certificate_path.is_none());
        assert!(req.request_timeout_ms.is_none());
    }

    #[test]
    fn update_fee_recipient_serde() {
        let json = r#"{"ethaddress":"0x0000000000000000000000000000000000000001"}"#;
        let req: UpdateFeeRecipientRequest = serde_json::from_str(json).unwrap();
        let out = serde_json::to_string(&req).unwrap();
        assert!(out.contains("0x0000000000000000000000000000000000000001"));
    }
}
