use crate::{
    ColumnIter, ColumnKeyIter, DBColumn, Error, ItemStore, Key, KeyValueStore, KeyValueStoreOp,
    errors::Error as DBError, get_key_for_col, hot_cold_store::BytesKey,
};
use parking_lot::RwLock;
use std::collections::{BTreeMap, HashSet};
use std::marker::PhantomData;
use types::*;

type DBMap = BTreeMap<BytesKey, Vec<u8>>;

/// A thread-safe `BTreeMap` wrapper.
pub struct MemoryStore<E: EthSpec> {
    db: RwLock<DBMap>,
    _phantom: PhantomData<E>,
}

impl<E: EthSpec> MemoryStore<E> {
    /// Create a new, empty database.
    pub fn open() -> Self {
        Self {
            db: RwLock::new(BTreeMap::new()),
            _phantom: PhantomData,
        }
    }
}

impl<E: EthSpec> KeyValueStore<E> for MemoryStore<E> {
    /// Get the value of some key from the database. Returns `None` if the key does not exist.
    fn get_bytes(&self, col: DBColumn, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        let column_key = BytesKey::from_vec(get_key_for_col(col, key));
        Ok(self.db.read().get(&column_key).cloned())
    }

    /// Puts a key in the database.
    fn put_bytes(&self, col: DBColumn, key: &[u8], val: &[u8]) -> Result<(), Error> {
        let column_key = BytesKey::from_vec(get_key_for_col(col, key));
        self.db.write().insert(column_key, val.to_vec());
        Ok(())
    }

    fn put_bytes_sync(&self, col: DBColumn, key: &[u8], val: &[u8]) -> Result<(), Error> {
        self.put_bytes(col, key, val)
    }

    fn sync(&self) -> Result<(), Error> {
        // no-op
        Ok(())
    }

    /// Return true if some key exists in some column.
    fn key_exists(&self, col: DBColumn, key: &[u8]) -> Result<bool, Error> {
        let column_key = BytesKey::from_vec(get_key_for_col(col, key));
        Ok(self.db.read().contains_key(&column_key))
    }

    /// Delete some key from the database.
    fn key_delete(&self, col: DBColumn, key: &[u8]) -> Result<(), Error> {
        let column_key = BytesKey::from_vec(get_key_for_col(col, key));
        self.db.write().remove(&column_key);
        Ok(())
    }

    fn do_atomically(&self, batch: Vec<KeyValueStoreOp>) -> Result<(), Error> {
        for op in batch {
            match op {
                KeyValueStoreOp::PutKeyValue(col, key, value) => {
                    let column_key = get_key_for_col(col, &key);
                    self.db
                        .write()
                        .insert(BytesKey::from_vec(column_key), value);
                }

                KeyValueStoreOp::DeleteKey(col, key) => {
                    let column_key = get_key_for_col(col, &key);
                    self.db.write().remove(&BytesKey::from_vec(column_key));
                }
            }
        }
        Ok(())
    }

