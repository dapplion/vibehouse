use ssz_types::VariableList;
use std::sync::Arc;
use types::{
    ChainSpec, DataColumnSidecar, DataColumnsByRootIdentifier, EthSpec, ForkName, Hash256,
};
use vibehouse_network::rpc::methods::DataColumnsByRootRequest;

use super::{ActiveRequestItems, LookupVerifyError};

#[derive(Debug, Clone)]
pub(crate) struct DataColumnsByRootSingleBlockRequest {
    pub block_root: Hash256,
    pub indices: Vec<u64>,
}

impl DataColumnsByRootSingleBlockRequest {
    pub(crate) fn try_into_request<E: EthSpec>(
        self,
        fork_name: ForkName,
        spec: &ChainSpec,
    ) -> Result<DataColumnsByRootRequest<E>, &'static str> {
        let columns = VariableList::new(self.indices)
            .map_err(|_| "Number of indices exceeds total number of columns")?;
        DataColumnsByRootRequest::new(
            vec![DataColumnsByRootIdentifier {
                block_root: self.block_root,
                columns,
            }],
            spec.max_request_blocks(fork_name),
        )
    }
}

pub(crate) struct DataColumnsByRootRequestItems<E: EthSpec> {
    request: DataColumnsByRootSingleBlockRequest,
    items: Vec<Arc<DataColumnSidecar<E>>>,
}

impl<E: EthSpec> DataColumnsByRootRequestItems<E> {
    pub(crate) fn new(request: DataColumnsByRootSingleBlockRequest) -> Self {
        Self {
            request,
            items: vec![],
        }
    }
}

impl<E: EthSpec> ActiveRequestItems for DataColumnsByRootRequestItems<E> {
    type Item = Arc<DataColumnSidecar<E>>;

    /// Appends a chunk to this multi-item request. If all expected chunks are received, this
    /// method returns `Some`, resolving the request before the stream terminator.
    /// The active request SHOULD be dropped after `add_response` returns an error
    fn add(&mut self, data_column: Self::Item) -> Result<bool, LookupVerifyError> {
        let block_root = data_column.block_root();
        if self.request.block_root != block_root {
            return Err(LookupVerifyError::UnrequestedBlockRoot(block_root));
        }
        if !data_column.verify_inclusion_proof() {
            return Err(LookupVerifyError::InvalidInclusionProof);
        }
        if !self.request.indices.contains(&data_column.index()) {
            return Err(LookupVerifyError::UnrequestedIndex(data_column.index()));
        }
        if self.items.iter().any(|d| d.index() == data_column.index()) {
            return Err(LookupVerifyError::DuplicatedData(
                data_column.slot(),
                data_column.index(),
            ));
        }

        self.items.push(data_column);

        Ok(self.items.len() >= self.request.indices.len())
    }

    fn consume(&mut self) -> Vec<Self::Item> {
        std::mem::take(&mut self.items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssz_types::VariableList;
    use types::{DataColumnSidecarGloas, FixedBytesExtended, MinimalEthSpec, Slot};

    type E = MinimalEthSpec;

    fn make_column_with_root(root: Hash256, index: u64) -> Arc<DataColumnSidecar<E>> {
        Arc::new(DataColumnSidecar::Gloas(DataColumnSidecarGloas {
            index,
            column: VariableList::empty(),
            kzg_proofs: VariableList::empty(),
            slot: Slot::new(10),
            beacon_block_root: root,
        }))
    }

    #[test]
    fn reject_column_with_wrong_block_root() {
        let actual_root = Hash256::repeat_byte(0xAA);
        let wrong_root = Hash256::repeat_byte(0xFF);
        let column = make_column_with_root(actual_root, 0);
        let mut req =
            DataColumnsByRootRequestItems::<E>::new(DataColumnsByRootSingleBlockRequest {
                block_root: wrong_root,
                indices: vec![0],
            });
        assert_eq!(
            req.add(column),
            Err(LookupVerifyError::UnrequestedBlockRoot(actual_root))
        );
    }

    #[test]
    fn consume_returns_empty_initially() {
        let mut req =
            DataColumnsByRootRequestItems::<E>::new(DataColumnsByRootSingleBlockRequest {
                block_root: Hash256::zero(),
                indices: vec![0],
            });
        assert!(req.consume().is_empty());
    }
}
