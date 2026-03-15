use crate::data_availability_checker::AvailabilityCheckError;
pub use crate::data_availability_checker::{AvailableBlock, MaybeAvailableBlock};
use crate::data_column_verification::{CustodyDataColumn, CustodyDataColumnList};
use crate::{PayloadVerificationOutcome, get_block_root};
use educe::Educe;
use ssz_types::VariableList;
use state_processing::ConsensusContext;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;
use types::blob_sidecar::BlobIdentifier;
use types::{
    BeaconBlockRef, BeaconState, BlindedPayload, BlobSidecarList, Epoch, EthSpec, Hash256,
    SignedBeaconBlock, SignedBeaconBlockHeader, SignedExecutionPayloadEnvelope, Slot,
};

/// A block that has been received over RPC. It has 2 internal variants:
///
/// 1. `BlockAndBlobs`: A fully available post deneb block with all the blobs available. This variant
///    is only constructed after making consistency checks between blocks and blobs.
///    Hence, it is fully self contained w.r.t verification. i.e. this block has all the required
///    data to get verified and imported into fork choice.
///
/// 2. `Block`: This can be a fully available pre-deneb block **or** a post-deneb block that may or may
///    not require blobs to be considered fully available.
///
/// Note: We make a distinction over blocks received over gossip because
/// in a post-deneb world, the blobs corresponding to a given block that are received
/// over rpc do not contain the proposer signature for dos resistance.
#[derive(Clone, Educe)]
#[educe(Hash(bound(E: EthSpec)))]
pub struct RpcBlock<E: EthSpec> {
    block_root: Hash256,
    block: RpcBlockInner<E>,
    /// Optional execution payload envelope for Gloas ePBS blocks.
    /// Set during range sync when envelopes are downloaded alongside blocks.
    #[educe(Hash(ignore))]
    envelope: Option<Arc<SignedExecutionPayloadEnvelope<E>>>,
}

impl<E: EthSpec> Debug for RpcBlock<E> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "RpcBlock({:?})", self.block_root)
    }
}

impl<E: EthSpec> RpcBlock<E> {
    pub fn block_root(&self) -> Hash256 {
        self.block_root
    }

    pub fn as_block(&self) -> &SignedBeaconBlock<E> {
        match &self.block {
            RpcBlockInner::Block(block) => block,
            RpcBlockInner::BlockAndBlobs(block, _) => block,
            RpcBlockInner::BlockAndCustodyColumns(block, _) => block,
        }
    }

    pub fn block_cloned(&self) -> Arc<SignedBeaconBlock<E>> {
        match &self.block {
            RpcBlockInner::Block(block) => block.clone(),
            RpcBlockInner::BlockAndBlobs(block, _) => block.clone(),
            RpcBlockInner::BlockAndCustodyColumns(block, _) => block.clone(),
        }
    }

    pub fn blobs(&self) -> Option<&BlobSidecarList<E>> {
        match &self.block {
            RpcBlockInner::Block(_) => None,
            RpcBlockInner::BlockAndBlobs(_, blobs) => Some(blobs),
            RpcBlockInner::BlockAndCustodyColumns(_, _) => None,
        }
    }

    pub fn custody_columns(&self) -> Option<&CustodyDataColumnList<E>> {
        match &self.block {
            RpcBlockInner::Block(_) => None,
            RpcBlockInner::BlockAndBlobs(_, _) => None,
            RpcBlockInner::BlockAndCustodyColumns(_, data_columns) => Some(data_columns),
        }
    }
}

/// Note: This variant is intentionally private because we want to safely construct the
/// internal variants after applying consistency checks to ensure that the block and blobs
/// are consistent with respect to each other.
#[derive(Debug, Clone, Educe)]
#[educe(Hash(bound(E: EthSpec)))]
enum RpcBlockInner<E: EthSpec> {
    /// Single block lookup response. This should potentially hit the data availability cache.
    Block(Arc<SignedBeaconBlock<E>>),
    /// This variant is used with parent lookups and by-range responses. It should have all blobs
    /// ordered, all block roots matching, and the correct number of blobs for this block.
    BlockAndBlobs(Arc<SignedBeaconBlock<E>>, BlobSidecarList<E>),
    /// This variant is used with parent lookups and by-range responses. It should have all
    /// requested data columns, all block roots matching for this block.
    BlockAndCustodyColumns(Arc<SignedBeaconBlock<E>>, CustodyDataColumnList<E>),
}

