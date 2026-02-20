use crate::{test_utils::TestRandom, *};
use derivative::Derivative;
use serde::{Deserialize, Deserializer, Serialize};
use ssz::{Decode, Encode};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

pub type Transaction<N> = VariableList<u8, N>;
pub type Transactions<E> = VariableList<
    Transaction<<E as EthSpec>::MaxBytesPerTransaction>,
    <E as EthSpec>::MaxTransactionsPerPayload,
>;

pub type Withdrawals<E> = VariableList<Withdrawal, <E as EthSpec>::MaxWithdrawalsPerPayload>;

#[superstruct(
    variants(Bellatrix, Capella, Deneb, Electra, Fulu, Gloas),
    variant_attributes(
        derive(
            Default,
            Debug,
            Clone,
            Serialize,
            Deserialize,
            Encode,
            Decode,
            TreeHash,
            TestRandom,
            Derivative,
        ),
        context_deserialize(ForkName),
        derivative(PartialEq, Hash(bound = "E: EthSpec")),
        serde(bound = "E: EthSpec", deny_unknown_fields),
        cfg_attr(
            feature = "arbitrary",
            derive(arbitrary::Arbitrary),
            arbitrary(bound = "E: EthSpec"),
        ),
    ),
    cast_error(ty = "Error", expr = "BeaconStateError::IncorrectStateVariant"),
    partial_getter_error(ty = "Error", expr = "BeaconStateError::IncorrectStateVariant"),
    map_into(FullPayload, BlindedPayload),
    map_ref_into(ExecutionPayloadHeader)
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary),
    arbitrary(bound = "E: EthSpec")
)]
#[derive(Debug, Clone, Serialize, Deserialize, Encode, TreeHash, Derivative)]
#[derivative(PartialEq, Hash(bound = "E: EthSpec"))]
#[serde(bound = "E: EthSpec", untagged)]
#[ssz(enum_behaviour = "transparent")]
#[tree_hash(enum_behaviour = "transparent")]
pub struct ExecutionPayload<E: EthSpec> {
    #[superstruct(getter(copy))]
    pub parent_hash: ExecutionBlockHash,
    #[superstruct(getter(copy))]
    #[serde(with = "serde_utils::address_hex")]
    pub fee_recipient: Address,
    #[superstruct(getter(copy))]
    pub state_root: Hash256,
    #[superstruct(getter(copy))]
    pub receipts_root: Hash256,
    #[serde(with = "ssz_types::serde_utils::hex_fixed_vec")]
    pub logs_bloom: FixedVector<u8, E::BytesPerLogsBloom>,
    #[superstruct(getter(copy))]
    pub prev_randao: Hash256,
    #[serde(with = "serde_utils::quoted_u64")]
    #[superstruct(getter(copy))]
    pub block_number: u64,
    #[serde(with = "serde_utils::quoted_u64")]
    #[superstruct(getter(copy))]
    pub gas_limit: u64,
    #[serde(with = "serde_utils::quoted_u64")]
    #[superstruct(getter(copy))]
    pub gas_used: u64,
    #[serde(with = "serde_utils::quoted_u64")]
    #[superstruct(getter(copy))]
    pub timestamp: u64,
    #[serde(with = "ssz_types::serde_utils::hex_var_list")]
    pub extra_data: VariableList<u8, E::MaxExtraDataBytes>,
    #[serde(with = "serde_utils::quoted_u256")]
    #[superstruct(getter(copy))]
    pub base_fee_per_gas: Uint256,
    #[superstruct(getter(copy))]
    pub block_hash: ExecutionBlockHash,
    #[serde(with = "ssz_types::serde_utils::list_of_hex_var_list")]
    pub transactions: Transactions<E>,
    #[superstruct(only(Capella, Deneb, Electra, Fulu, Gloas))]
    pub withdrawals: Withdrawals<E>,
    #[superstruct(only(Deneb, Electra, Fulu, Gloas), partial_getter(copy))]
    #[serde(with = "serde_utils::quoted_u64")]
    pub blob_gas_used: u64,
    #[superstruct(only(Deneb, Electra, Fulu, Gloas), partial_getter(copy))]
    #[serde(with = "serde_utils::quoted_u64")]
    pub excess_blob_gas: u64,
}

impl<'a, E: EthSpec> ExecutionPayloadRef<'a, E> {
    // this emulates clone on a normal reference type
    pub fn clone_from_ref(&self) -> ExecutionPayload<E> {
        map_execution_payload_ref!(&'a _, self, move |payload, cons| {
            cons(payload);
            payload.clone().into()
        })
    }
}

