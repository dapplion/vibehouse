use integer_sqrt::IntegerSquareRoot;
use safe_arith::{ArithError, SafeArith};
use types::*;

/// This type exists to avoid confusing `total_active_balance` with `sqrt_total_active_balance`,
/// since they are used in close proximity and have the same type (`u64`).
#[derive(Copy, Clone)]
pub struct SqrtTotalActiveBalance(u64);

impl SqrtTotalActiveBalance {
    pub fn new(total_active_balance: u64) -> Self {
        Self(total_active_balance.integer_sqrt())
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

/// Returns the base reward for some validator.
pub fn get_base_reward(
    validator_effective_balance: u64,
    sqrt_total_active_balance: SqrtTotalActiveBalance,
    spec: &ChainSpec,
) -> Result<u64, ArithError> {
    validator_effective_balance
        .safe_mul(spec.base_reward_factor)?
        .safe_div(sqrt_total_active_balance.as_u64())?
        .safe_div(spec.base_rewards_per_epoch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::MinimalEthSpec;

    type E = MinimalEthSpec;

    fn spec() -> ChainSpec {
        E::default_spec()
    }

    #[test]
    fn sqrt_total_active_balance_basic() {
        let s = SqrtTotalActiveBalance::new(100);
        assert_eq!(s.as_u64(), 10);
    }

    #[test]
    fn sqrt_total_active_balance_one() {
        let s = SqrtTotalActiveBalance::new(1);
        assert_eq!(s.as_u64(), 1);
    }

    #[test]
    fn sqrt_total_active_balance_non_perfect_square() {
        // sqrt(10) = 3 (integer)
        let s = SqrtTotalActiveBalance::new(10);
        assert_eq!(s.as_u64(), 3);
    }

    #[test]
    fn sqrt_total_active_balance_large() {
        // 32 ETH * 100 validators = 3_200_000_000_000 gwei
        let total = 3_200_000_000_000u64;
        let s = SqrtTotalActiveBalance::new(total);
        // sqrt(3_200_000_000_000) ≈ 1_788_854
        assert_eq!(s.as_u64(), (total as f64).sqrt() as u64);
    }

    #[test]
    fn base_reward_formula() {
        let spec = spec();
        let effective_balance = spec.max_effective_balance; // 32 ETH
        let total = effective_balance * 100; // 100 validators
        let sqrt = SqrtTotalActiveBalance::new(total);

        let reward = get_base_reward(effective_balance, sqrt, &spec).unwrap();
        // reward = eff_bal * base_reward_factor / sqrt(total) / base_rewards_per_epoch
        let expected = effective_balance * spec.base_reward_factor
            / sqrt.as_u64()
            / spec.base_rewards_per_epoch;
        assert_eq!(reward, expected);
    }

    #[test]
    fn base_reward_zero_balance() {
        let spec = spec();
        let sqrt = SqrtTotalActiveBalance::new(1_000_000_000_000);
        let reward = get_base_reward(0, sqrt, &spec).unwrap();
        assert_eq!(reward, 0);
    }

    #[test]
    fn base_reward_div_by_zero_sqrt() {
        let spec = spec();
        // SqrtTotalActiveBalance(0) → division by zero
        let sqrt = SqrtTotalActiveBalance::new(0);
        assert_eq!(sqrt.as_u64(), 0);
        let result = get_base_reward(spec.max_effective_balance, sqrt, &spec);
        assert!(result.is_err());
    }

    #[test]
    fn base_reward_proportional_to_balance() {
        let spec = spec();
        let sqrt = SqrtTotalActiveBalance::new(1_000_000_000_000);
        let r1 = get_base_reward(16_000_000_000, sqrt, &spec).unwrap();
        let r2 = get_base_reward(32_000_000_000, sqrt, &spec).unwrap();
        // Double balance → double reward (integer division)
        assert_eq!(r2, r1 * 2);
    }
}
