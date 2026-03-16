use super::CandidateError;
use eth2::BeaconNodeHttpClient;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt::{Debug, Display, Formatter};
use tracing::warn;
use types::Slot;

/// Sync distances between 0 and DEFAULT_SYNC_TOLERANCE are considered `synced`.
/// Sync distance tiers are determined by the different modifiers.
///
/// The default range is the following:
/// Synced: 0..=8
/// Small: 9..=16
/// Medium: 17..=64
/// Large: 65..
const DEFAULT_SYNC_TOLERANCE: Slot = Slot::new(8);
const DEFAULT_SMALL_SYNC_DISTANCE_MODIFIER: Slot = Slot::new(8);
const DEFAULT_MEDIUM_SYNC_DISTANCE_MODIFIER: Slot = Slot::new(48);

type HealthTier = u8;
type SyncDistance = Slot;

/// Helpful enum which is used when pattern matching to determine health tier.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum SyncDistanceTier {
    Synced,
    Small,
    Medium,
    Large,
}

/// Contains the different sync distance tiers which are determined at runtime by the
/// `beacon-nodes-sync-tolerances` flag.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct BeaconNodeSyncDistanceTiers {
    pub synced: SyncDistance,
    pub small: SyncDistance,
    pub medium: SyncDistance,
}

impl Default for BeaconNodeSyncDistanceTiers {
    fn default() -> Self {
        Self {
            synced: DEFAULT_SYNC_TOLERANCE,
            small: DEFAULT_SYNC_TOLERANCE + DEFAULT_SMALL_SYNC_DISTANCE_MODIFIER,
            medium: DEFAULT_SYNC_TOLERANCE
                + DEFAULT_SMALL_SYNC_DISTANCE_MODIFIER
                + DEFAULT_MEDIUM_SYNC_DISTANCE_MODIFIER,
        }
    }
}

impl BeaconNodeSyncDistanceTiers {
    /// Takes a given sync distance and determines its tier based on the `sync_tolerance` defined by
    /// the CLI.
    pub fn compute_distance_tier(&self, distance: SyncDistance) -> SyncDistanceTier {
        if distance <= self.synced {
            SyncDistanceTier::Synced
        } else if distance <= self.small {
            SyncDistanceTier::Small
        } else if distance <= self.medium {
            SyncDistanceTier::Medium
        } else {
            SyncDistanceTier::Large
        }
    }

    pub fn from_vec(tiers: &[u64]) -> Result<Self, String> {
        if tiers.len() != 3 {
            return Err("Invalid number of sync distance modifiers".to_string());
        }
        Ok(BeaconNodeSyncDistanceTiers {
            synced: Slot::new(tiers[0]),
            small: Slot::new(tiers[0] + tiers[1]),
            medium: Slot::new(tiers[0] + tiers[1] + tiers[2]),
        })
    }
}

/// Execution Node health metrics.
///
/// Currently only considers `el_offline`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum ExecutionEngineHealth {
    Healthy,
    Unhealthy,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum IsOptimistic {
    Yes,
    No,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct BeaconNodeHealthTier {
    pub tier: HealthTier,
    pub sync_distance: SyncDistance,
    pub distance_tier: SyncDistanceTier,
}

impl Display for BeaconNodeHealthTier {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Tier{}({})", self.tier, self.sync_distance)
    }
}

impl Ord for BeaconNodeHealthTier {
    fn cmp(&self, other: &Self) -> Ordering {
        let ordering = self.tier.cmp(&other.tier);
        if ordering == Ordering::Equal {
            if self.distance_tier == SyncDistanceTier::Synced {
                // Don't tie-break on sync distance in these cases.
                // This ensures validator clients don't artificially prefer one node.
                ordering
            } else {
                self.sync_distance.cmp(&other.sync_distance)
            }
        } else {
            ordering
        }
    }
}

impl PartialOrd for BeaconNodeHealthTier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl BeaconNodeHealthTier {
    pub fn new(
        tier: HealthTier,
        sync_distance: SyncDistance,
        distance_tier: SyncDistanceTier,
    ) -> Self {
        Self {
            tier,
            sync_distance,
            distance_tier,
        }
    }
}

