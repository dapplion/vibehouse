use beacon_chain::block_verification_types::RpcBlock;
use educe::Educe;
use std::collections::HashSet;
use std::hash::Hash;
use std::marker::PhantomData;
use std::ops::Sub;
use std::time::Duration;
use std::time::Instant;
use strum::{Display, EnumIter, IntoStaticStr};
use types::Slot;
use types::{DataColumnSidecarList, Epoch, EthSpec};
use vibehouse_network::PeerId;
use vibehouse_network::rpc::methods::BlocksByRangeRequest;
use vibehouse_network::rpc::methods::DataColumnsByRangeRequest;
use vibehouse_network::service::api_types::Id;

use crate::sync::network_context::PeerGroup;

/// Batch states used as metrics labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum BatchMetricsState {
    AwaitingDownload,
    Downloading,
    AwaitingProcessing,
    Processing,
    AwaitingValidation,
    Failed,
}

pub type BatchId = Epoch;

/// Type of expected batch.
#[derive(Debug, Clone, Display)]
#[strum(serialize_all = "snake_case")]
pub enum ByRangeRequestType {
    BlocksAndColumns,
    BlocksAndBlobs,
    Blocks,
    Columns(HashSet<u64>),
}

/// Allows customisation of the above constants used in other sync methods such as BackFillSync.
pub trait BatchConfig {
    /// The maximum batch download attempts.
    fn max_batch_download_attempts() -> u8;
    /// The max batch processing attempts.
    fn max_batch_processing_attempts() -> u8;
    /// Hashing function of a batch's attempt. Used for scoring purposes.
    ///
    /// When a batch fails processing, it is possible that the batch is wrong (faulty or
    /// incomplete) or that a previous one is wrong. For this reason we need to re-download and
    /// re-process the batches awaiting validation and the current one. Consider this scenario:
    ///
    /// ```text
    /// BatchA BatchB BatchC BatchD
    /// -----X Empty  Empty  Y-----
    /// ```
    ///
    /// BatchA declares that it refers X, but BatchD declares that it's first block is Y. There is no
    /// way to know if BatchD is faulty/incomplete or if batches B and/or C are missing blocks. It is
    /// also possible that BatchA belongs to a different chain to the rest starting in some block
    /// midway in the batch's range. For this reason, the four batches would need to be re-downloaded
    /// and re-processed.
    ///
    /// If batchD was actually good, it will still register two processing attempts for the same set of
    /// blocks. In this case, we don't want to penalize the peer that provided the first version, since
    /// it's equal to the successfully processed one.
    ///
    /// The function `batch_attempt_hash` provides a way to compare two batch attempts without
    /// storing the full set of blocks.
    ///
    /// Note that simpler hashing functions considered in the past (hash of first block, hash of last
    /// block, number of received blocks) are not good enough to differentiate attempts. For this
    /// reason, we hash the complete set of blocks both in RangeSync and BackFillSync.
    fn batch_attempt_hash<D: Hash>(data: &D) -> u64;
}

#[derive(Debug)]
pub struct WrongState(pub(crate) String);

/// After batch operations, we use this to communicate whether a batch can continue or not
pub enum BatchOperationOutcome {
    Continue,
    Failed { blacklist: bool },
}

#[derive(Debug)]
pub enum BatchProcessingResult {
    Success,
    FaultyFailure,
    NonFaultyFailure,
}

#[derive(Educe)]
#[educe(Debug)]
/// A segment of a chain.
pub struct BatchInfo<E: EthSpec, B: BatchConfig, D: Hash> {
    /// Start slot of the batch.
    start_slot: Slot,
    /// End slot of the batch.
    end_slot: Slot,
    /// The `Attempts` that have been made and failed to send us this batch.
    failed_processing_attempts: Vec<Attempt<D>>,
    /// Number of processing attempts that have failed but we do not count.
    non_faulty_processing_attempts: u8,
    /// The number of download retries this batch has undergone due to a failed request.
    failed_download_attempts: Vec<Option<PeerId>>,
    /// State of the batch.
    state: BatchState<D>,
    /// Whether this batch contains all blocks or all blocks and blobs.
    batch_type: ByRangeRequestType,
    /// Pin the generic
    #[educe(Debug(ignore))]
    marker: std::marker::PhantomData<(E, B)>,
}

impl<E: EthSpec, B: BatchConfig, D: std::fmt::Debug + Hash> std::fmt::Display
    for BatchInfo<E, B, D>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Start Slot: {}, End Slot: {}, State: {}",
            self.start_slot, self.end_slot, self.state
        )
    }
}

