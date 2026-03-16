//! This implements a time-based LRU cache for fast checking of duplicates
use fnv::FnvHashSet;
#[cfg(test)]
use mock_instant::global::Instant;
use std::collections::VecDeque;

#[cfg(not(test))]
use std::time::Instant;

use std::time::Duration;

struct Element<Key> {
    /// The key being inserted.
    key: Key,
    /// The instant the key was inserted.
    inserted: Instant,
}

pub struct LRUTimeCache<Key> {
    /// The duplicate cache.
    map: FnvHashSet<Key>,
    /// An ordered list of keys by insert time.
    list: VecDeque<Element<Key>>,
    /// The time elements remain in the cache.
    ttl: Duration,
}

impl<Key> LRUTimeCache<Key>
where
    Key: Eq + std::hash::Hash + Clone,
{
    pub fn new(ttl: Duration) -> Self {
        LRUTimeCache {
            map: FnvHashSet::default(),
            list: VecDeque::new(),
            ttl,
        }
    }

    /// Inserts a key without removal of potentially expired elements.
    /// Returns true if the key does not already exist.
    pub fn raw_insert(&mut self, key: Key) -> bool {
        // check the cache before removing elements
        let is_new = self.map.insert(key.clone());

        // add the new key to the list, if it doesn't already exist.
        if is_new {
            self.list.push_back(Element {
                key,
                inserted: Instant::now(),
            });
        } else {
            let position = self
                .list
                .iter()
                .position(|e| e.key == key)
                .expect("Key is not new");
            let mut element = self
                .list
                .remove(position)
                .expect("Position is not occupied");
            element.inserted = Instant::now();
            self.list.push_back(element);
        }
        #[cfg(test)]
        self.check_invariant();
        is_new
    }

    /// Removes a key from the cache without purging expired elements. Returns true if the key
    /// existed.
    pub fn raw_remove(&mut self, key: &Key) -> bool {
        if self.map.remove(key) {
            let position = self
                .list
                .iter()
                .position(|e| &e.key == key)
                .expect("Key must exist");
            self.list
                .remove(position)
                .expect("Position is not occupied");
            true
        } else {
            false
        }
    }

    /// Removes all expired elements and returns them
    pub fn remove_expired(&mut self) -> Vec<Key> {
        if self.list.is_empty() {
            return Vec::new();
        }

        let mut removed_elements = Vec::new();
        let now = Instant::now();
        // remove any expired results
        while let Some(element) = self.list.pop_front() {
            if element.inserted + self.ttl > now {
                self.list.push_front(element);
                break;
            }
            self.map.remove(&element.key);
            removed_elements.push(element.key);
        }
        #[cfg(test)]
        self.check_invariant();

        removed_elements
    }

    // Inserts a new key. It first purges expired elements to do so.
    //
    // If the key was not present this returns `true`. If the value was already present this
    // returns `false` and updates the insertion time of the key.
    pub fn insert(&mut self, key: Key) -> bool {
        self.update();
        // check the cache before removing elements
        let is_new = self.map.insert(key.clone());

        // add the new key to the list, if it doesn't already exist.
        if is_new {
            self.list.push_back(Element {
                key,
                inserted: Instant::now(),
            });
        } else {
            let position = self
                .list
                .iter()
                .position(|e| e.key == key)
                .expect("Key is not new");
            let mut element = self
                .list
                .remove(position)
                .expect("Position is not occupied");
            element.inserted = Instant::now();
            self.list.push_back(element);
        }
        #[cfg(test)]
        self.check_invariant();
        is_new
    }

    /// Removes any expired elements from the cache.
    pub fn update(&mut self) {
        if self.list.is_empty() {
            return;
        }

        let now = Instant::now();
        // remove any expired results
        while let Some(element) = self.list.pop_front() {
            if element.inserted + self.ttl > now {
                self.list.push_front(element);
                break;
            }
            self.map.remove(&element.key);
        }
        #[cfg(test)]
        self.check_invariant()
    }

    /// Returns if the key is present after removing expired elements.
    pub fn contains(&mut self, key: &Key) -> bool {
        self.update();
        self.map.contains(key)
    }

    /// List known keys
    pub fn keys(&mut self) -> impl Iterator<Item = &Key> {
        self.update();
        self.map.iter()
    }

    /// Shrink the mappings to fit the current size.
    pub fn shrink_to_fit(&mut self) {
        self.map.shrink_to_fit();
        self.list.shrink_to_fit();
    }

    #[cfg(test)]
    #[track_caller]
    fn check_invariant(&self) {
        // The list should be sorted. First element should have the oldest insertion
        let mut prev_insertion_time = None;
        for e in &self.list {
            match prev_insertion_time {
                Some(prev) => {
                    if prev <= e.inserted {
                        prev_insertion_time = Some(e.inserted);
                    } else {
                        panic!("List is not sorted by insertion time")
                    }
                }
                None => prev_insertion_time = Some(e.inserted),
            }
            // The key should be in the map
            assert!(self.map.contains(&e.key), "List and map should be in sync");
        }

        for k in &self.map {
            let _ = self
                .list
                .iter()
                .position(|e| &e.key == k)
                .expect("Map and list should be in sync");
        }

        // One last check to make sure there are no duplicates in the list
        assert_eq!(self.list.len(), self.map.len());
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn cache_added_entries_exist() {
        let mut cache = LRUTimeCache::new(Duration::from_secs(10));

        cache.insert("t");
        cache.insert("e");

        // Should report that 't' and 't' already exists
        assert!(!cache.insert("t"));
        assert!(!cache.insert("e"));
    }

    #[test]
    fn test_reinsertion_updates_timeout() {
        let mut cache = LRUTimeCache::new(Duration::from_millis(100));

        cache.insert("a");
        cache.insert("b");

        mock_instant::global::MockClock::advance(Duration::from_millis(20));
        cache.insert("a");
        // a is newer now

        mock_instant::global::MockClock::advance(Duration::from_millis(85));
        assert!(cache.contains(&"a"),);
        // b was inserted first but was not as recent it should have been removed
        assert!(!cache.contains(&"b"));

        mock_instant::global::MockClock::advance(Duration::from_millis(16));
        assert!(!cache.contains(&"a"));
    }

    // ── raw_insert tests ──────────────────────────────────────

    #[test]
    fn raw_insert_new_key_returns_true() {
        let mut cache = LRUTimeCache::new(Duration::from_secs(10));
        assert!(cache.raw_insert("a"), "new key should return true");
    }

    #[test]
    fn raw_insert_existing_key_returns_false() {
        let mut cache = LRUTimeCache::new(Duration::from_secs(10));
        cache.raw_insert("a");
        assert!(!cache.raw_insert("a"), "existing key should return false");
    }

    #[test]
    fn raw_insert_does_not_purge_expired() {
        let mut cache = LRUTimeCache::new(Duration::from_millis(50));
        cache.raw_insert("a");

        mock_instant::global::MockClock::advance(Duration::from_millis(100));

        // raw_insert does NOT purge expired entries
        cache.raw_insert("b");
        // "a" is expired but still in the map because raw_insert skips purging
        assert!(cache.map.contains(&"a"), "raw_insert should not purge");
        assert_eq!(cache.list.len(), 2);
    }

    #[test]
    fn raw_insert_reinsert_moves_to_back() {
        let mut cache = LRUTimeCache::new(Duration::from_millis(200));
        cache.raw_insert("a");
        cache.raw_insert("b");
        cache.raw_insert("c");

        mock_instant::global::MockClock::advance(Duration::from_millis(50));

        // Re-insert "a" — should move it to the back with a newer timestamp
        cache.raw_insert("a");

        // The list should have "a" at the back now
        assert_eq!(cache.list.back().unwrap().key, "a");
        assert_eq!(cache.list.len(), 3);
        assert_eq!(cache.map.len(), 3);
    }

    // ── raw_remove tests ──────────────────────────────────────

    #[test]
    fn raw_remove_existing_key_returns_true() {
        let mut cache = LRUTimeCache::new(Duration::from_secs(10));
        cache.raw_insert("a");
        assert!(
            cache.raw_remove(&"a"),
            "removing existing key should return true"
        );
        assert!(!cache.map.contains(&"a"));
        assert_eq!(cache.list.len(), 0);
    }

    #[test]
    fn raw_remove_nonexistent_key_returns_false() {
        let mut cache: LRUTimeCache<&str> = LRUTimeCache::new(Duration::from_secs(10));
        assert!(
            !cache.raw_remove(&"a"),
            "removing absent key should return false"
        );
    }

    #[test]
    fn raw_remove_middle_element() {
        let mut cache = LRUTimeCache::new(Duration::from_secs(10));
        cache.raw_insert("a");
        cache.raw_insert("b");
        cache.raw_insert("c");

        assert!(cache.raw_remove(&"b"));
        assert_eq!(cache.list.len(), 2);
        assert_eq!(cache.map.len(), 2);
        assert!(cache.map.contains(&"a"));
        assert!(!cache.map.contains(&"b"));
        assert!(cache.map.contains(&"c"));
    }

    // ── remove_expired tests ──────────────────────────────────

    #[test]
    fn remove_expired_empty_cache() {
        let mut cache: LRUTimeCache<&str> = LRUTimeCache::new(Duration::from_secs(10));
        let removed = cache.remove_expired();
        assert!(removed.is_empty());
    }

    #[test]
    fn remove_expired_nothing_expired() {
        let mut cache = LRUTimeCache::new(Duration::from_secs(10));
        cache.raw_insert("a");
        cache.raw_insert("b");

        let removed = cache.remove_expired();
        assert!(removed.is_empty());
        assert_eq!(cache.map.len(), 2);
    }

    #[test]
    fn remove_expired_returns_expired_keys() {
        let mut cache = LRUTimeCache::new(Duration::from_millis(50));
        cache.raw_insert("a");
        cache.raw_insert("b");

        mock_instant::global::MockClock::advance(Duration::from_millis(30));
        cache.raw_insert("c");

        mock_instant::global::MockClock::advance(Duration::from_millis(30));

        // "a" and "b" are expired (inserted 60ms ago, TTL is 50ms)
        // "c" is not expired (inserted 30ms ago)
        let removed = cache.remove_expired();
        assert_eq!(removed.len(), 2);
        assert!(removed.contains(&"a"));
        assert!(removed.contains(&"b"));
        assert!(cache.map.contains(&"c"));
        assert_eq!(cache.list.len(), 1);
    }

    #[test]
    fn remove_expired_all_expired() {
        let mut cache = LRUTimeCache::new(Duration::from_millis(50));
        cache.raw_insert("a");
        cache.raw_insert("b");

        mock_instant::global::MockClock::advance(Duration::from_millis(100));

        let removed = cache.remove_expired();
        assert_eq!(removed.len(), 2);
        assert!(cache.map.is_empty());
        assert!(cache.list.is_empty());
    }

    // ── contains tests ────────────────────────────────────────

    #[test]
    fn contains_existing_key() {
        let mut cache = LRUTimeCache::new(Duration::from_secs(10));
        cache.insert("a");
        assert!(cache.contains(&"a"));
    }

    #[test]
    fn contains_nonexistent_key() {
        let mut cache: LRUTimeCache<&str> = LRUTimeCache::new(Duration::from_secs(10));
        assert!(!cache.contains(&"a"));
    }

    #[test]
    fn contains_expired_key_returns_false() {
        let mut cache = LRUTimeCache::new(Duration::from_millis(50));
        cache.insert("a");

        mock_instant::global::MockClock::advance(Duration::from_millis(100));

        assert!(!cache.contains(&"a"), "expired key should not be found");
    }

    // ── keys tests ────────────────────────────────────────────

    #[test]
    fn keys_returns_all_active_keys() {
        let mut cache = LRUTimeCache::new(Duration::from_secs(10));
        cache.insert("a");
        cache.insert("b");
        cache.insert("c");

        let keys: Vec<_> = cache.keys().copied().collect();
        assert_eq!(keys.len(), 3);
        assert!(keys.contains(&"a"));
        assert!(keys.contains(&"b"));
        assert!(keys.contains(&"c"));
    }

    #[test]
    fn keys_excludes_expired() {
        let mut cache = LRUTimeCache::new(Duration::from_millis(50));
        cache.insert("a");

        mock_instant::global::MockClock::advance(Duration::from_millis(30));
        cache.insert("b");

        mock_instant::global::MockClock::advance(Duration::from_millis(30));

        // "a" expired, "b" still alive
        let keys: Vec<_> = cache.keys().copied().collect();
        assert_eq!(keys.len(), 1);
        assert!(keys.contains(&"b"));
    }

    #[test]
    fn keys_empty_cache() {
        let mut cache: LRUTimeCache<&str> = LRUTimeCache::new(Duration::from_secs(10));
        let keys: Vec<_> = cache.keys().collect();
        assert!(keys.is_empty());
    }

    // ── shrink_to_fit tests ───────────────────────────────────

    #[test]
    fn shrink_to_fit_preserves_data() {
        let mut cache = LRUTimeCache::new(Duration::from_secs(10));
        for i in 0..100 {
            cache.insert(i);
        }

        // Remove most entries via expiry
        mock_instant::global::MockClock::advance(Duration::from_secs(20));
        cache.update();

        // All should be expired
        assert!(cache.map.is_empty());

        cache.shrink_to_fit();

        // Should still work correctly after shrink
        cache.insert(200);
        assert!(cache.contains(&200));
    }

    // ── update tests ──────────────────────────────────────────

    #[test]
    fn update_removes_expired_entries() {
        let mut cache = LRUTimeCache::new(Duration::from_millis(50));
        cache.raw_insert("a");
        cache.raw_insert("b");

        mock_instant::global::MockClock::advance(Duration::from_millis(100));

        cache.update();
        assert!(cache.map.is_empty());
        assert!(cache.list.is_empty());
    }

    #[test]
    fn update_noop_on_empty() {
        let mut cache: LRUTimeCache<&str> = LRUTimeCache::new(Duration::from_secs(10));
        cache.update(); // should not panic
        assert!(cache.map.is_empty());
    }

    #[test]
    fn update_preserves_non_expired() {
        let mut cache = LRUTimeCache::new(Duration::from_millis(100));
        cache.raw_insert("a");

        mock_instant::global::MockClock::advance(Duration::from_millis(30));
        cache.raw_insert("b");

        mock_instant::global::MockClock::advance(Duration::from_millis(80));

        // "a" is 110ms old (expired), "b" is 80ms old (not expired)
        cache.update();
        assert!(!cache.map.contains(&"a"));
        assert!(cache.map.contains(&"b"));
        assert_eq!(cache.list.len(), 1);
    }

    // ── insert with auto-purge tests ──────────────────────────

    #[test]
    fn insert_purges_expired_before_inserting() {
        let mut cache = LRUTimeCache::new(Duration::from_millis(50));
        cache.insert("a");

        mock_instant::global::MockClock::advance(Duration::from_millis(100));

        // insert should purge "a" first, then insert "b"
        assert!(cache.insert("b"), "new key after purge should return true");
        assert!(!cache.map.contains(&"a"), "expired key should be purged");
        assert!(cache.map.contains(&"b"));
        assert_eq!(cache.list.len(), 1);
    }

    #[test]
    fn insert_reinsert_updates_timestamp() {
        let mut cache = LRUTimeCache::new(Duration::from_millis(100));
        cache.insert("a");
        cache.insert("b");

        mock_instant::global::MockClock::advance(Duration::from_millis(60));

        // Re-insert "a" — refreshes its timestamp
        assert!(!cache.insert("a"), "re-insert should return false");

        mock_instant::global::MockClock::advance(Duration::from_millis(50));

        // "b" is 110ms old (expired), "a" is 50ms old (alive)
        assert!(
            cache.contains(&"a"),
            "re-inserted key should still be alive"
        );
        assert!(!cache.contains(&"b"), "non-refreshed key should be expired");
    }

    // ── edge case tests ───────────────────────────────────────

    #[test]
    fn single_element_lifecycle() {
        let mut cache = LRUTimeCache::new(Duration::from_millis(50));

        assert!(cache.insert("only"));
        assert!(cache.contains(&"only"));

        mock_instant::global::MockClock::advance(Duration::from_millis(60));

        assert!(!cache.contains(&"only"), "single element should expire");
        assert!(cache.map.is_empty());
    }

    #[test]
    fn rapid_reinserts_keep_element_alive() {
        let mut cache = LRUTimeCache::new(Duration::from_millis(100));
        cache.insert("a");

        // Re-insert every 40ms — should never expire
        for _ in 0..10 {
            mock_instant::global::MockClock::advance(Duration::from_millis(40));
            assert!(!cache.insert("a"), "should find existing key");
            assert!(
                cache.contains(&"a"),
                "should still be alive after re-insert"
            );
        }
    }

    #[test]
    fn interleaved_insert_and_remove() {
        let mut cache = LRUTimeCache::new(Duration::from_secs(10));

        cache.raw_insert("a");
        cache.raw_insert("b");
        cache.raw_insert("c");

        cache.raw_remove(&"b");
        assert_eq!(cache.map.len(), 2);

        cache.raw_insert("d");
        assert_eq!(cache.map.len(), 3);

        cache.raw_remove(&"a");
        cache.raw_remove(&"c");
        assert_eq!(cache.map.len(), 1);
        assert!(cache.map.contains(&"d"));
    }

    #[test]
    fn integer_keys() {
        let mut cache = LRUTimeCache::new(Duration::from_secs(10));
        for i in 0u64..50 {
            assert!(cache.insert(i));
        }
        assert_eq!(cache.map.len(), 50);

        for i in 0u64..50 {
            assert!(cache.contains(&i));
        }

        // Re-insert all — should return false
        for i in 0u64..50 {
            assert!(!cache.insert(i));
        }
    }
}