/// Beacon Node Health metrics.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct BeaconNodeHealth {
    // The index of the Beacon Node. This should correspond with its position in the
    // `--beacon-nodes` list. Note that the `user_index` field is used to tie-break nodes with the
    // same health so that nodes with a lower index are preferred.
    pub user_index: usize,
    // The slot number of the head.
    pub head: Slot,
    // Whether the node is optimistically synced.
    pub optimistic_status: IsOptimistic,
    // The status of the nodes connected Execution Engine.
    pub execution_status: ExecutionEngineHealth,
    // The overall health tier of the Beacon Node. Used to rank the nodes for the purposes of
    // fallbacks.
    pub health_tier: BeaconNodeHealthTier,
}

impl Ord for BeaconNodeHealth {
    fn cmp(&self, other: &Self) -> Ordering {
        let ordering = self.health_tier.cmp(&other.health_tier);
        if ordering == Ordering::Equal {
            // Tie-break node health by `user_index`.
            self.user_index.cmp(&other.user_index)
        } else {
            ordering
        }
    }
}

impl PartialOrd for BeaconNodeHealth {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl BeaconNodeHealth {
    pub fn from_status(
        user_index: usize,
        sync_distance: Slot,
        head: Slot,
        optimistic_status: IsOptimistic,
        execution_status: ExecutionEngineHealth,
        distance_tiers: &BeaconNodeSyncDistanceTiers,
    ) -> Self {
        let health_tier = BeaconNodeHealth::compute_health_tier(
            sync_distance,
            optimistic_status,
            execution_status,
            distance_tiers,
        );

        Self {
            user_index,
            head,
            optimistic_status,
            execution_status,
            health_tier,
        }
    }

    pub fn get_index(&self) -> usize {
        self.user_index
    }

    pub fn get_health_tier(&self) -> BeaconNodeHealthTier {
        self.health_tier
    }

    fn compute_health_tier(
        sync_distance: SyncDistance,
        optimistic_status: IsOptimistic,
        execution_status: ExecutionEngineHealth,
        sync_distance_tiers: &BeaconNodeSyncDistanceTiers,
    ) -> BeaconNodeHealthTier {
        let sync_distance_tier = sync_distance_tiers.compute_distance_tier(sync_distance);
        let health = (sync_distance_tier, optimistic_status, execution_status);

        match health {
            (SyncDistanceTier::Synced, IsOptimistic::No, ExecutionEngineHealth::Healthy) => {
                BeaconNodeHealthTier::new(1, sync_distance, sync_distance_tier)
            }
            (SyncDistanceTier::Small, IsOptimistic::No, ExecutionEngineHealth::Healthy) => {
                BeaconNodeHealthTier::new(2, sync_distance, sync_distance_tier)
            }
            (SyncDistanceTier::Synced, IsOptimistic::No, ExecutionEngineHealth::Unhealthy) => {
                BeaconNodeHealthTier::new(3, sync_distance, sync_distance_tier)
            }
            (SyncDistanceTier::Medium, IsOptimistic::No, ExecutionEngineHealth::Healthy) => {
                BeaconNodeHealthTier::new(4, sync_distance, sync_distance_tier)
            }
            (SyncDistanceTier::Synced, IsOptimistic::Yes, ExecutionEngineHealth::Healthy) => {
                BeaconNodeHealthTier::new(5, sync_distance, sync_distance_tier)
            }
            (SyncDistanceTier::Synced, IsOptimistic::Yes, ExecutionEngineHealth::Unhealthy) => {
                BeaconNodeHealthTier::new(6, sync_distance, sync_distance_tier)
            }
            (SyncDistanceTier::Small, IsOptimistic::No, ExecutionEngineHealth::Unhealthy) => {
                BeaconNodeHealthTier::new(7, sync_distance, sync_distance_tier)
            }
            (SyncDistanceTier::Small, IsOptimistic::Yes, ExecutionEngineHealth::Healthy) => {
                BeaconNodeHealthTier::new(8, sync_distance, sync_distance_tier)
            }
            (SyncDistanceTier::Small, IsOptimistic::Yes, ExecutionEngineHealth::Unhealthy) => {
                BeaconNodeHealthTier::new(9, sync_distance, sync_distance_tier)
            }
            (SyncDistanceTier::Large, IsOptimistic::No, ExecutionEngineHealth::Healthy) => {
                BeaconNodeHealthTier::new(10, sync_distance, sync_distance_tier)
            }
            (SyncDistanceTier::Medium, IsOptimistic::No, ExecutionEngineHealth::Unhealthy) => {
                BeaconNodeHealthTier::new(11, sync_distance, sync_distance_tier)
            }
            (SyncDistanceTier::Medium, IsOptimistic::Yes, ExecutionEngineHealth::Healthy) => {
                BeaconNodeHealthTier::new(12, sync_distance, sync_distance_tier)
            }
            (SyncDistanceTier::Medium, IsOptimistic::Yes, ExecutionEngineHealth::Unhealthy) => {
                BeaconNodeHealthTier::new(13, sync_distance, sync_distance_tier)
            }
            (SyncDistanceTier::Large, IsOptimistic::No, ExecutionEngineHealth::Unhealthy) => {
                BeaconNodeHealthTier::new(14, sync_distance, sync_distance_tier)
            }
            (SyncDistanceTier::Large, IsOptimistic::Yes, ExecutionEngineHealth::Healthy) => {
                BeaconNodeHealthTier::new(15, sync_distance, sync_distance_tier)
            }
            (SyncDistanceTier::Large, IsOptimistic::Yes, ExecutionEngineHealth::Unhealthy) => {
                BeaconNodeHealthTier::new(16, sync_distance, sync_distance_tier)
            }
        }
    }
}

pub async fn check_node_health(
    beacon_node: &BeaconNodeHttpClient,
) -> Result<(Slot, bool, bool), CandidateError> {
    let resp = match beacon_node.get_node_syncing().await {
        Ok(resp) => resp,
        Err(e) => {
            warn!(
                error = %e,
                "Unable connect to beacon node"
            );

            return Err(CandidateError::Offline);
        }
    };

    Ok((
        resp.data.head_slot,
        resp.data.is_optimistic,
        resp.data.el_offline,
    ))
}

#[cfg(test)]
mod tests {
    use super::ExecutionEngineHealth::{Healthy, Unhealthy};
    use super::{
        BeaconNodeHealth, BeaconNodeHealthTier, BeaconNodeSyncDistanceTiers, IsOptimistic,
        SyncDistanceTier,
    };
    use crate::Config;
    use types::Slot;

