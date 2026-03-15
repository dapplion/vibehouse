use crate::common::{
    base::{SqrtTotalActiveBalance, get_base_reward},
    decrease_balance, increase_balance,
};
use crate::per_epoch_processing::{
    Delta, Error,
    base::{TotalBalances, ValidatorStatus, ValidatorStatuses},
};
use safe_arith::SafeArith;
use std::collections::HashSet;
use types::{BeaconState, ChainSpec, EthSpec};

/// Combination of several deltas for different components of an attestation reward.
///
/// Exists only for compatibility with EF rewards tests.
#[derive(Default, Clone)]
pub struct AttestationDelta {
    pub source_delta: Delta,
    pub target_delta: Delta,
    pub head_delta: Delta,
    pub inclusion_delay_delta: Delta,
    pub inactivity_penalty_delta: Delta,
}

impl AttestationDelta {
    /// Flatten into a single delta.
    pub fn flatten(self) -> Result<Delta, Error> {
        let AttestationDelta {
            source_delta,
            target_delta,
            head_delta,
            inclusion_delay_delta,
            inactivity_penalty_delta,
        } = self;
        let mut result = Delta::default();
        for delta in [
            source_delta,
            target_delta,
            head_delta,
            inclusion_delay_delta,
            inactivity_penalty_delta,
        ] {
            result.combine(delta)?;
        }
        Ok(result)
    }
}

#[derive(Debug)]
pub enum ProposerRewardCalculation {
    Include,
    Exclude,
}

/// Apply attester and proposer rewards.
pub fn process_rewards_and_penalties<E: EthSpec>(
    state: &mut BeaconState<E>,
    validator_statuses: &ValidatorStatuses,
    spec: &ChainSpec,
) -> Result<(), Error> {
    if state.current_epoch() == E::genesis_epoch() {
        return Ok(());
    }

    // Guard against an out-of-bounds during the validator balance update.
    if validator_statuses.statuses.len() != state.balances().len()
        || validator_statuses.statuses.len() != state.validators().len()
    {
        return Err(Error::ValidatorStatusesInconsistent);
    }

    let deltas = get_attestation_deltas_all(
        state,
        validator_statuses,
        ProposerRewardCalculation::Include,
        spec,
    )?;

    // Apply the deltas, erroring on overflow above but not on overflow below (saturating at 0
    // instead).
    for (i, delta) in deltas.into_iter().enumerate() {
        let combined_delta = delta.flatten()?;
        increase_balance(state, i, combined_delta.rewards)?;
        decrease_balance(state, i, combined_delta.penalties)?;
    }

    Ok(())
}

/// Apply rewards for participation in attestations during the previous epoch.
pub fn get_attestation_deltas_all<E: EthSpec>(
    state: &BeaconState<E>,
    validator_statuses: &ValidatorStatuses,
    proposer_reward: ProposerRewardCalculation,
    spec: &ChainSpec,
) -> Result<Vec<AttestationDelta>, Error> {
    get_attestation_deltas(state, validator_statuses, proposer_reward, None, spec)
}

/// Apply rewards for participation in attestations during the previous epoch, and only compute
/// rewards for a subset of validators.
pub fn get_attestation_deltas_subset<E: EthSpec>(
    state: &BeaconState<E>,
    validator_statuses: &ValidatorStatuses,
    proposer_reward: ProposerRewardCalculation,
    validators_subset: &[usize],
    spec: &ChainSpec,
) -> Result<Vec<(usize, AttestationDelta)>, Error> {
    let subset_set: HashSet<usize> = validators_subset.iter().copied().collect();
    get_attestation_deltas(
        state,
        validator_statuses,
        proposer_reward,
        Some(&subset_set),
        spec,
    )
    .map(|deltas| {
        deltas
            .into_iter()
            .enumerate()
            .filter(|(index, _)| subset_set.contains(index))
            .collect()
    })
}

