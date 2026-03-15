use integer_sqrt::IntegerSquareRoot;
use safe_arith::{ArithError, SafeArith};
use types::*;

/// This type exists to avoid confusing `total_active_balance` with `base_reward_per_increment`,
/// since they are used in close proximity and the same type (`u64`).
#[derive(Copy, Clone)]
pub struct BaseRewardPerIncrement(u64);

impl BaseRewardPerIncrement {
    pub fn new(total_active_balance: u64, spec: &ChainSpec) -> Result<Self, ArithError> {
        get_base_reward_per_increment(total_active_balance, spec).map(Self)
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

/// Returns the base reward for some validator.
///
/// The function has a different interface to the spec since it accepts the
/// `base_reward_per_increment` without computing it each time. Avoiding the re computation has
/// shown to be a significant optimisation.
///
/// Spec v1.1.0
pub fn get_base_reward(
    validator_effective_balance: u64,
    base_reward_per_increment: BaseRewardPerIncrement,
    spec: &ChainSpec,
) -> Result<u64, Error> {
    validator_effective_balance
        .safe_div(spec.effective_balance_increment)?
        .safe_mul(base_reward_per_increment.as_u64())
        .map_err(Into::into)
}

/// Returns the base reward for some validator.
///
/// Spec v1.1.0
fn get_base_reward_per_increment(
    total_active_balance: u64,
    spec: &ChainSpec,
) -> Result<u64, ArithError> {
    spec.effective_balance_increment
        .safe_mul(spec.base_reward_factor)?
        .safe_div(total_active_balance.integer_sqrt())
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
    fn base_reward_per_increment_basic() {
        let spec = spec();
        let total = spec.max_effective_balance * 100; // 100 validators
        let brpi = BaseRewardPerIncrement::new(total, &spec).unwrap();

        let expected =
            spec.effective_balance_increment * spec.base_reward_factor / total.integer_sqrt();
        assert_eq!(brpi.as_u64(), expected);
    }

    #[test]
    fn base_reward_per_increment_zero_total() {
        let spec = spec();
        // sqrt(0) = 0 → division by zero
        let result = BaseRewardPerIncrement::new(0, &spec);
        assert!(result.is_err());
    }

    #[test]
    fn altair_base_reward_formula() {
        let spec = spec();
        let total = spec.max_effective_balance * 100;
        let brpi = BaseRewardPerIncrement::new(total, &spec).unwrap();

        let eff_bal = spec.max_effective_balance;
        let reward = get_base_reward(eff_bal, brpi, &spec).unwrap();
        let expected = (eff_bal / spec.effective_balance_increment) * brpi.as_u64();
        assert_eq!(reward, expected);
    }

    #[test]
    fn altair_base_reward_zero_balance() {
        let spec = spec();
        let total = spec.max_effective_balance * 100;
        let brpi = BaseRewardPerIncrement::new(total, &spec).unwrap();

        let reward = get_base_reward(0, brpi, &spec).unwrap();
        assert_eq!(reward, 0);
    }

    #[test]
    fn altair_base_reward_proportional() {
        let spec = spec();
        let total = spec.max_effective_balance * 100;
        let brpi = BaseRewardPerIncrement::new(total, &spec).unwrap();

        let r1 = get_base_reward(spec.effective_balance_increment, brpi, &spec).unwrap();
        let r2 = get_base_reward(spec.effective_balance_increment * 3, brpi, &spec).unwrap();
        // 3x balance → 3x reward
        assert_eq!(r2, r1 * 3);
    }

    #[test]
    fn altair_base_reward_sub_increment_balance() {
        let spec = spec();
        let total = spec.max_effective_balance * 100;
        let brpi = BaseRewardPerIncrement::new(total, &spec).unwrap();

        // Balance less than one increment → integer division gives 0 increments → 0 reward
        let reward = get_base_reward(spec.effective_balance_increment - 1, brpi, &spec).unwrap();
        assert_eq!(reward, 0);
    }

    #[test]
    fn altair_more_total_balance_smaller_reward() {
        let spec = spec();
        let eff_bal = spec.max_effective_balance;

        let brpi_small = BaseRewardPerIncrement::new(eff_bal * 10, &spec).unwrap();
        let brpi_large = BaseRewardPerIncrement::new(eff_bal * 1000, &spec).unwrap();

        let r_small = get_base_reward(eff_bal, brpi_small, &spec).unwrap();
        let r_large = get_base_reward(eff_bal, brpi_large, &spec).unwrap();

        // More total stake → smaller per-validator reward
        assert!(r_small > r_large);
    }
}
