use beacon_node_fallback::{ApiTopic, BeaconNodeFallback};
use eth2::types::PtcDutyData;
use slot_clock::SlotClock;
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;
use task_executor::TaskExecutor;
use tokio::sync::RwLock;
use tokio::time::{Duration, sleep};
use tracing::{debug, error, info, trace};
use types::{ChainSpec, Epoch, EthSpec};
use validator_store::{DoppelgangerStatus, ValidatorStore};

/// Builds a `PayloadAttestationService`.
pub struct PayloadAttestationServiceBuilder<S: ValidatorStore, T: SlotClock + 'static> {
    validator_store: Option<Arc<S>>,
    slot_clock: Option<T>,
    beacon_nodes: Option<Arc<BeaconNodeFallback<T>>>,
    executor: Option<TaskExecutor>,
    gloas_fork_epoch: Option<Epoch>,
}

impl<S: ValidatorStore + 'static, T: SlotClock + 'static> Default
    for PayloadAttestationServiceBuilder<S, T>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<S: ValidatorStore + 'static, T: SlotClock + 'static>
    PayloadAttestationServiceBuilder<S, T>
{
    pub fn new() -> Self {
        Self {
            validator_store: None,
            slot_clock: None,
            beacon_nodes: None,
            executor: None,
            gloas_fork_epoch: None,
        }
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

    pub fn spec(mut self, spec: &ChainSpec) -> Self {
        self.gloas_fork_epoch = spec.gloas_fork_epoch;
        self
    }

    pub fn build(self) -> Result<PayloadAttestationService<S, T>, String> {
        Ok(PayloadAttestationService {
            inner: Arc::new(Inner {
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
                duties_cache: RwLock::new(DutiesCache::default()),
                gloas_fork_epoch: self.gloas_fork_epoch,
            }),
        })
    }
}

/// Cached PTC duties for the current and next epoch.
#[derive(Default)]
struct DutiesCache {
    duties: HashMap<Epoch, Vec<PtcDutyData>>,
}

pub struct Inner<S, T> {
    validator_store: Arc<S>,
    slot_clock: T,
    beacon_nodes: Arc<BeaconNodeFallback<T>>,
    executor: TaskExecutor,
    duties_cache: RwLock<DutiesCache>,
    gloas_fork_epoch: Option<Epoch>,
}

/// Produces payload timeliness attestations for PTC (Payload Timeliness Committee) duties.
///
/// PTC members attest at 3/4 of each slot to whether the execution payload was revealed on time.
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

    /// Starts the service which periodically produces payload attestations at 3/4 of each slot.
    pub fn start_update_service(self, spec: &ChainSpec) -> Result<(), String> {
        // If Gloas is not scheduled at all, don't start the service.
        if !spec.is_gloas_scheduled() {
            info!("Payload attestation service disabled (Gloas not scheduled)");
            return Ok(());
        }

        let slot_duration = Duration::from_secs(spec.seconds_per_slot);
        let duration_to_next_slot = self
            .slot_clock
            .duration_to_next_slot()
            .ok_or("Unable to determine duration to next slot")?;

        info!(
            next_update_millis = duration_to_next_slot.as_millis(),
            "Payload attestation service started"
        );

        let executor = self.executor.clone();

        let interval_fut = async move {
            loop {
                if let Some(duration_to_next_slot) = self.slot_clock.duration_to_next_slot() {
                    // PTC attestations happen at 3/4 of the slot.
                    sleep(duration_to_next_slot + slot_duration * 3 / 4).await;

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
        self.executor
            .spawn_ignoring_error(service.produce_payload_attestations(), "payload_attestation");
    }

    /// Fetches PTC duties for the given epoch, using a cache to avoid redundant requests.
    async fn get_duties_for_epoch(&self, epoch: Epoch) -> Result<Vec<PtcDutyData>, String> {
        // Check cache first.
        {
            let cache = self.duties_cache.read().await;
            if let Some(duties) = cache.duties.get(&epoch) {
                return Ok(duties.clone());
            }
        }

        // Fetch from BN. Use `ignored` filter since we're collecting duties, not signing.
        let indices: Vec<u64> = self
            .validator_store
            .voting_pubkeys::<Vec<_>, _>(DoppelgangerStatus::ignored)
            .into_iter()
            .filter_map(|pubkey| self.validator_store.validator_index(&pubkey))
            .collect();

        if indices.is_empty() {
            return Ok(vec![]);
        }

        let duties_response = self
            .beacon_nodes
            .first_success(|beacon_node| {
                let indices = indices.clone();
                async move {
                    beacon_node
                        .post_validator_duties_ptc(epoch, &indices)
                        .await
                        .map_err(|e| e.to_string())
                }
            })
            .await
            .map_err(|e| format!("Failed to get PTC duties: {}", e))?;

        let duties = duties_response.data;

        // Update cache and prune old epochs.
        {
            let mut cache = self.duties_cache.write().await;
            cache.duties.insert(epoch, duties.clone());
            cache.duties.retain(|&e, _| e >= epoch.saturating_sub(1u64));
        }

        Ok(duties)
    }

    /// Main routine: fetch duties, get attestation data, sign, and submit.
    async fn produce_payload_attestations(self) -> Result<(), ()> {
        let slot = self.slot_clock.now().ok_or_else(|| {
            error!("Failed to read slot clock");
        })?;

        let epoch = slot.epoch(S::E::slots_per_epoch());

        let duties = self.get_duties_for_epoch(epoch).await.map_err(|e| {
            error!(error = %e, "Failed to get PTC duties");
        })?;

        // Filter duties for the current slot.
        let slot_duties: Vec<&PtcDutyData> = duties.iter().filter(|d| d.slot == slot).collect();

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
