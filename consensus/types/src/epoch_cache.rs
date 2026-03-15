use crate::{ActivationQueue, BeaconStateError, ChainSpec, Epoch, Hash256, Slot};
use safe_arith::{ArithError, SafeArith};
use std::sync::Arc;

/// Cache of values which are uniquely determined at the start of an epoch.
///
/// The values are fixed with respect to the last block of the _prior_ epoch, which we refer
/// to as the "decision block".
///
/// Prior to Fulu this cache was similar to the `BeaconProposerCache` in that beacon proposers were
/// determined at exactly the same time as the values in this cache, so the keys for the two caches
/// were identical.
///
/// Post-Fulu, we use a different key (the proposers have more lookahead).
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct EpochCache {
    inner: Option<Arc<Inner>>,
}

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Debug, PartialEq, Eq, Clone)]
struct Inner {
    /// Unique identifier for this cache, which can be used to check its validity before use
    /// with any `BeaconState`.
    key: EpochCacheKey,
    /// Effective balance for every validator in this epoch.
    effective_balances: Vec<u64>,
    /// Base rewards for every effective balance increment (currently 0..32 ETH).
    ///
    /// Keyed by `effective_balance / effective_balance_increment`.
    base_rewards: Vec<u64>,
    /// Validator activation queue.
    activation_queue: ActivationQueue,
    /// Effective balance increment.
    effective_balance_increment: u64,
}

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct EpochCacheKey {
    pub epoch: Epoch,
    pub decision_block_root: Hash256,
}

#[derive(Debug, PartialEq, Clone)]
pub enum EpochCacheError {
    IncorrectEpoch { cache: Epoch, state: Epoch },
    IncorrectDecisionBlock { cache: Hash256, state: Hash256 },
    ValidatorIndexOutOfBounds { validator_index: usize },
    EffectiveBalanceOutOfBounds { effective_balance_eth: usize },
    InvalidSlot { slot: Slot },
    Arith(ArithError),
    BeaconState(BeaconStateError),
    CacheNotInitialized,
}

impl From<BeaconStateError> for EpochCacheError {
    fn from(e: BeaconStateError) -> Self {
        Self::BeaconState(e)
    }
}

impl From<ArithError> for EpochCacheError {
    fn from(e: ArithError) -> Self {
        Self::Arith(e)
    }
}

impl EpochCache {
    pub fn new(
        key: EpochCacheKey,
        effective_balances: Vec<u64>,
        base_rewards: Vec<u64>,
        activation_queue: ActivationQueue,
        spec: &ChainSpec,
    ) -> EpochCache {
        Self {
            inner: Some(Arc::new(Inner {
                key,
                effective_balances,
                base_rewards,
                activation_queue,
                effective_balance_increment: spec.effective_balance_increment,
            })),
        }
    }

    pub fn check_validity(
        &self,
        current_epoch: Epoch,
        state_decision_root: Hash256,
    ) -> Result<(), EpochCacheError> {
        let cache = self
            .inner
            .as_ref()
            .ok_or(EpochCacheError::CacheNotInitialized)?;
        if cache.key.epoch != current_epoch {
            return Err(EpochCacheError::IncorrectEpoch {
                cache: cache.key.epoch,
                state: current_epoch,
            });
        }
        if cache.key.decision_block_root != state_decision_root {
            return Err(EpochCacheError::IncorrectDecisionBlock {
                cache: cache.key.decision_block_root,
                state: state_decision_root,
            });
        }
        Ok(())
    }

    #[inline]
    pub fn get_effective_balance(&self, validator_index: usize) -> Result<u64, EpochCacheError> {
        self.inner
            .as_ref()
            .ok_or(EpochCacheError::CacheNotInitialized)?
            .effective_balances
            .get(validator_index)
            .copied()
            .ok_or(EpochCacheError::ValidatorIndexOutOfBounds { validator_index })
    }

