use crate::TopicHash;
use crate::types::{GossipEncoding, GossipKind, GossipTopic};
use gossipsub::{IdentTopic as Topic, PeerScoreParams, PeerScoreThresholds, TopicScoreParams};
use std::cmp::max;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::time::Duration;
use types::{
    ChainSpec, EnrForkId, EthSpec, ExecutionProofSubnetId, Slot, SubnetId,
    execution_proof_subnet_id::MAX_EXECUTION_PROOF_SUBNETS,
};

const MAX_IN_MESH_SCORE: f64 = 10.0;
const MAX_FIRST_MESSAGE_DELIVERIES_SCORE: f64 = 40.0;
const BEACON_BLOCK_WEIGHT: f64 = 0.5;
const BEACON_AGGREGATE_PROOF_WEIGHT: f64 = 0.5;
const VOLUNTARY_EXIT_WEIGHT: f64 = 0.05;
const PROPOSER_SLASHING_WEIGHT: f64 = 0.05;
const ATTESTER_SLASHING_WEIGHT: f64 = 0.05;

// Gloas ePBS topic weights
const EXECUTION_BID_WEIGHT: f64 = 0.5;
const EXECUTION_PAYLOAD_WEIGHT: f64 = 0.5;
const PAYLOAD_ATTESTATION_WEIGHT: f64 = 0.4;
const EXECUTION_PROOF_WEIGHT: f64 = 0.3;

/// The time window (seconds) that we expect messages to be forwarded to us in the mesh.
const MESH_MESSAGE_DELIVERIES_WINDOW: u64 = 2;

// Const as this is used in the peer manager to prevent gossip from disconnecting peers.
pub const GREYLIST_THRESHOLD: f64 = -16000.0;

/// Builds the peer score thresholds.
pub fn vibehouse_gossip_thresholds() -> PeerScoreThresholds {
    PeerScoreThresholds {
        gossip_threshold: -4000.0,
        publish_threshold: -8000.0,
        graylist_threshold: GREYLIST_THRESHOLD,
        accept_px_threshold: 100.0,
        opportunistic_graft_threshold: 5.0,
    }
}

pub struct PeerScoreSettings<E: EthSpec> {
    slot: Duration,
    epoch: Duration,

    beacon_attestation_subnet_weight: f64,
    max_positive_score: f64,

    decay_interval: Duration,
    decay_to_zero: f64,

    mesh_n: usize,
    max_committees_per_slot: usize,
    target_committee_size: usize,
    target_aggregators_per_committee: usize,
    attestation_subnet_count: u64,
    ptc_size: u64,
    phantom: PhantomData<E>,
}

impl<E: EthSpec> PeerScoreSettings<E> {
    pub fn new(chain_spec: &ChainSpec, mesh_n: usize) -> PeerScoreSettings<E> {
        let slot = Duration::from_secs(chain_spec.seconds_per_slot);
        let beacon_attestation_subnet_weight = 1.0 / chain_spec.attestation_subnet_count as f64;
        let execution_proof_subnet_weight =
            EXECUTION_PROOF_WEIGHT / MAX_EXECUTION_PROOF_SUBNETS.max(1) as f64;
        let max_positive_score = (MAX_IN_MESH_SCORE + MAX_FIRST_MESSAGE_DELIVERIES_SCORE)
            * (BEACON_BLOCK_WEIGHT
                + BEACON_AGGREGATE_PROOF_WEIGHT
                + beacon_attestation_subnet_weight * chain_spec.attestation_subnet_count as f64
                + VOLUNTARY_EXIT_WEIGHT
                + PROPOSER_SLASHING_WEIGHT
                + ATTESTER_SLASHING_WEIGHT
                + EXECUTION_BID_WEIGHT
                + EXECUTION_PAYLOAD_WEIGHT
                + PAYLOAD_ATTESTATION_WEIGHT
                + execution_proof_subnet_weight * MAX_EXECUTION_PROOF_SUBNETS as f64);

        PeerScoreSettings {
            slot,
            epoch: slot * E::slots_per_epoch() as u32,
            beacon_attestation_subnet_weight,
            max_positive_score,
            decay_interval: max(Duration::from_secs(1), slot),
            decay_to_zero: 0.01,
            mesh_n,
            max_committees_per_slot: chain_spec.max_committees_per_slot,
            target_committee_size: chain_spec.target_committee_size,
            target_aggregators_per_committee: chain_spec.target_aggregators_per_committee as usize,
            attestation_subnet_count: chain_spec.attestation_subnet_count,
            ptc_size: chain_spec.ptc_size,
            phantom: PhantomData,
        }
    }

