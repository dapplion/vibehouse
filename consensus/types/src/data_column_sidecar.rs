use crate::beacon_block_body::{BLOB_KZG_COMMITMENTS_INDEX, KzgCommitments};
use crate::context_deserialize;
use crate::test_utils::TestRandom;
use crate::{
    BeaconBlockHeader, BeaconStateError, Epoch, Error, EthSpec, ForkName, Hash256,
    SignedBeaconBlockHeader, Slot,
};
use bls::Signature;
use derivative::Derivative;
use kzg::Error as KzgError;
use kzg::{KzgCommitment, KzgProof};
use merkle_proof::verify_merkle_proof;
use safe_arith::ArithError;
use serde::{Deserialize, Deserializer, Serialize};
use ssz::{Decode, Encode};
use ssz_derive::{Decode, Encode};
use ssz_types::Error as SszError;
use ssz_types::{FixedVector, VariableList};
use std::sync::Arc;
use superstruct::superstruct;
use test_random_derive::TestRandom;
use tree_hash::TreeHash;
use tree_hash_derive::TreeHash;

pub type ColumnIndex = u64;
pub type Cell<E> = FixedVector<u8, <E as EthSpec>::BytesPerCell>;
pub type DataColumn<E> = VariableList<Cell<E>, <E as EthSpec>::MaxBlobCommitmentsPerBlock>;

/// Identifies a set of data columns associated with a specific beacon block.
#[derive(Encode, Decode, Clone, Debug, PartialEq, TreeHash, Deserialize)]
#[context_deserialize(ForkName)]
pub struct DataColumnsByRootIdentifier<E: EthSpec> {
    pub block_root: Hash256,
    pub columns: VariableList<ColumnIndex, E::NumberOfColumns>,
}

pub type DataColumnSidecarList<E> = Vec<Arc<DataColumnSidecar<E>>>;

#[superstruct(
    variants(Fulu, Gloas),
    variant_attributes(
        derive(
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
        derivative(PartialEq, Eq, Hash(bound = "E: EthSpec")),
        serde(bound = "E: EthSpec", deny_unknown_fields),
        cfg_attr(
            feature = "arbitrary",
            derive(arbitrary::Arbitrary),
            arbitrary(bound = "E: EthSpec"),
        ),
        context_deserialize(ForkName),
    ),
    ref_attributes(derive(Debug)),
    cast_error(ty = "Error", expr = "BeaconStateError::IncorrectStateVariant"),
    partial_getter_error(ty = "Error", expr = "BeaconStateError::IncorrectStateVariant")
)]
#[derive(Debug, Clone, Serialize, Deserialize, Encode, TreeHash, Derivative)]
#[derivative(PartialEq, Eq, Hash(bound = "E: EthSpec"))]
#[serde(bound = "E: EthSpec", untagged)]
#[tree_hash(enum_behaviour = "transparent")]
#[ssz(enum_behaviour = "transparent")]
pub struct DataColumnSidecar<E: EthSpec> {
    #[serde(with = "serde_utils::quoted_u64")]
    #[superstruct(getter(copy))]
    pub index: ColumnIndex,
    #[serde(with = "ssz_types::serde_utils::list_of_hex_fixed_vec")]
    pub column: DataColumn<E>,
    /// All the KZG commitments associated with the block, used for verifying sample cells.
    /// Removed in Gloas — commitments are in `signed_execution_payload_bid.message.blob_kzg_commitments`.
    #[superstruct(only(Fulu))]
    pub kzg_commitments: KzgCommitments<E>,
    pub kzg_proofs: VariableList<KzgProof, E::MaxBlobCommitmentsPerBlock>,
    #[superstruct(only(Fulu))]
    pub signed_block_header: SignedBeaconBlockHeader,
    /// An inclusion proof, proving the inclusion of `blob_kzg_commitments` in `BeaconBlockBody`.
    /// Removed in Gloas — no longer needed with ePBS.
    #[superstruct(only(Fulu))]
    pub kzg_commitments_inclusion_proof: FixedVector<Hash256, E::KzgCommitmentsInclusionProofDepth>,
    /// The slot of the beacon block. Added in Gloas to replace signed_block_header.
    #[superstruct(only(Gloas), partial_getter(rename = "sidecar_slot", copy))]
    #[serde(with = "serde_utils::quoted_u64")]
    pub slot: Slot,
    /// The root of the beacon block. Added in Gloas to replace signed_block_header.
    #[superstruct(
        only(Gloas),
        partial_getter(rename = "sidecar_beacon_block_root", copy)
    )]
    pub beacon_block_root: Hash256,
}