/// Apply rewards for participation in attestations during the previous epoch.
/// If `maybe_validators_subset` specified, only the deltas for the specified validator subset is
/// returned, otherwise deltas for all validators are returned.
///
/// Returns a vec of validator indices to `AttestationDelta`.
fn get_attestation_deltas<E: EthSpec>(
    state: &BeaconState<E>,
    validator_statuses: &ValidatorStatuses,
    proposer_reward: ProposerRewardCalculation,
    maybe_validators_subset: Option<&HashSet<usize>>,
    spec: &ChainSpec,
) -> Result<Vec<AttestationDelta>, Error> {
    let finality_delay = state
        .previous_epoch()
        .safe_sub(state.finalized_checkpoint().epoch)?
        .as_u64();

    let mut deltas = vec![AttestationDelta::default(); state.validators().len()];

    let total_balances = &validator_statuses.total_balances;
    let sqrt_total_active_balance = SqrtTotalActiveBalance::new(total_balances.current_epoch());

    // Ignore validator if a subset is specified and validator is not in the subset
    let include_validator_delta = |idx| match maybe_validators_subset.as_ref() {
        None => true,
        Some(validators_subset) if validators_subset.contains(&idx) => true,
        Some(_) => false,
    };

    for (index, validator) in validator_statuses.statuses.iter().enumerate() {
        // Ignore ineligible validators. All sub-functions of the spec do this except for
        // `get_inclusion_delay_deltas`. It's safe to do so here because any validator that is in
        // the unslashed indices of the matching source attestations is active, and therefore
        // eligible.
        if !validator.is_eligible {
            continue;
        }

        let base_reward = get_base_reward(
            validator.current_epoch_effective_balance,
            sqrt_total_active_balance,
            spec,
        )?;

        let (inclusion_delay_delta, proposer_delta) =
            get_inclusion_delay_delta(validator, base_reward, spec)?;

        if include_validator_delta(index) {
            let source_delta =
                get_source_delta(validator, base_reward, total_balances, finality_delay, spec)?;
            let target_delta =
                get_target_delta(validator, base_reward, total_balances, finality_delay, spec)?;
            let head_delta =
                get_head_delta(validator, base_reward, total_balances, finality_delay, spec)?;
            let inactivity_penalty_delta =
                get_inactivity_penalty_delta(validator, base_reward, finality_delay, spec)?;

            let delta = deltas
                .get_mut(index)
                .ok_or(Error::DeltaOutOfBounds(index))?;
            delta.source_delta.combine(source_delta)?;
            delta.target_delta.combine(target_delta)?;
            delta.head_delta.combine(head_delta)?;
            delta.inclusion_delay_delta.combine(inclusion_delay_delta)?;
            delta
                .inactivity_penalty_delta
                .combine(inactivity_penalty_delta)?;
        }

        if let ProposerRewardCalculation::Include = proposer_reward
            && let Some((proposer_index, proposer_delta)) = proposer_delta
            && include_validator_delta(proposer_index)
        {
            deltas
                .get_mut(proposer_index)
                .ok_or(Error::ValidatorStatusesInconsistent)?
                .inclusion_delay_delta
                .combine(proposer_delta)?;
        }
    }

    Ok(deltas)
}

pub fn get_attestation_component_delta(
    index_in_unslashed_attesting_indices: bool,
    attesting_balance: u64,
    total_balances: &TotalBalances,
    base_reward: u64,
    finality_delay: u64,
    spec: &ChainSpec,
) -> Result<Delta, Error> {
    let mut delta = Delta::default();

    let total_balance = total_balances.current_epoch();

    if index_in_unslashed_attesting_indices {
        if finality_delay > spec.min_epochs_to_inactivity_penalty {
            // Since full base reward will be canceled out by inactivity penalty deltas,
            // optimal participation receives full base reward compensation here.
            delta.reward(base_reward)?;
        } else {
            let reward_numerator = base_reward
                .safe_mul(attesting_balance.safe_div(spec.effective_balance_increment)?)?;
            delta.reward(
                reward_numerator
                    .safe_div(total_balance.safe_div(spec.effective_balance_increment)?)?,
            )?;
        }
    } else {
        delta.penalize(base_reward)?;
    }

    Ok(delta)
}

fn get_source_delta(
    validator: &ValidatorStatus,
    base_reward: u64,
    total_balances: &TotalBalances,
    finality_delay: u64,
    spec: &ChainSpec,
) -> Result<Delta, Error> {
    get_attestation_component_delta(
        validator.is_previous_epoch_attester && !validator.is_slashed,
        total_balances.previous_epoch_attesters(),
        total_balances,
        base_reward,
        finality_delay,
        spec,
    )
}

fn get_target_delta(
    validator: &ValidatorStatus,
    base_reward: u64,
    total_balances: &TotalBalances,
    finality_delay: u64,
    spec: &ChainSpec,
) -> Result<Delta, Error> {
    get_attestation_component_delta(
        validator.is_previous_epoch_target_attester && !validator.is_slashed,
        total_balances.previous_epoch_target_attesters(),
        total_balances,
        base_reward,
        finality_delay,
        spec,
    )
}

