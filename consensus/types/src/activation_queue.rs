use crate::{ChainSpec, Epoch, Validator};
use std::collections::BTreeSet;

/// Activation queue computed during epoch processing for use in the *next* epoch.
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Debug, PartialEq, Eq, Default, Clone)]
pub struct ActivationQueue {
    /// Validators represented by `(activation_eligibility_epoch, index)` in sorted order.
    ///
    /// These validators are not *necessarily* going to be activated. Their activation depends
    /// on how finalization is updated, and the `churn_limit`.
    queue: BTreeSet<(Epoch, usize)>,
}

impl ActivationQueue {
    /// Check if `validator` could be eligible for activation in the next epoch and add them to
    /// the tentative activation queue if this is the case.
    pub fn add_if_could_be_eligible_for_activation(
        &mut self,
        index: usize,
        validator: &Validator,
        next_epoch: Epoch,
        spec: &ChainSpec,
    ) {
        if validator.could_be_eligible_for_activation_at(next_epoch, spec) {
            self.queue
                .insert((validator.activation_eligibility_epoch, index));
        }
    }

    /// Determine the final activation queue after accounting for finalization & the churn limit.
    pub fn get_validators_eligible_for_activation(
        &self,
        finalized_epoch: Epoch,
        churn_limit: usize,
    ) -> BTreeSet<usize> {
        self.queue
            .iter()
            .filter_map(|&(eligibility_epoch, index)| {
                (eligibility_epoch <= finalized_epoch).then_some(index)
            })
            .take(churn_limit)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EthSpec;

    fn make_spec() -> ChainSpec {
        crate::MinimalEthSpec::default_spec()
    }

    fn validator_eligible_at(epoch: Epoch, spec: &ChainSpec) -> Validator {
        Validator {
            activation_eligibility_epoch: epoch,
            activation_epoch: spec.far_future_epoch,
            ..Validator::default()
        }
    }

    #[test]
    fn empty_queue() {
        let queue = ActivationQueue::default();
        let eligible = queue.get_validators_eligible_for_activation(Epoch::new(10), 100);
        assert!(eligible.is_empty());
    }

    #[test]
    fn add_eligible_validator() {
        let spec = make_spec();
        let mut queue = ActivationQueue::default();
        let v = validator_eligible_at(Epoch::new(3), &spec);
        // next_epoch=5 > eligibility_epoch=3, so could_be_eligible
        queue.add_if_could_be_eligible_for_activation(0, &v, Epoch::new(5), &spec);
        assert_eq!(queue.queue.len(), 1);
    }

    #[test]
    fn skip_already_activated_validator() {
        let spec = make_spec();
        let mut queue = ActivationQueue::default();
        let v = Validator {
            activation_eligibility_epoch: Epoch::new(3),
            activation_epoch: Epoch::new(5), // already activated
            ..Validator::default()
        };
        queue.add_if_could_be_eligible_for_activation(0, &v, Epoch::new(10), &spec);
        assert!(queue.queue.is_empty());
    }

    #[test]
    fn get_eligible_respects_finalized_epoch() {
        let spec = make_spec();
        let mut queue = ActivationQueue::default();
        // Add validators with eligibility at epoch 2, 5, 8
        for (i, e) in [2u64, 5, 8].iter().enumerate() {
            let v = validator_eligible_at(Epoch::new(*e), &spec);
            queue.add_if_could_be_eligible_for_activation(i, &v, Epoch::new(20), &spec);
        }
        // Only those with eligibility_epoch <= finalized_epoch=5 are eligible
        let eligible = queue.get_validators_eligible_for_activation(Epoch::new(5), 100);
        assert_eq!(eligible.len(), 2);
        assert!(eligible.contains(&0)); // epoch 2
        assert!(eligible.contains(&1)); // epoch 5
        assert!(!eligible.contains(&2)); // epoch 8
    }

    #[test]
    fn get_eligible_respects_churn_limit() {
        let spec = make_spec();
        let mut queue = ActivationQueue::default();
        for i in 0..10 {
            let v = validator_eligible_at(Epoch::new(1), &spec);
            queue.add_if_could_be_eligible_for_activation(i, &v, Epoch::new(20), &spec);
        }
        let eligible = queue.get_validators_eligible_for_activation(Epoch::new(10), 3);
        assert_eq!(eligible.len(), 3);
    }

    #[test]
    fn get_eligible_sorted_by_eligibility_epoch_then_index() {
        let spec = make_spec();
        let mut queue = ActivationQueue::default();
        // Index 5 at epoch 1, index 2 at epoch 1, index 10 at epoch 0
        let v0 = validator_eligible_at(Epoch::new(1), &spec);
        let v1 = validator_eligible_at(Epoch::new(1), &spec);
        let v2 = validator_eligible_at(Epoch::new(0), &spec);
        queue.add_if_could_be_eligible_for_activation(5, &v0, Epoch::new(20), &spec);
        queue.add_if_could_be_eligible_for_activation(2, &v1, Epoch::new(20), &spec);
        queue.add_if_could_be_eligible_for_activation(10, &v2, Epoch::new(20), &spec);

        // With churn_limit=2, should get index 10 (epoch 0) then index 2 (epoch 1, lower index)
        let eligible = queue.get_validators_eligible_for_activation(Epoch::new(1), 2);
        assert_eq!(eligible.len(), 2);
        assert!(eligible.contains(&10));
        assert!(eligible.contains(&2));
    }

    #[test]
    fn zero_churn_limit_returns_empty() {
        let spec = make_spec();
        let mut queue = ActivationQueue::default();
        let v = validator_eligible_at(Epoch::new(1), &spec);
        queue.add_if_could_be_eligible_for_activation(0, &v, Epoch::new(20), &spec);
        let eligible = queue.get_validators_eligible_for_activation(Epoch::new(10), 0);
        assert!(eligible.is_empty());
    }

    #[test]
    fn finalized_epoch_zero_filters_all_nonzero() {
        let spec = make_spec();
        let mut queue = ActivationQueue::default();
        let v = validator_eligible_at(Epoch::new(1), &spec);
        queue.add_if_could_be_eligible_for_activation(0, &v, Epoch::new(20), &spec);
        let eligible = queue.get_validators_eligible_for_activation(Epoch::new(0), 100);
        assert!(eligible.is_empty());
    }
}
