use crate::context_deserialize;
use crate::test_utils::TestRandom;
use crate::{ForkName, Hash256};

use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash::TreeHash;
use tree_hash_derive::TreeHash;

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(
    Debug, PartialEq, Clone, Copy, Serialize, Deserialize, Encode, Decode, TreeHash, TestRandom,
)]
#[context_deserialize(ForkName)]
pub struct SigningData {
    pub object_root: Hash256,
    pub domain: Hash256,
}

pub trait SignedRoot: TreeHash {
    fn signing_root(&self, domain: Hash256) -> Hash256 {
        SigningData {
            object_root: self.tree_hash_root(),
            domain,
        }
        .tree_hash_root()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FixedBytesExtended;
    use tree_hash_derive::TreeHash;

    #[derive(TreeHash)]
    struct TestContainer {
        value: u64,
    }

    impl SignedRoot for TestContainer {}

    #[test]
    fn signing_data_tree_hash_deterministic() {
        let sd1 = SigningData {
            object_root: Hash256::from_low_u64_be(42),
            domain: Hash256::from_low_u64_be(99),
        };
        let sd2 = SigningData {
            object_root: Hash256::from_low_u64_be(42),
            domain: Hash256::from_low_u64_be(99),
        };
        assert_eq!(sd1.tree_hash_root(), sd2.tree_hash_root());
    }

    #[test]
    fn signing_data_different_domain_different_root() {
        let sd1 = SigningData {
            object_root: Hash256::from_low_u64_be(42),
            domain: Hash256::from_low_u64_be(1),
        };
        let sd2 = SigningData {
            object_root: Hash256::from_low_u64_be(42),
            domain: Hash256::from_low_u64_be(2),
        };
        assert_ne!(sd1.tree_hash_root(), sd2.tree_hash_root());
    }

    #[test]
    fn signing_data_different_object_root_different_root() {
        let sd1 = SigningData {
            object_root: Hash256::from_low_u64_be(1),
            domain: Hash256::from_low_u64_be(99),
        };
        let sd2 = SigningData {
            object_root: Hash256::from_low_u64_be(2),
            domain: Hash256::from_low_u64_be(99),
        };
        assert_ne!(sd1.tree_hash_root(), sd2.tree_hash_root());
    }

    #[test]
    fn signed_root_signing_root_deterministic() {
        let tc = TestContainer { value: 123 };
        let domain = Hash256::from_low_u64_be(55);
        let root1 = tc.signing_root(domain);
        let root2 = tc.signing_root(domain);
        assert_eq!(root1, root2);
    }

    #[test]
    fn signed_root_different_domain_different_signing_root() {
        let tc = TestContainer { value: 123 };
        let root1 = tc.signing_root(Hash256::from_low_u64_be(1));
        let root2 = tc.signing_root(Hash256::from_low_u64_be(2));
        assert_ne!(root1, root2);
    }

    #[test]
    fn signed_root_different_value_different_signing_root() {
        let tc1 = TestContainer { value: 1 };
        let tc2 = TestContainer { value: 2 };
        let domain = Hash256::from_low_u64_be(99);
        assert_ne!(tc1.signing_root(domain), tc2.signing_root(domain));
    }

    #[test]
    fn signed_root_matches_manual_construction() {
        let tc = TestContainer { value: 42 };
        let domain = Hash256::from_low_u64_be(7);
        let manual = SigningData {
            object_root: tc.tree_hash_root(),
            domain,
        }
        .tree_hash_root();
        assert_eq!(tc.signing_root(domain), manual);
    }

    #[test]
    fn signing_data_zero_values() {
        let sd = SigningData {
            object_root: Hash256::zero(),
            domain: Hash256::zero(),
        };
        let root = sd.tree_hash_root();
        // tree hash of two zero Hash256 fields is not itself zero
        assert_ne!(root, Hash256::zero());
    }
}
