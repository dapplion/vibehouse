use crate::OpPoolError;
use bitvec::vec::BitVec;
use types::{
    BeaconState, BeaconStateError, Epoch, EthSpec, FixedBytesExtended, Hash256, ParticipationFlags,
};

#[derive(Debug, PartialEq, Eq, Clone)]
struct Initialization {
    current_epoch: Epoch,
    latest_block_root: Hash256,
}

/// Cache to store pre-computed information for block proposal.
#[derive(Debug, Clone, Default)]
pub struct RewardCache {
    initialization: Option<Initialization>,
    /// `BitVec` of validator indices which don't have default participation flags for the prev epoch.
    ///
    /// We choose to only track whether validators have *any* participation flag set because
    /// it's impossible to include a new attestation which is better than the existing participation
    /// UNLESS the validator makes a slashable attestation, and we assume that this is rare enough
    /// that it's acceptable to be slightly sub-optimal in this case.
    previous_epoch_participation: BitVec,
    /// `BitVec` of validator indices which don't have default participation flags for the current epoch.
    current_epoch_participation: BitVec,
}

impl RewardCache {
    pub fn has_attested_in_epoch(
        &self,
        validator_index: u64,
        epoch: Epoch,
    ) -> Result<bool, OpPoolError> {
        if let Some(init) = &self.initialization {
            if init.current_epoch == epoch {
                Ok(*self
                    .current_epoch_participation
                    .get(validator_index as usize)
                    .ok_or(OpPoolError::RewardCacheOutOfBounds)?)
            } else if init.current_epoch == epoch + 1 {
                Ok(*self
                    .previous_epoch_participation
                    .get(validator_index as usize)
                    .ok_or(OpPoolError::RewardCacheOutOfBounds)?)
            } else {
                Err(OpPoolError::RewardCacheWrongEpoch)
            }
        } else {
            Err(OpPoolError::RewardCacheWrongEpoch)
        }
    }

    /// Return the root of the latest block applied to `state`.
    ///
    /// For simplicity at genesis we return the zero hash, which will cause one unnecessary
    /// re-calculation in `update`.
    fn latest_block_root<E: EthSpec>(state: &BeaconState<E>) -> Result<Hash256, OpPoolError> {
        if state.slot() == 0 {
            Ok(Hash256::zero())
        } else {
            Ok(*state
                .get_block_root(state.slot() - 1)
                .map_err(OpPoolError::RewardCacheGetBlockRoot)?)
        }
    }

    /// Update the cache.
    pub fn update<E: EthSpec>(&mut self, state: &BeaconState<E>) -> Result<(), OpPoolError> {
        if matches!(state, BeaconState::Base(_)) {
            return Ok(());
        }

        let current_epoch = state.current_epoch();
        let latest_block_root = Self::latest_block_root(state)?;

        let new_init = Initialization {
            current_epoch,
            latest_block_root,
        };

        // The participation flags change every block, and will almost always need updating when
        // this function is called at a new slot.
        if self
            .initialization
            .as_ref()
            .is_none_or(|init| *init != new_init)
        {
            self.update_previous_epoch_participation(state)
                .map_err(OpPoolError::RewardCacheUpdatePrevEpoch)?;
            self.update_current_epoch_participation(state)
                .map_err(OpPoolError::RewardCacheUpdateCurrEpoch)?;

            self.initialization = Some(new_init);
        }

        Ok(())
    }

    fn update_previous_epoch_participation<E: EthSpec>(
        &mut self,
        state: &BeaconState<E>,
    ) -> Result<(), BeaconStateError> {
        let default_participation = ParticipationFlags::default();
        self.previous_epoch_participation = state
            .previous_epoch_participation()?
            .iter()
            .map(|participation| *participation != default_participation)
            .collect();
        Ok(())
    }