    fn iter_column_from<K: Key>(&self, column: DBColumn, from: &[u8]) -> ColumnIter<'_, K> {
        // We use this awkward pattern because we can't lock the `self.db` field *and* maintain a
        // reference to the lock guard across calls to `.next()`. This would be require a
        // struct with a field (the iterator) which references another field (the lock guard).
        let start_key = BytesKey::from_vec(get_key_for_col(column, from));
        let keys = self
            .db
            .read()
            .range(start_key..)
            .take_while(|(k, _)| k.remove_column_variable(column).is_some())
            .filter_map(|(k, _)| k.remove_column_variable(column).map(|k| k.to_vec()))
            .collect::<Vec<_>>();
        Box::new(keys.into_iter().filter_map(move |key| {
            self.get_bytes(column, &key).transpose().map(|res| {
                let k = K::from_bytes(&key)?;
                let v = res?;
                Ok((k, v))
            })
        }))
    }

    fn iter_column_keys<K: Key>(&self, column: DBColumn) -> ColumnKeyIter<'_, K> {
        Box::new(self.iter_column(column).map(|res| res.map(|(k, _)| k)))
    }

    fn compact_column(&self, _column: DBColumn) -> Result<(), Error> {
        Ok(())
    }

    fn iter_column_keys_from<K: Key>(&self, column: DBColumn, from: &[u8]) -> ColumnKeyIter<'_, K> {
        // We use this awkward pattern because we can't lock the `self.db` field *and* maintain a
        // reference to the lock guard across calls to `.next()`. This would be require a
        // struct with a field (the iterator) which references another field (the lock guard).
        let start_key = BytesKey::from_vec(get_key_for_col(column, from));
        let keys = self
            .db
            .read()
            .range(start_key..)
            .take_while(|(k, _)| k.remove_column_variable(column).is_some())
            .filter_map(|(k, _)| k.remove_column_variable(column).map(|k| k.to_vec()))
            .collect::<Vec<_>>();
        Box::new(keys.into_iter().map(move |key| K::from_bytes(&key)))
    }

    fn delete_batch(&self, col: DBColumn, ops: HashSet<&[u8]>) -> Result<(), DBError> {
        for op in ops {
            let column_key = get_key_for_col(col, op);
            self.db.write().remove(&BytesKey::from_vec(column_key));
        }
        Ok(())
    }

    fn delete_if(
        &self,
        column: DBColumn,
        mut f: impl FnMut(&[u8]) -> Result<bool, Error>,
    ) -> Result<(), Error> {
        self.db.write().retain(|key, value| {
            if key.remove_column_variable(column).is_some() {
                !f(value).unwrap_or(false)
            } else {
                true
            }
        });
        Ok(())
    }
}

impl<E: EthSpec> ItemStore<E> for MemoryStore<E> {}

#[cfg(test)]
mod tests {
    use super::*;
    use types::MainnetEthSpec;

    type E = MainnetEthSpec;

    fn store() -> MemoryStore<E> {
        MemoryStore::open()
    }

    #[test]
    fn open_creates_empty_store() {
        let s = store();
        assert!(!s.key_exists(DBColumn::BeaconMeta, &[0u8; 32]).unwrap());
    }

    #[test]
    fn put_and_get_bytes() {
        let s = store();
        let key = [1u8; 32];
        let val = b"hello";
        s.put_bytes(DBColumn::BeaconBlock, &key, val).unwrap();
        let got = s.get_bytes(DBColumn::BeaconBlock, &key).unwrap();
        assert_eq!(got, Some(val.to_vec()));
    }

    #[test]
    fn get_missing_returns_none() {
        let s = store();
        let got = s.get_bytes(DBColumn::BeaconBlock, &[9u8; 32]).unwrap();
        assert_eq!(got, None);
    }

    #[test]
    fn put_bytes_sync_same_as_put_bytes() {
        let s = store();
        let key = [2u8; 32];
        let val = b"sync_value";
        s.put_bytes_sync(DBColumn::BeaconBlock, &key, val).unwrap();
        let got = s.get_bytes(DBColumn::BeaconBlock, &key).unwrap();
        assert_eq!(got, Some(val.to_vec()));
    }

    #[test]
    fn key_exists_true_after_put() {
        let s = store();
        let key = [3u8; 32];
        s.put_bytes(DBColumn::BeaconBlock, &key, b"data").unwrap();
        assert!(s.key_exists(DBColumn::BeaconBlock, &key).unwrap());
    }

    #[test]
    fn key_exists_false_wrong_column() {
        let s = store();
        let key = [4u8; 32];
        s.put_bytes(DBColumn::BeaconBlock, &key, b"data").unwrap();
        assert!(!s.key_exists(DBColumn::BeaconMeta, &key).unwrap());
    }

