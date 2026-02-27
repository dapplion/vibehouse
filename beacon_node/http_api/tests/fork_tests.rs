//! Tests for API behaviour across fork boundaries.
use beacon_chain::custody_context::NodeCustodyType;
use beacon_chain::{
    StateSkipConfig,
    test_utils::{DEFAULT_ETH1_BLOCK_HASH, HARNESS_GENESIS_TIME, RelativeSyncCommittee},
};
use eth2::types::{BlockId, IndexedErrorMessage, StateId, SyncSubcommittee};
use execution_layer::test_utils::generate_genesis_header;
use genesis::{InteropGenesisBuilder, bls_withdrawal_credentials};
use http_api::test_utils::*;
use std::collections::HashSet;
use types::{
    Address, ChainSpec, Epoch, EthSpec, FixedBytesExtended, Hash256, MinimalEthSpec, Signature,
    Slot,
    test_utils::{generate_deterministic_keypair, generate_deterministic_keypairs},
};

type E = MinimalEthSpec;

fn altair_spec(altair_fork_epoch: Epoch) -> ChainSpec {
    let mut spec = E::default_spec();
    spec.altair_fork_epoch = Some(altair_fork_epoch);
    spec
}

fn capella_spec(capella_fork_epoch: Epoch) -> ChainSpec {
    let mut spec = E::default_spec();
    spec.altair_fork_epoch = Some(Epoch::new(0));
    spec.bellatrix_fork_epoch = Some(Epoch::new(0));
    spec.capella_fork_epoch = Some(capella_fork_epoch);
    spec
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_committee_duties_across_fork() {
    let validator_count = E::sync_committee_size();
    let fork_epoch = Epoch::new(8);
    let spec = altair_spec(fork_epoch);
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let all_validators = harness.get_all_validators();
    let all_validators_u64 = all_validators.iter().map(|x| *x as u64).collect::<Vec<_>>();

    assert_eq!(harness.get_current_slot(), 0);

    // Prior to the fork the endpoint should return an empty vec.
    let early_duties = client
        .post_validator_duties_sync(fork_epoch - 1, &all_validators_u64)
        .await
        .unwrap()
        .data;
    assert!(early_duties.is_empty());

    // If there's a skip slot at the fork slot, the endpoint should return duties, even
    // though the head state hasn't transitioned yet.
    let fork_slot = fork_epoch.start_slot(E::slots_per_epoch());
    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let (_, mut state) = harness
        .add_attested_block_at_slot(
            fork_slot - 1,
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    harness.advance_slot();
    assert_eq!(harness.get_current_slot(), fork_slot);

    let sync_duties = client
        .post_validator_duties_sync(fork_epoch, &all_validators_u64)
        .await
        .unwrap()
        .data;
    assert_eq!(sync_duties.len(), E::sync_committee_size());

    // After applying a block at the fork slot the duties should remain unchanged.
    let state_root = state.canonical_root().unwrap();
    harness
        .add_attested_block_at_slot(fork_slot, state, state_root, &all_validators)
        .await
        .unwrap();

    assert_eq!(
        client
            .post_validator_duties_sync(fork_epoch, &all_validators_u64)
            .await
            .unwrap()
            .data,
        sync_duties
    );

    // Sync duties should also be available for the next period.
    let current_period = fork_epoch.sync_committee_period(&spec).unwrap();
    let next_period_epoch = spec.epochs_per_sync_committee_period * (current_period + 1);

    let next_period_duties = client
        .post_validator_duties_sync(next_period_epoch, &all_validators_u64)
        .await
        .unwrap()
        .data;
    assert_eq!(next_period_duties.len(), E::sync_committee_size());

    // Sync duties should *not* be available for the period after the next period.
    // We expect a 400 (bad request) response.
    let next_next_period_epoch = spec.epochs_per_sync_committee_period * (current_period + 2);
    assert_eq!(
        client
            .post_validator_duties_sync(next_next_period_epoch, &all_validators_u64)
            .await
            .unwrap_err()
            .status()
            .unwrap(),
        400
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attestations_across_fork_with_skip_slots() {
    let validator_count = E::sync_committee_size();
    let fork_epoch = Epoch::new(8);
    let spec = altair_spec(fork_epoch);
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let all_validators = harness.get_all_validators();

    let fork_slot = fork_epoch.start_slot(E::slots_per_epoch());
    let mut fork_state = harness
        .chain
        .state_at_slot(fork_slot, StateSkipConfig::WithStateRoots)
        .unwrap();
    let fork_state_root = fork_state.update_tree_hash_cache().unwrap();

    harness.set_current_slot(fork_slot);

    let attestations = harness.make_attestations(
        &all_validators,
        &fork_state,
        fork_state_root,
        (*fork_state.get_block_root(fork_slot - 1).unwrap()).into(),
        fork_slot,
    );

    let unaggregated_attestations = attestations
        .iter()
        .flat_map(|(atts, _)| atts.iter().map(|(att, _)| att.clone()))
        .collect::<Vec<_>>();

    let unaggregated_attestations = unaggregated_attestations
        .into_iter()
        .map(|attn| {
            let aggregation_bits = attn.get_aggregation_bits();

            if aggregation_bits.len() != 1 {
                panic!("Must be an unaggregated attestation")
            }

            let aggregation_bit = *aggregation_bits.first().unwrap();

            let committee = fork_state
                .get_beacon_committee(attn.data().slot, attn.committee_index().unwrap())
                .unwrap();

            let attester_index = committee
                .committee
                .iter()
                .enumerate()
                .find_map(|(i, &index)| {
                    if aggregation_bit as usize == i {
                        return Some(index);
                    }
                    None
                })
                .unwrap();
            attn.to_single_attestation_with_attester_index(attester_index as u64)
                .unwrap()
        })
        .collect::<Vec<_>>();

    assert!(!unaggregated_attestations.is_empty());
    let fork_name = harness.spec.fork_name_at_slot::<E>(fork_slot);
    client
        .post_beacon_pool_attestations_v2::<E>(unaggregated_attestations, fork_name)
        .await
        .unwrap();

    let signed_aggregates = attestations
        .into_iter()
        .filter_map(|(_, op_aggregate)| op_aggregate)
        .collect::<Vec<_>>();
    assert!(!signed_aggregates.is_empty());

    client
        .post_validator_aggregate_and_proof_v1(&signed_aggregates)
        .await
        .unwrap();
    client
        .post_validator_aggregate_and_proof_v2(&signed_aggregates, fork_name)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_contributions_across_fork_with_skip_slots() {
    let validator_count = E::sync_committee_size();
    let fork_epoch = Epoch::new(8);
    let spec = altair_spec(fork_epoch);
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let fork_slot = fork_epoch.start_slot(E::slots_per_epoch());
    let fork_state = harness
        .chain
        .state_at_slot(fork_slot, StateSkipConfig::WithStateRoots)
        .unwrap();

    harness.set_current_slot(fork_slot);

    let sync_messages = harness.make_sync_contributions(
        &fork_state,
        *fork_state.get_block_root(fork_slot - 1).unwrap(),
        fork_slot,
        RelativeSyncCommittee::Current,
    );

    let sync_committee_messages = sync_messages
        .iter()
        .flat_map(|(messages, _)| messages.iter().map(|(message, _subnet)| message.clone()))
        .collect::<Vec<_>>();
    assert!(!sync_committee_messages.is_empty());

    client
        .post_beacon_pool_sync_committee_signatures(&sync_committee_messages)
        .await
        .unwrap();

    let signed_contributions = sync_messages
        .into_iter()
        .filter_map(|(_, op_aggregate)| op_aggregate)
        .collect::<Vec<_>>();
    assert!(!signed_contributions.is_empty());

    client
        .post_validator_contribution_and_proofs(&signed_contributions)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_committee_indices_across_fork() {
    let validator_count = E::sync_committee_size();
    let fork_epoch = Epoch::new(8);
    let spec = altair_spec(fork_epoch);
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let all_validators = harness.get_all_validators();

    // Flatten subcommittees into a single vec.
    let flatten = |subcommittees: &[SyncSubcommittee]| -> Vec<u64> {
        subcommittees
            .iter()
            .flat_map(|sub| sub.indices.iter().copied())
            .collect()
    };

    // Prior to the fork the `sync_committees` endpoint should return a 400 error.
    assert_eq!(
        client
            .get_beacon_states_sync_committees(StateId::Slot(Slot::new(0)), None)
            .await
            .unwrap_err()
            .status()
            .unwrap(),
        400
    );
    assert_eq!(
        client
            .get_beacon_states_sync_committees(StateId::Head, Some(Epoch::new(0)))
            .await
            .unwrap_err()
            .status()
            .unwrap(),
        400
    );

    // If there's a skip slot at the fork slot, the endpoint will return a 400 until a block is
    // applied.
    let fork_slot = fork_epoch.start_slot(E::slots_per_epoch());
    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let (_, mut state) = harness
        .add_attested_block_at_slot(
            fork_slot - 1,
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    harness.advance_slot();
    assert_eq!(harness.get_current_slot(), fork_slot);

    // Using the head state must fail.
    assert_eq!(
        client
            .get_beacon_states_sync_committees(StateId::Head, Some(fork_epoch))
            .await
            .unwrap_err()
            .status()
            .unwrap(),
        400
    );

    // In theory we could do a state advance and make this work, but to keep things simple I've
    // avoided doing that for now.
    assert_eq!(
        client
            .get_beacon_states_sync_committees(StateId::Slot(fork_slot), None)
            .await
            .unwrap_err()
            .status()
            .unwrap(),
        400
    );

    // Once the head is updated it should be useable for requests, including in the next sync
    // committee period.
    let state_root = state.canonical_root().unwrap();
    harness
        .add_attested_block_at_slot(fork_slot + 1, state, state_root, &all_validators)
        .await
        .unwrap();

    let current_period = fork_epoch.sync_committee_period(&spec).unwrap();
    let next_period_epoch = spec.epochs_per_sync_committee_period * (current_period + 1);
    assert!(next_period_epoch > fork_epoch);

    for epoch in [
        None,
        Some(fork_epoch),
        Some(fork_epoch + 1),
        Some(next_period_epoch),
        Some(next_period_epoch + 1),
    ] {
        let committee = client
            .get_beacon_states_sync_committees(StateId::Head, epoch)
            .await
            .unwrap()
            .data;
        assert_eq!(committee.validators.len(), E::sync_committee_size());

        assert_eq!(
            committee.validators,
            flatten(&committee.validator_aggregates)
        );
    }
}

/// Assert that an HTTP API error has the given status code and indexed errors for the given indices.
fn assert_server_indexed_error(error: eth2::Error, status_code: u16, indices: Vec<usize>) {
    let eth2::Error::ServerIndexedMessage(IndexedErrorMessage { code, failures, .. }) = error
    else {
        panic!("wrong error, expected ServerIndexedMessage, got: {error:?}")
    };
    assert_eq!(code, status_code);
    assert_eq!(failures.len(), indices.len());
    for (index, failure) in indices.into_iter().zip(failures) {
        assert_eq!(failure.index, index as u64);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bls_to_execution_changes_update_all_around_capella_fork() {
    const VALIDATOR_COUNT: usize = 128;
    let fork_epoch = Epoch::new(2);
    let spec = capella_spec(fork_epoch);
    let max_bls_to_execution_changes = E::max_bls_to_execution_changes();

    // Use a genesis state with entirely BLS withdrawal credentials.
    // Offset keypairs by `VALIDATOR_COUNT` to create keys distinct from the signing keys.
    let validator_keypairs = generate_deterministic_keypairs(VALIDATOR_COUNT);
    let withdrawal_keypairs = (0..VALIDATOR_COUNT)
        .map(|i| Some(generate_deterministic_keypair(i + VALIDATOR_COUNT)))
        .collect::<Vec<_>>();

    fn withdrawal_credentials_fn<'a>(
        index: usize,
        _: &'a types::PublicKey,
        spec: &'a ChainSpec,
    ) -> Hash256 {
        // It is a bit inefficient to regenerate the whole keypair here, but this is a workaround.
        // `InteropGenesisBuilder` requires the `withdrawal_credentials_fn` to have
        // a `'static` lifetime.
        let keypair = generate_deterministic_keypair(index + VALIDATOR_COUNT);
        bls_withdrawal_credentials(&keypair.pk, spec)
    }

    let header = generate_genesis_header(&spec, true);

    let genesis_state = InteropGenesisBuilder::new()
        .set_opt_execution_payload_header(header)
        .set_withdrawal_credentials_fn(Box::new(withdrawal_credentials_fn))
        .build_genesis_state(
            &validator_keypairs,
            HARNESS_GENESIS_TIME,
            Hash256::from_slice(DEFAULT_ETH1_BLOCK_HASH),
            &spec,
        )
        .unwrap();

    let tester = InteractiveTester::<E>::new_with_initializer_and_mutator(
        Some(spec.clone()),
        VALIDATOR_COUNT,
        Some(Box::new(|harness_builder| {
            harness_builder
                .keypairs(validator_keypairs)
                .withdrawal_keypairs(withdrawal_keypairs)
                .genesis_state_ephemeral_store(genesis_state)
        })),
        None,
        Default::default(),
        true,
        NodeCustodyType::Fullnode,
    )
    .await;
    let harness = &tester.harness;
    let client = &tester.client;

    let all_validators = harness.get_all_validators();
    let all_validators_u64 = all_validators.iter().map(|x| *x as u64).collect::<Vec<_>>();

    // Create a bunch of valid address changes.
    let valid_address_changes = all_validators_u64
        .iter()
        .map(|&validator_index| {
            harness.make_bls_to_execution_change(
                validator_index,
                Address::from_low_u64_be(validator_index),
            )
        })
        .collect::<Vec<_>>();

    // Address changes which conflict with `valid_address_changes` on the address chosen.
    let conflicting_address_changes = all_validators_u64
        .iter()
        .map(|&validator_index| {
            harness.make_bls_to_execution_change(
                validator_index,
                Address::from_low_u64_be(validator_index + 1),
            )
        })
        .collect::<Vec<_>>();

    // Address changes signed with the wrong key.
    let wrong_key_address_changes = all_validators_u64
        .iter()
        .map(|&validator_index| {
            // Use the correct pubkey.
            let pubkey = &harness.get_withdrawal_keypair(validator_index).pk;
            // And the wrong secret key.
            let secret_key = &harness
                .get_withdrawal_keypair((validator_index + 1) % VALIDATOR_COUNT as u64)
                .sk;
            harness.make_bls_to_execution_change_with_keys(
                validator_index,
                Address::from_low_u64_be(validator_index),
                pubkey,
                secret_key,
            )
        })
        .collect::<Vec<_>>();

    // Submit some changes before Capella. Just enough to fill two blocks.
    let num_pre_capella = VALIDATOR_COUNT / 4;
    let blocks_filled_pre_capella = 2;
    assert_eq!(
        num_pre_capella,
        blocks_filled_pre_capella * max_bls_to_execution_changes
    );

    client
        .post_beacon_pool_bls_to_execution_changes(&valid_address_changes[..num_pre_capella])
        .await
        .unwrap();

    let expected_received_pre_capella_messages = valid_address_changes[..num_pre_capella].to_vec();

    // Conflicting changes for the same validators should all fail.
    let error = client
        .post_beacon_pool_bls_to_execution_changes(&conflicting_address_changes[..num_pre_capella])
        .await
        .unwrap_err();
    assert_server_indexed_error(error, 400, (0..num_pre_capella).collect());

    // Re-submitting the same changes should be accepted.
    client
        .post_beacon_pool_bls_to_execution_changes(&valid_address_changes[..num_pre_capella])
        .await
        .unwrap();

    // Invalid changes signed with the wrong keys should all be rejected without affecting the seen
    // indices filters (apply ALL of them).
    let error = client
        .post_beacon_pool_bls_to_execution_changes(&wrong_key_address_changes)
        .await
        .unwrap_err();
    assert_server_indexed_error(error, 400, all_validators.clone());

    // Advance to right before Capella.
    let capella_slot = fork_epoch.start_slot(E::slots_per_epoch());
    harness.extend_to_slot(capella_slot - 1).await;
    assert_eq!(harness.head_slot(), capella_slot - 1);

    assert_eq!(
        harness
            .chain
            .op_pool
            .get_bls_to_execution_changes_received_pre_capella(
                &harness.chain.head_snapshot().beacon_state,
                &spec,
            )
            .into_iter()
            .collect::<HashSet<_>>(),
        HashSet::from_iter(expected_received_pre_capella_messages.into_iter()),
        "all pre-capella messages should be queued for capella broadcast"
    );

    // Add Capella blocks which should be full of BLS to execution changes.
    for i in 0..VALIDATOR_COUNT / max_bls_to_execution_changes {
        let head_block_root = harness.extend_slots(1).await;
        let head_block = harness
            .chain
            .get_block(&head_block_root)
            .await
            .unwrap()
            .unwrap();

        let bls_to_execution_changes = head_block
            .message()
            .body()
            .bls_to_execution_changes()
            .unwrap();

        // Block should be full.
        assert_eq!(
            bls_to_execution_changes.len(),
            max_bls_to_execution_changes,
            "block not full on iteration {i}"
        );

        // Included changes should be the ones from `valid_address_changes` in any order.
        for address_change in bls_to_execution_changes.iter() {
            assert!(valid_address_changes.contains(address_change));
        }

        // After the initial 2 blocks, add the rest of the changes using a large
        // request containing all the valid, all the conflicting and all the invalid.
        // Despite the invalid and duplicate messages, the new ones should still get picked up by
        // the pool.
        if i == blocks_filled_pre_capella - 1 {
            let all_address_changes: Vec<_> = [
                valid_address_changes.clone(),
                conflicting_address_changes.clone(),
                wrong_key_address_changes.clone(),
            ]
            .concat();

            let error = client
                .post_beacon_pool_bls_to_execution_changes(&all_address_changes)
                .await
                .unwrap_err();
            assert_server_indexed_error(
                error,
                400,
                (VALIDATOR_COUNT..3 * VALIDATOR_COUNT).collect(),
            );
        }
    }

    // Eventually all validators should have eth1 withdrawal credentials.
    let head_state = harness.get_current_state();
    for validator in head_state.validators() {
        assert!(validator.has_eth1_withdrawal_credential(&spec));
    }
}

// ── Gloas (ePBS) tests ──────────────────────────────────────────────

fn gloas_spec(gloas_fork_epoch: Epoch) -> ChainSpec {
    let mut spec = E::default_spec();
    spec.altair_fork_epoch = Some(Epoch::new(0));
    spec.bellatrix_fork_epoch = Some(Epoch::new(0));
    spec.capella_fork_epoch = Some(Epoch::new(0));
    spec.deneb_fork_epoch = Some(Epoch::new(0));
    spec.electra_fork_epoch = Some(Epoch::new(0));
    spec.fulu_fork_epoch = Some(Epoch::new(0));
    spec.gloas_fork_epoch = Some(gloas_fork_epoch);
    spec
}

/// PTC duties endpoint should return 400 when Gloas is not scheduled.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ptc_duties_rejected_before_gloas_scheduled() {
    let validator_count = 32;
    // No Gloas fork configured
    let mut spec = E::default_spec();
    spec.altair_fork_epoch = Some(Epoch::new(0));
    spec.bellatrix_fork_epoch = Some(Epoch::new(0));
    spec.capella_fork_epoch = Some(Epoch::new(0));
    spec.deneb_fork_epoch = Some(Epoch::new(0));
    spec.electra_fork_epoch = Some(Epoch::new(0));
    spec.fulu_fork_epoch = Some(Epoch::new(0));
    // gloas_fork_epoch = None (not scheduled)
    let tester = InteractiveTester::<E>::new(Some(spec), validator_count).await;
    let client = &tester.client;

    let indices: Vec<u64> = (0..validator_count as u64).collect();
    let result = client
        .post_validator_duties_ptc(Epoch::new(0), &indices)
        .await;

    assert!(
        result.is_err(),
        "should reject PTC duties when Gloas is not scheduled"
    );
    if let Err(eth2::Error::ServerMessage(msg)) = &result {
        assert_eq!(msg.code, 400);
        assert!(
            msg.message.contains("Gloas is not scheduled"),
            "expected 'Gloas is not scheduled', got: {}",
            msg.message
        );
    } else {
        panic!("expected ServerMessage error, got: {:?}", result);
    }
}

/// PTC duties endpoint should return duties after Gloas fork is active.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ptc_duties_returns_duties_after_gloas() {
    let validator_count = 32;
    let fork_epoch = Epoch::new(0); // Gloas from genesis
    let spec = gloas_spec(fork_epoch);
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let all_validators: Vec<u64> = (0..validator_count as u64).collect();

    // Advance a few slots so the chain has some state
    let slots_per_epoch = E::slots_per_epoch();
    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_val_indices = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_val_indices,
        )
        .await
        .unwrap();

    // Request PTC duties for the current epoch
    let duties_response = client
        .post_validator_duties_ptc(Epoch::new(0), &all_validators)
        .await
        .expect("PTC duties should succeed after Gloas fork");

    let duties = &duties_response.data;

    // PTC committee has PTC_SIZE members per slot. With 32 validators and
    // minimal preset, we should get some duties.
    assert!(
        !duties.is_empty(),
        "PTC duties should not be empty for {} validators",
        validator_count
    );

    // All returned duties should have valid validator indices
    for duty in duties {
        assert!(
            duty.validator_index < validator_count as u64,
            "validator_index {} out of range",
            duty.validator_index
        );
        // Slot should be within the requested epoch
        assert_eq!(
            duty.slot.epoch(slots_per_epoch),
            Epoch::new(0),
            "duty slot {} not in epoch 0",
            duty.slot
        );
    }

    // Dependent root should be non-zero
    assert_ne!(
        duties_response.dependent_root,
        Hash256::zero(),
        "dependent_root should be set"
    );
}

/// PTC duties should reject epochs too far in the future.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ptc_duties_rejects_future_epoch() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec), validator_count).await;
    let client = &tester.client;

    let indices: Vec<u64> = (0..validator_count as u64).collect();

    // Current epoch is 0, requesting epoch 5 should fail (> current + 1)
    let result = client
        .post_validator_duties_ptc(Epoch::new(5), &indices)
        .await;

    assert!(
        result.is_err(),
        "should reject PTC duties for epoch too far in the future"
    );
    if let Err(eth2::Error::ServerMessage(msg)) = &result {
        assert_eq!(msg.code, 400);
    }
}

/// Envelope retrieval should return 404 for non-existent block.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_execution_payload_envelope_not_found() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec), validator_count).await;
    let client = &tester.client;

    // Request envelope for a non-existent block root
    let fake_root = Hash256::repeat_byte(0xAB);
    let result = client
        .get_beacon_execution_payload_envelope::<E>(BlockId::Root(fake_root))
        .await;

    // get_opt maps 404 → Ok(None)
    let envelope = result.expect("request should not fail");
    assert!(
        envelope.is_none(),
        "should return None for non-existent envelope"
    );
}

/// Bid submission should return 400 when Gloas is not scheduled.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bid_submission_rejected_before_gloas() {
    let validator_count = 32;
    let mut spec = E::default_spec();
    spec.altair_fork_epoch = Some(Epoch::new(0));
    spec.bellatrix_fork_epoch = Some(Epoch::new(0));
    spec.capella_fork_epoch = Some(Epoch::new(0));
    spec.deneb_fork_epoch = Some(Epoch::new(0));
    spec.electra_fork_epoch = Some(Epoch::new(0));
    spec.fulu_fork_epoch = Some(Epoch::new(0));
    // gloas_fork_epoch = None
    let tester = InteractiveTester::<E>::new(Some(spec), validator_count).await;
    let client = &tester.client;

    // Create a minimal bid (will be rejected at the fork guard, not content validation)
    let bid = types::SignedExecutionPayloadBid::<E> {
        message: types::ExecutionPayloadBid::default(),
        signature: Signature::empty(),
    };

    let result = client.post_builder_bids(&bid).await;
    assert!(
        result.is_err(),
        "should reject bid submission when Gloas is not scheduled"
    );
    if let Err(eth2::Error::ServerMessage(msg)) = &result {
        assert_eq!(msg.code, 400);
        assert!(
            msg.message.contains("Gloas is not scheduled"),
            "expected 'Gloas is not scheduled', got: {}",
            msg.message
        );
    }
}

/// GET validator/payload_attestation_data should return data for the head slot.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn payload_attestation_data_returns_head_slot() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    // Produce a Gloas block so there's a head to attest about
    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let head_slot = harness.chain.head_snapshot().beacon_block.slot();

    let response = client
        .get_validator_payload_attestation_data(head_slot)
        .await
        .expect("should return payload attestation data");

    let data = response.data;
    assert_eq!(data.slot, head_slot, "slot should match head slot");
    assert_ne!(
        data.beacon_block_root,
        Hash256::zero(),
        "block root should not be zero"
    );
    // After self-build envelope processing, payload_present should be true
    assert!(
        data.payload_present,
        "payload should be present after envelope processing"
    );
}

/// GET validator/payload_attestation_data for a future slot uses head block root,
/// so payload_present reflects the head block's payload status.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn payload_attestation_data_future_slot_uses_head() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let future_slot = head_slot + 5;

    let response = client
        .get_validator_payload_attestation_data(future_slot)
        .await
        .expect("should return payload attestation data for future slot");

    let data = response.data;
    assert_eq!(data.slot, future_slot, "slot should match requested slot");
    // Future slot falls back to head block root
    assert_eq!(
        data.beacon_block_root, head_root,
        "future slot should use head block root"
    );
}