impl<E: EthSpec> ForkVersionDecode for ExecutionPayload<E> {
    /// SSZ decode with explicit fork variant.
    fn from_ssz_bytes_by_fork(bytes: &[u8], fork_name: ForkName) -> Result<Self, ssz::DecodeError> {
        match fork_name {
            ForkName::Base | ForkName::Altair => Err(ssz::DecodeError::BytesInvalid(format!(
                "unsupported fork for ExecutionPayload: {fork_name}",
            ))),
            ForkName::Bellatrix => {
                ExecutionPayloadBellatrix::from_ssz_bytes(bytes).map(Self::Bellatrix)
            }
            ForkName::Capella => ExecutionPayloadCapella::from_ssz_bytes(bytes).map(Self::Capella),
            ForkName::Deneb => ExecutionPayloadDeneb::from_ssz_bytes(bytes).map(Self::Deneb),
            ForkName::Electra => ExecutionPayloadElectra::from_ssz_bytes(bytes).map(Self::Electra),
            ForkName::Fulu => ExecutionPayloadFulu::from_ssz_bytes(bytes).map(Self::Fulu),
            ForkName::Gloas => ExecutionPayloadGloas::from_ssz_bytes(bytes).map(Self::Gloas),
        }
    }
}

impl<E: EthSpec> ExecutionPayload<E> {
    #[allow(clippy::arithmetic_side_effects)]
    /// Returns the maximum size of an execution payload.
    pub fn max_execution_payload_bellatrix_size() -> usize {
        // Fixed part
        ExecutionPayloadBellatrix::<E>::default().as_ssz_bytes().len()
            // Max size of variable length `extra_data` field
            + (E::max_extra_data_bytes() * <u8 as Encode>::ssz_fixed_len())
            // Max size of variable length `transactions` field
            + (E::max_transactions_per_payload() * (ssz::BYTES_PER_LENGTH_OFFSET + E::max_bytes_per_transaction()))
    }
}

impl<'de, E: EthSpec> ContextDeserialize<'de, ForkName> for ExecutionPayload<E> {
    fn context_deserialize<D>(deserializer: D, context: ForkName) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let convert_err = |e| {
            serde::de::Error::custom(format!("ExecutionPayload failed to deserialize: {:?}", e))
        };
        Ok(match context {
            ForkName::Base | ForkName::Altair => {
                return Err(serde::de::Error::custom(format!(
                    "ExecutionPayload failed to deserialize: unsupported fork '{}'",
                    context
                )));
            }
            ForkName::Bellatrix => {
                Self::Bellatrix(Deserialize::deserialize(deserializer).map_err(convert_err)?)
            }
            ForkName::Capella => {
                Self::Capella(Deserialize::deserialize(deserializer).map_err(convert_err)?)
            }
            ForkName::Deneb => {
                Self::Deneb(Deserialize::deserialize(deserializer).map_err(convert_err)?)
            }
            ForkName::Electra => {
                Self::Electra(Deserialize::deserialize(deserializer).map_err(convert_err)?)
            }
            ForkName::Fulu => {
                Self::Fulu(Deserialize::deserialize(deserializer).map_err(convert_err)?)
            }
            ForkName::Gloas => {
                Self::Gloas(Deserialize::deserialize(deserializer).map_err(convert_err)?)
            }
        })
    }
}

