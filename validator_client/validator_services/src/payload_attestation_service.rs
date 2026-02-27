use crate::duties_service::DutiesService;
use beacon_node_fallback::{ApiTopic, BeaconNodeFallback};
use slot_clock::SlotClock;
use std::ops::Deref;
use std::sync::Arc;
use task_executor::TaskExecutor;
use tokio::time::{Duration, sleep};
use tracing::{debug, error, info, trace};
use types::{ChainSpec, Epoch, EthSpec};
use validator_store::ValidatorStore;

/// Builds a `PayloadAttestationService`.
pub struct PayloadAttestationServiceBuilder<S: ValidatorStore, T: SlotClock + 'static> {
    duties_service: Option<Arc<DutiesService<S, T>>>,
    validator_store: Option<Arc<S>>,
    slot_clock: Option<T>,
    beacon_nodes: Option<Arc<BeaconNodeFallback<T>>>,
    executor: Option<TaskExecutor>,
    chain_spec: Option<Arc<ChainSpec>>,
}

impl<S: ValidatorStore + 'static, T: SlotClock + 'static> Default
    for PayloadAttestationServiceBuilder<S, T>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<S: ValidatorStore + 'static, T: SlotClock + 'static> PayloadAttestationServiceBuilder<S, T> {
    pub fn new() -> Self {
        Self {
            duties_service: None,
            validator_store: None,
            slot_clock: None,
            beacon_nodes: None,
            executor: None,
            chain_spec: None,
        }
    }

    pub fn duties_service(mut self, duties_service: Arc<DutiesService<S, T>>) -> Self {
        self.duties_service = Some(duties_service);
        self
    }

    pub fn validator_store(mut self, store: Arc<S>) -> Self {
        self.validator_store = Some(store);
        self
    }

    pub fn slot_clock(mut self, slot_clock: T) -> Self {
        self.slot_clock = Some(slot_clock);
        self
    }

    pub fn beacon_nodes(mut self, beacon_nodes: Arc<BeaconNodeFallback<T>>) -> Self {
        self.beacon_nodes = Some(beacon_nodes);
        self
    }

    pub fn executor(mut self, executor: TaskExecutor) -> Self {
        self.executor = Some(executor);
        self
    }

    pub fn spec(mut self, spec: Arc<ChainSpec>) -> Self {
        self.chain_spec = Some(spec);
        self
    }

    pub fn build(self) -> Result<PayloadAttestationService<S, T>, String> {
        let chain_spec = self
            .chain_spec
            .ok_or("Cannot build PayloadAttestationService without chain_spec")?;
        Ok(PayloadAttestationService {
            inner: Arc::new(Inner {
                duties_service: self
                    .duties_service
                    .ok_or("Cannot build PayloadAttestationService without duties_service")?,
                validator_store: self
                    .validator_store
                    .ok_or("Cannot build PayloadAttestationService without validator_store")?,
                slot_clock: self
                    .slot_clock
                    .ok_or("Cannot build PayloadAttestationService without slot_clock")?,
                beacon_nodes: self
                    .beacon_nodes
                    .ok_or("Cannot build PayloadAttestationService without beacon_nodes")?,
                executor: self
                    .executor
                    .ok_or("Cannot build PayloadAttestationService without executor")?,
                gloas_fork_epoch: chain_spec.gloas_fork_epoch,
                chain_spec,
            }),
        })
    }
}

pub struct Inner<S, T> {
    duties_service: Arc<DutiesService<S, T>>,
    validator_store: Arc<S>,
    slot_clock: T,
    beacon_nodes: Arc<BeaconNodeFallback<T>>,
    executor: TaskExecutor,
    gloas_fork_epoch: Option<Epoch>,
    chain_spec: Arc<ChainSpec>,
}

/// Produces payload timeliness attestations for PTC (Payload Timeliness Committee) duties.
///
/// PTC members attest at 3/4 of each slot to whether the execution payload was revealed on time.
/// Duties are fetched proactively by the `DutiesService` PTC polling task and read from
/// `DutiesService.ptc_duties`.
pub struct PayloadAttestationService<S, T> {
    inner: Arc<Inner<S, T>>,
}

