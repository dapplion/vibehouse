#![cfg(test)]
use crate::test_utils::*;
use beacon_chain::test_utils::{BeaconChainHarness, EphemeralHarnessType};
use beacon_chain::types::{
    BeaconState, BeaconStateAltair, BeaconStateBase, BeaconStateError, ChainSpec, Domain, Epoch,
    EthSpec, FixedBytesExtended, Hash256, Keypair, MainnetEthSpec, MinimalEthSpec, RelativeEpoch,
    Slot, Vector, test_utils::TestRandom,
};
use ssz::Encode;
use std::ops::Mul;
use std::sync::LazyLock;
use swap_or_not_shuffle::compute_shuffled_index;

pub const MAX_VALIDATOR_COUNT: usize = 129;
pub const SLOT_OFFSET: Slot = Slot::new(1);

/// A cached set of keys.
static KEYPAIRS: LazyLock<Vec<Keypair>> =
    LazyLock::new(|| generate_deterministic_keypairs(MAX_VALIDATOR_COUNT));

async fn get_harness<E: EthSpec>(
    validator_count: usize,
    slot: Slot,
) -> BeaconChainHarness<EphemeralHarnessType<E>> {
    let harness = BeaconChainHarness::builder(E::default())
        .default_spec()
        .keypairs(KEYPAIRS[0..validator_count].to_vec())
        .fresh_ephemeral_store()
        .build();

    let skip_to_slot = slot - SLOT_OFFSET;
    if skip_to_slot > Slot::new(0) {
        let slots = (skip_to_slot.as_u64()..=slot.as_u64())
            .map(Slot::new)
            .collect::<Vec<_>>();
        let state = harness.get_current_state();
        harness
            .add_attested_blocks_at_slots(
                state,
                Hash256::zero(),
                slots.as_slice(),
                (0..validator_count).collect::<Vec<_>>().as_slice(),
            )
            .await;
    }
    harness
}

async fn build_state<E: EthSpec>(validator_count: usize) -> BeaconState<E> {
    get_harness(validator_count, Slot::new(0))
        .await
        .chain
        .head_beacon_state_cloned()
}

async fn test_beacon_proposer_index<E: EthSpec>() {
    let spec = E::default_spec();

    // Get the i'th candidate proposer for the given state and slot
    let ith_candidate = |state: &BeaconState<E>, slot: Slot, i: usize, spec: &ChainSpec| {
        let epoch = slot.epoch(E::slots_per_epoch());
        let seed = state.get_beacon_proposer_seed(slot, spec).unwrap();
        let active_validators = state.get_active_validator_indices(epoch, spec).unwrap();
        active_validators[compute_shuffled_index(
            i,
            active_validators.len(),
            &seed,
            spec.shuffle_round_count,
        )
        .unwrap()]
    };

    // Run a test on the state.
    let test = |state: &BeaconState<E>, slot: Slot, candidate_index: usize| {
        assert_eq!(
            state.get_beacon_proposer_index(slot, &spec),
            Ok(ith_candidate(state, slot, candidate_index, &spec))
        );
    };

    // Test where we have one validator per slot.
    // 0th candidate should be chosen every time.
    let state = build_state(E::slots_per_epoch() as usize).await;
    for i in 0..E::slots_per_epoch() {
        test(&state, Slot::from(i), 0);
    }

    // Test where we have two validators per slot.
    // 0th candidate should be chosen every time.
    let state = build_state((E::slots_per_epoch() as usize).mul(2)).await;
    for i in 0..E::slots_per_epoch() {
        test(&state, Slot::from(i), 0);
    }

    // Test with two validators per slot, first validator has zero balance.
    let mut state = build_state::<E>((E::slots_per_epoch() as usize).mul(2)).await;
    let slot0_candidate0 = ith_candidate(&state, Slot::new(0), 0, &spec);
    state
        .validators_mut()
        .get_mut(slot0_candidate0)
        .unwrap()
        .effective_balance = 0;
    test(&state, Slot::new(0), 1);
    for i in 1..E::slots_per_epoch() {
        test(&state, Slot::from(i), 0);
    }
}

#[tokio::test]
async fn beacon_proposer_index() {
    test_beacon_proposer_index::<MinimalEthSpec>().await;
}

