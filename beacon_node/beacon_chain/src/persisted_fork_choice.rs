use crate::{beacon_fork_choice_store::PersistedForkChoiceStore, metrics};
use ssz::{Decode, Encode};
use ssz_derive::{Decode, Encode};
use store::{Error, KeyValueStoreOp, StoreConfig};
use types::Hash256;

#[derive(Encode, Decode)]
pub struct PersistedForkChoice {
    pub fork_choice: fork_choice::PersistedForkChoice,
    pub fork_choice_store: PersistedForkChoiceStore,
}

impl PersistedForkChoice {
    pub fn from_bytes(bytes: &[u8], store_config: &StoreConfig) -> Result<Self, Error> {
        let decompressed_bytes = store_config
            .decompress_bytes(bytes)
            .map_err(Error::Compression)?;
        Self::from_ssz_bytes(&decompressed_bytes).map_err(Into::into)
    }

    pub fn as_bytes(&self, store_config: &StoreConfig) -> Result<Vec<u8>, Error> {
        let encode_timer = metrics::start_timer(&metrics::FORK_CHOICE_ENCODE_TIMES);
        let ssz_bytes = self.as_ssz_bytes();
        drop(encode_timer);

        let _compress_timer = metrics::start_timer(&metrics::FORK_CHOICE_COMPRESS_TIMES);
        store_config
            .compress_bytes(&ssz_bytes)
            .map_err(Error::Compression)
    }

    pub fn as_kv_store_op(
        &self,
        key: Hash256,
        store_config: &StoreConfig,
    ) -> Result<KeyValueStoreOp, Error> {
        Ok(KeyValueStoreOp::PutKeyValue(
            store::DBColumn::ForkChoice,
            key.as_slice().to_vec(),
            self.as_bytes(store_config)?,
        ))
    }
}
