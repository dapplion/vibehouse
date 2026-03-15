use crate::hdiff::{Error, HDiffBuffer};
use crate::metrics;
use lru::LruCache;
use std::num::NonZeroUsize;
use types::{BeaconState, ChainSpec, EthSpec, Slot};

/// Holds a combination of finalized states in two formats:
/// - `hdiff_buffers`: Format close to an SSZ serialized state for rapid application of diffs on top
///   of it
/// - `states`: Deserialized states for direct use or for rapid application of blocks (replay)
///
/// An example use: when requesting state data for consecutive slots, this cache allows the node to
/// apply diffs once on the first request, and latter just apply blocks one at a time.
#[derive(Debug)]
pub struct HistoricStateCache<E: EthSpec> {
    hdiff_buffers: LruCache<Slot, HDiffBuffer>,
    states: LruCache<Slot, BeaconState<E>>,
}

#[derive(Debug, Default)]
pub struct Metrics {
    pub num_hdiff: usize,
    pub num_state: usize,
    pub hdiff_byte_size: usize,
}

impl<E: EthSpec> HistoricStateCache<E> {
    pub fn new(hdiff_buffer_cache_size: NonZeroUsize, state_cache_size: NonZeroUsize) -> Self {
        Self {
            hdiff_buffers: LruCache::new(hdiff_buffer_cache_size),
            states: LruCache::new(state_cache_size),
        }
    }

    pub fn get_hdiff_buffer(&mut self, slot: Slot) -> Option<HDiffBuffer> {
        if let Some(buffer_ref) = self.hdiff_buffers.get(&slot) {
            let _timer = metrics::start_timer_vec(
                &metrics::BEACON_HDIFF_BUFFER_CLONE_TIME,
                metrics::COLD_METRIC,
            );
            Some(buffer_ref.clone())
        } else if let Some(state) = self.states.get(&slot) {
            let buffer = HDiffBuffer::from_state(state.clone());
            let _timer = metrics::start_timer_vec(
                &metrics::BEACON_HDIFF_BUFFER_CLONE_TIME,
                metrics::COLD_METRIC,
            );
            let cloned = buffer.clone();
            drop(_timer);
            self.hdiff_buffers.put(slot, cloned);
            Some(buffer)
        } else {
            None
        }
    }

    pub fn get_state(
        &mut self,
        slot: Slot,
        spec: &ChainSpec,
    ) -> Result<Option<BeaconState<E>>, Error> {
        if let Some(state) = self.states.get(&slot) {
            Ok(Some(state.clone()))
        } else if let Some(buffer) = self.hdiff_buffers.get(&slot) {
            let state = buffer.as_state(spec)?;
            self.states.put(slot, state.clone());
            Ok(Some(state))
        } else {
            Ok(None)
        }
    }

    pub fn put_state(&mut self, slot: Slot, state: BeaconState<E>) {
        self.states.put(slot, state);
    }

    pub fn put_hdiff_buffer(&mut self, slot: Slot, buffer: HDiffBuffer) {
        self.hdiff_buffers.put(slot, buffer);
    }

    pub fn put_both(&mut self, slot: Slot, state: BeaconState<E>, buffer: HDiffBuffer) {
        self.put_state(slot, state);
        self.put_hdiff_buffer(slot, buffer);
    }