/// Test that
///
/// 1. Using the cache before it's built fails.
/// 2. Using the cache after it's build passes.
/// 3. Using the cache after it's dropped fails.
fn test_cache_initialization<E: EthSpec>(
    state: &mut BeaconState<E>,
    relative_epoch: RelativeEpoch,
    spec: &ChainSpec,
) {
    let slot = relative_epoch
        .into_epoch(state.slot().epoch(E::slots_per_epoch()))
        .start_slot(E::slots_per_epoch());

    // Build the cache.
    state.build_committee_cache(relative_epoch, spec).unwrap();

    // Assert a call to a cache-using function passes.
    state.get_beacon_committee(slot, 0).unwrap();

    // Drop the cache.
    state.drop_committee_cache(relative_epoch).unwrap();

    // Assert a call to a cache-using function fail.
    assert_eq!(
        state.get_beacon_committee(slot, 0),
        Err(BeaconStateError::CommitteeCacheUninitialized(Some(
            relative_epoch
        )))
    );
}

#[tokio::test]
async fn cache_initialization() {
    let spec = MinimalEthSpec::default_spec();

    let mut state = build_state::<MinimalEthSpec>(16).await;

    *state.slot_mut() =
        (MinimalEthSpec::genesis_epoch() + 1).start_slot(MinimalEthSpec::slots_per_epoch());

    test_cache_initialization(&mut state, RelativeEpoch::Previous, &spec);
    test_cache_initialization(&mut state, RelativeEpoch::Current, &spec);
    test_cache_initialization(&mut state, RelativeEpoch::Next, &spec);
}

/// Tests committee-specific components
#[cfg(test)]
mod committees {
    use super::*;
    use std::ops::{Add, Div};
    use swap_or_not_shuffle::shuffle_list;

    fn execute_committee_consistency_test<E: EthSpec>(
        state: BeaconState<E>,
        epoch: Epoch,
        validator_count: usize,
        spec: &ChainSpec,
    ) {
        let active_indices: Vec<usize> = (0..validator_count).collect();
        let seed = state.get_seed(epoch, Domain::BeaconAttester, spec).unwrap();
        let relative_epoch = RelativeEpoch::from_epoch(state.current_epoch(), epoch).unwrap();

        let mut ordered_indices = state
            .get_cached_active_validator_indices(relative_epoch)
            .unwrap()
            .to_vec();
        ordered_indices.sort_unstable();
        assert_eq!(
            active_indices, ordered_indices,
            "Validator indices mismatch"
        );

        let shuffling =
            shuffle_list(active_indices, spec.shuffle_round_count, &seed[..], false).unwrap();

        let mut expected_indices_iter = shuffling.iter();

        // Loop through all slots in the epoch being tested.
        for slot in epoch.slot_iter(E::slots_per_epoch()) {
            let beacon_committees = state.get_beacon_committees_at_slot(slot).unwrap();

            // Assert that the number of committees in this slot is consistent with the reported number
            // of committees in an epoch.
            assert_eq!(
                beacon_committees.len() as u64,
                state
                    .get_epoch_committee_count(relative_epoch)
                    .unwrap()
                    .div(E::slots_per_epoch())
            );

            for (committee_index, bc) in beacon_committees.iter().enumerate() {
                // Assert that indices are assigned sequentially across committees.
                assert_eq!(committee_index as u64, bc.index);
                // Assert that a committee lookup via slot is identical to a committee lookup via
                // index.
                assert_eq!(state.get_beacon_committee(bc.slot, bc.index).unwrap(), *bc);

                // Loop through each validator in the committee.
                for (committee_i, validator_i) in bc.committee.iter().enumerate() {
                    // Assert the validators are assigned contiguously across committees.
                    assert_eq!(
                        *validator_i,
                        *expected_indices_iter.next().unwrap(),
                        "Non-sequential validators."
                    );
                    // Assert a call to `get_attestation_duties` is consistent with a call to
                    // `get_beacon_committees_at_slot`
                    let attestation_duty = state
                        .get_attestation_duties(*validator_i, relative_epoch)
                        .unwrap()
                        .unwrap();
                    assert_eq!(attestation_duty.slot, slot);
                    assert_eq!(attestation_duty.index, bc.index);
                    assert_eq!(attestation_duty.committee_position, committee_i);
                    assert_eq!(attestation_duty.committee_len, bc.committee.len());
                }
            }
        }

        // Assert that all validators were assigned to a committee.
        assert!(expected_indices_iter.next().is_none());
    }

