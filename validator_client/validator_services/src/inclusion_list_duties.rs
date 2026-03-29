use crate::duties_service::{DutiesService, Error};
use eth2::types::InclusionListDutyData;
use parking_lot::RwLock;
use slot_clock::SlotClock;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};
use types::{Epoch, EthSpec, PublicKeyBytes, Slot};
use validator_store::{DoppelgangerStatus, ValidatorStore};

/// Map from epoch to inclusion list committee duties for that epoch.
///
/// This is the IL equivalent of `PtcDutiesMap`. It stores duties fetched from the BN
/// and is read by the `InclusionListService` at ~67% of each slot.
pub(crate) struct InclusionListDutiesMap {
    /// Map from epoch to the list of IL committee duties for local validators in that epoch.
    duties: RwLock<HashMap<Epoch, Vec<InclusionListDutyData>>>,
}

impl Default for InclusionListDutiesMap {
    fn default() -> Self {
        Self::new()
    }
}

impl InclusionListDutiesMap {
    pub(crate) fn new() -> Self {
        Self {
            duties: RwLock::new(HashMap::new()),
        }
    }

    /// Get IL committee duties for the given slot.
    pub(crate) fn duties_for_slot(
        &self,
        slot: Slot,
        slots_per_epoch: u64,
    ) -> Vec<InclusionListDutyData> {
        let epoch = slot.epoch(slots_per_epoch);
        self.duties
            .read()
            .get(&epoch)
            .map(|duties| duties.iter().filter(|d| d.slot == slot).cloned().collect())
            .unwrap_or_default()
    }

    /// Count IL duties for the given epoch.
    pub(crate) fn duty_count(
        &self,
        epoch: Epoch,
        signing_pubkeys: &std::collections::HashSet<PublicKeyBytes>,
    ) -> usize {
        self.duties.read().get(&epoch).map_or(0, |duties| {
            duties
                .iter()
                .filter(|d| signing_pubkeys.contains(&d.pubkey))
                .count()
        })
    }

    /// Check if duties are known for the given epoch.
    fn has_duties_for_epoch(&self, epoch: Epoch) -> bool {
        self.duties.read().contains_key(&epoch)
    }

    /// Store duties for an epoch.
    pub(crate) fn set_duties(&self, epoch: Epoch, duties: Vec<InclusionListDutyData>) {
        self.duties.write().insert(epoch, duties);
    }

    /// Prune duties older than the given epoch.
    fn prune(&self, current_epoch: Epoch) {
        self.duties
            .write()
            .retain(|&epoch, _| epoch >= current_epoch.saturating_sub(1u64));
    }
}

/// Poll the beacon node for inclusion list committee duties for the current and next epoch.
///
/// This follows the same pattern as `poll_ptc_duties` — fetch duties for current+next epoch,
/// cache in the map, prune old epochs.
pub(crate) async fn poll_inclusion_list_duties<
    S: ValidatorStore + 'static,
    T: SlotClock + 'static,
>(
    duties_service: &Arc<DutiesService<S, T>>,
) -> Result<(), Error<S::Error>> {
    let spec = &duties_service.spec;
    let current_slot = duties_service
        .slot_clock
        .now()
        .ok_or(Error::UnableToReadSlotClock)?;
    let current_epoch = current_slot.epoch(S::E::slots_per_epoch());

    // If Heze is not yet activated, do not poll for IL duties.
    if spec
        .heze_fork_epoch
        .is_none_or(|heze_epoch| current_epoch < heze_epoch)
    {
        return Ok(());
    }

    let il_duties = &duties_service.inclusion_list_duties;

    // Fetch duties for the current epoch if not yet known.
    if !il_duties.has_duties_for_epoch(current_epoch) {
        poll_inclusion_list_duties_for_epoch(duties_service, current_epoch).await?;
    }

    // Fetch duties for the next epoch.
    let next_epoch = current_epoch.saturating_add(1u64);
    if !il_duties.has_duties_for_epoch(next_epoch) {
        poll_inclusion_list_duties_for_epoch(duties_service, next_epoch).await?;
    }

    // Prune old epochs.
    il_duties.prune(current_epoch);

    Ok(())
}