    #[test]
    fn all_possible_health_tiers() {
        let config = Config::default();
        let beacon_node_sync_distance_tiers = config.sync_tolerances;

        let mut health_vec = vec![];

        for head_slot in 0..=64 {
            for optimistic_status in &[IsOptimistic::No, IsOptimistic::Yes] {
                for ee_health in &[Healthy, Unhealthy] {
                    let health = BeaconNodeHealth::from_status(
                        0,
                        Slot::new(0),
                        Slot::new(head_slot),
                        *optimistic_status,
                        *ee_health,
                        &beacon_node_sync_distance_tiers,
                    );
                    health_vec.push(health);
                }
            }
        }

        for health in health_vec {
            let health_tier = health.get_health_tier();
            let tier = health_tier.tier;
            let distance = health_tier.sync_distance;

            let distance_tier = beacon_node_sync_distance_tiers.compute_distance_tier(distance);

            // Check sync distance.
            if [1, 3, 5, 6].contains(&tier) {
                assert!(distance_tier == SyncDistanceTier::Synced)
            } else if [2, 7, 8, 9].contains(&tier) {
                assert!(distance_tier == SyncDistanceTier::Small);
            } else if [4, 11, 12, 13].contains(&tier) {
                assert!(distance_tier == SyncDistanceTier::Medium);
            } else {
                assert!(distance_tier == SyncDistanceTier::Large);
            }

            // Check optimistic status.
            if [1, 2, 3, 4, 7, 10, 11, 14].contains(&tier) {
                assert_eq!(health.optimistic_status, IsOptimistic::No);
            } else {
                assert_eq!(health.optimistic_status, IsOptimistic::Yes);
            }

            // Check execution health.
            if [3, 6, 7, 9, 11, 13, 14, 16].contains(&tier) {
                assert_eq!(health.execution_status, Unhealthy);
            } else {
                assert_eq!(health.execution_status, Healthy);
            }
        }
    }

