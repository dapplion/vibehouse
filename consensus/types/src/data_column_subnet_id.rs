//! Identifies each data column subnet by an integer identifier.
use crate::ChainSpec;
use crate::data_column_sidecar::ColumnIndex;
use safe_arith::{ArithError, SafeArith};
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display};
use std::ops::{Deref, DerefMut};

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DataColumnSubnetId(#[serde(with = "serde_utils::quoted_u64")] u64);

impl fmt::Debug for DataColumnSubnetId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl DataColumnSubnetId {
    pub fn new(id: u64) -> Self {
        id.into()
    }

    pub fn from_column_index(column_index: ColumnIndex, spec: &ChainSpec) -> Self {
        column_index
            .safe_rem(spec.data_column_sidecar_subnet_count)
            .expect(
                "data_column_sidecar_subnet_count should never be zero if this function is called",
            )
            .into()
    }
}

impl Display for DataColumnSubnetId {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.0)
    }
}

impl Deref for DataColumnSubnetId {
    type Target = u64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for DataColumnSubnetId {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<u64> for DataColumnSubnetId {
    fn from(x: u64) -> Self {
        Self(x)
    }
}

impl From<DataColumnSubnetId> for u64 {
    fn from(val: DataColumnSubnetId) -> Self {
        val.0
    }
}

impl From<&DataColumnSubnetId> for u64 {
    fn from(val: &DataColumnSubnetId) -> Self {
        val.0
    }
}

#[derive(Debug)]
pub enum Error {
    ArithError(ArithError),
    InvalidCustodySubnetCount(u64),
}

impl From<ArithError> for Error {
    fn from(e: ArithError) -> Self {
        Error::ArithError(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn spec_with_subnet_count(count: u64) -> ChainSpec {
        let mut spec = ChainSpec::minimal();
        spec.data_column_sidecar_subnet_count = count;
        spec
    }

    #[test]
    fn new_and_deref() {
        let id = DataColumnSubnetId::new(5);
        assert_eq!(*id, 5u64);
    }

    #[test]
    fn from_u64_round_trip() {
        let id = DataColumnSubnetId::from(42u64);
        let val: u64 = id.into();
        assert_eq!(val, 42);
    }

    #[test]
    fn from_ref_to_u64() {
        let id = DataColumnSubnetId::new(7);
        let val: u64 = (&id).into();
        assert_eq!(val, 7);
    }

    #[test]
    fn display() {
        let id = DataColumnSubnetId::new(13);
        assert_eq!(format!("{}", id), "13");
    }

    #[test]
    fn debug() {
        let id = DataColumnSubnetId::new(8);
        assert_eq!(format!("{:?}", id), "8");
    }

    #[test]
    fn from_column_index_wraps() {
        let spec = spec_with_subnet_count(4);
        assert_eq!(*DataColumnSubnetId::from_column_index(0, &spec), 0);
        assert_eq!(*DataColumnSubnetId::from_column_index(1, &spec), 1);
        assert_eq!(*DataColumnSubnetId::from_column_index(3, &spec), 3);
        assert_eq!(*DataColumnSubnetId::from_column_index(4, &spec), 0);
        assert_eq!(*DataColumnSubnetId::from_column_index(5, &spec), 1);
        assert_eq!(*DataColumnSubnetId::from_column_index(7, &spec), 3);
    }

    #[test]
    fn from_column_index_single_subnet() {
        let spec = spec_with_subnet_count(1);
        // All columns map to subnet 0
        for i in 0..10 {
            assert_eq!(*DataColumnSubnetId::from_column_index(i, &spec), 0);
        }
    }

    #[test]
    fn deref_mut() {
        let mut id = DataColumnSubnetId::new(1);
        *id = 99;
        assert_eq!(*id, 99);
    }

    #[test]
    fn equality_and_hash() {
        let a = DataColumnSubnetId::new(3);
        let b = DataColumnSubnetId::new(3);
        let c = DataColumnSubnetId::new(4);
        assert_eq!(a, b);
        assert_ne!(a, c);

        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
        assert!(!set.contains(&c));
    }

    #[test]
    fn serde_round_trip() {
        let id = DataColumnSubnetId::new(10);
        let json = serde_json::to_string(&id).unwrap();
        let decoded: DataColumnSubnetId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, decoded);
    }

    #[test]
    fn error_from_arith_error() {
        let arith_err = ArithError::Overflow;
        let err = Error::from(arith_err);
        assert!(matches!(err, Error::ArithError(ArithError::Overflow)));
    }
}
