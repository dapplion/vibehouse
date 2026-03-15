use crate::*;
use rpds::HashTrieMapSync as HashTrieMap;

type ValidatorIndex = usize;

#[allow(clippy::len_without_is_empty)]
#[derive(Debug, PartialEq, Clone, Default)]
pub struct PubkeyCache {
    /// Maintain the number of keys added to the map. It is not sufficient to just use the
    /// HashTrieMap len, as it does not increase when duplicate keys are added. Duplicate keys are
    /// used during testing.
    len: usize,
    map: HashTrieMap<PublicKeyBytes, ValidatorIndex>,
}

impl PubkeyCache {
    /// Returns the number of validator indices added to the map so far.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> ValidatorIndex {
        self.len
    }

    /// Inserts a validator index into the map.
    ///
    /// The added index must equal the number of validators already added to the map. This ensures
    /// that an index is never skipped.
    pub fn insert(&mut self, pubkey: PublicKeyBytes, index: ValidatorIndex) -> bool {
        if index == self.len {
            self.map.insert_mut(pubkey, index);
            self.len = self
                .len
                .checked_add(1)
                .expect("map length cannot exceed usize");
            true
        } else {
            false
        }
    }

    /// Looks up a validator index's by their public key.
    pub fn get(&self, pubkey: &PublicKeyBytes) -> Option<ValidatorIndex> {
        self.map.get(pubkey).copied()
    }
}

#[cfg(feature = "arbitrary")]
impl arbitrary::Arbitrary<'_> for PubkeyCache {
    fn arbitrary(_u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        Ok(Self::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pubkey(byte: u8) -> PublicKeyBytes {
        let mut bytes = [0u8; 48];
        bytes[0] = byte;
        PublicKeyBytes::deserialize(&bytes).unwrap()
    }

    #[test]
    fn default_is_empty() {
        let cache = PubkeyCache::default();
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn insert_first_validator() {
        let mut cache = PubkeyCache::default();
        assert!(cache.insert(pubkey(1), 0));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn insert_wrong_index_fails() {
        let mut cache = PubkeyCache::default();
        // Trying to insert index 1 when len is 0 should fail
        assert!(!cache.insert(pubkey(1), 1));
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn insert_skipped_index_fails() {
        let mut cache = PubkeyCache::default();
        assert!(cache.insert(pubkey(1), 0));
        // Skip index 1, try index 2
        assert!(!cache.insert(pubkey(3), 2));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn get_existing_key() {
        let mut cache = PubkeyCache::default();
        let pk = pubkey(42);
        cache.insert(pk, 0);
        assert_eq!(cache.get(&pk), Some(0));
    }

    #[test]
    fn get_missing_key() {
        let cache = PubkeyCache::default();
        assert_eq!(cache.get(&pubkey(1)), None);
    }

    #[test]
    fn sequential_inserts() {
        let mut cache = PubkeyCache::default();
        for i in 0..5 {
            assert!(cache.insert(pubkey(i as u8), i));
        }
        assert_eq!(cache.len(), 5);
        for i in 0..5 {
            assert_eq!(cache.get(&pubkey(i as u8)), Some(i));
        }
    }

    #[test]
    fn duplicate_pubkey_increments_len() {
        let mut cache = PubkeyCache::default();
        let pk = pubkey(1);
        assert!(cache.insert(pk, 0));
        assert!(cache.insert(pk, 1));
        // len tracks insertions, not unique keys
        assert_eq!(cache.len(), 2);
        // Map stores the latest index for the key
        assert_eq!(cache.get(&pk), Some(1));
    }

    #[test]
    fn get_after_many_inserts() {
        let mut cache = PubkeyCache::default();
        assert!(cache.insert(pubkey(10), 0));
        assert!(cache.insert(pubkey(20), 1));
        assert!(cache.insert(pubkey(30), 2));
        assert_eq!(cache.get(&pubkey(10)), Some(0));
        assert_eq!(cache.get(&pubkey(20)), Some(1));
        assert_eq!(cache.get(&pubkey(30)), Some(2));
        assert_eq!(cache.get(&pubkey(40)), None);
    }
}