    fn new_distance_tier(
        distance: u64,
        distance_tiers: &BeaconNodeSyncDistanceTiers,
    ) -> BeaconNodeHealthTier {
        BeaconNodeHealth::compute_health_tier(
            Slot::new(distance),
            IsOptimistic::No,
            Healthy,
            distance_tiers,
        )
    }

    #[test]
    fn sync_tolerance_default() {
        let distance_tiers = BeaconNodeSyncDistanceTiers::default();

        let synced_low = new_distance_tier(0, &distance_tiers);
        let synced_high = new_distance_tier(8, &distance_tiers);

        let small_low = new_distance_tier(9, &distance_tiers);
        let small_high = new_distance_tier(16, &distance_tiers);

        let medium_low = new_distance_tier(17, &distance_tiers);
        let medium_high = new_distance_tier(64, &distance_tiers);
        let large = new_distance_tier(65, &distance_tiers);

        assert_eq!(synced_low.tier, 1);
        assert_eq!(synced_high.tier, 1);
        assert_eq!(small_low.tier, 2);
        assert_eq!(small_high.tier, 2);
        assert_eq!(medium_low.tier, 4);
        assert_eq!(medium_high.tier, 4);
        assert_eq!(large.tier, 10);
    }

    #[test]
    fn sync_tolerance_from_str() {
        // String should set the tiers as:
        // synced: 0..=4
        // small: 5..=8
        // medium 9..=12
        // large: 13..

        let distance_tiers = BeaconNodeSyncDistanceTiers::from_vec(&[4, 4, 4]).unwrap();

        let synced_low = new_distance_tier(0, &distance_tiers);
        let synced_high = new_distance_tier(4, &distance_tiers);

        let small_low = new_distance_tier(5, &distance_tiers);
        let small_high = new_distance_tier(8, &distance_tiers);

        let medium_low = new_distance_tier(9, &distance_tiers);
        let medium_high = new_distance_tier(12, &distance_tiers);

        let large = new_distance_tier(13, &distance_tiers);

        assert_eq!(synced_low.tier, 1);
        assert_eq!(synced_high.tier, 1);
        assert_eq!(small_low.tier, 2);
        assert_eq!(small_high.tier, 2);
        assert_eq!(medium_low.tier, 4);
        assert_eq!(medium_high.tier, 4);
        assert_eq!(large.tier, 10);
    }

    // --- BeaconNodeSyncDistanceTiers tests ---

    #[test]
    fn sync_distance_tiers_default_values() {
        let tiers = BeaconNodeSyncDistanceTiers::default();
        assert_eq!(tiers.synced, Slot::new(8));
        assert_eq!(tiers.small, Slot::new(16));
        assert_eq!(tiers.medium, Slot::new(64));
    }

    #[test]
    fn from_vec_wrong_length_errors() {
        assert!(BeaconNodeSyncDistanceTiers::from_vec(&[]).is_err());
        assert!(BeaconNodeSyncDistanceTiers::from_vec(&[1]).is_err());
        assert!(BeaconNodeSyncDistanceTiers::from_vec(&[1, 2]).is_err());
        assert!(BeaconNodeSyncDistanceTiers::from_vec(&[1, 2, 3, 4]).is_err());
    }

    #[test]
    fn from_vec_cumulative_values() {
        let tiers = BeaconNodeSyncDistanceTiers::from_vec(&[10, 20, 30]).unwrap();
        assert_eq!(tiers.synced, Slot::new(10));
        assert_eq!(tiers.small, Slot::new(30)); // 10 + 20
        assert_eq!(tiers.medium, Slot::new(60)); // 10 + 20 + 30
    }

    #[test]
    fn from_vec_zero_modifiers() {
        let tiers = BeaconNodeSyncDistanceTiers::from_vec(&[0, 0, 0]).unwrap();
        assert_eq!(tiers.synced, Slot::new(0));
        assert_eq!(tiers.small, Slot::new(0));
        assert_eq!(tiers.medium, Slot::new(0));
    }