    #[inline]
    pub fn get_base_reward(&self, validator_index: usize) -> Result<u64, EpochCacheError> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(EpochCacheError::CacheNotInitialized)?;
        let effective_balance = self.get_effective_balance(validator_index)?;
        let effective_balance_eth =
            effective_balance.safe_div(inner.effective_balance_increment)? as usize;
        inner
            .base_rewards
            .get(effective_balance_eth)
            .copied()
            .ok_or(EpochCacheError::EffectiveBalanceOutOfBounds {
                effective_balance_eth,
            })
    }

    pub fn activation_queue(&self) -> Result<&ActivationQueue, EpochCacheError> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(EpochCacheError::CacheNotInitialized)?;
        Ok(&inner.activation_queue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FixedBytesExtended;

    fn spec() -> ChainSpec {
        ChainSpec::mainnet()
    }

    fn make_key(epoch: u64) -> EpochCacheKey {
        EpochCacheKey {
            epoch: Epoch::new(epoch),
            decision_block_root: Hash256::repeat_byte(0xaa),
        }
    }

    fn make_cache(epoch: u64) -> EpochCache {
        let s = spec();
        let ebi = s.effective_balance_increment;
        EpochCache::new(
            make_key(epoch),
            vec![32 * ebi, 16 * ebi, 0],
            vec![
                0, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000, 1100, 1200, 1300, 1400, 1500,
                1600, 1700, 1800, 1900, 2000, 2100, 2200, 2300, 2400, 2500, 2600, 2700, 2800, 2900,
                3000, 3100, 3200,
            ],
            ActivationQueue::default(),
            &s,
        )
    }

    #[test]
    fn default_is_uninitialized() {
        let cache = EpochCache::default();
        assert_eq!(
            cache.check_validity(Epoch::new(0), Hash256::zero()),
            Err(EpochCacheError::CacheNotInitialized)
        );
    }

    #[test]
    fn get_effective_balance_uninitialized() {
        let cache = EpochCache::default();
        assert_eq!(
            cache.get_effective_balance(0),
            Err(EpochCacheError::CacheNotInitialized)
        );
    }

    #[test]
    fn get_base_reward_uninitialized() {
        let cache = EpochCache::default();
        assert_eq!(
            cache.get_base_reward(0),
            Err(EpochCacheError::CacheNotInitialized)
        );
    }

    #[test]
    fn activation_queue_uninitialized() {
        let cache = EpochCache::default();
        assert_eq!(
            cache.activation_queue(),
            Err(EpochCacheError::CacheNotInitialized)
        );
    }

    #[test]
    fn check_validity_correct() {
        let cache = make_cache(5);
        assert!(
            cache
                .check_validity(Epoch::new(5), Hash256::repeat_byte(0xaa))
                .is_ok()
        );
    }

    #[test]
    fn check_validity_wrong_epoch() {
        let cache = make_cache(5);
        assert_eq!(
            cache.check_validity(Epoch::new(6), Hash256::repeat_byte(0xaa)),
            Err(EpochCacheError::IncorrectEpoch {
                cache: Epoch::new(5),
                state: Epoch::new(6),
            })
        );
    }

    #[test]
    fn check_validity_wrong_decision_block() {
        let cache = make_cache(5);
        assert_eq!(
            cache.check_validity(Epoch::new(5), Hash256::repeat_byte(0xbb)),
            Err(EpochCacheError::IncorrectDecisionBlock {
                cache: Hash256::repeat_byte(0xaa),
                state: Hash256::repeat_byte(0xbb),
            })
        );
    }

    #[test]
    fn get_effective_balance_valid() {
        let cache = make_cache(5);
        let ebi = spec().effective_balance_increment;
        assert_eq!(cache.get_effective_balance(0).unwrap(), 32 * ebi);
        assert_eq!(cache.get_effective_balance(1).unwrap(), 16 * ebi);
        assert_eq!(cache.get_effective_balance(2).unwrap(), 0);
    }

    #[test]
    fn get_effective_balance_out_of_bounds() {
        let cache = make_cache(5);
        assert_eq!(
            cache.get_effective_balance(3),
            Err(EpochCacheError::ValidatorIndexOutOfBounds { validator_index: 3 })
        );
    }

    #[test]
    fn get_base_reward_valid() {
        let cache = make_cache(5);
        // validator 0 has 32 ETH = index 32 in base_rewards
        assert_eq!(cache.get_base_reward(0).unwrap(), 3200);
        // validator 1 has 16 ETH = index 16 in base_rewards
        assert_eq!(cache.get_base_reward(1).unwrap(), 1600);
        // validator 2 has 0 ETH = index 0 in base_rewards
        assert_eq!(cache.get_base_reward(2).unwrap(), 0);
    }

    #[test]
    fn get_base_reward_validator_out_of_bounds() {
        let cache = make_cache(5);
        assert_eq!(
            cache.get_base_reward(99),
            Err(EpochCacheError::ValidatorIndexOutOfBounds {
                validator_index: 99
            })
        );
    }

    #[test]
    fn get_base_reward_effective_balance_out_of_bounds() {
        let s = spec();
        let ebi = s.effective_balance_increment;
        // Create cache where effective balance maps to index beyond base_rewards length
        let cache = EpochCache::new(
            make_key(1),
            vec![100 * ebi], // 100 ETH effective balance
            vec![0, 100],    // only 2 entries (indices 0..1)
            ActivationQueue::default(),
            &s,
        );
        assert_eq!(
            cache.get_base_reward(0),
            Err(EpochCacheError::EffectiveBalanceOutOfBounds {
                effective_balance_eth: 100
            })
        );
    }

    #[test]
    fn activation_queue_returns_ref() {
        let cache = make_cache(5);
        let queue = cache.activation_queue().unwrap();
        // Default activation queue should be empty
        assert_eq!(
            queue
                .get_validators_eligible_for_activation(Epoch::new(0), usize::MAX,)
                .len(),
            0
        );
    }

    #[test]
    fn clone_shares_arc() {
        let cache = make_cache(5);
        let cloned = cache.clone();
        assert_eq!(cache, cloned);
        // Both should be valid for the same key
        assert!(
            cloned
                .check_validity(Epoch::new(5), Hash256::repeat_byte(0xaa))
                .is_ok()
        );
    }
}