    pub fn get_peer_score_params(
        &self,
        active_validators: usize,
        thresholds: &PeerScoreThresholds,
        enr_fork_id: &EnrForkId,
        current_slot: Slot,
    ) -> Result<PeerScoreParams, String> {
        let mut params = PeerScoreParams {
            decay_interval: self.decay_interval,
            decay_to_zero: self.decay_to_zero,
            retain_score: self.epoch * 100,
            app_specific_weight: 1.0,
            ip_colocation_factor_threshold: 8.0, // Allow up to 8 nodes per IP
            behaviour_penalty_threshold: 6.0,
            behaviour_penalty_decay: self.score_parameter_decay(self.epoch * 10),
            slow_peer_decay: 0.1,
            slow_peer_weight: -10.0,
            slow_peer_threshold: 0.0,
            ..Default::default()
        };

        let target_value = Self::decay_convergence(
            params.behaviour_penalty_decay,
            10.0 / E::slots_per_epoch() as f64,
        ) - params.behaviour_penalty_threshold;
        params.behaviour_penalty_weight = thresholds.gossip_threshold / target_value.powi(2);

        params.topic_score_cap = self.max_positive_score * 0.5;
        params.ip_colocation_factor_weight = -params.topic_score_cap;

        params.topics = HashMap::new();

        let get_hash = |kind: GossipKind| -> TopicHash {
            let topic: Topic =
                GossipTopic::new(kind, GossipEncoding::default(), enr_fork_id.fork_digest).into();
            topic.hash()
        };

        //first all fixed topics
        params.topics.insert(
            get_hash(GossipKind::VoluntaryExit),
            Self::get_topic_params(
                self,
                VOLUNTARY_EXIT_WEIGHT,
                4.0 / E::slots_per_epoch() as f64,
                self.epoch * 100,
                None,
            ),
        );
        params.topics.insert(
            get_hash(GossipKind::AttesterSlashing),
            Self::get_topic_params(
                self,
                ATTESTER_SLASHING_WEIGHT,
                1.0 / 5.0 / E::slots_per_epoch() as f64,
                self.epoch * 100,
                None,
            ),
        );
        params.topics.insert(
            get_hash(GossipKind::ProposerSlashing),
            Self::get_topic_params(
                self,
                PROPOSER_SLASHING_WEIGHT,
                1.0 / 5.0 / E::slots_per_epoch() as f64,
                self.epoch * 100,
                None,
            ),
        );

        // Gloas ePBS topics
        //
        // ExecutionBid: 1 winning bid per slot, critical for block production
        params.topics.insert(
            get_hash(GossipKind::ExecutionBid),
            Self::get_topic_params(
                self,
                EXECUTION_BID_WEIGHT,
                1.0,
                self.epoch * 20,
                Some((E::slots_per_epoch() * 5, 3.0, self.epoch, current_slot)),
            ),
        );
        // ExecutionPayload: 1 payload reveal per slot from winning builder
        params.topics.insert(
            get_hash(GossipKind::ExecutionPayload),
            Self::get_topic_params(
                self,
                EXECUTION_PAYLOAD_WEIGHT,
                1.0,
                self.epoch * 20,
                Some((E::slots_per_epoch() * 5, 3.0, self.epoch, current_slot)),
            ),
        );
        // PayloadAttestation: ~ptc_size * 0.6 attestations per slot
        params.topics.insert(
            get_hash(GossipKind::PayloadAttestation),
            Self::get_topic_params(
                self,
                PAYLOAD_ATTESTATION_WEIGHT,
                self.ptc_size as f64 * 0.6,
                self.epoch * 4,
                Some((E::slots_per_epoch() * 2, 2.0, self.epoch / 2, current_slot)),
            ),
        );
        // ExecutionProof: 1 proof per subnet per slot, time-sensitive
        for i in 0..MAX_EXECUTION_PROOF_SUBNETS {
            if let Ok(subnet_id) = ExecutionProofSubnetId::new(i) {
                params.topics.insert(
                    get_hash(GossipKind::ExecutionProof(subnet_id)),
                    Self::get_topic_params(
                        self,
                        EXECUTION_PROOF_WEIGHT / MAX_EXECUTION_PROOF_SUBNETS.max(1) as f64,
                        1.0,
                        self.epoch * 20,
                        None,
                    ),
                );
            }
        }

        //dynamic topics
        let (beacon_block_params, beacon_aggregate_proof_params, beacon_attestation_subnet_params) =
            self.get_dynamic_topic_params(active_validators, current_slot)?;

        params
            .topics
            .insert(get_hash(GossipKind::BeaconBlock), beacon_block_params);

        params.topics.insert(
            get_hash(GossipKind::BeaconAggregateAndProof),
            beacon_aggregate_proof_params,
        );

        for i in 0..self.attestation_subnet_count {
            params.topics.insert(
                get_hash(GossipKind::Attestation(SubnetId::new(i))),
                beacon_attestation_subnet_params.clone(),
            );
        }

        Ok(params)
    }

