use std::sync::Arc;
use types::{BlobSidecar, EthSpec, ForkContext, Hash256, blob_sidecar::BlobIdentifier};
use vibehouse_network::rpc::methods::BlobsByRootRequest;

use super::{ActiveRequestItems, LookupVerifyError};

#[derive(Debug, Clone)]
pub(crate) struct BlobsByRootSingleBlockRequest {
    pub block_root: Hash256,
    pub indices: Vec<u64>,
}

impl BlobsByRootSingleBlockRequest {
    pub(crate) fn into_request(self, spec: &ForkContext) -> Result<BlobsByRootRequest, String> {
        BlobsByRootRequest::new(
            self.indices
                .into_iter()
                .map(|index| BlobIdentifier {
                    block_root: self.block_root,
                    index,
                })
                .collect(),
            spec,
        )
    }
}

pub(crate) struct BlobsByRootRequestItems<E: EthSpec> {
    request: BlobsByRootSingleBlockRequest,
    items: Vec<Arc<BlobSidecar<E>>>,
}

impl<E: EthSpec> BlobsByRootRequestItems<E> {
    pub(crate) fn new(request: BlobsByRootSingleBlockRequest) -> Self {
        Self {
            request,
            items: vec![],
        }
    }
}

impl<E: EthSpec> ActiveRequestItems for BlobsByRootRequestItems<E> {
    type Item = Arc<BlobSidecar<E>>;

    /// Appends a chunk to this multi-item request. If all expected chunks are received, this
    /// method returns `Some`, resolving the request before the stream terminator.
    /// The active request SHOULD be dropped after `add_response` returns an error
    fn add(&mut self, blob: Self::Item) -> Result<bool, LookupVerifyError> {
        let block_root = blob.block_root();
        if self.request.block_root != block_root {
            return Err(LookupVerifyError::UnrequestedBlockRoot(block_root));
        }
        if !blob.verify_blob_sidecar_inclusion_proof() {
            return Err(LookupVerifyError::InvalidInclusionProof);
        }
        if !self.request.indices.contains(&blob.index) {
            return Err(LookupVerifyError::UnrequestedIndex(blob.index));
        }
        if self.items.iter().any(|b| b.index == blob.index) {
            return Err(LookupVerifyError::DuplicatedData(blob.slot(), blob.index));
        }

        self.items.push(blob);

        Ok(self.items.len() >= self.request.indices.len())
    }

    fn consume(&mut self) -> Vec<Self::Item> {
        std::mem::take(&mut self.items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{BlobSidecar, FixedBytesExtended, MinimalEthSpec};

    type E = MinimalEthSpec;

    #[test]
    fn reject_blob_with_wrong_block_root() {
        let blob = Arc::new(BlobSidecar::<E>::empty());
        let actual_root = blob.block_root();
        let wrong_root = Hash256::repeat_byte(0xff);
        let mut req = BlobsByRootRequestItems::new(BlobsByRootSingleBlockRequest {
            block_root: wrong_root,
            indices: vec![0],
        });
        assert_eq!(
            req.add(blob),
            Err(LookupVerifyError::UnrequestedBlockRoot(actual_root))
        );
    }

    #[test]
    fn consume_returns_empty_initially() {
        let mut req = BlobsByRootRequestItems::<E>::new(BlobsByRootSingleBlockRequest {
            block_root: Hash256::zero(),
            indices: vec![0],
        });
        assert!(req.consume().is_empty());
    }
}
