use crate::AttestationStats;
use itertools::Itertools;
use std::collections::{BTreeMap, HashMap, HashSet};
use types::{
    AggregateSignature, Attestation, AttestationData, BeaconState, BitList, BitVector, Checkpoint,
    Epoch, EthSpec, Hash256, Slot, Unsigned,
    attestation::{AttestationBase, AttestationElectra},
    superstruct,
};

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct CheckpointKey {
    pub source: Checkpoint,
    pub target_epoch: Epoch,
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct CompactAttestationData {
    pub slot: Slot,
    pub index: u64,
    pub beacon_block_root: Hash256,
    pub target_root: Hash256,
}

#[superstruct(variants(Base, Electra), variant_attributes(derive(Debug, PartialEq,)))]
#[derive(Debug, PartialEq)]
pub struct CompactIndexedAttestation<E: EthSpec> {
    pub attesting_indices: Vec<u64>,
    #[superstruct(only(Base), partial_getter(rename = "aggregation_bits_base"))]
    pub aggregation_bits: BitList<E::MaxValidatorsPerCommittee>,
    #[superstruct(only(Electra), partial_getter(rename = "aggregation_bits_electra"))]
    pub aggregation_bits: BitList<E::MaxValidatorsPerSlot>,
    pub signature: AggregateSignature,
    #[superstruct(only(Electra))]
    pub committee_bits: BitVector<E::MaxCommitteesPerSlot>,
}

#[derive(Debug)]
pub struct SplitAttestation<E: EthSpec> {
    pub checkpoint: CheckpointKey,
    pub data: CompactAttestationData,
    pub indexed: CompactIndexedAttestation<E>,
}

#[derive(Debug, Clone)]
pub struct CompactAttestationRef<'a, E: EthSpec> {
    pub checkpoint: &'a CheckpointKey,
    pub data: &'a CompactAttestationData,
    pub indexed: &'a CompactIndexedAttestation<E>,
}

#[derive(Debug, Default, PartialEq)]
pub struct AttestationMap<E: EthSpec> {
    checkpoint_map: HashMap<CheckpointKey, AttestationDataMap<E>>,
}

#[derive(Debug, Default, PartialEq)]
pub struct AttestationDataMap<E: EthSpec> {
    attestations: HashMap<CompactAttestationData, Vec<CompactIndexedAttestation<E>>>,
}

impl<E: EthSpec> SplitAttestation<E> {
    pub fn new(attestation: Attestation<E>, attesting_indices: Vec<u64>) -> Self {
        let checkpoint = CheckpointKey {
            source: attestation.data().source,
            target_epoch: attestation.data().target.epoch,
        };
        let data = CompactAttestationData {
            slot: attestation.data().slot,
            index: attestation.data().index,
            beacon_block_root: attestation.data().beacon_block_root,
            target_root: attestation.data().target.root,
        };

        let indexed = match attestation.clone() {
            Attestation::Base(attn) => {
                CompactIndexedAttestation::Base(CompactIndexedAttestationBase {
                    attesting_indices,
                    aggregation_bits: attn.aggregation_bits,
                    signature: attestation.signature().clone(),
                })
            }
            Attestation::Electra(attn) => {
                CompactIndexedAttestation::Electra(CompactIndexedAttestationElectra {
                    attesting_indices,
                    aggregation_bits: attn.aggregation_bits,
                    signature: attestation.signature().clone(),
                    committee_bits: attn.committee_bits,
                })
            }
        };

        Self {
            checkpoint,
            data,
            indexed,
        }
    }

    pub fn as_ref(&self) -> CompactAttestationRef<'_, E> {
        CompactAttestationRef {
            checkpoint: &self.checkpoint,
            data: &self.data,
            indexed: &self.indexed,
        }
    }
}