#[derive(Display)]
/// Current state of a batch
pub enum BatchState<D: Hash> {
    /// The batch has failed either downloading or processing, but can be requested again.
    AwaitingDownload,
    /// The batch is being downloaded.
    Downloading(Id),
    /// The batch has been completely downloaded and is ready for processing.
    AwaitingProcessing(PeerGroup, D, Instant),
    /// The batch is being processed.
    Processing(Attempt<D>),
    /// The batch was successfully processed and is waiting to be validated.
    ///
    /// It is not sufficient to process a batch successfully to consider it correct. This is
    /// because batches could be erroneously empty, or incomplete. Therefore, a batch is considered
    /// valid, only if the next sequential batch imports at least a block.
    AwaitingValidation(Attempt<D>),
    /// Intermediate state for inner state handling.
    Poisoned,
    /// The batch has maxed out the allowed attempts for either downloading or processing. It
    /// cannot be recovered.
    Failed,
}

impl<D: Hash> BatchState<D> {
    /// Helper function for poisoning a state.
    pub fn poison(&mut self) -> BatchState<D> {
        std::mem::replace(self, BatchState::Poisoned)
    }

    /// Returns the metrics state for this batch.
    pub fn metrics_state(&self) -> BatchMetricsState {
        match self {
            BatchState::AwaitingDownload => BatchMetricsState::AwaitingDownload,
            BatchState::Downloading(_) => BatchMetricsState::Downloading,
            BatchState::AwaitingProcessing(..) => BatchMetricsState::AwaitingProcessing,
            BatchState::Processing(_) => BatchMetricsState::Processing,
            BatchState::AwaitingValidation(_) => BatchMetricsState::AwaitingValidation,
            BatchState::Poisoned | BatchState::Failed => BatchMetricsState::Failed,
        }
    }
}

impl<E: EthSpec, B: BatchConfig, D: Hash> BatchInfo<E, B, D> {
    /// Batches are downloaded excluding the first block of the epoch assuming it has already been
    /// downloaded.
    ///
    /// For example:
    ///
    /// Epoch boundary |                                   |
    ///  ... | 30 | 31 | 32 | 33 | 34 | ... | 61 | 62 | 63 | 64 | 65 |
    ///       Batch 1       |              Batch 2              |  Batch 3
    ///
    /// NOTE: Removed the shift by one for deneb because otherwise the last batch before the blob
    /// fork boundary will be of mixed type (all blocks and one last blockblob), and I don't want to
    /// deal with this for now.
    /// This means finalization might be slower in deneb
    pub fn new(start_epoch: &Epoch, num_of_epochs: u64, batch_type: ByRangeRequestType) -> Self {
        let start_slot = start_epoch.start_slot(E::slots_per_epoch());
        let end_slot = start_slot + num_of_epochs * E::slots_per_epoch();
        Self {
            start_slot,
            end_slot,
            failed_processing_attempts: Vec::new(),
            failed_download_attempts: Vec::new(),
            non_faulty_processing_attempts: 0,
            state: BatchState::<D>::AwaitingDownload,
            batch_type,
            marker: std::marker::PhantomData,
        }
    }

    /// Gives a list of peers from which this batch has had a failed download or processing
    /// attempt.
    pub fn failed_peers(&self) -> HashSet<PeerId> {
        let mut peers = HashSet::with_capacity(
            self.failed_processing_attempts.len() + self.failed_download_attempts.len(),
        );

        for attempt in &self.failed_processing_attempts {
            for peer in attempt.peer_group.all() {
                peers.insert(*peer);
            }
        }

        for peer in self.failed_download_attempts.iter().flatten() {
            peers.insert(*peer);
        }

        peers
    }

    /// Verifies if an incoming request id to this batch.
    pub fn is_expecting_request_id(&self, request_id: &Id) -> bool {
        if let BatchState::Downloading(expected_id) = &self.state {
            return expected_id == request_id;
        }
        false
    }

    /// Returns the peers that contributed to the current batch download.
    pub fn processing_peers(&self) -> Option<&PeerGroup> {
        match &self.state {
            BatchState::AwaitingDownload | BatchState::Failed | BatchState::Downloading(..) => None,
            BatchState::AwaitingProcessing(peer_group, _, _)
            | BatchState::Processing(Attempt { peer_group, .. })
            | BatchState::AwaitingValidation(Attempt { peer_group, .. }) => Some(peer_group),
            BatchState::Poisoned => unreachable!("Poisoned batch"),
        }
    }

