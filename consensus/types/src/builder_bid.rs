use crate::beacon_block_body::KzgCommitments;
use crate::{
    ChainSpec, ContextDeserialize, EthSpec, ExecutionPayloadHeaderBellatrix,
    ExecutionPayloadHeaderCapella, ExecutionPayloadHeaderDeneb, ExecutionPayloadHeaderElectra,
    ExecutionPayloadHeaderFulu, ExecutionPayloadHeaderGloas, ExecutionPayloadHeaderRef,
    ExecutionPayloadHeaderRefMut, ExecutionRequests, ForkName, ForkVersionDecode, SignedRoot,
    Uint256, test_utils::TestRandom,
};
use bls::PublicKeyBytes;
use bls::Signature;
use serde::{Deserialize, Deserializer, Serialize};
use ssz::Decode;
use ssz_derive::{Decode, Encode};
use superstruct::superstruct;
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

#[superstruct(
    variants(Bellatrix, Capella, Deneb, Electra, Fulu, Gloas),
    variant_attributes(
        derive(
            PartialEq,
            Debug,
            Encode,
            Serialize,
            Deserialize,
            TreeHash,
            Decode,
            Clone,
            TestRandom
        ),
        serde(bound = "E: EthSpec", deny_unknown_fields)
    ),
    map_ref_into(ExecutionPayloadHeaderRef),
    map_ref_mut_into(ExecutionPayloadHeaderRefMut)
)]
#[derive(PartialEq, Debug, Encode, Serialize, Deserialize, TreeHash, Clone)]
#[serde(bound = "E: EthSpec", deny_unknown_fields, untagged)]
#[ssz(enum_behaviour = "transparent")]
#[tree_hash(enum_behaviour = "transparent")]
pub struct BuilderBid<E: EthSpec> {
    #[superstruct(only(Bellatrix), partial_getter(rename = "header_bellatrix"))]
    pub header: ExecutionPayloadHeaderBellatrix<E>,
    #[superstruct(only(Capella), partial_getter(rename = "header_capella"))]
    pub header: ExecutionPayloadHeaderCapella<E>,
    #[superstruct(only(Deneb), partial_getter(rename = "header_deneb"))]
    pub header: ExecutionPayloadHeaderDeneb<E>,
    #[superstruct(only(Electra), partial_getter(rename = "header_electra"))]
    pub header: ExecutionPayloadHeaderElectra<E>,
    #[superstruct(only(Fulu), partial_getter(rename = "header_fulu"))]
    pub header: ExecutionPayloadHeaderFulu<E>,
    #[superstruct(only(Gloas), partial_getter(rename = "header_gloas"))]
    pub header: ExecutionPayloadHeaderGloas<E>,
    #[superstruct(only(Deneb, Electra, Fulu, Gloas))]
    pub blob_kzg_commitments: KzgCommitments<E>,
    #[superstruct(only(Electra, Fulu, Gloas))]
    pub execution_requests: ExecutionRequests<E>,
    #[serde(with = "serde_utils::quoted_u256")]
    pub value: Uint256,
    pub pubkey: PublicKeyBytes,
}

impl<E: EthSpec> BuilderBid<E> {
    pub fn header(&self) -> ExecutionPayloadHeaderRef<'_, E> {
        self.to_ref().header()
    }
}

impl<'a, E: EthSpec> BuilderBidRef<'a, E> {
    pub fn header(&self) -> ExecutionPayloadHeaderRef<'a, E> {
        map_builder_bid_ref_into_execution_payload_header_ref!(&'a _, self, |bid, cons| cons(
            &bid.header
        ))
    }
}

impl<'a, E: EthSpec> BuilderBidRefMut<'a, E> {
    pub fn header_mut(self) -> ExecutionPayloadHeaderRefMut<'a, E> {
        map_builder_bid_ref_mut_into_execution_payload_header_ref_mut!(&'a _, self, |bid, cons| {
            cons(&mut bid.header)
        })
    }
}

