//! Utilities for managing database schema changes.
//!
//! vibehouse only supports the current schema version. Legacy migrations from
//! Vibehouse (v22 through v28) have been removed since vibehouse has never run
//! with an older schema.

use crate::beacon_chain::BeaconChainTypes;
use std::sync::Arc;
use store::Error as StoreError;
use store::hot_cold_store::{HotColdDB, HotColdDBError};
use store::metadata::{CURRENT_SCHEMA_VERSION, SchemaVersion};

/// Migrate the database from one schema version to another.
///
/// vibehouse only supports the current schema version, so the only valid
/// migration is the identity (from == to == CURRENT_SCHEMA_VERSION).
pub fn migrate_schema<T: BeaconChainTypes>(
    _db: Arc<HotColdDB<T::EthSpec, T::HotStore, T::ColdStore>>,
    from: SchemaVersion,
    to: SchemaVersion,
) -> Result<(), StoreError> {
    if from == to && to == CURRENT_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(HotColdDBError::UnsupportedSchemaVersion {
            target_version: to,
            current_version: from,
        }
        .into())
    }
}