impl<E: EthSpec> CompactAttestationRef<'_, E> {
    pub fn attestation_data(&self) -> AttestationData {
        AttestationData {
            slot: self.data.slot,
            index: self.data.index,
            beacon_block_root: self.data.beacon_block_root,
            source: self.checkpoint.source,
            target: Checkpoint {
                epoch: self.checkpoint.target_epoch,
                root: self.data.target_root,
            },
        }
    }

    pub fn get_committee_indices_map(&self) -> HashSet<u64> {
        match self.indexed {
            CompactIndexedAttestation::Base(_) => HashSet::from([self.data.index]),
            CompactIndexedAttestation::Electra(indexed_att) => indexed_att
                .committee_bits
                .iter()
                .enumerate()
                .filter_map(|(index, bit)| if bit { Some(index as u64) } else { None })
                .collect(),
        }
    }

    pub fn clone_as_attestation(&self) -> Attestation<E> {
        match self.indexed {
            CompactIndexedAttestation::Base(indexed_att) => Attestation::Base(AttestationBase {
                aggregation_bits: indexed_att.aggregation_bits.clone(),
                data: self.attestation_data(),
                signature: indexed_att.signature.clone(),
            }),
            CompactIndexedAttestation::Electra(indexed_att) => {
                Attestation::Electra(AttestationElectra {
                    aggregation_bits: indexed_att.aggregation_bits.clone(),
                    data: self.attestation_data(),
                    signature: indexed_att.signature.clone(),
                    committee_bits: indexed_att.committee_bits.clone(),
                })
            }
        }
    }
}

impl CheckpointKey {
    /// Return two checkpoint keys: `(previous, current)` for the previous and current epochs of
    /// the `state`.
    pub fn keys_for_state<E: EthSpec>(state: &BeaconState<E>) -> (Self, Self) {
        (
            CheckpointKey {
                source: state.previous_justified_checkpoint(),
                target_epoch: state.previous_epoch(),
            },
            CheckpointKey {
                source: state.current_justified_checkpoint(),
                target_epoch: state.current_epoch(),
            },
        )
    }
}

impl<E: EthSpec> CompactIndexedAttestation<E> {
    pub fn should_aggregate(&self, other: &Self) -> bool {
        match (self, other) {
            (CompactIndexedAttestation::Base(this), CompactIndexedAttestation::Base(other)) => {
                this.should_aggregate(other)
            }
            (
                CompactIndexedAttestation::Electra(this),
                CompactIndexedAttestation::Electra(other),
            ) => this.should_aggregate(other),
            _ => false,
        }
    }

    /// Returns `true` if aggregated, otherwise `false`.
    pub fn aggregate(&mut self, other: &Self) -> bool {
        match (self, other) {
            (CompactIndexedAttestation::Base(this), CompactIndexedAttestation::Base(other)) => {
                this.aggregate(other);
                true
            }
            (
                CompactIndexedAttestation::Electra(this),
                CompactIndexedAttestation::Electra(other),
            ) => this.aggregate_same_committee(other),
            _ => false,
        }
    }
}

impl<E: EthSpec> CompactIndexedAttestationBase<E> {
    pub fn should_aggregate(&self, other: &Self) -> bool {
        self.aggregation_bits
            .intersection(&other.aggregation_bits)
            .is_zero()
    }

    pub fn aggregate(&mut self, other: &Self) {
        self.attesting_indices = self
            .attesting_indices
            .drain(..)
            .merge(other.attesting_indices.iter().copied())
            .dedup()
            .collect();
        self.aggregation_bits = self.aggregation_bits.union(&other.aggregation_bits);
        self.signature.add_assign_aggregate(&other.signature);
    }
}

impl<E: EthSpec> CompactIndexedAttestationElectra<E> {
    pub fn should_aggregate(&self, other: &Self) -> bool {
        // For Electra, only aggregate attestations in the same committee.
        self.committee_bits == other.committee_bits
            && self
                .aggregation_bits
                .intersection(&other.aggregation_bits)
                .is_zero()
    }

