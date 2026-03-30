use super::{ActiveRequestItems, LookupVerifyError};
use std::sync::Arc;
use types::{DataColumnSidecar, EthSpec};
use vibehouse_network::rpc::methods::DataColumnsByRangeRequest;

/// Accumulates results of a data_columns_by_range request. Only returns items after receiving the
/// stream termination.
pub(crate) struct DataColumnsByRangeRequestItems<E: EthSpec> {
    request: DataColumnsByRangeRequest,
    items: Vec<Arc<DataColumnSidecar<E>>>,
}

impl<E: EthSpec> DataColumnsByRangeRequestItems<E> {
    pub(crate) fn new(request: DataColumnsByRangeRequest) -> Self {
        Self {
            request,
            items: vec![],
        }
    }
}

impl<E: EthSpec> ActiveRequestItems for DataColumnsByRangeRequestItems<E> {
    type Item = Arc<DataColumnSidecar<E>>;

    fn add(&mut self, data_column: Self::Item) -> Result<bool, LookupVerifyError> {
        if data_column.slot() < self.request.start_slot
            || data_column.slot() >= self.request.start_slot + self.request.count
        {
            return Err(LookupVerifyError::UnrequestedSlot(data_column.slot()));
        }
        if !self.request.columns.contains(&data_column.index()) {
            return Err(LookupVerifyError::UnrequestedIndex(data_column.index()));
        }
        if !data_column.verify_inclusion_proof() {
            return Err(LookupVerifyError::InvalidInclusionProof);
        }
        if self.items.iter().any(|existing| {
            existing.slot() == data_column.slot() && existing.index() == data_column.index()
        }) {
            return Err(LookupVerifyError::DuplicatedData(
                data_column.slot(),
                data_column.index(),
            ));
        }

        self.items.push(data_column);

        Ok(self.items.len() >= self.request.count as usize * self.request.columns.len())
    }

    fn consume(&mut self) -> Vec<Self::Item> {
        std::mem::take(&mut self.items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bls::Signature;
    use ssz_types::{FixedVector, VariableList};
    use types::{
        BeaconBlockHeader, DataColumnSidecarFulu, MinimalEthSpec, SignedBeaconBlockHeader, Slot,
    };

    type E = MinimalEthSpec;

    fn make_column(slot: u64, index: u64) -> Arc<DataColumnSidecar<E>> {
        Arc::new(DataColumnSidecar::Fulu(DataColumnSidecarFulu {
            index,
            column: VariableList::empty(),
            kzg_commitments: VariableList::empty(),
            kzg_proofs: VariableList::empty(),
            signed_block_header: SignedBeaconBlockHeader {
                message: BeaconBlockHeader {
                    slot: Slot::new(slot),
                    proposer_index: 0,
                    parent_root: Default::default(),
                    state_root: Default::default(),
                    body_root: Default::default(),
                },
                signature: Signature::empty(),
            },
            kzg_commitments_inclusion_proof: FixedVector::default(),
        }))
    }

    fn make_request(
        start_slot: u64,
        count: u64,
        columns: Vec<u64>,
    ) -> DataColumnsByRangeRequestItems<E> {
        DataColumnsByRangeRequestItems::new(DataColumnsByRangeRequest {
            start_slot,
            count,
            columns,
        })
    }

    #[test]
    fn reject_column_before_start_slot() {
        let mut req = make_request(10, 5, vec![0, 1]);
        assert_eq!(
            req.add(make_column(9, 0)),
            Err(LookupVerifyError::UnrequestedSlot(Slot::new(9)))
        );
    }

    #[test]
    fn reject_column_at_end_of_range() {
        let mut req = make_request(10, 5, vec![0, 1]);
        // Range is [10, 15), so slot 15 is out of range
        assert_eq!(
            req.add(make_column(15, 0)),
            Err(LookupVerifyError::UnrequestedSlot(Slot::new(15)))
        );
    }

    #[test]
    fn reject_column_well_beyond_range() {
        let mut req = make_request(10, 5, vec![0, 1]);
        assert_eq!(
            req.add(make_column(100, 0)),
            Err(LookupVerifyError::UnrequestedSlot(Slot::new(100)))
        );
    }

    #[test]
    fn reject_unrequested_column_index() {
        let mut req = make_request(10, 5, vec![0, 1]);
        // Column index 5 not in requested set [0, 1]
        assert_eq!(
            req.add(make_column(10, 5)),
            Err(LookupVerifyError::UnrequestedIndex(5))
        );
    }

    #[test]
    fn reject_unrequested_column_index_large() {
        let mut req = make_request(10, 5, vec![3, 7]);
        assert_eq!(
            req.add(make_column(10, 0)),
            Err(LookupVerifyError::UnrequestedIndex(0))
        );
    }

    #[test]
    fn consume_returns_empty_initially() {
        let mut req = make_request(10, 5, vec![0, 1]);
        assert!(req.consume().is_empty());
    }
}
