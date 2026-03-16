use crate::metrics;
use itertools::Itertools;

/// Trait for types that we can compute a maximum cover for.
///
/// Terminology:
/// * `item`: something that implements this trait
/// * `element`: something contained in a set, and covered by the covering set of an item
/// * `object`: something extracted from an item in order to comprise a solution
///   See: <https://en.wikipedia.org/wiki/Maximum_coverage_problem>
pub trait MaxCover {
    /// The result type, of which we would eventually like a collection of maximal quality.
    type Object;
    /// The intermediate object type, which can be converted to `Object`.
    type Intermediate;
    /// The type used to represent sets.
    type Set;

    /// Extract the intermediate object.
    fn intermediate(&self) -> &Self::Intermediate;

    /// Convert the borrowed intermediate object to an owned object for the solution.
    fn convert_to_object(intermediate: &Self::Intermediate) -> Self::Object;

    /// Get the set of elements covered.
    fn covering_set(&self) -> &Self::Set;
    /// Update the set of items covered, for the inclusion of some object in the solution.
    fn update_covering_set(&mut self, max_obj: &Self::Intermediate, max_set: &Self::Set);
    /// The quality of this item's covering set, usually its cardinality.
    fn score(&self) -> usize;
}

/// Compute an approximate maximum cover using a greedy algorithm.
///
/// * Time complexity: `O(limit * items_iter.len())`
/// * Space complexity: `O(item_iter.len())`
pub fn maximum_cover<I, T>(items_iter: I, limit: usize, label: &str) -> Vec<T>
where
    I: IntoIterator<Item = T>,
    T: MaxCover,
{
    // Construct an initial vec of all items wrapped in Option. Items are taken (moved)
    // when selected, avoiding expensive clones of covering set data (e.g. HashMaps).
    let mut all_items: Vec<Option<T>> = items_iter
        .into_iter()
        .filter(|x| x.score() != 0)
        .map(Some)
        .collect();

    metrics::set_int_gauge(
        &metrics::MAX_COVER_NON_ZERO_ITEMS,
        &[label],
        all_items.len() as i64,
    );

    let mut result = vec![];

    for _ in 0..limit {
        // Select the item with the maximum score, computing score() once per item.
        let best_idx = all_items
            .iter()
            .enumerate()
            .filter_map(|(i, x)| {
                let item = x.as_ref()?;
                let score = item.score();
                (score != 0).then_some((i, score))
            })
            .max_by_key(|&(_, score)| score)
            .map(|(i, _)| i);

        let Some(best_idx) = best_idx else {
            return result;
        };

        // Use split_at_mut to borrow the best item immutably while mutating others,
        // avoiding the need to clone the item (which includes its covering set).
        let (before, rest) = all_items.split_at_mut(best_idx);
        let (best_slot, after) = rest.split_first_mut().unwrap();
        let best_item = best_slot.as_ref().unwrap();
        let best_intermediate = best_item.intermediate();
        let best_set = best_item.covering_set();

        for slot in before.iter_mut().chain(after.iter_mut()) {
            if let Some(item) = slot.as_mut() {
                item.update_covering_set(best_intermediate, best_set);
            }
        }

        // Move the best item into the result (no clone needed).
        result.push(best_slot.take().unwrap());
    }

    result
}

/// Perform a greedy merge of two max cover solutions, preferring higher-score values.
pub fn merge_solutions<I1, I2, T>(cover1: I1, cover2: I2, limit: usize) -> Vec<T::Object>
where
    I1: IntoIterator<Item = T>,
    I2: IntoIterator<Item = T>,
    T: MaxCover,
{
    cover1
        .into_iter()
        .merge_by(cover2, |item1, item2| item1.score() >= item2.score())
        .take(limit)
        .map(|item| T::convert_to_object(item.intermediate()))
        .collect()
}

#[cfg(test)]
mod test {
    use super::*;
    use std::{collections::HashSet, hash::Hash};

    impl<T> MaxCover for HashSet<T>
    where
        T: Clone + Eq + Hash,
    {
        type Object = Self;
        type Intermediate = Self;
        type Set = Self;

        fn intermediate(&self) -> &Self {
            self
        }

        fn convert_to_object(set: &Self) -> Self {
            set.clone()
        }

        fn covering_set(&self) -> &Self {
            self
        }

        fn update_covering_set(&mut self, _: &Self, other: &Self) {
            let mut difference = &*self - other;
            std::mem::swap(self, &mut difference);
        }

        fn score(&self) -> usize {
            self.len()
        }
    }

    fn example_system() -> Vec<HashSet<usize>> {
        vec![
            HashSet::from_iter(vec![3]),
            HashSet::from_iter(vec![1, 2, 4, 5]),
            HashSet::from_iter(vec![1, 2, 4, 5]),
            HashSet::from_iter(vec![1]),
            HashSet::from_iter(vec![2, 4, 5]),
        ]
    }

    #[test]
    fn zero_limit() {
        let cover = maximum_cover(example_system(), 0, "test");
        assert_eq!(cover.len(), 0);
    }

    #[test]
    fn one_limit() {
        let sets = example_system();
        let cover = maximum_cover(sets.clone(), 1, "test");
        assert_eq!(cover.len(), 1);
        assert_eq!(cover[0], sets[1]);
    }

    // Check that even if the limit provides room, we don't include useless items in the soln.
    #[test]
    fn exclude_zero_score() {
        let sets = example_system();
        for k in 2..10 {
            let cover = maximum_cover(sets.clone(), k, "test");
            assert_eq!(cover.len(), 2);
            assert_eq!(cover[0], sets[1]);
            assert_eq!(cover[1], sets[0]);
        }
    }