    pub fn get_dynamic_topic_params(
        &self,
        active_validators: usize,
        current_slot: Slot,
    ) -> Result<(TopicScoreParams, TopicScoreParams, TopicScoreParams), String> {
        let (aggregators_per_slot, committees_per_slot) =
            self.expected_aggregator_count_per_slot(active_validators)?;
        let multiple_bursts_per_subnet_per_epoch =
            committees_per_slot as u64 >= 2 * self.attestation_subnet_count / E::slots_per_epoch();

        let beacon_block_params = Self::get_topic_params(
            self,
            BEACON_BLOCK_WEIGHT,
            1.0,
            self.epoch * 20,
            Some((E::slots_per_epoch() * 5, 3.0, self.epoch, current_slot)),
        );

        let beacon_aggregate_proof_params = Self::get_topic_params(
            self,
            BEACON_AGGREGATE_PROOF_WEIGHT,
            aggregators_per_slot,
            self.epoch,
            Some((E::slots_per_epoch() * 2, 4.0, self.epoch, current_slot)),
        );
        let beacon_attestation_subnet_params = Self::get_topic_params(
            self,
            self.beacon_attestation_subnet_weight,
            active_validators as f64
                / self.attestation_subnet_count as f64
                / E::slots_per_epoch() as f64,
            self.epoch
                * (if multiple_bursts_per_subnet_per_epoch {
                    1
                } else {
                    4
                }),
            Some((
                E::slots_per_epoch()
                    * (if multiple_bursts_per_subnet_per_epoch {
                        4
                    } else {
                        16
                    }),
                16.0,
                if multiple_bursts_per_subnet_per_epoch {
                    self.slot * (E::slots_per_epoch() as u32 / 2 + 1)
                } else {
                    self.epoch * 3
                },
                current_slot,
            )),
        );

        Ok((
            beacon_block_params,
            beacon_aggregate_proof_params,
            beacon_attestation_subnet_params,
        ))
    }

    pub fn attestation_subnet_count(&self) -> u64 {
        self.attestation_subnet_count
    }

    fn score_parameter_decay_with_base(
        decay_time: Duration,
        decay_interval: Duration,
        decay_to_zero: f64,
    ) -> f64 {
        let ticks = decay_time.as_secs_f64() / decay_interval.as_secs_f64();
        decay_to_zero.powf(1.0 / ticks)
    }

    fn decay_convergence(decay: f64, rate: f64) -> f64 {
        rate / (1.0 - decay)
    }

    fn threshold(decay: f64, rate: f64) -> f64 {
        Self::decay_convergence(decay, rate) * decay
    }