    pub fn metrics(&self) -> Metrics {
        let hdiff_byte_size = self
            .hdiff_buffers
            .iter()
            .map(|(_, buffer)| buffer.size())
            .sum::<usize>();
        Metrics {
            num_hdiff: self.hdiff_buffers.len(),
            num_state: self.states.len(),
            hdiff_byte_size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroUsize;
    use types::{ChainSpec, Eth1Data, MinimalEthSpec};

    type E = MinimalEthSpec;

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    fn make_state(slot: u64) -> BeaconState<E> {
        let spec = ChainSpec::minimal();
        let mut state = BeaconState::new(0, Eth1Data::default(), &spec);
        *state.slot_mut() = Slot::new(slot);
        state
    }

    fn make_buffer(slot: u64) -> HDiffBuffer {
        HDiffBuffer::from_state(make_state(slot))
    }

    #[test]
    fn new_creates_empty_cache() {
        let cache = HistoricStateCache::<E>::new(nz(2), nz(2));
        let m = cache.metrics();
        assert_eq!(m.num_hdiff, 0);
        assert_eq!(m.num_state, 0);
        assert_eq!(m.hdiff_byte_size, 0);
    }

    #[test]
    fn put_state_and_get_state() {
        let spec = ChainSpec::minimal();
        let mut cache = HistoricStateCache::<E>::new(nz(2), nz(2));
        let state = make_state(10);
        let slot = Slot::new(10);

        cache.put_state(slot, state);
        let retrieved = cache.get_state(slot, &spec).unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().slot(), slot);
    }

    #[test]
    fn get_state_missing_returns_none() {
        let spec = ChainSpec::minimal();
        let mut cache = HistoricStateCache::<E>::new(nz(2), nz(2));
        let got = cache.get_state(Slot::new(99), &spec).unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn put_hdiff_buffer_and_get() {
        let mut cache = HistoricStateCache::<E>::new(nz(2), nz(2));
        let slot = Slot::new(5);
        let buffer = make_buffer(5);
        cache.put_hdiff_buffer(slot, buffer);

        let got = cache.get_hdiff_buffer(slot);
        assert!(got.is_some());
    }

    #[test]
    fn get_hdiff_buffer_missing_returns_none() {
        let mut cache = HistoricStateCache::<E>::new(nz(2), nz(2));
        assert!(cache.get_hdiff_buffer(Slot::new(42)).is_none());
    }

    #[test]
    fn get_hdiff_buffer_from_state_fallback() {
        let mut cache = HistoricStateCache::<E>::new(nz(2), nz(2));
        let slot = Slot::new(20);
        cache.put_state(slot, make_state(20));

        let got = cache.get_hdiff_buffer(slot);
        assert!(got.is_some());
        // After fallback, the buffer should also be cached
        let m = cache.metrics();
        assert_eq!(m.num_hdiff, 1);
    }

    #[test]
    fn get_state_from_hdiff_buffer_fallback() {
        let spec = ChainSpec::minimal();
        let mut cache = HistoricStateCache::<E>::new(nz(2), nz(2));
        let slot = Slot::new(30);
        cache.put_hdiff_buffer(slot, make_buffer(30));

        let got = cache.get_state(slot, &spec).unwrap();
        assert!(got.is_some());
        assert_eq!(got.unwrap().slot(), slot);
        // After fallback, the state should also be cached
        let m = cache.metrics();
        assert_eq!(m.num_state, 1);
    }

    #[test]
    fn put_both_stores_both() {
        let mut cache = HistoricStateCache::<E>::new(nz(2), nz(2));
        let slot = Slot::new(40);
        cache.put_both(slot, make_state(40), make_buffer(40));

        let m = cache.metrics();
        assert_eq!(m.num_hdiff, 1);
        assert_eq!(m.num_state, 1);
    }

    #[test]
    fn lru_eviction_states() {
        let spec = ChainSpec::minimal();
        let mut cache = HistoricStateCache::<E>::new(nz(2), nz(2));
        cache.put_state(Slot::new(1), make_state(1));
        cache.put_state(Slot::new(2), make_state(2));
        // Access slot 1 to make it recently used
        let _ = cache.get_state(Slot::new(1), &spec);
        // Adding a third evicts the LRU (slot 2)
        cache.put_state(Slot::new(3), make_state(3));

        assert!(cache.get_state(Slot::new(1), &spec).unwrap().is_some());
        assert!(cache.get_state(Slot::new(2), &spec).unwrap().is_none());
        assert!(cache.get_state(Slot::new(3), &spec).unwrap().is_some());
    }

    #[test]
    fn lru_eviction_hdiff_buffers() {
        let mut cache = HistoricStateCache::<E>::new(nz(2), nz(2));
        cache.put_hdiff_buffer(Slot::new(1), make_buffer(1));
        cache.put_hdiff_buffer(Slot::new(2), make_buffer(2));
        // Access slot 1 to make it recently used
        let _ = cache.get_hdiff_buffer(Slot::new(1));
        // Adding a third evicts the LRU (slot 2)
        cache.put_hdiff_buffer(Slot::new(3), make_buffer(3));

        assert!(cache.get_hdiff_buffer(Slot::new(1)).is_some());
        assert!(cache.get_hdiff_buffer(Slot::new(2)).is_none());
        assert!(cache.get_hdiff_buffer(Slot::new(3)).is_some());
    }

    #[test]
    fn metrics_reports_correct_counts() {
        let mut cache = HistoricStateCache::<E>::new(nz(5), nz(5));
        cache.put_state(Slot::new(1), make_state(1));
        cache.put_state(Slot::new(2), make_state(2));
        cache.put_hdiff_buffer(Slot::new(3), make_buffer(3));

        let m = cache.metrics();
        assert_eq!(m.num_state, 2);
        assert_eq!(m.num_hdiff, 1);
        assert!(m.hdiff_byte_size > 0);
    }

    #[test]
    fn metrics_hdiff_byte_size_accumulates() {
        let mut cache = HistoricStateCache::<E>::new(nz(5), nz(5));
        let buf1 = make_buffer(1);
        let size1 = buf1.size();
        cache.put_hdiff_buffer(Slot::new(1), buf1);

        let buf2 = make_buffer(2);
        let size2 = buf2.size();
        cache.put_hdiff_buffer(Slot::new(2), buf2);

        let m = cache.metrics();
        assert_eq!(m.hdiff_byte_size, size1 + size2);
    }

    #[test]
    fn overwrite_same_slot() {
        let spec = ChainSpec::minimal();
        let mut cache = HistoricStateCache::<E>::new(nz(2), nz(2));
        cache.put_state(Slot::new(5), make_state(5));
        cache.put_state(Slot::new(5), make_state(5));

        let m = cache.metrics();
        assert_eq!(m.num_state, 1);
        assert!(cache.get_state(Slot::new(5), &spec).unwrap().is_some());
    }
}
