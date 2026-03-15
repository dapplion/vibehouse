use serde::{Deserialize, Serialize};
use types::Slot;

/// The current state of the node.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SyncState {
    /// The node is performing a long-range (batch) sync over a finalized chain.
    /// In this state, parent lookups are disabled.
    SyncingFinalized { start_slot: Slot, target_slot: Slot },
    /// The node is performing a long-range (batch) sync over one or many head chains.
    /// In this state parent lookups are disabled.
    SyncingHead { start_slot: Slot, target_slot: Slot },
    /// The node is undertaking a backfill sync. This occurs when a user has specified a trusted
    /// state. The node first syncs "forward" by downloading blocks up to the current head as
    /// specified by its peers. Once completed, the node enters this sync state and attempts to
    /// download all required historical blocks.
    BackFillSyncing { completed: usize, remaining: usize },
    /// The node is undertaking a custody backfill sync. This occurs for a node that has completed forward and
    /// backfill sync and has undergone a custody count change. During custody backfill sync the node attempts
    /// to backfill its new column custody requirements up to the data availability window.
    CustodyBackFillSyncing { completed: usize, remaining: usize },
    /// The node has completed syncing a finalized chain and is in the process of re-evaluating
    /// which sync state to progress to.
    SyncTransition,
    /// The node is up to date with all known peers and is connected to at least one
    /// fully synced peer. In this state, parent lookups are enabled.
    Synced,
    /// No useful peers are connected. Long-range sync's cannot proceed and we have no useful
    /// peers to download parents for. More peers need to be connected before we can proceed.
    Stalled,
}

#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
/// The state of the backfill sync.
pub enum BackFillState {
    /// The sync is partially completed and currently paused.
    Paused,
    /// We are currently backfilling.
    Syncing,
    /// A backfill sync has completed.
    Completed,
    /// Too many failed attempts at backfilling. Consider it failed.
    Failed,
}

#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
/// The state of the custody backfill sync.
pub enum CustodyBackFillState {
    /// We are currently backfilling custody columns.
    Syncing,
    /// A custody backfill sync has completed.
    Completed,
    /// A custody sync should is set to Pending for various reasons.
    Pending(String),
}

impl PartialEq for SyncState {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (
                SyncState::SyncingFinalized { .. },
                SyncState::SyncingFinalized { .. }
            ) | (SyncState::SyncingHead { .. }, SyncState::SyncingHead { .. })
                | (SyncState::Synced, SyncState::Synced)
                | (SyncState::Stalled, SyncState::Stalled)
                | (SyncState::SyncTransition, SyncState::SyncTransition)
                | (
                    SyncState::BackFillSyncing { .. },
                    SyncState::BackFillSyncing { .. }
                )
                | (
                    SyncState::CustodyBackFillSyncing { .. },
                    SyncState::CustodyBackFillSyncing { .. }
                )
        )
    }
}

impl SyncState {
    /// Returns a boolean indicating the node is currently performing a long-range sync.
    pub fn is_syncing(&self) -> bool {
        match self {
            SyncState::SyncingFinalized { .. } => true,
            SyncState::SyncingHead { .. } => true,
            SyncState::SyncTransition => true,
            // Both backfill and custody backfill don't effect any logic, we consider this state, not syncing.
            SyncState::BackFillSyncing { .. } | SyncState::CustodyBackFillSyncing { .. } => false,
            SyncState::Synced => false,
            SyncState::Stalled => false,
        }
    }

    pub fn is_syncing_finalized(&self) -> bool {
        match self {
            SyncState::SyncingFinalized { .. } => true,
            SyncState::SyncingHead { .. } => false,
            SyncState::SyncTransition => false,
            SyncState::BackFillSyncing { .. } | SyncState::CustodyBackFillSyncing { .. } => false,
            SyncState::Synced => false,
            SyncState::Stalled => false,
        }
    }

    /// Returns true if the node is synced.
    ///
    /// NOTE: We consider the node synced if it is fetching old historical blocks.
    pub fn is_synced(&self) -> bool {
        matches!(
            self,
            SyncState::Synced
                | SyncState::BackFillSyncing { .. }
                | SyncState::CustodyBackFillSyncing { .. }
        )
    }

