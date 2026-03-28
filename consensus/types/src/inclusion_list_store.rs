use crate::{EthSpec, Hash256, InclusionList, SignedInclusionList, Slot};
use std::collections::{HashMap, HashSet};

/// Key for the inclusion list store: (slot, inclusion_list_committee_root).
pub type InclusionListKey = (Slot, Hash256);

/// Local store for tracking inclusion lists received on the P2P network.
///
/// Tracks valid inclusion lists per (slot, committee_root) and detects equivocators
/// (validators who submit conflicting inclusion lists).
///
/// Spec: <https://github.com/ethereum/consensus-specs/blob/master/specs/heze/inclusion-list.md#inclusionliststore>
#[derive(Debug, Clone, Default)]
pub struct InclusionListStore<E: EthSpec> {
    /// Valid inclusion lists indexed by (slot, committee_root).
    pub inclusion_lists: HashMap<InclusionListKey, HashSet<InclusionList<E>>>,
    /// Validator indices that have equivocated, indexed by (slot, committee_root).
    pub equivocators: HashMap<InclusionListKey, HashSet<u64>>,
    /// Cache of signed inclusion lists for RPC serving, indexed by (slot, committee_root)
    /// then validator_index.
    pub signed_cache: HashMap<InclusionListKey, HashMap<u64, SignedInclusionList<E>>>,
}

impl<E: EthSpec> InclusionListStore<E> {
    pub fn new() -> Self {
        Self {
            inclusion_lists: HashMap::new(),
            equivocators: HashMap::new(),
            signed_cache: HashMap::new(),
        }
    }

    /// Process a received inclusion list, detecting equivocations.
    ///
    /// - If the validator is already an equivocator, ignore.
    /// - If a different inclusion list from the same validator exists, mark as equivocator and remove.
    /// - Only store if received before the view freeze cutoff.
    ///
    /// Spec: process_inclusion_list(store, inclusion_list, is_before_view_freeze_cutoff)
    pub fn process_inclusion_list(
        &mut self,
        inclusion_list: InclusionList<E>,
        is_before_view_freeze_cutoff: bool,
    ) {
        let key = (
            inclusion_list.slot,
            inclusion_list.inclusion_list_committee_root,
        );
        let validator_index = inclusion_list.validator_index;

        // Ignore from equivocators
        if self
            .equivocators
            .get(&key)
            .is_some_and(|eq| eq.contains(&validator_index))
        {
            return;
        }

        // Check for existing inclusion list from this validator
        if let Some(lists) = self.inclusion_lists.get_mut(&key) {
            let existing = lists
                .iter()
                .find(|il| il.validator_index == validator_index)
                .cloned();

            if let Some(stored_il) = existing {
                if stored_il != inclusion_list {
                    // Equivocation detected — different IL from same validator
                    self.equivocators
                        .entry(key)
                        .or_default()
                        .insert(validator_index);
                    lists.remove(&stored_il);
                }
                // Whether equivocation or duplicate, we're done
                return;
            }
        }

        // Only store if before view freeze cutoff
        if is_before_view_freeze_cutoff {
            self.inclusion_lists
                .entry(key)
                .or_default()
                .insert(inclusion_list);
        }
    }

    /// Process a received signed inclusion list, detecting equivocations and caching
    /// the signed version for RPC serving.
    pub fn process_signed_inclusion_list(
        &mut self,
        signed_il: SignedInclusionList<E>,
        is_before_view_freeze_cutoff: bool,
    ) {
        let validator_index = signed_il.message.validator_index;
        let key = (
            signed_il.message.slot,
            signed_il.message.inclusion_list_committee_root,
        );

        // Process the unsigned IL (equivocation detection, storage).
        self.process_inclusion_list(signed_il.message.clone(), is_before_view_freeze_cutoff);

        // Cache the signed version if the unsigned was accepted (i.e., it's in the store
        // and the validator is not an equivocator).
        if self
            .inclusion_lists
            .get(&key)
            .is_some_and(|lists| lists.iter().any(|il| il.validator_index == validator_index))
        {
            self.signed_cache
                .entry(key)
                .or_default()
                .insert(validator_index, signed_il);
        } else {
            // If validator became equivocator, remove from signed cache too.
            if let Some(cache) = self.signed_cache.get_mut(&key) {
                cache.remove(&validator_index);
            }
        }
    }

