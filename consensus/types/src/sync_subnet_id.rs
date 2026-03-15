//! Identifies each sync committee subnet by an integer identifier.
use crate::EthSpec;
use crate::consts::altair::SYNC_COMMITTEE_SUBNET_COUNT;
use safe_arith::{ArithError, SafeArith};
use serde::{Deserialize, Serialize};
use ssz_types::typenum::Unsigned;
use std::collections::HashSet;
use std::fmt::{self, Display};
use std::ops::{Deref, DerefMut};
use std::sync::LazyLock;

static SYNC_SUBNET_ID_TO_STRING: LazyLock<Vec<String>> = LazyLock::new(|| {
    let mut v = Vec::with_capacity(SYNC_COMMITTEE_SUBNET_COUNT as usize);

    for i in 0..SYNC_COMMITTEE_SUBNET_COUNT {
        v.push(i.to_string());
    }
    v
});

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SyncSubnetId(#[serde(with = "serde_utils::quoted_u64")] u64);

pub fn sync_subnet_id_to_string(i: u64) -> &'static str {
    if i < SYNC_COMMITTEE_SUBNET_COUNT {
        SYNC_SUBNET_ID_TO_STRING
            .get(i as usize)
            .expect("index below SYNC_COMMITTEE_SUBNET_COUNT")
    } else {
        "sync subnet id out of range"
    }
}

impl SyncSubnetId {
    pub fn new(id: u64) -> Self {
        id.into()
    }

    /// Compute required subnets to subscribe to given the sync committee indices.
    pub fn compute_subnets_for_sync_committee<E: EthSpec>(
        sync_committee_indices: &[u64],
    ) -> Result<HashSet<Self>, ArithError> {
        let subcommittee_size = E::SyncSubcommitteeSize::to_u64();

        sync_committee_indices
            .iter()
            .map(|index| index.safe_div(subcommittee_size).map(Self::new))
            .collect()
    }
}

impl Display for SyncSubnetId {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.0)
    }
}

impl Deref for SyncSubnetId {
    type Target = u64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for SyncSubnetId {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<u64> for SyncSubnetId {
    fn from(x: u64) -> Self {
        Self(x)
    }
}

impl From<SyncSubnetId> for u64 {
    fn from(from: SyncSubnetId) -> u64 {
        from.0
    }
}

impl From<&SyncSubnetId> for u64 {
    fn from(from: &SyncSubnetId) -> u64 {
        from.0
    }
}

impl AsRef<str> for SyncSubnetId {
    fn as_ref(&self) -> &str {
        sync_subnet_id_to_string(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MinimalEthSpec;

    #[test]
    fn new_and_deref() {
        let id = SyncSubnetId::new(3);
        assert_eq!(*id, 3u64);
    }

    #[test]
    fn from_u64_round_trip() {
        let id = SyncSubnetId::from(7u64);
        let val: u64 = id.into();
        assert_eq!(val, 7);
    }

    #[test]
    fn from_ref_to_u64() {
        let id = SyncSubnetId::new(5);
        let val: u64 = (&id).into();
        assert_eq!(val, 5);
    }

    #[test]
    fn display() {
        let id = SyncSubnetId::new(2);
        assert_eq!(format!("{}", id), "2");
    }

    #[test]
    fn as_ref_str_in_range() {
        let id = SyncSubnetId::new(0);
        assert_eq!(id.as_ref(), "0");

        let id = SyncSubnetId::new(SYNC_COMMITTEE_SUBNET_COUNT - 1);
        assert_eq!(
            id.as_ref(),
            (SYNC_COMMITTEE_SUBNET_COUNT - 1).to_string().as_str()
        );
    }

    #[test]
    fn as_ref_str_out_of_range() {
        let id = SyncSubnetId::new(SYNC_COMMITTEE_SUBNET_COUNT);
        assert_eq!(id.as_ref(), "sync subnet id out of range");
    }

    #[test]
    fn deref_mut() {
        let mut id = SyncSubnetId::new(1);
        *id = 10;
        assert_eq!(*id, 10);
    }

    #[test]
    fn compute_subnets_single_index() {
        let indices = vec![0];
        let subnets =
            SyncSubnetId::compute_subnets_for_sync_committee::<MinimalEthSpec>(&indices).unwrap();
        assert_eq!(subnets.len(), 1);
        assert!(subnets.contains(&SyncSubnetId::new(0)));
    }

    #[test]
    fn compute_subnets_multiple_same_subcommittee() {
        let subcommittee_size = <MinimalEthSpec as EthSpec>::SyncSubcommitteeSize::to_u64();
        let indices: Vec<u64> = (0..subcommittee_size).collect();
        let subnets =
            SyncSubnetId::compute_subnets_for_sync_committee::<MinimalEthSpec>(&indices).unwrap();
        assert_eq!(subnets.len(), 1);
        assert!(subnets.contains(&SyncSubnetId::new(0)));
    }

    #[test]
    fn compute_subnets_different_subcommittees() {
        let subcommittee_size = <MinimalEthSpec as EthSpec>::SyncSubcommitteeSize::to_u64();
        let indices = vec![0, subcommittee_size];
        let subnets =
            SyncSubnetId::compute_subnets_for_sync_committee::<MinimalEthSpec>(&indices).unwrap();
        assert_eq!(subnets.len(), 2);
        assert!(subnets.contains(&SyncSubnetId::new(0)));
        assert!(subnets.contains(&SyncSubnetId::new(1)));
    }

    #[test]
    fn compute_subnets_empty() {
        let subnets =
            SyncSubnetId::compute_subnets_for_sync_committee::<MinimalEthSpec>(&[]).unwrap();
        assert!(subnets.is_empty());
    }

    #[test]
    fn equality_and_hash() {
        let a = SyncSubnetId::new(3);
        let b = SyncSubnetId::new(3);
        let c = SyncSubnetId::new(4);
        assert_eq!(a, b);
        assert_ne!(a, c);

        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
        assert!(!set.contains(&c));
    }

    #[test]
    fn serde_round_trip() {
        let id = SyncSubnetId::new(3);
        let json = serde_json::to_string(&id).unwrap();
        let decoded: SyncSubnetId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, decoded);
    }
}