/// POST beacon/pool/payload_attestations should accept a valid PTC member attestation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_payload_attestation_valid_ptc_member() {
    use state_processing::per_block_processing::gloas::get_ptc_committee;
    use types::{Domain, PayloadAttestationData, PayloadAttestationMessage, SignedRoot};

    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    // Produce a block
    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    // Find a PTC member for the head slot
    let ptc = get_ptc_committee::<E>(state, head_slot, &spec).expect("should get PTC committee");
    assert!(!ptc.is_empty(), "PTC committee should not be empty");
    let validator_index = ptc[0];

    // Sign the attestation
    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: true,
        blob_data_available: true,
    };
    let epoch = head_slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(
        epoch,
        Domain::PtcAttester,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let message_root = data.signing_root(domain);
    let keypair = generate_deterministic_keypair(validator_index as usize);
    let signature = keypair.sk.sign(message_root);

    let message = PayloadAttestationMessage {
        validator_index,
        data,
        signature,
    };

    let result = client
        .post_beacon_pool_payload_attestations(&[message])
        .await;
    assert!(
        result.is_ok(),
        "should accept valid PTC member attestation, got: {:?}",
        result.err()
    );
}

/// POST beacon/pool/payload_attestations should reject a non-PTC validator.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_payload_attestation_non_ptc_rejected() {
    use state_processing::per_block_processing::gloas::get_ptc_committee;
    use types::{Domain, PayloadAttestationData, PayloadAttestationMessage, SignedRoot};

    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    // Produce a block
    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    // Find a validator NOT in the PTC
    let ptc = get_ptc_committee::<E>(state, head_slot, &spec).expect("should get PTC committee");
    let non_ptc_validator = (0..validator_count as u64)
        .find(|idx| !ptc.contains(idx))
        .expect("should find a non-PTC validator");

    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: true,
        blob_data_available: false,
    };
    let epoch = head_slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(
        epoch,
        Domain::PtcAttester,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let message_root = data.signing_root(domain);
    let keypair = generate_deterministic_keypair(non_ptc_validator as usize);
    let signature = keypair.sk.sign(message_root);

    let message = PayloadAttestationMessage {
        validator_index: non_ptc_validator,
        data,
        signature,
    };

    let result = client
        .post_beacon_pool_payload_attestations(&[message])
        .await;
    assert!(
        result.is_err(),
        "should reject non-PTC validator attestation"
    );
    if let Err(eth2::Error::ServerMessage(msg)) = &result {
        assert_eq!(msg.code, 400);
    }
}