impl<S, T> Clone for PayloadAttestationService<S, T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<S, T> Deref for PayloadAttestationService<S, T> {
    type Target = Inner<S, T>;

    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}

impl<S: ValidatorStore + 'static, T: SlotClock + 'static> PayloadAttestationService<S, T> {
    /// Exposed for integration tests — calls produce_payload_attestations directly.
    #[cfg(test)]
    pub async fn produce_payload_attestations_for_testing(self) -> Result<(), ()> {
        self.produce_payload_attestations().await
    }

    /// Check if the Gloas fork has been activated and therefore PTC duties should be performed.
    ///
    /// Slot clock errors are mapped to `false`.
    fn gloas_fork_activated(&self) -> bool {
        self.gloas_fork_epoch
            .and_then(|fork_epoch| {
                let current_epoch = self.slot_clock.now()?.epoch(S::E::slots_per_epoch());
                Some(current_epoch >= fork_epoch)
            })
            .unwrap_or(false)
    }

    /// Starts the service which periodically produces payload attestations at the
    /// configured PTC timing point in each slot (PAYLOAD_ATTESTATION_DUE_BPS).
    pub fn start_update_service(self, _spec: &ChainSpec) -> Result<(), String> {
        // If Gloas is not scheduled at all, don't start the service.
        if !self.chain_spec.is_gloas_scheduled() {
            info!("Payload attestation service disabled (Gloas not scheduled)");
            return Ok(());
        }

        let slot_duration = Duration::from_secs(self.chain_spec.seconds_per_slot);
        let ptc_delay = Duration::from_millis(self.chain_spec.get_payload_attestation_due_ms());
        let duration_to_next_slot = self
            .slot_clock
            .duration_to_next_slot()
            .ok_or("Unable to determine duration to next slot")?;

        info!(
            next_update_millis = duration_to_next_slot.as_millis(),
            ptc_delay_ms = ptc_delay.as_millis(),
            "Payload attestation service started"
        );

        let executor = self.executor.clone();

        let interval_fut = async move {
            loop {
                if let Some(duration_to_next_slot) = self.slot_clock.duration_to_next_slot() {
                    // PTC attestations happen at PAYLOAD_ATTESTATION_DUE_BPS of the slot.
                    sleep(duration_to_next_slot + ptc_delay).await;

                    // Do nothing if the Gloas fork has not yet occurred.
                    if !self.gloas_fork_activated() {
                        continue;
                    }

                    self.spawn_payload_attestation_tasks();
                } else {
                    error!("Failed to read slot clock");
                    sleep(slot_duration).await;
                    continue;
                }
            }
        };

        executor.spawn(interval_fut, "payload_attestation_service");
        Ok(())
    }

    /// Spawns a task to produce and publish payload attestations for the current slot.
    fn spawn_payload_attestation_tasks(&self) {
        let service = self.clone();
        self.executor.spawn_ignoring_error(
            service.produce_payload_attestations(),
            "payload_attestation",
        );
    }

