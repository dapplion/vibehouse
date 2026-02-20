use crate::duties_service::{DutiesService, Error};
use eth2::types::PtcDutyData;
use parking_lot::RwLock;
use slot_clock::SlotClock;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};
use types::{Epoch, EthSpec, PublicKeyBytes, Slot};
use validator_store::{DoppelgangerStatus, ValidatorStore};

/// Map from epoch to PTC duties for that epoch.
///
/// This is the PTC equivalent of `SyncDutiesMap`. It stores duties fetched from the BN
/// and is read by the `PayloadAttestationService` at 3/4 of each slot.
pub struct PtcDutiesMap {
    /// Map from epoch to the list of PTC duties for local validators in that epoch.
    duties: RwLock<HashMap<Epoch, Vec<PtcDutyData>>>,
}

impl Default for PtcDutiesMap {
    fn default() -> Self {
        Self::new()
    }
}

impl PtcDutiesMap {
    pub fn new() -> Self {
        Self {
            duties: RwLock::new(HashMap::new()),
        }
    }

    /// Get PTC duties for the given slot.
    pub fn duties_for_slot(&self, slot: Slot, slots_per_epoch: u64) -> Vec<PtcDutyData> {
        let epoch = slot.epoch(slots_per_epoch);
        self.duties
            .read()
            .get(&epoch)
            .map(|duties| duties.iter().filter(|d| d.slot == slot).cloned().collect())
            .unwrap_or_default()
    }

    /// Count PTC duties for the given epoch.
    pub fn duty_count(
        &self,
        epoch: Epoch,
        signing_pubkeys: &std::collections::HashSet<PublicKeyBytes>,
    ) -> usize {
        self.duties
            .read()
            .get(&epoch)
            .map(|duties| {
                duties
                    .iter()
                    .filter(|d| signing_pubkeys.contains(&d.pubkey))
                    .count()
            })
            .unwrap_or(0)
    }

    /// Check if duties are known for the given epoch.
    fn has_duties_for_epoch(&self, epoch: Epoch) -> bool {
        self.duties.read().contains_key(&epoch)
    }

    /// Store duties for an epoch.
    fn set_duties(&self, epoch: Epoch, duties: Vec<PtcDutyData>) {
        self.duties.write().insert(epoch, duties);
    }

    /// Prune duties older than the given epoch.
    fn prune(&self, current_epoch: Epoch) {
        self.duties
            .write()
            .retain(|&epoch, _| epoch >= current_epoch.saturating_sub(1u64));
    }
}

/// Poll the beacon node for PTC duties for the current and next epoch.
///
/// This follows the same pattern as `poll_sync_committee_duties` but is much simpler
/// since PTC has no aggregation proofs or selection proofs.
pub async fn poll_ptc_duties<S: ValidatorStore + 'static, T: SlotClock + 'static>(
    duties_service: &Arc<DutiesService<S, T>>,
) -> Result<(), Error<S::Error>> {
    let spec = &duties_service.spec;
    let current_slot = duties_service
        .slot_clock
        .now()
        .ok_or(Error::UnableToReadSlotClock)?;
    let current_epoch = current_slot.epoch(S::E::slots_per_epoch());

    // If Gloas is not yet activated, do not poll for PTC duties.
    if spec
        .gloas_fork_epoch
        .is_none_or(|gloas_epoch| current_epoch < gloas_epoch)
    {
        return Ok(());
    }

    let ptc_duties = &duties_service.ptc_duties;

    // Fetch duties for the current epoch if not yet known.
    if !ptc_duties.has_duties_for_epoch(current_epoch) {
        poll_ptc_duties_for_epoch(duties_service, current_epoch).await?;
    }

    // Fetch duties for the next epoch.
    let next_epoch = current_epoch.saturating_add(1u64);
    if !ptc_duties.has_duties_for_epoch(next_epoch) {
        poll_ptc_duties_for_epoch(duties_service, next_epoch).await?;
    }

    // Prune old epochs.
    ptc_duties.prune(current_epoch);

    Ok(())
}