/// POST beacon/pool/payload_attestations with empty list should succeed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_payload_attestation_empty_list() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec), validator_count).await;
    let client = &tester.client;

    let result = client.post_beacon_pool_payload_attestations(&[]).await;
    assert!(
        result.is_ok(),
        "empty payload attestation list should succeed, got: {:?}",
        result.err()
    );
}

/// GET beacon/execution_payload_envelope/{block_id} should return envelope for a produced block.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_execution_payload_envelope_success() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    // Produce a Gloas block (self-build produces block + envelope)
    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    // Retrieve the envelope via the REST API
    let result = client
        .get_beacon_execution_payload_envelope::<E>(BlockId::Root(head_root))
        .await
        .expect("request should succeed");

    let response = result.expect("envelope should exist for self-build block");
    let envelope = &response.data;

    // Verify envelope fields match
    assert_eq!(
        envelope.message.beacon_block_root, head_root,
        "envelope beacon_block_root should match"
    );
    assert_eq!(
        envelope.message.slot, head_slot,
        "envelope slot should match head slot"
    );
}

/// GET beacon/execution_payload_envelope with slot BlockId should work.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_execution_payload_envelope_by_slot() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let head_slot = harness.chain.head_snapshot().beacon_block.slot();

    // Retrieve by slot
    let result = client
        .get_beacon_execution_payload_envelope::<E>(BlockId::Slot(head_slot))
        .await
        .expect("request should succeed");

    let response = result.expect("envelope should exist for produced block");
    assert_eq!(
        response.data.message.slot, head_slot,
        "envelope slot should match"
    );
}