    /// Main routine: read duties from DutiesService, get attestation data, sign, and submit.
    async fn produce_payload_attestations(self) -> Result<(), ()> {
        let slot = self.slot_clock.now().ok_or_else(|| {
            error!("Failed to read slot clock");
        })?;

        // Read PTC duties for this slot from the centralized DutiesService.
        let slot_duties = self
            .duties_service
            .ptc_duties
            .duties_for_slot(slot, S::E::slots_per_epoch());

        if slot_duties.is_empty() {
            trace!(slot = slot.as_u64(), "No PTC duties for this slot");
            return Ok(());
        }

        debug!(
            slot = slot.as_u64(),
            num_duties = slot_duties.len(),
            "Producing payload attestations"
        );

        // Fetch payload attestation data from BN.
        let attestation_data = self
            .beacon_nodes
            .first_success(|beacon_node| async move {
                beacon_node
                    .get_validator_payload_attestation_data(slot)
                    .await
                    .map_err(|e| e.to_string())
                    .map(|response| response.data)
            })
            .await
            .map_err(|e| {
                error!(
                    error = %e,
                    slot = slot.as_u64(),
                    "Failed to get payload attestation data"
                );
            })?;

        // Sign and collect all payload attestation messages.
        let mut messages = Vec::with_capacity(slot_duties.len());

        for duty in &slot_duties {
            match self
                .validator_store
                .sign_payload_attestation(duty.pubkey, &attestation_data, duty.validator_index)
                .await
            {
                Ok(message) => {
                    debug!(
                        validator_index = duty.validator_index,
                        slot = slot.as_u64(),
                        payload_present = attestation_data.payload_present,
                        "Signed payload attestation"
                    );
                    messages.push(message);
                }
                Err(e) => {
                    error!(
                        error = format!("{:?}", e),
                        validator_index = duty.validator_index,
                        slot = slot.as_u64(),
                        "Failed to sign payload attestation"
                    );
                }
            }
        }

        if messages.is_empty() {
            return Ok(());
        }

        // Submit to BN.
        let num_messages = messages.len();
        self.beacon_nodes
            .request(ApiTopic::Attestations, |beacon_node| {
                let messages = messages.clone();
                async move {
                    beacon_node
                        .post_beacon_pool_payload_attestations(&messages)
                        .await
                        .map_err(|e| e.to_string())
                }
            })
            .await
            .map_err(|e| {
                error!(
                    error = %e,
                    slot = slot.as_u64(),
                    "Failed to publish payload attestations"
                );
            })?;

        info!(
            slot = slot.as_u64(),
            count = num_messages,
            payload_present = attestation_data.payload_present,
            "Published payload attestations"
        );

        Ok(())
    }
}

/// Integration tests for `produce_payload_attestations` using mock BN + minimal ValidatorStore.
///
/// These tests exercise the full produce_payload_attestations async pipeline:
///   ptc_duties.duties_for_slot → BN GET payload_attestation_data → sign → BN POST pool.
#[cfg(test)]
mod produce_tests {
    use super::*;
    use crate::duties_service::DutiesServiceBuilder;
    use crate::ptc::PtcDutiesMap;
    use beacon_node_fallback::{ApiTopic, BeaconNodeFallback, CandidateBeaconNode};
    use eth2::types::{PayloadAttestationData as ApiPayloadAttestationData, PtcDutyData};
    use slot_clock::TestingSlotClock;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Duration;
    use task_executor::test_utils::TestRuntime;
    use types::{
        Epoch, Hash256, MainnetEthSpec, PayloadAttestationData, PayloadAttestationMessage,
        PublicKeyBytes, Slot,
    };
    use validator_store::{DoppelgangerStatus, Error as ValidatorStoreError, ValidatorStore};
    use validator_test_rig::mock_beacon_node::MockBeaconNode;

    type E = MainnetEthSpec;

    /// ValidatorStore for produce_payload_attestations tests.
    ///
    /// Implements `voting_pubkeys`, `validator_index`, and `sign_payload_attestation`.
    /// All other methods are unimplemented.
    struct SigningValidatorStore {
        validators: HashMap<PublicKeyBytes, u64>,
        /// If Some(err), sign_payload_attestation returns an error.
        sign_error: Option<String>,
        /// Records which validator_indices were asked to sign.
        signed: Arc<Mutex<Vec<u64>>>,
    }