impl<E: EthSpec> DataColumnSidecar<E> {
    pub fn slot(&self) -> Slot {
        match self {
            DataColumnSidecar::Fulu(inner) => inner.signed_block_header.message.slot,
            DataColumnSidecar::Gloas(inner) => inner.slot,
        }
    }

    pub fn epoch(&self) -> Epoch {
        self.slot().epoch(E::slots_per_epoch())
    }

    pub fn block_root(&self) -> Hash256 {
        match self {
            DataColumnSidecar::Fulu(inner) => inner.signed_block_header.message.tree_hash_root(),
            DataColumnSidecar::Gloas(inner) => inner.beacon_block_root,
        }
    }

    pub fn block_parent_root(&self) -> Option<Hash256> {
        match self {
            DataColumnSidecar::Fulu(inner) => Some(inner.signed_block_header.message.parent_root),
            DataColumnSidecar::Gloas(_) => None,
        }
    }

    pub fn block_proposer_index(&self) -> Option<u64> {
        match self {
            DataColumnSidecar::Fulu(inner) => {
                Some(inner.signed_block_header.message.proposer_index)
            }
            DataColumnSidecar::Gloas(_) => None,
        }
    }

    /// Verifies the kzg commitment inclusion merkle proof.
    /// Only available for Fulu — Gloas removed the inclusion proof.
    pub fn verify_inclusion_proof(&self) -> bool {
        match self {
            DataColumnSidecar::Fulu(inner) => {
                let blob_kzg_commitments_root = inner.kzg_commitments.tree_hash_root();
                verify_merkle_proof(
                    blob_kzg_commitments_root,
                    &inner.kzg_commitments_inclusion_proof,
                    E::kzg_commitments_inclusion_proof_depth(),
                    BLOB_KZG_COMMITMENTS_INDEX,
                    inner.signed_block_header.message.body_root,
                )
            }
            DataColumnSidecar::Gloas(_) => {
                // Gloas doesn't have inclusion proofs — validated via ePBS bid commitments
                true
            }
        }
    }

    pub fn min_size() -> usize {
        // Use Fulu variant for size calculation (larger variant)
        DataColumnSidecarFulu::<E> {
            index: 0,
            column: VariableList::new(vec![Cell::<E>::default()]).unwrap(),
            kzg_commitments: VariableList::new(vec![KzgCommitment::empty_for_testing()]).unwrap(),
            kzg_proofs: VariableList::new(vec![KzgProof::empty()]).unwrap(),
            signed_block_header: SignedBeaconBlockHeader {
                message: BeaconBlockHeader::empty(),
                signature: Signature::empty(),
            },
            kzg_commitments_inclusion_proof: Default::default(),
        }
        .as_ssz_bytes()
        .len()
    }

    pub fn max_size(max_blobs_per_block: usize) -> usize {
        // Use Fulu variant for size calculation (larger variant)
        DataColumnSidecarFulu::<E> {
            index: 0,
            column: VariableList::new(vec![Cell::<E>::default(); max_blobs_per_block]).unwrap(),
            kzg_commitments: VariableList::new(vec![
                KzgCommitment::empty_for_testing();
                max_blobs_per_block
            ])
            .unwrap(),
            kzg_proofs: VariableList::new(vec![KzgProof::empty(); max_blobs_per_block]).unwrap(),
            signed_block_header: SignedBeaconBlockHeader {
                message: BeaconBlockHeader::empty(),
                signature: Signature::empty(),
            },
            kzg_commitments_inclusion_proof: Default::default(),
        }
        .as_ssz_bytes()
        .len()
    }

