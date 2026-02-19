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