    /// Returns `true` if aggregated, otherwise `false`.
    pub fn aggregate_same_committee(&mut self, other: &Self) -> bool {
        if self.committee_bits != other.committee_bits {
            return false;
        }
        self.aggregation_bits = self.aggregation_bits.union(&other.aggregation_bits);
        self.attesting_indices = self
            .attesting_indices
            .drain(..)
            .merge(other.attesting_indices.iter().copied())
            .dedup()
            .collect();
        self.signature.add_assign_aggregate(&other.signature);
        true
    }

    pub fn aggregate_with_disjoint_committees(&mut self, other: &Self) -> Option<()> {
        if !self
            .committee_bits
            .intersection(&other.committee_bits)
            .is_zero()
        {
            return None;
        }
        // The attestation being aggregated in must only have 1 committee bit set.
        if other.committee_bits.num_set_bits() != 1 {
            return None;
        }

        // Check we are aggregating in increasing committee index order (so we can append
        // aggregation bits).
        if self.committee_bits.highest_set_bit() >= other.committee_bits.highest_set_bit() {
            return None;
        }

        self.committee_bits = self.committee_bits.union(&other.committee_bits);
        if let Some(agg_bits) = bitlist_extend(&self.aggregation_bits, &other.aggregation_bits) {
            self.aggregation_bits = agg_bits;

            self.attesting_indices = self
                .attesting_indices
                .drain(..)
                .merge(other.attesting_indices.iter().copied())
                .dedup()
                .collect();
            self.signature.add_assign_aggregate(&other.signature);

            return Some(());
        }

        None
    }

    pub fn committee_index(&self) -> Option<u64> {
        self.committee_bits
            .iter()
            .enumerate()
            .find(|&(_, bit)| bit)
            .map(|(index, _)| index as u64)
    }

    pub fn get_committee_indices(&self) -> Vec<u64> {
        self.committee_bits
            .iter()
            .enumerate()
            .filter_map(|(index, bit)| if bit { Some(index as u64) } else { None })
            .collect()
    }
}

/// Concatenate two bitlists using bulk byte operations instead of bit-by-bit iteration.
fn bitlist_extend<N: Unsigned>(list1: &BitList<N>, list2: &BitList<N>) -> Option<BitList<N>> {
    let len1 = list1.len();
    let len2 = list2.len();
    let new_length = len1 + len2;

    if new_length > N::to_usize() {
        return None;
    }

    // SSZ BitList encoding: data bits followed by a 1-bit at position `new_length`.
    let total_bits = new_length + 1;
    let num_bytes = total_bits.div_ceil(8).max(1);
    let mut bytes = smallvec::smallvec![0u8; num_bytes];

    let src1 = list1.as_slice();
    let src2 = list2.as_slice();

    // Copy list1's raw bytes directly.
    let full_bytes1 = len1 / 8;
    bytes[..full_bytes1].copy_from_slice(&src1[..full_bytes1]);

    let bit_offset = len1 % 8;

    if bit_offset == 0 {
        // Byte-aligned: copy list2's bytes directly after list1.
        let full_bytes2 = len2 / 8;
        bytes[full_bytes1..full_bytes1 + full_bytes2].copy_from_slice(&src2[..full_bytes2]);
        // Copy remaining partial byte from list2.
        if !len2.is_multiple_of(8) {
            bytes[full_bytes1 + full_bytes2] = src2[full_bytes2];
        }
    } else {
        // Not byte-aligned: copy partial byte from list1, then shift-and-OR list2's bytes.
        if full_bytes1 < src1.len() {
            bytes[full_bytes1] = src1[full_bytes1];
        }
        for (i, &b) in src2.iter().enumerate() {
            bytes[full_bytes1 + i] |= b << bit_offset;
            if full_bytes1 + i + 1 < num_bytes {
                bytes[full_bytes1 + i + 1] |= b >> (8 - bit_offset);
            }
        }
    }

    // Set the length bit (SSZ BitList sentinel).
    let sentinel_byte = new_length / 8;
    let sentinel_bit = new_length % 8;
    bytes[sentinel_byte] |= 1 << sentinel_bit;

    BitList::from_bytes(bytes).ok()
}

