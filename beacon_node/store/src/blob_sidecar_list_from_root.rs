use std::sync::Arc;
use types::{BlobSidecar, BlobSidecarList, EthSpec};

#[derive(Debug, Clone)]
pub enum BlobSidecarListFromRoot<E: EthSpec> {
    /// Valid root that exists in the DB, but has no blobs associated with it.
    NoBlobs,
    /// Contains > 1 blob for the requested root.
    Blobs(BlobSidecarList<E>),
    /// No root exists in the db or cache for the requested root.
    NoRoot,
}

impl<E: EthSpec> From<BlobSidecarList<E>> for BlobSidecarListFromRoot<E> {
    fn from(value: BlobSidecarList<E>) -> Self {
        Self::Blobs(value)
    }
}

impl<E: EthSpec> BlobSidecarListFromRoot<E> {
    pub fn blobs(self) -> Option<BlobSidecarList<E>> {
        match self {
            Self::NoBlobs | Self::NoRoot => None,
            Self::Blobs(blobs) => Some(blobs),
        }
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        match self {
            Self::NoBlobs | Self::NoRoot => 0,
            Self::Blobs(blobs) => blobs.len(),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<BlobSidecar<E>>> {
        match self {
            Self::NoBlobs | Self::NoRoot => [].iter(),
            Self::Blobs(list) => list.iter(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::MinimalEthSpec;

    type E = MinimalEthSpec;

    #[test]
    fn no_blobs_variant() {
        let result = BlobSidecarListFromRoot::<E>::NoBlobs;
        assert!(result.blobs().is_none());
    }

    #[test]
    fn no_root_variant() {
        let result = BlobSidecarListFromRoot::<E>::NoRoot;
        assert!(result.blobs().is_none());
    }

    #[test]
    fn no_blobs_len_is_zero() {
        let result = BlobSidecarListFromRoot::<E>::NoBlobs;
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn no_root_len_is_zero() {
        let result = BlobSidecarListFromRoot::<E>::NoRoot;
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn no_blobs_iter_is_empty() {
        let result = BlobSidecarListFromRoot::<E>::NoBlobs;
        assert_eq!(result.iter().count(), 0);
    }

    #[test]
    fn no_root_iter_is_empty() {
        let result = BlobSidecarListFromRoot::<E>::NoRoot;
        assert_eq!(result.iter().count(), 0);
    }

    #[test]
    fn blobs_variant_returns_some() {
        let list = BlobSidecarList::<E>::empty(6);
        let result = BlobSidecarListFromRoot::<E>::Blobs(list.clone());
        assert!(result.blobs().is_some());
    }

    #[test]
    fn blobs_variant_empty_list_len_zero() {
        let list = BlobSidecarList::<E>::empty(6);
        let result = BlobSidecarListFromRoot::<E>::Blobs(list);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn from_blob_sidecar_list() {
        let list = BlobSidecarList::<E>::empty(6);
        let result: BlobSidecarListFromRoot<E> = list.into();
        // From impl wraps in Blobs variant
        assert!(result.blobs().is_some());
    }

    #[test]
    fn clone_preserves_variant() {
        let no_blobs = BlobSidecarListFromRoot::<E>::NoBlobs;
        let cloned = no_blobs.clone();
        assert_eq!(cloned.len(), 0);
        assert!(cloned.blobs().is_none());

        let no_root = BlobSidecarListFromRoot::<E>::NoRoot;
        let cloned = no_root.clone();
        assert_eq!(cloned.len(), 0);
        assert!(cloned.blobs().is_none());
    }

    #[test]
    fn blobs_variant_iter_empty_list() {
        let list = BlobSidecarList::<E>::empty(6);
        let result = BlobSidecarListFromRoot::<E>::Blobs(list);
        assert_eq!(result.iter().count(), 0);
    }
}