    /// After different operations over a batch, this could be in a state that allows it to
    /// continue, or in failed state. When the batch has failed, we check if it did mainly due to
    /// processing failures. In this case the batch is considered failed and faulty.
    pub fn outcome(&self) -> BatchOperationOutcome {
        match self.state {
            BatchState::Poisoned => unreachable!("Poisoned batch"),
            BatchState::Failed => BatchOperationOutcome::Failed {
                blacklist: self.failed_processing_attempts.len()
                    > self.failed_download_attempts.len(),
            },
            _ => BatchOperationOutcome::Continue,
        }
    }

    pub fn state(&self) -> &BatchState<D> {
        &self.state
    }

    pub fn attempts(&self) -> &[Attempt<D>] {
        &self.failed_processing_attempts
    }

    /// Marks the batch as ready to be processed if the data columns are in the range. The number of
    /// received columns is returned, or the wrong batch end on failure
    #[must_use = "Batch may have failed"]
    pub fn download_completed(
        &mut self,
        data_columns: D,
        peer_group: PeerGroup,
    ) -> Result<(), WrongState> {
        match self.state.poison() {
            BatchState::Downloading(_) => {
                self.state =
                    BatchState::AwaitingProcessing(peer_group, data_columns, Instant::now());
                Ok(())
            }
            BatchState::Poisoned => unreachable!("Poisoned batch"),
            other => {
                self.state = other;
                Err(WrongState(format!(
                    "Download completed for batch in wrong state {:?}",
                    self.state
                )))
            }
        }
    }

    /// Mark the batch as failed and return whether we can attempt a re-download.
    ///
    /// This can happen if a peer disconnects or some error occurred that was not the peers fault.
    /// The `peer` parameter, when set to None, does not increment the failed attempts of
    /// this batch and register the peer, rather attempts a re-download.
    #[must_use = "Batch may have failed"]
    pub fn download_failed(
        &mut self,
        peer: Option<PeerId>,
    ) -> Result<BatchOperationOutcome, WrongState> {
        match self.state.poison() {
            BatchState::Downloading(_) => {
                // register the attempt and check if the batch can be tried again
                self.failed_download_attempts.push(peer);

                self.state = if self.failed_download_attempts.len()
                    >= B::max_batch_download_attempts() as usize
                {
                    BatchState::Failed
                } else {
                    // drop the blocks
                    BatchState::AwaitingDownload
                };
                Ok(self.outcome())
            }
            BatchState::Poisoned => unreachable!("Poisoned batch"),
            other => {
                self.state = other;
                Err(WrongState(format!(
                    "Download failed for batch in wrong state {:?}",
                    self.state
                )))
            }
        }
    }

    /// Change the batch state from `Self::Downloading` to `Self::AwaitingDownload` without
    /// registering a failed attempt.
    ///
    /// Note: must use this cautiously with some level of retry protection
    /// as not registering a failed attempt could lead to requesting in a loop.
    #[must_use = "Batch may have failed"]
    pub fn downloading_to_awaiting_download(
        &mut self,
    ) -> Result<BatchOperationOutcome, WrongState> {
        match self.state.poison() {
            BatchState::Downloading(_) => {
                self.state = BatchState::AwaitingDownload;
                Ok(self.outcome())
            }
            BatchState::Poisoned => unreachable!("Poisoned batch"),
            other => {
                self.state = other;
                Err(WrongState(format!(
                    "Download failed for batch in wrong state {:?}",
                    self.state
                )))
            }
        }
    }

    pub fn start_downloading(&mut self, request_id: Id) -> Result<(), WrongState> {
        match self.state.poison() {
            BatchState::AwaitingDownload => {
                self.state = BatchState::Downloading(request_id);
                Ok(())
            }
            BatchState::Poisoned => unreachable!("Poisoned batch"),
            other => {
                self.state = other;
                Err(WrongState(format!(
                    "Starting download for batch in wrong state {:?}",
                    self.state
                )))
            }
        }
    }

    pub fn start_processing(&mut self) -> Result<(D, Duration), WrongState> {
        match self.state.poison() {
            BatchState::AwaitingProcessing(peer_group, data_columns, start_instant) => {
                self.state = BatchState::Processing(Attempt::new::<B>(peer_group, &data_columns));
                Ok((data_columns, start_instant.elapsed()))
            }
            BatchState::Poisoned => unreachable!("Poisoned batch"),
            other => {
                self.state = other;
                Err(WrongState(format!(
                    "Starting processing batch in wrong state {:?}",
                    self.state
                )))
            }
        }
    }