impl<E: EthSpec> RpcBlock<E> {
    /// Constructs a `Block` variant.
    pub fn new_without_blobs(
        block_root: Option<Hash256>,
        block: Arc<SignedBeaconBlock<E>>,
    ) -> Self {
        let block_root = block_root.unwrap_or_else(|| get_block_root(&block));

        Self {
            block_root,
            block: RpcBlockInner::Block(block),
            envelope: None,
        }
    }

    /// Constructs a new `BlockAndBlobs` variant after making consistency
    /// checks between the provided blocks and blobs. This struct makes no
    /// guarantees about whether blobs should be present, only that they are
    /// consistent with the block. An empty list passed in for `blobs` is
    /// viewed the same as `None` passed in.
    pub fn new(
        block_root: Option<Hash256>,
        block: Arc<SignedBeaconBlock<E>>,
        blobs: Option<BlobSidecarList<E>>,
    ) -> Result<Self, AvailabilityCheckError> {
        let block_root = block_root.unwrap_or_else(|| get_block_root(&block));
        // Treat empty blob lists as if they are missing.
        let blobs = blobs.filter(|b| !b.is_empty());

        if let (Some(blobs), Ok(block_commitments)) = (
            blobs.as_ref(),
            block.message().body().blob_kzg_commitments(),
        ) {
            if blobs.len() != block_commitments.len() {
                return Err(AvailabilityCheckError::MissingBlobs);
            }
            for (blob, &block_commitment) in blobs.iter().zip(block_commitments.iter()) {
                let blob_commitment = blob.kzg_commitment;
                if blob_commitment != block_commitment {
                    return Err(AvailabilityCheckError::KzgCommitmentMismatch {
                        block_commitment,
                        blob_commitment,
                    });
                }
            }
        }
        let inner = match blobs {
            Some(blobs) => RpcBlockInner::BlockAndBlobs(block, blobs),
            None => RpcBlockInner::Block(block),
        };
        Ok(Self {
            block_root,
            block: inner,
            envelope: None,
        })
    }

    pub fn new_with_custody_columns(
        block_root: Option<Hash256>,
        block: Arc<SignedBeaconBlock<E>>,
        custody_columns: Vec<CustodyDataColumn<E>>,
    ) -> Result<Self, AvailabilityCheckError> {
        let block_root = block_root.unwrap_or_else(|| get_block_root(&block));

        if block.num_expected_blobs() > 0 && custody_columns.is_empty() {
            // The number of required custody columns is out of scope here.
            return Err(AvailabilityCheckError::MissingCustodyColumns);
        }
        // Treat empty data column lists as if they are missing.
        let inner = if !custody_columns.is_empty() {
            RpcBlockInner::BlockAndCustodyColumns(block, VariableList::new(custody_columns)?)
        } else {
            RpcBlockInner::Block(block)
        };
        Ok(Self {
            block_root,
            block: inner,
            envelope: None,
        })
    }

    #[allow(clippy::type_complexity)]
    pub fn deconstruct(
        self,
    ) -> (
        Hash256,
        Arc<SignedBeaconBlock<E>>,
        Option<BlobSidecarList<E>>,
        Option<CustodyDataColumnList<E>>,
    ) {
        let block_root = self.block_root();
        match self.block {
            RpcBlockInner::Block(block) => (block_root, block, None, None),
            RpcBlockInner::BlockAndBlobs(block, blobs) => (block_root, block, Some(blobs), None),
            RpcBlockInner::BlockAndCustodyColumns(block, data_columns) => {
                (block_root, block, None, Some(data_columns))
            }
        }
    }
    pub fn n_blobs(&self) -> usize {
        match &self.block {
            RpcBlockInner::Block(_) | RpcBlockInner::BlockAndCustodyColumns(_, _) => 0,
            RpcBlockInner::BlockAndBlobs(_, blobs) => blobs.len(),
        }
    }
    pub fn n_data_columns(&self) -> usize {
        match &self.block {
            RpcBlockInner::Block(_) | RpcBlockInner::BlockAndBlobs(_, _) => 0,
            RpcBlockInner::BlockAndCustodyColumns(_, data_columns) => data_columns.len(),
        }
    }