impl<E: EthSpec> AttestationMap<E> {
    pub fn insert(&mut self, attestation: Attestation<E>, attesting_indices: Vec<u64>) {
        let SplitAttestation {
            checkpoint,
            data,
            indexed,
        } = SplitAttestation::new(attestation.clone(), attesting_indices);

        let attestation_map = self.checkpoint_map.entry(checkpoint).or_default();
        let attestations = attestation_map.attestations.entry(data).or_default();

        // Greedily aggregate the attestation with all existing attestations.
        // NOTE: this is sub-optimal and in future we will remove this in favour of max-clique
        // aggregation.
        let mut aggregated = false;

        for existing_attestation in attestations.iter_mut() {
            if existing_attestation.should_aggregate(&indexed) {
                aggregated = existing_attestation.aggregate(&indexed);
            } else if *existing_attestation == indexed {
                aggregated = true;
            }
        }

        if !aggregated {
            attestations.push(indexed);
        }
    }

    /// Aggregate Electra attestations for the same attestation data signed by different
    /// committees.
    ///
    /// Non-Electra attestations are left as-is.
    pub fn aggregate_across_committees(&mut self, checkpoint_key: CheckpointKey) {
        let Some(attestation_map) = self.checkpoint_map.get_mut(&checkpoint_key) else {
            return;
        };
        for compact_indexed_attestations in attestation_map.attestations.values_mut() {
            let unaggregated_attestations = std::mem::take(compact_indexed_attestations);
            let mut aggregated_attestations: Vec<CompactIndexedAttestation<E>> = vec![];

            // Aggregate the best attestations for each committee and leave the rest.
            let mut best_attestations_by_committee: BTreeMap<
                u64,
                CompactIndexedAttestationElectra<E>,
            > = BTreeMap::new();

            for committee_attestation in unaggregated_attestations {
                let mut electra_attestation = match committee_attestation {
                    CompactIndexedAttestation::Electra(att)
                        if att.committee_bits.num_set_bits() == 1 =>
                    {
                        att
                    }
                    CompactIndexedAttestation::Electra(att) => {
                        // Aggregate already covers multiple committees, leave it as-is.
                        aggregated_attestations.push(CompactIndexedAttestation::Electra(att));
                        continue;
                    }
                    CompactIndexedAttestation::Base(att) => {
                        // Leave as-is.
                        aggregated_attestations.push(CompactIndexedAttestation::Base(att));
                        continue;
                    }
                };
                if let Some(committee_index) = electra_attestation.committee_index() {
                    if let Some(existing_attestation) =
                        best_attestations_by_committee.get_mut(&committee_index)
                    {
                        // Search for the best (most aggregation bits) attestation for this committee
                        // index.
                        if electra_attestation.aggregation_bits.num_set_bits()
                            > existing_attestation.aggregation_bits.num_set_bits()
                        {
                            // New attestation is better than the previously known one for this
                            // committee. Replace it.
                            std::mem::swap(existing_attestation, &mut electra_attestation);
                        }
                        // Put the inferior attestation into the list of aggregated attestations
                        // without performing any cross-committee aggregation.
                        aggregated_attestations
                            .push(CompactIndexedAttestation::Electra(electra_attestation));
                    } else {
                        // First attestation seen for this committee. Place it in the map
                        // provisionally.
                        best_attestations_by_committee.insert(committee_index, electra_attestation);
                    }
                }
            }

            if let Some(on_chain_aggregate) =
                Self::compute_on_chain_aggregate(best_attestations_by_committee)
            {
                aggregated_attestations
                    .push(CompactIndexedAttestation::Electra(on_chain_aggregate));
            }

            *compact_indexed_attestations = aggregated_attestations;
        }
    }

    pub fn compute_on_chain_aggregate(
        mut attestations_by_committee: BTreeMap<u64, CompactIndexedAttestationElectra<E>>,
    ) -> Option<CompactIndexedAttestationElectra<E>> {
        let (_, mut on_chain_aggregate) = attestations_by_committee.pop_first()?;
        for (_, attestation) in attestations_by_committee {
            on_chain_aggregate.aggregate_with_disjoint_committees(&attestation);
        }
        Some(on_chain_aggregate)
    }

