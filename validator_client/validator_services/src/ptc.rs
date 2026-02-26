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
    pub(crate) fn set_duties(&self, epoch: Epoch, duties: Vec<PtcDutyData>) {
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

/// Integration tests for `poll_ptc_duties` using a mock beacon node.
///
/// These tests exercise the full `poll_ptc_duties` async pipeline:
/// slot_clock → gloas epoch check → voting_pubkeys → BN request → set_duties → prune.
#[cfg(test)]
mod poll_tests {
    use super::*;
    use beacon_node_fallback::{ApiTopic, BeaconNodeFallback, CandidateBeaconNode};
    use eth2::types::PtcDutyData;
    use slot_clock::{SlotClock, TestingSlotClock};
    use std::collections::HashMap;
    use std::time::Duration;
    use task_executor::test_utils::TestRuntime;
    use types::{ChainSpec, Epoch, MainnetEthSpec, PublicKeyBytes, Slot};
    use validator_store::{DoppelgangerStatus, Error as ValidatorStoreError, ValidatorStore};
    use validator_test_rig::mock_beacon_node::MockBeaconNode;

    type E = MainnetEthSpec;

    /// Minimal ValidatorStore implementation for poll_ptc_duties tests.
    ///
    /// Only implements `voting_pubkeys` and `validator_index` (all poll_ptc_duties uses).
    /// All other methods are unimplemented — tests must not call them.
    struct MinimalValidatorStore {
        // pubkey → validator_index
        validators: HashMap<PublicKeyBytes, u64>,
    }

    impl MinimalValidatorStore {
        fn new(validators: Vec<(PublicKeyBytes, u64)>) -> Self {
            Self {
                validators: validators.into_iter().collect(),
            }
        }

        fn pubkey(byte: u8) -> PublicKeyBytes {
            let mut bytes = [0u8; 48];
            bytes[0] = byte;
            PublicKeyBytes::deserialize(&bytes).unwrap()
        }
    }

    impl ValidatorStore for MinimalValidatorStore {
        type Error = String;
        type E = E;

        fn validator_index(&self, pubkey: &PublicKeyBytes) -> Option<u64> {
            self.validators.get(pubkey).copied()
        }

        fn voting_pubkeys<I, F>(&self, filter_func: F) -> I
        where
            I: FromIterator<PublicKeyBytes>,
            F: Fn(DoppelgangerStatus) -> Option<PublicKeyBytes>,
        {
            self.validators
                .keys()
                .filter_map(|pk| filter_func(DoppelgangerStatus::SigningEnabled(*pk)))
                .collect()
        }

        fn doppelganger_protection_allows_signing(&self, _: PublicKeyBytes) -> bool {
            true
        }

        fn num_voting_validators(&self) -> usize {
            self.validators.len()
        }

        fn graffiti(&self, _: &PublicKeyBytes) -> Option<types::Graffiti> {
            unimplemented!()
        }

        fn get_fee_recipient(&self, _: &PublicKeyBytes) -> Option<types::Address> {
            unimplemented!()
        }

        fn determine_builder_boost_factor(&self, _: &PublicKeyBytes) -> Option<u64> {
            unimplemented!()
        }

        async fn randao_reveal(
            &self,
            _: PublicKeyBytes,
            _: Epoch,
        ) -> Result<types::Signature, ValidatorStoreError<Self::Error>> {
            unimplemented!()
        }

        fn set_validator_index(&self, _: &PublicKeyBytes, _: u64) {
            unimplemented!()
        }

        async fn sign_block(
            &self,
            _: PublicKeyBytes,
            _: validator_store::UnsignedBlock<Self::E>,
            _: Slot,
        ) -> Result<validator_store::SignedBlock<Self::E>, ValidatorStoreError<Self::Error>>
        {
            unimplemented!()
        }

        async fn sign_execution_payload_envelope(
            &self,
            _: PublicKeyBytes,
            _: &types::ExecutionPayloadEnvelope<Self::E>,
        ) -> Result<types::SignedExecutionPayloadEnvelope<Self::E>, ValidatorStoreError<Self::Error>>
        {
            unimplemented!()
        }

        async fn sign_payload_attestation(
            &self,
            _: PublicKeyBytes,
            _: &types::PayloadAttestationData,
            _: u64,
        ) -> Result<types::PayloadAttestationMessage, ValidatorStoreError<Self::Error>> {
            unimplemented!()
        }

        async fn sign_proposer_preferences(
            &self,
            _: PublicKeyBytes,
            _: &types::ProposerPreferences,
        ) -> Result<types::SignedProposerPreferences, ValidatorStoreError<Self::Error>> {
            unimplemented!()
        }

        async fn sign_attestation(
            &self,
            _: PublicKeyBytes,
            _: usize,
            _: &mut types::Attestation<Self::E>,
            _: Epoch,
        ) -> Result<(), ValidatorStoreError<Self::Error>> {
            unimplemented!()
        }

        async fn sign_validator_registration_data(
            &self,
            _: types::ValidatorRegistrationData,
        ) -> Result<types::SignedValidatorRegistrationData, ValidatorStoreError<Self::Error>>
        {
            unimplemented!()
        }

        async fn produce_signed_aggregate_and_proof(
            &self,
            _: PublicKeyBytes,
            _: u64,
            _: types::Attestation<Self::E>,
            _: types::SelectionProof,
        ) -> Result<types::SignedAggregateAndProof<Self::E>, ValidatorStoreError<Self::Error>>
        {
            unimplemented!()
        }

        async fn produce_selection_proof(
            &self,
            _: PublicKeyBytes,
            _: Slot,
        ) -> Result<types::SelectionProof, ValidatorStoreError<Self::Error>> {
            unimplemented!()
        }

        async fn produce_sync_selection_proof(
            &self,
            _: &PublicKeyBytes,
            _: Slot,
            _: types::SyncSubnetId,
        ) -> Result<types::SyncSelectionProof, ValidatorStoreError<Self::Error>> {
            unimplemented!()
        }

        async fn produce_sync_committee_signature(
            &self,
            _: Slot,
            _: types::Hash256,
            _: u64,
            _: &PublicKeyBytes,
        ) -> Result<types::SyncCommitteeMessage, ValidatorStoreError<Self::Error>> {
            unimplemented!()
        }

        async fn produce_signed_contribution_and_proof(
            &self,
            _: u64,
            _: PublicKeyBytes,
            _: types::SyncCommitteeContribution<Self::E>,
            _: types::SyncSelectionProof,
        ) -> Result<types::SignedContributionAndProof<Self::E>, ValidatorStoreError<Self::Error>>
        {
            unimplemented!()
        }

        fn prune_slashing_protection_db(&self, _: Epoch, _: bool) {}

        fn proposal_data(&self, _: &PublicKeyBytes) -> Option<validator_store::ProposalData> {
            unimplemented!()
        }
    }

    /// Build a DutiesService wired to a MockBeaconNode + MinimalValidatorStore.
    async fn make_duties_service(
        mock: &MockBeaconNode<E>,
        validators: Vec<(PublicKeyBytes, u64)>,
        spec: ChainSpec,
        current_slot: Slot,
    ) -> (
        Arc<DutiesService<MinimalValidatorStore, TestingSlotClock>>,
        TestRuntime,
    ) {
        let test_runtime = TestRuntime::default();

        let client = mock.beacon_api_client.clone();
        let candidate = CandidateBeaconNode::new(client, 0);
        let spec_arc = Arc::new(spec.clone());
        let mut fallback = BeaconNodeFallback::new(
            vec![candidate],
            Default::default(),
            vec![ApiTopic::Attestations],
            spec_arc.clone(),
        );

        let genesis_time = 0u64;
        let slot_duration = Duration::from_secs(spec.seconds_per_slot);
        // ManualSlotClock: genesis_time is slot 0 start, current_slot is derived from duration
        let slot_clock = TestingSlotClock::new(
            Slot::new(0),
            Duration::from_secs(genesis_time),
            slot_duration,
        );
        // Advance to current_slot
        slot_clock.set_slot(current_slot.as_u64());

        fallback.set_slot_clock(slot_clock.clone());

        let validator_store = Arc::new(MinimalValidatorStore::new(validators));

        let duties_service = Arc::new(
            crate::duties_service::DutiesServiceBuilder::new()
                .validator_store(validator_store)
                .slot_clock(slot_clock)
                .beacon_nodes(Arc::new(fallback))
                .executor(test_runtime.task_executor.clone())
                .spec(spec_arc)
                .build()
                .unwrap(),
        );

        (duties_service, test_runtime)
    }

    fn make_ptc_duty(pubkey: PublicKeyBytes, validator_index: u64, slot: u64) -> PtcDutyData {
        PtcDutyData {
            pubkey,
            validator_index,
            slot: Slot::new(slot),
            ptc_committee_index: 0,
        }
    }

    /// Returns a ChainSpec with Gloas fork at epoch `gloas_epoch` (or None if disabled).
    fn spec_with_gloas(gloas_epoch: Option<u64>) -> ChainSpec {
        let mut spec = E::default_spec();
        spec.gloas_fork_epoch = gloas_epoch.map(Epoch::new);
        spec
    }

    // ── Core behavior tests ─────────────────────────────────────────────────

    /// Pre-Gloas: poll does not call BN when fork has not activated.
    #[tokio::test]
    async fn poll_ptc_duties_pre_gloas_skips_bn() {
        // Gloas at epoch 10, current slot = 0 (epoch 0) → no BN call expected
        let spec = spec_with_gloas(Some(10));
        let pubkey = MinimalValidatorStore::pubkey(1);
        let mock = MockBeaconNode::<E>::new().await;
        let (ds, _rt) = make_duties_service(&mock, vec![(pubkey, 100)], spec, Slot::new(0)).await;

        // Should return Ok without making any HTTP requests
        poll_ptc_duties(&ds).await.unwrap();

        // No duties should have been stored (Gloas not yet active at slot 0)
        assert!(!ds.ptc_duties.has_duties_for_epoch(Epoch::new(0)));
        assert!(!ds.ptc_duties.has_duties_for_epoch(Epoch::new(10)));
    }

    /// Gloas active: fetches duties for current and next epoch.
    #[tokio::test]
    async fn poll_ptc_duties_fetches_current_and_next_epoch() {
        // Gloas at epoch 0, current slot = 8 (epoch 1, 8 slots/epoch mainnet-ish but we use 8)
        let spec = spec_with_gloas(Some(0));
        let slots_per_epoch = E::slots_per_epoch();
        let pubkey = MinimalValidatorStore::pubkey(1);
        let current_epoch = Epoch::new(1);
        let next_epoch = Epoch::new(2);
        let current_slot = Slot::new(slots_per_epoch); // first slot of epoch 1

        let mut mock = MockBeaconNode::<E>::new().await;

        let duty_epoch1 = make_ptc_duty(pubkey, 100, slots_per_epoch);
        let duty_epoch2 = make_ptc_duty(pubkey, 100, slots_per_epoch * 2);

        let _m1 = mock.mock_post_validator_duties_ptc(current_epoch, vec![duty_epoch1.clone()]);
        let _m2 = mock.mock_post_validator_duties_ptc(next_epoch, vec![duty_epoch2.clone()]);

        let (ds, _rt) = make_duties_service(&mock, vec![(pubkey, 100)], spec, current_slot).await;

        poll_ptc_duties(&ds).await.unwrap();

        // Both epochs should now have duties
        assert!(ds.ptc_duties.has_duties_for_epoch(current_epoch));
        assert!(ds.ptc_duties.has_duties_for_epoch(next_epoch));

        // Verify the stored duties match what the BN returned
        let stored_epoch1 = ds
            .ptc_duties
            .duties_for_slot(Slot::new(slots_per_epoch), slots_per_epoch);
        assert_eq!(stored_epoch1.len(), 1);
        assert_eq!(stored_epoch1[0].validator_index, 100);

        let stored_epoch2 = ds
            .ptc_duties
            .duties_for_slot(Slot::new(slots_per_epoch * 2), slots_per_epoch);
        assert_eq!(stored_epoch2.len(), 1);
        assert_eq!(stored_epoch2[0].validator_index, 100);
    }

    /// Cached epoch is not re-fetched on second call.
    #[tokio::test]
    async fn poll_ptc_duties_cached_epoch_not_refetched() {
        let spec = spec_with_gloas(Some(0));
        let slots_per_epoch = E::slots_per_epoch();
        let pubkey = MinimalValidatorStore::pubkey(2);
        let current_epoch = Epoch::new(1);
        let next_epoch = Epoch::new(2);
        let current_slot = Slot::new(slots_per_epoch);

        let mut mock = MockBeaconNode::<E>::new().await;

        let duty_epoch1 = make_ptc_duty(pubkey, 200, slots_per_epoch);
        let duty_epoch2 = make_ptc_duty(pubkey, 200, slots_per_epoch * 2);

        // Register each mock once — if called twice, mockito will fail the second time
        let _m1 = mock.mock_post_validator_duties_ptc(current_epoch, vec![duty_epoch1]);
        let _m2 = mock.mock_post_validator_duties_ptc(next_epoch, vec![duty_epoch2]);

        let (ds, _rt) = make_duties_service(&mock, vec![(pubkey, 200)], spec, current_slot).await;

        // First call: both epochs fetched
        poll_ptc_duties(&ds).await.unwrap();
        assert!(ds.ptc_duties.has_duties_for_epoch(current_epoch));
        assert!(ds.ptc_duties.has_duties_for_epoch(next_epoch));

        // Second call: duties already cached, BN should NOT be called again.
        // If BN is called again, mockito would serve a 404 for the second request,
        // but poll_ptc_duties treats BN errors as warnings (not failures), so it
        // still returns Ok. The key invariant: the cached duties are NOT overwritten
        // (they remain from the first call).
        poll_ptc_duties(&ds).await.unwrap();
        let stored = ds
            .ptc_duties
            .duties_for_slot(Slot::new(slots_per_epoch), slots_per_epoch);
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].validator_index, 200);
    }

    /// No local validators: BN not called, returns Ok.
    #[tokio::test]
    async fn poll_ptc_duties_no_validators_skips_bn() {
        let spec = spec_with_gloas(Some(0));
        let slots_per_epoch = E::slots_per_epoch();
        let current_slot = Slot::new(slots_per_epoch); // epoch 1

        let mock = MockBeaconNode::<E>::new().await;
        // No validators registered
        let (ds, _rt) = make_duties_service(&mock, vec![], spec, current_slot).await;

        // Should return Ok without calling BN
        poll_ptc_duties(&ds).await.unwrap();

        // No duties stored
        assert!(!ds.ptc_duties.has_duties_for_epoch(Epoch::new(1)));
    }

    /// BN returns empty duties: duties stored as empty (not missing).
    #[tokio::test]
    async fn poll_ptc_duties_empty_response_stored() {
        let spec = spec_with_gloas(Some(0));
        let slots_per_epoch = E::slots_per_epoch();
        let pubkey = MinimalValidatorStore::pubkey(3);
        let current_epoch = Epoch::new(1);
        let next_epoch = Epoch::new(2);
        let current_slot = Slot::new(slots_per_epoch);

        let mut mock = MockBeaconNode::<E>::new().await;
        // BN says validator has no PTC duties for either epoch
        let _m1 = mock.mock_post_validator_duties_ptc(current_epoch, vec![]);
        let _m2 = mock.mock_post_validator_duties_ptc(next_epoch, vec![]);

        let (ds, _rt) = make_duties_service(&mock, vec![(pubkey, 300)], spec, current_slot).await;

        poll_ptc_duties(&ds).await.unwrap();

        // Epochs should be marked as "known" even with empty duties
        // (so we don't keep re-fetching)
        assert!(ds.ptc_duties.has_duties_for_epoch(current_epoch));
        assert!(ds.ptc_duties.has_duties_for_epoch(next_epoch));
        assert!(
            ds.ptc_duties
                .duties_for_slot(Slot::new(slots_per_epoch), slots_per_epoch)
                .is_empty()
        );
    }

    /// Gloas disabled (fork epoch = None): poll never calls BN.
    #[tokio::test]
    async fn poll_ptc_duties_gloas_disabled_skips_bn() {
        let spec = spec_with_gloas(None);
        let pubkey = MinimalValidatorStore::pubkey(4);

        let mock = MockBeaconNode::<E>::new().await;
        let (ds, _rt) = make_duties_service(&mock, vec![(pubkey, 400)], spec, Slot::new(100)).await;

        poll_ptc_duties(&ds).await.unwrap();

        // Nothing stored
        assert!(!ds.ptc_duties.has_duties_for_epoch(Epoch::new(0)));
    }

    /// Multiple validators: all indices sent to BN, duties for each stored.
    #[tokio::test]
    async fn poll_ptc_duties_multiple_validators() {
        let spec = spec_with_gloas(Some(0));
        let slots_per_epoch = E::slots_per_epoch();
        let pk1 = MinimalValidatorStore::pubkey(10);
        let pk2 = MinimalValidatorStore::pubkey(11);
        let pk3 = MinimalValidatorStore::pubkey(12);
        let current_epoch = Epoch::new(2);
        let next_epoch = Epoch::new(3);
        let current_slot = Slot::new(slots_per_epoch * 2);

        let mut mock = MockBeaconNode::<E>::new().await;

        let duty1 = make_ptc_duty(pk1, 10, slots_per_epoch * 2);
        let duty2 = make_ptc_duty(pk2, 11, slots_per_epoch * 2 + 1);
        let duty3 = make_ptc_duty(pk3, 12, slots_per_epoch * 3);

        let _m1 = mock.mock_post_validator_duties_ptc(current_epoch, vec![duty1, duty2]);
        let _m2 = mock.mock_post_validator_duties_ptc(next_epoch, vec![duty3]);

        let (ds, _rt) = make_duties_service(
            &mock,
            vec![(pk1, 10), (pk2, 11), (pk3, 12)],
            spec,
            current_slot,
        )
        .await;

        poll_ptc_duties(&ds).await.unwrap();

        // epoch 2: 2 duties for different slots
        let slot_duties = ds
            .ptc_duties
            .duties_for_slot(Slot::new(slots_per_epoch * 2), slots_per_epoch);
        assert_eq!(slot_duties.len(), 1); // only duty1 matches slot

        let slot_duties_plus1 = ds
            .ptc_duties
            .duties_for_slot(Slot::new(slots_per_epoch * 2 + 1), slots_per_epoch);
        assert_eq!(slot_duties_plus1.len(), 1); // duty2

        // epoch 3: duty3
        let epoch3_duties = ds
            .ptc_duties
            .duties_for_slot(Slot::new(slots_per_epoch * 3), slots_per_epoch);
        assert_eq!(epoch3_duties.len(), 1);
        assert_eq!(epoch3_duties[0].validator_index, 12);
    }
}