    pub fn processing_completed(
        &mut self,
        processing_result: BatchProcessingResult,
    ) -> Result<BatchOperationOutcome, WrongState> {
        match self.state.poison() {
            BatchState::Processing(attempt) => {
                self.state = match processing_result {
                    BatchProcessingResult::Success => BatchState::AwaitingValidation(attempt),
                    BatchProcessingResult::FaultyFailure => {
                        // register the failed attempt
                        self.failed_processing_attempts.push(attempt);

                        // check if the batch can be downloaded again
                        if self.failed_processing_attempts.len()
                            >= B::max_batch_processing_attempts() as usize
                        {
                            BatchState::Failed
                        } else {
                            BatchState::AwaitingDownload
                        }
                    }
                    BatchProcessingResult::NonFaultyFailure => {
                        self.non_faulty_processing_attempts =
                            self.non_faulty_processing_attempts.saturating_add(1);
                        BatchState::AwaitingDownload
                    }
                };
                Ok(self.outcome())
            }
            BatchState::Poisoned => unreachable!("Poisoned batch"),
            other => {
                self.state = other;
                Err(WrongState(format!(
                    "Procesing completed for batch in wrong state: {:?}",
                    self.state
                )))
            }
        }
    }

    #[must_use = "Batch may have failed"]
    pub fn validation_failed(&mut self) -> Result<BatchOperationOutcome, WrongState> {
        match self.state.poison() {
            BatchState::AwaitingValidation(attempt) => {
                self.failed_processing_attempts.push(attempt);

                // check if the batch can be downloaded again
                self.state = if self.failed_processing_attempts.len()
                    >= B::max_batch_processing_attempts() as usize
                {
                    BatchState::Failed
                } else {
                    BatchState::AwaitingDownload
                };
                Ok(self.outcome())
            }
            BatchState::Poisoned => unreachable!("Poisoned batch"),
            other => {
                self.state = other;
                Err(WrongState(format!(
                    "Validation failed for batch in wrong state: {:?}",
                    self.state
                )))
            }
        }
    }

    // Visualizes the state of this batch using state::visualize()
    pub fn visualize(&self) -> char {
        self.state.visualize()
    }
}

// BatchInfo implementations for RangeSync
impl<E: EthSpec, B: BatchConfig> BatchInfo<E, B, Vec<RpcBlock<E>>> {
    /// Returns a BlocksByRange request associated with the batch.
    pub fn to_blocks_by_range_request(&self) -> (BlocksByRangeRequest, ByRangeRequestType) {
        (
            BlocksByRangeRequest::new(
                self.start_slot.into(),
                self.end_slot.sub(self.start_slot).into(),
            ),
            self.batch_type.clone(),
        )
    }

    /// Returns the count of stored pending blocks if in awaiting processing state
    pub fn pending_blocks(&self) -> usize {
        match &self.state {
            BatchState::AwaitingProcessing(_, blocks, _) => blocks.len(),
            BatchState::AwaitingDownload
            | BatchState::Downloading { .. }
            | BatchState::Processing { .. }
            | BatchState::AwaitingValidation { .. }
            | BatchState::Poisoned
            | BatchState::Failed => 0,
        }
    }
}

// BatchInfo implementation for CustodyBackFillSync
impl<E: EthSpec, B: BatchConfig> BatchInfo<E, B, DataColumnSidecarList<E>> {
    /// Returns a DataColumnsByRange request associated with the batch.
    pub fn to_data_columns_by_range_request(
        &self,
    ) -> Result<DataColumnsByRangeRequest, WrongState> {
        match &self.batch_type {
            ByRangeRequestType::Columns(columns) => Ok(DataColumnsByRangeRequest {
                start_slot: self.start_slot.into(),
                count: self.end_slot.sub(self.start_slot).into(),
                columns: columns.iter().copied().collect(),
            }),
            _ => Err(WrongState(
                "Custody backfill sync can only make data columns by range requests.".to_string(),
            )),
        }
    }
}

#[derive(Debug)]
pub struct Attempt<D: Hash> {
    /// The peers that contributed to this attempt.
    pub peer_group: PeerGroup,
    /// The hash of the blocks of the attempt.
    pub hash: u64,
    /// Pin the generic.
    marker: PhantomData<D>,
}

impl<D: Hash> Attempt<D> {
    fn new<B: BatchConfig>(peer_group: PeerGroup, data: &D) -> Self {
        let hash = B::batch_attempt_hash(data);
        Attempt {
            peer_group,
            hash,
            marker: PhantomData,
        }
    }
}

impl<D: Hash> std::fmt::Debug for BatchState<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BatchState::Processing(Attempt { peer_group, .. }) => {
                write!(f, "Processing({:?})", peer_group)
            }
            BatchState::AwaitingValidation(Attempt { peer_group, .. }) => {
                write!(f, "AwaitingValidation({:?})", peer_group)
            }
            BatchState::AwaitingDownload => f.write_str("AwaitingDownload"),
            BatchState::Failed => f.write_str("Failed"),
            BatchState::AwaitingProcessing(peer_group, ..) => {
                write!(f, "AwaitingProcessing({:?})", peer_group)
            }
            BatchState::Downloading(request_id) => {
                write!(f, "Downloading({})", request_id)
            }
            BatchState::Poisoned => f.write_str("Poisoned"),
        }
    }
}

