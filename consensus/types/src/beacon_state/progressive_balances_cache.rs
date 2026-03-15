use crate::beacon_state::balance::Balance;
use crate::{
    BeaconState, BeaconStateError, ChainSpec, Epoch, EthSpec, ParticipationFlags,
    consts::altair::{
        NUM_FLAG_INDICES, TIMELY_HEAD_FLAG_INDEX, TIMELY_SOURCE_FLAG_INDEX,
        TIMELY_TARGET_FLAG_INDEX,
    },
};
#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
use safe_arith::SafeArith;

/// This cache keeps track of the accumulated target attestation balance for the current & previous
/// epochs. The cached values can be utilised by fork choice to calculate unrealized justification
/// and finalization instead of converting epoch participation arrays to balances for each block we
/// process.
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
#[derive(Default, Debug, PartialEq, Clone)]
pub struct ProgressiveBalancesCache {
    inner: Option<Inner>,
}

#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
#[derive(Debug, PartialEq, Clone)]
struct Inner {
    pub current_epoch: Epoch,
    pub previous_epoch_cache: EpochTotalBalances,
    pub current_epoch_cache: EpochTotalBalances,
}

/// Caches the participation values for one epoch (either the previous or current).
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
#[derive(PartialEq, Debug, Clone)]
pub struct EpochTotalBalances {
    /// Stores the sum of the balances for all validators in `self.unslashed_participating_indices`
    /// for all flags in `NUM_FLAG_INDICES`.
    ///
    /// A flag balance is only incremented if a validator is in that flag set.
    pub total_flag_balances: [Balance; NUM_FLAG_INDICES],
}

impl EpochTotalBalances {
    pub fn new(spec: &ChainSpec) -> Self {
        let zero_balance = Balance::zero(spec.effective_balance_increment);

        Self {
            total_flag_balances: [zero_balance; NUM_FLAG_INDICES],
        }
    }

    /// Returns the total balance of attesters who have `flag_index` set.
    pub fn total_flag_balance(&self, flag_index: usize) -> Result<u64, BeaconStateError> {
        self.total_flag_balances
            .get(flag_index)
            .map(Balance::get)
            .ok_or(BeaconStateError::InvalidFlagIndex(flag_index))
    }

    /// Returns the raw total balance of attesters who have `flag_index` set.
    pub fn total_flag_balance_raw(&self, flag_index: usize) -> Result<Balance, BeaconStateError> {
        self.total_flag_balances
            .get(flag_index)
            .copied()
            .ok_or(BeaconStateError::InvalidFlagIndex(flag_index))
    }

    pub fn on_new_attestation(
        &mut self,
        is_slashed: bool,
        flag_index: usize,
        validator_effective_balance: u64,
    ) -> Result<(), BeaconStateError> {
        if is_slashed {
            return Ok(());
        }
        let balance = self
            .total_flag_balances
            .get_mut(flag_index)
            .ok_or(BeaconStateError::InvalidFlagIndex(flag_index))?;
        balance.safe_add_assign(validator_effective_balance)?;
        Ok(())
    }

    pub fn on_slashing(
        &mut self,
        participation_flags: ParticipationFlags,
        validator_effective_balance: u64,
    ) -> Result<(), BeaconStateError> {
        for flag_index in 0..NUM_FLAG_INDICES {
            if participation_flags.has_flag(flag_index)? {
                self.total_flag_balances
                    .get_mut(flag_index)
                    .ok_or(BeaconStateError::InvalidFlagIndex(flag_index))?
                    .safe_sub_assign(validator_effective_balance)?;
            }
        }
        Ok(())
    }

    pub fn on_effective_balance_change(
        &mut self,
        is_slashed: bool,
        current_epoch_participation_flags: ParticipationFlags,
        old_effective_balance: u64,
        new_effective_balance: u64,
    ) -> Result<(), BeaconStateError> {
        // If the validator is slashed then we should not update the effective balance, because this
        // validator's effective balance has already been removed from the totals.
        if is_slashed {
            return Ok(());
        }
        for flag_index in 0..NUM_FLAG_INDICES {
            if current_epoch_participation_flags.has_flag(flag_index)? {
                let total = self
                    .total_flag_balances
                    .get_mut(flag_index)
                    .ok_or(BeaconStateError::InvalidFlagIndex(flag_index))?;
                if new_effective_balance > old_effective_balance {
                    total
                        .safe_add_assign(new_effective_balance.safe_sub(old_effective_balance)?)?;
                } else {
                    total
                        .safe_sub_assign(old_effective_balance.safe_sub(new_effective_balance)?)?;
                }
            }
        }
        Ok(())
    }
}

