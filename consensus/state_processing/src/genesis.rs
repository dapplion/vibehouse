use super::per_block_processing::{
    errors::BlockProcessingError, process_operations::apply_deposit,
};
use crate::common::DepositDataTree;
use crate::upgrade::electra::upgrade_state_to_electra;
use crate::upgrade::{
    upgrade_to_altair, upgrade_to_bellatrix, upgrade_to_capella, upgrade_to_deneb, upgrade_to_fulu,
    upgrade_to_gloas,
};
use safe_arith::{ArithError, SafeArith};
use std::sync::Arc;
use tree_hash::TreeHash;
use types::*;

/// Initialize a `BeaconState` from genesis data.
pub fn initialize_beacon_state_from_eth1<E: EthSpec>(
    eth1_block_hash: Hash256,
    eth1_timestamp: u64,
    deposits: Vec<Deposit>,
    execution_payload_header: Option<ExecutionPayloadHeader<E>>,
    spec: &ChainSpec,
) -> Result<BeaconState<E>, BlockProcessingError> {
    let genesis_time = eth2_genesis_time(eth1_timestamp, spec)?;
    let eth1_data = Eth1Data {
        // Temporary deposit root
        deposit_root: Hash256::zero(),
        deposit_count: deposits.len() as u64,
        block_hash: eth1_block_hash,
    };
    let mut state = BeaconState::new(genesis_time, eth1_data, spec);

    // Seed RANDAO with Eth1 entropy
    state.fill_randao_mixes_with(eth1_block_hash)?;

    let mut deposit_tree = DepositDataTree::create(&[], 0, DEPOSIT_TREE_DEPTH);

    for deposit in deposits.into_iter() {
        deposit_tree
            .push_leaf(deposit.data.tree_hash_root())
            .map_err(BlockProcessingError::MerkleTreeError)?;
        state.eth1_data_mut().deposit_root = deposit_tree.root();
        let Deposit { proof, data } = deposit;
        apply_deposit(&mut state, data, Some(proof), true, spec)?;
    }

    process_activations(&mut state, spec)?;

    // To support testnets with Altair enabled from genesis, perform a possible state upgrade here.
    // This must happen *after* deposits and activations are processed or the calculation of sync
    // committees during the upgrade will fail. It's a bit cheeky to do this instead of having
    // separate Altair genesis initialization logic, but it turns out that our
    // use of `BeaconBlock::empty` in `BeaconState::new` is sufficient to correctly initialise
    // the `latest_block_header` as per:
    // https://github.com/ethereum/eth2.0-specs/pull/2323
    if spec
        .altair_fork_epoch
        .is_some_and(|fork_epoch| fork_epoch == E::genesis_epoch())
    {
        upgrade_to_altair(&mut state, spec)?;

        state.fork_mut().previous_version = spec.altair_fork_version;
    }

    // Similarly, perform an upgrade to the merge if configured from genesis.
    if spec
        .bellatrix_fork_epoch
        .is_some_and(|fork_epoch| fork_epoch == E::genesis_epoch())
    {
        // this will set state.latest_execution_payload_header = ExecutionPayloadHeaderBellatrix::default()
        upgrade_to_bellatrix(&mut state, spec)?;

        // Remove intermediate Altair fork from `state.fork`.
        state.fork_mut().previous_version = spec.bellatrix_fork_version;

        // Override latest execution payload header.
        // See https://github.com/ethereum/consensus-specs/blob/v1.1.0/specs/bellatrix/beacon-chain.md#testing
        if let Some(ExecutionPayloadHeader::Bellatrix(ref header)) = execution_payload_header {
            *state.latest_execution_payload_header_bellatrix_mut()? = header.clone();
        }
    }

    // Upgrade to capella if configured from genesis
    if spec
        .capella_fork_epoch
        .is_some_and(|fork_epoch| fork_epoch == E::genesis_epoch())
    {
        upgrade_to_capella(&mut state, spec)?;

        // Remove intermediate Bellatrix fork from `state.fork`.
        state.fork_mut().previous_version = spec.capella_fork_version;

        // Override latest execution payload header.
        // See https://github.com/ethereum/consensus-specs/blob/dev/specs/capella/beacon-chain.md#testing
        if let Some(ExecutionPayloadHeader::Capella(ref header)) = execution_payload_header {
            *state.latest_execution_payload_header_capella_mut()? = header.clone();
        }
    }

    // Upgrade to deneb if configured from genesis
    if spec
        .deneb_fork_epoch
        .is_some_and(|fork_epoch| fork_epoch == E::genesis_epoch())
    {
        upgrade_to_deneb(&mut state, spec)?;

        // Remove intermediate Capella fork from `state.fork`.
        state.fork_mut().previous_version = spec.deneb_fork_version;

        // Override latest execution payload header.
        // See https://github.com/ethereum/consensus-specs/blob/dev/specs/deneb/beacon-chain.md#testing
        if let Some(ExecutionPayloadHeader::Deneb(ref header)) = execution_payload_header {
            *state.latest_execution_payload_header_deneb_mut()? = header.clone();
        }
    }

    // Upgrade to electra if configured from genesis.
    if spec
        .electra_fork_epoch
        .is_some_and(|fork_epoch| fork_epoch == E::genesis_epoch())
    {
        let post = upgrade_state_to_electra(&mut state, Epoch::new(0), Epoch::new(0), spec)?;
        state = post;

        // Remove intermediate Deneb fork from `state.fork`.
        state.fork_mut().previous_version = spec.electra_fork_version;

        // The spec tests will expect that the sync committees are
        // calculated using the electra value for MAX_EFFECTIVE_BALANCE when
        // calling `initialize_beacon_state_from_eth1()`. But the sync committees
        // are actually calcuated back in `upgrade_to_altair()`. We need to
        // re-calculate the sync committees here now that the state is `Electra`
        let sync_committee = Arc::new(state.get_next_sync_committee(spec)?);
        *state.current_sync_committee_mut()? = sync_committee.clone();
        *state.next_sync_committee_mut()? = sync_committee;

        // Override latest execution payload header.
        // See https://github.com/ethereum/consensus-specs/blob/dev/specs/capella/beacon-chain.md#testing
        if let Some(ExecutionPayloadHeader::Electra(ref header)) = execution_payload_header {
            *state.latest_execution_payload_header_electra_mut()? = header.clone();
        }
    }

    // Upgrade to fulu if configured from genesis.
    if spec
        .fulu_fork_epoch
        .is_some_and(|fork_epoch| fork_epoch == E::genesis_epoch())
    {
        upgrade_to_fulu(&mut state, spec)?;

        // Remove intermediate Electra fork from `state.fork`.
        state.fork_mut().previous_version = spec.fulu_fork_version;

        // Override latest execution payload header.
        if let Some(ExecutionPayloadHeader::Fulu(ref header)) = execution_payload_header {
            *state.latest_execution_payload_header_fulu_mut()? = header.clone();
        }
    }

    // Upgrade to gloas if configured from genesis.
    if spec
        .gloas_fork_epoch
        .is_some_and(|fork_epoch| fork_epoch == E::genesis_epoch())
    {
        // When genesis is at Gloas, the execution_payload_header is a Gloas variant.
        // We need to set the intermediate Fulu header's block_hash before upgrading,
        // because upgrade_to_gloas reads it to initialize latest_block_hash.
        if let Some(ref header) = execution_payload_header {
            let fulu_header = state.latest_execution_payload_header_fulu_mut()?;
            fulu_header.block_hash = header.block_hash();
            fulu_header.transactions_root = header.transactions_root();
        }

        upgrade_to_gloas(&mut state, spec)?;

        // Remove intermediate Fulu fork from `state.fork`.
        state.fork_mut().previous_version = spec.gloas_fork_version;
    }

    // Now that we have our validators, initialize the caches (including the committees)
    state.build_caches(spec)?;

    // Set genesis validators root for domain separation and chain versioning
    *state.genesis_validators_root_mut() = state.update_validators_tree_hash_cache()?;

    Ok(state)
}