    /// SSZ decode with explicit fork variant.
    pub fn from_ssz_bytes_by_fork(
        bytes: &[u8],
        fork_name: ForkName,
    ) -> Result<Self, ssz::DecodeError> {
        match fork_name {
            ForkName::Fulu => DataColumnSidecarFulu::from_ssz_bytes(bytes).map(Self::Fulu),
            ForkName::Gloas => DataColumnSidecarGloas::from_ssz_bytes(bytes).map(Self::Gloas),
            _ => Err(ssz::DecodeError::BytesInvalid(format!(
                "unsupported fork for DataColumnSidecar: {fork_name}",
            ))),
        }
    }

    /// SSZ decode which attempts to decode all variants.
    ///
    /// This is useful when the fork is not known, e.g. when reading from the database.
    pub fn any_from_ssz_bytes(bytes: &[u8]) -> Result<Self, ssz::DecodeError> {
        DataColumnSidecarFulu::from_ssz_bytes(bytes)
            .map(Self::Fulu)
            .or_else(|_| DataColumnSidecarGloas::from_ssz_bytes(bytes).map(Self::Gloas))
    }
}

impl<'de, E: EthSpec> context_deserialize::ContextDeserialize<'de, ForkName>
    for DataColumnSidecar<E>
{
    fn context_deserialize<D>(deserializer: D, context: ForkName) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let convert_err = |e| {
            serde::de::Error::custom(format!("DataColumnSidecar failed to deserialize: {:?}", e))
        };
        Ok(match context {
            ForkName::Fulu => {
                Self::Fulu(Deserialize::deserialize(deserializer).map_err(convert_err)?)
            }
            ForkName::Gloas => {
                Self::Gloas(Deserialize::deserialize(deserializer).map_err(convert_err)?)
            }
            _ => {
                return Err(serde::de::Error::custom(format!(
                    "DataColumnSidecar failed to deserialize: unsupported fork '{}'",
                    context
                )));
            }
        })
    }
}

#[derive(Debug)]
pub enum DataColumnSidecarError {
    ArithError(ArithError),
    BeaconStateError(BeaconStateError),
    DataColumnIndexOutOfBounds,
    KzgCommitmentInclusionProofOutOfBounds,
    KzgError(KzgError),
    KzgNotInitialized,
    MissingBlobSidecars,
    PreDeneb,
    SszError(SszError),
    BuildSidecarFailed(String),
    InvalidCellProofLength { expected: usize, actual: usize },
}

impl From<ArithError> for DataColumnSidecarError {
    fn from(e: ArithError) -> Self {
        Self::ArithError(e)
    }
}

impl From<BeaconStateError> for DataColumnSidecarError {
    fn from(e: BeaconStateError) -> Self {
        Self::BeaconStateError(e)
    }
}

impl From<KzgError> for DataColumnSidecarError {
    fn from(e: KzgError) -> Self {
        Self::KzgError(e)
    }
}

