use itertools::Itertools;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap, btree_map::Entry},
};
use store::HotStateSummary;
use types::{Hash256, Slot};

#[derive(Debug, Clone, Copy)]
pub struct DAGStateSummary {
    pub slot: Slot,
    pub latest_block_root: Hash256,
    pub latest_block_slot: Slot,
    pub previous_state_root: Hash256,
}

#[derive(Debug, Clone, Copy)]
pub struct DAGStateSummaryV22 {
    pub slot: Slot,
    pub latest_block_root: Hash256,
    pub block_slot: Slot,
    pub block_parent_root: Hash256,
}

#[derive(Debug)]
pub struct StateSummariesDAG {
    // state_root -> state_summary
    state_summaries_by_state_root: HashMap<Hash256, DAGStateSummary>,
    // block_root -> state slot -> (state_root, state summary)
    state_summaries_by_block_root: HashMap<Hash256, BTreeMap<Slot, (Hash256, DAGStateSummary)>>,
    // parent_state_root -> Vec<children_state_root>
    // cached value to prevent having to recompute in each recursive call into `descendants_of`
    child_state_roots: HashMap<Hash256, Vec<Hash256>>,
}

#[derive(Debug)]
pub enum Error {
    DuplicateStateSummary {
        block_root: Hash256,
        existing_state_summary: Box<(Slot, Hash256)>,
        new_state_summary: (Slot, Hash256),
    },
    MissingStateSummary(Hash256),
    MissingStateSummaryByBlockRoot {
        state_root: Hash256,
        latest_block_root: Hash256,
    },
    MissingChildStateRoot(Hash256),
    RequestedSlotAboveSummary {
        starting_state_root: Hash256,
        ancestor_slot: Slot,
        state_root: Hash256,
        state_slot: Slot,
    },
    RootUnknownPreviousStateRoot(Slot, Hash256),
    RootUnknownAncestorStateRoot {
        starting_state_root: Hash256,
        ancestor_slot: Slot,
        root_state_root: Hash256,
        root_state_slot: Slot,
    },
    CircularAncestorChain {
        state_root: Hash256,
        previous_state_root: Hash256,
        slot: Slot,
        last_slot: Slot,
    },
}

impl StateSummariesDAG {
    pub fn new(state_summaries: Vec<(Hash256, DAGStateSummary)>) -> Result<Self, Error> {
        // Group them by latest block root, and sorted state slot
        let mut state_summaries_by_state_root = HashMap::new();
        let mut state_summaries_by_block_root = HashMap::<_, BTreeMap<_, _>>::new();
        let mut child_state_roots = HashMap::<_, Vec<_>>::new();

        for (state_root, summary) in state_summaries.into_iter() {
            let summaries = state_summaries_by_block_root
                .entry(summary.latest_block_root)
                .or_default();

            // Sanity check to ensure no duplicate summaries for the tuple (block_root, state_slot)
            match summaries.entry(summary.slot) {
                Entry::Vacant(entry) => {
                    entry.insert((state_root, summary));
                }
                Entry::Occupied(existing) => {
                    return Err(Error::DuplicateStateSummary {
                        block_root: summary.latest_block_root,
                        existing_state_summary: (summary.slot, state_root).into(),
                        new_state_summary: (*existing.key(), existing.get().0),
                    });
                }
            }

            state_summaries_by_state_root.insert(state_root, summary);

            child_state_roots
                .entry(summary.previous_state_root)
                .or_default()
                .push(state_root);
            // Add empty entry for the child state
            child_state_roots.entry(state_root).or_default();
        }

        Ok(Self {
            state_summaries_by_state_root,
            state_summaries_by_block_root,
            child_state_roots,
        })
    }