    /// Iterate all attestations matching the given `checkpoint_key`.
    pub fn get_attestations<'a>(
        &'a self,
        checkpoint_key: &'a CheckpointKey,
    ) -> impl Iterator<Item = CompactAttestationRef<'a, E>> + 'a {
        self.checkpoint_map
            .get(checkpoint_key)
            .into_iter()
            .flat_map(|attestation_map| attestation_map.iter(checkpoint_key))
    }

    /// Iterate all attestations in the map.
    pub fn iter(&self) -> impl Iterator<Item = CompactAttestationRef<'_, E>> {
        self.checkpoint_map
            .iter()
            .flat_map(|(checkpoint_key, attestation_map)| attestation_map.iter(checkpoint_key))
    }

    /// Prune attestations that are from before the previous epoch.
    pub fn prune(&mut self, current_epoch: Epoch) {
        self.checkpoint_map
            .retain(|checkpoint_key, _| current_epoch <= checkpoint_key.target_epoch + 1);
    }

    /// Statistics about all attestations stored in the map.
    pub fn stats(&self) -> AttestationStats {
        self.checkpoint_map
            .values()
            .map(AttestationDataMap::stats)
            .fold(AttestationStats::default(), |mut acc, new| {
                acc.num_attestations += new.num_attestations;
                acc.num_attestation_data += new.num_attestation_data;
                acc.max_aggregates_per_data =
                    std::cmp::max(acc.max_aggregates_per_data, new.max_aggregates_per_data);
                acc
            })
    }
}