    #[test]
    fn compute_distance_tier_boundary_exact() {
        let tiers = BeaconNodeSyncDistanceTiers::default();
        // At exact boundary values
        assert_eq!(
            tiers.compute_distance_tier(Slot::new(8)),
            SyncDistanceTier::Synced
        );
        assert_eq!(
            tiers.compute_distance_tier(Slot::new(9)),
            SyncDistanceTier::Small
        );
        assert_eq!(
            tiers.compute_distance_tier(Slot::new(16)),
            SyncDistanceTier::Small
        );
        assert_eq!(
            tiers.compute_distance_tier(Slot::new(17)),
            SyncDistanceTier::Medium
        );
        assert_eq!(
            tiers.compute_distance_tier(Slot::new(64)),
            SyncDistanceTier::Medium
        );
        assert_eq!(
            tiers.compute_distance_tier(Slot::new(65)),
            SyncDistanceTier::Large
        );
    }

    #[test]
    fn compute_distance_tier_zero_is_synced() {
        let tiers = BeaconNodeSyncDistanceTiers::default();
        assert_eq!(
            tiers.compute_distance_tier(Slot::new(0)),
            SyncDistanceTier::Synced
        );
    }

    #[test]
    fn compute_distance_tier_very_large() {
        let tiers = BeaconNodeSyncDistanceTiers::default();
        assert_eq!(
            tiers.compute_distance_tier(Slot::new(100_000)),
            SyncDistanceTier::Large
        );
    }

    #[test]
    fn compute_distance_tier_zero_threshold_tiers() {
        // When all thresholds are 0, only distance 0 is synced; everything else is large
        let tiers = BeaconNodeSyncDistanceTiers::from_vec(&[0, 0, 0]).unwrap();
        assert_eq!(
            tiers.compute_distance_tier(Slot::new(0)),
            SyncDistanceTier::Synced
        );
        assert_eq!(
            tiers.compute_distance_tier(Slot::new(1)),
            SyncDistanceTier::Large
        );
    }

    // --- BeaconNodeHealthTier tests ---

    #[test]
    fn health_tier_display() {
        let tier = BeaconNodeHealthTier::new(3, Slot::new(5), SyncDistanceTier::Synced);
        assert_eq!(format!("{}", tier), "Tier3(5)");
    }

    #[test]
    fn health_tier_display_zero() {
        let tier = BeaconNodeHealthTier::new(1, Slot::new(0), SyncDistanceTier::Synced);
        assert_eq!(format!("{}", tier), "Tier1(0)");
    }

    #[test]
    fn health_tier_ordering_different_tiers() {
        let tier1 = BeaconNodeHealthTier::new(1, Slot::new(0), SyncDistanceTier::Synced);
        let tier2 = BeaconNodeHealthTier::new(2, Slot::new(10), SyncDistanceTier::Small);
        assert!(tier1 < tier2);
    }

    #[test]
    fn health_tier_ordering_same_tier_synced_no_tiebreak() {
        // When distance_tier is Synced, don't tie-break on distance
        let a = BeaconNodeHealthTier::new(1, Slot::new(0), SyncDistanceTier::Synced);
        let b = BeaconNodeHealthTier::new(1, Slot::new(5), SyncDistanceTier::Synced);
        assert_eq!(a.cmp(&b), std::cmp::Ordering::Equal);
    }

    #[test]
    fn health_tier_ordering_same_tier_small_tiebreak_on_distance() {
        // When distance_tier is NOT Synced, tie-break on distance
        let closer = BeaconNodeHealthTier::new(2, Slot::new(9), SyncDistanceTier::Small);
        let further = BeaconNodeHealthTier::new(2, Slot::new(15), SyncDistanceTier::Small);
        assert!(closer < further);
    }

    #[test]
    fn health_tier_ordering_same_tier_medium_tiebreak() {
        let a = BeaconNodeHealthTier::new(4, Slot::new(20), SyncDistanceTier::Medium);
        let b = BeaconNodeHealthTier::new(4, Slot::new(50), SyncDistanceTier::Medium);
        assert!(a < b);
    }

    #[test]
    fn health_tier_ordering_same_tier_large_tiebreak() {
        let a = BeaconNodeHealthTier::new(10, Slot::new(100), SyncDistanceTier::Large);
        let b = BeaconNodeHealthTier::new(10, Slot::new(200), SyncDistanceTier::Large);
        assert!(a < b);
    }

