#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
use safe_arith::{ArithError, SafeArith};

/// A balance which will never be below the specified `minimum`.
///
/// This is an effort to ensure the `EFFECTIVE_BALANCE_INCREMENT` minimum is always respected.
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
#[derive(PartialEq, Debug, Clone, Copy)]
pub struct Balance {
    raw: u64,
    minimum: u64,
}

impl Balance {
    /// Initialize the balance to `0`, or the given `minimum`.
    pub fn zero(minimum: u64) -> Self {
        Self { raw: 0, minimum }
    }

    /// Returns the balance with respect to the initialization `minimum`.
    pub fn get(&self) -> u64 {
        std::cmp::max(self.raw, self.minimum)
    }

    /// Add-assign to the balance.
    pub fn safe_add_assign(&mut self, other: u64) -> Result<(), ArithError> {
        self.raw.safe_add_assign(other)
    }

    /// Sub-assign to the balance.
    pub fn safe_sub_assign(&mut self, other: u64) -> Result<(), ArithError> {
        self.raw.safe_sub_assign(other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_returns_minimum() {
        let b = Balance::zero(100);
        assert_eq!(b.get(), 100);
    }

    #[test]
    fn zero_minimum_returns_zero() {
        let b = Balance::zero(0);
        assert_eq!(b.get(), 0);
    }

    #[test]
    fn add_above_minimum() {
        let mut b = Balance::zero(100);
        b.safe_add_assign(200).unwrap();
        assert_eq!(b.get(), 200);
    }

    #[test]
    fn add_below_minimum_returns_minimum() {
        let mut b = Balance::zero(100);
        b.safe_add_assign(50).unwrap();
        assert_eq!(b.get(), 100);
    }

    #[test]
    fn sub_from_added() {
        let mut b = Balance::zero(10);
        b.safe_add_assign(100).unwrap();
        b.safe_sub_assign(30).unwrap();
        assert_eq!(b.get(), 70);
    }

    #[test]
    fn sub_below_zero_errors() {
        let mut b = Balance::zero(0);
        b.safe_add_assign(10).unwrap();
        assert!(b.safe_sub_assign(11).is_err());
    }

    #[test]
    fn add_overflow_errors() {
        let mut b = Balance::zero(0);
        b.safe_add_assign(u64::MAX).unwrap();
        assert!(b.safe_add_assign(1).is_err());
    }

    #[test]
    fn sub_to_zero_returns_minimum() {
        let mut b = Balance::zero(50);
        b.safe_add_assign(100).unwrap();
        b.safe_sub_assign(100).unwrap();
        // raw is 0, minimum is 50
        assert_eq!(b.get(), 50);
    }

    #[test]
    fn get_returns_max_of_raw_and_minimum() {
        let mut b = Balance::zero(75);
        b.safe_add_assign(75).unwrap();
        // raw == minimum
        assert_eq!(b.get(), 75);

        b.safe_add_assign(1).unwrap();
        assert_eq!(b.get(), 76);
    }
}
