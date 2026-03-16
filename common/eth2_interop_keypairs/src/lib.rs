//! Produces the "deterministic" validator private keys used for inter-operability testing for
//! Ethereum 2.0 clients.
//!
//! Each private key is the sha2 hash of the validator index (little-endian, padded to 32 bytes),
//! modulo the BLS-381 curve order.
//!
//! Keys generated here are **not secret** and are **not for production use**. It is trivial to
//! know the secret key for any validator.
//!
//!## Reference
//!
//! Reference implementation:
//!
//! <https://github.com/ethereum/eth2.0-pm/blob/6e41fcf383ebeb5125938850d8e9b4e9888389b4/interop/mocked_start/keygen.py>
//!
//!
//! This implementation passes the [reference implementation
//! tests](https://github.com/ethereum/eth2.0-pm/blob/6e41fcf383ebeb5125938850d8e9b4e9888389b4/interop/mocked_start/keygen_test_vector.yaml).
use bls::{Keypair, PublicKey, SecretKey};
use ethereum_hashing::hash_fixed;
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::PathBuf;
use std::sync::LazyLock;

pub const PRIVATE_KEY_BYTES: usize = 32;
pub const PUBLIC_KEY_BYTES: usize = 48;
pub const HASH_BYTES: usize = 32;

static CURVE_ORDER: LazyLock<BigUint> = LazyLock::new(|| {
    "52435875175126190479447740508185965837690552500527637822603658699938581184513"
        .parse::<BigUint>()
        .expect("Curve order should be valid")
});

/// Return a G1 point for the given `validator_index`, encoded as a compressed point in
/// big-endian byte-ordering.
pub fn be_private_key(validator_index: usize) -> [u8; PRIVATE_KEY_BYTES] {
    let preimage = {
        let mut bytes = [0; HASH_BYTES];
        let index = validator_index.to_le_bytes();
        bytes[0..index.len()].copy_from_slice(&index);
        bytes
    };

    let privkey = BigUint::from_bytes_le(&hash_fixed(&preimage)) % &*CURVE_ORDER;

    let mut bytes = [0; PRIVATE_KEY_BYTES];
    let privkey_bytes = privkey.to_bytes_be();
    bytes[PRIVATE_KEY_BYTES - privkey_bytes.len()..].copy_from_slice(&privkey_bytes);
    bytes
}

/// Return a public and private keypair for a given `validator_index`.
pub fn keypair(validator_index: usize) -> Keypair {
    let sk = SecretKey::deserialize(&be_private_key(validator_index)).unwrap_or_else(|_| {
        panic!(
            "Should build valid private key for validator index {}",
            validator_index
        )
    });

    Keypair::from_components(sk.public_key(), sk)
}

#[derive(Serialize, Deserialize)]
struct YamlKeypair {
    /// Big-endian.
    privkey: String,
    /// Big-endian.
    pubkey: String,
}

impl TryInto<Keypair> for YamlKeypair {
    type Error = String;

    fn try_into(self) -> Result<Keypair, Self::Error> {
        let privkey = string_to_bytes(&self.privkey)?;
        let pubkey = string_to_bytes(&self.pubkey)?;

        if (privkey.len() > PRIVATE_KEY_BYTES) || (pubkey.len() > PUBLIC_KEY_BYTES) {
            return Err("Public or private key is too long".into());
        }

        let sk = {
            let mut bytes = vec![0; PRIVATE_KEY_BYTES - privkey.len()];
            bytes.extend_from_slice(&privkey);
            SecretKey::deserialize(&bytes)
                .map_err(|e| format!("Failed to decode bytes into secret key: {:?}", e))?
        };

        let pk = {
            let mut bytes = vec![0; PUBLIC_KEY_BYTES - pubkey.len()];
            bytes.extend_from_slice(&pubkey);
            PublicKey::deserialize(&bytes)
                .map_err(|e| format!("Failed to decode bytes into public key: {:?}", e))?
        };

        Ok(Keypair::from_components(pk, sk))
    }
}

fn string_to_bytes(string: &str) -> Result<Vec<u8>, String> {
    let string = if let Some(stripped) = string.strip_prefix("0x") {
        stripped
    } else {
        string
    };

    hex::decode(string).map_err(|e| format!("Unable to decode public or private key: {}", e))
}