    /// Returns true if the node is *stalled*, i.e. has no synced peers.
    ///
    /// Usually this state is treated as unsynced, except in some places where we make an exception
    /// for single-node testnets where having 0 peers is desired.
    pub fn is_stalled(&self) -> bool {
        matches!(self, SyncState::Stalled)
    }
}

impl std::fmt::Display for SyncState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncState::SyncingFinalized { .. } => write!(f, "Syncing Finalized Chain"),
            SyncState::SyncingHead { .. } => write!(f, "Syncing Head Chain"),
            SyncState::Synced => write!(f, "Synced"),
            SyncState::Stalled => write!(f, "Stalled"),
            SyncState::SyncTransition => write!(f, "Evaluating known peers"),
            SyncState::BackFillSyncing { .. } => write!(f, "Syncing Historical Blocks"),
            SyncState::CustodyBackFillSyncing { .. } => {
                write!(f, "Syncing Historical Data Columns")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_eq_ignores_slot_values() {
        let a = SyncState::SyncingFinalized {
            start_slot: Slot::new(0),
            target_slot: Slot::new(100),
        };
        let b = SyncState::SyncingFinalized {
            start_slot: Slot::new(50),
            target_slot: Slot::new(200),
        };
        assert_eq!(a, b, "SyncingFinalized should match regardless of slots");
    }

    #[test]
    fn partial_eq_different_variants_not_equal() {
        let finalized = SyncState::SyncingFinalized {
            start_slot: Slot::new(0),
            target_slot: Slot::new(100),
        };
        let head = SyncState::SyncingHead {
            start_slot: Slot::new(0),
            target_slot: Slot::new(100),
        };
        assert_ne!(finalized, head);
        assert_ne!(SyncState::Synced, SyncState::Stalled);
        assert_ne!(SyncState::SyncTransition, SyncState::Synced);
    }

    #[test]
    fn partial_eq_syncing_head_ignores_slots() {
        let a = SyncState::SyncingHead {
            start_slot: Slot::new(10),
            target_slot: Slot::new(20),
        };
        let b = SyncState::SyncingHead {
            start_slot: Slot::new(30),
            target_slot: Slot::new(40),
        };
        assert_eq!(a, b);
    }

    #[test]
    fn partial_eq_backfill_ignores_counts() {
        let a = SyncState::BackFillSyncing {
            completed: 10,
            remaining: 90,
        };
        let b = SyncState::BackFillSyncing {
            completed: 50,
            remaining: 50,
        };
        assert_eq!(a, b);
    }

    #[test]
    fn partial_eq_custody_backfill_ignores_counts() {
        let a = SyncState::CustodyBackFillSyncing {
            completed: 0,
            remaining: 100,
        };
        let b = SyncState::CustodyBackFillSyncing {
            completed: 99,
            remaining: 1,
        };
        assert_eq!(a, b);
    }

    #[test]
    fn is_syncing_finalized_and_head() {
        assert!(
            SyncState::SyncingFinalized {
                start_slot: Slot::new(0),
                target_slot: Slot::new(100),
            }
            .is_syncing()
        );
        assert!(
            SyncState::SyncingHead {
                start_slot: Slot::new(0),
                target_slot: Slot::new(100),
            }
            .is_syncing()
        );
        assert!(SyncState::SyncTransition.is_syncing());
    }

    #[test]
    fn is_syncing_false_for_synced_stalled_backfill() {
        assert!(!SyncState::Synced.is_syncing());
        assert!(!SyncState::Stalled.is_syncing());
        assert!(
            !SyncState::BackFillSyncing {
                completed: 10,
                remaining: 90,
            }
            .is_syncing()
        );
        assert!(
            !SyncState::CustodyBackFillSyncing {
                completed: 0,
                remaining: 100,
            }
            .is_syncing()
        );
    }

    #[test]
    fn is_syncing_finalized_only_for_finalized() {
        assert!(
            SyncState::SyncingFinalized {
                start_slot: Slot::new(0),
                target_slot: Slot::new(100),
            }
            .is_syncing_finalized()
        );
        assert!(
            !SyncState::SyncingHead {
                start_slot: Slot::new(0),
                target_slot: Slot::new(100),
            }
            .is_syncing_finalized()
        );
        assert!(!SyncState::Synced.is_syncing_finalized());
        assert!(!SyncState::Stalled.is_syncing_finalized());
        assert!(!SyncState::SyncTransition.is_syncing_finalized());
    }

    #[test]
    fn is_synced_includes_backfill() {
        assert!(SyncState::Synced.is_synced());
        assert!(
            SyncState::BackFillSyncing {
                completed: 0,
                remaining: 100,
            }
            .is_synced()
        );
        assert!(
            SyncState::CustodyBackFillSyncing {
                completed: 0,
                remaining: 100,
            }
            .is_synced()
        );
    }

    #[test]
    fn is_synced_false_for_syncing_stalled() {
        assert!(
            !SyncState::SyncingFinalized {
                start_slot: Slot::new(0),
                target_slot: Slot::new(100),
            }
            .is_synced()
        );
        assert!(
            !SyncState::SyncingHead {
                start_slot: Slot::new(0),
                target_slot: Slot::new(100),
            }
            .is_synced()
        );
        assert!(!SyncState::Stalled.is_synced());
        assert!(!SyncState::SyncTransition.is_synced());
    }

    #[test]
    fn is_stalled_only_for_stalled() {
        assert!(SyncState::Stalled.is_stalled());
        assert!(!SyncState::Synced.is_stalled());
        assert!(
            !SyncState::SyncingFinalized {
                start_slot: Slot::new(0),
                target_slot: Slot::new(100),
            }
            .is_stalled()
        );
        assert!(!SyncState::SyncTransition.is_stalled());
    }

    #[test]
    fn display_variants() {
        assert_eq!(
            SyncState::SyncingFinalized {
                start_slot: Slot::new(0),
                target_slot: Slot::new(100),
            }
            .to_string(),
            "Syncing Finalized Chain"
        );
        assert_eq!(
            SyncState::SyncingHead {
                start_slot: Slot::new(0),
                target_slot: Slot::new(100),
            }
            .to_string(),
            "Syncing Head Chain"
        );
        assert_eq!(SyncState::Synced.to_string(), "Synced");
        assert_eq!(SyncState::Stalled.to_string(), "Stalled");
        assert_eq!(
            SyncState::SyncTransition.to_string(),
            "Evaluating known peers"
        );
        assert_eq!(
            SyncState::BackFillSyncing {
                completed: 0,
                remaining: 100,
            }
            .to_string(),
            "Syncing Historical Blocks"
        );
        assert_eq!(
            SyncState::CustodyBackFillSyncing {
                completed: 0,
                remaining: 100,
            }
            .to_string(),
            "Syncing Historical Data Columns"
        );
    }

    #[test]
    fn backfill_state_equality() {
        assert_eq!(BackFillState::Paused, BackFillState::Paused);
        assert_eq!(BackFillState::Syncing, BackFillState::Syncing);
        assert_eq!(BackFillState::Completed, BackFillState::Completed);
        assert_eq!(BackFillState::Failed, BackFillState::Failed);
        assert_ne!(BackFillState::Paused, BackFillState::Syncing);
        assert_ne!(BackFillState::Completed, BackFillState::Failed);
    }

    #[test]
    fn custody_backfill_state_equality() {
        assert_eq!(CustodyBackFillState::Syncing, CustodyBackFillState::Syncing);
        assert_eq!(
            CustodyBackFillState::Completed,
            CustodyBackFillState::Completed
        );
        assert_eq!(
            CustodyBackFillState::Pending("reason".to_string()),
            CustodyBackFillState::Pending("reason".to_string()),
        );
        assert_ne!(
            CustodyBackFillState::Pending("a".to_string()),
            CustodyBackFillState::Pending("b".to_string()),
        );
        assert_ne!(
            CustodyBackFillState::Syncing,
            CustodyBackFillState::Completed
        );
    }
}
