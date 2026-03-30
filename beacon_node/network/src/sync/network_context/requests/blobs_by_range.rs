use super::{ActiveRequestItems, LookupVerifyError};
use std::sync::Arc;
use types::{BlobSidecar, EthSpec};
use vibehouse_network::rpc::methods::BlobsByRangeRequest;

/// Accumulates results of a blobs_by_range request. Only returns items after receiving the
/// stream termination.
pub(crate) struct BlobsByRangeRequestItems<E: EthSpec> {
    request: BlobsByRangeRequest,
    items: Vec<Arc<BlobSidecar<E>>>,
    max_blobs_per_block: u64,
}

impl<E: EthSpec> BlobsByRangeRequestItems<E> {
    pub(crate) fn new(request: BlobsByRangeRequest, max_blobs_per_block: u64) -> Self {
        Self {
            request,
            items: vec![],
            max_blobs_per_block,
        }
    }
}

impl<E: EthSpec> ActiveRequestItems for BlobsByRangeRequestItems<E> {
    type Item = Arc<BlobSidecar<E>>;

    fn add(&mut self, blob: Self::Item) -> Result<bool, LookupVerifyError> {
        if blob.slot() < self.request.start_slot
            || blob.slot() >= self.request.start_slot + self.request.count
        {
            return Err(LookupVerifyError::UnrequestedSlot(blob.slot()));
        }
        if blob.index >= self.max_blobs_per_block {
            return Err(LookupVerifyError::UnrequestedIndex(blob.index));
        }
        if !blob.verify_blob_sidecar_inclusion_proof() {
            return Err(LookupVerifyError::InvalidInclusionProof);
        }
        if self
            .items
            .iter()
            .any(|existing| existing.slot() == blob.slot() && existing.index == blob.index)
        {
            return Err(LookupVerifyError::DuplicatedData(blob.slot(), blob.index));
        }

        self.items.push(blob);

        // Skip check if blobs are ready as it's rare that all blocks have max blobs
        Ok(false)
    }

    fn consume(&mut self) -> Vec<Self::Item> {
        std::mem::take(&mut self.items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{BlobSidecar, MinimalEthSpec, Slot};

    type E = MinimalEthSpec;

    fn make_blob(slot: u64, index: u64) -> Arc<BlobSidecar<E>> {
        let mut blob = BlobSidecar::empty();
        blob.signed_block_header.message.slot = Slot::new(slot);
        blob.index = index;
        Arc::new(blob)
    }

    fn make_request(start_slot: u64, count: u64, max_blobs: u64) -> BlobsByRangeRequestItems<E> {
        BlobsByRangeRequestItems::new(BlobsByRangeRequest { start_slot, count }, max_blobs)
    }

    #[test]
    fn reject_blob_before_start_slot() {
        let mut req = make_request(10, 5, 6);
        assert_eq!(
            req.add(make_blob(9, 0)),
            Err(LookupVerifyError::UnrequestedSlot(Slot::new(9)))
        );
    }

    #[test]
    fn reject_blob_at_end_of_range() {
        let mut req = make_request(10, 5, 6);
        // Range is [10, 15), so slot 15 is out of range
        assert_eq!(
            req.add(make_blob(15, 0)),
            Err(LookupVerifyError::UnrequestedSlot(Slot::new(15)))
        );
    }

    #[test]
    fn reject_blob_well_beyond_range() {
        let mut req = make_request(10, 5, 6);
        assert_eq!(
            req.add(make_blob(100, 0)),
            Err(LookupVerifyError::UnrequestedSlot(Slot::new(100)))
        );
    }

    #[test]
    fn reject_blob_index_exceeding_max_blobs() {
        let mut req = make_request(10, 5, 6);
        // max_blobs_per_block = 6, so index 6 is invalid
        assert_eq!(
            req.add(make_blob(10, 6)),
            Err(LookupVerifyError::UnrequestedIndex(6))
        );
    }

    #[test]
    fn reject_blob_index_well_beyond_max() {
        let mut req = make_request(10, 5, 6);
        assert_eq!(
            req.add(make_blob(10, 100)),
            Err(LookupVerifyError::UnrequestedIndex(100))
        );
    }

    #[test]
    fn consume_returns_empty_initially() {
        let mut req = make_request(10, 5, 6);
        assert!(req.consume().is_empty());
    }
}
