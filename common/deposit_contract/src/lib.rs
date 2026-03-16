use alloy_sol_types::{SolCall, sol};
use ssz::{Decode, DecodeError as SszDecodeError, Encode};
use tree_hash::TreeHash;
use types::{DepositData, Hash256, PublicKeyBytes, SignatureBytes};

sol! {
    function deposit(bytes pubkey, bytes withdrawal_credentials, bytes signature, bytes32 deposit_data_root);
}

pub const CONTRACT_DEPLOY_GAS: usize = 4_000_000;
pub const DEPOSIT_GAS: usize = 400_000;
pub const ABI: &[u8] = include_bytes!("../contracts/v0.12.1_validator_registration.json");
pub const BYTECODE: &[u8] = include_bytes!("../contracts/v0.12.1_validator_registration.bytecode");
pub const DEPOSIT_DATA_LEN: usize = 420; // lol

pub mod testnet {
    pub const ABI: &[u8] =
        include_bytes!("../contracts/v0.12.1_testnet_validator_registration.json");
    pub const BYTECODE: &[u8] =
        include_bytes!("../contracts/v0.12.1_testnet_validator_registration.bytecode");
}

#[derive(Debug)]
pub enum Error {
    AlloyError(alloy_sol_types::Error),
    SszDecodeError(SszDecodeError),
    MissingField,
    InadequateBytes,
}

impl From<alloy_sol_types::Error> for Error {
    fn from(e: alloy_sol_types::Error) -> Error {
        Error::AlloyError(e)
    }
}

pub fn encode_eth1_tx_data(deposit_data: &DepositData) -> Result<Vec<u8>, Error> {
    let call = depositCall {
        pubkey: deposit_data.pubkey.as_ssz_bytes().into(),
        withdrawal_credentials: deposit_data.withdrawal_credentials.as_ssz_bytes().into(),
        signature: deposit_data.signature.as_ssz_bytes().into(),
        deposit_data_root: deposit_data.tree_hash_root().0.into(),
    };
    Ok(call.abi_encode())
}