    fn update_current_epoch_participation<E: EthSpec>(
        &mut self,
        state: &BeaconState<E>,
    ) -> Result<(), BeaconStateError> {
        let default_participation = ParticipationFlags::default();
        self.current_epoch_participation = state
            .current_epoch_participation()?
            .iter()
            .map(|participation| *participation != default_participation)
            .collect();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use state_processing::initialize_beacon_state_from_eth1;
    use tree_hash::TreeHash;
    use types::{
        ChainSpec, DEPOSIT_TREE_DEPTH, Deposit, DepositData, FixedBytesExtended, FixedVector,
        ForkName, MinimalEthSpec, Signature, test_utils::generate_deterministic_keypairs,
    };

    type E = MinimalEthSpec;

    fn make_spec() -> ChainSpec {
        ForkName::Gloas.make_genesis_spec(E::default_spec())
    }

    fn make_genesis_state(spec: &ChainSpec) -> BeaconState<E> {
        let keypairs = generate_deterministic_keypairs(8);
        let mut deposit_datas = Vec::with_capacity(8);
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
        let mut tree =
            state_processing::common::DepositDataTree::create(&[], 0, DEPOSIT_TREE_DEPTH);
        let mut deposits = Vec::with_capacity(8);
        for data in deposit_datas {
            tree.push_leaf(data.tree_hash_root()).unwrap();
            let (_leaf, proof_vec) = tree.generate_proof(deposits.len()).unwrap();
            let mut proof = FixedVector::from(vec![Hash256::zero(); DEPOSIT_TREE_DEPTH + 1]);
            for (i, node) in proof_vec.iter().enumerate() {
                proof[i] = *node;
            }
            deposits.push(Deposit { proof, data });
        }
        initialize_beacon_state_from_eth1::<E>(
            Hash256::repeat_byte(0x42),
            2u64.pow(40),
            deposits,
            None,
            spec,
        )
        .unwrap()
    }

    #[test]
    fn default_cache_uninitialized() {
        let cache = RewardCache::default();
        assert!(cache.initialization.is_none());
    }

    #[test]
    fn has_attested_uninitialized_returns_wrong_epoch() {
        let cache = RewardCache::default();
        let result = cache.has_attested_in_epoch(0, Epoch::new(0));
        assert_eq!(result, Err(OpPoolError::RewardCacheWrongEpoch));
    }

    #[test]
    fn update_initializes_cache() {
        let spec = make_spec();
        let state = make_genesis_state(&spec);
        let mut cache = RewardCache::default();
        cache.update(&state).unwrap();
        assert!(cache.initialization.is_some());
        let init = cache.initialization.as_ref().unwrap();
        assert_eq!(init.current_epoch, state.current_epoch());
    }

    #[test]
    fn has_attested_current_epoch_default_participation() {
        let spec = make_spec();
        let state = make_genesis_state(&spec);
        let mut cache = RewardCache::default();
        cache.update(&state).unwrap();

        // At genesis, no one has participated yet — flags are all default
        for i in 0..8u64 {
            let attested = cache
                .has_attested_in_epoch(i, state.current_epoch())
                .unwrap();
            assert!(
                !attested,
                "validator {} should not have attested at genesis",
                i
            );
        }
    }

    #[test]
    fn has_attested_previous_epoch_default_participation() {
        let spec = make_spec();
        let state = make_genesis_state(&spec);
        let mut cache = RewardCache::default();
        cache.update(&state).unwrap();

        // previous_epoch = current_epoch - 1, query with current_epoch - 1
        let prev_epoch = state.current_epoch().saturating_sub(1u64);
        // At genesis (epoch 0), previous_epoch is also 0
        // has_attested checks current_epoch == epoch (match) or current_epoch == epoch + 1 (prev)
        // For epoch 0 and current_epoch 0: current_epoch == epoch → uses current_epoch_participation
        for i in 0..8u64 {
            let attested = cache.has_attested_in_epoch(i, prev_epoch).unwrap();
            assert!(!attested, "validator {} should not have attested", i);
        }
    }

    #[test]
    fn has_attested_wrong_epoch_returns_error() {
        let spec = make_spec();
        let state = make_genesis_state(&spec);
        let mut cache = RewardCache::default();
        cache.update(&state).unwrap();

        // Epoch 5 is far from current epoch 0
        let result = cache.has_attested_in_epoch(0, Epoch::new(5));
        assert_eq!(result, Err(OpPoolError::RewardCacheWrongEpoch));
    }

    #[test]
    fn has_attested_out_of_bounds_index() {
        let spec = make_spec();
        let state = make_genesis_state(&spec);
        let mut cache = RewardCache::default();
        cache.update(&state).unwrap();

        // Index 100 is out of bounds (only 8 validators)
        let result = cache.has_attested_in_epoch(100, state.current_epoch());
        assert_eq!(result, Err(OpPoolError::RewardCacheOutOfBounds));
    }

    #[test]
    fn has_attested_with_nondefault_participation() {
        let spec = make_spec();
        let mut state = make_genesis_state(&spec);
        let mut cache = RewardCache::default();

        // Set some participation flags on validator 3 for current epoch
        let mut flags = ParticipationFlags::default();
        flags.add_flag(0).unwrap();
        flags.add_flag(1).unwrap();
        flags.add_flag(2).unwrap();
        let participation = state.current_epoch_participation_mut().unwrap();
        *participation.get_mut(3).unwrap() = flags;

        cache.update(&state).unwrap();

        // Validator 3 should show as having attested
        assert!(
            cache
                .has_attested_in_epoch(3, state.current_epoch())
                .unwrap()
        );
        // Validator 0 should not (still default)
        assert!(
            !cache
                .has_attested_in_epoch(0, state.current_epoch())
                .unwrap()
        );
    }

    #[test]
    fn update_idempotent_same_block_root() {
        let spec = make_spec();
        let state = make_genesis_state(&spec);
        let mut cache = RewardCache::default();

        cache.update(&state).unwrap();
        let init1 = cache.initialization.clone();

        // Second update with same state should not change initialization
        cache.update(&state).unwrap();
        assert_eq!(cache.initialization, init1);
    }

    #[test]
    fn participation_bitvec_length_matches_validators() {
        let spec = make_spec();
        let state = make_genesis_state(&spec);
        let mut cache = RewardCache::default();
        cache.update(&state).unwrap();

        assert_eq!(cache.previous_epoch_participation.len(), 8);
        assert_eq!(cache.current_epoch_participation.len(), 8);
    }

    #[test]
    fn clone_preserves_state() {
        let spec = make_spec();
        let state = make_genesis_state(&spec);
        let mut cache = RewardCache::default();
        cache.update(&state).unwrap();

        let cloned = cache.clone();
        assert_eq!(cloned.initialization, cache.initialization);
        assert_eq!(
            cloned.previous_epoch_participation,
            cache.previous_epoch_participation
        );
        assert_eq!(
            cloned.current_epoch_participation,
            cache.current_epoch_participation
        );
    }
}
