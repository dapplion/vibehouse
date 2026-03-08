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
            withdrawal_credentials: Hash256::from_slice(&[42; 32]),
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
}