pub fn decode_eth1_tx_data(bytes: &[u8], amount: u64) -> Result<(DepositData, Hash256), Error> {
    let call = depositCall::abi_decode_raw(bytes.get(4..).ok_or(Error::InadequateBytes)?)
        .map_err(Error::AlloyError)?;

    let root = Hash256::from_slice(call.deposit_data_root.as_slice());

    let deposit_data = DepositData {
        amount,
        signature: SignatureBytes::from_ssz_bytes(&call.signature)
            .map_err(Error::SszDecodeError)?,
        withdrawal_credentials: Hash256::from_ssz_bytes(&call.withdrawal_credentials)
            .map_err(Error::SszDecodeError)?,
        pubkey: PublicKeyBytes::from_ssz_bytes(&call.pubkey).map_err(Error::SszDecodeError)?,
    };

    Ok((deposit_data, root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{
        ChainSpec, EthSpec, Keypair, MinimalEthSpec, Signature,
        test_utils::generate_deterministic_keypair,
    };

    type E = MinimalEthSpec;

    fn get_deposit(keypair: Keypair, spec: &ChainSpec) -> DepositData {
        let mut deposit_data = DepositData {
            pubkey: keypair.pk.into(),
            withdrawal_credentials: Hash256::from([42; 32]),
            amount: u64::MAX,
            signature: Signature::empty().into(),
        };
        deposit_data.signature = deposit_data.create_signature(&keypair.sk, spec);
        deposit_data
    }

    #[test]
    fn round_trip() {
        let spec = &E::default_spec();

        let keypair = generate_deterministic_keypair(42);
        let original = get_deposit(keypair, spec);

        let data = encode_eth1_tx_data(&original).expect("should produce tx data");

        assert_eq!(
            data.len(),
            DEPOSIT_DATA_LEN,
            "bytes should be correct length"
        );

        let (decoded, root) = decode_eth1_tx_data(&data, original.amount).expect("should decode");

        assert_eq!(decoded, original, "decoded should match original");
        assert_eq!(
            root,
            original.tree_hash_root(),
            "decode root should match original root"
        );
    }

    #[test]
    fn round_trip_multiple_keypairs() {
        let spec = &E::default_spec();
        for i in 0..5 {
            let keypair = generate_deterministic_keypair(i);
            let original = get_deposit(keypair, spec);
            let data = encode_eth1_tx_data(&original).unwrap();
            let (decoded, root) = decode_eth1_tx_data(&data, original.amount).unwrap();
            assert_eq!(decoded, original, "keypair {i} round-trip failed");
            assert_eq!(root, original.tree_hash_root());
        }
    }

    #[test]
    fn round_trip_zero_amount() {
        let spec = &E::default_spec();
        let keypair = generate_deterministic_keypair(0);
        let mut deposit_data = DepositData {
            pubkey: keypair.pk.into(),
            withdrawal_credentials: Hash256::from([0; 32]),
            amount: 0,
            signature: Signature::empty().into(),
        };
        deposit_data.signature = deposit_data.create_signature(&keypair.sk, spec);

        let data = encode_eth1_tx_data(&deposit_data).unwrap();
        let (decoded, _root) = decode_eth1_tx_data(&data, 0).unwrap();
        assert_eq!(decoded, deposit_data);
    }

    #[test]
    fn round_trip_standard_deposit_amount() {
        let spec = &E::default_spec();
        let keypair = generate_deterministic_keypair(10);
        let mut deposit_data = DepositData {
            pubkey: keypair.pk.into(),
            withdrawal_credentials: Hash256::from([0xab; 32]),
            amount: 32_000_000_000, // 32 ETH in gwei
            signature: Signature::empty().into(),
        };
        deposit_data.signature = deposit_data.create_signature(&keypair.sk, spec);

        let data = encode_eth1_tx_data(&deposit_data).unwrap();
        assert_eq!(data.len(), DEPOSIT_DATA_LEN);
        let (decoded, root) = decode_eth1_tx_data(&data, 32_000_000_000).unwrap();
        assert_eq!(decoded, deposit_data);
        assert_eq!(root, deposit_data.tree_hash_root());
    }

    #[test]
    fn decode_empty_bytes_fails() {
        let result = decode_eth1_tx_data(&[], 0);
        assert!(result.is_err());
    }

    #[test]
    fn decode_too_short_bytes_fails() {
        let result = decode_eth1_tx_data(&[0; 3], 0);
        assert!(result.is_err());
    }

    #[test]
    fn decode_four_zero_bytes_fails() {
        // Just the function selector, no data
        let result = decode_eth1_tx_data(&[0; 4], 0);
        assert!(result.is_err());
    }

    #[test]
    fn decode_garbage_fails() {
        let garbage = vec![0xde, 0xad, 0xbe, 0xef, 0x00, 0x01, 0x02, 0x03];
        let result = decode_eth1_tx_data(&garbage, 100);
        assert!(result.is_err());
    }

    #[test]
    fn encode_produces_consistent_length() {
        let spec = &E::default_spec();
        for i in 0..3 {
            let keypair = generate_deterministic_keypair(i);
            let deposit = get_deposit(keypair, spec);
            let data = encode_eth1_tx_data(&deposit).unwrap();
            assert_eq!(
                data.len(),
                DEPOSIT_DATA_LEN,
                "keypair {i} produced wrong length"
            );
        }
    }

    #[test]
    fn different_amounts_produce_different_roots() {
        let spec = &E::default_spec();
        let keypair = generate_deterministic_keypair(0);

        let mut deposit_a = DepositData {
            pubkey: keypair.pk.clone().into(),
            withdrawal_credentials: Hash256::from([42; 32]),
            amount: 100,
            signature: Signature::empty().into(),
        };
        deposit_a.signature = deposit_a.create_signature(&keypair.sk, spec);

        let mut deposit_b = DepositData {
            pubkey: keypair.pk.into(),
            withdrawal_credentials: Hash256::from([42; 32]),
            amount: 200,
            signature: Signature::empty().into(),
        };
        deposit_b.signature = deposit_b.create_signature(&keypair.sk, spec);

        let data_a = encode_eth1_tx_data(&deposit_a).unwrap();
        let data_b = encode_eth1_tx_data(&deposit_b).unwrap();

        // Different amounts should produce different encoded data
        assert_ne!(data_a, data_b);

        let (_, root_a) = decode_eth1_tx_data(&data_a, 100).unwrap();
        let (_, root_b) = decode_eth1_tx_data(&data_b, 200).unwrap();
        assert_ne!(root_a, root_b);
    }

    #[test]
    fn decode_with_wrong_amount_gives_mismatched_root() {
        let spec = &E::default_spec();
        let keypair = generate_deterministic_keypair(0);
        let original = get_deposit(keypair, spec);
        let data = encode_eth1_tx_data(&original).unwrap();

        // Decode with a different amount
        let (decoded, root) = decode_eth1_tx_data(&data, 999).unwrap();
        // The decoded amount should be the one we passed in, not the original
        assert_eq!(decoded.amount, 999);
        // The root should NOT match the tree hash root of the decoded data
        // (because the root was computed from the original amount)
        assert_ne!(root, decoded.tree_hash_root());
    }

    #[test]
    fn deposit_data_len_is_expected() {
        // ABI-encoded deposit call should be 420 bytes
        assert_eq!(DEPOSIT_DATA_LEN, 420);
    }

    #[test]
    fn contract_deploy_gas_is_reasonable() {
        let deploy_gas = CONTRACT_DEPLOY_GAS;
        assert!(deploy_gas > 0);
        assert!(deploy_gas <= 10_000_000);
    }

    #[test]
    fn deposit_gas_is_reasonable() {
        let gas = DEPOSIT_GAS;
        assert!(gas > 0);
        assert!(gas <= 1_000_000);
    }

    #[test]
    fn abi_and_bytecode_are_not_empty() {
        assert!(!ABI.is_empty());
        assert!(!BYTECODE.is_empty());
        assert!(!testnet::ABI.is_empty());
        assert!(!testnet::BYTECODE.is_empty());
    }
}