    /// Get deduplicated transactions from all valid, non-equivocating inclusion lists
    /// for the given slot and committee root.
    ///
    /// Spec: get_inclusion_list_transactions(store, state, slot)
    /// The caller computes committee_root = hash_tree_root(get_inclusion_list_committee(state, slot)).
    pub fn get_inclusion_list_transactions(
        &self,
        slot: Slot,
        committee_root: Hash256,
    ) -> Vec<Vec<u8>> {
        let key = (slot, committee_root);

        let equivocators = self.equivocators.get(&key);
        let mut seen = HashSet::new();
        let mut transactions = Vec::new();

        if let Some(lists) = self.inclusion_lists.get(&key) {
            for il in lists {
                // Skip equivocators
                if equivocators.is_some_and(|eq| eq.contains(&il.validator_index)) {
                    continue;
                }
                for tx in &il.transactions {
                    let tx_bytes: Vec<u8> = tx.to_vec();
                    if seen.insert(tx_bytes.clone()) {
                        transactions.push(tx_bytes);
                    }
                }
            }
        }

        transactions
    }

    /// Get a bitvector (as Vec<bool>) over inclusion list committee indices with bits set
    /// for valid, non-equivocating inclusion list submissions for the given slot.
    ///
    /// `committee` is the ordered list of validator indices in the IL committee.
    /// `committee_root` is hash_tree_root(committee).
    ///
    /// Spec: get_inclusion_list_bits(store, state, slot)
    pub fn get_inclusion_list_bits(
        &self,
        committee: &[u64],
        committee_root: Hash256,
        slot: Slot,
    ) -> Vec<bool> {
        let key = (slot, committee_root);

        let equivocators = self.equivocators.get(&key);

        // Collect validator indices with valid submissions
        let mut valid_indices = HashSet::new();
        if let Some(lists) = self.inclusion_lists.get(&key) {
            for il in lists {
                if !equivocators.is_some_and(|eq| eq.contains(&il.validator_index)) {
                    valid_indices.insert(il.validator_index);
                }
            }
        }

        committee
            .iter()
            .map(|vi| valid_indices.contains(vi))
            .collect()
    }

    /// Check if `inclusion_list_bits` is a superset of the locally observed inclusion list bits.
    ///
    /// Returns true iff every bit set in our local view is also set in the provided bits.
    ///
    /// Spec: is_inclusion_list_bits_inclusive(store, state, slot, inclusion_list_bits)
    pub fn is_inclusion_list_bits_inclusive(
        &self,
        committee: &[u64],
        committee_root: Hash256,
        slot: Slot,
        inclusion_list_bits: &[bool],
    ) -> bool {
        let local_bits = self.get_inclusion_list_bits(committee, committee_root, slot);

        local_bits
            .iter()
            .zip(inclusion_list_bits.iter())
            .all(|(&local_bit, &provided_bit)| provided_bit || !local_bit)
    }

