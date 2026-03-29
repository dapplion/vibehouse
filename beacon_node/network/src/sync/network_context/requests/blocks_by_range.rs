use super::{ActiveRequestItems, LookupVerifyError};
use std::sync::Arc;
use types::{EthSpec, SignedBeaconBlock};
use vibehouse_network::rpc::BlocksByRangeRequest;

/// Accumulates results of a blocks_by_range request. Only returns items after receiving the
/// stream termination.
pub(crate) struct BlocksByRangeRequestItems<E: EthSpec> {
    request: BlocksByRangeRequest,
    items: Vec<Arc<SignedBeaconBlock<E>>>,
}

impl<E: EthSpec> BlocksByRangeRequestItems<E> {
    pub(crate) fn new(request: BlocksByRangeRequest) -> Self {
        Self {
            request,
            items: vec![],
        }
    }
}

impl<E: EthSpec> ActiveRequestItems for BlocksByRangeRequestItems<E> {
    type Item = Arc<SignedBeaconBlock<E>>;

    fn add(&mut self, block: Self::Item) -> Result<bool, LookupVerifyError> {
        if block.slot().as_u64() < *self.request.start_slot()
            || block.slot().as_u64() >= self.request.start_slot() + self.request.count()
        {
            return Err(LookupVerifyError::UnrequestedSlot(block.slot()));
        }
        if self
            .items
            .iter()
            .any(|existing| existing.slot() == block.slot())
        {
            // DuplicatedData is a common error for all components, default index to 0
            return Err(LookupVerifyError::DuplicatedData(block.slot(), 0));
        }

        self.items.push(block);

        Ok(self.items.len() >= *self.request.count() as usize)
    }

    fn consume(&mut self) -> Vec<Self::Item> {
        std::mem::take(&mut self.items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{BeaconBlock, MinimalEthSpec, Signature, Slot};

    type E = MinimalEthSpec;

    fn make_block(slot: u64) -> Arc<SignedBeaconBlock<E>> {
        let spec = E::default_spec();
        let mut block = BeaconBlock::empty(&spec);
        *block.slot_mut() = Slot::new(slot);
        Arc::new(SignedBeaconBlock::from_block(block, Signature::empty()))
    }

    fn make_request(start_slot: u64, count: u64) -> BlocksByRangeRequestItems<E> {
        BlocksByRangeRequestItems::new(BlocksByRangeRequest::new(start_slot, count))
    }

    #[test]
    fn add_valid_blocks_returns_complete_when_count_reached() {
        let mut req = make_request(10, 3);
        assert_eq!(req.add(make_block(10)), Ok(false));
        assert_eq!(req.add(make_block(11)), Ok(false));
        assert_eq!(req.add(make_block(12)), Ok(true));
    }

    #[test]
    fn add_single_block_request_completes_immediately() {
        let mut req = make_request(5, 1);
        assert_eq!(req.add(make_block(5)), Ok(true));
    }

    #[test]
    fn reject_block_before_start_slot() {
        let mut req = make_request(10, 3);
        assert_eq!(
            req.add(make_block(9)),
            Err(LookupVerifyError::UnrequestedSlot(Slot::new(9)))
        );
    }

    #[test]
    fn reject_block_at_end_of_range() {
        let mut req = make_request(10, 3);
        // Range is [10, 13), so slot 13 is out of range
        assert_eq!(
            req.add(make_block(13)),
            Err(LookupVerifyError::UnrequestedSlot(Slot::new(13)))
        );
    }

    #[test]
    fn reject_block_well_beyond_range() {
        let mut req = make_request(10, 3);
        assert_eq!(
            req.add(make_block(100)),
            Err(LookupVerifyError::UnrequestedSlot(Slot::new(100)))
        );
    }

    #[test]
    fn reject_duplicate_slot() {
        let mut req = make_request(10, 3);
        assert_eq!(req.add(make_block(10)), Ok(false));
        assert_eq!(
            req.add(make_block(10)),
            Err(LookupVerifyError::DuplicatedData(Slot::new(10), 0))
        );
    }

    #[test]
    fn consume_returns_accumulated_items() {
        let mut req = make_request(10, 3);
        req.add(make_block(10)).unwrap();
        req.add(make_block(11)).unwrap();
        let items = req.consume();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].slot(), Slot::new(10));
        assert_eq!(items[1].slot(), Slot::new(11));
    }

    #[test]
    fn consume_empties_internal_state() {
        let mut req = make_request(10, 3);
        req.add(make_block(10)).unwrap();
        req.consume();
        let items = req.consume();
        assert!(items.is_empty());
    }

    #[test]
    fn blocks_can_arrive_out_of_order() {
        let mut req = make_request(10, 3);
        assert_eq!(req.add(make_block(12)), Ok(false));
        assert_eq!(req.add(make_block(10)), Ok(false));
        assert_eq!(req.add(make_block(11)), Ok(true));
    }
}