    #[test]
    fn key_delete_removes_key() {
        let s = store();
        let key = [5u8; 32];
        s.put_bytes(DBColumn::BeaconBlock, &key, b"data").unwrap();
        s.key_delete(DBColumn::BeaconBlock, &key).unwrap();
        assert_eq!(s.get_bytes(DBColumn::BeaconBlock, &key).unwrap(), None);
        assert!(!s.key_exists(DBColumn::BeaconBlock, &key).unwrap());
    }

    #[test]
    fn key_delete_nonexistent_is_noop() {
        let s = store();
        // Should not error
        s.key_delete(DBColumn::BeaconBlock, &[99u8; 32]).unwrap();
    }

    #[test]
    fn put_overwrites_existing() {
        let s = store();
        let key = [6u8; 32];
        s.put_bytes(DBColumn::BeaconBlock, &key, b"old").unwrap();
        s.put_bytes(DBColumn::BeaconBlock, &key, b"new").unwrap();
        let got = s.get_bytes(DBColumn::BeaconBlock, &key).unwrap();
        assert_eq!(got, Some(b"new".to_vec()));
    }

    #[test]
    fn do_atomically_put_and_delete() {
        let s = store();
        let key1 = [7u8; 32];
        let key2 = [8u8; 32];
        // Pre-insert key2 so we can delete it atomically
        s.put_bytes(DBColumn::BeaconBlock, &key2, b"to_delete")
            .unwrap();

        let ops = vec![
            KeyValueStoreOp::PutKeyValue(DBColumn::BeaconBlock, key1.to_vec(), b"val1".to_vec()),
            KeyValueStoreOp::DeleteKey(DBColumn::BeaconBlock, key2.to_vec()),
        ];
        s.do_atomically(ops).unwrap();

        assert_eq!(
            s.get_bytes(DBColumn::BeaconBlock, &key1).unwrap(),
            Some(b"val1".to_vec())
        );
        assert_eq!(s.get_bytes(DBColumn::BeaconBlock, &key2).unwrap(), None);
    }

    #[test]
    fn do_atomically_empty_batch() {
        let s = store();
        s.do_atomically(vec![]).unwrap();
    }

    #[test]
    fn different_columns_independent() {
        let s = store();
        let key = [10u8; 32];
        s.put_bytes(DBColumn::BeaconBlock, &key, b"block_data")
            .unwrap();
        s.put_bytes(DBColumn::BeaconMeta, &key, b"meta_data")
            .unwrap();

        assert_eq!(
            s.get_bytes(DBColumn::BeaconBlock, &key).unwrap(),
            Some(b"block_data".to_vec())
        );
        assert_eq!(
            s.get_bytes(DBColumn::BeaconMeta, &key).unwrap(),
            Some(b"meta_data".to_vec())
        );
    }

    #[test]
    fn sync_is_noop() {
        let s = store();
        s.sync().unwrap();
    }

    #[test]
    fn compact_column_is_noop() {
        let s = store();
        s.compact_column(DBColumn::BeaconBlock).unwrap();
    }

    #[test]
    fn delete_batch_removes_multiple_keys() {
        let s = store();
        let key1 = [11u8; 32];
        let key2 = [12u8; 32];
        let key3 = [13u8; 32];
        s.put_bytes(DBColumn::BeaconBlock, &key1, b"a").unwrap();
        s.put_bytes(DBColumn::BeaconBlock, &key2, b"b").unwrap();
        s.put_bytes(DBColumn::BeaconBlock, &key3, b"c").unwrap();

        let mut ops = HashSet::new();
        ops.insert(key1.as_slice());
        ops.insert(key2.as_slice());
        s.delete_batch(DBColumn::BeaconBlock, ops).unwrap();

        assert_eq!(s.get_bytes(DBColumn::BeaconBlock, &key1).unwrap(), None);
        assert_eq!(s.get_bytes(DBColumn::BeaconBlock, &key2).unwrap(), None);
        assert_eq!(
            s.get_bytes(DBColumn::BeaconBlock, &key3).unwrap(),
            Some(b"c".to_vec())
        );
    }