impl ProgressiveBalancesCache {
    pub fn initialize(
        &mut self,
        current_epoch: Epoch,
        previous_epoch_cache: EpochTotalBalances,
        current_epoch_cache: EpochTotalBalances,
    ) {
        self.inner = Some(Inner {
            current_epoch,
            previous_epoch_cache,
            current_epoch_cache,
        });
    }

    pub fn is_initialized(&self) -> bool {
        self.inner.is_some()
    }

    pub fn is_initialized_at(&self, epoch: Epoch) -> bool {
        self.inner
            .as_ref()
            .is_some_and(|inner| inner.current_epoch == epoch)
    }

    /// When a new target attestation has been processed, we update the cached
    /// `current_epoch_target_attesting_balance` to include the validator effective balance.
    /// If the epoch is neither the current epoch nor the previous epoch, an error is returned.
    pub fn on_new_attestation(
        &mut self,
        epoch: Epoch,
        is_slashed: bool,
        flag_index: usize,
        validator_effective_balance: u64,
    ) -> Result<(), BeaconStateError> {
        let cache = self.get_inner_mut()?;

        if epoch == cache.current_epoch {
            cache.current_epoch_cache.on_new_attestation(
                is_slashed,
                flag_index,
                validator_effective_balance,
            )?;
        } else if epoch.safe_add(1)? == cache.current_epoch {
            cache.previous_epoch_cache.on_new_attestation(
                is_slashed,
                flag_index,
                validator_effective_balance,
            )?;
        } else {
            return Err(BeaconStateError::ProgressiveBalancesCacheInconsistent);
        }

        Ok(())
    }

    /// When a validator is slashed, we reduce the `current_epoch_target_attesting_balance` by the
    /// validator's effective balance to exclude the validator weight.
    pub fn on_slashing(
        &mut self,
        previous_epoch_participation: ParticipationFlags,
        current_epoch_participation: ParticipationFlags,
        effective_balance: u64,
    ) -> Result<(), BeaconStateError> {
        let cache = self.get_inner_mut()?;
        cache
            .previous_epoch_cache
            .on_slashing(previous_epoch_participation, effective_balance)?;
        cache
            .current_epoch_cache
            .on_slashing(current_epoch_participation, effective_balance)?;
        Ok(())
    }

    /// When a current epoch target attester has its effective balance changed, we adjust the
    /// its share of the target attesting balance in the cache.
    pub fn on_effective_balance_change(
        &mut self,
        is_slashed: bool,
        current_epoch_participation: ParticipationFlags,
        old_effective_balance: u64,
        new_effective_balance: u64,
    ) -> Result<(), BeaconStateError> {
        let cache = self.get_inner_mut()?;
        cache.current_epoch_cache.on_effective_balance_change(
            is_slashed,
            current_epoch_participation,
            old_effective_balance,
            new_effective_balance,
        )?;
        Ok(())
    }

    /// On epoch transition, the balance from current epoch is shifted to previous epoch, and the
    /// current epoch balance is reset to 0.
    pub fn on_epoch_transition(&mut self, spec: &ChainSpec) -> Result<(), BeaconStateError> {
        let cache = self.get_inner_mut()?;
        cache.current_epoch.safe_add_assign(1)?;
        cache.previous_epoch_cache = std::mem::replace(
            &mut cache.current_epoch_cache,
            EpochTotalBalances::new(spec),
        );
        Ok(())
    }

    pub fn previous_epoch_flag_attesting_balance(
        &self,
        flag_index: usize,
    ) -> Result<u64, BeaconStateError> {
        self.get_inner()?
            .previous_epoch_cache
            .total_flag_balance(flag_index)
    }

    pub fn current_epoch_flag_attesting_balance(
        &self,
        flag_index: usize,
    ) -> Result<u64, BeaconStateError> {
        self.get_inner()?
            .current_epoch_cache
            .total_flag_balance(flag_index)
    }

    pub fn previous_epoch_source_attesting_balance(&self) -> Result<u64, BeaconStateError> {
        self.previous_epoch_flag_attesting_balance(TIMELY_SOURCE_FLAG_INDEX)
    }

    pub fn previous_epoch_target_attesting_balance(&self) -> Result<u64, BeaconStateError> {
        self.previous_epoch_flag_attesting_balance(TIMELY_TARGET_FLAG_INDEX)
    }

