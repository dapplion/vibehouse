use crate::per_epoch_processing::{Error, JustificationAndFinalizationState};
use safe_arith::SafeArith;
use std::ops::Range;
use types::{Checkpoint, EthSpec};

/// Update the justified and finalized checkpoints for matching target attestations.
#[allow(clippy::if_same_then_else)] // For readability and consistency with spec.
pub fn weigh_justification_and_finalization<E: EthSpec>(
    mut state: JustificationAndFinalizationState<E>,
    total_active_balance: u64,
    previous_target_balance: u64,
    current_target_balance: u64,
) -> Result<JustificationAndFinalizationState<E>, Error> {
    let previous_epoch = state.previous_epoch();
    let current_epoch = state.current_epoch();

    let old_previous_justified_checkpoint = state.previous_justified_checkpoint();
    let old_current_justified_checkpoint = state.current_justified_checkpoint();

    // Process justifications
    *state.previous_justified_checkpoint_mut() = state.current_justified_checkpoint();
    state.justification_bits_mut().shift_up(1)?;

    if previous_target_balance.safe_mul(3)? >= total_active_balance.safe_mul(2)? {
        *state.current_justified_checkpoint_mut() = Checkpoint {
            epoch: previous_epoch,
            root: state.get_block_root_at_epoch(previous_epoch)?,
        };
        state.justification_bits_mut().set(1, true)?;
    }
    // If the current epoch gets justified, fill the last bit.
    if current_target_balance.safe_mul(3)? >= total_active_balance.safe_mul(2)? {
        *state.current_justified_checkpoint_mut() = Checkpoint {
            epoch: current_epoch,
            root: state.get_block_root_at_epoch(current_epoch)?,
        };
        state.justification_bits_mut().set(0, true)?;
    }

    let bits = state.justification_bits().clone();
    let all_bits_set = |range: Range<usize>| -> Result<bool, Error> {
        for i in range {
            if !bits.get(i).map_err(Error::InvalidJustificationBit)? {
                return Ok(false);
            }
        }
        Ok(true)
    };

    // The 2nd/3rd/4th most recent epochs are all justified, the 2nd using the 4th as source.
    if all_bits_set(1..4)? && old_previous_justified_checkpoint.epoch.safe_add(3)? == current_epoch
    {
        *state.finalized_checkpoint_mut() = old_previous_justified_checkpoint;
    }
    // The 2nd/3rd most recent epochs are both justified, the 2nd using the 3rd as source.
    if all_bits_set(1..3)? && old_previous_justified_checkpoint.epoch.safe_add(2)? == current_epoch
    {
        *state.finalized_checkpoint_mut() = old_previous_justified_checkpoint;
    }
    // The 1st/2nd/3rd most recent epochs are all justified, the 1st using the 3nd as source.
    if all_bits_set(0..3)? && old_current_justified_checkpoint.epoch.safe_add(2)? == current_epoch {
        *state.finalized_checkpoint_mut() = old_current_justified_checkpoint;
    }
    // The 1st/2nd most recent epochs are both justified, the 1st using the 2nd as source.
    if all_bits_set(0..2)? && old_current_justified_checkpoint.epoch.safe_add(1)? == current_epoch {
        *state.finalized_checkpoint_mut() = old_current_justified_checkpoint;
    }

    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::per_epoch_processing::JustificationAndFinalizationState;
    use bls::PublicKeyBytes;
    use std::sync::Arc;
    use types::beacon_state::BuilderPubkeyCache;
    use types::*;

    type E = types::MinimalEthSpec;

    /// Build a minimal BeaconState at the given epoch with distinct block roots per slot.
    fn make_state_at_epoch(epoch: u64, spec: &types::ChainSpec) -> BeaconState<E> {
        let current_epoch = Epoch::new(epoch);
        // Slot must be past the epoch start so get_block_root_at_epoch(current_epoch) works
        // (requires slot > epoch_start_slot).
        let slot = current_epoch
            .start_slot(E::slots_per_epoch())
            .saturating_add(1u64);

        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

        // Fill block_roots with distinct values so get_block_root_at_epoch returns identifiable roots.
        let block_roots: Vec<Hash256> = (0..slots_per_hist)
            .map(|i| Hash256::from_low_u64_be(i as u64 + 1))
            .collect();

        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                PublicKeyBytes::empty();
                <E as EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: PublicKeyBytes::empty(),
        });

        BeaconState::Gloas(BeaconStateGloas {
            genesis_time: 0,
            genesis_validators_root: Hash256::repeat_byte(0),
            slot,
            fork: Fork {
                previous_version: spec.fulu_fork_version,
                current_version: spec.gloas_fork_version,
                epoch: current_epoch,
            },
            latest_block_header: BeaconBlockHeader {
                slot: slot.saturating_sub(1u64),
                proposer_index: 0,
                parent_root: Hash256::repeat_byte(0),
                state_root: Hash256::repeat_byte(0),
                body_root: Hash256::repeat_byte(0),
            },
            block_roots: Vector::new(block_roots).unwrap(),
            state_roots: Vector::new(vec![Hash256::repeat_byte(0); slots_per_hist]).unwrap(),
            historical_roots: List::default(),
            eth1_data: Eth1Data::default(),
            eth1_data_votes: List::default(),
            eth1_deposit_index: 0,
            validators: List::new(vec![
                Validator {
                    pubkey: PublicKeyBytes::empty(),
                    withdrawal_credentials: Hash256::repeat_byte(0),
                    effective_balance: spec.max_effective_balance,
                    slashed: false,
                    activation_eligibility_epoch: Epoch::new(0),
                    activation_epoch: Epoch::new(0),
                    exit_epoch: spec.far_future_epoch,
                    withdrawable_epoch: spec.far_future_epoch,
                };
                4
            ])
            .unwrap(),
            balances: List::new(vec![spec.max_effective_balance; 4]).unwrap(),
            randao_mixes: Vector::new(vec![Hash256::repeat_byte(0); epochs_per_vector]).unwrap(),
            slashings: Vector::new(vec![0; epochs_per_slash]).unwrap(),
            previous_epoch_participation: List::default(),
            current_epoch_participation: List::default(),
            justification_bits: BitVector::new(),
            previous_justified_checkpoint: Checkpoint::default(),
            current_justified_checkpoint: Checkpoint::default(),
            finalized_checkpoint: Checkpoint::default(),
            inactivity_scores: List::default(),
            current_sync_committee: sync_committee.clone(),
            next_sync_committee: sync_committee,
            latest_execution_payload_bid: ExecutionPayloadBid::default(),
            next_withdrawal_index: 0,
            next_withdrawal_validator_index: 0,
            historical_summaries: List::default(),
            deposit_requests_start_index: u64::MAX,
            deposit_balance_to_consume: 0,
            exit_balance_to_consume: 0,
            earliest_exit_epoch: Epoch::new(0),
            consolidation_balance_to_consume: 0,
            earliest_consolidation_epoch: Epoch::new(0),
            pending_deposits: List::default(),
            pending_partial_withdrawals: List::default(),
            pending_consolidations: List::default(),
            proposer_lookahead: Vector::new(vec![
                0u64;
                <E as EthSpec>::ProposerLookaheadSlots::to_usize()
            ])
            .unwrap(),
            builders: List::default(),
            next_withdrawal_builder_index: 0,
            execution_payload_availability: BitVector::from_bytes(
                vec![0xFFu8; slots_per_hist / 8].into(),
            )
            .unwrap(),
            builder_pending_payments: Vector::new(vec![
                BuilderPendingPayment::default();
                E::builder_pending_payments_limit()
            ])
            .unwrap(),
            builder_pending_withdrawals: List::default(),
            latest_block_hash: ExecutionBlockHash::zero(),
            payload_expected_withdrawals: List::default(),
            total_active_balance: None,
            progressive_balances_cache: ProgressiveBalancesCache::default(),
            committee_caches: <_>::default(),
            pubkey_cache: PubkeyCache::default(),
            builder_pubkey_cache: BuilderPubkeyCache::default(),
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: EpochCache::default(),
        })
    }

    /// Create a JustificationAndFinalizationState with customized checkpoints and bits.
    fn make_jf_state(
        epoch: u64,
        prev_justified_epoch: u64,
        curr_justified_epoch: u64,
        finalized_epoch: u64,
        bits: [bool; 4],
        spec: &types::ChainSpec,
    ) -> JustificationAndFinalizationState<E> {
        let mut state = make_state_at_epoch(epoch, spec);

        let prev_root = *state
            .get_block_root_at_epoch(Epoch::new(prev_justified_epoch))
            .unwrap_or(&Hash256::repeat_byte(0));
        let curr_root = *state
            .get_block_root_at_epoch(Epoch::new(curr_justified_epoch))
            .unwrap_or(&Hash256::repeat_byte(0));
        let fin_root = *state
            .get_block_root_at_epoch(Epoch::new(finalized_epoch))
            .unwrap_or(&Hash256::repeat_byte(0));

        *state.previous_justified_checkpoint_mut() = Checkpoint {
            epoch: Epoch::new(prev_justified_epoch),
            root: prev_root,
        };
        *state.current_justified_checkpoint_mut() = Checkpoint {
            epoch: Epoch::new(curr_justified_epoch),
            root: curr_root,
        };
        *state.finalized_checkpoint_mut() = Checkpoint {
            epoch: Epoch::new(finalized_epoch),
            root: fin_root,
        };

        // Set justification bits
        let jbits = state.justification_bits_mut();
        for (i, &val) in bits.iter().enumerate() {
            jbits.set(i, val).unwrap();
        }

        JustificationAndFinalizationState::new(&state)
    }

    // ── Justification tests ──

    #[test]
    fn no_justification_below_threshold() {
        let spec = E::default_spec();
        let state = make_jf_state(5, 2, 3, 1, [false; 4], &spec);
        let total = 300;
        // Both targets below 2/3 threshold (199 < 200)
        let result = weigh_justification_and_finalization::<E>(state, total, 199, 199).unwrap();

        // No new justification — bits should all be false (shifted up, nothing set)
        assert!(!result.justification_bits().get(0).unwrap());
        assert!(!result.justification_bits().get(1).unwrap());
    }

    #[test]
    fn previous_epoch_justified_at_exact_threshold() {
        let spec = E::default_spec();
        let state = make_jf_state(5, 2, 3, 1, [false; 4], &spec);
        let total = 300;
        // previous_target * 3 = 600 >= total * 2 = 600 → exactly at threshold
        let result = weigh_justification_and_finalization::<E>(state, total, 200, 0).unwrap();

        assert!(result.justification_bits().get(1).unwrap());
        assert!(!result.justification_bits().get(0).unwrap());
        // current_justified should be updated to previous_epoch
        assert_eq!(result.current_justified_checkpoint().epoch, Epoch::new(4));
    }

    #[test]
    fn current_epoch_justified_at_exact_threshold() {
        let spec = E::default_spec();
        let state = make_jf_state(5, 2, 3, 1, [false; 4], &spec);
        let total = 300;
        // Only current target at threshold
        let result = weigh_justification_and_finalization::<E>(state, total, 0, 200).unwrap();

        assert!(result.justification_bits().get(0).unwrap());
        assert!(!result.justification_bits().get(1).unwrap());
        assert_eq!(result.current_justified_checkpoint().epoch, Epoch::new(5));
    }

    #[test]
    fn both_epochs_justified() {
        let spec = E::default_spec();
        let state = make_jf_state(5, 2, 3, 1, [false; 4], &spec);
        let total = 300;
        let result = weigh_justification_and_finalization::<E>(state, total, 200, 200).unwrap();

        assert!(result.justification_bits().get(0).unwrap());
        assert!(result.justification_bits().get(1).unwrap());
        // When both justified, current epoch overwrites (it runs second)
        assert_eq!(result.current_justified_checkpoint().epoch, Epoch::new(5));
    }

    #[test]
    fn previous_justified_becomes_previous() {
        let spec = E::default_spec();
        // current_justified_checkpoint starts at epoch 3
        let state = make_jf_state(5, 2, 3, 1, [false; 4], &spec);
        let total = 300;
        let result = weigh_justification_and_finalization::<E>(state, total, 0, 0).unwrap();

        // previous_justified should be set to old current_justified (epoch 3)
        assert_eq!(result.previous_justified_checkpoint().epoch, Epoch::new(3));
    }

    #[test]
    fn justification_bits_shift_up() {
        let spec = E::default_spec();
        // Start with bits [true, false, true, false]
        let state = make_jf_state(5, 2, 3, 1, [true, false, true, false], &spec);
        let total = 300;
        // No new justification
        let result = weigh_justification_and_finalization::<E>(state, total, 0, 0).unwrap();

        // After shift_up(1): [false, true, false, true] → old bit 0→1, old bit 2→3
        assert!(!result.justification_bits().get(0).unwrap());
        assert!(result.justification_bits().get(1).unwrap());
        assert!(!result.justification_bits().get(2).unwrap());
        assert!(result.justification_bits().get(3).unwrap());
    }

    // ── Finalization rule tests ──
    // Current epoch = 5, so:
    //   Rule 1 (bits 1..4): old_prev_justified.epoch + 3 == 5 → epoch 2
    //   Rule 2 (bits 1..3): old_prev_justified.epoch + 2 == 5 → epoch 3
    //   Rule 3 (bits 0..3): old_curr_justified.epoch + 2 == 5 → epoch 3
    //   Rule 4 (bits 0..2): old_curr_justified.epoch + 1 == 5 → epoch 4

    #[test]
    fn finalization_rule_1_234_justified() {
        let spec = E::default_spec();
        // old_previous_justified at epoch 2 (2+3=5=current_epoch)
        // Need bits 1,2,3 set AFTER shift. Start with bits [true, true, true, false]
        // After shift: [false, true, true, true] → bits 1,2,3 set ✓
        // No new justification needed (balance=0), so bit 0 stays false
        let state = make_jf_state(5, 2, 3, 1, [true, true, true, false], &spec);
        let total = 300;
        let result = weigh_justification_and_finalization::<E>(state, total, 0, 0).unwrap();

        // Finalized at old_previous_justified (epoch 2)
        assert_eq!(result.finalized_checkpoint().epoch, Epoch::new(2));
    }

    #[test]
    fn finalization_rule_2_23_justified() {
        let spec = E::default_spec();
        // old_previous_justified at epoch 3 (3+2=5=current_epoch)
        // Need bits 1,2 set AFTER shift. Start with bits [true, true, false, false]
        // After shift: [false, true, true, false] → bits 1,2 set ✓
        let state = make_jf_state(5, 3, 3, 1, [true, true, false, false], &spec);
        let total = 300;
        let result = weigh_justification_and_finalization::<E>(state, total, 0, 0).unwrap();

        // Finalized at old_previous_justified (epoch 3)
        assert_eq!(result.finalized_checkpoint().epoch, Epoch::new(3));
    }

    #[test]
    fn finalization_rule_3_123_justified() {
        let spec = E::default_spec();
        // old_current_justified at epoch 3 (3+2=5=current_epoch)
        // Need bits 0,1,2 set AFTER shift. Start with bits [false, true, false, false]
        // After shift: [false, false, true, false] → bit 2 set
        // Then previous justified (200 >= 200) sets bit 1. current justified (200 >= 200) sets bit 0.
        // Final: bits 0,1,2 set ✓
        let state = make_jf_state(5, 2, 3, 1, [false, true, false, false], &spec);
        let total = 300;
        let result = weigh_justification_and_finalization::<E>(state, total, 200, 200).unwrap();

        // Finalized at old_current_justified (epoch 3)
        assert_eq!(result.finalized_checkpoint().epoch, Epoch::new(3));
    }

    #[test]
    fn finalization_rule_4_12_justified() {
        let spec = E::default_spec();
        // old_current_justified at epoch 4 (4+1=5=current_epoch)
        // Need bits 0,1 set AFTER shift.
        // Both balances above threshold → bit 1 set (previous) and bit 0 set (current)
        let state = make_jf_state(5, 2, 4, 1, [false; 4], &spec);
        let total = 300;
        let result = weigh_justification_and_finalization::<E>(state, total, 200, 200).unwrap();

        // Finalized at old_current_justified (epoch 4)
        assert_eq!(result.finalized_checkpoint().epoch, Epoch::new(4));
    }

    #[test]
    fn no_finalization_when_epoch_mismatch() {
        let spec = E::default_spec();
        // old_previous_justified at epoch 1 — 1+3=4 ≠ 5, 1+2=3 ≠ 5
        // old_current_justified at epoch 2 — 2+2=4 ≠ 5, 2+1=3 ≠ 5
        // Even with all bits set, no finalization rule matches
        let state = make_jf_state(5, 1, 2, 0, [true, true, true, true], &spec);
        let total = 300;
        let result = weigh_justification_and_finalization::<E>(state, total, 200, 200).unwrap();

        // Finalized checkpoint unchanged (epoch 0)
        assert_eq!(result.finalized_checkpoint().epoch, Epoch::new(0));
    }

    #[test]
    fn finalization_rule_4_takes_priority_over_rule_1() {
        let spec = E::default_spec();
        // Set up so both rule 1 and rule 4 can fire:
        // Rule 1: old_prev_justified epoch 2, bits 1,2,3 set → finalizes epoch 2
        // Rule 4: old_curr_justified epoch 4, bits 0,1 set → finalizes epoch 4
        // Rule 4 runs last, so epoch 4 wins
        // Start bits: [true, true, true, false] → after shift: [false, true, true, true]
        // Then both justified → bit 0 and bit 1 set → [true, true, true, true]
        let state = make_jf_state(5, 2, 4, 0, [true, true, true, false], &spec);
        let total = 300;
        let result = weigh_justification_and_finalization::<E>(state, total, 200, 200).unwrap();

        // Rule 4 finalizes old_current_justified (epoch 4), overwriting rule 1's epoch 2
        assert_eq!(result.finalized_checkpoint().epoch, Epoch::new(4));
    }

    #[test]
    fn just_below_threshold_no_justification() {
        let spec = E::default_spec();
        let state = make_jf_state(5, 2, 3, 1, [false; 4], &spec);
        let total = 300;
        // 199 * 3 = 597 < 300 * 2 = 600 → below threshold
        let result = weigh_justification_and_finalization::<E>(state, total, 199, 199).unwrap();

        assert!(!result.justification_bits().get(0).unwrap());
        assert!(!result.justification_bits().get(1).unwrap());
    }

    #[test]
    fn just_above_threshold_justifies() {
        let spec = E::default_spec();
        let state = make_jf_state(5, 2, 3, 1, [false; 4], &spec);
        let total = 300;
        // 201 * 3 = 603 >= 300 * 2 = 600 → above threshold
        let result = weigh_justification_and_finalization::<E>(state, total, 201, 201).unwrap();

        assert!(result.justification_bits().get(0).unwrap());
        assert!(result.justification_bits().get(1).unwrap());
    }

    #[test]
    fn zero_balances_no_justification() {
        let spec = E::default_spec();
        let state = make_jf_state(5, 2, 3, 1, [false; 4], &spec);
        // 0 * 3 = 0 >= 0 * 2 = 0 → both are zero, should justify (0 >= 0 is true)
        let result = weigh_justification_and_finalization::<E>(state, 0, 0, 0).unwrap();

        assert!(result.justification_bits().get(0).unwrap());
        assert!(result.justification_bits().get(1).unwrap());
    }

    #[test]
    fn finalization_does_not_regress() {
        let spec = E::default_spec();
        // Finalized already at epoch 3, rule 1 would finalize epoch 2 — but finalized_checkpoint
        // is just overwritten (the function doesn't check for regression, spec doesn't require it)
        let state = make_jf_state(5, 2, 3, 3, [true, true, true, false], &spec);
        let total = 300;
        let result = weigh_justification_and_finalization::<E>(state, total, 0, 0).unwrap();

        // Rule 1 fires: bits 1,2,3 set and epoch 2+3=5 → finalized goes to epoch 2
        // This demonstrates the function doesn't prevent regression (spec-conformant)
        assert_eq!(result.finalized_checkpoint().epoch, Epoch::new(2));
    }

    #[test]
    fn finalization_preserves_root() {
        let spec = E::default_spec();
        // Rule 2: old_previous_justified at epoch 3 with its block root
        let state = make_jf_state(5, 3, 3, 1, [true, true, false, false], &spec);
        let expected_root = state.previous_justified_checkpoint().root;
        let total = 300;
        let result = weigh_justification_and_finalization::<E>(state, total, 0, 0).unwrap();

        assert_eq!(result.finalized_checkpoint().epoch, Epoch::new(3));
        assert_eq!(result.finalized_checkpoint().root, expected_root);
    }

    #[test]
    fn current_justified_root_matches_block_root() {
        let spec = E::default_spec();
        let beacon_state = make_state_at_epoch(5, &spec);
        let expected_current_root = *beacon_state.get_block_root_at_epoch(Epoch::new(5)).unwrap();
        let expected_previous_root = *beacon_state.get_block_root_at_epoch(Epoch::new(4)).unwrap();

        let state = JustificationAndFinalizationState::new(&beacon_state);
        let total = 300;
        let result = weigh_justification_and_finalization::<E>(state, total, 200, 200).unwrap();

        // Current epoch justified → root should match block root at current epoch
        assert_eq!(
            result.current_justified_checkpoint().root,
            expected_current_root
        );
        // When only previous is justified (not current), root matches previous epoch block root
        let state2 = JustificationAndFinalizationState::new(&beacon_state);
        let result2 = weigh_justification_and_finalization::<E>(state2, total, 200, 0).unwrap();
        assert_eq!(
            result2.current_justified_checkpoint().root,
            expected_previous_root
        );
    }
}