fn get_head_delta(
    validator: &ValidatorStatus,
    base_reward: u64,
    total_balances: &TotalBalances,
    finality_delay: u64,
    spec: &ChainSpec,
) -> Result<Delta, Error> {
    get_attestation_component_delta(
        validator.is_previous_epoch_head_attester && !validator.is_slashed,
        total_balances.previous_epoch_head_attesters(),
        total_balances,
        base_reward,
        finality_delay,
        spec,
    )
}

pub fn get_inclusion_delay_delta(
    validator: &ValidatorStatus,
    base_reward: u64,
    spec: &ChainSpec,
) -> Result<(Delta, Option<(usize, Delta)>), Error> {
    // Spec: `index in get_unslashed_attesting_indices(state, matching_source_attestations)`
    if validator.is_previous_epoch_attester && !validator.is_slashed {
        let mut delta = Delta::default();
        let mut proposer_delta = Delta::default();

        let inclusion_info = validator
            .inclusion_info
            .ok_or(Error::ValidatorStatusesInconsistent)?;

        let proposer_reward = get_proposer_reward(base_reward, spec)?;
        proposer_delta.reward(proposer_reward)?;
        let max_attester_reward = base_reward.safe_sub(proposer_reward)?;
        delta.reward(max_attester_reward.safe_div(inclusion_info.delay)?)?;

        let proposer_index = inclusion_info.proposer_index;
        Ok((delta, Some((proposer_index, proposer_delta))))
    } else {
        Ok((Delta::default(), None))
    }
}

pub fn get_inactivity_penalty_delta(
    validator: &ValidatorStatus,
    base_reward: u64,
    finality_delay: u64,
    spec: &ChainSpec,
) -> Result<Delta, Error> {
    let mut delta = Delta::default();

    // Inactivity penalty
    if finality_delay > spec.min_epochs_to_inactivity_penalty {
        // If validator is performing optimally this cancels all rewards for a neutral balance
        delta.penalize(
            spec.base_rewards_per_epoch
                .safe_mul(base_reward)?
                .safe_sub(get_proposer_reward(base_reward, spec)?)?,
        )?;

        // Additionally, all validators whose FFG target didn't match are penalized extra
        // This condition is equivalent to this condition from the spec:
        // `index not in get_unslashed_attesting_indices(state, matching_target_attestations)`
        if validator.is_slashed || !validator.is_previous_epoch_target_attester {
            delta.penalize(
                validator
                    .current_epoch_effective_balance
                    .safe_mul(finality_delay)?
                    .safe_div(spec.inactivity_penalty_quotient)?,
            )?;
        }
    }

    Ok(delta)
}

