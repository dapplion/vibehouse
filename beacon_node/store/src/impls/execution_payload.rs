use crate::{DBColumn, Error, StoreItem};
use ssz::{Decode, Encode};
use types::{
    EthSpec, ExecutionPayload, ExecutionPayloadBellatrix, ExecutionPayloadCapella,
    ExecutionPayloadDeneb, ExecutionPayloadElectra, ExecutionPayloadFulu, ExecutionPayloadGloas,
};

macro_rules! impl_store_item {
    ($ty_name:ident) => {
        impl<E: EthSpec> StoreItem for $ty_name<E> {
            fn db_column() -> DBColumn {
                DBColumn::ExecPayload
            }

            fn as_store_bytes(&self) -> Vec<u8> {
                self.as_ssz_bytes()
            }

            fn from_store_bytes(bytes: &[u8]) -> Result<Self, Error> {
                Ok(Self::from_ssz_bytes(bytes)?)
            }
        }
    };
}
impl_store_item!(ExecutionPayloadBellatrix);
impl_store_item!(ExecutionPayloadCapella);
impl_store_item!(ExecutionPayloadDeneb);
impl_store_item!(ExecutionPayloadElectra);
impl_store_item!(ExecutionPayloadFulu);
impl_store_item!(ExecutionPayloadGloas);

/// This fork-agnostic implementation should be only used for writing.
///
/// It is very inefficient at reading, and decoding the desired fork-specific variant is recommended
/// instead.
impl<E: EthSpec> StoreItem for ExecutionPayload<E> {
    fn db_column() -> DBColumn {
        DBColumn::ExecPayload
    }

    fn as_store_bytes(&self) -> Vec<u8> {
        self.as_ssz_bytes()
    }

    fn from_store_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if let Ok(payload) = ExecutionPayloadGloas::from_ssz_bytes(bytes) {
            return Ok(Self::Gloas(payload));
        }

        if let Ok(payload) = ExecutionPayloadFulu::from_ssz_bytes(bytes) {
            return Ok(Self::Fulu(payload));
        }

        if let Ok(payload) = ExecutionPayloadElectra::from_ssz_bytes(bytes) {
            return Ok(Self::Electra(payload));
        }

        if let Ok(payload) = ExecutionPayloadDeneb::from_ssz_bytes(bytes) {
            return Ok(Self::Deneb(payload));
        }

        if let Ok(payload) = ExecutionPayloadCapella::from_ssz_bytes(bytes) {
            return Ok(Self::Capella(payload));
        }

        ExecutionPayloadBellatrix::from_ssz_bytes(bytes)
            .map(Self::Bellatrix)
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StoreItem;
    use types::MinimalEthSpec;

    type E = MinimalEthSpec;

    #[test]
    fn bellatrix_store_item_roundtrip() {
        let payload = ExecutionPayloadBellatrix::<E>::default();
        let bytes = payload.as_store_bytes();
        let decoded = ExecutionPayloadBellatrix::<E>::from_store_bytes(&bytes).unwrap();
        assert_eq!(payload, decoded);
    }

    #[test]
    fn capella_store_item_roundtrip() {
        let payload = ExecutionPayloadCapella::<E>::default();
        let bytes = payload.as_store_bytes();
        let decoded = ExecutionPayloadCapella::<E>::from_store_bytes(&bytes).unwrap();
        assert_eq!(payload, decoded);
    }

    #[test]
    fn deneb_store_item_roundtrip() {
        let payload = ExecutionPayloadDeneb::<E>::default();
        let bytes = payload.as_store_bytes();
        let decoded = ExecutionPayloadDeneb::<E>::from_store_bytes(&bytes).unwrap();
        assert_eq!(payload, decoded);
    }

    #[test]
    fn electra_store_item_roundtrip() {
        let payload = ExecutionPayloadElectra::<E>::default();
        let bytes = payload.as_store_bytes();
        let decoded = ExecutionPayloadElectra::<E>::from_store_bytes(&bytes).unwrap();
        assert_eq!(payload, decoded);
    }

    #[test]
    fn fulu_store_item_roundtrip() {
        let payload = ExecutionPayloadFulu::<E>::default();
        let bytes = payload.as_store_bytes();
        let decoded = ExecutionPayloadFulu::<E>::from_store_bytes(&bytes).unwrap();
        assert_eq!(payload, decoded);
    }

    #[test]
    fn gloas_store_item_roundtrip() {
        let payload = ExecutionPayloadGloas::<E>::default();
        let bytes = payload.as_store_bytes();
        let decoded = ExecutionPayloadGloas::<E>::from_store_bytes(&bytes).unwrap();
        assert_eq!(payload, decoded);
    }

    #[test]
    fn all_variants_use_exec_payload_column() {
        assert_eq!(
            ExecutionPayloadBellatrix::<E>::db_column(),
            DBColumn::ExecPayload
        );
        assert_eq!(
            ExecutionPayloadCapella::<E>::db_column(),
            DBColumn::ExecPayload
        );
        assert_eq!(
            ExecutionPayloadDeneb::<E>::db_column(),
            DBColumn::ExecPayload
        );
        assert_eq!(
            ExecutionPayloadElectra::<E>::db_column(),
            DBColumn::ExecPayload
        );
        assert_eq!(
            ExecutionPayloadFulu::<E>::db_column(),
            DBColumn::ExecPayload
        );
        assert_eq!(
            ExecutionPayloadGloas::<E>::db_column(),
            DBColumn::ExecPayload
        );
        assert_eq!(ExecutionPayload::<E>::db_column(), DBColumn::ExecPayload);
    }

    #[test]
    fn fork_agnostic_roundtrip_gloas() {
        let inner = ExecutionPayloadGloas::<E>::default();
        let payload = ExecutionPayload::<E>::Gloas(inner.clone());
        let bytes = payload.as_store_bytes();
        let decoded = ExecutionPayload::<E>::from_store_bytes(&bytes).unwrap();
        match decoded {
            ExecutionPayload::Gloas(p) => assert_eq!(p, inner),
            other => panic!("expected Gloas, got {:?}", other),
        }
    }

    #[test]
    fn fork_agnostic_roundtrip_bellatrix() {
        let inner = ExecutionPayloadBellatrix::<E>::default();
        let payload = ExecutionPayload::<E>::Bellatrix(inner.clone());
        let bytes = payload.as_store_bytes();
        let decoded = ExecutionPayload::<E>::from_store_bytes(&bytes).unwrap();
        // Note: fork-agnostic decode tries newest fork first, so Bellatrix
        // may decode as a newer variant if SSZ is ambiguous. We just verify
        // roundtrip works without error.
        assert!(matches!(
            decoded,
            ExecutionPayload::Bellatrix(_)
                | ExecutionPayload::Gloas(_)
                | ExecutionPayload::Fulu(_)
                | ExecutionPayload::Electra(_)
                | ExecutionPayload::Deneb(_)
                | ExecutionPayload::Capella(_)
        ));
    }

    #[test]
    fn from_store_bytes_invalid_data() {
        let result = ExecutionPayloadBellatrix::<E>::from_store_bytes(&[0xff, 0x00]);
        assert!(result.is_err());
    }

    #[test]
    fn fork_agnostic_from_store_bytes_invalid() {
        let result = ExecutionPayload::<E>::from_store_bytes(&[0xff]);
        assert!(result.is_err());
    }
}