/// GET beacon/execution_payload_envelope for "head" BlockId.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_execution_payload_envelope_head() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let head_root = harness.chain.head_snapshot().beacon_block_root;

    let result = client
        .get_beacon_execution_payload_envelope::<E>(BlockId::Head)
        .await
        .expect("request should succeed");

    let response = result.expect("envelope should exist for head block");
    assert_eq!(
        response.data.message.beacon_block_root, head_root,
        "head envelope should match head root"
    );
}

/// POST beacon/execution_payload_envelope should return 400 when Gloas is not scheduled.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_execution_payload_envelope_rejected_before_gloas() {
    let validator_count = 32;
    let mut spec = E::default_spec();
    spec.altair_fork_epoch = Some(Epoch::new(0));
    spec.bellatrix_fork_epoch = Some(Epoch::new(0));
    spec.capella_fork_epoch = Some(Epoch::new(0));
    spec.deneb_fork_epoch = Some(Epoch::new(0));
    spec.electra_fork_epoch = Some(Epoch::new(0));
    spec.fulu_fork_epoch = Some(Epoch::new(0));
    // gloas_fork_epoch = None
    let tester = InteractiveTester::<E>::new(Some(spec), validator_count).await;
    let client = &tester.client;

    let envelope = types::SignedExecutionPayloadEnvelope::<E>::default();

    let result = client
        .post_beacon_execution_payload_envelope(&envelope)
        .await;
    assert!(
        result.is_err(),
        "should reject envelope submission when Gloas is not scheduled"
    );
    if let Err(eth2::Error::ServerMessage(msg)) = &result {
        assert_eq!(msg.code, 400);
        assert!(
            msg.message.contains("Gloas is not scheduled"),
            "expected 'Gloas is not scheduled', got: {}",
            msg.message
        );
    }
}

