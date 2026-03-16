//! Handles individual sync status for peers.

use serde::Serialize;
use types::{Epoch, Hash256, Slot};

#[derive(Clone, Debug, Serialize)]
/// The current sync status of the peer.
pub enum SyncStatus {
    /// At the current state as our node or ahead of us.
    Synced { info: SyncInfo },
    /// The peer has greater knowledge about the canonical chain than we do.
    Advanced { info: SyncInfo },
    /// Is behind our current head and not useful for block downloads.
    Behind { info: SyncInfo },
    /// This peer is in an incompatible network.
    IrrelevantPeer,
    /// Not currently known as a STATUS handshake has not occurred.
    Unknown,
}

/// A relevant peer's sync information.
#[derive(Clone, Debug, Serialize)]
pub struct SyncInfo {
    pub head_slot: Slot,
    pub head_root: Hash256,
    pub finalized_epoch: Epoch,
    pub finalized_root: Hash256,
    pub earliest_available_slot: Option<Slot>,
}

impl SyncInfo {
    /// Returns true if the provided slot is greater than or equal to the peer's `earliest_available_slot`.
    ///
    /// If `earliest_available_slot` is None, then we just assume that the peer has the slot.
    pub fn has_slot(&self, slot: Slot) -> bool {
        if let Some(earliest_available_slot) = self.earliest_available_slot {
            slot >= earliest_available_slot
        } else {
            true
        }
    }
}

impl std::cmp::PartialEq for SyncStatus {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (SyncStatus::Synced { .. }, SyncStatus::Synced { .. })
                | (SyncStatus::Advanced { .. }, SyncStatus::Advanced { .. })
                | (SyncStatus::Behind { .. }, SyncStatus::Behind { .. })
                | (SyncStatus::IrrelevantPeer, SyncStatus::IrrelevantPeer)
                | (SyncStatus::Unknown, SyncStatus::Unknown)
        )
    }
}

impl SyncStatus {
    /// Returns true if the peer has advanced knowledge of the chain.
    pub fn is_advanced(&self) -> bool {
        matches!(self, SyncStatus::Advanced { .. })
    }

    /// Returns true if the peer is up to date with the current chain.
    pub fn is_synced(&self) -> bool {
        matches!(self, SyncStatus::Synced { .. })
    }

    /// Returns true if the peer is behind the current chain.
    pub fn is_behind(&self) -> bool {
        matches!(self, SyncStatus::Behind { .. })
    }

    /// Updates the peer's sync status, returning whether the status transitioned.
    ///
    /// E.g. returns `true` if the state changed from `Synced` to `Advanced`, but not if
    /// the status remained `Synced` with different `SyncInfo` within.
    pub fn update(&mut self, new_state: SyncStatus) -> bool {
        let changed_status = *self != new_state;
        *self = new_state;
        changed_status
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SyncStatus::Advanced { .. } => "Advanced",
            SyncStatus::Behind { .. } => "Behind",
            SyncStatus::Synced { .. } => "Synced",
            SyncStatus::Unknown => "Unknown",
            SyncStatus::IrrelevantPeer => "Irrelevant",
        }
    }
}

