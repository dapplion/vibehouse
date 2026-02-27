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
