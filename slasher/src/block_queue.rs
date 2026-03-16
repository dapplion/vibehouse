use parking_lot::Mutex;
use std::collections::HashSet;
use types::SignedBeaconBlockHeader;

#[derive(Debug, Default)]
pub struct BlockQueue {
    blocks: Mutex<HashSet<SignedBeaconBlockHeader>>,
}

impl BlockQueue {
    pub fn queue(&self, block_header: SignedBeaconBlockHeader) {
        self.blocks.lock().insert(block_header);
    }

    pub fn dequeue(&self) -> HashSet<SignedBeaconBlockHeader> {
        let mut blocks = self.blocks.lock();
        std::mem::take(&mut *blocks)
    }

    pub fn len(&self) -> usize {
        self.blocks.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{BeaconBlockHeader, FixedBytesExtended, Hash256, Signature, Slot};

    fn make_header(slot: u64, proposer_index: u64) -> SignedBeaconBlockHeader {
        SignedBeaconBlockHeader {
            message: BeaconBlockHeader {
                slot: Slot::new(slot),
                proposer_index,
                parent_root: Hash256::zero(),
                state_root: Hash256::zero(),
                body_root: Hash256::zero(),
            },
            signature: Signature::empty(),
        }
    }

    #[test]
    fn empty_queue() {
        let q = BlockQueue::default();
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn queue_single_block() {
        let q = BlockQueue::default();
        q.queue(make_header(1, 0));
        assert_eq!(q.len(), 1);
        assert!(!q.is_empty());
    }

    #[test]
    fn queue_duplicate_block_deduplicates() {
        let q = BlockQueue::default();
        let header = make_header(1, 0);
        q.queue(header.clone());
        q.queue(header);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn queue_different_blocks() {
        let q = BlockQueue::default();
        q.queue(make_header(1, 0));
        q.queue(make_header(2, 0));
        q.queue(make_header(1, 1));
        assert_eq!(q.len(), 3);
    }

    #[test]
    fn dequeue_clears_queue() {
        let q = BlockQueue::default();
        q.queue(make_header(1, 0));
        q.queue(make_header(2, 0));

        let dequeued = q.dequeue();
        assert_eq!(dequeued.len(), 2);
        assert!(q.is_empty());
    }

    #[test]
    fn dequeue_empty_returns_empty_set() {
        let q = BlockQueue::default();
        let dequeued = q.dequeue();
        assert!(dequeued.is_empty());
    }

    #[test]
    fn queue_after_dequeue() {
        let q = BlockQueue::default();
        q.queue(make_header(1, 0));
        let _ = q.dequeue();

        q.queue(make_header(2, 0));
        assert_eq!(q.len(), 1);
        let dequeued = q.dequeue();
        assert_eq!(dequeued.len(), 1);
    }
}