    /// Returns the optional execution payload envelope (Gloas ePBS).
    pub fn envelope(&self) -> Option<&Arc<SignedExecutionPayloadEnvelope<E>>> {
        self.envelope.as_ref()
    }

    /// Attaches an execution payload envelope to this block (for range sync).
    pub fn set_envelope(&mut self, envelope: Arc<SignedExecutionPayloadEnvelope<E>>) {
        self.envelope = Some(envelope);
    }

    /// Takes the envelope out of this block, leaving `None` in its place.
    pub fn take_envelope(&mut self) -> Option<Arc<SignedExecutionPayloadEnvelope<E>>> {
        self.envelope.take()
    }
}

/// A block that has gone through all pre-deneb block processing checks including block processing
/// and execution by an EL client. This block hasn't necessarily completed data availability checks.
///
///
/// It contains 2 variants:
/// 1. `Available`: This block has been executed and also contains all data to consider it a
///    fully available block. i.e. for post-deneb, this implies that this contains all the
///    required blobs.
/// 2. `AvailabilityPending`: This block hasn't received all required blobs to consider it a
///    fully available block.
pub enum ExecutedBlock<E: EthSpec> {
    Available(AvailableExecutedBlock<E>),
    AvailabilityPending(AvailabilityPendingExecutedBlock<E>),
}

impl<E: EthSpec> ExecutedBlock<E> {
    pub fn new(
        block: MaybeAvailableBlock<E>,
        import_data: BlockImportData<E>,
        payload_verification_outcome: PayloadVerificationOutcome,
    ) -> Self {
        match block {
            MaybeAvailableBlock::Available(available_block) => {
                Self::Available(AvailableExecutedBlock::new(
                    available_block,
                    import_data,
                    payload_verification_outcome,
                ))
            }
            MaybeAvailableBlock::AvailabilityPending {
                block_root: _,
                block: pending_block,
            } => Self::AvailabilityPending(AvailabilityPendingExecutedBlock::new(
                pending_block,
                import_data,
                payload_verification_outcome,
            )),
        }
    }

    pub fn as_block(&self) -> &SignedBeaconBlock<E> {
        match self {
            Self::Available(available) => available.block.block(),
            Self::AvailabilityPending(pending) => &pending.block,
        }
    }

    pub fn block_root(&self) -> Hash256 {
        match self {
            ExecutedBlock::AvailabilityPending(pending) => pending.import_data.block_root,
            ExecutedBlock::Available(available) => available.import_data.block_root,
        }
    }
}

/// A block that has completed all pre-deneb block processing checks including verification
/// by an EL client **and** has all requisite blob data to be imported into fork choice.
pub struct AvailableExecutedBlock<E: EthSpec> {
    pub block: AvailableBlock<E>,
    pub import_data: BlockImportData<E>,
    pub payload_verification_outcome: PayloadVerificationOutcome,
}

impl<E: EthSpec> AvailableExecutedBlock<E> {
    pub fn new(
        block: AvailableBlock<E>,
        import_data: BlockImportData<E>,
        payload_verification_outcome: PayloadVerificationOutcome,
    ) -> Self {
        Self {
            block,
            import_data,
            payload_verification_outcome,
        }
    }

    pub fn get_all_blob_ids(&self) -> Vec<BlobIdentifier> {
        let num_blobs_expected = self
            .block
            .message()
            .body()
            .blob_kzg_commitments()
            .map_or(0, |commitments| commitments.len());
        let mut blob_ids = Vec::with_capacity(num_blobs_expected);
        for i in 0..num_blobs_expected {
            blob_ids.push(BlobIdentifier {
                block_root: self.import_data.block_root,
                index: i as u64,
            });
        }
        blob_ids
    }
}

/// A block that has completed all pre-deneb block processing checks, verification
/// by an EL client but does not have all requisite blob data to get imported into
/// fork choice.
pub struct AvailabilityPendingExecutedBlock<E: EthSpec> {
    pub block: Arc<SignedBeaconBlock<E>>,
    pub import_data: BlockImportData<E>,
    pub payload_verification_outcome: PayloadVerificationOutcome,
}

