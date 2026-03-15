use ssz::{Decode, Encode};
use ssz_derive::{Decode, Encode};
use store::{DBColumn, Error as StoreError, StoreItem};
use types::Hash256;

#[derive(Clone, Encode, Decode)]
pub struct PersistedBeaconChain {
    pub genesis_block_root: Hash256,
}

impl StoreItem for PersistedBeaconChain {
    fn db_column() -> DBColumn {
        DBColumn::BeaconChain
    }

    fn as_store_bytes(&self) -> Vec<u8> {
        self.as_ssz_bytes()
    }

    fn from_store_bytes(bytes: &[u8]) -> Result<Self, StoreError> {
        Self::from_ssz_bytes(bytes).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::FixedBytesExtended;

    #[test]
    fn store_item_column_is_beacon_chain() {
        assert_eq!(PersistedBeaconChain::db_column(), DBColumn::BeaconChain);
    }

    #[test]
    fn ssz_roundtrip_zero_root() {
        let chain = PersistedBeaconChain {
            genesis_block_root: Hash256::zero(),
        };
        let bytes = chain.as_store_bytes();
        let decoded = PersistedBeaconChain::from_store_bytes(&bytes).unwrap();
        assert_eq!(chain.genesis_block_root, decoded.genesis_block_root);
    }

    #[test]
    fn ssz_roundtrip_nonzero_root() {
        let chain = PersistedBeaconChain {
            genesis_block_root: Hash256::repeat_byte(0xab),
        };
        let bytes = chain.as_store_bytes();
        let decoded = PersistedBeaconChain::from_store_bytes(&bytes).unwrap();
        assert_eq!(chain.genesis_block_root, decoded.genesis_block_root);
    }

    #[test]
    fn from_store_bytes_invalid_data() {
        let result = PersistedBeaconChain::from_store_bytes(&[1, 2, 3]);
        assert!(result.is_err());
    }

    #[test]
    fn from_store_bytes_empty() {
        let result = PersistedBeaconChain::from_store_bytes(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn clone_preserves_root() {
        let chain = PersistedBeaconChain {
            genesis_block_root: Hash256::repeat_byte(0xff),
        };
        let cloned = chain.clone();
        assert_eq!(chain.genesis_block_root, cloned.genesis_block_root);
    }

    #[test]
    fn store_bytes_length_is_32() {
        let chain = PersistedBeaconChain {
            genesis_block_root: Hash256::zero(),
        };
        // Hash256 is 32 bytes
        assert_eq!(chain.as_store_bytes().len(), 32);
    }
}