    fn quality<T: Eq + Hash>(solution: &[HashSet<T>]) -> usize {
        solution.iter().map(HashSet::len).sum()
    }

    // Optimal solution is the first three sets (quality 15) but our greedy algorithm
    // will select the last three (quality 11). The comment at the end of each line
    // shows that set's score at each iteration, with a * indicating that it will be chosen.
    #[test]
    fn suboptimal() {
        let sets = vec![
            HashSet::from_iter(vec![0, 1, 8, 11, 14]), // 5, 3, 2
            HashSet::from_iter(vec![2, 3, 7, 9, 10]),  // 5, 3, 2
            HashSet::from_iter(vec![4, 5, 6, 12, 13]), // 5, 4, 2
            HashSet::from_iter(vec![9, 10]),           // 4, 4, 2*
            HashSet::from_iter(vec![5, 6, 7, 8]),      // 4, 4*
            HashSet::from_iter(vec![0, 1, 2, 3, 4]),   // 5*
        ];
        let cover = maximum_cover(sets, 3, "test");
        assert_eq!(quality(&cover), 11);
    }

    #[test]
    fn intersecting_ok() {
        let sets = vec![
            HashSet::from_iter(vec![1, 2, 3, 4, 5, 6, 7, 8]),
            HashSet::from_iter(vec![1, 2, 3, 9, 10, 11]),
            HashSet::from_iter(vec![4, 5, 6, 12, 13, 14]),
            HashSet::from_iter(vec![7, 8, 15, 16, 17, 18]),
            HashSet::from_iter(vec![1, 2, 9, 10]),
            HashSet::from_iter(vec![1, 5, 6, 8]),
            HashSet::from_iter(vec![1, 7, 11, 19]),
        ];
        let cover = maximum_cover(sets, 5, "test");
        assert_eq!(quality(&cover), 19);
        assert_eq!(cover.len(), 5);
    }

    // ── merge_solutions tests ──────────────────────────────────

    #[test]
    fn merge_empty_solutions() {
        let result: Vec<HashSet<usize>> = merge_solutions(
            Vec::<HashSet<usize>>::new(),
            Vec::<HashSet<usize>>::new(),
            10,
        );
        assert!(result.is_empty());
    }

    #[test]
    fn merge_one_empty_one_nonempty() {
        let cover1 = vec![HashSet::from_iter(vec![1, 2, 3])];
        let result: Vec<HashSet<usize>> = merge_solutions(cover1, Vec::<HashSet<usize>>::new(), 10);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], HashSet::from_iter(vec![1, 2, 3]));
    }

    #[test]
    fn merge_prefers_higher_score() {
        let cover1 = vec![
            HashSet::from_iter(vec![1, 2]), // score 2
            HashSet::from_iter(vec![3]),    // score 1
        ];
        let cover2 = vec![
            HashSet::from_iter(vec![4, 5, 6]), // score 3
            HashSet::from_iter(vec![7, 8]),    // score 2
        ];
        let result: Vec<HashSet<usize>> = merge_solutions(cover1, cover2, 3);
        assert_eq!(result.len(), 3);
        // First should be the highest-score item
        assert_eq!(result[0], HashSet::from_iter(vec![4, 5, 6]));
    }

    #[test]
    fn merge_respects_limit() {
        let cover1 = vec![
            HashSet::from_iter(vec![1, 2, 3]),
            HashSet::from_iter(vec![4, 5]),
        ];
        let cover2 = vec![
            HashSet::from_iter(vec![6, 7, 8, 9]),
            HashSet::from_iter(vec![10]),
        ];
        let result: Vec<HashSet<usize>> = merge_solutions(cover1, cover2, 2);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn merge_zero_limit() {
        let cover1 = vec![HashSet::from_iter(vec![1, 2])];
        let cover2 = vec![HashSet::from_iter(vec![3, 4])];
        let result: Vec<HashSet<usize>> = merge_solutions(cover1, cover2, 0);
        assert!(result.is_empty());
    }

    #[test]
    fn merge_equal_scores_stable() {
        let cover1 = vec![
            HashSet::from_iter(vec![1, 2]),
            HashSet::from_iter(vec![3, 4]),
        ];
        let cover2 = vec![
            HashSet::from_iter(vec![5, 6]),
            HashSet::from_iter(vec![7, 8]),
        ];
        // All have score 2, merge_by with >= favors cover1 when equal
        let result: Vec<HashSet<usize>> = merge_solutions(cover1, cover2, 4);
        assert_eq!(result.len(), 4);
    }

    // ── maximum_cover additional edge cases ──────────────────

    #[test]
    fn maximum_cover_empty_input() {
        let sets: Vec<HashSet<usize>> = vec![];
        let cover = maximum_cover(sets, 5, "test");
        assert!(cover.is_empty());
    }

    #[test]
    fn maximum_cover_all_zero_score() {
        let sets: Vec<HashSet<usize>> = vec![HashSet::new(), HashSet::new()];
        let cover = maximum_cover(sets, 5, "test");
        assert!(cover.is_empty(), "all zero-score items should be excluded");
    }

    #[test]
    fn maximum_cover_single_item() {
        let sets = vec![HashSet::from_iter(vec![1, 2, 3])];
        let cover = maximum_cover(sets.clone(), 5, "test");
        assert_eq!(cover.len(), 1);
        assert_eq!(cover[0], sets[0]);
    }

    #[test]
    fn maximum_cover_disjoint_sets() {
        let sets = vec![
            HashSet::from_iter(vec![1, 2]),
            HashSet::from_iter(vec![3, 4]),
            HashSet::from_iter(vec![5, 6]),
        ];
        let cover = maximum_cover(sets, 3, "test");
        assert_eq!(cover.len(), 3);
        assert_eq!(quality(&cover), 6);
    }
}
