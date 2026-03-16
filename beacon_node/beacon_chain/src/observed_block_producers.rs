//! Provides the `ObservedBlockProducers` struct which allows for rejecting gossip blocks from
//! validators that have already produced a block.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;
use types::{BeaconBlockRef, Epoch, EthSpec, Hash256, Slot, Unsigned};

#[derive(Debug, PartialEq)]
pub enum Error {
    /// The slot of the provided block is prior to finalization and should not have been provided
    /// to this function. This is an internal error.
    FinalizedBlock { slot: Slot, finalized_slot: Slot },
    /// The function to obtain a set index failed, this is an internal error.
    ValidatorIndexTooHigh(u64),
}

#[derive(Eq, Hash, PartialEq, Debug, Default)]
pub struct ProposalKey {
    pub slot: Slot,
    pub proposer: u64,
}

impl ProposalKey {
    pub fn new(proposer: u64, slot: Slot) -> Self {
        Self { slot, proposer }
    }
}

/// Maintains a cache of observed `(block.slot, block.proposer)`.
///
/// The cache supports pruning based upon the finalized epoch. It does not automatically prune, you
/// must call `Self::prune` manually.
///
/// The maximum size of the cache is determined by `slots_since_finality *
/// VALIDATOR_REGISTRY_LIMIT`. This is quite a large size, so it's important that upstream
/// functions only use this cache for blocks with a valid signature. Only allowing valid signed
/// blocks reduces the theoretical maximum size of this cache to `slots_since_finality *
/// active_validator_count`, however in reality that is more like `slots_since_finality *
/// known_distinct_shufflings` which is much smaller.
pub struct ObservedBlockProducers<E: EthSpec> {
    finalized_slot: Slot,
    items: HashMap<ProposalKey, HashSet<Hash256>>,
    _phantom: PhantomData<E>,
}

impl<E: EthSpec> Default for ObservedBlockProducers<E> {
    /// Instantiates `Self` with `finalized_slot == 0`.
    fn default() -> Self {
        Self {
            finalized_slot: Slot::new(0),
            items: HashMap::new(),
            _phantom: PhantomData,
        }
    }
}

#[derive(Debug)]
pub enum SeenBlock {
    Duplicate,
    Slashable,
    UniqueNonSlashable,
}

impl SeenBlock {
    pub fn proposer_previously_observed(self) -> bool {
        match self {
            Self::Duplicate | Self::Slashable => true,
            Self::UniqueNonSlashable => false,
        }
    }
    pub fn is_slashable(&self) -> bool {
        matches!(self, Self::Slashable)
    }
}

impl<E: EthSpec> ObservedBlockProducers<E> {
    /// Observe that the `block` was produced by `block.proposer_index` at `block.slot`. This will
    /// update `self` so future calls to it indicate that this block is known.
    ///
    /// The supplied `block` **MUST** be signature verified (see struct-level documentation).
    ///
    /// ## Errors
    ///
    /// - `block.proposer_index` is greater than `VALIDATOR_REGISTRY_LIMIT`.
    /// - `block.slot` is equal to or less than the latest pruned `finalized_slot`.
    pub fn observe_proposal(
        &mut self,
        block_root: Hash256,
        block: BeaconBlockRef<'_, E>,
    ) -> Result<SeenBlock, Error> {
        self.sanitize_block(block)?;

        let key = ProposalKey {
            slot: block.slot(),
            proposer: block.proposer_index(),
        };

        let entry = self.items.entry(key);

        let slashable_proposal = match entry {
            Entry::Occupied(mut occupied_entry) => {
                let block_roots = occupied_entry.get_mut();
                let newly_inserted = block_roots.insert(block_root);

                let is_equivocation = block_roots.len() > 1;

                if is_equivocation {
                    SeenBlock::Slashable
                } else if !newly_inserted {
                    SeenBlock::Duplicate
                } else {
                    SeenBlock::UniqueNonSlashable
                }
            }
            Entry::Vacant(vacant_entry) => {
                let block_roots = HashSet::from([block_root]);
                vacant_entry.insert(block_roots);

                SeenBlock::UniqueNonSlashable
            }
        };

        Ok(slashable_proposal)
    }