    async fn committee_consistency_test<E: EthSpec>(
        validator_count: usize,
        state_epoch: Epoch,
        cache_epoch: RelativeEpoch,
    ) {
        let spec = &E::default_spec();

        let slot = state_epoch.start_slot(E::slots_per_epoch());
        let harness = get_harness::<E>(validator_count, slot).await;
        let mut new_head_state = harness.get_current_state();

        let distinct_hashes =
            (0..E::epochs_per_historical_vector()).map(|i| Hash256::from_low_u64_be(i as u64));
        *new_head_state.randao_mixes_mut() = Vector::try_from_iter(distinct_hashes).unwrap();

        new_head_state
            .force_build_committee_cache(RelativeEpoch::Previous, spec)
            .unwrap();
        new_head_state
            .force_build_committee_cache(RelativeEpoch::Current, spec)
            .unwrap();
        new_head_state
            .force_build_committee_cache(RelativeEpoch::Next, spec)
            .unwrap();

        let cache_epoch = cache_epoch.into_epoch(state_epoch);

        execute_committee_consistency_test(new_head_state, cache_epoch, validator_count, spec);
    }

    async fn committee_consistency_test_suite<E: EthSpec>(cached_epoch: RelativeEpoch) {
        let spec = E::default_spec();

        let validator_count = spec
            .max_committees_per_slot
            .mul(E::slots_per_epoch() as usize)
            .mul(spec.target_committee_size)
            .add(1);

        committee_consistency_test::<E>(validator_count, Epoch::new(0), cached_epoch).await;

        committee_consistency_test::<E>(validator_count, E::genesis_epoch() + 4, cached_epoch)
            .await;

        committee_consistency_test::<E>(
            validator_count,
            E::genesis_epoch()
                + (E::slots_per_historical_root() as u64)
                    .mul(E::slots_per_epoch())
                    .mul(4),
            cached_epoch,
        )
        .await;
    }

    #[tokio::test]
    async fn current_epoch_committee_consistency() {
        committee_consistency_test_suite::<MinimalEthSpec>(RelativeEpoch::Current).await;
    }

    #[tokio::test]
    async fn previous_epoch_committee_consistency() {
        committee_consistency_test_suite::<MinimalEthSpec>(RelativeEpoch::Previous).await;
    }

    #[tokio::test]
    async fn next_epoch_committee_consistency() {
        committee_consistency_test_suite::<MinimalEthSpec>(RelativeEpoch::Next).await;
    }
}

/// Tests for BeaconStateGloas — Gloas-specific fields and accessor behavior.
mod gloas {
    use crate::*;
    use ssz::Encode;
    use ssz_types::BitVector;
    use std::sync::Arc;

    type E = MinimalEthSpec;

    /// Construct a minimal `BeaconStateGloas` for testing.
    fn make_gloas_state() -> BeaconState<E> {
        let spec = E::default_spec();
        let slot = Slot::new(E::slots_per_epoch());
        let epoch = slot.epoch(E::slots_per_epoch());
        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

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
                parent_root: Hash256::zero(),
                state_root: Hash256::zero(),
                body_root: Hash256::zero(),
            },
            block_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            state_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            historical_roots: List::default(),
            eth1_data: Eth1Data::default(),
            eth1_data_votes: List::default(),
            eth1_deposit_index: 0,
            validators: List::default(),
            balances: List::default(),
            randao_mixes: Vector::new(vec![Hash256::zero(); epochs_per_vector]).unwrap(),
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
            builders: List::new(vec![Builder {
                pubkey: PublicKeyBytes::empty(),
                version: 0x03,
                execution_address: Address::repeat_byte(0xBB),
                balance: 64_000_000_000,
                deposit_epoch: Epoch::new(0),
                withdrawable_epoch: Epoch::new(u64::MAX),
            }])
            .unwrap(),
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

    // --- Fork name ---

    #[test]
    fn fork_name_is_gloas() {
        let state = make_gloas_state();
        assert_eq!(state.fork_name_unchecked(), ForkName::Gloas);
    }

    // --- Gloas-only field accessors ---

    #[test]
    fn latest_execution_payload_bid_accessible() {
        let state = make_gloas_state();
        let bid = state.latest_execution_payload_bid().unwrap();
        assert_eq!(*bid, ExecutionPayloadBid::default());
    }

    #[test]
    fn latest_execution_payload_bid_mut_accessible() {
        let mut state = make_gloas_state();
        let bid = state.latest_execution_payload_bid_mut().unwrap();
        bid.builder_index = 42;
        assert_eq!(
            state.latest_execution_payload_bid().unwrap().builder_index,
            42
        );
    }

