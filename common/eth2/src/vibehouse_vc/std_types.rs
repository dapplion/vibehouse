use eth2_keystore::Keystore;
use serde::{Deserialize, Serialize};
use types::{Address, Graffiti, PublicKeyBytes};
use zeroize::Zeroizing;

pub use eip_3076::Interchange;

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct GetFeeRecipientResponse {
    pub pubkey: PublicKeyBytes,
    #[serde(with = "serde_utils::address_hex")]
    pub ethaddress: Address,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct GetGasLimitResponse {
    pub pubkey: PublicKeyBytes,
    #[serde(with = "serde_utils::quoted_u64")]
    pub gas_limit: u64,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct AuthResponse {
    pub token_path: String,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct ListKeystoresResponse {
    pub data: Vec<SingleKeystoreResponse>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct SingleKeystoreResponse {
    pub validating_pubkey: PublicKeyBytes,
    pub derivation_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readonly: Option<bool>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImportKeystoresRequest {
    pub keystores: Vec<KeystoreJsonStr>,
    pub passwords: Vec<Zeroizing<String>>,
    pub slashing_protection: Option<InterchangeJsonStr>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct KeystoreJsonStr(#[serde(with = "serde_utils::json_str")] pub Keystore);

impl std::ops::Deref for KeystoreJsonStr {
    type Target = Keystore;
    fn deref(&self) -> &Keystore {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct InterchangeJsonStr(#[serde(with = "serde_utils::json_str")] pub Interchange);

#[derive(Debug, Deserialize, Serialize)]
pub struct ImportKeystoresResponse {
    pub data: Vec<Status<ImportKeystoreStatus>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Status<T> {
    pub status: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl<T> Status<T> {
    pub fn ok(status: T) -> Self {
        Self {
            status,
            message: None,
        }
    }

    pub fn error(status: T, message: String) -> Self {
        Self {
            status,
            message: Some(message),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportKeystoreStatus {
    Imported,
    Duplicate,
    Error,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteKeystoresRequest {
    pub pubkeys: Vec<PublicKeyBytes>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DeleteKeystoresResponse {
    pub data: Vec<Status<DeleteKeystoreStatus>>,
    #[serde(with = "serde_utils::json_str")]
    pub slashing_protection: Interchange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteKeystoreStatus {
    Deleted,
    NotActive,
    NotFound,
    Error,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct ListRemotekeysResponse {
    pub data: Vec<SingleListRemotekeysResponse>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct SingleListRemotekeysResponse {
    pub pubkey: PublicKeyBytes,
    pub url: String,
    pub readonly: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRemotekeysRequest {
    pub remote_keys: Vec<SingleImportRemotekeysRequest>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct SingleImportRemotekeysRequest {
    pub pubkey: PublicKeyBytes,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportRemotekeyStatus {
    Imported,
    Duplicate,
    Error,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ImportRemotekeysResponse {
    pub data: Vec<Status<ImportRemotekeyStatus>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteRemotekeysRequest {
    pub pubkeys: Vec<PublicKeyBytes>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteRemotekeyStatus {
    Deleted,
    NotFound,
    Error,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DeleteRemotekeysResponse {
    pub data: Vec<Status<DeleteRemotekeyStatus>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GetGraffitiResponse {
    pub pubkey: PublicKeyBytes,
    pub graffiti: Graffiti,
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::FixedBytesExtended;

    #[test]
    fn status_ok_has_no_message() {
        let s = Status::ok(ImportKeystoreStatus::Imported);
        assert_eq!(s.status, ImportKeystoreStatus::Imported);
        assert!(s.message.is_none());
    }

    #[test]
    fn status_error_has_message() {
        let s = Status::error(ImportKeystoreStatus::Error, "failed".into());
        assert_eq!(s.status, ImportKeystoreStatus::Error);
        assert_eq!(s.message.as_deref(), Some("failed"));
    }

    #[test]
    fn status_message_skipped_when_none() {
        let s = Status::ok(DeleteKeystoreStatus::Deleted);
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("message"));
    }

    #[test]
    fn import_keystore_status_serde() {
        let cases = vec![
            (ImportKeystoreStatus::Imported, "\"imported\""),
            (ImportKeystoreStatus::Duplicate, "\"duplicate\""),
            (ImportKeystoreStatus::Error, "\"error\""),
        ];
        for (status, expected) in cases {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, expected);
            let decoded: ImportKeystoreStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, status);
        }
    }

    #[test]
    fn delete_keystore_status_serde() {
        let cases = vec![
            (DeleteKeystoreStatus::Deleted, "\"deleted\""),
            (DeleteKeystoreStatus::NotActive, "\"not_active\""),
            (DeleteKeystoreStatus::NotFound, "\"not_found\""),
            (DeleteKeystoreStatus::Error, "\"error\""),
        ];
        for (status, expected) in cases {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, expected);
            let decoded: DeleteKeystoreStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, status);
        }
    }

    #[test]
    fn import_remotekey_status_serde() {
        let cases = vec![
            (ImportRemotekeyStatus::Imported, "\"imported\""),
            (ImportRemotekeyStatus::Duplicate, "\"duplicate\""),
            (ImportRemotekeyStatus::Error, "\"error\""),
        ];
        for (status, expected) in cases {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, expected);
        }
    }

    #[test]
    fn delete_remotekey_status_serde() {
        let cases = vec![
            (DeleteRemotekeyStatus::Deleted, "\"deleted\""),
            (DeleteRemotekeyStatus::NotFound, "\"not_found\""),
            (DeleteRemotekeyStatus::Error, "\"error\""),
        ];
        for (status, expected) in cases {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, expected);
        }
    }

    #[test]
    fn single_keystore_response_serde_roundtrip() {
        let resp = SingleKeystoreResponse {
            validating_pubkey: PublicKeyBytes::empty(),
            derivation_path: Some("m/12381/3600/0/0/0".into()),
            readonly: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let decoded: SingleKeystoreResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, decoded);
    }

    #[test]
    fn single_keystore_response_readonly_skipped_when_none() {
        let resp = SingleKeystoreResponse {
            validating_pubkey: PublicKeyBytes::empty(),
            derivation_path: None,
            readonly: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("readonly"));
    }

    #[test]
    fn get_fee_recipient_response_serde() {
        let resp = GetFeeRecipientResponse {
            pubkey: PublicKeyBytes::empty(),
            ethaddress: Address::zero(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let decoded: GetFeeRecipientResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, decoded);
    }

    #[test]
    fn get_gas_limit_response_quoted() {
        let resp = GetGasLimitResponse {
            pubkey: PublicKeyBytes::empty(),
            gas_limit: 30000000,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"30000000\""));
        let decoded: GetGasLimitResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, decoded);
    }

    #[test]
    fn auth_response_serde() {
        let resp = AuthResponse {
            token_path: "/tmp/token".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let decoded: AuthResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, decoded);
    }

    #[test]
    fn single_list_remotekeys_response_serde() {
        let resp = SingleListRemotekeysResponse {
            pubkey: PublicKeyBytes::empty(),
            url: "https://example.com".into(),
            readonly: false,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let decoded: SingleListRemotekeysResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, decoded);
    }

    #[test]
    fn single_import_remotekeys_request_serde() {
        let req = SingleImportRemotekeysRequest {
            pubkey: PublicKeyBytes::empty(),
            url: "https://signer.example.com".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: SingleImportRemotekeysRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, decoded);
    }
}
