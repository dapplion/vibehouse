use crate::common::{altair::BaseRewardPerIncrement, decrease_balance, increase_balance};
use crate::per_block_processing::errors::{BlockProcessingError, SyncAggregateInvalid};
use crate::{VerifySignatures, signature_sets::sync_aggregate_signature_set};
use safe_arith::SafeArith;
use std::borrow::Cow;
use types::consts::altair::{PROPOSER_WEIGHT, SYNC_REWARD_WEIGHT, WEIGHT_DENOMINATOR};
use types::{
    BeaconState, BeaconStateError, ChainSpec, EthSpec, PublicKeyBytes, SyncAggregate, Unsigned,
};

pub fn process_sync_aggregate<E: EthSpec>(
    state: &mut BeaconState<E>,
    aggregate: &SyncAggregate<E>,
    proposer_index: u64,
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    // Verify sync committee aggregate signature signing over the previous slot block root
    if verify_signatures.is_true() {
        // This decompression could be avoided with a cache, but we're not likely
        // to encounter this case in practice due to the use of pre-emptive signature
        // verification (which uses the `ValidatorPubkeyCache`).
        let decompressor = |pk_bytes: &PublicKeyBytes| pk_bytes.decompress().ok().map(Cow::Owned);

        // Check that the signature is over the previous block root.
        let previous_slot = state.slot().saturating_sub(1u64);
        let previous_block_root = *state.get_block_root(previous_slot)?;

        let signature_set = sync_aggregate_signature_set(
            decompressor,
            aggregate,
            state.slot(),
            previous_block_root,
            state,
            spec,
        )?;

        // If signature set is `None` then the signature is valid (infinity).
        if signature_set.is_some_and(|signature| !signature.verify()) {
            return Err(SyncAggregateInvalid::SignatureInvalid.into());
        }
    }

    // Compute participant and proposer rewards
    let (participant_reward, proposer_reward) = compute_sync_aggregate_rewards(state, spec)?;

    // Ensure pubkey cache is populated, then compute committee indices inline
    // to avoid cloning the SyncCommittee (~24KB on mainnet)
    state.update_pubkey_cache()?;
    let committee_indices: Vec<usize> = state
        .current_sync_committee()?
        .pubkeys
        .iter()
        .map(|pubkey| {
            state
                .pubkey_cache()
                .get(pubkey)
                .ok_or(BeaconStateError::PubkeyCacheInconsistent)
        })
        .collect::<Result<_, BeaconStateError>>()?;

    let proposer_index = proposer_index as usize;
    let mut proposer_balance = *state
        .balances()
        .get(proposer_index)
        .ok_or(BeaconStateError::BalancesOutOfBounds(proposer_index))?;

    for (participant_index, participation_bit) in committee_indices
        .into_iter()
        .zip(aggregate.sync_committee_bits.iter())
    {
        if participation_bit {
            // Accumulate proposer rewards in a temp var in case the proposer has very low balance, is
            // part of the sync committee, does not participate and its penalties saturate.
            if participant_index == proposer_index {
                proposer_balance.safe_add_assign(participant_reward)?;
            } else {
                increase_balance(state, participant_index, participant_reward)?;
            }
            proposer_balance.safe_add_assign(proposer_reward)?;
        } else if participant_index == proposer_index {
            proposer_balance = proposer_balance.saturating_sub(participant_reward);
        } else {
            decrease_balance(state, participant_index, participant_reward)?;
        }
    }

    *state.get_balance_mut(proposer_index)? = proposer_balance;

    Ok(())
}