impl<E: EthSpec> ForkVersionDecode for BuilderBid<E> {
    /// SSZ decode with explicit fork variant.
    fn from_ssz_bytes_by_fork(bytes: &[u8], fork_name: ForkName) -> Result<Self, ssz::DecodeError> {
        let builder_bid = match fork_name {
            ForkName::Altair | ForkName::Base => {
                return Err(ssz::DecodeError::BytesInvalid(format!(
                    "unsupported fork for ExecutionPayloadHeader: {fork_name}",
                )));
            }
            ForkName::Bellatrix => {
                BuilderBid::Bellatrix(BuilderBidBellatrix::from_ssz_bytes(bytes)?)
            }
            ForkName::Capella => BuilderBid::Capella(BuilderBidCapella::from_ssz_bytes(bytes)?),
            ForkName::Deneb => BuilderBid::Deneb(BuilderBidDeneb::from_ssz_bytes(bytes)?),
            ForkName::Electra => BuilderBid::Electra(BuilderBidElectra::from_ssz_bytes(bytes)?),
            ForkName::Fulu => BuilderBid::Fulu(BuilderBidFulu::from_ssz_bytes(bytes)?),
            ForkName::Gloas => BuilderBid::Gloas(BuilderBidGloas::from_ssz_bytes(bytes)?),
        };
        Ok(builder_bid)
    }
}

impl<E: EthSpec> SignedRoot for BuilderBid<E> {}

/// Validator registration, for use in interacting with servers implementing the builder API.
#[derive(PartialEq, Debug, Encode, Serialize, Deserialize, Clone)]
#[serde(bound = "E: EthSpec")]
pub struct SignedBuilderBid<E: EthSpec> {
    pub message: BuilderBid<E>,
    pub signature: Signature,
}

impl<E: EthSpec> ForkVersionDecode for SignedBuilderBid<E> {
    /// SSZ decode with explicit fork variant.
    fn from_ssz_bytes_by_fork(bytes: &[u8], fork_name: ForkName) -> Result<Self, ssz::DecodeError> {
        let mut builder = ssz::SszDecoderBuilder::new(bytes);

        builder.register_anonymous_variable_length_item()?;
        builder.register_type::<Signature>()?;

        let mut decoder = builder.build()?;
        let message = decoder
            .decode_next_with(|bytes| BuilderBid::from_ssz_bytes_by_fork(bytes, fork_name))?;
        let signature = decoder.decode_next()?;

        Ok(Self { message, signature })
    }
}

impl<'de, E: EthSpec> ContextDeserialize<'de, ForkName> for BuilderBid<E> {
    fn context_deserialize<D>(deserializer: D, context: ForkName) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let convert_err =
            |e| serde::de::Error::custom(format!("BuilderBid failed to deserialize: {:?}", e));
        Ok(match context {
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
            ForkName::Base | ForkName::Altair => {
                return Err(serde::de::Error::custom(format!(
                    "BuilderBid failed to deserialize: unsupported fork '{}'",
                    context
                )));
            }
        })
    }
}

impl<'de, E: EthSpec> ContextDeserialize<'de, ForkName> for SignedBuilderBid<E> {
    fn context_deserialize<D>(deserializer: D, context: ForkName) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper {
            message: serde_json::Value,
            signature: Signature,
        }

        let helper = Helper::deserialize(deserializer)?;

        // Deserialize `data` using ContextDeserialize
        let message = BuilderBid::<E>::context_deserialize(helper.message, context)
            .map_err(serde::de::Error::custom)?;

        Ok(SignedBuilderBid {
            message,
            signature: helper.signature,
        })
    }
}

