use ethereum_hashing::hash_fixed;
use int_to_bytes::int_to_bytes32;
use merkle_proof::{MerkleTree, MerkleTreeError};
use safe_arith::SafeArith;
use types::{DepositTreeSnapshot, FinalizedExecutionBlock, Hash256};

/// Emulates the eth1 deposit contract merkle tree.
#[derive(PartialEq)]
pub struct DepositDataTree {
    tree: MerkleTree,
    mix_in_length: usize,
    finalized_execution_block: Option<FinalizedExecutionBlock>,
    depth: usize,
}

impl DepositDataTree {
    /// Create a new Merkle tree from a list of leaves (`DepositData::tree_hash_root`) and a fixed depth.
    pub fn create(leaves: &[Hash256], mix_in_length: usize, depth: usize) -> Self {
        Self {
            tree: MerkleTree::create(leaves, depth),
            mix_in_length,
            finalized_execution_block: None,
            depth,
        }
    }

    /// Returns 32 bytes representing the "mix in length" for the merkle root of this tree.
    fn length_bytes(&self) -> [u8; 32] {
        int_to_bytes32(self.mix_in_length as u64)
    }

    /// Retrieve the root hash of this Merkle tree with the length mixed in.
    pub fn root(&self) -> Hash256 {
        let mut preimage = [0; 64];
        preimage[0..32].copy_from_slice(&self.tree.hash()[..]);
        preimage[32..64].copy_from_slice(&self.length_bytes());
        Hash256::from(hash_fixed(&preimage))
    }

    /// Return the leaf at `index` and a Merkle proof of its inclusion.
    ///
    /// The Merkle proof is in "bottom-up" order, starting with a leaf node
    /// and moving up the tree. Its length will be exactly equal to `depth + 1`.
    pub fn generate_proof(&self, index: usize) -> Result<(Hash256, Vec<Hash256>), MerkleTreeError> {
        let (root, mut proof) = self.tree.generate_proof(index, self.depth)?;
        proof.push(Hash256::from(self.length_bytes()));
        Ok((root, proof))
    }

    /// Add a deposit to the merkle tree.
    pub fn push_leaf(&mut self, leaf: Hash256) -> Result<(), MerkleTreeError> {
        self.tree.push_leaf(leaf, self.depth)?;
        self.mix_in_length.safe_add_assign(1)?;
        Ok(())
    }

    /// Finalize deposits up to `finalized_execution_block.deposit_count`
    pub fn finalize(
        &mut self,
        finalized_execution_block: FinalizedExecutionBlock,
    ) -> Result<(), MerkleTreeError> {
        self.tree
            .finalize_deposits(finalized_execution_block.deposit_count as usize, self.depth)?;
        self.finalized_execution_block = Some(finalized_execution_block);
        Ok(())
    }

    /// Get snapshot of finalized deposit tree (if tree is finalized)
    pub fn get_snapshot(&self) -> Option<DepositTreeSnapshot> {
        let finalized_execution_block = self.finalized_execution_block.as_ref()?;
        Some(DepositTreeSnapshot {
            finalized: self.tree.get_finalized_hashes(),
            deposit_root: finalized_execution_block.deposit_root,
            deposit_count: finalized_execution_block.deposit_count,
            execution_block_hash: finalized_execution_block.block_hash,
            execution_block_height: finalized_execution_block.block_height,
        })
    }

