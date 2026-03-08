use crate::database::redb_impl;
use crate::{ColumnIter, ColumnKeyIter, DBColumn, Error, ItemStore, Key, KeyValueStore, metrics};
use crate::{KeyValueStoreOp, StoreConfig};
use std::collections::HashSet;
use std::path::Path;
use types::EthSpec;

pub struct BeaconNodeBackend<E: EthSpec>(redb_impl::Redb<E>);

impl<E: EthSpec> ItemStore<E> for BeaconNodeBackend<E> {}

impl<E: EthSpec> KeyValueStore<E> for BeaconNodeBackend<E> {
    fn get_bytes(&self, column: DBColumn, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        redb_impl::Redb::get_bytes(&self.0, column, key)
    }

    fn put_bytes(&self, column: DBColumn, key: &[u8], value: &[u8]) -> Result<(), Error> {
        redb_impl::Redb::put_bytes_with_options(&self.0, column, key, value, self.0.write_options())
    }

    fn put_bytes_sync(&self, column: DBColumn, key: &[u8], value: &[u8]) -> Result<(), Error> {
        redb_impl::Redb::put_bytes_with_options(
            &self.0,
            column,
            key,
            value,
            self.0.write_options_sync(),
        )
    }

    fn sync(&self) -> Result<(), Error> {
        redb_impl::Redb::sync(&self.0)
    }

    fn key_exists(&self, column: DBColumn, key: &[u8]) -> Result<bool, Error> {
        redb_impl::Redb::key_exists(&self.0, column, key)
    }

    fn key_delete(&self, column: DBColumn, key: &[u8]) -> Result<(), Error> {
        redb_impl::Redb::key_delete(&self.0, column, key)
    }

    fn do_atomically(&self, batch: Vec<KeyValueStoreOp>) -> Result<(), Error> {
        redb_impl::Redb::do_atomically(&self.0, batch)
    }

    fn compact(&self) -> Result<(), Error> {
        redb_impl::Redb::compact(&self.0)
    }

    fn iter_column_keys_from<K: Key>(&self, column: DBColumn, from: &[u8]) -> ColumnKeyIter<'_, K> {
        redb_impl::Redb::iter_column_keys_from(&self.0, column, from)
    }

    fn iter_column_keys<K: Key>(&self, column: DBColumn) -> ColumnKeyIter<'_, K> {
        redb_impl::Redb::iter_column_keys(&self.0, column)
    }

    fn iter_column_from<K: Key>(&self, column: DBColumn, from: &[u8]) -> ColumnIter<'_, K> {
        redb_impl::Redb::iter_column_from(&self.0, column, from)
    }

    fn compact_column(&self, _column: DBColumn) -> Result<(), Error> {
        redb_impl::Redb::compact(&self.0)
    }

    fn delete_batch(&self, col: DBColumn, ops: HashSet<&[u8]>) -> Result<(), Error> {
        redb_impl::Redb::delete_batch(&self.0, col, ops)
    }

    fn delete_if(
        &self,
        column: DBColumn,
        f: impl FnMut(&[u8]) -> Result<bool, Error>,
    ) -> Result<(), Error> {
        redb_impl::Redb::delete_if(&self.0, column, f)
    }
}

impl<E: EthSpec> BeaconNodeBackend<E> {
    pub fn open(config: &StoreConfig, path: &Path) -> Result<Self, Error> {
        let _ = config; // backend field removed, always redb
        metrics::inc_counter_vec(&metrics::DISK_DB_TYPE, &["redb"]);
        redb_impl::Redb::open(path).map(BeaconNodeBackend)
    }
}

pub struct WriteOptions {
    /// fsync before acknowledging a write operation.
    pub sync: bool,
}

impl WriteOptions {
    pub fn new() -> Self {
        WriteOptions { sync: false }
    }
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self::new()
    }
}
