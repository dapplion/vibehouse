use crate::{DBColumn, Error, StoreItem};
use ssz::{Decode, Encode};
use types::{EthSpec, SignedBlindedExecutionPayloadEnvelope};

/// The `BeaconEnvelope` column stores blinded envelopes (header instead of full
/// payload). The full execution payload is stored separately in `ExecPayload`.
impl<E: EthSpec> StoreItem for SignedBlindedExecutionPayloadEnvelope<E> {
    fn db_column() -> DBColumn {
        DBColumn::BeaconEnvelope
    }

    fn as_store_bytes(&self) -> Vec<u8> {
        self.as_ssz_bytes()
    }

    fn from_store_bytes(bytes: &[u8]) -> Result<Self, Error> {
        Ok(Self::from_ssz_bytes(bytes)?)
    }
}