impl<E: EthSpec> SignedBuilderBid<E> {
    pub fn verify_signature(&self, spec: &ChainSpec) -> bool {
        self.message
            .pubkey()
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
    use crate::{
        Address, ExecutionBlockHash, FixedVector, Hash256, MainnetEthSpec, Uint256, VariableList,
    };
    use ssz::Encode;
    use tree_hash::TreeHash;

    type E = MainnetEthSpec;

    /// Helper: build a non-default Gloas BuilderBid with distinct field values.
    fn make_gloas_bid() -> BuilderBidGloas<E> {
        BuilderBidGloas {
            header: ExecutionPayloadHeaderGloas {
                parent_hash: ExecutionBlockHash::from(Hash256::repeat_byte(0x01)),
                fee_recipient: Address::repeat_byte(0x02),
                state_root: Hash256::repeat_byte(0x03),
                receipts_root: Hash256::repeat_byte(0x04),
                logs_bloom: FixedVector::from(vec![0x05; 256]),
                prev_randao: Hash256::repeat_byte(0x06),
                block_number: 42,
                gas_limit: 30_000_000,
                gas_used: 15_000_000,
                timestamp: 1_700_000_000,
                extra_data: VariableList::from(vec![0xAA, 0xBB]),
                base_fee_per_gas: Uint256::from(1_000_000_000u64),
                block_hash: ExecutionBlockHash::from(Hash256::repeat_byte(0x07)),
                transactions_root: Hash256::repeat_byte(0x08),
                withdrawals_root: Hash256::repeat_byte(0x09),
                blob_gas_used: 131_072,
                excess_blob_gas: 65_536,
            },
            blob_kzg_commitments: <_>::default(),
            execution_requests: ExecutionRequests::default(),
            value: Uint256::from(1_000_000_000u64),
            pubkey: PublicKeyBytes::empty(),
        }
    }

    /// Helper: build a non-default Fulu BuilderBid.
    fn make_fulu_bid() -> BuilderBidFulu<E> {
        BuilderBidFulu {
            header: ExecutionPayloadHeaderFulu {
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
                transactions_root: Hash256::repeat_byte(0x18),
                withdrawals_root: Hash256::repeat_byte(0x19),
                blob_gas_used: 262_144,
                excess_blob_gas: 131_072,
            },
            blob_kzg_commitments: <_>::default(),
            execution_requests: ExecutionRequests::default(),
            value: Uint256::from(2_000_000_000u64),
            pubkey: PublicKeyBytes::empty(),
        }
    }

    /// Helper: build a Bellatrix BuilderBid (minimal fields).
    fn make_bellatrix_bid() -> BuilderBidBellatrix<E> {
        BuilderBidBellatrix {
            header: ExecutionPayloadHeaderBellatrix {
                parent_hash: ExecutionBlockHash::from(Hash256::repeat_byte(0x21)),
                fee_recipient: Address::repeat_byte(0x22),
                state_root: Hash256::repeat_byte(0x23),
                receipts_root: Hash256::repeat_byte(0x24),
                logs_bloom: FixedVector::from(vec![0x25; 256]),
                prev_randao: Hash256::repeat_byte(0x26),
                block_number: 1,
                gas_limit: 15_000_000,
                gas_used: 7_000_000,
                timestamp: 1_600_000_000,
                extra_data: VariableList::from(vec![0xFF]),
                base_fee_per_gas: Uint256::from(500_000_000u64),
                block_hash: ExecutionBlockHash::from(Hash256::repeat_byte(0x27)),
                transactions_root: Hash256::repeat_byte(0x28),
            },
            value: Uint256::from(500_000_000u64),
            pubkey: PublicKeyBytes::empty(),
        }
    }

    // ── header() accessor ──

    #[test]
    fn header_accessor_gloas() {
        let bid = BuilderBid::<E>::Gloas(make_gloas_bid());
        let header = bid.header();
        assert_eq!(header.block_hash(), make_gloas_bid().header.block_hash);
        assert_eq!(header.block_number(), make_gloas_bid().header.block_number);
        assert_eq!(header.gas_limit(), make_gloas_bid().header.gas_limit);
    }

    #[test]
    fn header_accessor_fulu() {
        let bid = BuilderBid::<E>::Fulu(make_fulu_bid());
        let header = bid.header();
        assert_eq!(header.block_hash(), make_fulu_bid().header.block_hash);
        assert_eq!(header.block_number(), make_fulu_bid().header.block_number);
    }

    #[test]
    fn header_accessor_bellatrix() {
        let bid = BuilderBid::<E>::Bellatrix(make_bellatrix_bid());
        let header = bid.header();
        assert_eq!(header.block_hash(), make_bellatrix_bid().header.block_hash);
    }

    // ── header_mut() accessor ──

    #[test]
    fn header_mut_accessor_gloas() {
        let mut bid = make_gloas_bid();
        let new_hash = ExecutionBlockHash::from(Hash256::repeat_byte(0xFF));
        {
            let wrapped = BuilderBid::<E>::Gloas(bid.clone());
            // Verify initial value
            assert_eq!(wrapped.header().block_hash(), bid.header.block_hash);
        }
        bid.header.block_hash = new_hash;
        let wrapped = BuilderBid::<E>::Gloas(bid);
        assert_eq!(wrapped.header().block_hash(), new_hash);
    }

    // ── Common field accessors ──

    #[test]
    fn value_accessor_gloas() {
        let bid = BuilderBid::<E>::Gloas(make_gloas_bid());
        assert_eq!(*bid.value(), Uint256::from(1_000_000_000u64));
    }

    #[test]
    fn pubkey_accessor_gloas() {
        let bid = BuilderBid::<E>::Gloas(make_gloas_bid());
        assert_eq!(*bid.pubkey(), PublicKeyBytes::empty());
    }

    // ── Gloas-specific partial getters ──

    #[test]
    fn blob_kzg_commitments_gloas_accessible() {
        let bid = BuilderBid::<E>::Gloas(make_gloas_bid());
        assert!(bid.blob_kzg_commitments().is_ok());
        assert!(bid.blob_kzg_commitments().unwrap().is_empty());
    }

    #[test]
    fn execution_requests_gloas_accessible() {
        let bid = BuilderBid::<E>::Gloas(make_gloas_bid());
        assert!(bid.execution_requests().is_ok());
    }

    #[test]
    fn blob_kzg_commitments_bellatrix_inaccessible() {
        let bid = BuilderBid::<E>::Bellatrix(make_bellatrix_bid());
        assert!(bid.blob_kzg_commitments().is_err());
    }

    #[test]
    fn execution_requests_bellatrix_inaccessible() {
        let bid = BuilderBid::<E>::Bellatrix(make_bellatrix_bid());
        assert!(bid.execution_requests().is_err());
    }

    // ── SSZ roundtrip (BuilderBidGloas inner type) ──

    #[test]
    fn ssz_roundtrip_gloas_bid() {
        let original = make_gloas_bid();
        let bytes = original.as_ssz_bytes();
        let decoded =
            BuilderBidGloas::<E>::from_ssz_bytes(&bytes).expect("SSZ decode should succeed");
        assert_eq!(decoded, original);
    }

    #[test]
    fn ssz_roundtrip_fulu_bid() {
        let original = make_fulu_bid();
        let bytes = original.as_ssz_bytes();
        let decoded =
            BuilderBidFulu::<E>::from_ssz_bytes(&bytes).expect("SSZ decode should succeed");
        assert_eq!(decoded, original);
    }

    // ── from_ssz_bytes_by_fork ──

    #[test]
    fn ssz_fork_dispatch_gloas() {
        let inner = make_gloas_bid();
        let wrapped = BuilderBid::<E>::Gloas(inner.clone());
        let bytes = wrapped.as_ssz_bytes();
        let decoded = BuilderBid::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Gloas)
            .expect("decode as Gloas");
        assert_eq!(decoded, wrapped);
    }