/// Loads keypairs from a YAML encoded file.
///
/// Uses this as reference:
/// <https://github.com/ethereum/eth2.0-pm/blob/9a9dbcd95e2b8e10287797bd768014ab3d842e99/interop/mocked_start/keygen_10_validators.yaml>
pub fn keypairs_from_yaml_file(path: PathBuf) -> Result<Vec<Keypair>, String> {
    let file = File::open(path).map_err(|e| format!("Unable to open YAML key file: {}", e))?;

    serde_yaml::from_reader::<_, Vec<YamlKeypair>>(file)
        .map_err(|e| format!("Could not parse YAML: {:?}", e))?
        .into_iter()
        .map(TryInto::try_into)
        .collect::<Result<Vec<_>, String>>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn be_private_key_deterministic() {
        // Same index always produces same key
        let key1 = be_private_key(0);
        let key2 = be_private_key(0);
        assert_eq!(key1, key2);
    }

    #[test]
    fn be_private_key_different_indices() {
        let key0 = be_private_key(0);
        let key1 = be_private_key(1);
        assert_ne!(key0, key1);
    }

    #[test]
    fn be_private_key_nonzero() {
        // Keys should not be all zeros
        let key = be_private_key(0);
        assert!(key.iter().any(|&b| b != 0));
    }

    #[test]
    fn be_private_key_correct_length() {
        let key = be_private_key(42);
        assert_eq!(key.len(), PRIVATE_KEY_BYTES);
    }

    #[test]
    fn keypair_valid_for_multiple_indices() {
        // Should not panic for various indices
        for i in 0..10 {
            let kp = keypair(i);
            // Public key should be derivable from secret key
            assert_eq!(kp.pk, kp.sk.public_key());
        }
    }

    #[test]
    fn keypair_deterministic() {
        let kp1 = keypair(5);
        let kp2 = keypair(5);
        assert_eq!(kp1.pk, kp2.pk);
    }

    #[test]
    fn keypair_different_indices_different_keys() {
        let kp0 = keypair(0);
        let kp1 = keypair(1);
        assert_ne!(kp0.pk, kp1.pk);
    }

    #[test]
    fn string_to_bytes_hex_with_prefix() {
        let bytes = string_to_bytes("0xdeadbeef").unwrap();
        assert_eq!(bytes, vec![0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn string_to_bytes_hex_without_prefix() {
        let bytes = string_to_bytes("deadbeef").unwrap();
        assert_eq!(bytes, vec![0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn string_to_bytes_empty() {
        let bytes = string_to_bytes("").unwrap();
        assert!(bytes.is_empty());
    }

    #[test]
    fn string_to_bytes_empty_after_prefix() {
        let bytes = string_to_bytes("0x").unwrap();
        assert!(bytes.is_empty());
    }

    #[test]
    fn string_to_bytes_invalid_hex() {
        assert!(string_to_bytes("0xZZZZ").is_err());
    }

    #[test]
    fn string_to_bytes_odd_length() {
        // Odd-length hex is invalid
        assert!(string_to_bytes("0xabc").is_err());
    }

    #[test]
    fn yaml_keypair_try_into_valid() {
        // Use index 0 to get a known keypair, then reconstruct via YamlKeypair
        let expected = keypair(0);
        let privkey_hex = hex::encode(be_private_key(0));
        let pubkey_hex = hex::encode(expected.pk.serialize());

        let yaml_kp = YamlKeypair {
            privkey: format!("0x{}", privkey_hex),
            pubkey: format!("0x{}", pubkey_hex),
        };
        let result: Result<Keypair, String> = yaml_kp.try_into();
        assert!(result.is_ok());
        let kp = result.unwrap();
        assert_eq!(kp.pk, expected.pk);
    }

    #[test]
    fn yaml_keypair_try_into_invalid_privkey() {
        let yaml_kp = YamlKeypair {
            privkey: "0x00".to_string(),
            pubkey: "0x".to_string(),
        };
        let result: Result<Keypair, String> = yaml_kp.try_into();
        assert!(result.is_err());
    }

    #[test]
    fn be_private_key_index_0_known_value() {
        // Verify index 0 produces a valid BLS secret key
        let key = be_private_key(0);
        let sk = SecretKey::deserialize(&key);
        assert!(sk.is_ok(), "Index 0 should produce a valid secret key");
    }

    #[test]
    fn be_private_key_large_index() {
        // Should work for large indices without panic
        let key = be_private_key(1_000_000);
        assert_eq!(key.len(), PRIVATE_KEY_BYTES);
        let sk = SecretKey::deserialize(&key);
        assert!(sk.is_ok(), "Large index should produce a valid secret key");
    }

    #[test]
    fn keypairs_from_yaml_file_missing_file() {
        let result = keypairs_from_yaml_file(PathBuf::from("/nonexistent/path.yaml"));
        assert!(result.is_err());
    }
}