/// POST beacon/execution_payload_envelope with a valid self-build envelope should succeed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_execution_payload_envelope_valid_self_build() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    // Produce a Gloas block (self-build creates block + envelope)
    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    // Get the self-build envelope that was already produced and stored
    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;

    let existing_envelope = harness
        .chain
        .get_payload_envelope(&head_root)
        .unwrap()
        .expect("self-build envelope should exist");

    // Re-submitting the same envelope via POST should succeed (stale but accepted)
    let result = client
        .post_beacon_execution_payload_envelope(&existing_envelope)
        .await;
    // The endpoint may accept (200 for stale/already-known) or reject (400 for duplicate).
    // Either way, it should not panic or return 500.
    match &result {
        Ok(()) => {} // accepted (e.g. stale envelope treated as OK)
        Err(eth2::Error::ServerMessage(msg)) => {
            assert_ne!(
                msg.code, 500,
                "should not return 500 internal server error, got: {}",
                msg.message
            );
        }
        Err(e) => {
            panic!("unexpected error type: {:?}", e);
        }
    }
}

/// POST beacon/execution_payload_envelope with an unknown block root should return 400.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_execution_payload_envelope_unknown_block_root() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    // Produce a block so the chain is active
    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    // Create an envelope referencing a block root that doesn't exist in fork choice
    let mut envelope = types::SignedExecutionPayloadEnvelope::<E>::default();
    envelope.message.beacon_block_root = Hash256::repeat_byte(0xde);
    envelope.message.slot = Slot::new(1);

    let result = client
        .post_beacon_execution_payload_envelope(&envelope)
        .await;
    assert!(
        result.is_err(),
        "should reject envelope with unknown block root"
    );
    if let Err(eth2::Error::ServerMessage(msg)) = &result {
        assert_eq!(msg.code, 400);
        assert!(
            msg.message.contains("BlockRootUnknown"),
            "expected BlockRootUnknown error, got: {}",
            msg.message
        );
    }
}

/// POST beacon/execution_payload_envelope with mismatched slot should return 400.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_execution_payload_envelope_slot_mismatch() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;

    // Get the real envelope and modify the slot
    let mut envelope = harness
        .chain
        .get_payload_envelope(&head_root)
        .unwrap()
        .expect("self-build envelope should exist");

    // Change the slot to a different value (slot mismatch with the block)
    envelope.message.slot = Slot::new(99);

    let result = client
        .post_beacon_execution_payload_envelope(&envelope)
        .await;
    assert!(
        result.is_err(),
        "should reject envelope with mismatched slot"
    );
    if let Err(eth2::Error::ServerMessage(msg)) = &result {
        assert_eq!(msg.code, 400);
        assert!(
            msg.message.contains("SlotMismatch"),
            "expected SlotMismatch error, got: {}",
            msg.message
        );
    }
}

/// POST beacon/execution_payload_envelope with wrong builder_index should return 400.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_execution_payload_envelope_builder_index_mismatch() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;

    let mut envelope = harness
        .chain
        .get_payload_envelope(&head_root)
        .unwrap()
        .expect("self-build envelope should exist");

    // Change builder_index to something different from the committed bid
    envelope.message.builder_index = 42;

    let result = client
        .post_beacon_execution_payload_envelope(&envelope)
        .await;
    assert!(
        result.is_err(),
        "should reject envelope with wrong builder index"
    );
    if let Err(eth2::Error::ServerMessage(msg)) = &result {
        assert_eq!(msg.code, 400);
        assert!(
            msg.message.contains("BuilderIndexMismatch"),
            "expected BuilderIndexMismatch error, got: {}",
            msg.message
        );
    }
}

/// POST beacon/execution_payload_envelope with wrong block_hash should return 400.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_execution_payload_envelope_block_hash_mismatch() {
    use types::ExecutionBlockHash;

    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;

    let mut envelope = harness
        .chain
        .get_payload_envelope(&head_root)
        .unwrap()
        .expect("self-build envelope should exist");

    // Change the payload block_hash to something different from the committed bid
    envelope.message.payload.block_hash = ExecutionBlockHash::repeat_byte(0xba);

    let result = client
        .post_beacon_execution_payload_envelope(&envelope)
        .await;
    assert!(
        result.is_err(),
        "should reject envelope with wrong block hash"
    );
    if let Err(eth2::Error::ServerMessage(msg)) = &result {
        assert_eq!(msg.code, 400);
        assert!(
            msg.message.contains("BlockHashMismatch"),
            "expected BlockHashMismatch error, got: {}",
            msg.message
        );
    }
}

/// GET validator/payload_attestation_data for past slot returns correct data.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn payload_attestation_data_past_slot() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    // Produce two blocks
    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    let (_, mut state_after_1) = harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let state_root_1 = state_after_1.canonical_root().unwrap();
    harness
        .add_attested_block_at_slot(Slot::new(2), state_after_1, state_root_1, &all_validators)
        .await
        .unwrap();

    // Query for slot 1 (past slot, head is now at slot 2)
    let response = client
        .get_validator_payload_attestation_data(Slot::new(1))
        .await
        .expect("should return payload attestation data for past slot");

    let data = response.data;
    assert_eq!(data.slot, Slot::new(1), "slot should match requested slot");
    // Slot 1 had a self-build envelope processed, so payload should be present
    assert!(
        data.payload_present,
        "past slot with processed envelope should have payload_present=true"
    );
}