    /// Returns `Ok(true)` if the `block` has been observed before, `Ok(false)` if not. Does not
    /// update the cache, so calling this function multiple times will continue to return
    /// `Ok(false)`, until `Self::observe_proposer` is called.
    ///
    /// ## Errors
    ///
    /// - `block.proposer_index` is greater than `VALIDATOR_REGISTRY_LIMIT`.
    /// - `block.slot` is equal to or less than the latest pruned `finalized_slot`.
    pub fn proposer_has_been_observed(
        &self,
        block: BeaconBlockRef<'_, E>,
        block_root: Hash256,
    ) -> Result<SeenBlock, Error> {
        self.sanitize_block(block)?;

        let key = ProposalKey {
            slot: block.slot(),
            proposer: block.proposer_index(),
        };

        if let Some(block_roots) = self.items.get(&key) {
            let block_already_known = block_roots.contains(&block_root);
            let has_other_blocks = block_roots.iter().any(|r| r != &block_root);

            if has_other_blocks {
                Ok(SeenBlock::Slashable)
            } else if block_already_known {
                Ok(SeenBlock::Duplicate)
            } else {
                Ok(SeenBlock::UniqueNonSlashable)
            }
        } else {
            Ok(SeenBlock::UniqueNonSlashable)
        }
    }

    /// Returns `Ok(())` if the given `block` is sane.
    fn sanitize_block(&self, block: BeaconBlockRef<'_, E>) -> Result<(), Error> {
        if block.proposer_index() >= E::ValidatorRegistryLimit::to_u64() {
            return Err(Error::ValidatorIndexTooHigh(block.proposer_index()));
        }

        let finalized_slot = self.finalized_slot;
        if finalized_slot > 0 && block.slot() <= finalized_slot {
            return Err(Error::FinalizedBlock {
                slot: block.slot(),
                finalized_slot,
            });
        }

        Ok(())
    }

    /// Removes all observations of blocks equal to or earlier than `finalized_slot`.
    ///
    /// Stores `finalized_slot` in `self`, so that `self` will reject any block that has a slot
    /// equal to or less than `finalized_slot`.
    ///
    /// No-op if `finalized_slot == 0`.
    pub fn prune(&mut self, finalized_slot: Slot) {
        if finalized_slot == 0 {
            return;
        }

        self.finalized_slot = finalized_slot;
        self.items.retain(|key, _| key.slot > finalized_slot);
    }