    fn expected_aggregator_count_per_slot(
        &self,
        active_validators: usize,
    ) -> Result<(f64, usize), String> {
        let committees_per_slot = E::get_committee_count_per_slot_with(
            active_validators,
            self.max_committees_per_slot,
            self.target_committee_size,
        )
        .map_err(|e| format!("Could not get committee count from spec: {:?}", e))?;

        let committees = committees_per_slot * E::slots_per_epoch() as usize;

        let smaller_committee_size = active_validators / committees;
        let num_larger_committees = active_validators - smaller_committee_size * committees;

        let modulo_smaller = max(
            1,
            smaller_committee_size / self.target_aggregators_per_committee,
        );
        let modulo_larger = max(
            1,
            (smaller_committee_size + 1) / self.target_aggregators_per_committee,
        );

        Ok((
            (((committees - num_larger_committees) * smaller_committee_size) as f64
                / modulo_smaller as f64
                + (num_larger_committees * (smaller_committee_size + 1)) as f64
                    / modulo_larger as f64)
                / E::slots_per_epoch() as f64,
            committees_per_slot,
        ))
    }

    fn score_parameter_decay(&self, decay_time: Duration) -> f64 {
        Self::score_parameter_decay_with_base(decay_time, self.decay_interval, self.decay_to_zero)
    }

