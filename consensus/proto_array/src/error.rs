use safe_arith::ArithError;
use types::{Checkpoint, Epoch, ExecutionBlockHash, Hash256, Slot};

#[derive(Clone, PartialEq, Debug)]
pub enum Error {
    FinalizedNodeUnknown(Hash256),
    JustifiedNodeUnknown(Hash256),
    NodeUnknown(Hash256),
    InvalidNodeIndex(usize),
    InvalidParentIndex(usize),
    InvalidBestChildIndex(usize),
    InvalidJustifiedIndex(usize),
    InvalidBestDescendant(usize),
    InvalidParentDelta(usize),
    InvalidNodeDelta(usize),
    DeltaOverflow(usize),
    ProposerBoostOverflow(usize),
    ReOrgThresholdOverflow,
    IndexOverflow(&'static str),
    InvalidExecutionDeltaOverflow(usize),
    InvalidDeltaLen {
        deltas: usize,
        indices: usize,
    },
    RevertedFinalizedEpoch {
        current_finalized_epoch: Epoch,
        new_finalized_epoch: Epoch,
    },
    InvalidBestNode(Box<InvalidBestNodeInfo>),
    InvalidAncestorOfValidPayload {
        ancestor_block_root: Hash256,
        ancestor_payload_block_hash: ExecutionBlockHash,
    },
    ValidExecutionStatusBecameInvalid {
        block_root: Hash256,
        payload_block_hash: ExecutionBlockHash,
    },
    InvalidJustifiedCheckpointExecutionStatus {
        justified_root: Hash256,
    },
    IrrelevantDescendant {
        block_root: Hash256,
    },
    ParentExecutionStatusIsInvalid {
        block_root: Hash256,
        parent_root: Hash256,
    },
    InvalidEpochOffset(u64),
    Arith(ArithError),
}

impl From<ArithError> for Error {
    fn from(e: ArithError) -> Self {
        Error::Arith(e)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct InvalidBestNodeInfo {
    pub current_slot: Slot,
    pub start_root: Hash256,
    pub justified_checkpoint: Checkpoint,
    pub finalized_checkpoint: Checkpoint,
    pub head_root: Hash256,
    pub head_justified_checkpoint: Checkpoint,
    pub head_finalized_checkpoint: Checkpoint,
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::FixedBytesExtended;

    #[test]
    fn from_arith_error() {
        let err: Error = ArithError::Overflow.into();
        assert_eq!(err, Error::Arith(ArithError::Overflow));
    }

    #[test]
    fn error_equality() {
        assert_eq!(
            Error::NodeUnknown(Hash256::zero()),
            Error::NodeUnknown(Hash256::zero())
        );
        assert_ne!(
            Error::NodeUnknown(Hash256::zero()),
            Error::NodeUnknown(Hash256::repeat_byte(1))
        );
    }

    #[test]
    fn error_clone() {
        let err = Error::InvalidNodeIndex(42);
        assert_eq!(err.clone(), err);
    }

    #[test]
    fn error_debug() {
        let err = Error::FinalizedNodeUnknown(Hash256::zero());
        let dbg = format!("{:?}", err);
        assert!(dbg.contains("FinalizedNodeUnknown"));
    }

    #[test]
    fn invalid_delta_len_fields() {
        let err = Error::InvalidDeltaLen {
            deltas: 10,
            indices: 20,
        };
        if let Error::InvalidDeltaLen { deltas, indices } = err {
            assert_eq!(deltas, 10);
            assert_eq!(indices, 20);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn reverted_finalized_epoch_fields() {
        let err = Error::RevertedFinalizedEpoch {
            current_finalized_epoch: Epoch::new(5),
            new_finalized_epoch: Epoch::new(3),
        };
        if let Error::RevertedFinalizedEpoch {
            current_finalized_epoch,
            new_finalized_epoch,
        } = err
        {
            assert_eq!(current_finalized_epoch, Epoch::new(5));
            assert_eq!(new_finalized_epoch, Epoch::new(3));
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn invalid_best_node_info_clone_eq() {
        let info = InvalidBestNodeInfo {
            current_slot: Slot::new(1),
            start_root: Hash256::zero(),
            justified_checkpoint: Checkpoint {
                epoch: Epoch::new(0),
                root: Hash256::zero(),
            },
            finalized_checkpoint: Checkpoint {
                epoch: Epoch::new(0),
                root: Hash256::zero(),
            },
            head_root: Hash256::repeat_byte(1),
            head_justified_checkpoint: Checkpoint {
                epoch: Epoch::new(1),
                root: Hash256::repeat_byte(1),
            },
            head_finalized_checkpoint: Checkpoint {
                epoch: Epoch::new(0),
                root: Hash256::zero(),
            },
        };
        assert_eq!(info.clone(), info);
    }

    #[test]
    fn invalid_best_node_wraps_in_box() {
        let info = InvalidBestNodeInfo {
            current_slot: Slot::new(0),
            start_root: Hash256::zero(),
            justified_checkpoint: Checkpoint {
                epoch: Epoch::new(0),
                root: Hash256::zero(),
            },
            finalized_checkpoint: Checkpoint {
                epoch: Epoch::new(0),
                root: Hash256::zero(),
            },
            head_root: Hash256::zero(),
            head_justified_checkpoint: Checkpoint {
                epoch: Epoch::new(0),
                root: Hash256::zero(),
            },
            head_finalized_checkpoint: Checkpoint {
                epoch: Epoch::new(0),
                root: Hash256::zero(),
            },
        };
        let err = Error::InvalidBestNode(Box::new(info.clone()));
        if let Error::InvalidBestNode(boxed) = &err {
            assert_eq!(**boxed, info);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn execution_related_errors() {
        let root = Hash256::repeat_byte(0xaa);
        let hash = ExecutionBlockHash::from_root(root);

        let err1 = Error::InvalidAncestorOfValidPayload {
            ancestor_block_root: root,
            ancestor_payload_block_hash: hash,
        };
        let err2 = Error::ValidExecutionStatusBecameInvalid {
            block_root: root,
            payload_block_hash: hash,
        };
        let err3 = Error::ParentExecutionStatusIsInvalid {
            block_root: root,
            parent_root: Hash256::zero(),
        };

        // Just verify they're constructable and debuggable
        assert!(format!("{:?}", err1).contains("InvalidAncestor"));
        assert!(format!("{:?}", err2).contains("ValidExecution"));
        assert!(format!("{:?}", err3).contains("ParentExecution"));
    }
}
