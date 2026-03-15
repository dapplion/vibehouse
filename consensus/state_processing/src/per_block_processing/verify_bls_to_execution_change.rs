use super::errors::{BlockOperationError, BlsExecutionChangeInvalid as Invalid};
use crate::VerifySignatures;
use crate::per_block_processing::signature_sets::bls_execution_change_signature_set;
use ethereum_hashing::hash_fixed;
use types::*;

type Result<T> = std::result::Result<T, BlockOperationError<Invalid>>;

fn error(reason: Invalid) -> BlockOperationError<Invalid> {
    BlockOperationError::invalid(reason)
}

/// Indicates if a `BlsToExecutionChange` is valid to be included in a block,
/// where the block is being applied to the given `state`.
///
/// Returns `Ok(())` if the `SignedBlsToExecutionChange` is valid, otherwise indicates the reason for invalidity.
pub fn verify_bls_to_execution_change<E: EthSpec>(
    state: &BeaconState<E>,
    signed_address_change: &SignedBlsToExecutionChange,
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<()> {
    let address_change = &signed_address_change.message;

    let validator = state
        .validators()
        .get(address_change.validator_index as usize)
        .ok_or_else(|| error(Invalid::ValidatorUnknown(address_change.validator_index)))?;

    verify!(
        validator
            .withdrawal_credentials
            .as_slice()
            .first()
            .map(|byte| *byte == spec.bls_withdrawal_prefix_byte)
            .unwrap_or(false),
        Invalid::NonBlsWithdrawalCredentials
    );

    // Re-hashing the pubkey isn't necessary during block replay, so we may want to skip that in
    // future.
    let pubkey_hash = hash_fixed(address_change.from_bls_pubkey.as_serialized());
    verify!(
        validator.withdrawal_credentials.as_slice().get(1..) == pubkey_hash.get(1..),
        Invalid::WithdrawalCredentialsMismatch
    );

    if verify_signatures.is_true() {
        verify!(
            bls_execution_change_signature_set(state, signed_address_change, spec)?.verify(),
            Invalid::BadSignature
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bls::PublicKeyBytes;
    use std::sync::Arc;
    use types::{
        BeaconState, BeaconStateGloas, BitVector, BuilderPendingPayment, Checkpoint, EpochCache,
        Eth1Data, ExecutionBlockHash, ExecutionPayloadBid, ExitCache, ForkName, MinimalEthSpec,
        ProgressiveBalancesCache, PubkeyCache, SlashingsCache, SyncCommittee,
        beacon_state::BuilderPubkeyCache,
    };

    type E = MinimalEthSpec;

    fn make_spec() -> ChainSpec {
        ForkName::Gloas.make_genesis_spec(E::default_spec())
    }

    /// Create a BLS withdrawal credential from a BLS pubkey.
    /// First byte is 0x00 (BLS prefix), remaining 31 bytes are from the hash of the pubkey.
    fn make_bls_withdrawal_credentials(pubkey: &PublicKeyBytes, spec: &ChainSpec) -> Hash256 {
        let pubkey_hash = ethereum_hashing::hash_fixed(pubkey.as_serialized());
        let mut creds = [0u8; 32];
        creds[0] = spec.bls_withdrawal_prefix_byte;
        creds[1..].copy_from_slice(&pubkey_hash[1..]);
        Hash256::from_slice(&creds)
    }

    /// Create a minimal Gloas state with validators.
    fn make_gloas_state(spec: &ChainSpec) -> BeaconState<E> {
        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

        let current_epoch = Epoch::new(10);
        let slot = current_epoch.start_slot(E::slots_per_epoch());

        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                PublicKeyBytes::empty();
                <E as EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: PublicKeyBytes::empty(),
        });

        // Create deterministic keypairs for validators
        let keypairs = types::test_utils::generate_deterministic_keypairs(4);

        let mut validators = Vec::with_capacity(4);
        let mut balances = Vec::with_capacity(4);
        for kp in &keypairs {
            let pubkey_bytes: PublicKeyBytes = kp.pk.clone().into();
            let withdrawal_creds = make_bls_withdrawal_credentials(&pubkey_bytes, spec);

            validators.push(Validator {
                pubkey: pubkey_bytes,
                withdrawal_credentials: withdrawal_creds,
                effective_balance: spec.max_effective_balance,
                slashed: false,
                activation_eligibility_epoch: Epoch::new(0),
                activation_epoch: Epoch::new(0),
                exit_epoch: spec.far_future_epoch,
                withdrawable_epoch: spec.far_future_epoch,
            });
            balances.push(spec.max_effective_balance);
        }

        BeaconState::Gloas(BeaconStateGloas {
            genesis_time: 0,
            genesis_validators_root: Hash256::repeat_byte(0xAA),
            slot,
            fork: Fork {
                previous_version: spec.fulu_fork_version,
                current_version: spec.gloas_fork_version,
                epoch: current_epoch,
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
            validators: List::new(validators).unwrap(),
            balances: List::new(balances).unwrap(),
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

    fn make_signed_change(
        validator_index: u64,
        from_bls_pubkey: PublicKeyBytes,
        to_execution_address: Address,
    ) -> SignedBlsToExecutionChange {
        SignedBlsToExecutionChange {
            message: BlsToExecutionChange {
                validator_index,
                from_bls_pubkey,
                to_execution_address,
            },
            signature: Signature::empty(),
        }
    }

    #[test]
    fn valid_bls_to_execution_change() {
        let spec = make_spec();
        let state = make_gloas_state(&spec);

        let validator = state.validators().get(0).unwrap();
        let pubkey = validator.pubkey;
        let to_address = Address::repeat_byte(0xCC);

        let signed_change = make_signed_change(0, pubkey, to_address);
        let result =
            verify_bls_to_execution_change(&state, &signed_change, VerifySignatures::False, &spec);
        assert!(result.is_ok());
    }

    #[test]
    fn valid_change_different_validator_indices() {
        let spec = make_spec();
        let state = make_gloas_state(&spec);
        let to_address = Address::repeat_byte(0xCC);

        // Test all 4 validators
        for i in 0..4 {
            let validator = state.validators().get(i).unwrap();
            let pubkey = validator.pubkey;
            let signed_change = make_signed_change(i as u64, pubkey, to_address);
            let result = verify_bls_to_execution_change(
                &state,
                &signed_change,
                VerifySignatures::False,
                &spec,
            );
            assert!(result.is_ok(), "validator {} should succeed", i);
        }
    }

    #[test]
    fn unknown_validator() {
        let spec = make_spec();
        let state = make_gloas_state(&spec);
        let pubkey = PublicKeyBytes::empty();
        let to_address = Address::repeat_byte(0xCC);

        let signed_change = make_signed_change(999, pubkey, to_address);
        let result =
            verify_bls_to_execution_change(&state, &signed_change, VerifySignatures::False, &spec);
        assert_eq!(
            result,
            Err(BlockOperationError::Invalid(Invalid::ValidatorUnknown(999)))
        );
    }

    #[test]
    fn non_bls_withdrawal_credentials_eth1_prefix() {
        let spec = make_spec();
        let mut state = make_gloas_state(&spec);

        // Change validator 0's withdrawal credentials to eth1 prefix (0x01)
        let mut creds = [0u8; 32];
        creds[0] = spec.eth1_address_withdrawal_prefix_byte; // 0x01
        creds[12..].copy_from_slice(&[0xAA; 20]);
        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .withdrawal_credentials = Hash256::from_slice(&creds);

        let pubkey = state.validators().get(0).unwrap().pubkey;
        let signed_change = make_signed_change(0, pubkey, Address::repeat_byte(0xCC));
        let result =
            verify_bls_to_execution_change(&state, &signed_change, VerifySignatures::False, &spec);
        assert_eq!(
            result,
            Err(BlockOperationError::Invalid(
                Invalid::NonBlsWithdrawalCredentials
            ))
        );
    }

    #[test]
    fn non_bls_withdrawal_credentials_compounding_prefix() {
        let spec = make_spec();
        let mut state = make_gloas_state(&spec);

        // Change validator 0's withdrawal credentials to compounding prefix (0x02)
        let mut creds = [0u8; 32];
        creds[0] = spec.compounding_withdrawal_prefix_byte; // 0x02
        creds[12..].copy_from_slice(&[0xAA; 20]);
        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .withdrawal_credentials = Hash256::from_slice(&creds);

        let pubkey = state.validators().get(0).unwrap().pubkey;
        let signed_change = make_signed_change(0, pubkey, Address::repeat_byte(0xCC));
        let result =
            verify_bls_to_execution_change(&state, &signed_change, VerifySignatures::False, &spec);
        assert_eq!(
            result,
            Err(BlockOperationError::Invalid(
                Invalid::NonBlsWithdrawalCredentials
            ))
        );
    }

    #[test]
    fn withdrawal_credentials_mismatch_wrong_pubkey() {
        let spec = make_spec();
        let state = make_gloas_state(&spec);

        // Use a different pubkey that doesn't match validator 0's withdrawal credentials
        let wrong_pubkey = state.validators().get(1).unwrap().pubkey;
        let signed_change = make_signed_change(0, wrong_pubkey, Address::repeat_byte(0xCC));
        let result =
            verify_bls_to_execution_change(&state, &signed_change, VerifySignatures::False, &spec);
        assert_eq!(
            result,
            Err(BlockOperationError::Invalid(
                Invalid::WithdrawalCredentialsMismatch
            ))
        );
    }

    #[test]
    fn withdrawal_credentials_mismatch_empty_pubkey() {
        let spec = make_spec();
        let state = make_gloas_state(&spec);

        // Empty pubkey won't match
        let signed_change =
            make_signed_change(0, PublicKeyBytes::empty(), Address::repeat_byte(0xCC));
        let result =
            verify_bls_to_execution_change(&state, &signed_change, VerifySignatures::False, &spec);
        assert_eq!(
            result,
            Err(BlockOperationError::Invalid(
                Invalid::WithdrawalCredentialsMismatch
            ))
        );
    }

    #[test]
    fn non_bls_credentials_checked_before_pubkey_mismatch() {
        let spec = make_spec();
        let mut state = make_gloas_state(&spec);

        // Set eth1 withdrawal credentials with some random content
        let mut creds = [0u8; 32];
        creds[0] = spec.eth1_address_withdrawal_prefix_byte;
        creds[12..].copy_from_slice(&[0xBB; 20]);
        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .withdrawal_credentials = Hash256::from_slice(&creds);

        // Even though pubkey won't match either, NonBlsWithdrawalCredentials should fire first
        let signed_change =
            make_signed_change(0, PublicKeyBytes::empty(), Address::repeat_byte(0xCC));
        let result =
            verify_bls_to_execution_change(&state, &signed_change, VerifySignatures::False, &spec);
        assert_eq!(
            result,
            Err(BlockOperationError::Invalid(
                Invalid::NonBlsWithdrawalCredentials
            ))
        );
    }

    #[test]
    fn validator_unknown_checked_first() {
        let spec = make_spec();
        let state = make_gloas_state(&spec);

        // Large index won't exist — should fail with ValidatorUnknown before any credential check
        let signed_change =
            make_signed_change(12345, PublicKeyBytes::empty(), Address::repeat_byte(0xCC));
        let result =
            verify_bls_to_execution_change(&state, &signed_change, VerifySignatures::False, &spec);
        assert_eq!(
            result,
            Err(BlockOperationError::Invalid(Invalid::ValidatorUnknown(
                12345
            )))
        );
    }

    #[test]
    fn valid_change_any_to_execution_address() {
        let spec = make_spec();
        let state = make_gloas_state(&spec);
        let pubkey = state.validators().get(0).unwrap().pubkey;

        // The to_execution_address is not validated by this function
        let addresses = [
            Address::zero(),
            Address::repeat_byte(0xFF),
            Address::repeat_byte(0x01),
        ];
        for addr in &addresses {
            let signed_change = make_signed_change(0, pubkey, *addr);
            let result = verify_bls_to_execution_change(
                &state,
                &signed_change,
                VerifySignatures::False,
                &spec,
            );
            assert!(result.is_ok(), "should accept any to_execution_address");
        }
    }
}
