use eth2::types::FullPayloadContents;
use lru::LruCache;
use parking_lot::Mutex;
use std::num::NonZeroUsize;
use tree_hash::TreeHash;
use types::non_zero_usize::new_non_zero_usize;
use types::{EthSpec, Hash256};

pub const DEFAULT_PAYLOAD_CACHE_SIZE: NonZeroUsize = new_non_zero_usize(10);

/// A cache mapping execution payloads by tree hash roots.
pub struct PayloadCache<E: EthSpec> {
    payloads: Mutex<LruCache<PayloadCacheId, FullPayloadContents<E>>>,
}

#[derive(Hash, PartialEq, Eq)]
struct PayloadCacheId(Hash256);

impl<E: EthSpec> Default for PayloadCache<E> {
    fn default() -> Self {
        PayloadCache {
            payloads: Mutex::new(LruCache::new(DEFAULT_PAYLOAD_CACHE_SIZE)),
        }
    }
}

impl<E: EthSpec> PayloadCache<E> {
    pub fn put(&self, payload: FullPayloadContents<E>) -> Option<FullPayloadContents<E>> {
        let root = payload.payload_ref().tree_hash_root();
        self.payloads.lock().put(PayloadCacheId(root), payload)
    }

    pub fn pop(&self, root: &Hash256) -> Option<FullPayloadContents<E>> {
        self.payloads.lock().pop(&PayloadCacheId(*root))
    }

    pub fn get(&self, hash: &Hash256) -> Option<FullPayloadContents<E>> {
        self.payloads.lock().get(&PayloadCacheId(*hash)).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{
        ExecutionPayload, ExecutionPayloadBellatrix, ExecutionPayloadCapella,
        ExecutionPayloadDeneb, FixedBytesExtended, MainnetEthSpec,
    };

    type E = MainnetEthSpec;

    fn make_bellatrix_payload() -> FullPayloadContents<E> {
        FullPayloadContents::Payload(ExecutionPayload::Bellatrix(
            ExecutionPayloadBellatrix::default(),
        ))
    }

    fn make_capella_payload() -> FullPayloadContents<E> {
        FullPayloadContents::Payload(ExecutionPayload::Capella(ExecutionPayloadCapella::default()))
    }

    fn make_deneb_payload() -> FullPayloadContents<E> {
        FullPayloadContents::Payload(ExecutionPayload::Deneb(ExecutionPayloadDeneb::default()))
    }

    fn payload_root(payload: &FullPayloadContents<E>) -> Hash256 {
        payload.payload_ref().tree_hash_root()
    }

    #[test]
    fn put_and_get() {
        let cache = PayloadCache::<E>::default();
        let payload = make_bellatrix_payload();
        let root = payload_root(&payload);

        assert!(cache.get(&root).is_none(), "empty cache returns None");

        cache.put(payload);
        assert!(
            cache.get(&root).is_some(),
            "inserted payload is retrievable"
        );
    }

    #[test]
    fn put_returns_previous_for_same_key() {
        let cache = PayloadCache::<E>::default();
        let payload = make_bellatrix_payload();

        let prev = cache.put(payload.clone());
        assert!(prev.is_none(), "first insert returns None");

        let prev = cache.put(payload);
        assert!(prev.is_some(), "re-insert returns evicted payload");
    }

    #[test]
    fn pop_removes_entry() {
        let cache = PayloadCache::<E>::default();
        let payload = make_capella_payload();
        let root = payload_root(&payload);

        cache.put(payload);
        let popped = cache.pop(&root);
        assert!(popped.is_some(), "pop returns the payload");
        assert!(cache.get(&root).is_none(), "entry is gone after pop");
    }

    #[test]
    fn pop_nonexistent_returns_none() {
        let cache = PayloadCache::<E>::default();
        assert!(cache.pop(&Hash256::zero()).is_none());
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let cache = PayloadCache::<E>::default();
        assert!(cache.get(&Hash256::repeat_byte(0xff)).is_none());
    }

    #[test]
    fn get_does_not_remove() {
        let cache = PayloadCache::<E>::default();
        let payload = make_deneb_payload();
        let root = payload_root(&payload);

        cache.put(payload);
        assert!(cache.get(&root).is_some());
        assert!(cache.get(&root).is_some(), "get is non-destructive");
    }

    #[test]
    fn lru_eviction() {
        // DEFAULT_PAYLOAD_CACHE_SIZE is 10, insert 11 entries and verify the first is evicted.
        let cache = PayloadCache::<E>::default();
        let mut roots = Vec::new();

        for i in 0..11u64 {
            let inner = ExecutionPayloadBellatrix::<E> {
                gas_limit: i,
                ..Default::default()
            };
            let payload = ExecutionPayload::Bellatrix(inner);
            let contents = FullPayloadContents::Payload(payload);
            roots.push(contents.payload_ref().tree_hash_root());
            cache.put(contents);
        }

        assert!(
            cache.get(&roots[0]).is_none(),
            "oldest entry evicted by LRU"
        );
        assert!(
            cache.get(&roots[10]).is_some(),
            "newest entry still present"
        );
        assert!(
            cache.get(&roots[1]).is_some(),
            "second-oldest entry still present"
        );
    }
}