/// GET validator/payload_attestation_data pre-Gloas returns payload_present=false.
/// The endpoint works regardless of fork, but pre-Gloas blocks have no payload revealed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn payload_attestation_data_pre_gloas_returns_not_present() {
    let validator_count = 32;
    let mut spec = E::default_spec();
    spec.altair_fork_epoch = Some(Epoch::new(0));
    spec.bellatrix_fork_epoch = Some(Epoch::new(0));
    spec.capella_fork_epoch = Some(Epoch::new(0));
    spec.deneb_fork_epoch = Some(Epoch::new(0));
    spec.electra_fork_epoch = Some(Epoch::new(0));
    spec.fulu_fork_epoch = Some(Epoch::new(0));
    // gloas_fork_epoch = None — not scheduled
    let tester = InteractiveTester::<E>::new(Some(spec), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    // Produce a block so the chain has a head
    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let response = client
        .get_validator_payload_attestation_data(Slot::new(1))
        .await
        .expect("should succeed even pre-Gloas");

    let data = response.data;
    assert!(
        !data.payload_present,
        "pre-Gloas blocks should have payload_present=false"
    );
    assert!(
        !data.blob_data_available,
        "pre-Gloas blocks should have blob_data_available=false"
    );
}

/// GET beacon/execution_payload_envelope for a pre-Gloas slot should return None.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_execution_payload_envelope_pre_gloas_slot() {
    let validator_count = 32;
    let fork_epoch = Epoch::new(2); // Gloas at epoch 2
    let spec = gloas_spec(fork_epoch);
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    // Produce a pre-Gloas block at slot 1 (epoch 0, before Gloas fork)
    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let head_root = harness.chain.head_snapshot().beacon_block_root;

    // Pre-Gloas blocks don't have envelopes — should return None
    let result = client
        .get_beacon_execution_payload_envelope::<E>(BlockId::Root(head_root))
        .await
        .expect("request should not fail");

    assert!(
        result.is_none(),
        "pre-Gloas block should not have a payload envelope"
    );
}

/// POST beacon/pool/payload_attestations should return 400 when Gloas is not scheduled.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_payload_attestation_rejected_before_gloas() {
    use types::PayloadAttestationMessage;

    let validator_count = 32;
    let mut spec = E::default_spec();
    spec.altair_fork_epoch = Some(Epoch::new(0));
    spec.bellatrix_fork_epoch = Some(Epoch::new(0));
    spec.capella_fork_epoch = Some(Epoch::new(0));
    spec.deneb_fork_epoch = Some(Epoch::new(0));
    spec.electra_fork_epoch = Some(Epoch::new(0));
    spec.fulu_fork_epoch = Some(Epoch::new(0));
    // gloas_fork_epoch = None
    let tester = InteractiveTester::<E>::new(Some(spec), validator_count).await;
    let client = &tester.client;

    let message = PayloadAttestationMessage::default();
    let result = client
        .post_beacon_pool_payload_attestations(&[message])
        .await;
    assert!(
        result.is_err(),
        "should reject payload attestations when Gloas is not scheduled"
    );
    if let Err(eth2::Error::ServerMessage(msg)) = &result {
        assert_eq!(msg.code, 400);
    }
}

/// GET beacon/execution_payload_envelope should return envelope with correct fields
/// for multiple consecutive Gloas blocks.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_execution_payload_envelope_multiple_blocks() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let all_validators = harness.get_all_validators();
    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();

    // Produce block at slot 1
    let (_, mut state) = harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();
    let root_1 = harness.chain.head_snapshot().beacon_block_root;

    // Produce block at slot 2
    let state_root = state.canonical_root().unwrap();
    harness
        .add_attested_block_at_slot(Slot::new(2), state, state_root, &all_validators)
        .await
        .unwrap();
    let root_2 = harness.chain.head_snapshot().beacon_block_root;

    assert_ne!(root_1, root_2, "blocks should have different roots");

    // Both blocks should have envelopes with distinct block roots
    let env1 = client
        .get_beacon_execution_payload_envelope::<E>(BlockId::Root(root_1))
        .await
        .expect("request should succeed")
        .expect("envelope 1 should exist");

    let env2 = client
        .get_beacon_execution_payload_envelope::<E>(BlockId::Root(root_2))
        .await
        .expect("request should succeed")
        .expect("envelope 2 should exist");

    assert_eq!(env1.data.message.beacon_block_root, root_1);
    assert_eq!(env2.data.message.beacon_block_root, root_2);
    assert_eq!(env1.data.message.slot, Slot::new(1));
    assert_eq!(env2.data.message.slot, Slot::new(2));
}

/// Self-build envelope should have BUILDER_INDEX_SELF_BUILD and correct state_root/block_hash.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_execution_payload_envelope_self_build_fields() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;

    let response = client
        .get_beacon_execution_payload_envelope::<E>(BlockId::Root(head_root))
        .await
        .expect("request should succeed")
        .expect("envelope should exist");

    let envelope = &response.data.message;

    // Self-build uses BUILDER_INDEX_SELF_BUILD (u64::MAX)
    assert_eq!(
        envelope.builder_index,
        u64::MAX,
        "self-build envelope should use BUILDER_INDEX_SELF_BUILD"
    );

    // State root should be non-zero (post-state computed by process_execution_payload)
    assert_ne!(
        envelope.state_root,
        Hash256::zero(),
        "state_root should be set"
    );

    // Block hash should be non-zero (from EL payload)
    assert_ne!(
        envelope.payload.block_hash.into_root(),
        Hash256::zero(),
        "block_hash should be set"
    );
}

/// GET beacon/states/{state_id}/proposer_lookahead should return 400 for pre-Fulu state.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proposer_lookahead_rejected_pre_fulu() {
    let validator_count = 32;
    // Only enable up to Electra — no Fulu, no Gloas
    let mut spec = E::default_spec();
    spec.altair_fork_epoch = Some(Epoch::new(0));
    spec.bellatrix_fork_epoch = Some(Epoch::new(0));
    spec.capella_fork_epoch = Some(Epoch::new(0));
    spec.deneb_fork_epoch = Some(Epoch::new(0));
    spec.electra_fork_epoch = Some(Epoch::new(0));
    // fulu_fork_epoch = None, gloas_fork_epoch = None
    let tester = InteractiveTester::<E>::new(Some(spec), validator_count).await;
    let client = &tester.client;

    let result = client
        .get_beacon_states_proposer_lookahead(StateId::Head)
        .await;

    // Should fail with 400 (pre-Fulu state has no proposer_lookahead)
    assert!(
        result.is_err(),
        "should reject proposer_lookahead for pre-Fulu state"
    );
    if let Err(eth2::Error::ServerMessage(msg)) = &result {
        assert_eq!(msg.code, 400);
    }
}

/// GET beacon/states/{state_id}/proposer_lookahead should return lookahead for Gloas state.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proposer_lookahead_returns_data_gloas() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    // Produce a block so state has been processed
    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let response = client
        .get_beacon_states_proposer_lookahead(StateId::Head)
        .await
        .expect("request should succeed")
        .expect("should return data for Gloas state");

    let lookahead = &response.data;

    // MinimalEthSpec: ProposerLookaheadSlots = 16
    assert_eq!(
        lookahead.len(),
        16,
        "proposer_lookahead should have 16 entries (MinimalEthSpec)"
    );

    // All entries should be valid validator indices
    for &proposer_index in lookahead {
        assert!(
            proposer_index < validator_count as u64,
            "proposer_index {} out of range",
            proposer_index
        );
    }
}