    /// Computes a DAG from a sequence of state summaries, including their parent block
    /// relationships.
    ///
    /// - Expects summaries to be contiguous per slot: there must exist a summary at every slot
    ///   of each tree branch
    /// - Maybe include multiple disjoint trees. The root of each tree will have a ZERO parent state
    ///   root, which will error later when calling `previous_state_root`.
    pub fn new_from_v22(
        state_summaries_v22: Vec<(Hash256, DAGStateSummaryV22)>,
    ) -> Result<Self, Error> {
        // Group them by latest block root, and sorted state slot
        let mut state_summaries_by_block_root = HashMap::<_, BTreeMap<_, _>>::new();
        for (state_root, summary) in state_summaries_v22.iter() {
            let summaries = state_summaries_by_block_root
                .entry(summary.latest_block_root)
                .or_default();

            // Sanity check to ensure no duplicate summaries for the tuple (block_root, state_slot)
            match summaries.entry(summary.slot) {
                Entry::Vacant(entry) => {
                    entry.insert((state_root, summary));
                }
                Entry::Occupied(existing) => {
                    return Err(Error::DuplicateStateSummary {
                        block_root: summary.latest_block_root,
                        existing_state_summary: (summary.slot, *state_root).into(),
                        new_state_summary: (*existing.key(), *existing.get().0),
                    });
                }
            }
        }

        let state_summaries = state_summaries_v22
            .iter()
            .map(|(state_root, summary)| {
                let previous_state_root = if summary.slot == 0 {
                    Hash256::ZERO
                } else {
                    let previous_slot = summary.slot - 1;

                    // Check the set of states in the same state's block root
                    let same_block_root_summaries = state_summaries_by_block_root
                        .get(&summary.latest_block_root)
                        // Should never error: we construct the HashMap here and must have at least
                        // one entry per block root
                        .ok_or(Error::MissingStateSummaryByBlockRoot {
                            state_root: *state_root,
                            latest_block_root: summary.latest_block_root,
                        })?;
                    if let Some((state_root, _)) = same_block_root_summaries.get(&previous_slot) {
                        // Skipped slot: block root at previous slot is the same as latest block root.
                        **state_root
                    } else {
                        // Common case: not a skipped slot.
                        //
                        // If we can't find a state summmary for the parent block and previous slot,
                        // then there is some amount of disjointedness in the DAG. We set the parent
                        // state root to 0x0 in this case, and will prune any dangling states.
                        let parent_block_root = summary.block_parent_root;
                        state_summaries_by_block_root
                            .get(&parent_block_root)
                            .and_then(|parent_block_summaries| {
                                parent_block_summaries.get(&previous_slot)
                            })
                            .map_or(Hash256::ZERO, |(parent_state_root, _)| **parent_state_root)
                    }
                };

                Ok((
                    *state_root,
                    DAGStateSummary {
                        slot: summary.slot,
                        latest_block_root: summary.latest_block_root,
                        latest_block_slot: summary.block_slot,
                        previous_state_root,
                    },
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Self::new(state_summaries)
    }

    // Returns all non-unique latest block roots of a given set of states
    pub fn blocks_of_states<'a, I: Iterator<Item = &'a Hash256>>(
        &self,
        state_roots: I,
    ) -> Result<Vec<(Hash256, Slot)>, Error> {
        state_roots
            .map(|state_root| {
                let summary = self
                    .state_summaries_by_state_root
                    .get(state_root)
                    .ok_or(Error::MissingStateSummary(*state_root))?;
                Ok((summary.latest_block_root, summary.latest_block_slot))
            })
            .collect()
    }

    // Returns all unique latest blocks of this DAG's summaries
    pub fn iter_blocks(&self) -> impl Iterator<Item = (Hash256, Slot)> + '_ {
        self.state_summaries_by_state_root
            .values()
            .map(|summary| (summary.latest_block_root, summary.latest_block_slot))
            .unique()
    }

    /// Returns a vec of state summaries that have an unknown parent when forming the DAG tree
    pub fn tree_roots(&self) -> Vec<(Hash256, DAGStateSummary)> {
        self.state_summaries_by_state_root
            .iter()
            .filter_map(|(state_root, summary)| {
                if self
                    .state_summaries_by_state_root
                    .contains_key(&summary.previous_state_root)
                {
                    // Summaries with a known parent are not roots
                    None
                } else {
                    Some((*state_root, *summary))
                }
            })
            .collect()
    }

    pub fn summaries_count(&self) -> usize {
        self.state_summaries_by_block_root
            .values()
            .map(|s| s.len())
            .sum()
    }

    pub fn summaries_by_slot_ascending(&self) -> BTreeMap<Slot, Vec<(Hash256, DAGStateSummary)>> {
        let mut summaries = BTreeMap::<Slot, Vec<_>>::new();
        for (state_root, summary) in self.state_summaries_by_state_root.iter() {
            summaries
                .entry(summary.slot)
                .or_default()
                .push((*state_root, *summary));
        }
        summaries
    }

    pub fn previous_state_root(&self, state_root: Hash256) -> Result<Hash256, Error> {
        let summary = self
            .state_summaries_by_state_root
            .get(&state_root)
            .ok_or(Error::MissingStateSummary(state_root))?;
        if summary.previous_state_root == Hash256::ZERO {
            Err(Error::RootUnknownPreviousStateRoot(
                summary.slot,
                state_root,
            ))
        } else {
            Ok(summary.previous_state_root)
        }
    }

    pub fn ancestor_state_root_at_slot(
        &self,
        starting_state_root: Hash256,
        ancestor_slot: Slot,
    ) -> Result<Hash256, Error> {
        let mut state_root = starting_state_root;
        // Walk backwards until we reach the state at `ancestor_slot`.
        loop {
            let summary = self
                .state_summaries_by_state_root
                .get(&state_root)
                .ok_or(Error::MissingStateSummary(state_root))?;

            // Assumes all summaries are contiguous
            match summary.slot.cmp(&ancestor_slot) {
                Ordering::Less => {
                    return Err(Error::RequestedSlotAboveSummary {
                        starting_state_root,
                        ancestor_slot,
                        state_root,
                        state_slot: summary.slot,
                    });
                }
                Ordering::Equal => {
                    return Ok(state_root);
                }
                Ordering::Greater => {
                    if summary.previous_state_root == Hash256::ZERO {
                        return Err(Error::RootUnknownAncestorStateRoot {
                            starting_state_root,
                            ancestor_slot,
                            root_state_root: state_root,
                            root_state_slot: summary.slot,
                        });
                    } else {
                        state_root = summary.previous_state_root;
                    }
                }
            }
        }
    }

    /// Returns all ancestors of `state_root` INCLUDING `state_root` until the next parent is not
    /// known.
    pub fn ancestors_of(&self, mut state_root: Hash256) -> Result<Vec<(Hash256, Slot)>, Error> {
        // Sanity check that the first summary exists
        if !self.state_summaries_by_state_root.contains_key(&state_root) {
            return Err(Error::MissingStateSummary(state_root));
        }

        let mut ancestors = vec![];
        let mut last_slot = None;
        loop {
            if let Some(summary) = self.state_summaries_by_state_root.get(&state_root) {
                // Detect cycles, including the case where `previous_state_root == state_root`.
                if let Some(last_slot) = last_slot
                    && summary.slot >= last_slot
                {
                    return Err(Error::CircularAncestorChain {
                        state_root,
                        previous_state_root: summary.previous_state_root,
                        slot: summary.slot,
                        last_slot,
                    });
                }

                ancestors.push((state_root, summary.slot));
                last_slot = Some(summary.slot);
                state_root = summary.previous_state_root;
            } else {
                return Ok(ancestors);
            }
        }
    }

    /// Returns of the descendant state summaries roots given an initiail state root.
    pub fn descendants_of(&self, query_state_root: &Hash256) -> Result<Vec<Hash256>, Error> {
        let mut descendants = vec![];
        for child_root in self
            .child_state_roots
            .get(query_state_root)
            .ok_or(Error::MissingChildStateRoot(*query_state_root))?
        {
            descendants.push(*child_root);
            descendants.extend(self.descendants_of(child_root)?);
        }
        Ok(descendants)
    }

    /// Returns the root of the state at `slot` with `latest_block_root`, if it exists.
    ///
    /// The `slot` must be the slot of the `latest_block_root` or a skipped slot following it. This
    /// function will not return the `state_root` of a state with a different `latest_block_root`
    /// even if it lies on the same chain.
    pub fn state_root_at_slot(&self, latest_block_root: Hash256, slot: Slot) -> Option<Hash256> {
        self.state_summaries_by_block_root
            .get(&latest_block_root)?
            .get(&slot)
            .map(|(state_root, _)| *state_root)
    }
}

impl From<HotStateSummary> for DAGStateSummary {
    fn from(value: HotStateSummary) -> Self {
        Self {
            slot: value.slot,
            latest_block_root: value.latest_block_root,
            latest_block_slot: value.latest_block_slot,
            previous_state_root: value.previous_state_root,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DAGStateSummaryV22, Error, StateSummariesDAG};
    use bls::FixedBytesExtended;
    use types::{Hash256, Slot};

    fn root(n: u64) -> Hash256 {
        Hash256::from_low_u64_le(n)
    }

    #[test]
    fn new_from_v22_empty() {
        StateSummariesDAG::new_from_v22(vec![]).unwrap();
    }

    fn assert_previous_state_root_is_zero(dag: &StateSummariesDAG, root: Hash256) {
        assert!(matches!(
            dag.previous_state_root(root).unwrap_err(),
            Error::RootUnknownPreviousStateRoot { .. }
        ));
    }

    #[test]
    fn new_from_v22_one_state() {
        let root_a = root(0xa);
        let root_1 = root(1);
        let root_2 = root(2);
        let summary_1 = DAGStateSummaryV22 {
            slot: Slot::new(1),
            latest_block_root: root_1,
            block_parent_root: root_2,
            block_slot: Slot::new(1),
        };

        let dag = StateSummariesDAG::new_from_v22(vec![(root_a, summary_1)]).unwrap();

        // The parent of the root summary is ZERO
        assert_previous_state_root_is_zero(&dag, root_a);
    }

    #[test]
    fn new_from_v22_multiple_states() {
        let dag = StateSummariesDAG::new_from_v22(vec![
            (
                root(0xa),
                DAGStateSummaryV22 {
                    slot: Slot::new(3),
                    latest_block_root: root(3),
                    block_parent_root: root(1),
                    block_slot: Slot::new(3),
                },
            ),
            (
                root(0xb),
                DAGStateSummaryV22 {
                    slot: Slot::new(4),
                    latest_block_root: root(4),
                    block_parent_root: root(3),
                    block_slot: Slot::new(4),
                },
            ),
            // fork 1
            (
                root(0xc),
                DAGStateSummaryV22 {
                    slot: Slot::new(5),
                    latest_block_root: root(5),
                    block_parent_root: root(4),
                    block_slot: Slot::new(5),
                },
            ),
            // fork 2
            // skipped slot
            (
                root(0xd),
                DAGStateSummaryV22 {
                    slot: Slot::new(5),
                    latest_block_root: root(4),
                    block_parent_root: root(3),
                    block_slot: Slot::new(4),
                },
            ),
            // normal slot
            (
                root(0xe),
                DAGStateSummaryV22 {
                    slot: Slot::new(6),
                    latest_block_root: root(6),
                    block_parent_root: root(4),
                    block_slot: Slot::new(6),
                },
            ),
        ])
        .unwrap();

        // The parent of the root summary is ZERO
        assert_previous_state_root_is_zero(&dag, root(0xa));
        assert_eq!(dag.previous_state_root(root(0xc)).unwrap(), root(0xb));
        assert_eq!(dag.previous_state_root(root(0xd)).unwrap(), root(0xb));
        assert_eq!(dag.previous_state_root(root(0xe)).unwrap(), root(0xd));
    }

    use super::DAGStateSummary;

    fn summary(slot: u64, block_root: u64, prev: u64) -> DAGStateSummary {
        DAGStateSummary {
            slot: Slot::new(slot),
            latest_block_root: root(block_root),
            latest_block_slot: Slot::new(slot),
            previous_state_root: root(prev),
        }
    }

    /// Build a simple linear chain: slots 1→2→3, each with a unique block root.
    fn linear_dag() -> StateSummariesDAG {
        StateSummariesDAG::new(vec![
            (root(0xa), summary(1, 1, 0)), // root (prev=0x0 is unknown)
            (root(0xb), summary(2, 2, 0xa)),
            (root(0xc), summary(3, 3, 0xb)),
        ])
        .unwrap()
    }

    #[test]
    fn new_empty() {
        let dag = StateSummariesDAG::new(vec![]).unwrap();
        assert_eq!(dag.summaries_count(), 0);
    }

    #[test]
    fn new_duplicate_block_root_slot_errors() {
        let result = StateSummariesDAG::new(vec![
            (root(0xa), summary(1, 1, 0)),
            (root(0xb), summary(1, 1, 0)), // same (block_root=1, slot=1)
        ]);
        assert!(matches!(
            result.unwrap_err(),
            Error::DuplicateStateSummary { .. }
        ));
    }

    #[test]
    fn summaries_count() {
        let dag = linear_dag();
        assert_eq!(dag.summaries_count(), 3);
    }

    #[test]
    fn previous_state_root_chain() {
        let dag = linear_dag();
        assert_previous_state_root_is_zero(&dag, root(0xa));
        assert_eq!(dag.previous_state_root(root(0xb)).unwrap(), root(0xa));
        assert_eq!(dag.previous_state_root(root(0xc)).unwrap(), root(0xb));
    }

    #[test]
    fn previous_state_root_missing() {
        let dag = linear_dag();
        assert!(matches!(
            dag.previous_state_root(root(0xff)).unwrap_err(),
            Error::MissingStateSummary(_)
        ));
    }

    #[test]
    fn ancestor_state_root_at_slot_same() {
        let dag = linear_dag();
        assert_eq!(
            dag.ancestor_state_root_at_slot(root(0xc), Slot::new(3))
                .unwrap(),
            root(0xc)
        );
    }

    #[test]
    fn ancestor_state_root_at_slot_walk_back() {
        let dag = linear_dag();
        assert_eq!(
            dag.ancestor_state_root_at_slot(root(0xc), Slot::new(2))
                .unwrap(),
            root(0xb)
        );
        assert_eq!(
            dag.ancestor_state_root_at_slot(root(0xc), Slot::new(1))
                .unwrap(),
            root(0xa)
        );
    }

    #[test]
    fn ancestor_state_root_at_slot_above_errors() {
        let dag = linear_dag();
        assert!(matches!(
            dag.ancestor_state_root_at_slot(root(0xa), Slot::new(5))
                .unwrap_err(),
            Error::RequestedSlotAboveSummary { .. }
        ));
    }

    #[test]
    fn ancestor_state_root_at_root_unknown_parent() {
        let dag = linear_dag();
        // root(0xa) has prev=0x0 which is unknown, so walking past it errors
        assert!(matches!(
            dag.ancestor_state_root_at_slot(root(0xa), Slot::new(0))
                .unwrap_err(),
            Error::RootUnknownAncestorStateRoot { .. }
        ));
    }

    #[test]
    fn ancestors_of_leaf() {
        let dag = linear_dag();
        let ancestors = dag.ancestors_of(root(0xc)).unwrap();
        assert_eq!(ancestors.len(), 3);
        assert_eq!(ancestors[0], (root(0xc), Slot::new(3)));
        assert_eq!(ancestors[1], (root(0xb), Slot::new(2)));
        assert_eq!(ancestors[2], (root(0xa), Slot::new(1)));
    }

    #[test]
    fn ancestors_of_root() {
        let dag = linear_dag();
        let ancestors = dag.ancestors_of(root(0xa)).unwrap();
        assert_eq!(ancestors.len(), 1);
        assert_eq!(ancestors[0], (root(0xa), Slot::new(1)));
    }

    #[test]
    fn ancestors_of_missing_errors() {
        let dag = linear_dag();
        assert!(matches!(
            dag.ancestors_of(root(0xff)).unwrap_err(),
            Error::MissingStateSummary(_)
        ));
    }

    #[test]
    fn descendants_of_root() {
        let dag = linear_dag();
        let mut desc = dag.descendants_of(&root(0xa)).unwrap();
        desc.sort();
        let mut expected = vec![root(0xb), root(0xc)];
        expected.sort();
        assert_eq!(desc, expected);
    }

    #[test]
    fn descendants_of_leaf_empty() {
        let dag = linear_dag();
        let desc = dag.descendants_of(&root(0xc)).unwrap();
        assert!(desc.is_empty());
    }

    #[test]
    fn descendants_of_missing_errors() {
        let dag = linear_dag();
        assert!(matches!(
            dag.descendants_of(&root(0xff)).unwrap_err(),
            Error::MissingChildStateRoot(_)
        ));
    }

    #[test]
    fn tree_roots_identifies_roots() {
        let dag = linear_dag();
        let roots = dag.tree_roots();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].0, root(0xa));
    }

    #[test]
    fn state_root_at_slot_found() {
        let dag = linear_dag();
        assert_eq!(
            dag.state_root_at_slot(root(2), Slot::new(2)),
            Some(root(0xb))
        );
    }

    #[test]
    fn state_root_at_slot_wrong_block_root() {
        let dag = linear_dag();
        assert_eq!(dag.state_root_at_slot(root(99), Slot::new(2)), None);
    }

    #[test]
    fn state_root_at_slot_wrong_slot() {
        let dag = linear_dag();
        assert_eq!(dag.state_root_at_slot(root(2), Slot::new(5)), None);
    }

    #[test]
    fn blocks_of_states() {
        let dag = linear_dag();
        let state_roots = [root(0xa), root(0xb)];
        let blocks = dag.blocks_of_states(state_roots.iter()).unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], (root(1), Slot::new(1)));
        assert_eq!(blocks[1], (root(2), Slot::new(2)));
    }

