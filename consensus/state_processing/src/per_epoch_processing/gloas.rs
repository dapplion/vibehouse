use crate::EpochProcessingError;
use safe_arith::SafeArith;
use types::{BeaconState, BuilderPendingPayment, ChainSpec, EthSpec};

/// Processes the builder pending payments from the previous epoch.
///
/// Checks accumulated weights against the quorum threshold. Payments meeting the
/// threshold are moved to the withdrawal queue. The payment window then rotates forward.
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#new-process_builder_pending_payments
pub fn process_builder_pending_payments<E: EthSpec>(
    state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), EpochProcessingError> {
    let slots_per_epoch = E::slots_per_epoch() as usize;

    // Calculate quorum threshold: get_builder_payment_quorum_threshold
    // per_slot_balance = total_active_balance // SLOTS_PER_EPOCH
    // quorum = per_slot_balance * BUILDER_PAYMENT_THRESHOLD_NUMERATOR // BUILDER_PAYMENT_THRESHOLD_DENOMINATOR
    let total_active_balance = state.get_total_active_balance()?;
    let per_slot_balance = total_active_balance.safe_div(E::slots_per_epoch())?;
    let quorum = per_slot_balance
        .saturating_mul(spec.builder_payment_threshold_numerator)
        .safe_div(spec.builder_payment_threshold_denominator)?;

    let state_gloas = state.as_gloas_mut()?;

    // Check first SLOTS_PER_EPOCH entries against quorum, append qualifying withdrawals
    for i in 0..slots_per_epoch {
        if let Some(payment) = state_gloas.builder_pending_payments.get(i)
            && payment.weight >= quorum
        {
            let withdrawal = payment.withdrawal.clone();
            state_gloas.builder_pending_withdrawals.push(withdrawal)?;
        }
    }

    // Rotate: move second half to first half, clear second half
    // old_payments = state.builder_pending_payments[SLOTS_PER_EPOCH:]
    // new_payments = [BuilderPendingPayment() for _ in range(SLOTS_PER_EPOCH)]
    // state.builder_pending_payments = old_payments + new_payments
    let total_len = state_gloas.builder_pending_payments.len();
    for i in 0..slots_per_epoch {
        let src_idx = i.saturating_add(slots_per_epoch);
        let new_value = if src_idx < total_len {
            state_gloas
                .builder_pending_payments
                .get(src_idx)
                .cloned()
                .unwrap_or_default()
        } else {
            BuilderPendingPayment::default()
        };
        if let Some(slot) = state_gloas.builder_pending_payments.get_mut(i) {
            *slot = new_value;
        }
    }

    // Clear second half (set to default)
    for i in slots_per_epoch..total_len {
        if let Some(slot) = state_gloas.builder_pending_payments.get_mut(i) {
            *slot = BuilderPendingPayment::default();
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bls::FixedBytesExtended;
    use ssz_types::BitVector;
    use std::sync::Arc;
    use types::{
        Address, BeaconBlockHeader, BeaconStateGloas, Builder, BuilderPendingWithdrawal,
        CACHED_EPOCHS, Checkpoint, CommitteeCache, Epoch, ExecutionBlockHash, ExecutionPayloadBid,
        ExitCache, FixedVector, Fork, Hash256, List, MinimalEthSpec, ProgressiveBalancesCache,
        PubkeyCache, SlashingsCache, Slot, SyncCommittee, Unsigned, Vector,
    };

    type E = MinimalEthSpec;

    // For MinimalEthSpec:
    // - slots_per_epoch = 8
    // - BuilderPendingPaymentsLimit = 16 (2 * 8)
    // - quorum = (total_active_balance / 8) * 6 / 10
    //
    // With 8 validators at 32 ETH (32_000_000_000 Gwei each):
    // - total_active_balance = 256_000_000_000
    // - per_slot_balance = 32_000_000_000
    // - quorum = 32_000_000_000 * 6 / 10 = 19_200_000_000
    const BALANCE: u64 = 32_000_000_000;
    const NUM_VALIDATORS: usize = 8;

    fn quorum_for_balance(total_active: u64) -> u64 {
        let per_slot = total_active / E::slots_per_epoch();
        per_slot.saturating_mul(6) / 10
    }

    /// Build a minimal Gloas state for testing process_builder_pending_payments.
    fn make_state_for_payments(
        payments: Vec<BuilderPendingPayment>,
    ) -> (BeaconState<E>, ChainSpec) {
        let spec = E::default_spec();
        let slot = Slot::new(E::slots_per_epoch()); // slot 8, epoch 1
        let epoch = slot.epoch(E::slots_per_epoch());

        let keypairs = types::test_utils::generate_deterministic_keypairs(NUM_VALIDATORS);
        let mut validators = Vec::with_capacity(NUM_VALIDATORS);
        let mut balances = Vec::with_capacity(NUM_VALIDATORS);
        for kp in &keypairs {
            let mut creds = [0u8; 32];
            creds[0] = 0x01;
            creds[12..].copy_from_slice(&[0xAA; 20]);
            validators.push(types::Validator {
                pubkey: kp.pk.compress(),
                effective_balance: BALANCE,
                activation_epoch: Epoch::new(0),
                exit_epoch: spec.far_future_epoch,
                withdrawable_epoch: spec.far_future_epoch,
                withdrawal_credentials: Hash256::from_slice(&creds),
                ..types::Validator::default()
            });
            balances.push(BALANCE);
        }

        let builder = Builder {
            pubkey: types::PublicKeyBytes::empty(),
            version: 0x03,
            execution_address: Address::repeat_byte(0xBB),
            balance: 100_000_000_000,
            deposit_epoch: Epoch::new(0),
            withdrawable_epoch: spec.far_future_epoch,
        };

        let parent_root = Hash256::repeat_byte(0x01);
        let parent_block_hash = ExecutionBlockHash::repeat_byte(0x02);

        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let randao_mixes = vec![Hash256::zero(); epochs_per_vector];

        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                types::PublicKeyBytes::empty();
                <E as EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: types::PublicKeyBytes::empty(),
        });

        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

        // Fill payments vector to full length (16 for minimal)
        let payments_limit = E::builder_pending_payments_limit();
        let mut full_payments = payments;
        full_payments.resize(payments_limit, BuilderPendingPayment::default());

        let mut state = BeaconState::Gloas(BeaconStateGloas {
            genesis_time: 0,
            genesis_validators_root: Hash256::repeat_byte(0xAA),
            slot,
            fork: Fork {
                previous_version: spec.fulu_fork_version,
                current_version: spec.gloas_fork_version,
                epoch,
            },
            latest_block_header: BeaconBlockHeader {
                slot: slot.saturating_sub(1u64),
                proposer_index: 0,
                parent_root,
                state_root: Hash256::zero(),
                body_root: Hash256::zero(),
            },
            block_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            state_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            historical_roots: List::default(),
            eth1_data: types::Eth1Data::default(),
            eth1_data_votes: List::default(),
            eth1_deposit_index: 0,
            validators: List::new(validators).unwrap(),
            balances: List::new(balances).unwrap(),
            randao_mixes: Vector::new(randao_mixes).unwrap(),
            slashings: Vector::new(vec![0; epochs_per_slash]).unwrap(),
            previous_epoch_participation: List::default(),
            current_epoch_participation: List::default(),
            justification_bits: BitVector::new(),
            previous_justified_checkpoint: Checkpoint::default(),
            current_justified_checkpoint: Checkpoint::default(),
            finalized_checkpoint: Checkpoint {
                epoch: Epoch::new(1),
                root: Hash256::zero(),
            },
            inactivity_scores: List::default(),
            current_sync_committee: sync_committee.clone(),
            next_sync_committee: sync_committee,
            latest_execution_payload_bid: ExecutionPayloadBid {
                parent_block_hash,
                parent_block_root: parent_root,
                block_hash: ExecutionBlockHash::repeat_byte(0x04),
                slot,
                ..Default::default()
            },
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
            builders: List::new(vec![builder]).unwrap(),
            next_withdrawal_builder_index: 0,
            execution_payload_availability: BitVector::from_bytes(
                vec![0xFFu8; slots_per_hist / 8].into(),
            )
            .unwrap(),
            builder_pending_payments: Vector::new(full_payments).unwrap(),
            builder_pending_withdrawals: List::default(),
            latest_block_hash: parent_block_hash,
            payload_expected_withdrawals: List::default(),
            total_active_balance: None,
            progressive_balances_cache: ProgressiveBalancesCache::default(),
            committee_caches: <[Arc<CommitteeCache>; CACHED_EPOCHS]>::default(),
            pubkey_cache: PubkeyCache::default(),
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: types::EpochCache::default(),
        });

        // Initialize total active balance cache
        let total_active = NUM_VALIDATORS as u64 * BALANCE;
        state.set_total_active_balance(epoch, total_active, &spec);

        (state, spec)
    }

    fn make_payment(weight: u64, amount: u64, builder_index: u64) -> BuilderPendingPayment {
        BuilderPendingPayment {
            weight,
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xCC),
                amount,
                builder_index,
            },
        }
    }

    // ── Empty / all-default payments ──

    #[test]
    fn empty_payments_no_withdrawals() {
        let (mut state, spec) = make_state_for_payments(vec![]);
        process_builder_pending_payments::<E>(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.builder_pending_withdrawals.len(), 0);
    }

    // ── Quorum threshold checks ──

    #[test]
    fn payment_below_quorum_not_promoted() {
        // quorum = 19_200_000_000, set weight just below
        let quorum = quorum_for_balance(NUM_VALIDATORS as u64 * BALANCE);
        let payment = make_payment(quorum - 1, 1_000_000_000, 0);
        let (mut state, spec) = make_state_for_payments(vec![payment]);

        process_builder_pending_payments::<E>(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.builder_pending_withdrawals.len(), 0);
    }

    #[test]
    fn payment_at_quorum_promoted() {
        let quorum = quorum_for_balance(NUM_VALIDATORS as u64 * BALANCE);
        let payment = make_payment(quorum, 5_000_000_000, 0);
        let (mut state, spec) = make_state_for_payments(vec![payment]);

        process_builder_pending_payments::<E>(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.builder_pending_withdrawals.len(), 1);
        assert_eq!(
            gloas.builder_pending_withdrawals.get(0).unwrap().amount,
            5_000_000_000
        );
        assert_eq!(
            gloas
                .builder_pending_withdrawals
                .get(0)
                .unwrap()
                .builder_index,
            0
        );
    }

    #[test]
    fn payment_above_quorum_promoted() {
        let quorum = quorum_for_balance(NUM_VALIDATORS as u64 * BALANCE);
        let payment = make_payment(quorum + 1_000_000_000, 7_000_000_000, 0);
        let (mut state, spec) = make_state_for_payments(vec![payment]);

        process_builder_pending_payments::<E>(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.builder_pending_withdrawals.len(), 1);
        assert_eq!(
            gloas.builder_pending_withdrawals.get(0).unwrap().amount,
            7_000_000_000
        );
    }

    #[test]
    fn zero_weight_payment_not_promoted() {
        let payment = make_payment(0, 1_000_000_000, 0);
        let (mut state, spec) = make_state_for_payments(vec![payment]);

        process_builder_pending_payments::<E>(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.builder_pending_withdrawals.len(), 0);
    }

    // ── Mixed first-half payments ──

    #[test]
    fn mixed_payments_only_qualifying_promoted() {
        let quorum = quorum_for_balance(NUM_VALIDATORS as u64 * BALANCE);
        // 8 slots in first half: alternate above/below quorum
        let payments: Vec<_> = (0..8)
            .map(|i| {
                if i % 2 == 0 {
                    make_payment(quorum + 100, (i + 1) as u64 * 1_000_000_000, 0)
                } else {
                    make_payment(quorum - 100, (i + 1) as u64 * 1_000_000_000, 0)
                }
            })
            .collect();

        let (mut state, spec) = make_state_for_payments(payments);
        process_builder_pending_payments::<E>(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        // Only even indices (0, 2, 4, 6) should be promoted = 4 withdrawals
        assert_eq!(gloas.builder_pending_withdrawals.len(), 4);
        assert_eq!(
            gloas.builder_pending_withdrawals.get(0).unwrap().amount,
            1_000_000_000
        );
        assert_eq!(
            gloas.builder_pending_withdrawals.get(1).unwrap().amount,
            3_000_000_000
        );
        assert_eq!(
            gloas.builder_pending_withdrawals.get(2).unwrap().amount,
            5_000_000_000
        );
        assert_eq!(
            gloas.builder_pending_withdrawals.get(3).unwrap().amount,
            7_000_000_000
        );
    }

    #[test]
    fn all_payments_above_quorum() {
        let quorum = quorum_for_balance(NUM_VALIDATORS as u64 * BALANCE);
        let payments: Vec<_> = (0..8)
            .map(|i| make_payment(quorum, (i + 1) as u64 * 1_000_000_000, 0))
            .collect();

        let (mut state, spec) = make_state_for_payments(payments);
        process_builder_pending_payments::<E>(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.builder_pending_withdrawals.len(), 8);
    }

    // ── Multiple builders ──

    #[test]
    fn payments_for_different_builders() {
        let quorum = quorum_for_balance(NUM_VALIDATORS as u64 * BALANCE);
        let payments = vec![
            make_payment(quorum, 2_000_000_000, 0),
            make_payment(quorum, 3_000_000_000, 1),
            make_payment(quorum - 1, 4_000_000_000, 2), // below quorum
        ];

        let (mut state, spec) = make_state_for_payments(payments);
        process_builder_pending_payments::<E>(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.builder_pending_withdrawals.len(), 2);
        assert_eq!(
            gloas
                .builder_pending_withdrawals
                .get(0)
                .unwrap()
                .builder_index,
            0
        );
        assert_eq!(
            gloas
                .builder_pending_withdrawals
                .get(1)
                .unwrap()
                .builder_index,
            1
        );
    }

    // ── Rotation: second-half to first-half ──

    #[test]
    fn rotation_moves_second_half_to_first() {
        let quorum = quorum_for_balance(NUM_VALIDATORS as u64 * BALANCE);
        // Place payments in second half (indices 8-15)
        let mut payments = vec![BuilderPendingPayment::default(); 8]; // empty first half
        for i in 0..8 {
            payments.push(make_payment(
                quorum + 100,
                (i + 10) as u64 * 1_000_000_000,
                0,
            ));
        }

        let (mut state, spec) = make_state_for_payments(payments);
        process_builder_pending_payments::<E>(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        // No withdrawals from first half (all default/zero weight)
        assert_eq!(gloas.builder_pending_withdrawals.len(), 0);

        // Second half should now be in first half
        for i in 0..8 {
            let payment = gloas.builder_pending_payments.get(i).unwrap();
            assert_eq!(
                payment.weight,
                quorum + 100,
                "slot {} should have second-half payment weight",
                i
            );
            assert_eq!(
                payment.withdrawal.amount,
                (i + 10) as u64 * 1_000_000_000,
                "slot {} should have second-half payment amount",
                i
            );
        }

        // Second half should be cleared
        for i in 8..16 {
            let payment = gloas.builder_pending_payments.get(i).unwrap();
            assert_eq!(
                payment.weight, 0,
                "second half slot {} should be cleared",
                i
            );
        }
    }

    #[test]
    fn rotation_clears_second_half() {
        let quorum = quorum_for_balance(NUM_VALIDATORS as u64 * BALANCE);
        // Fill both halves with payments
        let payments: Vec<_> = (0..16)
            .map(|i| make_payment(quorum, (i + 1) as u64 * 1_000_000_000, 0))
            .collect();

        let (mut state, spec) = make_state_for_payments(payments);
        process_builder_pending_payments::<E>(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        // First half (0-7) promoted to withdrawals
        assert_eq!(gloas.builder_pending_withdrawals.len(), 8);

        // After rotation: first half has old second half values
        for i in 0..8 {
            let payment = gloas.builder_pending_payments.get(i).unwrap();
            assert_eq!(
                payment.withdrawal.amount,
                (i + 9) as u64 * 1_000_000_000,
                "first half slot {} should have old second-half amount",
                i
            );
        }

        // Second half all cleared
        for i in 8..16 {
            let payment = gloas.builder_pending_payments.get(i).unwrap();
            assert_eq!(payment.weight, 0, "second half slot {} should be zero", i);
            assert_eq!(payment.withdrawal.amount, 0);
        }
    }

    // ── Fee recipient preserved ──

    #[test]
    fn fee_recipient_preserved_in_withdrawal() {
        let quorum = quorum_for_balance(NUM_VALIDATORS as u64 * BALANCE);
        let payment = BuilderPendingPayment {
            weight: quorum,
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xDD),
                amount: 1_000_000_000,
                builder_index: 0,
            },
        };

        let (mut state, spec) = make_state_for_payments(vec![payment]);
        process_builder_pending_payments::<E>(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.builder_pending_withdrawals.len(), 1);
        assert_eq!(
            gloas
                .builder_pending_withdrawals
                .get(0)
                .unwrap()
                .fee_recipient,
            Address::repeat_byte(0xDD)
        );
    }

    // ── Only first-half checked for quorum ──

    #[test]
    fn second_half_payments_not_checked_for_quorum() {
        let quorum = quorum_for_balance(NUM_VALIDATORS as u64 * BALANCE);
        // Put qualifying payment only in second half
        let mut payments = vec![BuilderPendingPayment::default(); 8];
        payments.push(make_payment(quorum + 1000, 9_000_000_000, 0));

        let (mut state, spec) = make_state_for_payments(payments);
        process_builder_pending_payments::<E>(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        // No withdrawals generated — second half is not checked
        assert_eq!(gloas.builder_pending_withdrawals.len(), 0);
    }

    // ── Pre-existing withdrawals preserved ──

    #[test]
    fn existing_pending_withdrawals_preserved() {
        let quorum = quorum_for_balance(NUM_VALIDATORS as u64 * BALANCE);
        let payment = make_payment(quorum, 2_000_000_000, 0);

        let (mut state, spec) = make_state_for_payments(vec![payment]);

        // Add a pre-existing withdrawal
        let existing = BuilderPendingWithdrawal {
            fee_recipient: Address::repeat_byte(0xEE),
            amount: 500_000_000,
            builder_index: 0,
        };
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(existing)
            .unwrap();

        process_builder_pending_payments::<E>(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        // Pre-existing + newly promoted = 2
        assert_eq!(gloas.builder_pending_withdrawals.len(), 2);
        assert_eq!(
            gloas.builder_pending_withdrawals.get(0).unwrap().amount,
            500_000_000
        );
        assert_eq!(
            gloas.builder_pending_withdrawals.get(1).unwrap().amount,
            2_000_000_000
        );
    }

    // ── Quorum with different total balances ──

    #[test]
    fn quorum_scales_with_validator_balance() {
        let epoch = Slot::new(E::slots_per_epoch()).epoch(E::slots_per_epoch());

        // Use smaller balance so quorum is lower
        let small_balance: u64 = 1_000_000_000; // 1 ETH
        let total_active = NUM_VALIDATORS as u64 * small_balance; // 8 ETH
        // quorum = (8_000_000_000 / 8) * 6 / 10 = 600_000_000
        let expected_quorum = quorum_for_balance(total_active);
        assert_eq!(expected_quorum, 600_000_000);

        // Create state with small balance and a payment at exactly the quorum
        let payment = make_payment(expected_quorum, 100_000_000, 0);
        let (mut state, spec) = make_state_for_payments(vec![payment]);

        // Override total active balance to match small_balance
        state.set_total_active_balance(epoch, total_active, &spec);

        process_builder_pending_payments::<E>(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.builder_pending_withdrawals.len(), 1);
    }

    // ── Double processing idempotency ──

    #[test]
    fn second_call_processes_rotated_payments() {
        let quorum = quorum_for_balance(NUM_VALIDATORS as u64 * BALANCE);

        // Set up: qualifying payment in first half, qualifying payment in second half
        let mut payments = vec![make_payment(quorum, 1_000_000_000, 0)]; // slot 0
        payments.extend(vec![BuilderPendingPayment::default(); 7]); // slots 1-7 empty
        payments.push(make_payment(quorum, 2_000_000_000, 0)); // slot 8
        // slots 9-15 empty via padding

        let (mut state, spec) = make_state_for_payments(payments);

        // First call: slot 0 promoted, second half rotated to first
        process_builder_pending_payments::<E>(&mut state, &spec).unwrap();
        assert_eq!(
            state.as_gloas().unwrap().builder_pending_withdrawals.len(),
            1
        );

        // The payment that was at slot 8 is now at slot 0
        let rotated = state
            .as_gloas()
            .unwrap()
            .builder_pending_payments
            .get(0)
            .unwrap()
            .clone();
        assert_eq!(rotated.weight, quorum);
        assert_eq!(rotated.withdrawal.amount, 2_000_000_000);

        // Second call: now that rotated payment should be promoted
        process_builder_pending_payments::<E>(&mut state, &spec).unwrap();
        assert_eq!(
            state.as_gloas().unwrap().builder_pending_withdrawals.len(),
            2
        );
        assert_eq!(
            state
                .as_gloas()
                .unwrap()
                .builder_pending_withdrawals
                .get(1)
                .unwrap()
                .amount,
            2_000_000_000
        );
    }
}