    #[test]
    fn builders_accessible() {
        let state = make_gloas_state();
        let builders = state.builders().unwrap();
        assert_eq!(builders.len(), 1);
        assert_eq!(builders.get(0).unwrap().balance, 64_000_000_000);
    }

    #[test]
    fn next_withdrawal_builder_index_accessible() {
        let state = make_gloas_state();
        assert_eq!(state.next_withdrawal_builder_index().unwrap(), 0);
    }

    #[test]
    fn execution_payload_availability_accessible() {
        let state = make_gloas_state();
        let bits = state.execution_payload_availability().unwrap();
        // All bits set to 1 in our test state
        assert_eq!(
            bits.num_set_bits(),
            <E as EthSpec>::SlotsPerHistoricalRoot::to_usize()
        );
    }

    #[test]
    fn builder_pending_payments_accessible() {
        let state = make_gloas_state();
        let payments = state.builder_pending_payments().unwrap();
        assert_eq!(payments.len(), E::builder_pending_payments_limit());
    }

    #[test]
    fn builder_pending_withdrawals_accessible() {
        let state = make_gloas_state();
        let withdrawals = state.builder_pending_withdrawals().unwrap();
        assert!(withdrawals.is_empty());
    }

    #[test]
    fn latest_block_hash_accessible() {
        let state = make_gloas_state();
        assert_eq!(
            *state.latest_block_hash().unwrap(),
            ExecutionBlockHash::zero()
        );
    }

    #[test]
    fn payload_expected_withdrawals_accessible() {
        let state = make_gloas_state();
        let withdrawals = state.payload_expected_withdrawals().unwrap();
        assert!(withdrawals.is_empty());
    }

    // --- latest_execution_payload_header returns Err for Gloas ---

    #[test]
    fn latest_execution_payload_header_returns_err() {
        let state = make_gloas_state();
        assert!(
            state.latest_execution_payload_header().is_err(),
            "Gloas has no latest_execution_payload_header (replaced by bid)"
        );
    }

    #[test]
    fn latest_execution_payload_header_mut_returns_err() {
        let mut state = make_gloas_state();
        assert!(
            state.latest_execution_payload_header_mut().is_err(),
            "Gloas has no latest_execution_payload_header_mut (replaced by bid)"
        );
    }

    // --- Gloas-only fields return Err on non-Gloas states ---

    #[test]
    fn gloas_fields_err_on_base_state() {
        use crate::test_utils::{SeedableRng, TestRandom, XorShiftRng};
        let rng = &mut XorShiftRng::from_seed([42; 16]);
        let state: BeaconState<E> = BeaconState::Base(BeaconStateBase::random_for_test(rng));

        assert!(state.latest_execution_payload_bid().is_err());
        assert!(state.builders().is_err());
        assert!(state.next_withdrawal_builder_index().is_err());
        assert!(state.execution_payload_availability().is_err());
        assert!(state.builder_pending_payments().is_err());
        assert!(state.builder_pending_withdrawals().is_err());
        assert!(state.latest_block_hash().is_err());
        assert!(state.payload_expected_withdrawals().is_err());
    }

    // --- Mutability ---

    #[test]
    fn latest_block_hash_mut() {
        let mut state = make_gloas_state();
        *state.latest_block_hash_mut().unwrap() = ExecutionBlockHash::repeat_byte(0xDD);
        assert_eq!(
            *state.latest_block_hash().unwrap(),
            ExecutionBlockHash::repeat_byte(0xDD)
        );
    }

    #[test]
    fn builders_mut() {
        let mut state = make_gloas_state();
        state.builders_mut().unwrap().get_mut(0).unwrap().balance = 999;
        assert_eq!(state.builders().unwrap().get(0).unwrap().balance, 999);
    }

    #[test]
    fn execution_payload_availability_mut() {
        let mut state = make_gloas_state();
        let bits = state.execution_payload_availability_mut().unwrap();
        bits.set(0, false).unwrap();
        assert!(
            !state
                .execution_payload_availability()
                .unwrap()
                .get(0)
                .unwrap()
        );
    }

    // --- SSZ roundtrip ---

    #[test]
    fn ssz_roundtrip() {
        let spec = ForkName::Gloas.make_genesis_spec(E::default_spec());
        let state = make_gloas_state();
        let bytes = state.as_ssz_bytes();
        let decoded = BeaconState::<E>::from_ssz_bytes(&bytes, &spec).unwrap();
        assert_eq!(state, decoded);
    }