    fn get_topic_params(
        &self,
        topic_weight: f64,
        expected_message_rate: f64,
        first_message_decay_time: Duration,
        // decay slots (decay time in slots), cap factor, activation window, current slot
        mesh_message_info: Option<(u64, f64, Duration, Slot)>,
    ) -> TopicScoreParams {
        let mut t_params = TopicScoreParams::default();

        t_params.topic_weight = topic_weight;

        t_params.time_in_mesh_quantum = self.slot;
        t_params.time_in_mesh_cap = 3600.0 / t_params.time_in_mesh_quantum.as_secs_f64();
        t_params.time_in_mesh_weight = 10.0 / t_params.time_in_mesh_cap;

        t_params.first_message_deliveries_decay =
            self.score_parameter_decay(first_message_decay_time);
        t_params.first_message_deliveries_cap = Self::decay_convergence(
            t_params.first_message_deliveries_decay,
            2.0 * expected_message_rate / self.mesh_n as f64,
        );
        t_params.first_message_deliveries_weight = 40.0 / t_params.first_message_deliveries_cap;

        if let Some((decay_slots, cap_factor, activation_window, current_slot)) = mesh_message_info
        {
            let decay_time = self.slot * decay_slots as u32;
            t_params.mesh_message_deliveries_decay = self.score_parameter_decay(decay_time);
            t_params.mesh_message_deliveries_threshold = Self::threshold(
                t_params.mesh_message_deliveries_decay,
                expected_message_rate / 50.0,
            );
            t_params.mesh_message_deliveries_cap =
                if cap_factor * t_params.mesh_message_deliveries_threshold < 2.0 {
                    2.0
                } else {
                    cap_factor * t_params.mesh_message_deliveries_threshold
                };
            t_params.mesh_message_deliveries_activation = activation_window;
            t_params.mesh_message_deliveries_window =
                Duration::from_secs(MESH_MESSAGE_DELIVERIES_WINDOW);
            t_params.mesh_failure_penalty_decay = t_params.mesh_message_deliveries_decay;
            t_params.mesh_message_deliveries_weight = -t_params.topic_weight;
            t_params.mesh_failure_penalty_weight = t_params.mesh_message_deliveries_weight;
            if decay_slots >= current_slot.as_u64() {
                t_params.mesh_message_deliveries_threshold = 0.0;
                t_params.mesh_message_deliveries_weight = 0.0;
            }
        } else {
            t_params.mesh_message_deliveries_weight = 0.0;
            t_params.mesh_message_deliveries_threshold = 0.0;
            t_params.mesh_message_deliveries_decay = 0.0;
            t_params.mesh_message_deliveries_cap = 0.0;
            t_params.mesh_message_deliveries_window = Duration::from_secs(0);
            t_params.mesh_message_deliveries_activation = Duration::from_secs(0);
            t_params.mesh_failure_penalty_decay = 0.0;
            t_params.mesh_failure_penalty_weight = 0.0;
        }

        t_params.invalid_message_deliveries_weight =
            -self.max_positive_score / t_params.topic_weight;
        t_params.invalid_message_deliveries_decay = self.score_parameter_decay(self.epoch * 50);

        t_params
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::MinimalEthSpec;

    type E = MinimalEthSpec;

    fn default_settings() -> PeerScoreSettings<E> {
        let spec = E::default_spec();
        PeerScoreSettings::new(&spec, 6)
    }

    // --- vibehouse_gossip_thresholds ---

    #[test]
    fn gossip_thresholds_values() {
        let t = vibehouse_gossip_thresholds();
        assert_eq!(t.gossip_threshold, -4000.0);
        assert_eq!(t.publish_threshold, -8000.0);
        assert_eq!(t.graylist_threshold, GREYLIST_THRESHOLD);
        assert_eq!(t.accept_px_threshold, 100.0);
        assert_eq!(t.opportunistic_graft_threshold, 5.0);
    }

    #[test]
    fn gossip_threshold_ordering() {
        let t = vibehouse_gossip_thresholds();
        // publish should be stricter (more negative) than gossip
        assert!(t.publish_threshold < t.gossip_threshold);
        // graylist should be stricter than publish
        assert!(t.graylist_threshold < t.publish_threshold);
    }

    // --- PeerScoreSettings::new ---

    #[test]
    fn settings_new_slot_duration() {
        let spec = E::default_spec();
        let settings = PeerScoreSettings::<E>::new(&spec, 6);
        assert_eq!(settings.slot, Duration::from_secs(spec.seconds_per_slot));
    }

    #[test]
    fn settings_new_epoch_duration() {
        let settings = default_settings();
        let expected = settings.slot * E::slots_per_epoch() as u32;
        assert_eq!(settings.epoch, expected);
    }

    #[test]
    fn settings_new_mesh_n() {
        let spec = E::default_spec();
        let settings = PeerScoreSettings::<E>::new(&spec, 12);
        assert_eq!(settings.mesh_n, 12);
    }

    #[test]
    fn settings_new_attestation_subnet_count() {
        let spec = E::default_spec();
        let settings = PeerScoreSettings::<E>::new(&spec, 6);
        assert_eq!(
            settings.attestation_subnet_count,
            spec.attestation_subnet_count
        );
    }

    #[test]
    fn settings_new_max_positive_score_positive() {
        let settings = default_settings();
        assert!(settings.max_positive_score > 0.0);
    }

    #[test]
    fn settings_attestation_subnet_count_accessor() {
        let settings = default_settings();
        let spec = E::default_spec();
        assert_eq!(
            settings.attestation_subnet_count(),
            spec.attestation_subnet_count
        );
    }

    // --- score_parameter_decay_with_base ---

    #[test]
    fn decay_with_base_single_tick() {
        // When decay_time == decay_interval, ticks = 1, result = decay_to_zero^1
        let result = PeerScoreSettings::<E>::score_parameter_decay_with_base(
            Duration::from_secs(10),
            Duration::from_secs(10),
            0.01,
        );
        assert!((result - 0.01).abs() < 1e-10);
    }

    #[test]
    fn decay_with_base_two_ticks() {
        // When decay_time = 2 * decay_interval, ticks = 2, result = 0.01^0.5 = 0.1
        let result = PeerScoreSettings::<E>::score_parameter_decay_with_base(
            Duration::from_secs(20),
            Duration::from_secs(10),
            0.01,
        );
        assert!((result - 0.1).abs() < 1e-10);
    }

    #[test]
    fn decay_with_base_result_in_range() {
        // Result should be between decay_to_zero and 1.0
        let result = PeerScoreSettings::<E>::score_parameter_decay_with_base(
            Duration::from_secs(100),
            Duration::from_secs(12),
            0.01,
        );
        assert!(result > 0.01);
        assert!(result < 1.0);
    }

    #[test]
    fn decay_with_base_longer_time_lower_decay() {
        // Longer decay time should produce a higher per-tick decay (closer to 1)
        let short = PeerScoreSettings::<E>::score_parameter_decay_with_base(
            Duration::from_secs(10),
            Duration::from_secs(1),
            0.01,
        );
        let long = PeerScoreSettings::<E>::score_parameter_decay_with_base(
            Duration::from_secs(100),
            Duration::from_secs(1),
            0.01,
        );
        assert!(long > short);
    }

    // --- decay_convergence ---

    #[test]
    fn decay_convergence_known_value() {
        // decay=0.5, rate=1.0 => 1.0 / (1.0 - 0.5) = 2.0
        let result = PeerScoreSettings::<E>::decay_convergence(0.5, 1.0);
        assert!((result - 2.0).abs() < 1e-10);
    }

    #[test]
    fn decay_convergence_high_decay() {
        // Higher decay -> higher convergence value
        let low = PeerScoreSettings::<E>::decay_convergence(0.5, 1.0);
        let high = PeerScoreSettings::<E>::decay_convergence(0.9, 1.0);
        assert!(high > low);
    }

    #[test]
    fn decay_convergence_scales_with_rate() {
        // Double rate -> double convergence
        let single = PeerScoreSettings::<E>::decay_convergence(0.5, 1.0);
        let double = PeerScoreSettings::<E>::decay_convergence(0.5, 2.0);
        assert!((double - 2.0 * single).abs() < 1e-10);
    }

    // --- threshold ---

    #[test]
    fn threshold_known_value() {
        // threshold(0.5, 1.0) = decay_convergence(0.5, 1.0) * 0.5 = 2.0 * 0.5 = 1.0
        let result = PeerScoreSettings::<E>::threshold(0.5, 1.0);
        assert!((result - 1.0).abs() < 1e-10);
    }

    #[test]
    fn threshold_less_than_convergence() {
        // threshold should always be less than convergence (since decay < 1)
        let decay = 0.8;
        let rate = 5.0;
        let conv = PeerScoreSettings::<E>::decay_convergence(decay, rate);
        let thresh = PeerScoreSettings::<E>::threshold(decay, rate);
        assert!(thresh < conv);
    }

    // --- expected_aggregator_count_per_slot ---

    #[test]
    fn expected_aggregators_positive() {
        let settings = default_settings();
        let (agg_per_slot, committees) = settings.expected_aggregator_count_per_slot(1024).unwrap();
        assert!(agg_per_slot > 0.0);
        assert!(committees > 0);
    }

    #[test]
    fn expected_aggregators_different_validator_counts() {
        let settings = default_settings();
        let (agg_1024, committees_1024) =
            settings.expected_aggregator_count_per_slot(1024).unwrap();
        let (agg_65536, committees_65536) =
            settings.expected_aggregator_count_per_slot(65536).unwrap();
        // Both should be positive
        assert!(agg_1024 > 0.0);
        assert!(agg_65536 > 0.0);
        // More validators should produce at least as many committees
        assert!(committees_65536 >= committees_1024);
    }

    // --- score_parameter_decay ---

    #[test]
    fn score_parameter_decay_uses_settings() {
        let settings = default_settings();
        let decay = settings.score_parameter_decay(settings.epoch);
        assert!(decay > 0.0);
        assert!(decay < 1.0);
    }

    // --- get_topic_params ---

    #[test]
    fn topic_params_without_mesh_info() {
        let settings = default_settings();
        let params = settings.get_topic_params(0.5, 1.0, settings.epoch, None);

        assert_eq!(params.topic_weight, 0.5);
        assert!(params.time_in_mesh_weight > 0.0);
        assert!(params.first_message_deliveries_weight > 0.0);
        assert!(params.first_message_deliveries_cap > 0.0);
        // mesh delivery params should be zero without mesh info
        assert_eq!(params.mesh_message_deliveries_weight, 0.0);
        assert_eq!(params.mesh_message_deliveries_threshold, 0.0);
        assert_eq!(params.mesh_message_deliveries_cap, 0.0);
        assert_eq!(params.mesh_failure_penalty_weight, 0.0);
    }

    #[test]
    fn topic_params_with_mesh_info() {
        let settings = default_settings();
        let current_slot = Slot::new(100);
        let params = settings.get_topic_params(
            0.5,
            1.0,
            settings.epoch,
            Some((E::slots_per_epoch() * 5, 3.0, settings.epoch, current_slot)),
        );

        assert_eq!(params.topic_weight, 0.5);
        // With mesh info and slot > decay_slots, mesh params should be active
        assert!(params.mesh_message_deliveries_decay > 0.0);
        assert!(params.mesh_message_deliveries_cap > 0.0);
    }

    #[test]
    fn topic_params_mesh_disabled_early_slot() {
        let settings = default_settings();
        // When current_slot < decay_slots, mesh delivery scoring is disabled
        let decay_slots = E::slots_per_epoch() * 5;
        let current_slot = Slot::new(0);
        let params = settings.get_topic_params(
            0.5,
            1.0,
            settings.epoch,
            Some((decay_slots, 3.0, settings.epoch, current_slot)),
        );
        // Mesh delivery threshold and weight should be zeroed out
        assert_eq!(params.mesh_message_deliveries_threshold, 0.0);
        assert_eq!(params.mesh_message_deliveries_weight, 0.0);
    }

    #[test]
    fn topic_params_invalid_message_weight_negative() {
        let settings = default_settings();
        let params = settings.get_topic_params(0.5, 1.0, settings.epoch, None);
        // Invalid message deliveries weight should be negative (penalizing)
        assert!(params.invalid_message_deliveries_weight < 0.0);
    }

    #[test]
    fn topic_params_time_in_mesh_cap_based_on_slot() {
        let settings = default_settings();
        let params = settings.get_topic_params(0.5, 1.0, settings.epoch, None);
        // time_in_mesh_cap should be 3600 / slot_seconds
        let expected_cap = 3600.0 / settings.slot.as_secs_f64();
        assert!((params.time_in_mesh_cap - expected_cap).abs() < 1e-10);
    }

    // --- get_dynamic_topic_params ---

    #[test]
    fn dynamic_topic_params_returns_three_params() {
        let settings = default_settings();
        let current_slot = Slot::new(100);
        let (block, agg, subnet) = settings
            .get_dynamic_topic_params(1024, current_slot)
            .unwrap();
        assert_eq!(block.topic_weight, BEACON_BLOCK_WEIGHT);
        assert_eq!(agg.topic_weight, BEACON_AGGREGATE_PROOF_WEIGHT);
        assert!(subnet.topic_weight > 0.0);
    }

    // --- get_peer_score_params ---

    #[test]
    fn peer_score_params_has_all_topics() {
        let settings = default_settings();
        let thresholds = vibehouse_gossip_thresholds();
        let enr_fork_id = EnrForkId::default();
        let current_slot = Slot::new(100);

        let params = settings
            .get_peer_score_params(1024, &thresholds, &enr_fork_id, current_slot)
            .unwrap();

        // Should have topics for: beacon_block, aggregate, voluntary_exit, attester_slashing,
        // proposer_slashing, execution_bid, execution_payload, payload_attestation,
        // execution_proof subnets, and attestation subnets
        let spec = E::default_spec();
        let min_topics = 3 // fixed (exit, attester, proposer slashing)
            + 3 // gloas (bid, payload, payload_attest)
            + 1 // beacon_block
            + 1 // aggregate
            + spec.attestation_subnet_count as usize; // attestation subnets
        assert!(params.topics.len() >= min_topics);
    }

    #[test]
    fn peer_score_params_decay_interval() {
        let settings = default_settings();
        let thresholds = vibehouse_gossip_thresholds();
        let enr_fork_id = EnrForkId::default();
        let params = settings
            .get_peer_score_params(1024, &thresholds, &enr_fork_id, Slot::new(100))
            .unwrap();
        assert_eq!(params.decay_interval, settings.decay_interval);
    }

    #[test]
    fn peer_score_params_topic_score_cap_positive() {
        let settings = default_settings();
        let thresholds = vibehouse_gossip_thresholds();
        let enr_fork_id = EnrForkId::default();
        let params = settings
            .get_peer_score_params(1024, &thresholds, &enr_fork_id, Slot::new(100))
            .unwrap();
        assert!(params.topic_score_cap > 0.0);
    }

    #[test]
    fn peer_score_params_ip_colocation_negative() {
        let settings = default_settings();
        let thresholds = vibehouse_gossip_thresholds();
        let enr_fork_id = EnrForkId::default();
        let params = settings
            .get_peer_score_params(1024, &thresholds, &enr_fork_id, Slot::new(100))
            .unwrap();
        // IP colocation weight should be negative (penalizing)
        assert!(params.ip_colocation_factor_weight < 0.0);
    }

    #[test]
    fn peer_score_params_behaviour_penalty_weight_negative() {
        let settings = default_settings();
        let thresholds = vibehouse_gossip_thresholds();
        let enr_fork_id = EnrForkId::default();
        let params = settings
            .get_peer_score_params(1024, &thresholds, &enr_fork_id, Slot::new(100))
            .unwrap();
        assert!(params.behaviour_penalty_weight < 0.0);
    }
}
