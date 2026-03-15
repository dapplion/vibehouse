use crate::{BeaconStateError, Slot, Validator};
#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
use rpds::HashTrieSetSync as HashTrieSet;

/// Persistent (cheap to clone) cache of all slashed validator indices.
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SlashingsCache {
    latest_block_slot: Option<Slot>,
    #[cfg_attr(feature = "arbitrary", arbitrary(default))]
    slashed_validators: HashTrieSet<usize>,
}

impl SlashingsCache {
    /// Initialize a new cache for the given list of validators.
    pub fn new<'a, V, I>(latest_block_slot: Slot, validators: V) -> Self
    where
        V: IntoIterator<Item = &'a Validator, IntoIter = I>,
        I: ExactSizeIterator + Iterator<Item = &'a Validator>,
    {
        let slashed_validators = validators
            .into_iter()
            .enumerate()
            .filter_map(|(i, validator)| validator.slashed.then_some(i))
            .collect();
        Self {
            latest_block_slot: Some(latest_block_slot),
            slashed_validators,
        }
    }

    pub fn is_initialized(&self, slot: Slot) -> bool {
        self.latest_block_slot == Some(slot)
    }

    pub fn check_initialized(&self, latest_block_slot: Slot) -> Result<(), BeaconStateError> {
        if self.is_initialized(latest_block_slot) {
            Ok(())
        } else {
            Err(BeaconStateError::SlashingsCacheUninitialized {
                initialized_slot: self.latest_block_slot,
                latest_block_slot,
            })
        }
    }

    pub fn record_validator_slashing(
        &mut self,
        block_slot: Slot,
        validator_index: usize,
    ) -> Result<(), BeaconStateError> {
        self.check_initialized(block_slot)?;
        self.slashed_validators.insert_mut(validator_index);
        Ok(())
    }

    pub fn is_slashed(&self, validator_index: usize) -> bool {
        self.slashed_validators.contains(&validator_index)
    }

    pub fn update_latest_block_slot(&mut self, latest_block_slot: Slot) {
        self.latest_block_slot = Some(latest_block_slot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_validator(slashed: bool) -> Validator {
        Validator {
            slashed,
            ..Validator::default()
        }
    }

    #[test]
    fn default_is_uninitialized() {
        let cache = SlashingsCache::default();
        assert!(!cache.is_initialized(Slot::new(0)));
        assert!(cache.check_initialized(Slot::new(0)).is_err());
        assert!(!cache.is_slashed(0));
    }

    #[test]
    fn new_no_slashed_validators() {
        let validators = [make_validator(false), make_validator(false)];
        let cache = SlashingsCache::new(Slot::new(5), validators.iter());
        assert!(cache.is_initialized(Slot::new(5)));
        assert!(!cache.is_initialized(Slot::new(6)));
        assert!(!cache.is_slashed(0));
        assert!(!cache.is_slashed(1));
    }

    #[test]
    fn new_with_slashed_validators() {
        let validators = [
            make_validator(false),
            make_validator(true),
            make_validator(false),
            make_validator(true),
        ];
        let cache = SlashingsCache::new(Slot::new(10), validators.iter());
        assert!(!cache.is_slashed(0));
        assert!(cache.is_slashed(1));
        assert!(!cache.is_slashed(2));
        assert!(cache.is_slashed(3));
        // Out-of-range index returns false
        assert!(!cache.is_slashed(99));
    }

    #[test]
    fn check_initialized_correct_slot() {
        let validators = [make_validator(false)];
        let cache = SlashingsCache::new(Slot::new(7), validators.iter());
        assert!(cache.check_initialized(Slot::new(7)).is_ok());
    }

    #[test]
    fn check_initialized_wrong_slot() {
        let validators = [make_validator(false)];
        let cache = SlashingsCache::new(Slot::new(7), validators.iter());
        assert!(cache.check_initialized(Slot::new(8)).is_err());
    }

    #[test]
    fn record_validator_slashing_success() {
        let validators = [make_validator(false), make_validator(false)];
        let mut cache = SlashingsCache::new(Slot::new(3), validators.iter());
        assert!(!cache.is_slashed(0));
        cache.record_validator_slashing(Slot::new(3), 0).unwrap();
        assert!(cache.is_slashed(0));
        assert!(!cache.is_slashed(1));
    }

    #[test]
    fn record_validator_slashing_wrong_slot() {
        let validators = [make_validator(false)];
        let mut cache = SlashingsCache::new(Slot::new(3), validators.iter());
        assert!(cache.record_validator_slashing(Slot::new(4), 0).is_err());
    }

    #[test]
    fn record_slashing_idempotent() {
        let validators = [make_validator(false)];
        let mut cache = SlashingsCache::new(Slot::new(1), validators.iter());
        cache.record_validator_slashing(Slot::new(1), 0).unwrap();
        cache.record_validator_slashing(Slot::new(1), 0).unwrap();
        assert!(cache.is_slashed(0));
    }

    #[test]
    fn update_latest_block_slot_changes_initialization() {
        let validators = [make_validator(false)];
        let mut cache = SlashingsCache::new(Slot::new(1), validators.iter());
        assert!(cache.is_initialized(Slot::new(1)));
        cache.update_latest_block_slot(Slot::new(5));
        assert!(!cache.is_initialized(Slot::new(1)));
        assert!(cache.is_initialized(Slot::new(5)));
    }

    #[test]
    fn update_slot_preserves_slashed_set() {
        let validators = [make_validator(true), make_validator(false)];
        let mut cache = SlashingsCache::new(Slot::new(1), validators.iter());
        assert!(cache.is_slashed(0));
        cache.update_latest_block_slot(Slot::new(2));
        // Slashed set should be preserved after slot update
        assert!(cache.is_slashed(0));
        assert!(!cache.is_slashed(1));
    }

    #[test]
    fn new_empty_validators() {
        let validators: [Validator; 0] = [];
        let cache = SlashingsCache::new(Slot::new(0), validators.iter());
        assert!(cache.is_initialized(Slot::new(0)));
        assert!(!cache.is_slashed(0));
    }
}
