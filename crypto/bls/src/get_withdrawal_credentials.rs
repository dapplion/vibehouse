use crate::PublicKey;
use ethereum_hashing::hash_fixed;

/// Returns the withdrawal credentials for a given public key.
///
/// Used for submitting deposits to the Eth1 deposit contract.
pub fn get_withdrawal_credentials(pubkey: &PublicKey, prefix_byte: u8) -> Vec<u8> {
    let hashed = hash_fixed(&pubkey.serialize());
    let mut prefixed = vec![prefix_byte];
    prefixed.extend_from_slice(&hashed[1..]);

    prefixed
}