    // --- Tree hash (via canonical_root / get_beacon_state_leaves) ---

    #[test]
    fn canonical_root_deterministic() {
        let mut state = make_gloas_state();
        let root1 = state.canonical_root().unwrap();
        let root2 = state.canonical_root().unwrap();
        assert_eq!(root1, root2);
        assert_ne!(root1, Hash256::ZERO);
    }

    #[test]
    fn canonical_root_changes_with_bid() {
        let mut state1 = make_gloas_state();
        let mut state2 = make_gloas_state();

        state1.latest_execution_payload_bid_mut().unwrap().value = 1000;
        assert_ne!(
            state1.canonical_root().unwrap(),
            state2.canonical_root().unwrap()
        );
    }

    #[test]
    fn leaves_change_with_latest_block_hash() {
        let mut state1 = make_gloas_state();
        let state2 = make_gloas_state();

        *state1.latest_block_hash_mut().unwrap() = ExecutionBlockHash::repeat_byte(0xEE);
        assert_ne!(
            state1.get_beacon_state_leaves(),
            state2.get_beacon_state_leaves()
        );
    }

    #[test]
    fn leaves_nonempty() {
        let state = make_gloas_state();
        let leaves = state.get_beacon_state_leaves();
        assert!(!leaves.is_empty());
    }

    // --- Clone ---

    #[test]
    fn clone_preserves_equality() {
        let state = make_gloas_state();
        let cloned = state.clone();
        assert_eq!(state, cloned);
    }

    // --- Shared field accessors still work ---

    #[test]
    fn slot_accessor() {
        let state = make_gloas_state();
        assert_eq!(state.slot(), Slot::new(E::slots_per_epoch()));
    }

    #[test]
    fn fork_accessor() {
        let spec = E::default_spec();
        let state = make_gloas_state();
        assert_eq!(state.fork().current_version, spec.gloas_fork_version);
        assert_eq!(state.fork().previous_version, spec.fulu_fork_version);
    }

    #[test]
    fn proposer_lookahead_accessible() {
        let state = make_gloas_state();
        let lookahead = state.proposer_lookahead().unwrap();
        assert_eq!(
            lookahead.len(),
            <E as EthSpec>::ProposerLookaheadSlots::to_usize()
        );
    }
}

#[test]
fn decode_base_and_altair() {
    type E = MainnetEthSpec;
    let spec = E::default_spec();

    let rng = &mut XorShiftRng::from_seed([42; 16]);

    let fork_epoch = spec.altair_fork_epoch.unwrap();

    let base_epoch = fork_epoch.saturating_sub(1_u64);
    let base_slot = base_epoch.end_slot(E::slots_per_epoch());
    let altair_epoch = fork_epoch;
    let altair_slot = altair_epoch.start_slot(E::slots_per_epoch());

    // BeaconStateBase
    {
        let good_base_state: BeaconState<MainnetEthSpec> = BeaconState::Base(BeaconStateBase {
            slot: base_slot,
            ..<_>::random_for_test(rng)
        });
        // It's invalid to have a base state with a slot higher than the fork slot.
        let bad_base_state = {
            let mut bad = good_base_state.clone();
            *bad.slot_mut() = altair_slot;
            bad
        };

        assert_eq!(
            BeaconState::from_ssz_bytes(&good_base_state.as_ssz_bytes(), &spec)
                .expect("good base state can be decoded"),
            good_base_state
        );
        <BeaconState<MainnetEthSpec>>::from_ssz_bytes(&bad_base_state.as_ssz_bytes(), &spec)
            .expect_err("bad base state cannot be decoded");
    }

    // BeaconStateAltair
    {
        let good_altair_state: BeaconState<MainnetEthSpec> =
            BeaconState::Altair(BeaconStateAltair {
                slot: altair_slot,
                ..<_>::random_for_test(rng)
            });
        // It's invalid to have an Altair state with a slot lower than the fork slot.
        let bad_altair_state = {
            let mut bad = good_altair_state.clone();
            *bad.slot_mut() = base_slot;
            bad
        };

        assert_eq!(
            BeaconState::from_ssz_bytes(&good_altair_state.as_ssz_bytes(), &spec)
                .expect("good altair state can be decoded"),
            good_altair_state
        );
        <BeaconState<MainnetEthSpec>>::from_ssz_bytes(&bad_altair_state.as_ssz_bytes(), &spec)
            .expect_err("bad altair state cannot be decoded");
    }
}