    #[test]
    fn health_tier_eq() {
        let a = BeaconNodeHealthTier::new(1, Slot::new(3), SyncDistanceTier::Synced);
        let b = BeaconNodeHealthTier::new(1, Slot::new(3), SyncDistanceTier::Synced);
        assert_eq!(a, b);
    }

    // --- BeaconNodeHealth ordering tests ---

    #[test]
    fn health_ordering_different_tiers() {
        let tiers = BeaconNodeSyncDistanceTiers::default();
        let h1 = BeaconNodeHealth::from_status(
            0,
            Slot::new(0),
            Slot::new(100),
            IsOptimistic::No,
            Healthy,
            &tiers,
        );
        let h2 = BeaconNodeHealth::from_status(
            1,
            Slot::new(0),
            Slot::new(100),
            IsOptimistic::No,
            Unhealthy,
            &tiers,
        );
        // h1 is tier 1 (synced, not optimistic, healthy), h2 is tier 3 (synced, not optimistic, unhealthy)
        assert!(h1 < h2);
    }

    #[test]
    fn health_ordering_tiebreak_by_user_index() {
        let tiers = BeaconNodeSyncDistanceTiers::default();
        let h1 = BeaconNodeHealth::from_status(
            0,
            Slot::new(0),
            Slot::new(100),
            IsOptimistic::No,
            Healthy,
            &tiers,
        );
        let h2 = BeaconNodeHealth::from_status(
            1,
            Slot::new(0),
            Slot::new(100),
            IsOptimistic::No,
            Healthy,
            &tiers,
        );
        // Same tier (1), tie-break by user_index — 0 < 1
        assert!(h1 < h2);
    }

    #[test]
    fn health_ordering_higher_index_loses_tiebreak() {
        let tiers = BeaconNodeSyncDistanceTiers::default();
        let h1 = BeaconNodeHealth::from_status(
            5,
            Slot::new(0),
            Slot::new(100),
            IsOptimistic::No,
            Healthy,
            &tiers,
        );
        let h2 = BeaconNodeHealth::from_status(
            2,
            Slot::new(0),
            Slot::new(100),
            IsOptimistic::No,
            Healthy,
            &tiers,
        );
        // Same tier, h2 has lower user_index
        assert!(h2 < h1);
    }

    #[test]
    fn health_ordering_lower_tier_wins_over_lower_index() {
        let tiers = BeaconNodeSyncDistanceTiers::default();
        // Node 5 is synced+healthy (tier 1), node 0 is synced+unhealthy (tier 3)
        let h_tier1 = BeaconNodeHealth::from_status(
            5,
            Slot::new(0),
            Slot::new(100),
            IsOptimistic::No,
            Healthy,
            &tiers,
        );
        let h_tier3 = BeaconNodeHealth::from_status(
            0,
            Slot::new(0),
            Slot::new(100),
            IsOptimistic::No,
            Unhealthy,
            &tiers,
        );
        assert!(h_tier1 < h_tier3);
    }

    #[test]
    fn health_get_index() {
        let tiers = BeaconNodeSyncDistanceTiers::default();
        let h = BeaconNodeHealth::from_status(
            42,
            Slot::new(0),
            Slot::new(100),
            IsOptimistic::No,
            Healthy,
            &tiers,
        );
        assert_eq!(h.get_index(), 42);
    }

    #[test]
    fn health_get_health_tier_returns_correct_tier() {
        let tiers = BeaconNodeSyncDistanceTiers::default();
        let h = BeaconNodeHealth::from_status(
            0,
            Slot::new(10),
            Slot::new(100),
            IsOptimistic::No,
            Healthy,
            &tiers,
        );
        // sync_distance=10, which is Small tier, not optimistic, healthy => tier 2
        assert_eq!(h.get_health_tier().tier, 2);
    }

    // --- Exhaustive health tier classification tests ---

    fn health_tier_for(
        sync_distance: u64,
        optimistic: IsOptimistic,
        ee_health: super::ExecutionEngineHealth,
    ) -> u8 {
        let tiers = BeaconNodeSyncDistanceTiers::default();
        BeaconNodeHealth::compute_health_tier(
            Slot::new(sync_distance),
            optimistic,
            ee_health,
            &tiers,
        )
        .tier
    }

