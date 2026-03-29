use beacon_chain::get_block_root;
use std::sync::Arc;
use types::{EthSpec, ForkContext, Hash256, SignedBeaconBlock};
use vibehouse_network::rpc::BlocksByRootRequest;

use super::{ActiveRequestItems, LookupVerifyError};

#[derive(Debug, Copy, Clone)]
pub(crate) struct BlocksByRootSingleRequest(pub Hash256);

impl BlocksByRootSingleRequest {
    pub(crate) fn into_request(
        self,
        fork_context: &ForkContext,
    ) -> Result<BlocksByRootRequest, String> {
        // This should always succeed (single block root), but we return a `Result` for safety.
        BlocksByRootRequest::new(vec![self.0], fork_context)
    }
}

pub(crate) struct BlocksByRootRequestItems<E: EthSpec> {
    request: BlocksByRootSingleRequest,
    items: Vec<Arc<SignedBeaconBlock<E>>>,
}

impl<E: EthSpec> BlocksByRootRequestItems<E> {
    pub(crate) fn new(request: BlocksByRootSingleRequest) -> Self {
        Self {
            request,
            items: vec![],
        }
    }
}

impl<E: EthSpec> ActiveRequestItems for BlocksByRootRequestItems<E> {
    type Item = Arc<SignedBeaconBlock<E>>;

    /// Append a response to the single chunk request. If the chunk is valid, the request is
    /// resolved immediately.
    /// The active request SHOULD be dropped after `add_response` returns an error
    fn add(&mut self, block: Self::Item) -> Result<bool, LookupVerifyError> {
        let block_root = get_block_root(&block);
        if self.request.0 != block_root {
            return Err(LookupVerifyError::UnrequestedBlockRoot(block_root));
        }

        self.items.push(block);
        // Always returns true, blocks by root expects a single response
        Ok(true)
    }

    fn consume(&mut self) -> Vec<Self::Item> {
        std::mem::take(&mut self.items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{BeaconBlock, MinimalEthSpec, Signature};

    type E = MinimalEthSpec;

    fn make_block_with_root() -> (Hash256, Arc<SignedBeaconBlock<E>>) {
        let spec = E::default_spec();
        let block = BeaconBlock::empty(&spec);
        let signed = SignedBeaconBlock::from_block(block, Signature::empty());
        let root = signed.canonical_root();
        (root, Arc::new(signed))
    }

    #[test]
    fn accept_block_with_matching_root() {
        let (root, block) = make_block_with_root();
        let mut req = BlocksByRootRequestItems::new(BlocksByRootSingleRequest(root));
        // Single block request completes immediately
        assert_eq!(req.add(block), Ok(true));
    }

    #[test]
    fn reject_block_with_wrong_root() {
        let (_, block) = make_block_with_root();
        let wrong_root = Hash256::repeat_byte(0xff);
        let mut req = BlocksByRootRequestItems::new(BlocksByRootSingleRequest(wrong_root));
        let actual_root = get_block_root(&block);
        assert_eq!(
            req.add(block),
            Err(LookupVerifyError::UnrequestedBlockRoot(actual_root))
        );
    }

    #[test]
    fn consume_returns_added_block() {
        let (root, block) = make_block_with_root();
        let mut req = BlocksByRootRequestItems::new(BlocksByRootSingleRequest(root));
        req.add(block).unwrap();
        let items = req.consume();
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn consume_empties_state() {
        let (root, block) = make_block_with_root();
        let mut req = BlocksByRootRequestItems::new(BlocksByRootSingleRequest(root));
        req.add(block).unwrap();
        req.consume();
        assert!(req.consume().is_empty());
    }
}