impl<E: EthSpec> AttestationDataMap<E> {
    pub fn iter<'a>(
        &'a self,
        checkpoint_key: &'a CheckpointKey,
    ) -> impl Iterator<Item = CompactAttestationRef<'a, E>> + 'a {
        self.attestations.iter().flat_map(|(data, vec_indexed)| {
            vec_indexed.iter().map(|indexed| CompactAttestationRef {
                checkpoint: checkpoint_key,
                data,
                indexed,
            })
        })
    }

    pub fn stats(&self) -> AttestationStats {
        let mut stats = AttestationStats::default();

        for aggregates in self.attestations.values() {
            stats.num_attestations += aggregates.len();
            stats.num_attestation_data += 1;
            stats.max_aggregates_per_data =
                std::cmp::max(stats.max_aggregates_per_data, aggregates.len());
        }
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{FixedBytesExtended, MinimalEthSpec, typenum};

    type E = MinimalEthSpec;

    fn make_checkpoint() -> CheckpointKey {
        CheckpointKey {
            source: Checkpoint {
                epoch: Epoch::new(0),
                root: Hash256::zero(),
            },
            target_epoch: Epoch::new(1),
        }
    }

    fn make_electra_attestation(slot: u64, index: u64, committee_idx: u64) -> Attestation<E> {
        let mut committee_bits: BitVector<<E as EthSpec>::MaxCommitteesPerSlot> =
            BitVector::default();
        committee_bits.set(committee_idx as usize, true).unwrap();

        Attestation::Electra(AttestationElectra {
            aggregation_bits: BitList::with_capacity(8).unwrap(),
            data: AttestationData {
                slot: Slot::new(slot),
                index,
                beacon_block_root: Hash256::repeat_byte(0x01),
                source: Checkpoint {
                    epoch: Epoch::new(0),
                    root: Hash256::zero(),
                },
                target: Checkpoint {
                    epoch: Epoch::new(1),
                    root: Hash256::repeat_byte(0x02),
                },
            },
            committee_bits,
            signature: AggregateSignature::infinity(),
        })
    }

    fn set_attestation_bit(att: &mut Attestation<E>, bit: usize) {
        match att {
            Attestation::Electra(a) => a.aggregation_bits.set(bit, true).unwrap(),
            Attestation::Base(a) => a.aggregation_bits.set(bit, true).unwrap(),
        }
    }

    /// Index=0 and index=1 attestations for the same slot/block are stored in separate buckets.
    /// In Gloas, index=0 means payload-absent or same-slot, index=1 means payload-present.
    /// They represent different votes and must not be merged.
    #[test]
    fn gloas_index_zero_and_one_stored_separately() {
        let mut map = AttestationMap::<E>::default();

        let mut att0 = make_electra_attestation(10, 0, 0);
        set_attestation_bit(&mut att0, 0);
        map.insert(att0, vec![100]);

        let mut att1 = make_electra_attestation(10, 1, 0);
        set_attestation_bit(&mut att1, 0);
        map.insert(att1, vec![100]);

        let stats = map.stats();
        // Two distinct attestation data entries (one for index=0, one for index=1)
        assert_eq!(stats.num_attestation_data, 2);
        assert_eq!(stats.num_attestations, 2);
    }

    /// Attestations with the same index are grouped together and can aggregate.
    #[test]
    fn gloas_same_index_attestations_aggregate() {
        let mut map = AttestationMap::<E>::default();

        let mut att_a = make_electra_attestation(10, 1, 0);
        set_attestation_bit(&mut att_a, 0);
        map.insert(att_a, vec![100]);

        let mut att_b = make_electra_attestation(10, 1, 0);
        set_attestation_bit(&mut att_b, 1);
        map.insert(att_b, vec![101]);

        let stats = map.stats();
        // Same attestation data (same index=1), should have aggregated
        assert_eq!(stats.num_attestation_data, 1);
        assert_eq!(stats.num_attestations, 1);
    }

    /// Index=0 and index=1 attestations from different committees for the same slot
    /// are stored independently and aggregate_across_committees only merges within the
    /// same index group.
    #[test]
    fn gloas_cross_committee_aggregation_respects_index() {
        let mut map = AttestationMap::<E>::default();
        let checkpoint = make_checkpoint();

        // Committee 0, index=0 (payload absent)
        let mut att_c0_i0 = make_electra_attestation(10, 0, 0);
        set_attestation_bit(&mut att_c0_i0, 0);
        map.insert(att_c0_i0, vec![100]);

        // Committee 1, index=0 (payload absent)
        let mut att_c1_i0 = make_electra_attestation(10, 0, 1);
        set_attestation_bit(&mut att_c1_i0, 0);
        map.insert(att_c1_i0, vec![200]);

        // Committee 0, index=1 (payload present)
        let mut att_c0_i1 = make_electra_attestation(10, 1, 0);
        set_attestation_bit(&mut att_c0_i1, 0);
        map.insert(att_c0_i1, vec![100]);

        // Before aggregation: 3 attestations, 2 data keys (index=0 and index=1)
        let stats = map.stats();
        assert_eq!(stats.num_attestation_data, 2);
        // index=0 has 2 attestations (different committees), index=1 has 1
        assert_eq!(stats.num_attestations, 3);

        // Aggregate across committees for this checkpoint
        map.aggregate_across_committees(checkpoint);

        let stats = map.stats();
        // After aggregation: index=0 committees 0+1 merged into 1 aggregate,
        // index=1 has only committee 0 (no merging needed)
        assert_eq!(stats.num_attestation_data, 2);
        assert_eq!(stats.num_attestations, 2);
    }

    /// The CompactAttestationData hash/eq distinguishes index=0 from index=1.
    #[test]
    fn compact_attestation_data_index_hash_distinct() {
        let data0 = CompactAttestationData {
            slot: Slot::new(10),
            index: 0,
            beacon_block_root: Hash256::repeat_byte(0x01),
            target_root: Hash256::repeat_byte(0x02),
        };
        let data1 = CompactAttestationData {
            slot: Slot::new(10),
            index: 1,
            beacon_block_root: Hash256::repeat_byte(0x01),
            target_root: Hash256::repeat_byte(0x02),
        };
        assert_ne!(data0, data1);

        let mut set = std::collections::HashSet::new();
        set.insert(data0);
        set.insert(data1);
        assert_eq!(set.len(), 2);
    }

    /// The clone_as_attestation preserves index value for both 0 and 1.
    #[test]
    fn compact_attestation_ref_preserves_index() {
        for index in [0u64, 1u64] {
            let mut att = make_electra_attestation(10, index, 0);
            set_attestation_bit(&mut att, 0);
            let split = SplitAttestation::<E>::new(att, vec![100]);
            let att_ref = split.as_ref();
            let reconstructed = att_ref.clone_as_attestation();
            assert_eq!(reconstructed.data().index, index);
        }
    }

    #[test]
    fn bitlist_extend_byte_aligned() {
        // 8-bit list1 (byte-aligned) + 8-bit list2
        type B = typenum::U64;
        let mut l1 = BitList::<B>::with_capacity(8).unwrap();
        l1.set(0, true).unwrap();
        l1.set(7, true).unwrap();
        let mut l2 = BitList::<B>::with_capacity(8).unwrap();
        l2.set(3, true).unwrap();

        let result = bitlist_extend(&l1, &l2).unwrap();
        assert_eq!(result.len(), 16);
        assert!(result.get(0).unwrap()); // from l1
        assert!(result.get(7).unwrap()); // from l1
        assert!(result.get(11).unwrap()); // from l2 (bit 3 + offset 8)
        assert!(!result.get(1).unwrap());
        assert!(!result.get(8).unwrap());
    }

    #[test]
    fn bitlist_extend_non_aligned() {
        // 5-bit list1 (not byte-aligned) + 7-bit list2
        type B = typenum::U64;
        let mut l1 = BitList::<B>::with_capacity(5).unwrap();
        l1.set(0, true).unwrap();
        l1.set(4, true).unwrap();
        let mut l2 = BitList::<B>::with_capacity(7).unwrap();
        l2.set(0, true).unwrap();
        l2.set(6, true).unwrap();

        let result = bitlist_extend(&l1, &l2).unwrap();
        assert_eq!(result.len(), 12);
        assert!(result.get(0).unwrap()); // l1 bit 0
        assert!(result.get(4).unwrap()); // l1 bit 4
        assert!(result.get(5).unwrap()); // l2 bit 0 at offset 5
        assert!(result.get(11).unwrap()); // l2 bit 6 at offset 5
        assert!(!result.get(1).unwrap());
        assert!(!result.get(6).unwrap());
    }

    #[test]
    fn bitlist_extend_empty_lists() {
        type B = typenum::U64;
        let l1 = BitList::<B>::with_capacity(0).unwrap();
        let mut l2 = BitList::<B>::with_capacity(3).unwrap();
        l2.set(1, true).unwrap();

        // Empty + non-empty
        let result = bitlist_extend(&l1, &l2).unwrap();
        assert_eq!(result.len(), 3);
        assert!(result.get(1).unwrap());

        // Non-empty + empty
        let result = bitlist_extend(&l2, &l1).unwrap();
        assert_eq!(result.len(), 3);
        assert!(result.get(1).unwrap());

        // Empty + empty
        let result = bitlist_extend(&l1, &l1).unwrap();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn bitlist_extend_all_bits_set() {
        type B = typenum::U64;
        let mut l1 = BitList::<B>::with_capacity(6).unwrap();
        for i in 0..6 {
            l1.set(i, true).unwrap();
        }
        let mut l2 = BitList::<B>::with_capacity(5).unwrap();
        for i in 0..5 {
            l2.set(i, true).unwrap();
        }

        let result = bitlist_extend(&l1, &l2).unwrap();
        assert_eq!(result.len(), 11);
        for i in 0..11 {
            assert!(result.get(i).unwrap(), "bit {} should be set", i);
        }
    }

    #[test]
    fn bitlist_extend_exceeds_max() {
        type B = typenum::U8;
        let l1 = BitList::<B>::with_capacity(5).unwrap();
        let l2 = BitList::<B>::with_capacity(5).unwrap();
        // 5 + 5 = 10 > 8, should return None
        assert!(bitlist_extend(&l1, &l2).is_none());
    }
}