    /// Prune all entries for slots older than the given slot.
    pub fn prune(&mut self, min_slot: Slot) {
        self.inclusion_lists
            .retain(|(slot, _), _| *slot >= min_slot);
        self.equivocators.retain(|(slot, _), _| *slot >= min_slot);
        self.signed_cache.retain(|(slot, _), _| *slot >= min_slot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MinimalEthSpec;

    type E = MinimalEthSpec;

    /// Simple committee root for tests — just hash the raw bytes.
    fn test_committee_root(committee: &[u64]) -> Hash256 {
        use ethereum_hashing::hash;
        let bytes: Vec<u8> = committee.iter().flat_map(|v| v.to_le_bytes()).collect();
        Hash256::from_slice(&hash(&bytes))
    }

    fn make_il(slot: u64, validator_index: u64, committee_root: Hash256) -> InclusionList<E> {
        InclusionList {
            slot: Slot::new(slot),
            validator_index,
            inclusion_list_committee_root: committee_root,
            transactions: <_>::default(),
        }
    }

    fn make_il_with_txs(
        slot: u64,
        validator_index: u64,
        committee_root: Hash256,
        txs: Vec<Vec<u8>>,
    ) -> InclusionList<E> {
        use ssz_types::VariableList;
        let transactions = VariableList::new(
            txs.into_iter()
                .map(|tx| VariableList::new(tx).unwrap())
                .collect(),
        )
        .unwrap();
        InclusionList {
            slot: Slot::new(slot),
            validator_index,
            inclusion_list_committee_root: committee_root,
            transactions,
        }
    }

    #[test]
    fn process_stores_before_cutoff() {
        let mut store = InclusionListStore::<E>::new();
        let committee = vec![0, 1, 2, 3];
        let cr = test_committee_root(&committee);
        let il = make_il(1, 0, cr);

        store.process_inclusion_list(il, true);

        let key = (Slot::new(1), cr);
        assert_eq!(store.inclusion_lists.get(&key).unwrap().len(), 1);
    }

    #[test]
    fn process_ignores_after_cutoff() {
        let mut store = InclusionListStore::<E>::new();
        let committee = vec![0, 1, 2, 3];
        let cr = test_committee_root(&committee);
        let il = make_il(1, 0, cr);

        store.process_inclusion_list(il, false);

        let key = (Slot::new(1), cr);
        assert!(!store.inclusion_lists.contains_key(&key));
    }

    #[test]
    fn process_detects_equivocation() {
        let mut store = InclusionListStore::<E>::new();
        let committee = vec![0, 1, 2, 3];
        let cr = test_committee_root(&committee);
        let il1 = make_il(1, 0, cr);
        let il2 = make_il_with_txs(1, 0, cr, vec![vec![1, 2, 3]]);

        store.process_inclusion_list(il1, true);
        store.process_inclusion_list(il2, true);

        let key = (Slot::new(1), cr);
        // Original removed, equivocator recorded
        assert!(store.inclusion_lists.get(&key).unwrap().is_empty());
        assert!(store.equivocators.get(&key).unwrap().contains(&0));
    }

    #[test]
    fn process_ignores_duplicate() {
        let mut store = InclusionListStore::<E>::new();
        let committee = vec![0, 1, 2, 3];
        let cr = test_committee_root(&committee);
        let il = make_il(1, 0, cr);

        store.process_inclusion_list(il.clone(), true);
        store.process_inclusion_list(il, true);

        let key = (Slot::new(1), cr);
        assert_eq!(store.inclusion_lists.get(&key).unwrap().len(), 1);
        assert!(!store.equivocators.contains_key(&key));
    }

    #[test]
    fn process_ignores_from_equivocator() {
        let mut store = InclusionListStore::<E>::new();
        let committee = vec![0, 1, 2, 3];
        let cr = test_committee_root(&committee);
        let il1 = make_il(1, 0, cr);
        let il2 = make_il_with_txs(1, 0, cr, vec![vec![1, 2, 3]]);
        let il3 = make_il_with_txs(1, 0, cr, vec![vec![4, 5, 6]]);

        store.process_inclusion_list(il1, true);
        store.process_inclusion_list(il2, true);
        // Third attempt from equivocator should be ignored
        store.process_inclusion_list(il3, true);

        let key = (Slot::new(1), cr);
        assert!(store.inclusion_lists.get(&key).unwrap().is_empty());
    }

    #[test]
    fn get_transactions_deduplicates() {
        let mut store = InclusionListStore::<E>::new();
        let committee = vec![0, 1, 2, 3];
        let cr = test_committee_root(&committee);
        let tx = vec![0xaa, 0xbb];
        let il1 = make_il_with_txs(1, 0, cr, vec![tx.clone()]);
        let il2 = make_il_with_txs(1, 1, cr, vec![tx.clone()]);

        store.process_inclusion_list(il1, true);
        store.process_inclusion_list(il2, true);

        let txs = store.get_inclusion_list_transactions(Slot::new(1), cr);
        assert_eq!(txs.len(), 1, "duplicate tx should be deduplicated");
        assert_eq!(txs[0], tx);
    }

    #[test]
    fn get_transactions_excludes_equivocators() {
        let mut store = InclusionListStore::<E>::new();
        let committee = vec![0, 1, 2, 3];
        let cr = test_committee_root(&committee);
        let il1 = make_il_with_txs(1, 0, cr, vec![vec![1]]);
        let il1_eq = make_il_with_txs(1, 0, cr, vec![vec![2]]);
        let il2 = make_il_with_txs(1, 1, cr, vec![vec![3]]);

        store.process_inclusion_list(il1, true);
        store.process_inclusion_list(il1_eq, true);
        store.process_inclusion_list(il2, true);

        let txs = store.get_inclusion_list_transactions(Slot::new(1), cr);
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0], vec![3]);
    }