    #[test]
    fn delete_if_removes_matching_values() {
        let s = store();
        let key1 = [14u8; 32];
        let key2 = [15u8; 32];
        s.put_bytes(DBColumn::BeaconBlock, &key1, b"remove_me")
            .unwrap();
        s.put_bytes(DBColumn::BeaconBlock, &key2, b"keep_me")
            .unwrap();

        s.delete_if(DBColumn::BeaconBlock, |val| Ok(val == b"remove_me"))
            .unwrap();

        assert_eq!(s.get_bytes(DBColumn::BeaconBlock, &key1).unwrap(), None);
        assert_eq!(
            s.get_bytes(DBColumn::BeaconBlock, &key2).unwrap(),
            Some(b"keep_me".to_vec())
        );
    }

    #[test]
    fn delete_if_does_not_affect_other_columns() {
        let s = store();
        let key = [16u8; 32];
        s.put_bytes(DBColumn::BeaconBlock, &key, b"target").unwrap();
        s.put_bytes(DBColumn::BeaconMeta, &key, b"target").unwrap();

        s.delete_if(DBColumn::BeaconBlock, |_val| Ok(true)).unwrap();

        assert_eq!(s.get_bytes(DBColumn::BeaconBlock, &key).unwrap(), None);
        assert_eq!(
            s.get_bytes(DBColumn::BeaconMeta, &key).unwrap(),
            Some(b"target".to_vec())
        );
    }

    #[test]
    fn iter_column_from_returns_matching_keys() {
        use types::Hash256;
        let s = store();
        // BeaconBlock column uses 32-byte keys matching Hash256
        let key1 = [0u8; 32];
        let key2 = [1u8; 32];
        s.put_bytes(DBColumn::BeaconBlock, &key1, b"v1").unwrap();
        s.put_bytes(DBColumn::BeaconBlock, &key2, b"v2").unwrap();

        let results: Vec<(Hash256, Vec<u8>)> = s
            .iter_column_from::<Hash256>(DBColumn::BeaconBlock, &key1)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn iter_column_keys_returns_keys() {
        use types::Hash256;
        let s = store();
        let key1 = [0u8; 32];
        let key2 = [1u8; 32];
        s.put_bytes(DBColumn::BeaconBlock, &key1, b"v1").unwrap();
        s.put_bytes(DBColumn::BeaconBlock, &key2, b"v2").unwrap();

        let keys: Vec<Hash256> = s
            .iter_column_keys::<Hash256>(DBColumn::BeaconBlock)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn iter_column_keys_from_returns_keys_from_offset() {
        use types::Hash256;
        let s = store();
        let key1 = [0u8; 32];
        let key2 = [1u8; 32];
        s.put_bytes(DBColumn::BeaconBlock, &key1, b"v1").unwrap();
        s.put_bytes(DBColumn::BeaconBlock, &key2, b"v2").unwrap();

        // Start from key2 — should only get key2
        let keys: Vec<Hash256> = s
            .iter_column_keys_from::<Hash256>(DBColumn::BeaconBlock, &key2)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn empty_value_stored_and_retrieved() {
        let s = store();
        let key = [17u8; 32];
        s.put_bytes(DBColumn::BeaconBlock, &key, b"").unwrap();
        let got = s.get_bytes(DBColumn::BeaconBlock, &key).unwrap();
        assert_eq!(got, Some(vec![]));
    }

    #[test]
    fn large_value_stored_and_retrieved() {
        let s = store();
        let key = [18u8; 32];
        let val = vec![0xABu8; 1_000_000];
        s.put_bytes(DBColumn::BeaconBlock, &key, &val).unwrap();
        let got = s.get_bytes(DBColumn::BeaconBlock, &key).unwrap();
        assert_eq!(got, Some(val));
    }
}