/// Compute the reward awarded to a proposer for including an attestation from a validator.
///
/// The `base_reward` param should be the `base_reward` of the attesting validator.
fn get_proposer_reward(base_reward: u64, spec: &ChainSpec) -> Result<u64, Error> {
    Ok(base_reward.safe_div(spec.proposer_reward_quotient)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::per_epoch_processing::base::validator_statuses::InclusionInfo;

    fn mainnet_spec() -> ChainSpec {
        ChainSpec::mainnet()
    }

    fn make_total_balances(current_epoch: u64, attesting: u64) -> TotalBalances {
        let spec = mainnet_spec();
        TotalBalances::new_for_testing(
            &spec,
            current_epoch,
            current_epoch,
            attesting,
            attesting,
            attesting,
            attesting,
            attesting,
        )
    }

    // --- AttestationDelta::flatten ---

    #[test]
    fn attestation_delta_flatten_default_is_zero() {
        let delta = AttestationDelta::default().flatten().unwrap();
        assert_eq!(delta.rewards, 0);
        assert_eq!(delta.penalties, 0);
    }

    #[test]
    fn attestation_delta_flatten_sums_rewards_and_penalties() {
        let mut ad = AttestationDelta::default();
        ad.source_delta.reward(100).unwrap();
        ad.target_delta.reward(200).unwrap();
        ad.head_delta.penalize(50).unwrap();
        ad.inactivity_penalty_delta.penalize(30).unwrap();
        let delta = ad.flatten().unwrap();
        assert_eq!(delta.rewards, 300);
        assert_eq!(delta.penalties, 80);
    }

    // --- get_attestation_component_delta ---

    #[test]
    fn component_delta_attesting_gets_reward() {
        let spec = mainnet_spec();
        let total = make_total_balances(32_000_000_000, 24_000_000_000);
        let base_reward = 1000;
        let finality_delay = 1; // below inactivity threshold

        let delta = get_attestation_component_delta(
            true,
            24_000_000_000,
            &total,
            base_reward,
            finality_delay,
            &spec,
        )
        .unwrap();
        assert!(delta.rewards > 0, "attesting validator should be rewarded");
        assert_eq!(delta.penalties, 0);
    }

    #[test]
    fn component_delta_non_attesting_gets_penalty() {
        let spec = mainnet_spec();
        let total = make_total_balances(32_000_000_000, 24_000_000_000);
        let base_reward = 1000;
        let finality_delay = 1;

        let delta = get_attestation_component_delta(
            false,
            24_000_000_000,
            &total,
            base_reward,
            finality_delay,
            &spec,
        )
        .unwrap();
        assert_eq!(delta.rewards, 0);
        assert_eq!(
            delta.penalties, base_reward,
            "non-attester penalized by base_reward"
        );
    }

    #[test]
    fn component_delta_inactivity_leak_full_base_reward() {
        let spec = mainnet_spec();
        let total = make_total_balances(32_000_000_000, 16_000_000_000);
        let base_reward = 1000;
        // finality_delay > min_epochs_to_inactivity_penalty triggers inactivity leak
        let finality_delay = spec.min_epochs_to_inactivity_penalty + 1;

        let delta = get_attestation_component_delta(
            true,
            16_000_000_000,
            &total,
            base_reward,
            finality_delay,
            &spec,
        )
        .unwrap();
        // During inactivity leak, attesting validators get full base_reward
        assert_eq!(delta.rewards, base_reward);
        assert_eq!(delta.penalties, 0);
    }

    // --- get_inclusion_delay_delta ---

    #[test]
    fn inclusion_delay_delta_eligible_validator() {
        let spec = mainnet_spec();
        let base_reward = 10000;
        let validator = ValidatorStatus {
            is_previous_epoch_attester: true,
            is_slashed: false,
            inclusion_info: Some(InclusionInfo {
                delay: 1,
                proposer_index: 5,
            }),
            ..ValidatorStatus::default()
        };

        let (attester_delta, proposer) =
            get_inclusion_delay_delta(&validator, base_reward, &spec).unwrap();
        assert!(attester_delta.rewards > 0, "attester gets inclusion reward");
        let (proposer_index, proposer_delta) = proposer.unwrap();
        assert_eq!(proposer_index, 5);
        assert!(proposer_delta.rewards > 0, "proposer gets reward");
        // proposer_reward + max_attester_reward = base_reward
        let proposer_reward = proposer_delta.rewards;
        let max_attester_reward = base_reward - proposer_reward;
        assert_eq!(attester_delta.rewards, max_attester_reward); // delay=1 means full attester reward
    }

    #[test]
    fn inclusion_delay_delta_higher_delay_less_reward() {
        let spec = mainnet_spec();
        let base_reward = 10000;

        let make_validator = |delay: u64| ValidatorStatus {
            is_previous_epoch_attester: true,
            is_slashed: false,
            inclusion_info: Some(InclusionInfo {
                delay,
                proposer_index: 0,
            }),
            ..ValidatorStatus::default()
        };

        let (delta_1, _) =
            get_inclusion_delay_delta(&make_validator(1), base_reward, &spec).unwrap();
        let (delta_2, _) =
            get_inclusion_delay_delta(&make_validator(2), base_reward, &spec).unwrap();
        let (delta_4, _) =
            get_inclusion_delay_delta(&make_validator(4), base_reward, &spec).unwrap();

        assert!(
            delta_1.rewards > delta_2.rewards,
            "delay=1 should reward more than delay=2"
        );
        assert!(
            delta_2.rewards > delta_4.rewards,
            "delay=2 should reward more than delay=4"
        );
    }

    #[test]
    fn inclusion_delay_delta_slashed_validator_gets_nothing() {
        let spec = mainnet_spec();
        let validator = ValidatorStatus {
            is_previous_epoch_attester: true,
            is_slashed: true,
            inclusion_info: Some(InclusionInfo {
                delay: 1,
                proposer_index: 0,
            }),
            ..ValidatorStatus::default()
        };

        let (delta, proposer) = get_inclusion_delay_delta(&validator, 10000, &spec).unwrap();
        assert_eq!(delta.rewards, 0);
        assert_eq!(delta.penalties, 0);
        assert!(proposer.is_none());
    }

    #[test]
    fn inclusion_delay_delta_non_attester_gets_nothing() {
        let spec = mainnet_spec();
        let validator = ValidatorStatus::default();

        let (delta, proposer) = get_inclusion_delay_delta(&validator, 10000, &spec).unwrap();
        assert_eq!(delta.rewards, 0);
        assert_eq!(delta.penalties, 0);
        assert!(proposer.is_none());
    }

    // --- get_inactivity_penalty_delta ---

    #[test]
    fn inactivity_penalty_no_leak_no_penalty() {
        let spec = mainnet_spec();
        let validator = ValidatorStatus {
            is_eligible: true,
            is_previous_epoch_target_attester: true,
            ..ValidatorStatus::default()
        };
        // finality_delay <= min_epochs_to_inactivity_penalty = no inactivity leak
        let finality_delay = spec.min_epochs_to_inactivity_penalty;

        let delta = get_inactivity_penalty_delta(&validator, 1000, finality_delay, &spec).unwrap();
        assert_eq!(delta.rewards, 0);
        assert_eq!(delta.penalties, 0);
    }

    #[test]
    fn inactivity_penalty_leak_target_attester_base_penalty_only() {
        let spec = mainnet_spec();
        let base_reward = 1000;
        let validator = ValidatorStatus {
            is_eligible: true,
            is_previous_epoch_target_attester: true,
            current_epoch_effective_balance: 32_000_000_000,
            ..ValidatorStatus::default()
        };
        let finality_delay = spec.min_epochs_to_inactivity_penalty + 1;

        let delta =
            get_inactivity_penalty_delta(&validator, base_reward, finality_delay, &spec).unwrap();
        // Target attester: gets base penalty but NOT the extra inactivity penalty
        let proposer_reward = base_reward / spec.proposer_reward_quotient;
        let expected_penalty = spec.base_rewards_per_epoch * base_reward - proposer_reward;
        assert_eq!(delta.penalties, expected_penalty);
    }

    #[test]
    fn inactivity_penalty_leak_non_target_attester_gets_extra_penalty() {
        let spec = mainnet_spec();
        let base_reward = 1000;
        let effective_balance = 32_000_000_000u64;
        let finality_delay = spec.min_epochs_to_inactivity_penalty + 10;

        let target_attester = ValidatorStatus {
            is_eligible: true,
            is_previous_epoch_target_attester: true,
            current_epoch_effective_balance: effective_balance,
            ..ValidatorStatus::default()
        };
        let non_target = ValidatorStatus {
            is_eligible: true,
            is_previous_epoch_target_attester: false,
            current_epoch_effective_balance: effective_balance,
            ..ValidatorStatus::default()
        };

        let delta_target =
            get_inactivity_penalty_delta(&target_attester, base_reward, finality_delay, &spec)
                .unwrap();
        let delta_non_target =
            get_inactivity_penalty_delta(&non_target, base_reward, finality_delay, &spec).unwrap();

        assert!(
            delta_non_target.penalties > delta_target.penalties,
            "non-target attester should get extra inactivity penalty"
        );
        // The extra penalty = effective_balance * finality_delay / inactivity_penalty_quotient
        let extra = effective_balance * finality_delay / spec.inactivity_penalty_quotient;
        assert_eq!(delta_non_target.penalties - delta_target.penalties, extra);
    }

    #[test]
    fn inactivity_penalty_slashed_gets_extra_penalty() {
        let spec = mainnet_spec();
        let base_reward = 1000;
        let effective_balance = 32_000_000_000u64;
        let finality_delay = spec.min_epochs_to_inactivity_penalty + 5;

        let slashed = ValidatorStatus {
            is_eligible: true,
            is_slashed: true,
            is_previous_epoch_target_attester: true, // even with target match
            current_epoch_effective_balance: effective_balance,
            ..ValidatorStatus::default()
        };

        let delta =
            get_inactivity_penalty_delta(&slashed, base_reward, finality_delay, &spec).unwrap();
        // Slashed validators get the extra penalty even if they matched target
        let extra = effective_balance * finality_delay / spec.inactivity_penalty_quotient;
        let proposer_reward = base_reward / spec.proposer_reward_quotient;
        let base_penalty = spec.base_rewards_per_epoch * base_reward - proposer_reward;
        assert_eq!(delta.penalties, base_penalty + extra);
    }
}