    #[test]
    fn tier_1_synced_not_optimistic_healthy() {
        assert_eq!(health_tier_for(0, IsOptimistic::No, Healthy), 1);
    }

    #[test]
    fn tier_2_small_not_optimistic_healthy() {
        assert_eq!(health_tier_for(10, IsOptimistic::No, Healthy), 2);
    }

    #[test]
    fn tier_3_synced_not_optimistic_unhealthy() {
        assert_eq!(health_tier_for(0, IsOptimistic::No, Unhealthy), 3);
    }

    #[test]
    fn tier_4_medium_not_optimistic_healthy() {
        assert_eq!(health_tier_for(20, IsOptimistic::No, Healthy), 4);
    }

    #[test]
    fn tier_5_synced_optimistic_healthy() {
        assert_eq!(health_tier_for(0, IsOptimistic::Yes, Healthy), 5);
    }

    #[test]
    fn tier_6_synced_optimistic_unhealthy() {
        assert_eq!(health_tier_for(0, IsOptimistic::Yes, Unhealthy), 6);
    }

    #[test]
    fn tier_7_small_not_optimistic_unhealthy() {
        assert_eq!(health_tier_for(10, IsOptimistic::No, Unhealthy), 7);
    }

    #[test]
    fn tier_8_small_optimistic_healthy() {
        assert_eq!(health_tier_for(10, IsOptimistic::Yes, Healthy), 8);
    }

    #[test]
    fn tier_9_small_optimistic_unhealthy() {
        assert_eq!(health_tier_for(10, IsOptimistic::Yes, Unhealthy), 9);
    }

    #[test]
    fn tier_10_large_not_optimistic_healthy() {
        assert_eq!(health_tier_for(100, IsOptimistic::No, Healthy), 10);
    }

    #[test]
    fn tier_11_medium_not_optimistic_unhealthy() {
        assert_eq!(health_tier_for(20, IsOptimistic::No, Unhealthy), 11);
    }

    #[test]
    fn tier_12_medium_optimistic_healthy() {
        assert_eq!(health_tier_for(20, IsOptimistic::Yes, Healthy), 12);
    }

    #[test]
    fn tier_13_medium_optimistic_unhealthy() {
        assert_eq!(health_tier_for(20, IsOptimistic::Yes, Unhealthy), 13);
    }

    #[test]
    fn tier_14_large_not_optimistic_unhealthy() {
        assert_eq!(health_tier_for(100, IsOptimistic::No, Unhealthy), 14);
    }

    #[test]
    fn tier_15_large_optimistic_healthy() {
        assert_eq!(health_tier_for(100, IsOptimistic::Yes, Healthy), 15);
    }

    #[test]
    fn tier_16_large_optimistic_unhealthy() {
        assert_eq!(health_tier_for(100, IsOptimistic::Yes, Unhealthy), 16);
    }

    // --- Sorting tests ---

    #[test]
    fn sort_health_tiers_ascending() {
        let tiers = BeaconNodeSyncDistanceTiers::default();
        let mut nodes = [
            BeaconNodeHealth::from_status(
                0,
                Slot::new(100),
                Slot::new(50),
                IsOptimistic::Yes,
                Unhealthy,
                &tiers,
            ), // large+opt+unhealthy => tier 16
            BeaconNodeHealth::from_status(
                1,
                Slot::new(0),
                Slot::new(50),
                IsOptimistic::No,
                Healthy,
                &tiers,
            ), // synced+no_opt+healthy => tier 1
            BeaconNodeHealth::from_status(
                2,
                Slot::new(10),
                Slot::new(50),
                IsOptimistic::No,
                Healthy,
                &tiers,
            ), // small+no_opt+healthy => tier 2
        ];
        nodes.sort();
        assert_eq!(nodes[0].user_index, 1); // tier 1
        assert_eq!(nodes[1].user_index, 2); // tier 2
        assert_eq!(nodes[2].user_index, 0); // tier 16
    }