impl From<SszError> for DataColumnSidecarError {
    fn from(e: SszError) -> Self {
        Self::SszError(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MinimalEthSpec;

    type E = MinimalEthSpec;

    fn make_gloas_sidecar() -> DataColumnSidecarGloas<E> {
        DataColumnSidecarGloas {
            index: 5,
            column: VariableList::empty(),
            kzg_proofs: VariableList::empty(),
            slot: Slot::new(42),
            beacon_block_root: Hash256::repeat_byte(0xAA),
        }
    }

    fn make_fulu_sidecar() -> DataColumnSidecarFulu<E> {
        DataColumnSidecarFulu {
            index: 3,
            column: VariableList::empty(),
            kzg_commitments: VariableList::empty(),
            kzg_proofs: VariableList::empty(),
            signed_block_header: SignedBeaconBlockHeader {
                message: BeaconBlockHeader {
                    slot: Slot::new(10),
                    proposer_index: 7,
                    parent_root: Hash256::repeat_byte(0xBB),
                    state_root: Hash256::repeat_byte(0xCC),
                    body_root: Hash256::repeat_byte(0xDD),
                },
                signature: Signature::empty(),
            },
            kzg_commitments_inclusion_proof: FixedVector::default(),
        }
    }

    // ── slot() ──

    #[test]
    fn gloas_slot_from_field() {
        let sidecar = DataColumnSidecar::<E>::Gloas(make_gloas_sidecar());
        assert_eq!(sidecar.slot(), Slot::new(42));
    }

    #[test]
    fn fulu_slot_from_header() {
        let sidecar = DataColumnSidecar::<E>::Fulu(make_fulu_sidecar());
        assert_eq!(sidecar.slot(), Slot::new(10));
    }

    // ── epoch() ──

    #[test]
    fn gloas_epoch() {
        let sidecar = DataColumnSidecar::<E>::Gloas(make_gloas_sidecar());
        // MinimalEthSpec: 8 slots per epoch. slot 42 / 8 = epoch 5
        assert_eq!(sidecar.epoch(), Epoch::new(5));
    }

    #[test]
    fn fulu_epoch() {
        let sidecar = DataColumnSidecar::<E>::Fulu(make_fulu_sidecar());
        // slot 10 / 8 = epoch 1
        assert_eq!(sidecar.epoch(), Epoch::new(1));
    }

    // ── block_root() ──

    #[test]
    fn gloas_block_root_from_field() {
        let sidecar = DataColumnSidecar::<E>::Gloas(make_gloas_sidecar());
        assert_eq!(sidecar.block_root(), Hash256::repeat_byte(0xAA));
    }

    #[test]
    fn fulu_block_root_from_header_tree_hash() {
        let inner = make_fulu_sidecar();
        let expected = inner.signed_block_header.message.tree_hash_root();
        let sidecar = DataColumnSidecar::<E>::Fulu(inner);
        assert_eq!(sidecar.block_root(), expected);
    }

    // ── block_parent_root() ──

    #[test]
    fn gloas_parent_root_is_none() {
        let sidecar = DataColumnSidecar::<E>::Gloas(make_gloas_sidecar());
        assert_eq!(sidecar.block_parent_root(), None);
    }

    #[test]
    fn fulu_parent_root_from_header() {
        let sidecar = DataColumnSidecar::<E>::Fulu(make_fulu_sidecar());
        assert_eq!(
            sidecar.block_parent_root(),
            Some(Hash256::repeat_byte(0xBB))
        );
    }

    // ── block_proposer_index() ──

    #[test]
    fn gloas_proposer_index_is_none() {
        let sidecar = DataColumnSidecar::<E>::Gloas(make_gloas_sidecar());
        assert_eq!(sidecar.block_proposer_index(), None);
    }

    #[test]
    fn fulu_proposer_index_from_header() {
        let sidecar = DataColumnSidecar::<E>::Fulu(make_fulu_sidecar());
        assert_eq!(sidecar.block_proposer_index(), Some(7));
    }

    // ── verify_inclusion_proof() ──

    #[test]
    fn gloas_inclusion_proof_always_true() {
        let sidecar = DataColumnSidecar::<E>::Gloas(make_gloas_sidecar());
        assert!(sidecar.verify_inclusion_proof());
    }

    #[test]
    fn fulu_inclusion_proof_default_fails() {
        // A default/empty Fulu sidecar won't have a valid merkle proof
        let sidecar = DataColumnSidecar::<E>::Fulu(make_fulu_sidecar());
        assert!(!sidecar.verify_inclusion_proof());
    }

    // ── index() (shared getter) ──

    #[test]
    fn gloas_index() {
        let sidecar = DataColumnSidecar::<E>::Gloas(make_gloas_sidecar());
        assert_eq!(sidecar.index(), 5);
    }

    #[test]
    fn fulu_index() {
        let sidecar = DataColumnSidecar::<E>::Fulu(make_fulu_sidecar());
        assert_eq!(sidecar.index(), 3);
    }

    // ── SSZ roundtrip (inner types) ──

    #[test]
    fn ssz_roundtrip_gloas_inner() {
        let original = make_gloas_sidecar();
        let bytes = original.as_ssz_bytes();
        let decoded =
            DataColumnSidecarGloas::<E>::from_ssz_bytes(&bytes).expect("SSZ decode succeeds");
        assert_eq!(decoded, original);
    }

    #[test]
    fn ssz_roundtrip_fulu_inner() {
        let original = make_fulu_sidecar();
        let bytes = original.as_ssz_bytes();
        let decoded =
            DataColumnSidecarFulu::<E>::from_ssz_bytes(&bytes).expect("SSZ decode succeeds");
        assert_eq!(decoded, original);
    }

    // ── SSZ roundtrip (enum via from_ssz_bytes_by_fork) ──

    #[test]
    fn ssz_roundtrip_gloas_via_fork_dispatch() {
        let inner = make_gloas_sidecar();
        let wrapped = DataColumnSidecar::<E>::Gloas(inner);
        let bytes = wrapped.as_ssz_bytes();
        let decoded = DataColumnSidecar::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Gloas)
            .expect("SSZ decode succeeds");
        assert_eq!(decoded, wrapped);
    }

    #[test]
    fn ssz_roundtrip_fulu_via_fork_dispatch() {
        let inner = make_fulu_sidecar();
        let wrapped = DataColumnSidecar::<E>::Fulu(inner);
        let bytes = wrapped.as_ssz_bytes();
        let decoded = DataColumnSidecar::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Fulu)
            .expect("SSZ decode succeeds");
        assert_eq!(decoded, wrapped);
    }