    /// Returns `true` if the given `validator_index` has been stored in `self` at `epoch`.
    ///
    /// This is useful for doppelganger detection.
    pub fn index_seen_at_epoch(&self, validator_index: u64, epoch: Epoch) -> bool {
        self.items.iter().any(|(key, _)| {
            key.slot.epoch(E::slots_per_epoch()) == epoch && key.proposer == validator_index
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{BeaconBlock, Epoch, MainnetEthSpec, Unsigned};

    type E = MainnetEthSpec;

    fn get_block(slot: u64, proposer: u64) -> BeaconBlock<E> {
        let mut block = BeaconBlock::empty(&E::default_spec());
        *block.slot_mut() = slot.into();
        *block.proposer_index_mut() = proposer;
        block
    }

    #[test]
    fn pruning() {
        let mut cache = ObservedBlockProducers::default();

        assert_eq!(cache.finalized_slot, 0, "finalized slot is zero");
        assert_eq!(cache.items.len(), 0, "no slots should be present");

        // Slot 0, proposer 0
        let block_a = get_block(0, 0);
        let block_root = block_a.canonical_root();

        assert_eq!(
            cache
                .observe_proposal(block_root, block_a.to_ref())
                .map(SeenBlock::proposer_previously_observed),
            Ok(false),
            "can observe proposer, indicates proposer unobserved"
        );

        /*
         * Preconditions.
         */

        assert_eq!(cache.finalized_slot, 0, "finalized slot is zero");
        assert_eq!(cache.items.len(), 1, "only one slot should be present");
        assert_eq!(
            cache
                .items
                .get(&ProposalKey {
                    slot: Slot::new(0),
                    proposer: 0
                })
                .expect("slot zero should be present")
                .len(),
            1,
            "only one proposer should be present"
        );

        /*
         * Check that a prune at the genesis slot does nothing.
         */

        cache.prune(Slot::new(0));

        assert_eq!(cache.finalized_slot, 0, "finalized slot is zero");
        assert_eq!(cache.items.len(), 1, "only one slot should be present");
        assert_eq!(
            cache
                .items
                .get(&ProposalKey {
                    slot: Slot::new(0),
                    proposer: 0
                })
                .expect("slot zero should be present")
                .len(),
            1,
            "only one proposer should be present"
        );

        /*
         * Check that a prune empties the cache
         */

        cache.prune(E::slots_per_epoch().into());
        assert_eq!(
            cache.finalized_slot,
            Slot::from(E::slots_per_epoch()),
            "finalized slot is updated"
        );
        assert_eq!(cache.items.len(), 0, "no items left");

        /*
         * Check that we can't insert a finalized block
         */

        // First slot of finalized epoch, proposer 0
        let block_b = get_block(E::slots_per_epoch(), 0);
        let block_root_b = block_b.canonical_root();

        assert_eq!(
            cache
                .observe_proposal(block_root_b, block_b.to_ref())
                .map(SeenBlock::proposer_previously_observed),
            Err(Error::FinalizedBlock {
                slot: E::slots_per_epoch().into(),
                finalized_slot: E::slots_per_epoch().into(),
            }),
            "cant insert finalized block"
        );

        assert_eq!(cache.items.len(), 0, "block was not added");

        /*
         * Check that we _can_ insert a non-finalized block
         */

        let three_epochs = E::slots_per_epoch() * 3;

        // First slot of finalized epoch, proposer 0
        let block_b = get_block(three_epochs, 0);

        assert_eq!(
            cache
                .observe_proposal(block_root_b, block_b.to_ref())
                .map(SeenBlock::proposer_previously_observed),
            Ok(false),
            "can insert non-finalized block"
        );

        assert_eq!(cache.items.len(), 1, "only one slot should be present");
        assert_eq!(
            cache
                .items
                .get(&ProposalKey {
                    slot: Slot::new(three_epochs),
                    proposer: 0
                })
                .expect("the three epochs slot should be present")
                .len(),
            1,
            "only one proposer should be present"
        );

        /*
         * Check that a prune doesnt wipe later blocks
         */

        let two_epochs = E::slots_per_epoch() * 2;
        cache.prune(two_epochs.into());

        assert_eq!(
            cache.finalized_slot,
            Slot::from(two_epochs),
            "finalized slot is updated"
        );

        assert_eq!(cache.items.len(), 1, "only one slot should be present");
        assert_eq!(
            cache
                .items
                .get(&ProposalKey {
                    slot: Slot::new(three_epochs),
                    proposer: 0
                })
                .expect("the three epochs slot should be present")
                .len(),
            1,
            "only one proposer should be present"
        );
    }

    #[test]
    fn simple_observations() {
        let mut cache = ObservedBlockProducers::default();

        // Slot 0, proposer 0
        let block_a = get_block(0, 0);
        let block_root_a = block_a.canonical_root();

        assert_eq!(
            cache
                .proposer_has_been_observed(block_a.to_ref(), block_a.canonical_root())
                .map(|x| x.proposer_previously_observed()),
            Ok(false),
            "no observation in empty cache"
        );
        assert_eq!(
            cache
                .observe_proposal(block_root_a, block_a.to_ref())
                .map(SeenBlock::proposer_previously_observed),
            Ok(false),
            "can observe proposer, indicates proposer unobserved"
        );
        assert_eq!(
            cache
                .proposer_has_been_observed(block_a.to_ref(), block_a.canonical_root())
                .map(|x| x.proposer_previously_observed()),
            Ok(true),
            "observed block is indicated as true"
        );
        assert_eq!(
            cache
                .observe_proposal(block_root_a, block_a.to_ref())
                .map(SeenBlock::proposer_previously_observed),
            Ok(true),
            "observing again indicates true"
        );

        assert_eq!(cache.finalized_slot, 0, "finalized slot is zero");
        assert_eq!(cache.items.len(), 1, "only one slot should be present");
        assert_eq!(
            cache
                .items
                .get(&ProposalKey {
                    slot: Slot::new(0),
                    proposer: 0
                })
                .expect("slot zero should be present")
                .len(),
            1,
            "only one proposer should be present"
        );

        // Slot 1, proposer 0
        let block_b = get_block(1, 0);
        let block_root_b = block_b.canonical_root();

        assert_eq!(
            cache
                .proposer_has_been_observed(block_b.to_ref(), block_b.canonical_root())
                .map(|x| x.proposer_previously_observed()),
            Ok(false),
            "no observation for new slot"
        );
        assert_eq!(
            cache
                .observe_proposal(block_root_b, block_b.to_ref())
                .map(SeenBlock::proposer_previously_observed),
            Ok(false),
            "can observe proposer for new slot, indicates proposer unobserved"
        );
        assert_eq!(
            cache
                .proposer_has_been_observed(block_b.to_ref(), block_b.canonical_root())
                .map(|x| x.proposer_previously_observed()),
            Ok(true),
            "observed block in slot 1 is indicated as true"
        );
        assert_eq!(
            cache
                .observe_proposal(block_root_b, block_b.to_ref())
                .map(SeenBlock::proposer_previously_observed),
            Ok(true),
            "observing slot 1 again indicates true"
        );

        assert_eq!(cache.finalized_slot, 0, "finalized slot is zero");
        assert_eq!(cache.items.len(), 2, "two slots should be present");
        assert_eq!(
            cache
                .items
                .get(&ProposalKey {
                    slot: Slot::new(0),
                    proposer: 0
                })
                .expect("slot zero should be present")
                .len(),
            1,
            "only one proposer should be present in slot 0"
        );
        assert_eq!(
            cache
                .items
                .get(&ProposalKey {
                    slot: Slot::new(1),
                    proposer: 0
                })
                .expect("slot zero should be present")
                .len(),
            1,
            "only one proposer should be present in slot 1"
        );

        // Slot 0, proposer 1
        let block_c = get_block(0, 1);
        let block_root_c = block_c.canonical_root();

        assert_eq!(
            cache
                .proposer_has_been_observed(block_c.to_ref(), block_c.canonical_root())
                .map(|x| x.proposer_previously_observed()),
            Ok(false),
            "no observation for new proposer"
        );
        assert_eq!(
            cache
                .observe_proposal(block_root_c, block_c.to_ref())
                .map(SeenBlock::proposer_previously_observed),
            Ok(false),
            "can observe new proposer, indicates proposer unobserved"
        );
        assert_eq!(
            cache
                .proposer_has_been_observed(block_c.to_ref(), block_c.canonical_root())
                .map(|x| x.proposer_previously_observed()),
            Ok(true),
            "observed new proposer block is indicated as true"
        );
        assert_eq!(
            cache
                .observe_proposal(block_root_c, block_c.to_ref())
                .map(SeenBlock::proposer_previously_observed),
            Ok(true),
            "observing new proposer again indicates true"
        );

        assert_eq!(cache.finalized_slot, 0, "finalized slot is zero");
        assert_eq!(cache.items.len(), 3, "three slots should be present");
        assert_eq!(
            cache
                .items
                .iter()
                .filter(|(k, _)| k.slot == cache.finalized_slot)
                .count(),
            2,
            "two proposers should be present in slot 0"
        );
        assert_eq!(
            cache
                .items
                .iter()
                .filter(|(k, _)| k.slot == Slot::new(1))
                .count(),
            1,
            "only one proposer should be present in slot 1"
        );
    }

    #[test]
    fn slashable_detection() {
        let mut cache = ObservedBlockProducers::<E>::default();

        // Same slot, same proposer, different block roots = slashable
        let block_a = get_block(5, 42);
        let root_a = block_a.canonical_root();

        // Create a different block with the same slot and proposer but different content
        let mut block_b = get_block(5, 42);
        *block_b.state_root_mut() = Hash256::repeat_byte(0xFF);
        let root_b = block_b.canonical_root();
        assert_ne!(root_a, root_b, "sanity: block roots should differ");

        // First observation: unique
        let result = cache.observe_proposal(root_a, block_a.to_ref()).unwrap();
        assert!(!result.is_slashable());
        // (proposer_previously_observed consumes self, call it last)

        // Second observation with different root: slashable
        let result = cache.observe_proposal(root_b, block_b.to_ref()).unwrap();
        assert!(result.is_slashable());

        // Re-observe first block: still slashable (2 distinct roots exist)
        let result = cache.observe_proposal(root_a, block_a.to_ref()).unwrap();
        assert!(result.is_slashable());

        // Check proposer_has_been_observed for slashable
        let seen = cache
            .proposer_has_been_observed(block_a.to_ref(), root_a)
            .unwrap();
        assert!(seen.is_slashable());

        // Check proposer_has_been_observed with a third unknown root
        let seen = cache
            .proposer_has_been_observed(block_a.to_ref(), Hash256::repeat_byte(0xAA))
            .unwrap();
        assert!(seen.is_slashable());
    }

    #[test]
    fn duplicate_is_not_slashable() {
        let mut cache = ObservedBlockProducers::<E>::default();

        let block = get_block(1, 0);
        let root = block.canonical_root();

        cache.observe_proposal(root, block.to_ref()).unwrap();
        let result = cache.observe_proposal(root, block.to_ref()).unwrap();

        // Same root re-observed = duplicate, NOT slashable
        assert!(!result.is_slashable());
        assert!(result.proposer_previously_observed());
    }

    #[test]
    fn index_seen_at_epoch() {
        let mut cache = ObservedBlockProducers::<E>::default();

        let slots_per_epoch = E::slots_per_epoch();

        // Epoch 0, validator 5
        let block = get_block(0, 5);
        let root = block.canonical_root();
        cache.observe_proposal(root, block.to_ref()).unwrap();

        assert!(
            cache.index_seen_at_epoch(5, Epoch::new(0)),
            "validator 5 seen in epoch 0"
        );
        assert!(
            !cache.index_seen_at_epoch(5, Epoch::new(1)),
            "validator 5 not seen in epoch 1"
        );
        assert!(
            !cache.index_seen_at_epoch(99, Epoch::new(0)),
            "validator 99 not seen"
        );

        // Add validator 5 in epoch 1 as well
        let block2 = get_block(slots_per_epoch, 5);
        let root2 = block2.canonical_root();
        cache.observe_proposal(root2, block2.to_ref()).unwrap();

        assert!(cache.index_seen_at_epoch(5, Epoch::new(1)));
    }

    #[test]
    fn validator_index_too_high() {
        let mut cache = ObservedBlockProducers::<E>::default();

        let limit = <E as types::EthSpec>::ValidatorRegistryLimit::to_u64();
        let block = get_block(0, limit);
        let root = block.canonical_root();

        let result = cache.observe_proposal(root, block.to_ref());
        assert!(
            matches!(
                result,
                Err(Error::ValidatorIndexTooHigh(idx)) if idx == limit
            ),
            "expected ValidatorIndexTooHigh, got {:?}",
            result,
        );
    }

    #[test]
    fn proposal_key_equality() {
        let k1 = ProposalKey::new(42, Slot::new(10));
        let k2 = ProposalKey::new(42, Slot::new(10));
        let k3 = ProposalKey::new(42, Slot::new(11));
        let k4 = ProposalKey::new(43, Slot::new(10));

        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
        assert_ne!(k1, k4);
    }

    #[test]
    fn seen_block_methods() {
        assert!(!SeenBlock::UniqueNonSlashable.is_slashable());
        assert!(!SeenBlock::UniqueNonSlashable.proposer_previously_observed());

        assert!(!SeenBlock::Duplicate.is_slashable());
        assert!(SeenBlock::Duplicate.proposer_previously_observed());

        assert!(SeenBlock::Slashable.is_slashable());
        assert!(SeenBlock::Slashable.proposer_previously_observed());
    }

    #[test]
    fn prune_retains_later_slots() {
        let mut cache = ObservedBlockProducers::<E>::default();

        for slot in 0..10 {
            let block = get_block(slot, 0);
            let root = block.canonical_root();
            cache.observe_proposal(root, block.to_ref()).unwrap();
        }

        assert_eq!(cache.items.len(), 10);

        cache.prune(Slot::new(5));
        // Retain only slots > 5: slots 6, 7, 8, 9
        assert_eq!(cache.items.len(), 4);

        // Can't insert at finalized slot
        let block = get_block(5, 1);
        let root = block.canonical_root();
        assert!(matches!(
            cache.observe_proposal(root, block.to_ref()),
            Err(Error::FinalizedBlock { .. })
        ));
    }
}
