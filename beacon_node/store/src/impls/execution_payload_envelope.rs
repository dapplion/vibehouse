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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StoreItem;
    use types::{
        BlindedExecutionPayloadEnvelope, ExecutionPayloadEnvelope, ExecutionPayloadGloas,
        MinimalEthSpec,
    };

    type E = MinimalEthSpec;

    #[test]
    fn blinded_envelope_store_item_roundtrip() {
        let full_envelope = ExecutionPayloadEnvelope::<E> {
            builder_index: 42,
            payload: ExecutionPayloadGloas {
                gas_limit: 30_000_000,
                ..Default::default()
            },
            ..Default::default()
        };
        let blinded = BlindedExecutionPayloadEnvelope::from_full(&full_envelope);
        let signed = SignedBlindedExecutionPayloadEnvelope::<E> {
            message: blinded,
            signature: bls::Signature::empty(),
        };

        let bytes = signed.as_store_bytes();
        let decoded = SignedBlindedExecutionPayloadEnvelope::<E>::from_store_bytes(&bytes).unwrap();
        assert_eq!(signed, decoded);
        assert_eq!(decoded.message.builder_index, 42);
        assert_eq!(decoded.message.payload_header.gas_limit, 30_000_000);
    }

    #[test]
    fn blinded_envelope_uses_beacon_envelope_column() {
        assert_eq!(
            SignedBlindedExecutionPayloadEnvelope::<E>::db_column(),
            DBColumn::BeaconEnvelope
        );
    }

    #[test]
    fn blinded_envelope_from_store_bytes_invalid() {
        let result =
            SignedBlindedExecutionPayloadEnvelope::<E>::from_store_bytes(&[0xff, 0x00, 0x01]);
        assert!(result.is_err());
    }

    #[test]
    fn blinded_envelope_empty_roundtrip() {
        let full_envelope = ExecutionPayloadEnvelope::<E>::empty();
        let blinded = BlindedExecutionPayloadEnvelope::from_full(&full_envelope);
        let signed = SignedBlindedExecutionPayloadEnvelope::<E> {
            message: blinded,
            signature: bls::Signature::empty(),
        };

        let bytes = signed.as_store_bytes();
        let decoded = SignedBlindedExecutionPayloadEnvelope::<E>::from_store_bytes(&bytes).unwrap();
        assert_eq!(signed, decoded);
    }
}