    /// Create a new Merkle tree from a snapshot
    pub fn from_snapshot(
        snapshot: &DepositTreeSnapshot,
        depth: usize,
    ) -> Result<Self, MerkleTreeError> {
        Ok(Self {
            tree: MerkleTree::from_finalized_snapshot(
                &snapshot.finalized,
                snapshot.deposit_count as usize,
                depth,
            )?,
            mix_in_length: snapshot.deposit_count as usize,
            finalized_execution_block: Some(snapshot.into()),
            depth,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEPTH: usize = 32;

    fn leaf(byte: u8) -> Hash256 {
        Hash256::repeat_byte(byte)
    }

    #[test]
    fn empty_tree_root_deterministic() {
        let tree = DepositDataTree::create(&[], 0, DEPTH);
        let root = tree.root();
        // Root should be deterministic
        let tree2 = DepositDataTree::create(&[], 0, DEPTH);
        assert_eq!(root, tree2.root());
    }

    #[test]
    fn single_leaf_changes_root() {
        let empty = DepositDataTree::create(&[], 0, DEPTH);
        let one_leaf = DepositDataTree::create(&[leaf(1)], 1, DEPTH);
        assert_ne!(empty.root(), one_leaf.root());
    }

    #[test]
    fn different_leaves_different_roots() {
        let tree_a = DepositDataTree::create(&[leaf(1)], 1, DEPTH);
        let tree_b = DepositDataTree::create(&[leaf(2)], 1, DEPTH);
        assert_ne!(tree_a.root(), tree_b.root());
    }

    #[test]
    fn mix_in_length_affects_root() {
        // Same leaves but different mix_in_length → different root
        let leaves = vec![leaf(1), leaf(2)];
        let tree1 = DepositDataTree::create(&leaves, 1, DEPTH);
        let tree2 = DepositDataTree::create(&leaves, 2, DEPTH);
        assert_ne!(tree1.root(), tree2.root());
    }

    #[test]
    fn push_leaf_increments_length() {
        let mut tree = DepositDataTree::create(&[], 0, DEPTH);
        let root_before = tree.root();
        tree.push_leaf(leaf(1)).unwrap();
        let root_after = tree.root();
        assert_ne!(root_before, root_after);
    }

    #[test]
    fn push_matches_create() {
        // Creating with leaves should match pushing them one by one
        let leaves = vec![leaf(1), leaf(2), leaf(3)];

        let tree_batch = DepositDataTree::create(&leaves, 3, DEPTH);

        let mut tree_incr = DepositDataTree::create(&[], 0, DEPTH);
        for l in &leaves {
            tree_incr.push_leaf(*l).unwrap();
        }

        assert_eq!(tree_batch.root(), tree_incr.root());
    }

    #[test]
    fn generate_proof_valid() {
        let leaves = vec![leaf(1), leaf(2), leaf(3), leaf(4)];
        let tree = DepositDataTree::create(&leaves, 4, DEPTH);

        for (i, expected_leaf) in leaves.iter().enumerate() {
            let (proof_leaf, proof) = tree.generate_proof(i).unwrap();
            assert_eq!(proof_leaf, *expected_leaf);
            // Proof length should be depth + 1 (for the mix-in length)
            assert_eq!(proof.len(), DEPTH + 1);
        }
    }

    #[test]
    fn generate_proof_root_verifiable() {
        let leaves = vec![leaf(0xAA), leaf(0xBB)];
        let tree = DepositDataTree::create(&leaves, 2, DEPTH);

        let (proof_leaf, proof) = tree.generate_proof(0).unwrap();
        // The proof should verify against the tree root
        assert!(merkle_proof::verify_merkle_proof(
            proof_leaf,
            &proof,
            DEPTH + 1,
            0,
            tree.root(),
        ));
    }

    #[test]
    fn snapshot_round_trip() {
        let leaves = vec![leaf(1), leaf(2), leaf(3)];
        let mut tree = DepositDataTree::create(&leaves, 3, DEPTH);

        let finalized = FinalizedExecutionBlock {
            deposit_root: tree.root(),
            deposit_count: 3,
            block_hash: Hash256::repeat_byte(0xFF),
            block_height: 100,
        };
        tree.finalize(finalized).unwrap();

        let snapshot = tree.get_snapshot().unwrap();
        assert_eq!(snapshot.deposit_count, 3);
        assert_eq!(snapshot.execution_block_hash, Hash256::repeat_byte(0xFF));
        assert_eq!(snapshot.execution_block_height, 100);

        // Reconstruct from snapshot
        let restored = DepositDataTree::from_snapshot(&snapshot, DEPTH).unwrap();
        // Push a new leaf to both and compare
        let mut tree_clone = tree;
        tree_clone.push_leaf(leaf(4)).unwrap();
        let mut restored_clone = restored;
        restored_clone.push_leaf(leaf(4)).unwrap();
        assert_eq!(tree_clone.root(), restored_clone.root());
    }

    #[test]
    fn no_snapshot_before_finalize() {
        let tree = DepositDataTree::create(&[leaf(1)], 1, DEPTH);
        assert!(tree.get_snapshot().is_none());
    }
}
