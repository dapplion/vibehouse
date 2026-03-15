use crate::OperationPool;
use crate::attestation_storage::AttestationMap;
use crate::bls_to_execution_changes::{BlsToExecutionChanges, ReceivedPreCapella};
use crate::sync_aggregate_id::SyncAggregateId;
use educe::Educe;
use parking_lot::RwLock;
use ssz::{Decode, Encode};
use ssz_derive::{Decode, Encode};
use state_processing::SigVerifiedOp;
use std::collections::HashSet;
use std::mem;
use store::{DBColumn, Error as StoreError, StoreItem};
use types::attestation::AttestationOnDisk;
use types::*;

type PersistedSyncContributions<E> = Vec<(SyncAggregateId, Vec<SyncCommitteeContribution<E>>)>;

/// SSZ-serializable version of `OperationPool`.
///
/// Operations are stored in arbitrary order, so it's not a good idea to compare instances
/// of this type (or its encoded form) for equality. Convert back to an `OperationPool` first.
#[derive(Educe, PartialEq, Debug, Encode, Decode)]
#[educe(Clone)]
pub struct PersistedOperationPool<E: EthSpec> {
    /// Attestations and their attesting indices.
    pub attestations: Vec<(AttestationOnDisk<E>, Vec<u64>)>,
    /// Mapping from sync contribution ID to sync contributions and aggregate.
    pub sync_contributions: PersistedSyncContributions<E>,
    /// Attester slashings.
    pub attester_slashings: Vec<SigVerifiedOp<AttesterSlashing<E>, E>>,
    /// Proposer slashings with fork information.
    pub proposer_slashings: Vec<SigVerifiedOp<ProposerSlashing, E>>,
    /// Voluntary exits with fork information.
    pub voluntary_exits: Vec<SigVerifiedOp<SignedVoluntaryExit, E>>,
    /// BLS to Execution Changes
    pub bls_to_execution_changes: Vec<SigVerifiedOp<SignedBlsToExecutionChange, E>>,
    /// Validator indices with BLS to Execution Changes to be broadcast at the
    /// Capella fork.
    pub capella_bls_change_broadcast_indices: Vec<u64>,
}

impl<E: EthSpec> PersistedOperationPool<E> {
    /// Convert an `OperationPool` into serializable form.
    pub fn from_operation_pool(operation_pool: &OperationPool<E>) -> Self {
        let attestations = operation_pool
            .attestations
            .read()
            .iter()
            .map(|att| {
                (
                    AttestationOnDisk::from(att.clone_as_attestation()),
                    att.indexed.attesting_indices().clone(),
                )
            })
            .collect();

        let sync_contributions = operation_pool
            .sync_contributions
            .read()
            .iter()
            .map(|(id, contribution)| (id.clone(), contribution.clone()))
            .collect();

        let attester_slashings = operation_pool
            .attester_slashings
            .read()
            .iter()
            .cloned()
            .collect();

        let proposer_slashings = operation_pool
            .proposer_slashings
            .read()
            .values()
            .cloned()
            .collect();

        let voluntary_exits = operation_pool
            .voluntary_exits
            .read()
            .values()
            .cloned()
            .collect();

        let bls_to_execution_changes = operation_pool
            .bls_to_execution_changes
            .read()
            .iter_fifo()
            .map(|bls_to_execution_change| (**bls_to_execution_change).clone())
            .collect();

        let capella_bls_change_broadcast_indices = operation_pool
            .bls_to_execution_changes
            .read()
            .iter_pre_capella_indices()
            .copied()
            .collect();

        PersistedOperationPool {
            attestations,
            sync_contributions,
            attester_slashings,
            proposer_slashings,
            voluntary_exits,
            bls_to_execution_changes,
            capella_bls_change_broadcast_indices,
        }
    }