    #[test]
    fn blocks_of_states_missing_errors() {
        let dag = linear_dag();
        let state_roots = [root(0xff)];
        assert!(matches!(
            dag.blocks_of_states(state_roots.iter()).unwrap_err(),
            Error::MissingStateSummary(_)
        ));
    }

    #[test]
    fn summaries_by_slot_ascending_order() {
        let dag = linear_dag();
        let by_slot = dag.summaries_by_slot_ascending();
        let slots: Vec<Slot> = by_slot.keys().cloned().collect();
        assert_eq!(slots, vec![Slot::new(1), Slot::new(2), Slot::new(3)]);
    }

    #[test]
    fn forked_dag_descendants() {
        // Build a fork: 0xa (slot 1) -> 0xb (slot 2) -> 0xc (slot 3, fork 1)
        //                                             -> 0xd (slot 3, fork 2)
        let dag = StateSummariesDAG::new(vec![
            (root(0xa), summary(1, 1, 0)),
            (root(0xb), summary(2, 2, 0xa)),
            (
                root(0xc),
                DAGStateSummary {
                    slot: Slot::new(3),
                    latest_block_root: root(3),
                    latest_block_slot: Slot::new(3),
                    previous_state_root: root(0xb),
                },
            ),
            (
                root(0xd),
                DAGStateSummary {
                    slot: Slot::new(3),
                    latest_block_root: root(4),
                    latest_block_slot: Slot::new(3),
                    previous_state_root: root(0xb),
                },
            ),
        ])
        .unwrap();

        let mut desc = dag.descendants_of(&root(0xb)).unwrap();
        desc.sort();
        let mut expected = vec![root(0xc), root(0xd)];
        expected.sort();
        assert_eq!(desc, expected);

        // Both forks are leaves
        assert!(dag.descendants_of(&root(0xc)).unwrap().is_empty());
        assert!(dag.descendants_of(&root(0xd)).unwrap().is_empty());
    }

    #[test]
    fn iter_blocks_unique() {
        let dag = linear_dag();
        let mut blocks: Vec<_> = dag.iter_blocks().collect();
        blocks.sort();
        // Each state has a unique block root in our linear dag
        assert_eq!(blocks.len(), 3);
    }
}