    #[test]
    fn get_bits_correct() {
        let mut store = InclusionListStore::<E>::new();
        let committee = vec![10, 20, 30, 40];
        let cr = test_committee_root(&committee);
        let il1 = make_il(1, 10, cr);
        let il2 = make_il(1, 30, cr);

        store.process_inclusion_list(il1, true);
        store.process_inclusion_list(il2, true);

        let bits = store.get_inclusion_list_bits(&committee, cr, Slot::new(1));
        assert_eq!(bits, vec![true, false, true, false]);
    }

    #[test]
    fn get_bits_excludes_equivocators() {
        let mut store = InclusionListStore::<E>::new();
        let committee = vec![10, 20, 30, 40];
        let cr = test_committee_root(&committee);
        let il1 = make_il(1, 10, cr);
        let il1_eq = make_il_with_txs(1, 10, cr, vec![vec![1]]);
        let il2 = make_il(1, 30, cr);

        store.process_inclusion_list(il1, true);
        store.process_inclusion_list(il1_eq, true);
        store.process_inclusion_list(il2, true);

        let bits = store.get_inclusion_list_bits(&committee, cr, Slot::new(1));
        assert_eq!(bits, vec![false, false, true, false]);
    }

    #[test]
    fn is_bits_inclusive_superset() {
        let mut store = InclusionListStore::<E>::new();
        let committee = vec![10, 20, 30, 40];
        let cr = test_committee_root(&committee);
        let il = make_il(1, 10, cr);
        store.process_inclusion_list(il, true);

        // Local: [true, false, false, false]
        // Provided superset: [true, true, false, false] — should be inclusive
        assert!(store.is_inclusion_list_bits_inclusive(
            &committee,
            cr,
            Slot::new(1),
            &[true, true, false, false]
        ));
    }

    #[test]
    fn is_bits_inclusive_not_superset() {
        let mut store = InclusionListStore::<E>::new();
        let committee = vec![10, 20, 30, 40];
        let cr = test_committee_root(&committee);
        let il = make_il(1, 10, cr);
        store.process_inclusion_list(il, true);

        // Local: [true, false, false, false]
        // Provided: [false, true, false, false] — NOT inclusive (missing bit 0)
        assert!(!store.is_inclusion_list_bits_inclusive(
            &committee,
            cr,
            Slot::new(1),
            &[false, true, false, false]
        ));
    }

    #[test]
    fn is_bits_inclusive_empty_local() {
        let store = InclusionListStore::<E>::new();
        let committee = vec![10, 20, 30, 40];
        let cr = test_committee_root(&committee);

        // No local ILs — any bits should be inclusive
        assert!(store.is_inclusion_list_bits_inclusive(
            &committee,
            cr,
            Slot::new(1),
            &[false, false, false, false]
        ));
    }

    #[test]
    fn prune_removes_old_slots() {
        let mut store = InclusionListStore::<E>::new();
        let committee = vec![0, 1, 2, 3];
        let cr = test_committee_root(&committee);
        let il1 = make_il(1, 0, cr);
        let il2 = make_il(5, 1, cr);

        store.process_inclusion_list(il1, true);
        store.process_inclusion_list(il2, true);

        store.prune(Slot::new(3));

        // Slot 1 should be pruned, slot 5 should remain
        assert_eq!(store.inclusion_lists.len(), 1);
        let key = (Slot::new(5), cr);
        assert!(store.inclusion_lists.contains_key(&key));
    }
}