impl<E: EthSpec> AvailabilityPendingExecutedBlock<E> {
    pub fn new(
        block: Arc<SignedBeaconBlock<E>>,
        import_data: BlockImportData<E>,
        payload_verification_outcome: PayloadVerificationOutcome,
    ) -> Self {
        Self {
            block,
            import_data,
            payload_verification_outcome,
        }
    }

    pub fn as_block(&self) -> &SignedBeaconBlock<E> {
        &self.block
    }

    pub fn num_blobs_expected(&self) -> usize {
        self.block
            .message()
            .body()
            .blob_kzg_commitments()
            .map_or(0, |commitments| commitments.len())
    }
}

#[derive(Debug, PartialEq)]
pub struct BlockImportData<E: EthSpec> {
    pub block_root: Hash256,
    pub state: BeaconState<E>,
    pub parent_block: SignedBeaconBlock<E, BlindedPayload<E>>,
    pub consensus_context: ConsensusContext<E>,
}

impl<E: EthSpec> BlockImportData<E> {
    pub fn __new_for_test(
        block_root: Hash256,
        state: BeaconState<E>,
        parent_block: SignedBeaconBlock<E, BlindedPayload<E>>,
    ) -> Self {
        Self {
            block_root,
            state,
            parent_block,
            consensus_context: ConsensusContext::new(Slot::new(0)),
        }
    }
}

/// Trait for common block operations.
pub trait AsBlock<E: EthSpec> {
    fn slot(&self) -> Slot;
    fn epoch(&self) -> Epoch;
    fn parent_root(&self) -> Hash256;
    fn state_root(&self) -> Hash256;
    fn signed_block_header(&self) -> SignedBeaconBlockHeader;
    fn message(&self) -> BeaconBlockRef<'_, E>;
    fn as_block(&self) -> &SignedBeaconBlock<E>;
    fn block_cloned(&self) -> Arc<SignedBeaconBlock<E>>;
    fn canonical_root(&self) -> Hash256;
}

impl<E: EthSpec> AsBlock<E> for Arc<SignedBeaconBlock<E>> {
    fn slot(&self) -> Slot {
        SignedBeaconBlock::slot(self)
    }

    fn epoch(&self) -> Epoch {
        SignedBeaconBlock::epoch(self)
    }

    fn parent_root(&self) -> Hash256 {
        SignedBeaconBlock::parent_root(self)
    }

    fn state_root(&self) -> Hash256 {
        SignedBeaconBlock::state_root(self)
    }

    fn signed_block_header(&self) -> SignedBeaconBlockHeader {
        SignedBeaconBlock::signed_block_header(self)
    }

    fn message(&self) -> BeaconBlockRef<'_, E> {
        SignedBeaconBlock::message(self)
    }

    fn as_block(&self) -> &SignedBeaconBlock<E> {
        self
    }

    fn block_cloned(&self) -> Arc<SignedBeaconBlock<E>> {
        Arc::<SignedBeaconBlock<E>>::clone(self)
    }

    fn canonical_root(&self) -> Hash256 {
        SignedBeaconBlock::canonical_root(self)
    }
}

impl<E: EthSpec> AsBlock<E> for MaybeAvailableBlock<E> {
    fn slot(&self) -> Slot {
        self.as_block().slot()
    }
    fn epoch(&self) -> Epoch {
        self.as_block().epoch()
    }
    fn parent_root(&self) -> Hash256 {
        self.as_block().parent_root()
    }
    fn state_root(&self) -> Hash256 {
        self.as_block().state_root()
    }
    fn signed_block_header(&self) -> SignedBeaconBlockHeader {
        self.as_block().signed_block_header()
    }
    fn message(&self) -> BeaconBlockRef<'_, E> {
        self.as_block().message()
    }
    fn as_block(&self) -> &SignedBeaconBlock<E> {
        match &self {
            MaybeAvailableBlock::Available(block) => block.as_block(),
            MaybeAvailableBlock::AvailabilityPending { block, .. } => block,
        }
    }
    fn block_cloned(&self) -> Arc<SignedBeaconBlock<E>> {
        match &self {
            MaybeAvailableBlock::Available(block) => block.block_cloned(),
            MaybeAvailableBlock::AvailabilityPending { block, .. } => block.clone(),
        }
    }
    fn canonical_root(&self) -> Hash256 {
        self.as_block().canonical_root()
    }
}

