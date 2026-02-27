use crate::*;
use rpds::HashTrieMapSync as HashTrieMap;

type BuilderIdx = usize;

/// Cache mapping builder pubkeys to their index in `state.builders`.
///
/// Unlike the validator `PubkeyCache`, builder indices can be reused when exited builders
/// are replaced. The `insert` method handles both new builders and index reuse.
#[allow(clippy::len_without_is_empty)]
#[derive(Debug, PartialEq, Clone, Default)]
pub struct BuilderPubkeyCache {
    map: HashTrieMap<PublicKeyBytes, BuilderIdx>,
}

impl BuilderPubkeyCache {
    /// Returns the builder index for the given pubkey, if present.
    pub fn get(&self, pubkey: &PublicKeyBytes) -> Option<BuilderIdx> {
        self.map.get(pubkey).copied()
    }

    /// Insert a new builder pubkey → index mapping.
    ///
    /// If the index was previously used by a different builder (index reuse after exit),
    /// the old pubkey must be removed first via `remove`.
    pub fn insert(&mut self, pubkey: PublicKeyBytes, index: BuilderIdx) {
        self.map.insert_mut(pubkey, index);
    }

    /// Remove a builder pubkey from the cache.
    pub fn remove(&mut self, pubkey: &PublicKeyBytes) {
        self.map.remove_mut(pubkey);
    }

    /// Returns the number of builders in the cache.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.map.size()
    }
}

#[cfg(feature = "arbitrary")]
impl arbitrary::Arbitrary<'_> for BuilderPubkeyCache {
    fn arbitrary(_u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        Ok(Self::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pk(byte: u8) -> PublicKeyBytes {
        let mut bytes = [0u8; 48];
        bytes[0] = byte;
        PublicKeyBytes::deserialize(&bytes).unwrap()
    }

    #[test]
    fn empty_cache_returns_none() {
        let cache = BuilderPubkeyCache::default();
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.get(&PublicKeyBytes::empty()), None);
        assert_eq!(cache.get(&pk(0x01)), None);
    }

    #[test]
    fn insert_and_get() {
        let mut cache = BuilderPubkeyCache::default();
        let key_a = pk(0x01);
        let key_b = pk(0x02);

        cache.insert(key_a, 0);
        cache.insert(key_b, 1);

        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&key_a), Some(0));
        assert_eq!(cache.get(&key_b), Some(1));
        // Unknown key still returns None
        assert_eq!(cache.get(&pk(0x03)), None);
    }

    #[test]
    fn remove_deletes_entry() {
        let mut cache = BuilderPubkeyCache::default();
        let key = pk(0x01);

        cache.insert(key, 5);
        assert_eq!(cache.get(&key), Some(5));
        assert_eq!(cache.len(), 1);

        cache.remove(&key);
        assert_eq!(cache.get(&key), None);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn insert_overwrites_same_pubkey() {
        // If the same pubkey is inserted twice with different indices,
        // the second insert overwrites the first.
        let mut cache = BuilderPubkeyCache::default();
        let key = pk(0x01);

        cache.insert(key, 0);
        assert_eq!(cache.get(&key), Some(0));

        cache.insert(key, 7);
        assert_eq!(cache.get(&key), Some(7));
        // Length stays 1 — same key, updated value
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn index_reuse_via_remove_then_insert() {
        // Simulates the index reuse pattern: exited builder at index 2
        // is replaced by a new builder with a different pubkey.
        let mut cache = BuilderPubkeyCache::default();
        let old_pk = pk(0x01);
        let new_pk = pk(0x02);

        cache.insert(old_pk, 2);
        assert_eq!(cache.get(&old_pk), Some(2));

        // Remove old builder, insert new one at the same index
        cache.remove(&old_pk);
        cache.insert(new_pk, 2);

        assert_eq!(cache.get(&old_pk), None);
        assert_eq!(cache.get(&new_pk), Some(2));
        assert_eq!(cache.len(), 1);
    }
}