/// Determine whether a candidate genesis state is suitable for starting the chain.
pub fn is_valid_genesis_state<E: EthSpec>(state: &BeaconState<E>, spec: &ChainSpec) -> bool {
    state
        .get_active_validator_indices(E::genesis_epoch(), spec)
        .is_ok_and(|active_validators| {
            state.genesis_time() >= spec.min_genesis_time
                && active_validators.len() as u64 >= spec.min_genesis_active_validator_count
        })
}

/// Activate genesis validators, if their balance is acceptable.
pub fn process_activations<E: EthSpec>(
    state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), Error> {
    let (validators, balances, _) = state.validators_and_balances_and_progressive_balances_mut();
    let mut validators_iter = validators.iter_cow();
    while let Some((index, validator)) = validators_iter.next_cow() {
        let validator = validator.into_mut()?;
        let balance = balances
            .get(index)
            .copied()
            .ok_or(Error::BalancesOutOfBounds(index))?;
        validator.effective_balance = std::cmp::min(
            balance.safe_sub(balance.safe_rem(spec.effective_balance_increment)?)?,
            spec.max_effective_balance,
        );
        if validator.effective_balance == spec.max_effective_balance {
            validator.activation_eligibility_epoch = E::genesis_epoch();
            validator.activation_epoch = E::genesis_epoch();
        }
    }
    Ok(())
}