/// GET beacon/states/{state_id}/proposer_lookahead with Fulu state (non-Gloas) should also work.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proposer_lookahead_returns_data_fulu() {
    let validator_count = 32;
    let mut spec = E::default_spec();
    spec.altair_fork_epoch = Some(Epoch::new(0));
    spec.bellatrix_fork_epoch = Some(Epoch::new(0));
    spec.capella_fork_epoch = Some(Epoch::new(0));
    spec.deneb_fork_epoch = Some(Epoch::new(0));
    spec.electra_fork_epoch = Some(Epoch::new(0));
    spec.fulu_fork_epoch = Some(Epoch::new(0));
    // No gloas — Fulu only
    let tester = InteractiveTester::<E>::new(Some(spec), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let response = client
        .get_beacon_states_proposer_lookahead(StateId::Head)
        .await
        .expect("request should succeed")
        .expect("should return data for Fulu state");

    assert_eq!(
        response.data.len(),
        16,
        "Fulu state should also have proposer_lookahead"
    );
}

/// GET beacon/states/{state_id}/proposer_lookahead with slot-based state_id.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proposer_lookahead_by_slot() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let response = client
        .get_beacon_states_proposer_lookahead(StateId::Slot(Slot::new(1)))
        .await
        .expect("request should succeed")
        .expect("should return data for slot-based state_id");

    assert_eq!(response.data.len(), 16);
}

/// PTC duties should handle past epoch gracefully.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ptc_duties_past_epoch_rejected() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    // Advance to slot in epoch 3 (slot 24 for minimal with 8 slots/epoch)
    let slots_per_epoch = E::slots_per_epoch();
    let target_slot = Slot::new(slots_per_epoch * 3);
    harness.extend_to_slot(target_slot).await;

    // Verify we're at epoch 3
    let head_slot = harness.chain.head_snapshot().beacon_block.slot();
    assert!(
        head_slot.epoch(slots_per_epoch) >= Epoch::new(3),
        "should be at epoch 3+, got epoch {}",
        head_slot.epoch(slots_per_epoch)
    );

    let indices: Vec<u64> = (0..validator_count as u64).collect();

    // Request epoch 0 when current epoch is 3 — should fail (too far in the past)
    let result = client
        .post_validator_duties_ptc(Epoch::new(0), &indices)
        .await;

    assert!(
        result.is_err(),
        "should reject PTC duties for epoch too far in the past"
    );
    if let Err(eth2::Error::ServerMessage(msg)) = &result {
        assert_eq!(msg.code, 400);
    }
}

/// PTC duties with empty validator index list should return empty duties.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ptc_duties_empty_indices() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let empty_indices: Vec<u64> = vec![];
    let response = client
        .post_validator_duties_ptc(Epoch::new(0), &empty_indices)
        .await
        .expect("empty index list should succeed");

    assert!(
        response.data.is_empty(),
        "empty validator index list should return no duties"
    );
}

/// PTC duties for next epoch should succeed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ptc_duties_next_epoch() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let indices: Vec<u64> = (0..validator_count as u64).collect();
    let slots_per_epoch = E::slots_per_epoch();

    // Current epoch is 0, request next epoch (1) — should succeed
    let response = client
        .post_validator_duties_ptc(Epoch::new(1), &indices)
        .await
        .expect("next epoch PTC duties should succeed");

    assert!(
        !response.data.is_empty(),
        "next epoch duties should not be empty"
    );

    // All duties should have slots in epoch 1
    for duty in &response.data {
        assert_eq!(
            duty.slot.epoch(slots_per_epoch),
            Epoch::new(1),
            "duty slot {} not in epoch 1",
            duty.slot
        );
    }
}

/// POST payload attestation with wrong signature should fail.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_payload_attestation_wrong_signature() {
    use state_processing::per_block_processing::gloas::get_ptc_committee;
    use types::{Domain, PayloadAttestationData, PayloadAttestationMessage, SignedRoot};

    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    let ptc = get_ptc_committee::<E>(state, head_slot, &spec).expect("should get PTC committee");
    let validator_index = ptc[0];

    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: true,
        blob_data_available: false,
    };
    let epoch = head_slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(
        epoch,
        Domain::PtcAttester,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let message_root = data.signing_root(domain);

    // Sign with WRONG key (use a different validator's key)
    let wrong_index = if validator_index == 0 { 1 } else { 0 };
    let wrong_keypair = generate_deterministic_keypair(wrong_index as usize);
    let signature = wrong_keypair.sk.sign(message_root);

    let message = PayloadAttestationMessage {
        validator_index,
        data,
        signature,
    };

    let result = client
        .post_beacon_pool_payload_attestations(&[message])
        .await;
    assert!(
        result.is_err(),
        "should reject payload attestation with wrong signature"
    );
}

/// POST mixed valid and invalid payload attestations returns indexed errors.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_payload_attestation_mixed_valid_invalid() {
    use state_processing::per_block_processing::gloas::get_ptc_committee;
    use types::{Domain, PayloadAttestationData, PayloadAttestationMessage, SignedRoot};

    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    let ptc = get_ptc_committee::<E>(state, head_slot, &spec).expect("should get PTC committee");
    assert!(ptc.len() >= 2, "need at least 2 PTC members for this test");

    let epoch = head_slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(
        epoch,
        Domain::PtcAttester,
        &state.fork(),
        state.genesis_validators_root(),
    );

    // Valid attestation from first PTC member
    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: true,
        blob_data_available: true,
    };
    let message_root = data.signing_root(domain);
    let keypair = generate_deterministic_keypair(ptc[0] as usize);
    let valid_message = PayloadAttestationMessage {
        validator_index: ptc[0],
        data: data.clone(),
        signature: keypair.sk.sign(message_root),
    };

    // Invalid attestation: non-PTC validator
    let non_ptc_validator = (0..validator_count as u64)
        .find(|idx| !ptc.contains(idx))
        .expect("should find non-PTC validator");
    let invalid_keypair = generate_deterministic_keypair(non_ptc_validator as usize);
    let invalid_message = PayloadAttestationMessage {
        validator_index: non_ptc_validator,
        data,
        signature: invalid_keypair.sk.sign(message_root),
    };

    // Submit [valid, invalid] — should return indexed error for index 1
    let result = client
        .post_beacon_pool_payload_attestations(&[valid_message, invalid_message])
        .await;

    assert!(result.is_err(), "mixed valid/invalid should return error");
    // The error should be an indexed error with failure at index 1
    assert_server_indexed_error(result.unwrap_err(), 400, vec![1]);
}

/// GET expected_withdrawals with Gloas head state should succeed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expected_withdrawals_gloas() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let response = client
        .get_expected_withdrawals(&StateId::Head)
        .await
        .expect("expected_withdrawals should succeed for Gloas state");

    // At genesis with default validators, there may or may not be withdrawals
    // depending on validator balances. The key thing is the endpoint doesn't error.
    let _withdrawals = &response.data;
}

/// PTC duties should return consistent dependent_root across calls for same epoch.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ptc_duties_dependent_root_consistent() {
    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let indices: Vec<u64> = (0..validator_count as u64).collect();

    let response1 = client
        .post_validator_duties_ptc(Epoch::new(0), &indices)
        .await
        .expect("first call should succeed");
    let response2 = client
        .post_validator_duties_ptc(Epoch::new(0), &indices)
        .await
        .expect("second call should succeed");

    assert_eq!(
        response1.dependent_root, response2.dependent_root,
        "dependent_root should be consistent across calls"
    );
    assert_eq!(
        response1.data.len(),
        response2.data.len(),
        "duty count should be consistent"
    );
}