    #[test]
    fn ssz_fork_dispatch_fulu() {
        let inner = make_fulu_bid();
        let wrapped = BuilderBid::<E>::Fulu(inner.clone());
        let bytes = wrapped.as_ssz_bytes();
        let decoded = BuilderBid::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Fulu)
            .expect("decode as Fulu");
        assert_eq!(decoded, wrapped);
    }

    #[test]
    fn ssz_fork_dispatch_bellatrix() {
        let inner = make_bellatrix_bid();
        let wrapped = BuilderBid::<E>::Bellatrix(inner.clone());
        let bytes = wrapped.as_ssz_bytes();
        let decoded = BuilderBid::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Bellatrix)
            .expect("decode as Bellatrix");
        assert_eq!(decoded, wrapped);
    }

    #[test]
    fn ssz_fork_dispatch_base_fails() {
        let bytes = [0u8; 32];
        assert!(BuilderBid::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Base).is_err());
    }

    #[test]
    fn ssz_fork_dispatch_altair_fails() {
        let bytes = [0u8; 32];
        assert!(BuilderBid::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Altair).is_err());
    }

    #[test]
    fn ssz_fork_dispatch_produces_correct_variant() {
        // Gloas and Fulu bids have the same SSZ layout, so cross-decoding should
        // produce different enum variants with the same data.
        let gloas = make_gloas_bid();
        let bytes = gloas.as_ssz_bytes();

        let as_gloas = BuilderBid::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Gloas)
            .expect("decode as Gloas");
        let as_fulu = BuilderBid::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Fulu)
            .expect("decode as Fulu");

        // Both decode successfully but produce different enum variants
        assert!(matches!(as_gloas, BuilderBid::Gloas(_)));
        assert!(matches!(as_fulu, BuilderBid::Fulu(_)));
    }

    // ── SignedBuilderBid SSZ roundtrip ──

    #[test]
    fn signed_builder_bid_ssz_roundtrip_gloas() {
        let bid = BuilderBid::<E>::Gloas(make_gloas_bid());
        let signed = SignedBuilderBid {
            message: bid,
            signature: Signature::empty(),
        };
        let bytes = signed.as_ssz_bytes();
        let decoded = SignedBuilderBid::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Gloas)
            .expect("SSZ decode should succeed");
        assert_eq!(decoded, signed);
    }

    #[test]
    fn signed_builder_bid_ssz_roundtrip_fulu() {
        let bid = BuilderBid::<E>::Fulu(make_fulu_bid());
        let signed = SignedBuilderBid {
            message: bid,
            signature: Signature::empty(),
        };
        let bytes = signed.as_ssz_bytes();
        let decoded = SignedBuilderBid::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Fulu)
            .expect("SSZ decode should succeed");
        assert_eq!(decoded, signed);
    }

    #[test]
    fn signed_builder_bid_ssz_base_fails() {
        let bid = BuilderBid::<E>::Gloas(make_gloas_bid());
        let signed = SignedBuilderBid {
            message: bid,
            signature: Signature::empty(),
        };
        let bytes = signed.as_ssz_bytes();
        assert!(SignedBuilderBid::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Base).is_err());
    }

    // ── verify_signature ──

    #[test]
    fn verify_signature_empty_pubkey_fails() {
        let bid = BuilderBid::<E>::Gloas(make_gloas_bid());
        let signed = SignedBuilderBid {
            message: bid,
            signature: Signature::empty(),
        };
        let spec = ChainSpec::mainnet();
        // Empty pubkey cannot be decompressed, so signature verification should fail
        assert!(!signed.verify_signature(&spec));
    }

    #[test]
    fn verify_signature_valid_keypair() {
        use bls::{Keypair, SecretKey};

        let sk = SecretKey::deserialize(&[
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67,
            0x89, 0xab, 0xcd, 0xef,
        ])
        .expect("valid secret key");
        let kp = Keypair::from_components(sk.public_key(), sk.clone());

        let spec = ChainSpec::mainnet();
        let domain = spec.get_builder_domain();

        let mut bid = make_gloas_bid();
        bid.pubkey = kp.pk.compress();
        let message_bid = BuilderBid::<E>::Gloas(bid);
        let signing_root = message_bid.signing_root(domain);
        let signature = sk.sign(signing_root);

        let signed = SignedBuilderBid {
            message: message_bid,
            signature,
        };
        assert!(signed.verify_signature(&spec));
    }

    #[test]
    fn verify_signature_wrong_key_fails() {
        use bls::{Keypair, SecretKey};

        let sk1 = SecretKey::deserialize(&[
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67,
            0x89, 0xab, 0xcd, 0xef,
        ])
        .expect("valid secret key");
        let sk2 = SecretKey::deserialize(&[
            0x02, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x02, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef, 0x02, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x02, 0x23, 0x45, 0x67,
            0x89, 0xab, 0xcd, 0xef,
        ])
        .expect("valid secret key");
        let kp1 = Keypair::from_components(sk1.public_key(), sk1.clone());

        let spec = ChainSpec::mainnet();
        let domain = spec.get_builder_domain();

        let mut bid = make_gloas_bid();
        bid.pubkey = kp1.pk.compress(); // pubkey from key 1
        let message_bid = BuilderBid::<E>::Gloas(bid);
        let signing_root = message_bid.signing_root(domain);
        let signature = sk2.sign(signing_root); // sign with key 2

        let signed = SignedBuilderBid {
            message: message_bid,
            signature,
        };
        // Signature from wrong key should fail verification
        assert!(!signed.verify_signature(&spec));
    }

    // ── Tree hash ──

    #[test]
    fn tree_hash_differs_for_different_values() {
        let a = make_gloas_bid();
        let mut b = make_gloas_bid();
        b.value = Uint256::from(999u64);
        assert_ne!(a.tree_hash_root(), b.tree_hash_root());
    }

    #[test]
    fn tree_hash_same_for_equal_values() {
        let a = make_gloas_bid();
        let b = make_gloas_bid();
        assert_eq!(a.tree_hash_root(), b.tree_hash_root());
    }

    // ── Clone + equality ──

    #[test]
    fn clone_preserves_equality_gloas() {
        let original = BuilderBid::<E>::Gloas(make_gloas_bid());
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn different_variants_not_equal() {
        // Even with identical field values, Gloas and Fulu enum variants should never be equal.
        let mut gloas_inner = make_gloas_bid();
        gloas_inner.value = Uint256::from(42u64);
        let mut fulu_inner = make_fulu_bid();
        fulu_inner.value = Uint256::from(42u64);
        let gloas = BuilderBid::<E>::Gloas(gloas_inner);
        let fulu = BuilderBid::<E>::Fulu(fulu_inner);
        assert_ne!(gloas, fulu);
    }

    // ── Partial getters for variant-specific headers ──

    #[test]
    fn partial_getter_header_gloas_succeeds() {
        let bid = BuilderBid::<E>::Gloas(make_gloas_bid());
        assert!(bid.header_gloas().is_ok());
    }

    #[test]
    fn partial_getter_header_gloas_on_fulu_fails() {
        let bid = BuilderBid::<E>::Fulu(make_fulu_bid());
        assert!(bid.header_gloas().is_err());
    }

    #[test]
    fn partial_getter_header_fulu_on_gloas_fails() {
        let bid = BuilderBid::<E>::Gloas(make_gloas_bid());
        assert!(bid.header_fulu().is_err());
    }

    #[test]
    fn partial_getter_header_bellatrix_on_gloas_fails() {
        let bid = BuilderBid::<E>::Gloas(make_gloas_bid());
        assert!(bid.header_bellatrix().is_err());
    }
}