/// Fetch inclusion list duties for a specific epoch and store them in the map.
async fn poll_inclusion_list_duties_for_epoch<S: ValidatorStore, T: SlotClock + 'static>(
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
        "Fetching inclusion list duties"
    );

    let duties_response = duties_service
        .beacon_nodes
        .first_success(|beacon_node| {
            let indices = local_indices.clone();
            async move {
                beacon_node
                    .post_validator_duties_inclusion_list(epoch, &indices)
                    .await
            }
        })
        .await;

    match duties_response {
        Ok(res) => {
            let duties = res.data;
            debug!(
                %epoch,
                count = duties.len(),
                "Fetched inclusion list duties from BN"
            );
            duties_service
                .inclusion_list_duties
                .set_duties(epoch, duties);
        }
        Err(e) => {
            warn!(
                %epoch,
                error = %e,
                "Failed to download inclusion list duties"
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use types::Hash256;

    const SLOTS_PER_EPOCH: u64 = 8;

    fn make_duty(pubkey_byte: u8, validator_index: u64, slot: u64) -> InclusionListDutyData {
        let mut bytes = [0u8; 48];
        bytes[0] = pubkey_byte;
        InclusionListDutyData {
            pubkey: PublicKeyBytes::deserialize(&bytes).unwrap(),
            validator_index,
            slot: Slot::new(slot),
            il_committee_index: 0,
            inclusion_list_committee_root: Hash256::ZERO,
        }
    }

    fn pubkey_bytes(byte: u8) -> PublicKeyBytes {
        let mut bytes = [0u8; 48];
        bytes[0] = byte;
        PublicKeyBytes::deserialize(&bytes).unwrap()
    }

    #[test]
    fn new_map_is_empty() {
        let map = InclusionListDutiesMap::new();
        assert!(!map.has_duties_for_epoch(Epoch::new(0)));
        assert!(!map.has_duties_for_epoch(Epoch::new(1)));
    }

    #[test]
    fn default_is_new() {
        let map = InclusionListDutiesMap::default();
        assert!(!map.has_duties_for_epoch(Epoch::new(0)));
    }

    #[test]
    fn set_and_has_duties() {
        let map = InclusionListDutiesMap::new();
        assert!(!map.has_duties_for_epoch(Epoch::new(1)));
        map.set_duties(Epoch::new(1), vec![make_duty(1, 100, 8)]);
        assert!(map.has_duties_for_epoch(Epoch::new(1)));
        assert!(!map.has_duties_for_epoch(Epoch::new(0)));
    }

    #[test]
    fn set_duties_empty_vec_still_present() {
        let map = InclusionListDutiesMap::new();
        map.set_duties(Epoch::new(5), vec![]);
        assert!(map.has_duties_for_epoch(Epoch::new(5)));
    }

    #[test]
    fn set_duties_overwrites() {
        let map = InclusionListDutiesMap::new();
        map.set_duties(Epoch::new(1), vec![make_duty(1, 100, 8)]);
        map.set_duties(
            Epoch::new(1),
            vec![make_duty(2, 200, 9), make_duty(3, 300, 10)],
        );
        let duties = map.duties_for_slot(Slot::new(9), SLOTS_PER_EPOCH);
        assert_eq!(duties.len(), 1);
        assert_eq!(duties[0].validator_index, 200);
    }

    #[test]
    fn duties_for_slot_returns_matching() {
        let map = InclusionListDutiesMap::new();
        map.set_duties(
            Epoch::new(1),
            vec![
                make_duty(1, 100, 8),
                make_duty(2, 200, 9),
                make_duty(3, 300, 8),
                make_duty(4, 400, 10),
            ],
        );
        let slot_8 = map.duties_for_slot(Slot::new(8), SLOTS_PER_EPOCH);
        assert_eq!(slot_8.len(), 2);
        assert!(slot_8.iter().any(|d| d.validator_index == 100));
        assert!(slot_8.iter().any(|d| d.validator_index == 300));
    }

    #[test]
    fn duties_for_slot_empty_when_no_epoch() {
        let map = InclusionListDutiesMap::new();
        let duties = map.duties_for_slot(Slot::new(8), SLOTS_PER_EPOCH);
        assert!(duties.is_empty());
    }

    #[test]
    fn duties_for_slot_empty_when_no_slot_match() {
        let map = InclusionListDutiesMap::new();
        map.set_duties(Epoch::new(1), vec![make_duty(1, 100, 8)]);
        let duties = map.duties_for_slot(Slot::new(9), SLOTS_PER_EPOCH);
        assert!(duties.is_empty());
    }

    #[test]
    fn duty_count_filters_by_pubkey() {
        let map = InclusionListDutiesMap::new();
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
        let map = InclusionListDutiesMap::new();
        let signing = HashSet::new();
        assert_eq!(map.duty_count(Epoch::new(99), &signing), 0);
    }

    #[test]
    fn prune_removes_old_epochs() {
        let map = InclusionListDutiesMap::new();
        map.set_duties(Epoch::new(0), vec![make_duty(1, 100, 0)]);
        map.set_duties(Epoch::new(1), vec![make_duty(2, 200, 8)]);
        map.set_duties(Epoch::new(2), vec![make_duty(3, 300, 16)]);
        map.set_duties(Epoch::new(3), vec![make_duty(4, 400, 24)]);

        map.prune(Epoch::new(3));

        assert!(!map.has_duties_for_epoch(Epoch::new(0)));
        assert!(!map.has_duties_for_epoch(Epoch::new(1)));
        assert!(map.has_duties_for_epoch(Epoch::new(2)));
        assert!(map.has_duties_for_epoch(Epoch::new(3)));
    }

    #[test]
    fn prune_at_epoch_zero_keeps_everything() {
        let map = InclusionListDutiesMap::new();
        map.set_duties(Epoch::new(0), vec![make_duty(1, 100, 0)]);
        map.prune(Epoch::new(0));
        assert!(map.has_duties_for_epoch(Epoch::new(0)));
    }

    #[test]
    fn prune_empty_map_is_noop() {
        let map = InclusionListDutiesMap::new();
        map.prune(Epoch::new(100));
    }

    #[test]
    fn set_then_prune_then_query() {
        let map = InclusionListDutiesMap::new();
        map.set_duties(Epoch::new(5), vec![make_duty(1, 100, 40)]);
        map.set_duties(Epoch::new(6), vec![make_duty(2, 200, 48)]);
        map.set_duties(Epoch::new(7), vec![make_duty(3, 300, 56)]);

        map.prune(Epoch::new(7));

        assert!(!map.has_duties_for_epoch(Epoch::new(5)));
        assert!(map.has_duties_for_epoch(Epoch::new(6)));
        assert!(map.has_duties_for_epoch(Epoch::new(7)));

        let duties = map.duties_for_slot(Slot::new(48), SLOTS_PER_EPOCH);
        assert_eq!(duties.len(), 1);
        assert_eq!(duties[0].validator_index, 200);
    }
}