    #[test]
    fn sort_health_same_tier_by_user_index() {
        let tiers = BeaconNodeSyncDistanceTiers::default();
        let mut nodes = [
            BeaconNodeHealth::from_status(
                3,
                Slot::new(0),
                Slot::new(50),
                IsOptimistic::No,
                Healthy,
                &tiers,
            ),
            BeaconNodeHealth::from_status(
                1,
                Slot::new(0),
                Slot::new(50),
                IsOptimistic::No,
                Healthy,
                &tiers,
            ),
            BeaconNodeHealth::from_status(
                2,
                Slot::new(0),
                Slot::new(50),
                IsOptimistic::No,
                Healthy,
                &tiers,
            ),
        ];
        nodes.sort();
        assert_eq!(nodes[0].user_index, 1);
        assert_eq!(nodes[1].user_index, 2);
        assert_eq!(nodes[2].user_index, 3);
    }

    // --- Serialization round-trip tests ---

    #[test]
    fn sync_distance_tier_serde_roundtrip() {
        for tier in [
            SyncDistanceTier::Synced,
            SyncDistanceTier::Small,
            SyncDistanceTier::Medium,
            SyncDistanceTier::Large,
        ] {
            let json = serde_json::to_string(&tier).unwrap();
            let deserialized: SyncDistanceTier = serde_json::from_str(&json).unwrap();
            assert_eq!(tier, deserialized);
        }
    }

    #[test]
    fn beacon_node_health_tier_serde_roundtrip() {
        let tier = BeaconNodeHealthTier::new(5, Slot::new(3), SyncDistanceTier::Synced);
        let json = serde_json::to_string(&tier).unwrap();
        let deserialized: BeaconNodeHealthTier = serde_json::from_str(&json).unwrap();
        assert_eq!(tier, deserialized);
    }

    #[test]
    fn beacon_node_health_serde_roundtrip() {
        let tiers = BeaconNodeSyncDistanceTiers::default();
        let health = BeaconNodeHealth::from_status(
            2,
            Slot::new(5),
            Slot::new(100),
            IsOptimistic::No,
            Healthy,
            &tiers,
        );
        let json = serde_json::to_string(&health).unwrap();
        let deserialized: BeaconNodeHealth = serde_json::from_str(&json).unwrap();
        assert_eq!(health, deserialized);
    }

    #[test]
    fn sync_distance_tiers_serde_roundtrip() {
        let tiers = BeaconNodeSyncDistanceTiers::default();
        let json = serde_json::to_string(&tiers).unwrap();
        let deserialized: BeaconNodeSyncDistanceTiers = serde_json::from_str(&json).unwrap();
        assert_eq!(tiers, deserialized);
    }

    #[test]
    fn execution_engine_health_serde_roundtrip() {
        for status in [Healthy, Unhealthy] {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: super::ExecutionEngineHealth = serde_json::from_str(&json).unwrap();
            assert_eq!(status, deserialized);
        }
    }

    #[test]
    fn is_optimistic_serde_roundtrip() {
        for status in [IsOptimistic::Yes, IsOptimistic::No] {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: IsOptimistic = serde_json::from_str(&json).unwrap();
            assert_eq!(status, deserialized);
        }
    }

    #[test]
    fn config_serde_roundtrip() {
        let config = Config::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(config.sync_tolerances, deserialized.sync_tolerances);
    }

    // --- PartialOrd consistency ---

    #[test]
    fn health_tier_partial_ord_consistent_with_ord() {
        let a = BeaconNodeHealthTier::new(1, Slot::new(0), SyncDistanceTier::Synced);
        let b = BeaconNodeHealthTier::new(2, Slot::new(10), SyncDistanceTier::Small);
        assert_eq!(a.partial_cmp(&b), Some(a.cmp(&b)));
    }

    #[test]
    fn health_partial_ord_consistent_with_ord() {
        let tiers = BeaconNodeSyncDistanceTiers::default();
        let a = BeaconNodeHealth::from_status(
            0,
            Slot::new(0),
            Slot::new(100),
            IsOptimistic::No,
            Healthy,
            &tiers,
        );
        let b = BeaconNodeHealth::from_status(
            1,
            Slot::new(10),
            Slot::new(100),
            IsOptimistic::No,
            Healthy,
            &tiers,
        );
        assert_eq!(a.partial_cmp(&b), Some(a.cmp(&b)));
    }
}