impl<D: Hash> BatchState<D> {
    /// Creates a character representation/visualization for the batch state to display in logs for quicker and
    /// easier recognition
    fn visualize(&self) -> char {
        match self {
            BatchState::Downloading(..) => 'D',
            BatchState::Processing(_) => 'P',
            BatchState::AwaitingValidation(_) => 'v',
            BatchState::AwaitingDownload => 'd',
            BatchState::Failed => 'F',
            BatchState::AwaitingProcessing(..) => 'p',
            BatchState::Poisoned => 'X',
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beacon_chain::block_verification_types::RpcBlock;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;
    use types::MinimalEthSpec;

    type E = MinimalEthSpec;

    /// Test BatchConfig with small limits for easy testing.
    struct TestBatchConfig;

    impl BatchConfig for TestBatchConfig {
        fn max_batch_download_attempts() -> u8 {
            3
        }
        fn max_batch_processing_attempts() -> u8 {
            3
        }
        fn batch_attempt_hash<D: Hash>(data: &D) -> u64 {
            let mut hasher = DefaultHasher::new();
            data.hash(&mut hasher);
            hasher.finish()
        }
    }

    /// Simple data type for most state machine tests.
    type SimpleBatch = BatchInfo<E, TestBatchConfig, Vec<u8>>;
    /// RpcBlock-based type for methods that require it.
    type RpcBatch = BatchInfo<E, TestBatchConfig, Vec<RpcBlock<E>>>;

    fn make_batch() -> SimpleBatch {
        BatchInfo::new(&Epoch::new(0), 1, ByRangeRequestType::Blocks)
    }

    fn make_rpc_batch() -> RpcBatch {
        BatchInfo::new(&Epoch::new(0), 1, ByRangeRequestType::Blocks)
    }

    fn peer(id: u8) -> PeerId {
        let mut bytes = [0u8; 38];
        bytes[0] = 0; // multihash identity prefix
        bytes[1] = 36; // length
        bytes[2..].copy_from_slice(&[id; 36]);
        PeerId::from_bytes(&bytes).unwrap()
    }

    fn peer_group(id: u8) -> PeerGroup {
        PeerGroup::from_single(peer(id))
    }

    // ── Construction ──

    #[test]
    fn new_batch_starts_awaiting_download() {
        let batch = make_batch();
        assert!(matches!(batch.state(), BatchState::AwaitingDownload));
        assert_eq!(batch.visualize(), 'd');
    }

    #[test]
    fn new_batch_has_correct_slot_range() {
        let batch: RpcBatch = BatchInfo::new(&Epoch::new(2), 1, ByRangeRequestType::Blocks);
        // MinimalEthSpec: 8 slots per epoch
        // start_slot = 2 * 8 = 16, end_slot = 16 + 8 = 24
        let (req, _) = batch.to_blocks_by_range_request();
        assert_eq!(*req.start_slot(), 16);
        assert_eq!(*req.count(), 8);
    }

    #[test]
    fn new_batch_multi_epoch() {
        let batch: RpcBatch = BatchInfo::new(&Epoch::new(0), 3, ByRangeRequestType::Blocks);
        let (req, _) = batch.to_blocks_by_range_request();
        assert_eq!(*req.start_slot(), 0);
        assert_eq!(*req.count(), 24); // 3 * 8
    }

    #[test]
    fn new_batch_no_failed_peers() {
        let batch = make_batch();
        assert!(batch.failed_peers().is_empty());
    }

    // ── Happy path: download → process → validate ──

    #[test]
    fn happy_path_full_lifecycle() {
        let mut batch = make_batch();
        let pg = peer_group(1);

        // Start download
        batch.start_downloading(42).unwrap();
        assert!(matches!(batch.state(), BatchState::Downloading(42)));
        assert_eq!(batch.visualize(), 'D');
        assert!(batch.is_expecting_request_id(&42));
        assert!(!batch.is_expecting_request_id(&99));

        // Complete download
        batch.download_completed(vec![1, 2, 3], pg).unwrap();
        assert!(matches!(batch.state(), BatchState::AwaitingProcessing(..)));
        assert_eq!(batch.visualize(), 'p');
        assert!(batch.processing_peers().is_some());

        // Start processing
        let (data, _duration) = batch.start_processing().unwrap();
        assert_eq!(data, vec![1, 2, 3]);
        assert!(matches!(batch.state(), BatchState::Processing(_)));
        assert_eq!(batch.visualize(), 'P');

        // Processing success
        let outcome = batch
            .processing_completed(BatchProcessingResult::Success)
            .unwrap();
        assert!(matches!(outcome, BatchOperationOutcome::Continue));
        assert!(matches!(batch.state(), BatchState::AwaitingValidation(_)));
        assert_eq!(batch.visualize(), 'v');
    }

    // ── Download failures ──

    #[test]
    fn download_failure_retries_up_to_limit() {
        let mut batch = make_batch();
        let p = peer(1);

        // First failure
        batch.start_downloading(1).unwrap();
        let outcome = batch.download_failed(Some(p)).unwrap();
        assert!(matches!(outcome, BatchOperationOutcome::Continue));
        assert!(matches!(batch.state(), BatchState::AwaitingDownload));

        // Second failure
        batch.start_downloading(2).unwrap();
        let outcome = batch.download_failed(Some(peer(2))).unwrap();
        assert!(matches!(outcome, BatchOperationOutcome::Continue));

        // Third failure → max_batch_download_attempts (3) reached → Failed
        batch.start_downloading(3).unwrap();
        let outcome = batch.download_failed(Some(peer(3))).unwrap();
        assert!(matches!(
            outcome,
            BatchOperationOutcome::Failed { blacklist: false }
        ));
        assert!(matches!(batch.state(), BatchState::Failed));
        assert_eq!(batch.visualize(), 'F');
    }

    #[test]
    fn download_failure_none_peer_still_counts() {
        let mut batch = make_batch();

        for i in 0..3u32 {
            batch.start_downloading(i).unwrap();
            batch.download_failed(None).unwrap();
        }
        assert!(matches!(batch.state(), BatchState::Failed));
    }

    #[test]
    fn download_failed_peers_tracked() {
        let mut batch = make_batch();
        let p1 = peer(1);
        let p2 = peer(2);

        batch.start_downloading(1).unwrap();
        batch.download_failed(Some(p1)).unwrap();

        batch.start_downloading(2).unwrap();
        batch.download_failed(Some(p2)).unwrap();

        let failed = batch.failed_peers();
        assert!(failed.contains(&p1));
        assert!(failed.contains(&p2));
        assert_eq!(failed.len(), 2);
    }

    // ── downloading_to_awaiting_download (no failed attempt registered) ──

    #[test]
    fn downloading_to_awaiting_download_no_failure_registered() {
        let mut batch = make_batch();
        batch.start_downloading(1).unwrap();

        let outcome = batch.downloading_to_awaiting_download().unwrap();
        assert!(matches!(outcome, BatchOperationOutcome::Continue));
        assert!(matches!(batch.state(), BatchState::AwaitingDownload));

        // No failed attempts registered
        assert!(batch.failed_peers().is_empty());
    }

    // ── Processing failures ──

    #[test]
    fn faulty_processing_failure_retries_up_to_limit() {
        let mut batch = make_batch();

        for i in 0..3u32 {
            batch.start_downloading(i).unwrap();
            batch
                .download_completed(vec![i as u8], peer_group(1))
                .unwrap();
            batch.start_processing().unwrap();
            let outcome = batch
                .processing_completed(BatchProcessingResult::FaultyFailure)
                .unwrap();

            if i < 2 {
                assert!(matches!(outcome, BatchOperationOutcome::Continue));
                assert!(matches!(batch.state(), BatchState::AwaitingDownload));
            } else {
                // 3rd faulty failure → Failed, blacklist=true (more processing than download failures)
                assert!(matches!(
                    outcome,
                    BatchOperationOutcome::Failed { blacklist: true }
                ));
                assert!(matches!(batch.state(), BatchState::Failed));
            }
        }
    }

    #[test]
    fn non_faulty_processing_failure_does_not_count_toward_limit() {
        let mut batch = make_batch();

        // 10 non-faulty failures should never exhaust the limit
        for i in 0..10u32 {
            batch.start_downloading(i).unwrap();
            batch
                .download_completed(vec![i as u8], peer_group(1))
                .unwrap();
            batch.start_processing().unwrap();
            let outcome = batch
                .processing_completed(BatchProcessingResult::NonFaultyFailure)
                .unwrap();
            assert!(matches!(outcome, BatchOperationOutcome::Continue));
            assert!(matches!(batch.state(), BatchState::AwaitingDownload));
        }
    }

    // ── Validation failures ──

    #[test]
    fn validation_failure_retries_up_to_limit() {
        let mut batch = make_batch();

        for i in 0..3u32 {
            batch.start_downloading(i).unwrap();
            batch
                .download_completed(vec![i as u8], peer_group(1))
                .unwrap();
            batch.start_processing().unwrap();
            batch
                .processing_completed(BatchProcessingResult::Success)
                .unwrap();

            let outcome = batch.validation_failed().unwrap();
            if i < 2 {
                assert!(matches!(outcome, BatchOperationOutcome::Continue));
            } else {
                assert!(matches!(
                    outcome,
                    BatchOperationOutcome::Failed { blacklist: true }
                ));
            }
        }
    }

    // ── Wrong state errors ──

    #[test]
    fn start_downloading_wrong_state() {
        let mut batch = make_batch();
        batch.start_downloading(1).unwrap();
        // Already downloading, can't start again
        assert!(batch.start_downloading(2).is_err());
    }

    #[test]
    fn download_completed_wrong_state() {
        let mut batch = make_batch();
        // Not downloading yet
        assert!(batch.download_completed(vec![], peer_group(1)).is_err());
    }

    #[test]
    fn download_failed_wrong_state() {
        let mut batch = make_batch();
        // Not downloading
        assert!(batch.download_failed(Some(peer(1))).is_err());
    }

    #[test]
    fn start_processing_wrong_state() {
        let mut batch = make_batch();
        // Still in AwaitingDownload
        assert!(batch.start_processing().is_err());
    }

    #[test]
    fn processing_completed_wrong_state() {
        let mut batch = make_batch();
        assert!(
            batch
                .processing_completed(BatchProcessingResult::Success)
                .is_err()
        );
    }

    #[test]
    fn validation_failed_wrong_state() {
        let mut batch = make_batch();
        assert!(batch.validation_failed().is_err());
    }

    #[test]
    fn downloading_to_awaiting_wrong_state() {
        let mut batch = make_batch();
        // Not in Downloading state
        assert!(batch.downloading_to_awaiting_download().is_err());
    }

    // ── processing_peers ──

    #[test]
    fn processing_peers_none_when_awaiting_download() {
        let batch = make_batch();
        assert!(batch.processing_peers().is_none());
    }

    #[test]
    fn processing_peers_none_when_downloading() {
        let mut batch = make_batch();
        batch.start_downloading(1).unwrap();
        assert!(batch.processing_peers().is_none());
    }

    #[test]
    fn processing_peers_set_during_processing() {
        let mut batch = make_batch();
        batch.start_downloading(1).unwrap();
        batch.download_completed(vec![1], peer_group(1)).unwrap();
        assert!(batch.processing_peers().is_some());

        batch.start_processing().unwrap();
        assert!(batch.processing_peers().is_some());
    }

    // ── is_expecting_request_id ──

    #[test]
    fn not_expecting_when_not_downloading() {
        let batch = make_batch();
        assert!(!batch.is_expecting_request_id(&1));
    }

    // ── pending_blocks ──

    #[test]
    fn pending_blocks_zero_in_awaiting_download() {
        let batch = make_rpc_batch();
        assert_eq!(batch.pending_blocks(), 0);
    }

    #[test]
    fn pending_blocks_zero_when_downloading() {
        let mut batch = make_rpc_batch();
        batch.start_downloading(1).unwrap();
        assert_eq!(batch.pending_blocks(), 0);
    }

    // ── metrics_state ──

    #[test]
    fn metrics_state_mapping() {
        let mut batch = make_batch();
        assert!(matches!(
            batch.state().metrics_state(),
            BatchMetricsState::AwaitingDownload
        ));

        batch.start_downloading(1).unwrap();
        assert!(matches!(
            batch.state().metrics_state(),
            BatchMetricsState::Downloading
        ));

        batch.download_completed(vec![1], peer_group(1)).unwrap();
        assert!(matches!(
            batch.state().metrics_state(),
            BatchMetricsState::AwaitingProcessing
        ));

        batch.start_processing().unwrap();
        assert!(matches!(
            batch.state().metrics_state(),
            BatchMetricsState::Processing
        ));

        batch
            .processing_completed(BatchProcessingResult::Success)
            .unwrap();
        assert!(matches!(
            batch.state().metrics_state(),
            BatchMetricsState::AwaitingValidation
        ));
    }

    #[test]
    fn metrics_state_failed() {
        let mut batch = make_batch();
        for i in 0..3u32 {
            batch.start_downloading(i).unwrap();
            batch.download_failed(Some(peer(1))).unwrap();
        }
        assert!(matches!(
            batch.state().metrics_state(),
            BatchMetricsState::Failed
        ));
    }

    // ── attempts tracking ──

    #[test]
    fn attempts_empty_initially() {
        let batch = make_batch();
        assert!(batch.attempts().is_empty());
    }

    #[test]
    fn attempts_tracked_on_faulty_failure() {
        let mut batch = make_batch();

        batch.start_downloading(1).unwrap();
        batch.download_completed(vec![42], peer_group(1)).unwrap();
        batch.start_processing().unwrap();
        batch
            .processing_completed(BatchProcessingResult::FaultyFailure)
            .unwrap();

        assert_eq!(batch.attempts().len(), 1);
        assert!(batch.attempts()[0].peer_group.all().any(|p| *p == peer(1)));
    }

    #[test]
    fn attempt_hash_differs_for_different_data() {
        let mut batch1 = make_batch();
        let mut batch2 = make_batch();

        // batch1: data = [1, 2, 3]
        batch1.start_downloading(1).unwrap();
        batch1
            .download_completed(vec![1, 2, 3], peer_group(1))
            .unwrap();
        batch1.start_processing().unwrap();
        batch1
            .processing_completed(BatchProcessingResult::FaultyFailure)
            .unwrap();

        // batch2: data = [4, 5, 6]
        batch2.start_downloading(1).unwrap();
        batch2
            .download_completed(vec![4, 5, 6], peer_group(1))
            .unwrap();
        batch2.start_processing().unwrap();
        batch2
            .processing_completed(BatchProcessingResult::FaultyFailure)
            .unwrap();

        assert_ne!(batch1.attempts()[0].hash, batch2.attempts()[0].hash);
    }

    // ── outcome blacklist logic ──

    #[test]
    fn outcome_blacklist_when_more_processing_failures() {
        let mut batch = make_batch();
        // 3 processing failures, 0 download failures → blacklist
        for i in 0..3u32 {
            batch.start_downloading(i).unwrap();
            batch
                .download_completed(vec![i as u8], peer_group(1))
                .unwrap();
            batch.start_processing().unwrap();
            batch
                .processing_completed(BatchProcessingResult::FaultyFailure)
                .unwrap();
        }
        assert!(matches!(
            batch.outcome(),
            BatchOperationOutcome::Failed { blacklist: true }
        ));
    }

    #[test]
    fn outcome_no_blacklist_when_more_download_failures() {
        let mut batch = make_batch();

        // 3 download failures, 0 processing failures → no blacklist
        for i in 0..3u32 {
            batch.start_downloading(i).unwrap();
            batch.download_failed(Some(peer(i as u8))).unwrap();
        }
        assert!(matches!(
            batch.outcome(),
            BatchOperationOutcome::Failed { blacklist: false }
        ));
    }

    // ── Display / Debug ──

    #[test]
    fn batch_display_includes_slot_range() {
        let batch = make_batch();
        let display = format!("{}", batch);
        assert!(display.contains("Start Slot: 0"));
        assert!(display.contains("End Slot: 8"));
    }

    // ── Mixed failure scenarios ──

    #[test]
    fn mixed_download_and_processing_failures() {
        let mut batch = make_batch();

        // 1 download failure
        batch.start_downloading(0).unwrap();
        batch.download_failed(Some(peer(1))).unwrap();

        // 1 processing failure
        batch.start_downloading(1).unwrap();
        batch.download_completed(vec![1], peer_group(1)).unwrap();
        batch.start_processing().unwrap();
        batch
            .processing_completed(BatchProcessingResult::FaultyFailure)
            .unwrap();

        // 1 more processing failure
        batch.start_downloading(2).unwrap();
        batch.download_completed(vec![2], peer_group(1)).unwrap();
        batch.start_processing().unwrap();
        batch
            .processing_completed(BatchProcessingResult::FaultyFailure)
            .unwrap();

        // 1 more processing failure → 3 total → Failed
        batch.start_downloading(3).unwrap();
        batch.download_completed(vec![3], peer_group(1)).unwrap();
        batch.start_processing().unwrap();
        let outcome = batch
            .processing_completed(BatchProcessingResult::FaultyFailure)
            .unwrap();

        // 3 processing > 1 download → blacklist
        assert!(matches!(
            outcome,
            BatchOperationOutcome::Failed { blacklist: true }
        ));
    }

    #[test]
    fn failed_peers_includes_both_download_and_processing() {
        let mut batch = make_batch();
        let p1 = peer(1);
        let p2 = peer(2);

        // Download failure from p1
        batch.start_downloading(1).unwrap();
        batch.download_failed(Some(p1)).unwrap();

        // Processing failure from p2
        batch.start_downloading(2).unwrap();
        batch.download_completed(vec![1], peer_group(2)).unwrap();
        batch.start_processing().unwrap();
        batch
            .processing_completed(BatchProcessingResult::FaultyFailure)
            .unwrap();

        let failed = batch.failed_peers();
        assert!(failed.contains(&p1));
        assert!(failed.contains(&p2));
    }
}