    pub fn previous_epoch_head_attesting_balance(&self) -> Result<u64, BeaconStateError> {
        self.previous_epoch_flag_attesting_balance(TIMELY_HEAD_FLAG_INDEX)
    }

    pub fn current_epoch_source_attesting_balance(&self) -> Result<u64, BeaconStateError> {
        self.current_epoch_flag_attesting_balance(TIMELY_SOURCE_FLAG_INDEX)
    }

    pub fn current_epoch_target_attesting_balance(&self) -> Result<u64, BeaconStateError> {
        self.current_epoch_flag_attesting_balance(TIMELY_TARGET_FLAG_INDEX)
    }

    pub fn current_epoch_head_attesting_balance(&self) -> Result<u64, BeaconStateError> {
        self.current_epoch_flag_attesting_balance(TIMELY_HEAD_FLAG_INDEX)
    }

    fn get_inner_mut(&mut self) -> Result<&mut Inner, BeaconStateError> {
        self.inner
            .as_mut()
            .ok_or(BeaconStateError::ProgressiveBalancesCacheNotInitialized)
    }

    fn get_inner(&self) -> Result<&Inner, BeaconStateError> {
        self.inner
            .as_ref()
            .ok_or(BeaconStateError::ProgressiveBalancesCacheNotInitialized)
    }
}