    // ── from_ssz_bytes_by_fork unsupported forks ──

    #[test]
    fn ssz_decode_base_fork_fails() {
        let bytes = [0u8; 64];
        assert!(DataColumnSidecar::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Base).is_err());
    }

    #[test]
    fn ssz_decode_altair_fork_fails() {
        let bytes = [0u8; 64];
        assert!(DataColumnSidecar::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Altair).is_err());
    }

    #[test]
    fn ssz_decode_deneb_fork_fails() {
        let bytes = [0u8; 64];
        assert!(DataColumnSidecar::<E>::from_ssz_bytes_by_fork(&bytes, ForkName::Deneb).is_err());
    }

    // ── from_ssz_bytes_by_fork cross-variant ──

    #[test]
    fn ssz_fork_dispatch_produces_correct_variant() {
        let gloas = make_gloas_sidecar();
        let gloas_bytes = gloas.as_ssz_bytes();

        let as_gloas =
            DataColumnSidecar::<E>::from_ssz_bytes_by_fork(&gloas_bytes, ForkName::Gloas)
                .expect("decode as Gloas");

        // Should be the Gloas variant
        assert!(matches!(as_gloas, DataColumnSidecar::Gloas(_)));
    }

    // ── any_from_ssz_bytes ──

    #[test]
    fn any_from_ssz_bytes_fulu() {
        let inner = make_fulu_sidecar();
        let bytes = inner.as_ssz_bytes();
        let decoded =
            DataColumnSidecar::<E>::any_from_ssz_bytes(&bytes).expect("any decode succeeds");
        // Fulu is tried first, so Fulu bytes should decode as Fulu
        assert!(matches!(decoded, DataColumnSidecar::Fulu(_)));
        assert_eq!(decoded.index(), 3);
    }

    #[test]
    fn any_from_ssz_bytes_gloas() {
        let inner = make_gloas_sidecar();
        let bytes = inner.as_ssz_bytes();
        // Gloas bytes are structurally different from Fulu (no header, no commitments),
        // so Fulu decode should fail and fallback to Gloas
        let decoded =
            DataColumnSidecar::<E>::any_from_ssz_bytes(&bytes).expect("any decode succeeds");
        assert_eq!(decoded.slot(), Slot::new(42));
        assert_eq!(decoded.index(), 5);
    }

    // ── min_size / max_size ──

    #[test]
    fn min_size_is_positive() {
        let min = DataColumnSidecar::<E>::min_size();
        assert!(min > 0, "min_size should be positive, got {min}");
    }

    #[test]
    fn max_size_greater_than_min() {
        let min = DataColumnSidecar::<E>::min_size();
        let max = DataColumnSidecar::<E>::max_size(6);
        assert!(
            max > min,
            "max_size({max}) should exceed min_size({min}) with >1 blob"
        );
    }

    #[test]
    fn max_size_one_blob_greater_than_min() {
        let min = DataColumnSidecar::<E>::min_size();
        let max = DataColumnSidecar::<E>::max_size(1);
        // With 1 blob, max should equal min (both use 1 element vectors)
        assert_eq!(min, max);
    }

    // ── Gloas-specific partial getters ──

    #[test]
    fn gloas_sidecar_slot_getter() {
        let inner = make_gloas_sidecar();
        let sidecar = DataColumnSidecar::<E>::Gloas(inner);
        assert_eq!(sidecar.sidecar_slot().unwrap(), Slot::new(42));
    }

    #[test]
    fn gloas_sidecar_beacon_block_root_getter() {
        let inner = make_gloas_sidecar();
        let sidecar = DataColumnSidecar::<E>::Gloas(inner);
        assert_eq!(
            sidecar.sidecar_beacon_block_root().unwrap(),
            Hash256::repeat_byte(0xAA)
        );
    }

    #[test]
    fn fulu_sidecar_slot_getter_fails() {
        let inner = make_fulu_sidecar();
        let sidecar = DataColumnSidecar::<E>::Fulu(inner);
        assert!(sidecar.sidecar_slot().is_err());
    }

    #[test]
    fn fulu_sidecar_beacon_block_root_getter_fails() {
        let inner = make_fulu_sidecar();
        let sidecar = DataColumnSidecar::<E>::Fulu(inner);
        assert!(sidecar.sidecar_beacon_block_root().is_err());
    }

    // ── Fulu-specific partial getters ──

    #[test]
    fn fulu_kzg_commitments_getter() {
        let inner = make_fulu_sidecar();
        let sidecar = DataColumnSidecar::<E>::Fulu(inner);
        assert!(sidecar.kzg_commitments().is_ok());
    }

    #[test]
    fn gloas_kzg_commitments_getter_fails() {
        let inner = make_gloas_sidecar();
        let sidecar = DataColumnSidecar::<E>::Gloas(inner);
        assert!(sidecar.kzg_commitments().is_err());
    }

    #[test]
    fn fulu_signed_block_header_getter() {
        let inner = make_fulu_sidecar();
        let sidecar = DataColumnSidecar::<E>::Fulu(inner);
        let header = sidecar.signed_block_header().unwrap();
        assert_eq!(header.message.proposer_index, 7);
    }

    #[test]
    fn gloas_signed_block_header_getter_fails() {
        let inner = make_gloas_sidecar();
        let sidecar = DataColumnSidecar::<E>::Gloas(inner);
        assert!(sidecar.signed_block_header().is_err());
    }

    // ── Clone and equality ──

    #[test]
    fn gloas_clone_equality() {
        let sidecar = DataColumnSidecar::<E>::Gloas(make_gloas_sidecar());
        let cloned = sidecar.clone();
        assert_eq!(sidecar, cloned);
    }

    #[test]
    fn fulu_clone_equality() {
        let sidecar = DataColumnSidecar::<E>::Fulu(make_fulu_sidecar());
        let cloned = sidecar.clone();
        assert_eq!(sidecar, cloned);
    }

    #[test]
    fn different_variants_not_equal() {
        let gloas = DataColumnSidecar::<E>::Gloas(make_gloas_sidecar());
        let fulu = DataColumnSidecar::<E>::Fulu(make_fulu_sidecar());
        assert_ne!(gloas, fulu);
    }

    // ── Tree hash stability ──

    #[test]
    fn gloas_tree_hash_deterministic() {
        let sidecar = DataColumnSidecar::<E>::Gloas(make_gloas_sidecar());
        let hash1 = sidecar.tree_hash_root();
        let hash2 = sidecar.tree_hash_root();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn gloas_tree_hash_changes_with_different_data() {
        let mut inner = make_gloas_sidecar();
        let sidecar1 = DataColumnSidecar::<E>::Gloas(inner.clone());

        inner.beacon_block_root = Hash256::repeat_byte(0xFF);
        let sidecar2 = DataColumnSidecar::<E>::Gloas(inner);

        assert_ne!(sidecar1.tree_hash_root(), sidecar2.tree_hash_root());
    }

    // ── Gloas epoch boundary ──

    #[test]
    fn gloas_epoch_at_boundary() {
        let mut inner = make_gloas_sidecar();
        // Slot 0 = epoch 0
        inner.slot = Slot::new(0);
        let sidecar = DataColumnSidecar::<E>::Gloas(inner);
        assert_eq!(sidecar.epoch(), Epoch::new(0));
    }

    #[test]
    fn gloas_epoch_at_boundary_next() {
        let mut inner = make_gloas_sidecar();
        // Slot 8 (first slot of epoch 1 in minimal)
        inner.slot = Slot::new(8);
        let sidecar = DataColumnSidecar::<E>::Gloas(inner);
        assert_eq!(sidecar.epoch(), Epoch::new(1));
    }
}
