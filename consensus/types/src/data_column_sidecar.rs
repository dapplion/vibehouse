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