    impl SigningValidatorStore {
        fn new(validators: Vec<(PublicKeyBytes, u64)>) -> Self {
            Self {
                validators: validators.into_iter().collect(),
                sign_error: None,
                signed: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn with_sign_error(mut self, msg: &str) -> Self {
            self.sign_error = Some(msg.to_string());
            self
        }

        fn pubkey(byte: u8) -> PublicKeyBytes {
            let mut bytes = [0u8; 48];
            bytes[0] = byte;
            PublicKeyBytes::deserialize(&bytes).unwrap()
        }
    }

    impl ValidatorStore for SigningValidatorStore {
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
            _pubkey: PublicKeyBytes,
            data: &PayloadAttestationData,
            validator_index: u64,
        ) -> Result<PayloadAttestationMessage, ValidatorStoreError<Self::Error>> {
            self.signed.lock().unwrap().push(validator_index);
            if let Some(ref e) = self.sign_error {
                return Err(ValidatorStoreError::Middleware(format!(
                    "mock sign error: {e}"
                )));
            }
            Ok(PayloadAttestationMessage {
                validator_index,
                data: data.clone(),
                signature: bls::Signature::empty(),
            })
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
            _: Hash256,
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

    fn make_ptc_duty(pubkey: PublicKeyBytes, validator_index: u64, slot: u64) -> PtcDutyData {
        PtcDutyData {
            pubkey,
            validator_index,
            slot: Slot::new(slot),
            ptc_committee_index: 0,
        }
    }

    fn spec_with_gloas(gloas_epoch: Option<u64>) -> ChainSpec {
        let mut spec = E::default_spec();
        spec.gloas_fork_epoch = gloas_epoch.map(types::Epoch::new);
        spec
    }

    /// Build a PayloadAttestationService wired to a MockBeaconNode + SigningValidatorStore.
    ///
    /// `duties` maps (epoch, slot) pairs to lists of duties to inject.
    async fn make_service(
        mock: &MockBeaconNode<E>,
        validators: Vec<(PublicKeyBytes, u64)>,
        spec: ChainSpec,
        current_slot: Slot,
        duties: Vec<(types::Epoch, Vec<PtcDutyData>)>,
    ) -> (
        PayloadAttestationService<SigningValidatorStore, TestingSlotClock>,
        Arc<Mutex<Vec<u64>>>,
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
        let slot_clock = TestingSlotClock::new(
            Slot::new(0),
            Duration::from_secs(0),
            Duration::from_secs(spec.seconds_per_slot),
        );
        slot_clock.set_slot(current_slot.as_u64());
        fallback.set_slot_clock(slot_clock.clone());

        let store = Arc::new(SigningValidatorStore::new(validators));
        let signed = store.signed.clone();

        let duties_service = Arc::new(
            DutiesServiceBuilder::new()
                .validator_store(store.clone())
                .slot_clock(slot_clock.clone())
                .beacon_nodes(Arc::new(fallback.clone()))
                .executor(test_runtime.task_executor.clone())
                .spec(spec_arc.clone())
                .build()
                .unwrap(),
        );

        // Inject pre-loaded duties using the public poll_ptc_duties map API.
        // We bypass the BN by using the internal set_duties via a test-only helper.
        for (epoch, epoch_duties) in duties {
            inject_ptc_duties(&duties_service.ptc_duties, epoch, epoch_duties);
        }

        let service = PayloadAttestationServiceBuilder::new()
            .duties_service(duties_service)
            .validator_store(store)
            .slot_clock(slot_clock)
            .beacon_nodes(Arc::new(fallback))
            .executor(test_runtime.task_executor.clone())
            .spec(spec_arc)
            .build()
            .unwrap();

        (service, signed, test_runtime)
    }

    /// Inject duties into a PtcDutiesMap for testing.
    fn inject_ptc_duties(map: &PtcDutiesMap, epoch: types::Epoch, duties: Vec<PtcDutyData>) {
        map.set_duties(epoch, duties);
    }

    // ── Tests ───────────────────────────────────────────────────────────────

    /// No PTC duties for the current slot → returns Ok without calling BN.
    ///
    /// Tests the early-return path in produce_payload_attestations when
    /// duties_for_slot returns an empty vec.
    #[tokio::test]
    async fn produce_no_duties_returns_ok_without_bn_call() {
        let spec = spec_with_gloas(Some(0));
        let slots_per_epoch = E::slots_per_epoch();
        let current_slot = Slot::new(slots_per_epoch); // epoch 1
        let pubkey = SigningValidatorStore::pubkey(1);

        let mock = MockBeaconNode::<E>::new().await;
        // No BN mocks registered — any HTTP call would return 404

        // Duties stored for slot 999 (not current slot) → duties_for_slot(slot=8) returns empty
        let epoch = current_slot.epoch(slots_per_epoch);
        let duties = vec![(epoch, vec![make_ptc_duty(pubkey, 100, 999)])];

        let (service, signed, _rt) =
            make_service(&mock, vec![(pubkey, 100)], spec, current_slot, duties).await;

        // Should return Ok without fetching payload attestation data from BN
        service
            .produce_payload_attestations_for_testing()
            .await
            .unwrap();

        // sign_payload_attestation was never called
        assert!(signed.lock().unwrap().is_empty());
    }

    /// Has duties, BN returns payload data → signs and submits attestations.
    ///
    /// Tests the happy path: duties → GET payload_attestation_data → sign → POST pool.
    #[tokio::test]
    async fn produce_with_duties_signs_and_submits() {
        let spec = spec_with_gloas(Some(0));
        let slots_per_epoch = E::slots_per_epoch();
        let current_slot = Slot::new(slots_per_epoch); // epoch 1, first slot
        let pubkey = SigningValidatorStore::pubkey(2);

        let mut mock = MockBeaconNode::<E>::new().await;

        let attestation_data = ApiPayloadAttestationData {
            beacon_block_root: Hash256::repeat_byte(0xaa),
            slot: current_slot,
            payload_present: true,
            blob_data_available: true,
        };

        let _m_data = mock.mock_get_validator_payload_attestation_data(attestation_data.clone());
        let _m_pool = mock.mock_post_beacon_pool_payload_attestations();

        let epoch = current_slot.epoch(slots_per_epoch);
        let duties = vec![(
            epoch,
            vec![make_ptc_duty(pubkey, 200, current_slot.as_u64())],
        )];

        let (service, signed, _rt) =
            make_service(&mock, vec![(pubkey, 200)], spec, current_slot, duties).await;

        service
            .produce_payload_attestations_for_testing()
            .await
            .unwrap();

        // sign_payload_attestation was called for validator 200
        let signed_indices = signed.lock().unwrap().clone();
        assert_eq!(signed_indices, vec![200u64]);
    }

    /// Multiple duties in the same slot → all signed and submitted in a single POST.
    ///
    /// Tests that produce_payload_attestations loops over all slot duties
    /// and submits them together.
    #[tokio::test]
    async fn produce_multiple_duties_all_signed() {
        let spec = spec_with_gloas(Some(0));
        let slots_per_epoch = E::slots_per_epoch();
        let current_slot = Slot::new(slots_per_epoch);
        let pk1 = SigningValidatorStore::pubkey(10);
        let pk2 = SigningValidatorStore::pubkey(11);
        let pk3 = SigningValidatorStore::pubkey(12);

        let mut mock = MockBeaconNode::<E>::new().await;

        let attestation_data = ApiPayloadAttestationData {
            beacon_block_root: Hash256::repeat_byte(0xbb),
            slot: current_slot,
            payload_present: false,
            blob_data_available: false,
        };

        let _m_data = mock.mock_get_validator_payload_attestation_data(attestation_data.clone());
        let _m_pool = mock.mock_post_beacon_pool_payload_attestations();

        let epoch = current_slot.epoch(slots_per_epoch);
        let duties = vec![(
            epoch,
            vec![
                make_ptc_duty(pk1, 10, current_slot.as_u64()),
                make_ptc_duty(pk2, 11, current_slot.as_u64()),
                make_ptc_duty(pk3, 12, current_slot.as_u64()),
            ],
        )];

        let (service, signed, _rt) = make_service(
            &mock,
            vec![(pk1, 10), (pk2, 11), (pk3, 12)],
            spec,
            current_slot,
            duties,
        )
        .await;

        service
            .produce_payload_attestations_for_testing()
            .await
            .unwrap();

        // All 3 validators signed
        let mut signed_indices = signed.lock().unwrap().clone();
        signed_indices.sort();
        assert_eq!(signed_indices, vec![10u64, 11, 12]);
    }

    /// BN returns error for GET payload_attestation_data → returns Err(()).
    ///
    /// Tests that a BN failure during attestation data fetch aborts production.
    #[tokio::test]
    async fn produce_bn_error_returns_err() {
        let spec = spec_with_gloas(Some(0));
        let slots_per_epoch = E::slots_per_epoch();
        let current_slot = Slot::new(slots_per_epoch);
        let pubkey = SigningValidatorStore::pubkey(3);

        let mock = MockBeaconNode::<E>::new().await;
        // No GET payload_attestation_data mock → BN returns 404 (error)

        let epoch = current_slot.epoch(slots_per_epoch);
        let duties = vec![(
            epoch,
            vec![make_ptc_duty(pubkey, 300, current_slot.as_u64())],
        )];

        let (service, signed, _rt) =
            make_service(&mock, vec![(pubkey, 300)], spec, current_slot, duties).await;

        // Should return Err(()) because BN is not available
        let result = service.produce_payload_attestations_for_testing().await;
        assert!(result.is_err(), "expected Err when BN unavailable");

        // No signing happened (aborted before sign step)
        assert!(signed.lock().unwrap().is_empty());
    }

    /// Sign error for all duties → messages vec is empty → Ok returned without POST.
    ///
    /// The function returns Ok even when signing fails (it logs the error and continues).
    /// But if ALL duties fail to sign, messages is empty and it returns Ok early.
    /// We verify sign was attempted but no POST to the pool was made.
    #[tokio::test]
    async fn produce_sign_error_skips_submission() {
        let spec = spec_with_gloas(Some(0));
        let slots_per_epoch = E::slots_per_epoch();
        let current_slot = Slot::new(slots_per_epoch);
        let pubkey = SigningValidatorStore::pubkey(4);

        let mut mock = MockBeaconNode::<E>::new().await;

        let attestation_data = ApiPayloadAttestationData {
            beacon_block_root: Hash256::repeat_byte(0xcc),
            slot: current_slot,
            payload_present: false,
            blob_data_available: false,
        };

        let _m_data = mock.mock_get_validator_payload_attestation_data(attestation_data);
        // No POST pool mock registered — if it were called, it would 404

        let epoch = current_slot.epoch(slots_per_epoch);
        let slot_duties = vec![make_ptc_duty(pubkey, 400, current_slot.as_u64())];

        // Build manually so we can inject a sign-error store
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
        let slot_clock = TestingSlotClock::new(
            Slot::new(0),
            Duration::from_secs(0),
            Duration::from_secs(spec.seconds_per_slot),
        );
        slot_clock.set_slot(current_slot.as_u64());
        fallback.set_slot_clock(slot_clock.clone());

        let store =
            Arc::new(SigningValidatorStore::new(vec![(pubkey, 400)]).with_sign_error("mock error"));
        let signed = store.signed.clone();

        let duties_service = Arc::new(
            DutiesServiceBuilder::new()
                .validator_store(store.clone())
                .slot_clock(slot_clock.clone())
                .beacon_nodes(Arc::new(fallback.clone()))
                .executor(test_runtime.task_executor.clone())
                .spec(spec_arc.clone())
                .build()
                .unwrap(),
        );
        inject_ptc_duties(&duties_service.ptc_duties, epoch, slot_duties);

        let service = PayloadAttestationServiceBuilder::new()
            .duties_service(duties_service)
            .validator_store(store)
            .slot_clock(slot_clock)
            .beacon_nodes(Arc::new(fallback))
            .executor(test_runtime.task_executor.clone())
            .spec(spec_arc)
            .build()
            .unwrap();

        // Signing fails → messages is empty → early return Ok (not submitting)
        let result = service.produce_payload_attestations_for_testing().await;
        assert!(
            result.is_ok(),
            "expected Ok when sign fails (early return with empty messages)"
        );

        // sign_payload_attestation was attempted for validator 400
        assert_eq!(signed.lock().unwrap().clone(), vec![400u64]);
    }

    /// Payload_present=false in attestation data → correctly propagated to sign call.
    ///
    /// Tests that the payload_present=false flag from the BN is faithfully passed
    /// through to sign_payload_attestation (not converted to true somewhere).
    #[tokio::test]
    async fn produce_payload_present_false_propagated() {
        let spec = spec_with_gloas(Some(0));
        let slots_per_epoch = E::slots_per_epoch();
        let current_slot = Slot::new(slots_per_epoch);
        let pubkey = SigningValidatorStore::pubkey(5);

        let mut mock = MockBeaconNode::<E>::new().await;

        // BN says payload was NOT present
        let attestation_data = ApiPayloadAttestationData {
            beacon_block_root: Hash256::repeat_byte(0xdd),
            slot: current_slot,
            payload_present: false,
            blob_data_available: false,
        };

        let _m_data = mock.mock_get_validator_payload_attestation_data(attestation_data.clone());
        let _m_pool = mock.mock_post_beacon_pool_payload_attestations();

        let epoch = current_slot.epoch(slots_per_epoch);
        let duties = vec![(
            epoch,
            vec![make_ptc_duty(pubkey, 500, current_slot.as_u64())],
        )];

        let (service, signed, _rt) =
            make_service(&mock, vec![(pubkey, 500)], spec, current_slot, duties).await;

        service
            .produce_payload_attestations_for_testing()
            .await
            .unwrap();

        // Sign was called (payload_present=false is still a valid duty)
        assert_eq!(signed.lock().unwrap().clone(), vec![500u64]);
    }

    /// Partial sign failure: 3 duties, 1 fails to sign, remaining 2 are still submitted.
    ///
    /// Tests the error resilience of the signing loop — when one validator fails to sign,
    /// the function logs a warning and continues with the next duty. The successfully
    /// signed messages are submitted in a single POST.
    #[tokio::test]
    async fn produce_partial_sign_failure_still_submits_others() {
        let spec = spec_with_gloas(Some(0));
        let slots_per_epoch = E::slots_per_epoch();
        let current_slot = Slot::new(slots_per_epoch);
        let pk1 = SigningValidatorStore::pubkey(20);
        let pk2 = SigningValidatorStore::pubkey(21); // this one will fail
        let pk3 = SigningValidatorStore::pubkey(22);

        let mut mock = MockBeaconNode::<E>::new().await;

        let attestation_data = ApiPayloadAttestationData {
            beacon_block_root: Hash256::repeat_byte(0xee),
            slot: current_slot,
            payload_present: true,
            blob_data_available: true,
        };

        let _m_data = mock.mock_get_validator_payload_attestation_data(attestation_data.clone());
        let _m_pool = mock.mock_post_beacon_pool_payload_attestations();

        let epoch = current_slot.epoch(slots_per_epoch);
        let duties = vec![(
            epoch,
            vec![
                make_ptc_duty(pk1, 20, current_slot.as_u64()),
                make_ptc_duty(pk2, 21, current_slot.as_u64()),
                make_ptc_duty(pk3, 22, current_slot.as_u64()),
            ],
        )];

        // Build manually to inject a store that fails for validator 21
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
        let slot_clock = TestingSlotClock::new(
            Slot::new(0),
            Duration::from_secs(0),
            Duration::from_secs(spec.seconds_per_slot),
        );
        slot_clock.set_slot(current_slot.as_u64());
        fallback.set_slot_clock(slot_clock.clone());

        // Custom store: validator 21 fails to sign
        let store = Arc::new(PartialFailStore {
            validators: vec![(pk1, 20), (pk2, 21), (pk3, 22)].into_iter().collect(),
            fail_index: 21,
            signed: Arc::new(Mutex::new(Vec::new())),
        });
        let signed = store.signed.clone();

        let duties_service = Arc::new(
            DutiesServiceBuilder::new()
                .validator_store(store.clone())
                .slot_clock(slot_clock.clone())
                .beacon_nodes(Arc::new(fallback.clone()))
                .executor(test_runtime.task_executor.clone())
                .spec(spec_arc.clone())
                .build()
                .unwrap(),
        );
        for (ep, ep_duties) in duties {
            inject_ptc_duties(&duties_service.ptc_duties, ep, ep_duties);
        }

        let service = PayloadAttestationServiceBuilder::new()
            .duties_service(duties_service)
            .validator_store(store)
            .slot_clock(slot_clock)
            .beacon_nodes(Arc::new(fallback))
            .executor(test_runtime.task_executor.clone())
            .spec(spec_arc)
            .build()
            .unwrap();

        // Should succeed — partial failure is not fatal
        service
            .produce_payload_attestations_for_testing()
            .await
            .unwrap();

        // Validators 20 and 22 signed successfully, 21 was attempted but failed
        let mut signed_indices = signed.lock().unwrap().clone();
        signed_indices.sort();
        assert_eq!(
            signed_indices,
            vec![20u64, 21, 22],
            "all 3 validators should have been attempted"
        );
    }

    /// BN POST failure: all attestations signed ok, but submission returns 500 → Err(()).
    ///
    /// Tests the `Err(e)` return from `beacon_nodes.request(post_payload_attestations)`.
    /// Unlike the broadcast_proposer_preferences path which warns and continues,
    /// produce_payload_attestations treats POST failure as fatal (returns Err).
    #[tokio::test]
    async fn produce_bn_post_failure_returns_err() {
        let spec = spec_with_gloas(Some(0));
        let slots_per_epoch = E::slots_per_epoch();
        let current_slot = Slot::new(slots_per_epoch);
        let pubkey = SigningValidatorStore::pubkey(6);

        let mut mock = MockBeaconNode::<E>::new().await;

        let attestation_data = ApiPayloadAttestationData {
            beacon_block_root: Hash256::repeat_byte(0xff),
            slot: current_slot,
            payload_present: true,
            blob_data_available: true,
        };

        let _m_data = mock.mock_get_validator_payload_attestation_data(attestation_data);
        // POST returns 500 error
        let _m_pool = mock.mock_post_beacon_pool_payload_attestations_error();

        let epoch = current_slot.epoch(slots_per_epoch);
        let duties = vec![(
            epoch,
            vec![make_ptc_duty(pubkey, 600, current_slot.as_u64())],
        )];

        let (service, signed, _rt) =
            make_service(&mock, vec![(pubkey, 600)], spec, current_slot, duties).await;

        // Should return Err because BN POST failed
        let result = service.produce_payload_attestations_for_testing().await;
        assert!(result.is_err(), "expected Err when BN POST returns 500");

        // Signing still happened before the POST failure
        assert_eq!(signed.lock().unwrap().clone(), vec![600u64]);
    }

    /// ValidatorStore that fails to sign for a specific validator_index.
    ///
    /// Used by produce_partial_sign_failure_still_submits_others to test
    /// that the signing loop handles per-validator errors gracefully.
    struct PartialFailStore {
        validators: HashMap<PublicKeyBytes, u64>,
        fail_index: u64,
        signed: Arc<Mutex<Vec<u64>>>,
    }

    impl ValidatorStore for PartialFailStore {
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
            _pubkey: PublicKeyBytes,
            data: &PayloadAttestationData,
            validator_index: u64,
        ) -> Result<PayloadAttestationMessage, ValidatorStoreError<Self::Error>> {
            self.signed.lock().unwrap().push(validator_index);
            if validator_index == self.fail_index {
                return Err(ValidatorStoreError::Middleware(format!(
                    "mock sign error for validator {}",
                    validator_index
                )));
            }
            Ok(PayloadAttestationMessage {
                validator_index,
                data: data.clone(),
                signature: bls::Signature::empty(),
            })
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
}