impl<E: EthSpec> AsBlock<E> for AvailableBlock<E> {
    fn slot(&self) -> Slot {
        self.block().slot()
    }

    fn epoch(&self) -> Epoch {
        self.block().epoch()
    }

    fn parent_root(&self) -> Hash256 {
        self.block().parent_root()
    }

    fn state_root(&self) -> Hash256 {
        self.block().state_root()
    }

    fn signed_block_header(&self) -> SignedBeaconBlockHeader {
        self.block().signed_block_header()
    }

    fn message(&self) -> BeaconBlockRef<'_, E> {
        self.block().message()
    }

    fn as_block(&self) -> &SignedBeaconBlock<E> {
        self.block()
    }

    fn block_cloned(&self) -> Arc<SignedBeaconBlock<E>> {
        AvailableBlock::block_cloned(self)
    }

    fn canonical_root(&self) -> Hash256 {
        self.block().canonical_root()
    }
}

impl<E: EthSpec> AsBlock<E> for RpcBlock<E> {
    fn slot(&self) -> Slot {
        self.as_block().slot()
    }
    fn epoch(&self) -> Epoch {
        self.as_block().epoch()
    }
    fn parent_root(&self) -> Hash256 {
        self.as_block().parent_root()
    }
    fn state_root(&self) -> Hash256 {
        self.as_block().state_root()
    }
    fn signed_block_header(&self) -> SignedBeaconBlockHeader {
        self.as_block().signed_block_header()
    }
    fn message(&self) -> BeaconBlockRef<'_, E> {
        self.as_block().message()
    }
    fn as_block(&self) -> &SignedBeaconBlock<E> {
        match &self.block {
            RpcBlockInner::Block(block) => block,
            RpcBlockInner::BlockAndBlobs(block, _) => block,
            RpcBlockInner::BlockAndCustodyColumns(block, _) => block,
        }
    }
    fn block_cloned(&self) -> Arc<SignedBeaconBlock<E>> {
        match &self.block {
            RpcBlockInner::Block(block) => block.clone(),
            RpcBlockInner::BlockAndBlobs(block, _) => block.clone(),
            RpcBlockInner::BlockAndCustodyColumns(block, _) => block.clone(),
        }
    }
    fn canonical_root(&self) -> Hash256 {
        self.as_block().canonical_root()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use types::{
        BeaconBlock, BlobSidecarList, ChainSpec, FixedBytesExtended, MinimalEthSpec,
        RuntimeVariableList, Signature, SignedBeaconBlock, SignedExecutionPayloadEnvelope,
    };

    type E = MinimalEthSpec;

    const MAX_BLOBS: usize = 6;

    fn make_block() -> Arc<SignedBeaconBlock<E>> {
        let spec = ChainSpec::minimal();
        Arc::new(SignedBeaconBlock::from_block(
            BeaconBlock::empty(&spec),
            Signature::empty(),
        ))
    }

    fn make_deneb_block(num_commitments: usize) -> Arc<SignedBeaconBlock<E>> {
        use rand::rng;
        use types::test_utils::TestRandom;
        let mut rng = rng();
        let inner = BeaconBlock::Deneb(types::BeaconBlockDeneb::random_for_test(&mut rng));
        let mut block = SignedBeaconBlock::from_block(inner, Signature::random_for_test(&mut rng));

        {
            let mut body = block.message_mut().body_mut();
            let commitments = body
                .blob_kzg_commitments_mut()
                .expect("deneb has commitments");

            *commitments = Default::default();

            for _ in 0..num_commitments {
                commitments
                    .push(kzg::KzgCommitment::empty_for_testing())
                    .unwrap();
            }
        }

        Arc::new(block)
    }

    fn make_blob_sidecar(
        block: &SignedBeaconBlock<E>,
        index: u64,
        commitment: kzg::KzgCommitment,
    ) -> Arc<types::BlobSidecar<E>> {
        Arc::new(types::BlobSidecar {
            index,
            blob: types::Blob::<E>::default(),
            kzg_commitment: commitment,
            kzg_proof: kzg::KzgProof::empty(),
            signed_block_header: block.signed_block_header(),
            kzg_commitment_inclusion_proof: Default::default(),
        })
    }

    fn make_blob_list(blobs: Vec<Arc<types::BlobSidecar<E>>>) -> BlobSidecarList<E> {
        RuntimeVariableList::new(blobs, MAX_BLOBS).unwrap()
    }

    // --- RpcBlock::new_without_blobs ---

    #[test]
    fn new_without_blobs_preserves_block() {
        let block = make_block();
        let rpc = RpcBlock::new_without_blobs(None, block.clone());
        assert_eq!(rpc.as_block().slot(), block.slot());
        assert!(rpc.blobs().is_none());
        assert!(rpc.custody_columns().is_none());
        assert!(rpc.envelope().is_none());
    }

    #[test]
    fn new_without_blobs_uses_provided_root() {
        let block = make_block();
        let custom_root = Hash256::from_low_u64_le(42);
        let rpc = RpcBlock::new_without_blobs(Some(custom_root), block);
        assert_eq!(rpc.block_root(), custom_root);
    }

    #[test]
    fn new_without_blobs_computes_root_when_none() {
        let block = make_block();
        let expected_root = block.canonical_root();
        let rpc = RpcBlock::new_without_blobs(None, block);
        assert_eq!(rpc.block_root(), expected_root);
    }

    // --- RpcBlock::new (blob consistency) ---

    #[test]
    fn new_with_no_blobs_returns_block_variant() {
        let block = make_block();
        let rpc = RpcBlock::new(None, block, None).unwrap();
        assert!(rpc.blobs().is_none());
        assert_eq!(rpc.n_blobs(), 0);
    }

    #[test]
    fn new_with_empty_blob_list_treated_as_none() {
        let block = make_block();
        let empty_blobs: BlobSidecarList<E> = RuntimeVariableList::empty(MAX_BLOBS);
        let rpc = RpcBlock::new(None, block, Some(empty_blobs)).unwrap();
        assert!(rpc.blobs().is_none());
        assert_eq!(rpc.n_blobs(), 0);
    }

    #[test]
    fn new_with_matching_blobs_succeeds() {
        let block = make_deneb_block(2);
        let commitments = block
            .message()
            .body()
            .blob_kzg_commitments()
            .unwrap()
            .clone();

        let blobs = make_blob_list(vec![
            make_blob_sidecar(&block, 0, commitments[0]),
            make_blob_sidecar(&block, 1, commitments[1]),
        ]);

        let rpc = RpcBlock::new(None, block, Some(blobs)).unwrap();
        assert!(rpc.blobs().is_some());
        assert_eq!(rpc.n_blobs(), 2);
    }

    #[test]
    fn new_with_wrong_blob_count_returns_missing_blobs() {
        let block = make_deneb_block(2);
        let commitments = block
            .message()
            .body()
            .blob_kzg_commitments()
            .unwrap()
            .clone();

        let blobs = make_blob_list(vec![make_blob_sidecar(&block, 0, commitments[0])]);

        let result = RpcBlock::new(None, block, Some(blobs));
        assert!(
            matches!(result, Err(AvailabilityCheckError::MissingBlobs)),
            "expected MissingBlobs, got: {:?}",
            result
        );
    }

    #[test]
    fn new_with_mismatched_commitment_returns_error() {
        let block = make_deneb_block(1);

        let wrong_commitment = kzg::KzgCommitment([0xAB; 48]);
        let blobs = make_blob_list(vec![make_blob_sidecar(&block, 0, wrong_commitment)]);

        let result = RpcBlock::new(None, block, Some(blobs));
        assert!(
            matches!(
                result,
                Err(AvailabilityCheckError::KzgCommitmentMismatch { .. })
            ),
            "expected KzgCommitmentMismatch, got: {:?}",
            result
        );
    }

    // --- RpcBlock::deconstruct ---

    #[test]
    fn deconstruct_block_only() {
        let block = make_block();
        let root = block.canonical_root();
        let rpc = RpcBlock::new_without_blobs(None, block.clone());
        let (got_root, got_block, blobs, columns) = rpc.deconstruct();
        assert_eq!(got_root, root);
        assert_eq!(got_block.slot(), block.slot());
        assert!(blobs.is_none());
        assert!(columns.is_none());
    }

    #[test]
    fn deconstruct_block_and_blobs() {
        let block = make_deneb_block(1);
        let commitments = block
            .message()
            .body()
            .blob_kzg_commitments()
            .unwrap()
            .clone();

        let blobs = make_blob_list(vec![make_blob_sidecar(&block, 0, commitments[0])]);

        let rpc = RpcBlock::new(None, block, Some(blobs)).unwrap();
        let (_, _, blobs, columns) = rpc.deconstruct();
        assert!(blobs.is_some());
        assert_eq!(blobs.unwrap().len(), 1);
        assert!(columns.is_none());
    }

    // --- n_blobs, n_data_columns ---

    #[test]
    fn n_blobs_zero_for_block_only() {
        let block = make_block();
        let rpc = RpcBlock::new_without_blobs(None, block);
        assert_eq!(rpc.n_blobs(), 0);
        assert_eq!(rpc.n_data_columns(), 0);
    }

    #[test]
    fn n_blobs_matches_blob_count() {
        let block = make_deneb_block(3);
        let commitments = block
            .message()
            .body()
            .blob_kzg_commitments()
            .unwrap()
            .clone();

        let blobs = make_blob_list(vec![
            make_blob_sidecar(&block, 0, commitments[0]),
            make_blob_sidecar(&block, 1, commitments[1]),
            make_blob_sidecar(&block, 2, commitments[2]),
        ]);

        let rpc = RpcBlock::new(None, block, Some(blobs)).unwrap();
        assert_eq!(rpc.n_blobs(), 3);
        assert_eq!(rpc.n_data_columns(), 0);
    }

    // --- envelope operations ---

    #[test]
    fn envelope_initially_none() {
        let block = make_block();
        let rpc = RpcBlock::new_without_blobs(None, block);
        assert!(rpc.envelope().is_none());
    }

    #[test]
    fn set_and_get_envelope() {
        let block = make_block();
        let mut rpc = RpcBlock::new_without_blobs(None, block);
        let envelope = Arc::new(SignedExecutionPayloadEnvelope::<E>::empty());
        rpc.set_envelope(envelope.clone());
        assert!(rpc.envelope().is_some());
    }

    #[test]
    fn take_envelope_returns_and_clears() {
        let block = make_block();
        let mut rpc = RpcBlock::new_without_blobs(None, block);
        let envelope = Arc::new(SignedExecutionPayloadEnvelope::<E>::empty());
        rpc.set_envelope(envelope);
        let taken = rpc.take_envelope();
        assert!(taken.is_some());
        assert!(rpc.envelope().is_none());
    }

    #[test]
    fn take_envelope_from_empty_returns_none() {
        let block = make_block();
        let mut rpc = RpcBlock::new_without_blobs(None, block);
        assert!(rpc.take_envelope().is_none());
    }

    // --- AsBlock trait ---

    #[test]
    fn as_block_trait_slot() {
        let block = make_block();
        let expected_slot = block.slot();
        let rpc = RpcBlock::new_without_blobs(None, block);
        assert_eq!(AsBlock::slot(&rpc), expected_slot);
    }

    #[test]
    fn as_block_trait_parent_root() {
        let block = make_block();
        let expected = block.parent_root();
        let rpc = RpcBlock::new_without_blobs(None, block);
        assert_eq!(AsBlock::parent_root(&rpc), expected);
    }

    #[test]
    fn as_block_trait_canonical_root() {
        let block = make_block();
        let expected = block.canonical_root();
        let rpc = RpcBlock::new_without_blobs(None, block);
        assert_eq!(AsBlock::canonical_root(&rpc), expected);
    }

    #[test]
    fn block_cloned_returns_same_block() {
        let block = make_block();
        let rpc = RpcBlock::new_without_blobs(None, block.clone());
        let cloned = RpcBlock::block_cloned(&rpc);
        assert_eq!(cloned.canonical_root(), block.canonical_root());
    }

    // --- Pre-Deneb block with blobs ---

    #[test]
    fn pre_deneb_block_ignores_blobs() {
        let block = make_block();
        let rpc = RpcBlock::new(None, block, None).unwrap();
        assert!(rpc.blobs().is_none());
    }
}