/// Returns the `state.genesis_time` for the corresponding `eth1_timestamp`.
///
/// Does _not_ ensure that the time is greater than `MIN_GENESIS_TIME`.
///
/// Spec v0.12.1
pub fn eth2_genesis_time(eth1_timestamp: u64, spec: &ChainSpec) -> Result<u64, ArithError> {
    eth1_timestamp.safe_add(spec.genesis_delay)
}

#[cfg(test)]
mod gloas_genesis_tests {
    use super::*;
    use types::{
        ExecutionBlockHash, ExecutionPayloadHeaderGloas, ForkName, MinimalEthSpec,
        test_utils::generate_deterministic_keypairs,
    };

    type E = MinimalEthSpec;

    /// Create a spec with all forks active at genesis (epoch 0), including Gloas.
    fn gloas_genesis_spec() -> ChainSpec {
        ForkName::Gloas.make_genesis_spec(E::default_spec())
    }

    /// Create deposits with proper merkle proofs for genesis initialization.
    fn make_genesis_deposits(num_validators: usize, spec: &ChainSpec) -> Vec<Deposit> {
        let keypairs = generate_deterministic_keypairs(num_validators);
        let mut deposit_datas = Vec::with_capacity(num_validators);
        for kp in &keypairs {
            let mut creds = [0u8; 32];
            creds[0] = spec.eth1_address_withdrawal_prefix_byte;
            creds[12..].copy_from_slice(&[0xAA; 20]);
            let withdrawal_credentials = Hash256::from_slice(&creds);

            let mut data = DepositData {
                pubkey: kp.pk.clone().into(),
                withdrawal_credentials,
                amount: spec.max_effective_balance,
                signature: Signature::empty().into(),
            };
            data.signature = data.create_signature(&kp.sk, spec);
            deposit_datas.push(data);
        }

        let mut tree = crate::common::DepositDataTree::create(&[], 0, DEPOSIT_TREE_DEPTH);
        let mut deposits = Vec::with_capacity(num_validators);
        for data in deposit_datas {
            tree.push_leaf(data.tree_hash_root())
                .expect("should push leaf");
            let (_leaf, proof_vec) = tree
                .generate_proof(deposits.len())
                .expect("should generate proof");
            let mut proof = FixedVector::from(vec![Hash256::zero(); DEPOSIT_TREE_DEPTH + 1]);
            for (i, node) in proof_vec.iter().enumerate() {
                proof[i] = *node;
            }
            deposits.push(Deposit { proof, data });
        }
        deposits
    }

    #[test]
    fn gloas_genesis_produces_gloas_state() {
        let spec = gloas_genesis_spec();
        let deposits = make_genesis_deposits(8, &spec);

        let state = initialize_beacon_state_from_eth1::<E>(
            Hash256::repeat_byte(0x42),
            2u64.pow(40),
            deposits,
            None,
            &spec,
        )
        .expect("should initialize genesis state");

        assert!(
            state.as_gloas().is_ok(),
            "genesis state should be Gloas variant"
        );

        let fork = state.fork();
        assert_eq!(fork.current_version, spec.gloas_fork_version);
        assert_eq!(
            fork.previous_version, spec.gloas_fork_version,
            "previous_version should be overridden to gloas_fork_version"
        );
        assert_eq!(fork.epoch, E::genesis_epoch());
    }

    #[test]
    fn gloas_genesis_initializes_gloas_fields() {
        let spec = gloas_genesis_spec();
        let deposits = make_genesis_deposits(8, &spec);

        let state = initialize_beacon_state_from_eth1::<E>(
            Hash256::repeat_byte(0x42),
            2u64.pow(40),
            deposits,
            None,
            &spec,
        )
        .expect("should initialize genesis state");

        let state_gloas = state.as_gloas().unwrap();

        // Builders list starts empty (no builder deposits at genesis)
        assert_eq!(state_gloas.builders.len(), 0);

        // Builder pending payments initialized to default
        for payment in state_gloas.builder_pending_payments.iter() {
            assert_eq!(payment.withdrawal.amount, 0);
            assert_eq!(payment.weight, 0);
        }

        // Builder pending withdrawals starts empty
        assert!(state_gloas.builder_pending_withdrawals.is_empty());

        // Execution payload availability all set to true
        for i in 0..E::slots_per_historical_root() {
            assert!(
                state_gloas.execution_payload_availability.get(i).unwrap(),
                "execution_payload_availability bit {} should be set",
                i
            );
        }

        // Expected withdrawals starts empty
        assert!(state_gloas.payload_expected_withdrawals.is_empty());
    }