impl std::fmt::Display for SyncStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sync_info(head_slot: u64, earliest: Option<u64>) -> SyncInfo {
        SyncInfo {
            head_slot: Slot::new(head_slot),
            head_root: Hash256::ZERO,
            finalized_epoch: Epoch::new(0),
            finalized_root: Hash256::ZERO,
            earliest_available_slot: earliest.map(Slot::new),
        }
    }

    // --- SyncInfo::has_slot ---

    #[test]
    fn has_slot_none_earliest_returns_true() {
        let info = make_sync_info(100, None);
        assert!(info.has_slot(Slot::new(0)));
        assert!(info.has_slot(Slot::new(999)));
    }

    #[test]
    fn has_slot_at_earliest_boundary() {
        let info = make_sync_info(100, Some(50));
        assert!(info.has_slot(Slot::new(50)));
    }

    #[test]
    fn has_slot_above_earliest() {
        let info = make_sync_info(100, Some(50));
        assert!(info.has_slot(Slot::new(51)));
        assert!(info.has_slot(Slot::new(999)));
    }

    #[test]
    fn has_slot_below_earliest() {
        let info = make_sync_info(100, Some(50));
        assert!(!info.has_slot(Slot::new(49)));
        assert!(!info.has_slot(Slot::new(0)));
    }

    // --- SyncStatus predicates ---

    #[test]
    fn is_advanced() {
        let info = make_sync_info(100, None);
        assert!(SyncStatus::Advanced { info: info.clone() }.is_advanced());
        assert!(!SyncStatus::Synced { info: info.clone() }.is_advanced());
        assert!(!SyncStatus::Behind { info }.is_advanced());
        assert!(!SyncStatus::IrrelevantPeer.is_advanced());
        assert!(!SyncStatus::Unknown.is_advanced());
    }

    #[test]
    fn is_synced() {
        let info = make_sync_info(100, None);
        assert!(SyncStatus::Synced { info: info.clone() }.is_synced());
        assert!(!SyncStatus::Advanced { info: info.clone() }.is_synced());
        assert!(!SyncStatus::Behind { info }.is_synced());
        assert!(!SyncStatus::IrrelevantPeer.is_synced());
        assert!(!SyncStatus::Unknown.is_synced());
    }

    #[test]
    fn is_behind() {
        let info = make_sync_info(100, None);
        assert!(SyncStatus::Behind { info: info.clone() }.is_behind());
        assert!(!SyncStatus::Synced { info: info.clone() }.is_behind());
        assert!(!SyncStatus::Advanced { info }.is_behind());
        assert!(!SyncStatus::IrrelevantPeer.is_behind());
        assert!(!SyncStatus::Unknown.is_behind());
    }

    // --- PartialEq ---

    #[test]
    fn partial_eq_same_variant_different_info() {
        let info1 = make_sync_info(100, None);
        let info2 = make_sync_info(200, Some(10));
        // Same variant = equal, regardless of SyncInfo contents
        assert_eq!(
            SyncStatus::Synced {
                info: info1.clone()
            },
            SyncStatus::Synced {
                info: info2.clone()
            }
        );
        assert_eq!(
            SyncStatus::Advanced {
                info: info1.clone()
            },
            SyncStatus::Advanced {
                info: info2.clone()
            }
        );
        assert_eq!(
            SyncStatus::Behind { info: info1 },
            SyncStatus::Behind { info: info2 }
        );
    }

    #[test]
    fn partial_eq_different_variants() {
        let info = make_sync_info(100, None);
        assert_ne!(
            SyncStatus::Synced { info: info.clone() },
            SyncStatus::Advanced { info: info.clone() }
        );
        assert_ne!(
            SyncStatus::Synced { info: info.clone() },
            SyncStatus::Behind { info: info.clone() }
        );
        assert_ne!(SyncStatus::Synced { info }, SyncStatus::Unknown);
        assert_ne!(SyncStatus::IrrelevantPeer, SyncStatus::Unknown);
    }

    #[test]
    fn partial_eq_stateless_variants() {
        assert_eq!(SyncStatus::IrrelevantPeer, SyncStatus::IrrelevantPeer);
        assert_eq!(SyncStatus::Unknown, SyncStatus::Unknown);
    }

    // --- update ---

    #[test]
    fn update_returns_true_on_status_change() {
        let info = make_sync_info(100, None);
        let mut status = SyncStatus::Unknown;
        assert!(status.update(SyncStatus::Synced { info }));
        assert!(status.is_synced());
    }

    #[test]
    fn update_returns_false_on_same_variant() {
        let info1 = make_sync_info(100, None);
        let info2 = make_sync_info(200, None);
        let mut status = SyncStatus::Synced { info: info1 };
        assert!(!status.update(SyncStatus::Synced { info: info2 }));
    }

    #[test]
    fn update_transitions_through_all_states() {
        let info = make_sync_info(100, None);
        let mut status = SyncStatus::Unknown;

        assert!(status.update(SyncStatus::Behind { info: info.clone() }));
        assert!(status.is_behind());

        assert!(status.update(SyncStatus::Synced { info: info.clone() }));
        assert!(status.is_synced());

        assert!(status.update(SyncStatus::Advanced { info: info.clone() }));
        assert!(status.is_advanced());

        assert!(status.update(SyncStatus::IrrelevantPeer));
        assert_eq!(status, SyncStatus::IrrelevantPeer);

        assert!(status.update(SyncStatus::Unknown));
        assert_eq!(status, SyncStatus::Unknown);
    }

    // --- as_str / Display ---

    #[test]
    fn as_str_values() {
        let info = make_sync_info(100, None);
        assert_eq!(
            SyncStatus::Advanced { info: info.clone() }.as_str(),
            "Advanced"
        );
        assert_eq!(SyncStatus::Behind { info: info.clone() }.as_str(), "Behind");
        assert_eq!(SyncStatus::Synced { info }.as_str(), "Synced");
        assert_eq!(SyncStatus::Unknown.as_str(), "Unknown");
        assert_eq!(SyncStatus::IrrelevantPeer.as_str(), "Irrelevant");
    }

    #[test]
    fn display_matches_as_str() {
        let info = make_sync_info(100, None);
        let status = SyncStatus::Advanced { info };
        assert_eq!(format!("{}", status), status.as_str());
    }
}