/// Fetch PTC duties for a specific epoch and store them in the map.
async fn poll_ptc_duties_for_epoch<S: ValidatorStore, T: SlotClock + 'static>(
    duties_service: &Arc<DutiesService<S, T>>,
    epoch: Epoch,
) -> Result<(), Error<S::Error>> {
    // Collect all local validator indices.
    let local_indices: Vec<u64> = duties_service
        .validator_store
        .voting_pubkeys::<Vec<_>, _>(DoppelgangerStatus::ignored)
        .into_iter()
        .filter_map(|pubkey| duties_service.validator_store.validator_index(&pubkey))
        .collect();

    if local_indices.is_empty() {
        return Ok(());
    }

    debug!(
        %epoch,
        num_validators = local_indices.len(),
        "Fetching PTC duties"
    );

    let duties_response = duties_service
        .beacon_nodes
        .first_success(|beacon_node| {
            let indices = local_indices.clone();
            async move { beacon_node.post_validator_duties_ptc(epoch, &indices).await }
        })
        .await;

    match duties_response {
        Ok(res) => {
            let duties = res.data;
            debug!(
                %epoch,
                count = duties.len(),
                "Fetched PTC duties from BN"
            );
            duties_service.ptc_duties.set_duties(epoch, duties);
        }
        Err(e) => {
            warn!(
                %epoch,
                error = %e,
                "Failed to download PTC duties"
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const SLOTS_PER_EPOCH: u64 = 8;

    fn make_duty(pubkey_byte: u8, validator_index: u64, slot: u64) -> PtcDutyData {
        let mut bytes = [0u8; 48];
        bytes[0] = pubkey_byte;
        PtcDutyData {
            pubkey: PublicKeyBytes::deserialize(&bytes).unwrap(),
            validator_index,
            slot: Slot::new(slot),
            ptc_committee_index: 0,
        }
    }

    fn pubkey_bytes(byte: u8) -> PublicKeyBytes {
        let mut bytes = [0u8; 48];
        bytes[0] = byte;
        PublicKeyBytes::deserialize(&bytes).unwrap()
    }

    // -- new / default --

    #[test]
    fn new_map_is_empty() {
        let map = PtcDutiesMap::new();
        assert!(!map.has_duties_for_epoch(Epoch::new(0)));
        assert!(!map.has_duties_for_epoch(Epoch::new(1)));
    }

    #[test]
    fn default_is_new() {
        let map = PtcDutiesMap::default();
        assert!(!map.has_duties_for_epoch(Epoch::new(0)));
    }

    // -- set_duties / has_duties_for_epoch --

    #[test]
    fn set_and_has_duties() {
        let map = PtcDutiesMap::new();
        assert!(!map.has_duties_for_epoch(Epoch::new(1)));
        map.set_duties(Epoch::new(1), vec![make_duty(1, 100, 8)]);
        assert!(map.has_duties_for_epoch(Epoch::new(1)));
        assert!(!map.has_duties_for_epoch(Epoch::new(0)));
    }

    #[test]
    fn set_duties_empty_vec_still_present() {
        let map = PtcDutiesMap::new();
        map.set_duties(Epoch::new(5), vec![]);
        assert!(map.has_duties_for_epoch(Epoch::new(5)));
    }

    #[test]
    fn set_duties_overwrites() {
        let map = PtcDutiesMap::new();
        map.set_duties(Epoch::new(1), vec![make_duty(1, 100, 8)]);
        map.set_duties(
            Epoch::new(1),
            vec![make_duty(2, 200, 9), make_duty(3, 300, 10)],
        );
        let duties = map.duties_for_slot(Slot::new(9), SLOTS_PER_EPOCH);
        assert_eq!(duties.len(), 1);
        assert_eq!(duties[0].validator_index, 200);
    }

    // -- duties_for_slot --

    #[test]
    fn duties_for_slot_returns_matching() {
        let map = PtcDutiesMap::new();
        // Epoch 1 = slots 8..16 (with 8 slots/epoch)
        map.set_duties(
            Epoch::new(1),
            vec![
                make_duty(1, 100, 8),
                make_duty(2, 200, 9),
                make_duty(3, 300, 8),
                make_duty(4, 400, 10),
            ],
        );
        // Should get duties for slot 8 only
        let slot_8 = map.duties_for_slot(Slot::new(8), SLOTS_PER_EPOCH);
        assert_eq!(slot_8.len(), 2);
        assert!(slot_8.iter().any(|d| d.validator_index == 100));
        assert!(slot_8.iter().any(|d| d.validator_index == 300));
    }

    #[test]
    fn duties_for_slot_empty_when_no_epoch() {
        let map = PtcDutiesMap::new();
        let duties = map.duties_for_slot(Slot::new(8), SLOTS_PER_EPOCH);
        assert!(duties.is_empty());
    }

    #[test]
    fn duties_for_slot_empty_when_no_slot_match() {
        let map = PtcDutiesMap::new();
        map.set_duties(Epoch::new(1), vec![make_duty(1, 100, 8)]);
        let duties = map.duties_for_slot(Slot::new(9), SLOTS_PER_EPOCH);
        assert!(duties.is_empty());
    }

    #[test]
    fn duties_for_slot_across_epochs() {
        let map = PtcDutiesMap::new();
        map.set_duties(Epoch::new(0), vec![make_duty(1, 100, 3)]);
        map.set_duties(Epoch::new(1), vec![make_duty(2, 200, 11)]);
        // Slot 3 is in epoch 0
        assert_eq!(map.duties_for_slot(Slot::new(3), SLOTS_PER_EPOCH).len(), 1);
        // Slot 11 is in epoch 1
        assert_eq!(map.duties_for_slot(Slot::new(11), SLOTS_PER_EPOCH).len(), 1);
        // Slot 0 is in epoch 0 but no duty there
        assert!(
            map.duties_for_slot(Slot::new(0), SLOTS_PER_EPOCH)
                .is_empty()
        );
    }

    // -- duty_count --

    #[test]
    fn duty_count_filters_by_pubkey() {
        let map = PtcDutiesMap::new();
        map.set_duties(
            Epoch::new(1),
            vec![
                make_duty(1, 100, 8),
                make_duty(2, 200, 9),
                make_duty(3, 300, 10),
            ],
        );
        let mut signing = HashSet::new();
        signing.insert(pubkey_bytes(1));
        signing.insert(pubkey_bytes(3));
        assert_eq!(map.duty_count(Epoch::new(1), &signing), 2);
    }

    #[test]
    fn duty_count_zero_for_unknown_epoch() {
        let map = PtcDutiesMap::new();
        let signing = HashSet::new();
        assert_eq!(map.duty_count(Epoch::new(99), &signing), 0);
    }

    #[test]
    fn duty_count_zero_when_no_overlap() {
        let map = PtcDutiesMap::new();
        map.set_duties(Epoch::new(1), vec![make_duty(1, 100, 8)]);
        let mut signing = HashSet::new();
        signing.insert(pubkey_bytes(99));
        assert_eq!(map.duty_count(Epoch::new(1), &signing), 0);
    }

    #[test]
    fn duty_count_empty_signing_set() {
        let map = PtcDutiesMap::new();
        map.set_duties(Epoch::new(1), vec![make_duty(1, 100, 8)]);
        let signing = HashSet::new();
        assert_eq!(map.duty_count(Epoch::new(1), &signing), 0);
    }

    // -- prune --

    #[test]
    fn prune_removes_old_epochs() {
        let map = PtcDutiesMap::new();
        map.set_duties(Epoch::new(0), vec![make_duty(1, 100, 0)]);
        map.set_duties(Epoch::new(1), vec![make_duty(2, 200, 8)]);
        map.set_duties(Epoch::new(2), vec![make_duty(3, 300, 16)]);
        map.set_duties(Epoch::new(3), vec![make_duty(4, 400, 24)]);

        // current_epoch = 3, retains epochs >= 2
        map.prune(Epoch::new(3));

        assert!(!map.has_duties_for_epoch(Epoch::new(0)));
        assert!(!map.has_duties_for_epoch(Epoch::new(1)));
        assert!(map.has_duties_for_epoch(Epoch::new(2)));
        assert!(map.has_duties_for_epoch(Epoch::new(3)));
    }

    #[test]
    fn prune_at_epoch_zero_keeps_everything() {
        let map = PtcDutiesMap::new();
        map.set_duties(Epoch::new(0), vec![make_duty(1, 100, 0)]);

        map.prune(Epoch::new(0));

        // saturating_sub(1) of 0 = 0, so epoch 0 >= 0 → retained
        assert!(map.has_duties_for_epoch(Epoch::new(0)));
    }

    #[test]
    fn prune_at_epoch_one_keeps_zero_and_one() {
        let map = PtcDutiesMap::new();
        map.set_duties(Epoch::new(0), vec![make_duty(1, 100, 0)]);
        map.set_duties(Epoch::new(1), vec![make_duty(2, 200, 8)]);

        map.prune(Epoch::new(1));

        // retains epochs >= 0
        assert!(map.has_duties_for_epoch(Epoch::new(0)));
        assert!(map.has_duties_for_epoch(Epoch::new(1)));
    }

    #[test]
    fn prune_empty_map_is_noop() {
        let map = PtcDutiesMap::new();
        map.prune(Epoch::new(100));
        // No panic, no crash
    }

    // -- multiple operations --

    #[test]
    fn set_then_prune_then_query() {
        let map = PtcDutiesMap::new();
        map.set_duties(Epoch::new(5), vec![make_duty(1, 100, 40)]);
        map.set_duties(Epoch::new(6), vec![make_duty(2, 200, 48)]);
        map.set_duties(Epoch::new(7), vec![make_duty(3, 300, 56)]);

        map.prune(Epoch::new(7));

        // Epoch 5 removed, 6 and 7 remain
        assert!(!map.has_duties_for_epoch(Epoch::new(5)));
        assert!(map.has_duties_for_epoch(Epoch::new(6)));
        assert!(map.has_duties_for_epoch(Epoch::new(7)));

        // Can still query slot 48 in epoch 6
        let duties = map.duties_for_slot(Slot::new(48), SLOTS_PER_EPOCH);
        assert_eq!(duties.len(), 1);
        assert_eq!(duties[0].validator_index, 200);
    }
}
