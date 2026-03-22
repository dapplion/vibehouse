//! This provides the logic for syncing a chain when the local node is far behind it's current
//! peers.
mod chain;
mod chain_collection;
mod range;
mod sync_type;

pub(crate) use chain::{ChainId, EPOCHS_PER_BATCH};
#[cfg(test)]
pub(crate) use chain_collection::SyncChainStatus;
pub(crate) use range::RangeSync;
pub(crate) use sync_type::RangeSyncType;