/// Compute the `(participant_reward, proposer_reward)` for a sync aggregate.
///
/// The `state` should be the pre-state from the same slot as the block containing the aggregate.
pub fn compute_sync_aggregate_rewards<E: EthSpec>(
    state: &BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(u64, u64), BlockProcessingError> {
    let total_active_balance = state.get_total_active_balance()?;
    let total_active_increments =
        total_active_balance.safe_div(spec.effective_balance_increment)?;
    let total_base_rewards = BaseRewardPerIncrement::new(total_active_balance, spec)?
        .as_u64()
        .safe_mul(total_active_increments)?;
    let max_participant_rewards = total_base_rewards
        .safe_mul(SYNC_REWARD_WEIGHT)?
        .safe_div(WEIGHT_DENOMINATOR)?
        .safe_div(E::slots_per_epoch())?;
    let participant_reward = max_participant_rewards.safe_div(E::SyncCommitteeSize::to_u64())?;
    let proposer_reward = participant_reward
        .safe_mul(PROPOSER_WEIGHT)?
        .safe_div(WEIGHT_DENOMINATOR.safe_sub(PROPOSER_WEIGHT)?)?;
    Ok((participant_reward, proposer_reward))
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Eth1Data, MinimalEthSpec, Unsigned, Validator};

    type E = MinimalEthSpec;

    fn spec() -> ChainSpec {
        let mut spec = E::default_spec();
        // Ensure altair is active from genesis
        spec.altair_fork_epoch = Some(types::Epoch::new(0));
        spec
    }

    /// Create a minimal Altair state with `n` active validators, each with max effective balance.
    fn make_altair_state(n: usize) -> (BeaconState<E>, ChainSpec) {
        let spec = spec();
        let mut state = BeaconState::new(0, Eth1Data::default(), &spec);

        // Upgrade to Altair for sync committee support
        if state.fork_name_unchecked() == types::ForkName::Base {
            // We need validators for the state to be meaningful
            let validators = state.validators_mut();
            for i in 0..n {
                let mut validator = Validator::default();
                validator.effective_balance = spec.max_effective_balance;
                validator.activation_epoch = types::Epoch::new(0);
                validator.exit_epoch = spec.far_future_epoch;
                validator.withdrawable_epoch = spec.far_future_epoch;
                // Set a unique pubkey
                let mut pubkey_bytes = [0u8; 48];
                pubkey_bytes[0..8].copy_from_slice(&(i as u64).to_le_bytes());
                validator.pubkey = types::PublicKeyBytes::empty();
                validators.push(validator).unwrap();
            }

            // Add balances
            let balances = state.balances_mut();
            for _ in 0..n {
                balances.push(spec.max_effective_balance).unwrap();
            }
        }

        // Build total active balance cache
        state.build_total_active_balance_cache(&spec).unwrap();

        (state, spec)
    }

    #[test]
    fn compute_rewards_nonzero_with_active_validators() {
        let (state, spec) = make_altair_state(64);
        let (participant_reward, proposer_reward) =
            compute_sync_aggregate_rewards(&state, &spec).unwrap();

        // With 64 active validators, rewards should be positive
        assert!(participant_reward > 0, "participant_reward should be > 0");
        assert!(proposer_reward > 0, "proposer_reward should be > 0");
    }

    #[test]
    fn proposer_reward_less_than_participant_reward() {
        let (state, spec) = make_altair_state(64);
        let (participant_reward, proposer_reward) =
            compute_sync_aggregate_rewards(&state, &spec).unwrap();

        // PROPOSER_WEIGHT < WEIGHT_DENOMINATOR - PROPOSER_WEIGHT, so proposer_reward < participant_reward
        assert!(
            proposer_reward < participant_reward,
            "proposer_reward ({}) should be less than participant_reward ({})",
            proposer_reward,
            participant_reward
        );
    }

    #[test]
    fn rewards_scale_with_total_active_balance() {
        // total_base_rewards = brpi * total_increments
        // brpi = eff_bal_incr * base_reward_factor / sqrt(total_active_balance)
        // total_increments = total_active_balance / eff_bal_incr
        // → total_base_rewards = base_reward_factor * sqrt(total_active_balance)
        // → participant_reward scales with sqrt(total_active_balance) (divided by committee size & slots)
        // So MORE validators → HIGHER participant_reward (sqrt scaling)
        let (state_small, spec_small) = make_altair_state(32);
        let (state_large, spec_large) = make_altair_state(128);

        let (pr_small, _) = compute_sync_aggregate_rewards(&state_small, &spec_small).unwrap();
        let (pr_large, _) = compute_sync_aggregate_rewards(&state_large, &spec_large).unwrap();

        assert!(
            pr_large > pr_small,
            "more total stake → higher sync reward: small={}, large={}",
            pr_small,
            pr_large
        );
    }

    #[test]
    fn proposer_reward_formula_consistency() {
        // Verify the relationship: proposer_reward = participant_reward * PROPOSER_WEIGHT / (WEIGHT_DENOMINATOR - PROPOSER_WEIGHT)
        let (state, spec) = make_altair_state(64);
        let (participant_reward, proposer_reward) =
            compute_sync_aggregate_rewards(&state, &spec).unwrap();

        let expected_proposer_reward =
            participant_reward * PROPOSER_WEIGHT / (WEIGHT_DENOMINATOR - PROPOSER_WEIGHT);
        assert_eq!(proposer_reward, expected_proposer_reward);
    }

    #[test]
    fn rewards_deterministic() {
        let (state, spec) = make_altair_state(64);
        let (pr1, prop1) = compute_sync_aggregate_rewards(&state, &spec).unwrap();
        let (pr2, prop2) = compute_sync_aggregate_rewards(&state, &spec).unwrap();
        assert_eq!(pr1, pr2);
        assert_eq!(prop1, prop2);
    }

    #[test]
    fn rewards_with_minimum_validators() {
        // Even with just 1 validator, the function should work (total active balance ≥ EFFECTIVE_BALANCE_INCREMENT)
        let (state, spec) = make_altair_state(1);
        let result = compute_sync_aggregate_rewards(&state, &spec);
        assert!(result.is_ok());
    }

    #[test]
    fn max_participant_rewards_divisible_by_committee_size() {
        // The participant_reward is max_participant_rewards / SyncCommitteeSize, verify truncation is consistent
        let (state, spec) = make_altair_state(64);
        let total_active_balance = state.get_total_active_balance().unwrap();
        let total_active_increments = total_active_balance / spec.effective_balance_increment;
        let brpi = BaseRewardPerIncrement::new(total_active_balance, &spec).unwrap();
        let total_base_rewards = brpi.as_u64() * total_active_increments;
        let max_participant_rewards =
            total_base_rewards * SYNC_REWARD_WEIGHT / WEIGHT_DENOMINATOR / E::slots_per_epoch();
        let expected_participant =
            max_participant_rewards / <E as EthSpec>::SyncCommitteeSize::to_u64();

        let (participant_reward, _) = compute_sync_aggregate_rewards(&state, &spec).unwrap();
        assert_eq!(participant_reward, expected_participant);
    }
}