    #[test]
    fn gloas_genesis_with_execution_payload_header() {
        let spec = gloas_genesis_spec();
        let deposits = make_genesis_deposits(8, &spec);
        let block_hash = ExecutionBlockHash::repeat_byte(0xBB);

        let header = ExecutionPayloadHeader::Gloas(ExecutionPayloadHeaderGloas {
            block_hash,
            transactions_root: Hash256::repeat_byte(0xCC),
            ..Default::default()
        });

        let state = initialize_beacon_state_from_eth1::<E>(
            Hash256::repeat_byte(0x42),
            2u64.pow(40),
            deposits,
            Some(header),
            &spec,
        )
        .expect("should initialize genesis state with header");

        let state_gloas = state.as_gloas().unwrap();

        // latest_block_hash should be set from the header's block_hash via
        // the intermediate Fulu header during upgrade_to_gloas
        assert_eq!(
            state_gloas.latest_block_hash, block_hash,
            "latest_block_hash should match the provided header's block_hash"
        );
    }

    #[test]
    fn gloas_genesis_validators_activated() {
        let spec = gloas_genesis_spec();
        let num_validators = 8;
        let deposits = make_genesis_deposits(num_validators, &spec);

        let state = initialize_beacon_state_from_eth1::<E>(
            Hash256::repeat_byte(0x42),
            2u64.pow(40),
            deposits,
            None,
            &spec,
        )
        .expect("should initialize genesis state");

        assert_eq!(state.validators().len(), num_validators);
        assert_eq!(state.balances().len(), num_validators);

        for v in state.validators() {
            assert_eq!(
                v.activation_epoch,
                E::genesis_epoch(),
                "validator should be active at genesis"
            );
            assert_eq!(
                v.effective_balance, spec.max_effective_balance,
                "validator should have max effective balance"
            );
        }
    }

    #[test]
    fn gloas_genesis_caches_built() {
        let spec = gloas_genesis_spec();
        let deposits = make_genesis_deposits(8, &spec);

        let state = initialize_beacon_state_from_eth1::<E>(
            Hash256::repeat_byte(0x42),
            2u64.pow(40),
            deposits,
            None,
            &spec,
        )
        .expect("should initialize genesis state");

        assert_ne!(
            state.genesis_validators_root(),
            Hash256::zero(),
            "genesis_validators_root should be set"
        );
    }

    #[test]
    fn gloas_genesis_is_valid() {
        let spec = gloas_genesis_spec();
        // MinimalEthSpec requires min_genesis_active_validator_count=64
        let deposits = make_genesis_deposits(64, &spec);

        let state = initialize_beacon_state_from_eth1::<E>(
            Hash256::repeat_byte(0x42),
            2u64.pow(40),
            deposits,
            None,
            &spec,
        )
        .expect("should initialize genesis state");

        assert!(
            is_valid_genesis_state::<E>(&state, &spec),
            "state should be a valid genesis state"
        );
    }

    #[test]
    fn gloas_genesis_no_execution_header_zero_block_hash() {
        let spec = gloas_genesis_spec();
        let deposits = make_genesis_deposits(8, &spec);

        let state = initialize_beacon_state_from_eth1::<E>(
            Hash256::repeat_byte(0x42),
            2u64.pow(40),
            deposits,
            None,
            &spec,
        )
        .expect("should initialize genesis state without header");

        let state_gloas = state.as_gloas().unwrap();

        // Without a header, latest_block_hash comes from the default Fulu header
        assert_eq!(
            state_gloas.latest_block_hash,
            ExecutionBlockHash::zero(),
            "latest_block_hash should be zero when no header provided"
        );
    }

    #[test]
    fn gloas_genesis_bid_defaults() {
        let spec = gloas_genesis_spec();
        let deposits = make_genesis_deposits(8, &spec);

        let state = initialize_beacon_state_from_eth1::<E>(
            Hash256::repeat_byte(0x42),
            2u64.pow(40),
            deposits,
            None,
            &spec,
        )
        .expect("should initialize genesis state");

        let state_gloas = state.as_gloas().unwrap();

        assert_eq!(state_gloas.latest_execution_payload_bid.value, 0);
        assert_eq!(state_gloas.latest_execution_payload_bid.builder_index, 0);
        assert_eq!(state_gloas.latest_execution_payload_bid.slot, Slot::new(0));
    }

    #[test]
    fn gloas_genesis_sync_committees_set() {
        let spec = gloas_genesis_spec();
        let deposits = make_genesis_deposits(8, &spec);

        let state = initialize_beacon_state_from_eth1::<E>(
            Hash256::repeat_byte(0x42),
            2u64.pow(40),
            deposits,
            None,
            &spec,
        )
        .expect("should initialize genesis state");

        let current_sync = state.current_sync_committee().unwrap();
        let has_real_pubkeys = current_sync
            .pubkeys
            .iter()
            .any(|pk| *pk != types::PublicKeyBytes::empty());
        assert!(
            has_real_pubkeys,
            "current sync committee should have real pubkeys"
        );
    }
}
