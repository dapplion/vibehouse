use crate::duties_service::DutiesService;
use beacon_node_fallback::{ApiTopic, BeaconNodeFallback};
use slot_clock::SlotClock;
use std::ops::Deref;
use std::sync::Arc;
use task_executor::TaskExecutor;
use tokio::time::{Duration, sleep};
use tracing::{debug, error, info, trace};
use types::{ChainSpec, Epoch, EthSpec, InclusionList};
use validator_metrics::{self as metrics};
use validator_store::ValidatorStore;

/// Builds an `InclusionListService`.
pub struct InclusionListServiceBuilder<S: ValidatorStore, T: SlotClock + 'static> {
    duties_service: Option<Arc<DutiesService<S, T>>>,
    validator_store: Option<Arc<S>>,
    slot_clock: Option<T>,
    beacon_nodes: Option<Arc<BeaconNodeFallback<T>>>,
    executor: Option<TaskExecutor>,
    chain_spec: Option<Arc<ChainSpec>>,
}

impl<S: ValidatorStore + 'static, T: SlotClock + 'static> Default
    for InclusionListServiceBuilder<S, T>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<S: ValidatorStore + 'static, T: SlotClock + 'static> InclusionListServiceBuilder<S, T> {
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

    pub fn build(self) -> Result<InclusionListService<S, T>, String> {
        let chain_spec = self
            .chain_spec
            .ok_or("Cannot build InclusionListService without chain_spec")?;
        Ok(InclusionListService {
            inner: Arc::new(Inner {
                duties_service: self
                    .duties_service
                    .ok_or("Cannot build InclusionListService without duties_service")?,
                validator_store: self
                    .validator_store
                    .ok_or("Cannot build InclusionListService without validator_store")?,
                slot_clock: self
                    .slot_clock
                    .ok_or("Cannot build InclusionListService without slot_clock")?,
                beacon_nodes: self
                    .beacon_nodes
                    .ok_or("Cannot build InclusionListService without beacon_nodes")?,
                executor: self
                    .executor
                    .ok_or("Cannot build InclusionListService without executor")?,
                heze_fork_epoch: chain_spec.heze_fork_epoch,
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
    heze_fork_epoch: Option<Epoch>,
    chain_spec: Arc<ChainSpec>,
}

/// Produces and broadcasts inclusion lists for FOCIL committee duties.
///
/// IL committee members construct and sign inclusion lists at ~67% of each slot
/// (before the 75% view freeze cutoff). Duties are fetched proactively by the
/// `DutiesService` IL polling task and read from `DutiesService.inclusion_list_duties`.
pub struct InclusionListService<S, T> {
    inner: Arc<Inner<S, T>>,
}

impl<S, T> Clone for InclusionListService<S, T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<S, T> Deref for InclusionListService<S, T> {
    type Target = Inner<S, T>;

    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}

/// IL broadcast target: 6667 BPS (66.67% of slot), before the 75% view freeze cutoff.
const INCLUSION_LIST_DUE_BPS: u64 = 6667;

impl<S: ValidatorStore + 'static, T: SlotClock + 'static> InclusionListService<S, T> {
    /// Exposed for integration tests.
    #[cfg(test)]
    pub async fn produce_inclusion_lists_for_testing(self) -> Result<(), ()> {
        self.produce_inclusion_lists().await
    }

    fn heze_fork_activated(&self) -> bool {
        self.heze_fork_epoch
            .and_then(|fork_epoch| {
                let current_epoch = self.slot_clock.now()?.epoch(S::E::slots_per_epoch());
                Some(current_epoch >= fork_epoch)
            })
            .unwrap_or(false)
    }

    /// Starts the service which periodically produces inclusion lists at ~67% of each slot.
    pub fn start_update_service(self, _spec: &ChainSpec) -> Result<(), String> {
        if !self.chain_spec.is_heze_scheduled() {
            info!("Inclusion list service disabled (Heze not scheduled)");
            return Ok(());
        }

        let slot_duration = Duration::from_secs(self.chain_spec.seconds_per_slot);
        let il_delay = Duration::from_millis(
            self.chain_spec.seconds_per_slot * 1000 * INCLUSION_LIST_DUE_BPS / 10000,
        );
        let duration_to_next_slot = self
            .slot_clock
            .duration_to_next_slot()
            .ok_or("Unable to determine duration to next slot")?;

        info!(
            next_update_millis = duration_to_next_slot.as_millis(),
            il_delay_ms = il_delay.as_millis(),
            "Inclusion list service started"
        );

        let executor = self.executor.clone();

        let interval_fut = async move {
            loop {
                if let Some(duration_to_next_slot) = self.slot_clock.duration_to_next_slot() {
                    sleep(duration_to_next_slot + il_delay).await;

                    if !self.heze_fork_activated() {
                        continue;
                    }

                    self.spawn_inclusion_list_tasks();
                } else {
                    error!("Failed to read slot clock");
                    sleep(slot_duration).await;
                }
            }
        };

        executor.spawn(interval_fut, "inclusion_list_service");
        Ok(())
    }

    fn spawn_inclusion_list_tasks(&self) {
        let service = self.clone();
        self.executor
            .spawn_ignoring_error(service.produce_inclusion_lists(), "inclusion_list");
    }

    /// Main routine: read duties from DutiesService, construct ILs, sign, and submit.
    async fn produce_inclusion_lists(self) -> Result<(), ()> {
        let _timer = metrics::start_timer_vec(
            &metrics::INCLUSION_LIST_SERVICE_TIMES,
            &[metrics::INCLUSION_LISTS],
        );

        let slot = self.slot_clock.now().ok_or_else(|| {
            error!("Failed to read slot clock");
        })?;

        let slot_duties = self
            .duties_service
            .inclusion_list_duties
            .duties_for_slot(slot, S::E::slots_per_epoch());

        if slot_duties.is_empty() {
            trace!(slot = slot.as_u64(), "No IL committee duties for this slot");
            return Ok(());
        }

        debug!(
            slot = slot.as_u64(),
            num_duties = slot_duties.len(),
            "Producing inclusion lists"
        );

        // Sign and submit inclusion lists for each duty.
        let mut submitted = 0u64;

        for duty in &slot_duties {
            // Construct the inclusion list.
            // Transactions are empty for now — EL integration (engine_getInclusionList)
            // will populate them in a future phase.
            let inclusion_list = InclusionList {
                slot,
                validator_index: duty.validator_index,
                inclusion_list_committee_root: duty.inclusion_list_committee_root,
                transactions: <_>::default(),
            };

            let signed_il = match self
                .validator_store
                .sign_inclusion_list(duty.pubkey, &inclusion_list)
                .await
            {
                Ok(signed) => {
                    metrics::inc_counter_vec(
                        &metrics::SIGNED_INCLUSION_LISTS_TOTAL,
                        &[metrics::SUCCESS],
                    );
                    signed
                }
                Err(e) => {
                    metrics::inc_counter_vec(
                        &metrics::SIGNED_INCLUSION_LISTS_TOTAL,
                        &[metrics::ERROR],
                    );
                    error!(
                        error = format!("{:?}", e),
                        validator_index = duty.validator_index,
                        slot = slot.as_u64(),
                        "Failed to sign inclusion list"
                    );
                    continue;
                }
            };

            // Submit to BN for gossip propagation.
            let submit_result = {
                let _http_timer = metrics::start_timer_vec(
                    &metrics::INCLUSION_LIST_SERVICE_TIMES,
                    &[metrics::INCLUSION_LISTS_HTTP_POST],
                );
                self.beacon_nodes
                    .request(ApiTopic::Attestations, |beacon_node| {
                        let signed_il = signed_il.clone();
                        async move {
                            beacon_node
                                .post_beacon_pool_inclusion_lists(&signed_il)
                                .await
                                .map_err(|e| e.to_string())
                        }
                    })
                    .await
            };
            match submit_result {
                Ok(()) => {
                    debug!(
                        validator_index = duty.validator_index,
                        slot = slot.as_u64(),
                        "Published inclusion list"
                    );
                    submitted += 1;
                }
                Err(e) => {
                    error!(
                        error = %e,
                        validator_index = duty.validator_index,
                        slot = slot.as_u64(),
                        "Failed to publish inclusion list"
                    );
                }
            }
        }

        if submitted > 0 {
            info!(
                slot = slot.as_u64(),
                count = submitted,
                "Published inclusion lists"
            );
        }

        Ok(())
    }
}