/// `ProgressiveBalancesCache` is only enabled from `Altair` as it uses Altair-specific logic.
pub fn is_progressive_balances_enabled<E: EthSpec>(state: &BeaconState<E>) -> bool {
    state.fork_name_unchecked().altair_enabled()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ChainSpec {
        ChainSpec::minimal()
    }

    /// The minimum balance returned by `Balance::get()` when raw is 0.
    fn min_bal() -> u64 {
        spec().effective_balance_increment
    }

    fn new_balances() -> EpochTotalBalances {
        EpochTotalBalances::new(&spec())
    }

    fn make_cache(epoch: u64) -> ProgressiveBalancesCache {
        let mut cache = ProgressiveBalancesCache::default();
        cache.initialize(Epoch::new(epoch), new_balances(), new_balances());
        cache
    }

    // ── EpochTotalBalances tests ──

    #[test]
    fn epoch_total_balances_new_returns_minimum() {
        // Balance::get() returns max(raw, minimum) — so "zero" returns effective_balance_increment
        let balances = new_balances();
        for i in 0..NUM_FLAG_INDICES {
            assert_eq!(balances.total_flag_balance(i).unwrap(), min_bal());
        }
    }

    #[test]
    fn epoch_total_balances_invalid_flag_index() {
        let balances = new_balances();
        assert!(balances.total_flag_balance(NUM_FLAG_INDICES).is_err());
    }

    #[test]
    fn epoch_total_balances_on_new_attestation_unslashed() {
        let mut balances = new_balances();
        balances
            .on_new_attestation(false, TIMELY_TARGET_FLAG_INDEX, 32_000_000_000)
            .unwrap();
        assert_eq!(
            balances
                .total_flag_balance(TIMELY_TARGET_FLAG_INDEX)
                .unwrap(),
            32_000_000_000
        );
        // Other flags unchanged — still at minimum
        assert_eq!(
            balances
                .total_flag_balance(TIMELY_SOURCE_FLAG_INDEX)
                .unwrap(),
            min_bal()
        );
    }

    #[test]
    fn epoch_total_balances_on_new_attestation_slashed_ignored() {
        let mut balances = new_balances();
        balances
            .on_new_attestation(true, TIMELY_TARGET_FLAG_INDEX, 32_000_000_000)
            .unwrap();
        // Raw stayed at 0, get() returns minimum
        assert_eq!(
            balances
                .total_flag_balance(TIMELY_TARGET_FLAG_INDEX)
                .unwrap(),
            min_bal()
        );
    }

    #[test]
    fn epoch_total_balances_on_slashing_subtracts() {
        let mut balances = new_balances();
        balances
            .on_new_attestation(false, TIMELY_TARGET_FLAG_INDEX, 32_000_000_000)
            .unwrap();
        balances
            .on_new_attestation(false, TIMELY_SOURCE_FLAG_INDEX, 32_000_000_000)
            .unwrap();

        let mut flags = ParticipationFlags::default();
        flags.add_flag(TIMELY_TARGET_FLAG_INDEX).unwrap();

        balances.on_slashing(flags, 32_000_000_000).unwrap();
        // Back to minimum after subtracting all added balance
        assert_eq!(
            balances
                .total_flag_balance(TIMELY_TARGET_FLAG_INDEX)
                .unwrap(),
            min_bal()
        );
        // Source not affected (flag wasn't set)
        assert_eq!(
            balances
                .total_flag_balance(TIMELY_SOURCE_FLAG_INDEX)
                .unwrap(),
            32_000_000_000
        );
    }

    #[test]
    fn epoch_total_balances_effective_balance_change_increase() {
        let mut balances = new_balances();
        balances
            .on_new_attestation(false, TIMELY_TARGET_FLAG_INDEX, 16_000_000_000)
            .unwrap();

        let mut flags = ParticipationFlags::default();
        flags.add_flag(TIMELY_TARGET_FLAG_INDEX).unwrap();

        balances
            .on_effective_balance_change(false, flags, 16_000_000_000, 32_000_000_000)
            .unwrap();
        assert_eq!(
            balances
                .total_flag_balance(TIMELY_TARGET_FLAG_INDEX)
                .unwrap(),
            32_000_000_000
        );
    }

    #[test]
    fn epoch_total_balances_effective_balance_change_decrease() {
        let mut balances = new_balances();
        balances
            .on_new_attestation(false, TIMELY_TARGET_FLAG_INDEX, 32_000_000_000)
            .unwrap();

        let mut flags = ParticipationFlags::default();
        flags.add_flag(TIMELY_TARGET_FLAG_INDEX).unwrap();

        balances
            .on_effective_balance_change(false, flags, 32_000_000_000, 16_000_000_000)
            .unwrap();
        assert_eq!(
            balances
                .total_flag_balance(TIMELY_TARGET_FLAG_INDEX)
                .unwrap(),
            16_000_000_000
        );
    }

    #[test]
    fn epoch_total_balances_effective_balance_change_slashed_ignored() {
        let mut balances = new_balances();
        balances
            .on_new_attestation(false, TIMELY_TARGET_FLAG_INDEX, 32_000_000_000)
            .unwrap();

        let mut flags = ParticipationFlags::default();
        flags.add_flag(TIMELY_TARGET_FLAG_INDEX).unwrap();

        balances
            .on_effective_balance_change(true, flags, 32_000_000_000, 0)
            .unwrap();
        assert_eq!(
            balances
                .total_flag_balance(TIMELY_TARGET_FLAG_INDEX)
                .unwrap(),
            32_000_000_000
        );
    }

    // ── ProgressiveBalancesCache tests ──

    #[test]
    fn default_is_not_initialized() {
        let cache = ProgressiveBalancesCache::default();
        assert!(!cache.is_initialized());
        assert!(!cache.is_initialized_at(Epoch::new(0)));
    }

    #[test]
    fn initialize_sets_epoch() {
        let cache = make_cache(5);
        assert!(cache.is_initialized());
        assert!(cache.is_initialized_at(Epoch::new(5)));
        assert!(!cache.is_initialized_at(Epoch::new(4)));
    }

    #[test]
    fn uninitialized_errors_on_query() {
        let cache = ProgressiveBalancesCache::default();
        assert!(cache.previous_epoch_target_attesting_balance().is_err());
        assert!(cache.current_epoch_target_attesting_balance().is_err());
    }

    #[test]
    fn on_new_attestation_current_epoch() {
        let mut cache = make_cache(10);
        cache
            .on_new_attestation(
                Epoch::new(10),
                false,
                TIMELY_TARGET_FLAG_INDEX,
                32_000_000_000,
            )
            .unwrap();
        assert_eq!(
            cache.current_epoch_target_attesting_balance().unwrap(),
            32_000_000_000
        );
        // Previous epoch untouched — at minimum
        assert_eq!(
            cache.previous_epoch_target_attesting_balance().unwrap(),
            min_bal()
        );
    }

    #[test]
    fn on_new_attestation_previous_epoch() {
        let mut cache = make_cache(10);
        cache
            .on_new_attestation(
                Epoch::new(9),
                false,
                TIMELY_TARGET_FLAG_INDEX,
                32_000_000_000,
            )
            .unwrap();
        assert_eq!(
            cache.previous_epoch_target_attesting_balance().unwrap(),
            32_000_000_000
        );
        assert_eq!(
            cache.current_epoch_target_attesting_balance().unwrap(),
            min_bal()
        );
    }

    #[test]
    fn on_new_attestation_wrong_epoch_errors() {
        let mut cache = make_cache(10);
        assert!(
            cache
                .on_new_attestation(
                    Epoch::new(8),
                    false,
                    TIMELY_TARGET_FLAG_INDEX,
                    32_000_000_000
                )
                .is_err()
        );
    }

    #[test]
    fn on_epoch_transition_shifts_balances() {
        let mut cache = make_cache(10);
        cache
            .on_new_attestation(
                Epoch::new(10),
                false,
                TIMELY_TARGET_FLAG_INDEX,
                32_000_000_000,
            )
            .unwrap();

        cache.on_epoch_transition(&spec()).unwrap();

        assert!(cache.is_initialized_at(Epoch::new(11)));
        // Previous epoch now has the old current balance
        assert_eq!(
            cache.previous_epoch_target_attesting_balance().unwrap(),
            32_000_000_000
        );
        // Current epoch is reset — returns minimum
        assert_eq!(
            cache.current_epoch_target_attesting_balance().unwrap(),
            min_bal()
        );
    }

    #[test]
    fn on_slashing_reduces_both_epochs() {
        let mut cache = make_cache(10);
        cache
            .on_new_attestation(
                Epoch::new(10),
                false,
                TIMELY_TARGET_FLAG_INDEX,
                32_000_000_000,
            )
            .unwrap();
        cache
            .on_new_attestation(
                Epoch::new(9),
                false,
                TIMELY_TARGET_FLAG_INDEX,
                32_000_000_000,
            )
            .unwrap();

        let mut prev_flags = ParticipationFlags::default();
        prev_flags.add_flag(TIMELY_TARGET_FLAG_INDEX).unwrap();
        let mut curr_flags = ParticipationFlags::default();
        curr_flags.add_flag(TIMELY_TARGET_FLAG_INDEX).unwrap();

        cache
            .on_slashing(prev_flags, curr_flags, 32_000_000_000)
            .unwrap();

        // Both reduced back to minimum
        assert_eq!(
            cache.previous_epoch_target_attesting_balance().unwrap(),
            min_bal()
        );
        assert_eq!(
            cache.current_epoch_target_attesting_balance().unwrap(),
            min_bal()
        );
    }

    #[test]
    fn source_head_balance_accessors() {
        let mut cache = make_cache(5);
        cache
            .on_new_attestation(
                Epoch::new(5),
                false,
                TIMELY_SOURCE_FLAG_INDEX,
                5_000_000_000,
            )
            .unwrap();
        cache
            .on_new_attestation(Epoch::new(5), false, TIMELY_HEAD_FLAG_INDEX, 10_000_000_000)
            .unwrap();
        cache
            .on_new_attestation(
                Epoch::new(4),
                false,
                TIMELY_SOURCE_FLAG_INDEX,
                15_000_000_000,
            )
            .unwrap();
        cache
            .on_new_attestation(Epoch::new(4), false, TIMELY_HEAD_FLAG_INDEX, 20_000_000_000)
            .unwrap();

        assert_eq!(
            cache.current_epoch_source_attesting_balance().unwrap(),
            5_000_000_000
        );
        assert_eq!(
            cache.current_epoch_head_attesting_balance().unwrap(),
            10_000_000_000
        );
        assert_eq!(
            cache.previous_epoch_source_attesting_balance().unwrap(),
            15_000_000_000
        );
        assert_eq!(
            cache.previous_epoch_head_attesting_balance().unwrap(),
            20_000_000_000
        );
    }

    #[test]
    fn on_effective_balance_change_through_cache() {
        let mut cache = make_cache(10);
        cache
            .on_new_attestation(
                Epoch::new(10),
                false,
                TIMELY_TARGET_FLAG_INDEX,
                32_000_000_000,
            )
            .unwrap();

        let mut flags = ParticipationFlags::default();
        flags.add_flag(TIMELY_TARGET_FLAG_INDEX).unwrap();

        cache
            .on_effective_balance_change(false, flags, 32_000_000_000, 16_000_000_000)
            .unwrap();
        assert_eq!(
            cache.current_epoch_target_attesting_balance().unwrap(),
            16_000_000_000
        );
    }

    #[test]
    fn uninitialized_errors_on_mutation() {
        let mut cache = ProgressiveBalancesCache::default();
        assert!(
            cache
                .on_new_attestation(Epoch::new(0), false, TIMELY_TARGET_FLAG_INDEX, 100)
                .is_err()
        );
        assert!(cache.on_epoch_transition(&spec()).is_err());
    }
}