impl<E: EthSpec> ExecutionPayload<E> {
    pub fn fork_name(&self) -> ForkName {
        match self {
            ExecutionPayload::Bellatrix(_) => ForkName::Bellatrix,
            ExecutionPayload::Capella(_) => ForkName::Capella,
            ExecutionPayload::Deneb(_) => ForkName::Deneb,
            ExecutionPayload::Electra(_) => ForkName::Electra,
            ExecutionPayload::Fulu(_) => ForkName::Fulu,
            ExecutionPayload::Gloas(_) => ForkName::Gloas,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MainnetEthSpec;

    type E = MainnetEthSpec;

    fn make_gloas_payload() -> ExecutionPayloadGloas<E> {
        ExecutionPayloadGloas {
            parent_hash: ExecutionBlockHash::from(Hash256::repeat_byte(0x11)),
            fee_recipient: Address::repeat_byte(0x12),
            state_root: Hash256::repeat_byte(0x13),
            receipts_root: Hash256::repeat_byte(0x14),
            logs_bloom: FixedVector::from(vec![0x15; 256]),
            prev_randao: Hash256::repeat_byte(0x16),
            block_number: 100,
            gas_limit: 60_000_000,
            gas_used: 30_000_000,
            timestamp: 1_800_000_000,
            extra_data: VariableList::from(vec![0xCC, 0xDD]),
            base_fee_per_gas: Uint256::from(2_000_000_000u64),
            block_hash: ExecutionBlockHash::from(Hash256::repeat_byte(0x17)),
            transactions: <_>::default(),
            withdrawals: <_>::default(),
            blob_gas_used: 262_144,
            excess_blob_gas: 131_072,
        }
    }

    // ── fork_name ──

    #[test]
    fn fork_name_gloas() {
        let payload = ExecutionPayload::<E>::Gloas(make_gloas_payload());
        assert_eq!(payload.fork_name(), ForkName::Gloas);
    }

    #[test]
    fn fork_name_fulu() {
        let payload = ExecutionPayload::<E>::Fulu(ExecutionPayloadFulu::default());
        assert_eq!(payload.fork_name(), ForkName::Fulu);
    }

    // ── SSZ roundtrip (from_ssz_bytes_by_fork) ──

    #[test]
    fn ssz_roundtrip_gloas_payload() {
        let original = make_gloas_payload();
        let bytes = original.as_ssz_bytes();
        let decoded =
            ExecutionPayloadGloas::<E>::from_ssz_bytes(&bytes).expect("SSZ decode should succeed");
        assert_eq!(decoded, original);
    }

    #[test]
    fn ssz_roundtrip_via_fork_dispatch_gloas() {
        let inner = make_gloas_payload();
        let wrapped = ExecutionPayload::<E>::Gloas(inner.clone());
        let bytes = wrapped.as_ssz_bytes();
        let decoded = ExecutionPayload::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Gloas)
            .expect("SSZ decode should succeed");
        assert_eq!(decoded, wrapped);
    }

    #[test]
    fn ssz_decode_base_fork_fails() {
        let bytes = [0u8; 32];
        assert!(ExecutionPayload::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Base).is_err());
    }

    #[test]
    fn ssz_decode_altair_fork_fails() {
        let bytes = [0u8; 32];
        assert!(ExecutionPayload::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Altair).is_err());
    }

    #[test]
    fn ssz_fork_dispatch_produces_correct_variant() {
        let gloas = make_gloas_payload();
        let bytes = gloas.as_ssz_bytes();

        let as_gloas = ExecutionPayload::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Gloas)
            .expect("decode as Gloas");
        let as_fulu = ExecutionPayload::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Fulu)
            .expect("decode as Fulu");

        assert_eq!(as_gloas.fork_name(), ForkName::Gloas);
        assert_eq!(as_fulu.fork_name(), ForkName::Fulu);
    }

    // ── clone_from_ref ──

    #[test]
    fn clone_from_ref_gloas() {
        let payload = ExecutionPayload::<E>::Gloas(make_gloas_payload());
        let cloned = payload.to_ref().clone_from_ref();
        assert_eq!(cloned, payload);
    }

    // ── Field accessors through the enum ──

    #[test]
    fn enum_field_accessors_gloas() {
        let inner = make_gloas_payload();
        let payload = ExecutionPayload::<E>::Gloas(inner.clone());

        assert_eq!(payload.parent_hash(), inner.parent_hash);
        assert_eq!(payload.fee_recipient(), inner.fee_recipient);
        assert_eq!(payload.block_hash(), inner.block_hash);
        assert_eq!(payload.block_number(), inner.block_number);
        assert_eq!(payload.gas_limit(), inner.gas_limit);
        assert_eq!(payload.gas_used(), inner.gas_used);
        assert_eq!(payload.timestamp(), inner.timestamp);
        assert_eq!(payload.prev_randao(), inner.prev_randao);
        assert_eq!(payload.base_fee_per_gas(), inner.base_fee_per_gas);
        assert_eq!(payload.blob_gas_used().unwrap(), inner.blob_gas_used);
        assert_eq!(payload.excess_blob_gas().unwrap(), inner.excess_blob_gas);
    }

    // ── Default ──

    #[test]
    fn default_gloas_payload_has_zero_fields() {
        let payload = ExecutionPayloadGloas::<E>::default();
        assert_eq!(payload.parent_hash, ExecutionBlockHash::zero());
        assert_eq!(payload.block_number, 0);
        assert_eq!(payload.gas_limit, 0);
        assert_eq!(payload.blob_gas_used, 0);
        assert_eq!(payload.excess_blob_gas, 0);
    }
}