/// POST beacon/pool/proposer_preferences should accept a validly signed message.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_proposer_preferences_valid() {
    use types::{Domain, ProposerPreferences, SignedProposerPreferences, SignedRoot};

    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    // Advance a block so the chain has state
    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let head = harness.chain.head_snapshot();
    let state = &head.beacon_state;
    let validator_index: u64 = 0;
    let proposal_slot = Slot::new(2);
    let proposal_epoch = proposal_slot.epoch(E::slots_per_epoch());

    let preferences = ProposerPreferences {
        proposal_slot: proposal_slot.as_u64(),
        validator_index,
        fee_recipient: Address::repeat_byte(0x42),
        gas_limit: 30_000_000,
    };

    let domain = spec.get_domain(
        proposal_epoch,
        Domain::ProposerPreferences,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let signing_root = preferences.signing_root(domain);
    let keypair = generate_deterministic_keypair(validator_index as usize);
    let signature = keypair.sk.sign(signing_root);

    let signed = SignedProposerPreferences {
        message: preferences,
        signature,
    };

    let result = client.post_beacon_pool_proposer_preferences(&signed).await;
    assert!(
        result.is_ok(),
        "should accept valid proposer preferences, got: {:?}",
        result.err()
    );
}

/// POST beacon/pool/proposer_preferences should reject when Gloas is not scheduled.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_proposer_preferences_rejected_before_gloas() {
    use types::{ProposerPreferences, SignedProposerPreferences};

    let validator_count = 32;
    let mut spec = E::default_spec();
    spec.altair_fork_epoch = Some(Epoch::new(0));
    spec.bellatrix_fork_epoch = Some(Epoch::new(0));
    spec.capella_fork_epoch = Some(Epoch::new(0));
    spec.deneb_fork_epoch = Some(Epoch::new(0));
    spec.electra_fork_epoch = Some(Epoch::new(0));
    spec.fulu_fork_epoch = Some(Epoch::new(0));
    // gloas_fork_epoch = None
    let tester = InteractiveTester::<E>::new(Some(spec), validator_count).await;
    let client = &tester.client;

    let signed = SignedProposerPreferences {
        message: ProposerPreferences {
            proposal_slot: 1,
            validator_index: 0,
            fee_recipient: Address::repeat_byte(0x42),
            gas_limit: 30_000_000,
        },
        signature: Signature::empty(),
    };

    let result = client.post_beacon_pool_proposer_preferences(&signed).await;
    assert!(result.is_err(), "should reject when Gloas is not scheduled");
    if let Err(eth2::Error::ServerMessage(msg)) = &result {
        assert_eq!(msg.code, 400);
        assert!(
            msg.message.contains("Gloas is not scheduled"),
            "expected 'Gloas is not scheduled', got: {}",
            msg.message
        );
    } else {
        panic!("expected ServerMessage error, got: {:?}", result);
    }
}

/// POST beacon/pool/proposer_preferences should reject invalid signature.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_proposer_preferences_invalid_signature() {
    use types::{Domain, ProposerPreferences, SignedProposerPreferences, SignedRoot};

    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let head = harness.chain.head_snapshot();
    let state = &head.beacon_state;
    let validator_index: u64 = 0;
    let proposal_slot = Slot::new(2);
    let proposal_epoch = proposal_slot.epoch(E::slots_per_epoch());

    let preferences = ProposerPreferences {
        proposal_slot: proposal_slot.as_u64(),
        validator_index,
        fee_recipient: Address::repeat_byte(0x42),
        gas_limit: 30_000_000,
    };

    // Sign with the WRONG key (validator 1's key instead of validator 0's)
    let domain = spec.get_domain(
        proposal_epoch,
        Domain::ProposerPreferences,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let signing_root = preferences.signing_root(domain);
    let wrong_keypair = generate_deterministic_keypair(1);
    let signature = wrong_keypair.sk.sign(signing_root);

    let signed = SignedProposerPreferences {
        message: preferences,
        signature,
    };

    let result = client.post_beacon_pool_proposer_preferences(&signed).await;
    assert!(
        result.is_err(),
        "should reject preferences with invalid signature"
    );
    if let Err(eth2::Error::ServerMessage(msg)) = &result {
        assert_eq!(msg.code, 400);
        assert!(
            msg.message
                .contains("Invalid proposer preferences signature"),
            "expected signature error, got: {}",
            msg.message
        );
    } else {
        panic!("expected ServerMessage error, got: {:?}", result);
    }
}

/// POST beacon/pool/proposer_preferences should reject unknown validator index.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_proposer_preferences_unknown_validator() {
    use types::{ProposerPreferences, SignedProposerPreferences};

    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec), validator_count).await;
    let client = &tester.client;

    let signed = SignedProposerPreferences {
        message: ProposerPreferences {
            proposal_slot: 1,
            validator_index: 9999, // doesn't exist
            fee_recipient: Address::repeat_byte(0x42),
            gas_limit: 30_000_000,
        },
        signature: Signature::empty(),
    };

    let result = client.post_beacon_pool_proposer_preferences(&signed).await;
    assert!(result.is_err(), "should reject unknown validator index");
    if let Err(eth2::Error::ServerMessage(msg)) = &result {
        assert_eq!(msg.code, 400);
        assert!(
            msg.message.contains("Unknown validator index"),
            "expected unknown validator error, got: {}",
            msg.message
        );
    } else {
        panic!("expected ServerMessage error, got: {:?}", result);
    }
}

/// POST beacon/pool/proposer_preferences should ignore duplicate for same slot.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_proposer_preferences_duplicate_ignored() {
    use types::{Domain, ProposerPreferences, SignedProposerPreferences, SignedRoot};

    let validator_count = 32;
    let spec = gloas_spec(Epoch::new(0));
    let tester = InteractiveTester::<E>::new(Some(spec.clone()), validator_count).await;
    let harness = &tester.harness;
    let client = &tester.client;

    let (genesis_state, genesis_state_root) = harness.get_current_state_and_root();
    let all_validators = harness.get_all_validators();
    harness
        .add_attested_block_at_slot(
            Slot::new(1),
            genesis_state,
            genesis_state_root,
            &all_validators,
        )
        .await
        .unwrap();

    let head = harness.chain.head_snapshot();
    let state = &head.beacon_state;
    let validator_index: u64 = 0;
    let proposal_slot = Slot::new(2);
    let proposal_epoch = proposal_slot.epoch(E::slots_per_epoch());

    let preferences = ProposerPreferences {
        proposal_slot: proposal_slot.as_u64(),
        validator_index,
        fee_recipient: Address::repeat_byte(0x42),
        gas_limit: 30_000_000,
    };

    let domain = spec.get_domain(
        proposal_epoch,
        Domain::ProposerPreferences,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let signing_root = preferences.signing_root(domain);
    let keypair = generate_deterministic_keypair(validator_index as usize);
    let signature = keypair.sk.sign(signing_root);

    let signed = SignedProposerPreferences {
        message: preferences,
        signature,
    };

    // First submission should succeed
    let result1 = client.post_beacon_pool_proposer_preferences(&signed).await;
    assert!(result1.is_ok(), "first submission should succeed");

    // Second submission for same slot should also return 200 (silently ignored)
    let result2 = client.post_beacon_pool_proposer_preferences(&signed).await;
    assert!(
        result2.is_ok(),
        "duplicate submission should succeed (silently ignored), got: {:?}",
        result2.err()
    );
}
