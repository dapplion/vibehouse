use crate::*;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use tree_hash_derive::TreeHash;

/// Validator registration, for use in interacting with servers implementing the builder API.
#[derive(PartialEq, Debug, Serialize, Deserialize, Clone, Encode, Decode)]
pub struct SignedValidatorRegistrationData {
    pub message: ValidatorRegistrationData,
    pub signature: Signature,
}

#[derive(PartialEq, Debug, Serialize, Deserialize, Clone, Encode, Decode, TreeHash)]
pub struct ValidatorRegistrationData {
    #[serde(with = "serde_utils::address_hex")]
    pub fee_recipient: Address,
    #[serde(with = "serde_utils::quoted_u64")]
    pub gas_limit: u64,
    #[serde(with = "serde_utils::quoted_u64")]
    pub timestamp: u64,
    pub pubkey: PublicKeyBytes,
}

impl SignedRoot for ValidatorRegistrationData {}

impl SignedValidatorRegistrationData {
    pub fn verify_signature(&self, spec: &ChainSpec) -> bool {
        self.message
            .pubkey
            .decompress()
            .map(|pubkey| {
                let domain = spec.get_builder_domain();
                let message = self.message.signing_root(domain);
                self.signature.verify(&pubkey, message)
            })
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_hash::TreeHash;

    fn make_registration() -> ValidatorRegistrationData {
        ValidatorRegistrationData {
            fee_recipient: Address::repeat_byte(0xab),
            gas_limit: 30_000_000,
            timestamp: 1_700_000_000,
            pubkey: PublicKeyBytes::empty(),
        }
    }

    #[test]
    fn registration_clone_and_eq() {
        let reg = make_registration();
        assert_eq!(reg, reg.clone());
    }

    #[test]
    fn registration_serde_round_trip() {
        let reg = make_registration();
        let json = serde_json::to_string(&reg).unwrap();
        let decoded: ValidatorRegistrationData = serde_json::from_str(&json).unwrap();
        assert_eq!(reg, decoded);
    }

    #[test]
    fn registration_ssz_round_trip() {
        let reg = make_registration();
        let encoded = ssz::Encode::as_ssz_bytes(&reg);
        let decoded = <ValidatorRegistrationData as ssz::Decode>::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(reg, decoded);
    }

    #[test]
    fn registration_tree_hash_deterministic() {
        let reg = make_registration();
        let hash1 = reg.tree_hash_root();
        let hash2 = reg.tree_hash_root();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn registration_tree_hash_different() {
        let reg1 = make_registration();
        let mut reg2 = make_registration();
        reg2.gas_limit = 999;
        assert_ne!(reg1.tree_hash_root(), reg2.tree_hash_root());
    }

    #[test]
    fn registration_serde_gas_limit_quoted() {
        let reg = make_registration();
        let json = serde_json::to_string(&reg).unwrap();
        assert!(json.contains("\"30000000\""));
    }

    #[test]
    fn registration_serde_timestamp_quoted() {
        let reg = make_registration();
        let json = serde_json::to_string(&reg).unwrap();
        assert!(json.contains("\"1700000000\""));
    }

    #[test]
    fn signed_registration_clone_and_eq() {
        let signed = SignedValidatorRegistrationData {
            message: make_registration(),
            signature: Signature::empty(),
        };
        assert_eq!(signed, signed.clone());
    }

    #[test]
    fn signed_registration_ssz_round_trip() {
        let signed = SignedValidatorRegistrationData {
            message: make_registration(),
            signature: Signature::empty(),
        };
        let encoded = ssz::Encode::as_ssz_bytes(&signed);
        let decoded =
            <SignedValidatorRegistrationData as ssz::Decode>::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(signed, decoded);
    }

    #[test]
    fn verify_signature_empty_key_returns_false() {
        let signed = SignedValidatorRegistrationData {
            message: make_registration(),
            signature: Signature::empty(),
        };
        let spec = ChainSpec::minimal();
        // Empty pubkey can't decompress, so verify should return false
        assert!(!signed.verify_signature(&spec));
    }

    #[test]
    fn signed_root_impl() {
        // ValidatorRegistrationData implements SignedRoot
        let reg = make_registration();
        let domain = Hash256::zero();
        let root = reg.signing_root(domain);
        assert_ne!(root, Hash256::zero());
    }

    #[test]
    fn debug_format() {
        let reg = make_registration();
        let debug = format!("{:?}", reg);
        assert!(debug.contains("ValidatorRegistrationData"));
    }
}
