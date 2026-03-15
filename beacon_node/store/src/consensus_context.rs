use ssz_derive::{Decode, Encode};
use state_processing::ConsensusContext;
use std::collections::HashMap;
use types::{EthSpec, Hash256, IndexedAttestation, Slot};

/// The consensus context is stored on disk as part of the data availability overflow cache.
///
/// We use this separate struct to keep the on-disk format stable in the presence of changes to the
/// in-memory `ConsensusContext`. You MUST NOT change the fields of this struct without
/// superstructing it and implementing a schema migration.
#[derive(Debug, PartialEq, Clone, Encode, Decode)]
pub struct OnDiskConsensusContext<E: EthSpec> {
    /// Slot to act as an identifier/safeguard
    slot: Slot,
    /// Proposer index of the block at `slot`.
    proposer_index: Option<u64>,
    /// Block root of the block at `slot`.
    current_block_root: Option<Hash256>,
    /// We keep the indexed attestations in the *in-memory* version of this struct so that we don't
    /// need to regenerate them if roundtripping via this type *without* going to disk.
    ///
    /// They are not part of the on-disk format.
    #[ssz(skip_serializing, skip_deserializing)]
    indexed_attestations: HashMap<Hash256, IndexedAttestation<E>>,
}

impl<E: EthSpec> OnDiskConsensusContext<E> {
    pub fn from_consensus_context(ctxt: ConsensusContext<E>) -> Self {
        // Match exhaustively on fields here so we are forced to *consider* updating the on-disk
        // format when the `ConsensusContext` fields change.
        let ConsensusContext {
            slot,
            previous_epoch: _,
            current_epoch: _,
            proposer_index,
            current_block_root,
            indexed_attestations,
        } = ctxt;
        OnDiskConsensusContext {
            slot,
            proposer_index,
            current_block_root,
            indexed_attestations,
        }
    }

    pub fn into_consensus_context(self) -> ConsensusContext<E> {
        let OnDiskConsensusContext {
            slot,
            proposer_index,
            current_block_root,
            indexed_attestations,
        } = self;

        let mut ctxt = ConsensusContext::new(slot);

        if let Some(proposer_index) = proposer_index {
            ctxt = ctxt.set_proposer_index(proposer_index);
        }
        if let Some(block_root) = current_block_root {
            ctxt = ctxt.set_current_block_root(block_root);
        }
        ctxt.set_indexed_attestations(indexed_attestations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssz::{Decode, Encode};
    use types::MinimalEthSpec;

    type E = MinimalEthSpec;

    #[test]
    fn roundtrip_with_all_fields() {
        let block_root = Hash256::repeat_byte(0xab);
        let ctxt = ConsensusContext::<E>::new(Slot::new(42))
            .set_proposer_index(7)
            .set_current_block_root(block_root);

        let on_disk = OnDiskConsensusContext::from_consensus_context(ctxt);
        let recovered = on_disk.into_consensus_context();

        assert_eq!(recovered.slot, Slot::new(42));
        assert_eq!(recovered.proposer_index, Some(7));
        assert_eq!(recovered.current_block_root, Some(block_root));
    }

    #[test]
    fn roundtrip_with_no_optional_fields() {
        let ctxt = ConsensusContext::<E>::new(Slot::new(100));
        let on_disk = OnDiskConsensusContext::from_consensus_context(ctxt);
        let recovered = on_disk.into_consensus_context();

        assert_eq!(recovered.slot, Slot::new(100));
        assert_eq!(recovered.proposer_index, None);
        assert_eq!(recovered.current_block_root, None);
    }

    #[test]
    fn ssz_roundtrip_strips_indexed_attestations() {
        let block_root = Hash256::repeat_byte(0x01);
        let ctxt = ConsensusContext::<E>::new(Slot::new(10))
            .set_proposer_index(3)
            .set_current_block_root(block_root);

        let on_disk = OnDiskConsensusContext::from_consensus_context(ctxt);
        let bytes = on_disk.as_ssz_bytes();
        let decoded = OnDiskConsensusContext::<E>::from_ssz_bytes(&bytes).unwrap();

        // indexed_attestations are skipped in SSZ, so they should be empty after decode
        assert!(decoded.indexed_attestations.is_empty());
        // Other fields preserved
        assert_eq!(decoded.slot, Slot::new(10));
        assert_eq!(decoded.proposer_index, Some(3));
        assert_eq!(decoded.current_block_root, Some(block_root));
    }

    #[test]
    fn clone_preserves_fields() {
        let ctxt = ConsensusContext::<E>::new(Slot::new(50)).set_proposer_index(99);
        let on_disk = OnDiskConsensusContext::from_consensus_context(ctxt);
        let cloned = on_disk.clone();
        assert_eq!(on_disk, cloned);
    }

    #[test]
    fn epoch_computed_correctly_on_recovery() {
        // Slot 64 with 8 slots/epoch = epoch 8, previous = 7
        let ctxt = ConsensusContext::<E>::new(Slot::new(64));
        let on_disk = OnDiskConsensusContext::from_consensus_context(ctxt);
        let recovered = on_disk.into_consensus_context();
        assert_eq!(
            recovered.current_epoch,
            Slot::new(64).epoch(E::slots_per_epoch())
        );
        assert_eq!(
            recovered.previous_epoch,
            Slot::new(64)
                .epoch(E::slots_per_epoch())
                .saturating_sub(1u64)
        );
    }
}
