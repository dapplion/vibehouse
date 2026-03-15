use crate::custody_context::CustodyContextSsz;
use ssz::{Decode, Encode};
use std::sync::Arc;
use store::{DBColumn, Error as StoreError, HotColdDB, ItemStore, StoreItem};
use types::{EthSpec, Hash256};

/// 32-byte key for accessing the `CustodyContext`. All zero because `CustodyContext` has its own column.
pub const CUSTODY_DB_KEY: Hash256 = Hash256::ZERO;

pub struct PersistedCustody(pub CustodyContextSsz);

pub fn load_custody_context<E: EthSpec, Hot: ItemStore<E>, Cold: ItemStore<E>>(
    store: Arc<HotColdDB<E, Hot, Cold>>,
) -> Option<CustodyContextSsz> {
    let res: Result<Option<PersistedCustody>, _> =
        store.get_item::<PersistedCustody>(&CUSTODY_DB_KEY);
    // Load context from the store
    match res {
        Ok(Some(c)) => Some(c.0),
        _ => None,
    }
}

/// Attempt to persist the custody context object to `self.store`.
pub fn persist_custody_context<E: EthSpec, Hot: ItemStore<E>, Cold: ItemStore<E>>(
    store: Arc<HotColdDB<E, Hot, Cold>>,
    custody_context: CustodyContextSsz,
) -> Result<(), store::Error> {
    store.put_item(&CUSTODY_DB_KEY, &PersistedCustody(custody_context))
}

impl StoreItem for PersistedCustody {
    fn db_column() -> DBColumn {
        DBColumn::CustodyContext
    }

    fn as_store_bytes(&self) -> Vec<u8> {
        self.0.as_ssz_bytes()
    }

    fn from_store_bytes(bytes: &[u8]) -> Result<Self, StoreError> {
        let custody_context = CustodyContextSsz::from_ssz_bytes(bytes)?;

        Ok(PersistedCustody(custody_context))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custody_db_key_is_zero() {
        assert_eq!(CUSTODY_DB_KEY, Hash256::ZERO);
    }

    #[test]
    fn store_item_column_is_custody_context() {
        assert_eq!(PersistedCustody::db_column(), DBColumn::CustodyContext);
    }

    #[test]
    fn ssz_roundtrip() {
        let ctx = CustodyContextSsz {
            validator_custody_at_head: 4,
            persisted_is_supernode: false,
            epoch_validator_custody_requirements: vec![
                (types::Epoch::new(0), 4),
                (types::Epoch::new(10), 8),
            ],
        };
        let persisted = PersistedCustody(ctx);
        let bytes = persisted.as_store_bytes();
        let decoded = PersistedCustody::from_store_bytes(&bytes).unwrap();
        assert_eq!(decoded.0.validator_custody_at_head, 4);
        assert!(!decoded.0.persisted_is_supernode);
        assert_eq!(decoded.0.epoch_validator_custody_requirements.len(), 2);
    }

    #[test]
    fn ssz_roundtrip_empty_requirements() {
        let ctx = CustodyContextSsz {
            validator_custody_at_head: 0,
            persisted_is_supernode: true,
            epoch_validator_custody_requirements: vec![],
        };
        let persisted = PersistedCustody(ctx);
        let bytes = persisted.as_store_bytes();
        let decoded = PersistedCustody::from_store_bytes(&bytes).unwrap();
        assert_eq!(decoded.0.validator_custody_at_head, 0);
        assert!(decoded.0.persisted_is_supernode);
        assert!(decoded.0.epoch_validator_custody_requirements.is_empty());
    }

    #[test]
    fn from_store_bytes_invalid_data() {
        let result = PersistedCustody::from_store_bytes(&[0xff, 0xff, 0xff]);
        assert!(result.is_err());
    }

    #[test]
    fn from_store_bytes_empty() {
        let result = PersistedCustody::from_store_bytes(&[]);
        // Empty bytes may or may not be valid depending on SSZ encoding;
        // the important thing is it doesn't panic
        let _ = result;
    }
}