    /// Reconstruct an `OperationPool`.
    pub fn into_operation_pool(mut self) -> OperationPool<E> {
        let attester_slashings = RwLock::new(self.attester_slashings.iter().cloned().collect());

        let proposer_slashings = RwLock::new(
            self.proposer_slashings
                .iter()
                .cloned()
                .map(|slashing| (slashing.as_inner().proposer_index(), slashing))
                .collect(),
        );
        let voluntary_exits = RwLock::new(
            self.voluntary_exits
                .iter()
                .cloned()
                .map(|exit| (exit.as_inner().message.validator_index, exit))
                .collect(),
        );
        let sync_contributions = RwLock::new(self.sync_contributions.iter().cloned().collect());

        let mut map = AttestationMap::default();
        for (att, attesting_indices) in self.attestations.iter().map(|(att, attesting_indices)| {
            (
                AttestationRef::from(att.to_ref()).clone_as_attestation(),
                attesting_indices.clone(),
            )
        }) {
            map.insert(att, attesting_indices);
        }
        let attestations = RwLock::new(map);

        let mut bls_to_execution_changes = BlsToExecutionChanges::default();
        let persisted_changes = mem::take(&mut self.bls_to_execution_changes);
        let broadcast_indices: HashSet<_> =
            mem::take(&mut self.capella_bls_change_broadcast_indices)
                .into_iter()
                .collect();

        for bls_to_execution_change in persisted_changes {
            let received_pre_capella = if broadcast_indices
                .contains(&bls_to_execution_change.as_inner().message.validator_index)
            {
                ReceivedPreCapella::Yes
            } else {
                ReceivedPreCapella::No
            };
            bls_to_execution_changes.insert(bls_to_execution_change, received_pre_capella);
        }

        OperationPool {
            attestations,
            sync_contributions,
            attester_slashings,
            proposer_slashings,
            voluntary_exits,
            bls_to_execution_changes: RwLock::new(bls_to_execution_changes),
            reward_cache: Default::default(),
            _phantom: Default::default(),
        }
    }
}

impl<E: EthSpec> StoreItem for PersistedOperationPool<E> {
    fn db_column() -> DBColumn {
        DBColumn::OpPool
    }

    fn as_store_bytes(&self) -> Vec<u8> {
        self.as_ssz_bytes()
    }

    fn from_store_bytes(bytes: &[u8]) -> Result<Self, StoreError> {
        PersistedOperationPool::from_ssz_bytes(bytes).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::MinimalEthSpec;

    type E = MinimalEthSpec;

    #[test]
    fn empty_pool_ssz_roundtrip() {
        let pool = PersistedOperationPool::<E> {
            attestations: vec![],
            sync_contributions: vec![],
            attester_slashings: vec![],
            proposer_slashings: vec![],
            voluntary_exits: vec![],
            bls_to_execution_changes: vec![],
            capella_bls_change_broadcast_indices: vec![],
        };
        let bytes = pool.as_ssz_bytes();
        let decoded = PersistedOperationPool::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(pool, decoded);
    }

    #[test]
    fn store_item_column() {
        assert_eq!(PersistedOperationPool::<E>::db_column(), DBColumn::OpPool);
    }

    #[test]
    fn store_item_roundtrip() {
        let pool = PersistedOperationPool::<E> {
            attestations: vec![],
            sync_contributions: vec![],
            attester_slashings: vec![],
            proposer_slashings: vec![],
            voluntary_exits: vec![],
            bls_to_execution_changes: vec![],
            capella_bls_change_broadcast_indices: vec![1, 2, 3],
        };
        let bytes = pool.as_store_bytes();
        let decoded = PersistedOperationPool::<E>::from_store_bytes(&bytes).unwrap();
        assert_eq!(pool, decoded);
    }

    #[test]
    fn store_item_invalid_bytes() {
        let result = PersistedOperationPool::<E>::from_store_bytes(&[0xff, 0x00]);
        assert!(result.is_err());
    }

    #[test]
    fn into_operation_pool_empty() {
        let pool = PersistedOperationPool::<E> {
            attestations: vec![],
            sync_contributions: vec![],
            attester_slashings: vec![],
            proposer_slashings: vec![],
            voluntary_exits: vec![],
            bls_to_execution_changes: vec![],
            capella_bls_change_broadcast_indices: vec![],
        };
        let op_pool = pool.into_operation_pool();
        assert!(op_pool.attester_slashings.read().is_empty());
        assert!(op_pool.proposer_slashings.read().is_empty());
        assert!(op_pool.voluntary_exits.read().is_empty());
    }

    #[test]
    fn broadcast_indices_preserved() {
        let pool = PersistedOperationPool::<E> {
            attestations: vec![],
            sync_contributions: vec![],
            attester_slashings: vec![],
            proposer_slashings: vec![],
            voluntary_exits: vec![],
            bls_to_execution_changes: vec![],
            capella_bls_change_broadcast_indices: vec![42, 99],
        };
        let bytes = pool.as_ssz_bytes();
        let decoded = PersistedOperationPool::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(decoded.capella_bls_change_broadcast_indices, vec![42, 99]);
    }

    #[test]
    fn clone_preserves_fields() {
        let pool = PersistedOperationPool::<E> {
            attestations: vec![],
            sync_contributions: vec![],
            attester_slashings: vec![],
            proposer_slashings: vec![],
            voluntary_exits: vec![],
            bls_to_execution_changes: vec![],
            capella_bls_change_broadcast_indices: vec![1],
        };
        assert_eq!(pool.clone(), pool);
    }
}
