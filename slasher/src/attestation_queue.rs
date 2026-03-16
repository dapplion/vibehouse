use crate::{AttesterRecord, Config, IndexedAttesterRecord};
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::sync::{Arc, Weak};
use types::{EthSpec, Hash256, IndexedAttestation};

/// Staging area for attestations received from the network.
///
/// Attestations are not grouped by validator index at this stage so that they can be easily
/// filtered for timeliness.
#[derive(Debug, Default)]
pub struct AttestationQueue<E: EthSpec> {
    pub queue: Mutex<SimpleBatch<E>>,
}

pub type SimpleBatch<E> = Vec<Arc<IndexedAttesterRecord<E>>>;

/// Attestations dequeued from the queue and in preparation for processing.
///
/// This struct is responsible for mapping validator indices to attestations and performing
/// de-duplication to remove redundant attestations.
#[derive(Debug, Default)]
pub struct AttestationBatch<E: EthSpec> {
    /// Map from (`validator_index`, `attestation_data_hash`) to indexed attester record.
    ///
    /// This mapping is used for de-duplication.
    pub attesters: BTreeMap<(u64, Hash256), Arc<IndexedAttesterRecord<E>>>,

    /// Vec of all unique indexed attester records.
    ///
    /// The weak references account for the fact that some records might prove useless after
    /// de-duplication.
    pub attestations: Vec<Weak<IndexedAttesterRecord<E>>>,
}

/// Attestations grouped by validator index range.
#[derive(Debug)]
pub struct GroupedAttestations<E: EthSpec> {
    pub subqueues: Vec<SimpleBatch<E>>,
}

impl<E: EthSpec> AttestationBatch<E> {
    /// Add an attestation to the queue.
    pub fn queue(&mut self, indexed_record: Arc<IndexedAttesterRecord<E>>) {
        self.attestations.push(Arc::downgrade(&indexed_record));

        let attestation_data_hash = indexed_record.record.attestation_data_hash;

        for &validator_index in indexed_record.indexed.attesting_indices_iter() {
            self.attesters
                .entry((validator_index, attestation_data_hash))
                .and_modify(|existing_entry| {
                    // If the new record is for the same attestation data but with more bits set
                    // then replace the existing record so that we might avoid storing the
                    // smaller indexed attestation. Single-bit attestations will usually be removed
                    // completely by this process, and aggregates will only be retained if they
                    // are not redundant with respect to a larger aggregate seen in the same batch.
                    if existing_entry.indexed.attesting_indices_len()
                        < indexed_record.indexed.attesting_indices_len()
                    {
                        *existing_entry = indexed_record.clone();
                    }
                })
                .or_insert_with(|| indexed_record.clone());
        }
    }

    /// Group the attestations by validator chunk index.
    pub fn group_by_validator_chunk_index(self, config: &Config) -> GroupedAttestations<E> {
        let mut grouped_attestations = GroupedAttestations { subqueues: vec![] };

        for ((validator_index, _), indexed_record) in self.attesters {
            let subqueue_id = config.validator_chunk_index(validator_index);

            if subqueue_id >= grouped_attestations.subqueues.len() {
                grouped_attestations
                    .subqueues
                    .resize_with(subqueue_id + 1, SimpleBatch::default);
            }

            grouped_attestations.subqueues[subqueue_id].push(indexed_record);
        }

        grouped_attestations
    }
}

impl<E: EthSpec> AttestationQueue<E> {
    pub fn queue(&self, attestation: IndexedAttestation<E>) {
        let attester_record = AttesterRecord::from(attestation.clone());
        let indexed_record = IndexedAttesterRecord::new(attestation, attester_record);
        self.queue.lock().push(indexed_record);
    }

    pub fn dequeue(&self) -> SimpleBatch<E> {
        std::mem::take(&mut self.queue.lock())
    }

    pub fn requeue(&self, batch: SimpleBatch<E>) {
        self.queue.lock().extend(batch);
    }

    pub fn len(&self) -> usize {
        self.queue.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{E, indexed_att_electra};
    use std::path::PathBuf;

    fn test_config() -> Config {
        Config {
            database_path: PathBuf::from("/tmp/slasher-test"),
            chunk_size: 4,
            validator_chunk_size: 256,
            history_length: 16,
            ..Config::new(PathBuf::from("/tmp/slasher-test"))
        }
    }

    // ── AttestationQueue ───────────────────────────────────────

    #[test]
    fn attestation_queue_empty() {
        let q = AttestationQueue::<E>::default();
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn attestation_queue_enqueue_dequeue() {
        let q = AttestationQueue::<E>::default();
        let att = indexed_att_electra(vec![1], 0, 1, 0);
        q.queue(att);
        assert_eq!(q.len(), 1);

        let batch = q.dequeue();
        assert_eq!(batch.len(), 1);
        assert!(q.is_empty());
    }

    #[test]
    fn attestation_queue_multiple_enqueue() {
        let q = AttestationQueue::<E>::default();
        q.queue(indexed_att_electra(vec![1], 0, 1, 0));
        q.queue(indexed_att_electra(vec![2], 0, 2, 0));
        q.queue(indexed_att_electra(vec![3], 0, 3, 0));
        assert_eq!(q.len(), 3);
    }

    #[test]
    fn attestation_queue_dequeue_empty() {
        let q = AttestationQueue::<E>::default();
        let batch = q.dequeue();
        assert!(batch.is_empty());
    }

    #[test]
    fn attestation_queue_requeue() {
        let q = AttestationQueue::<E>::default();
        q.queue(indexed_att_electra(vec![1], 0, 1, 0));
        q.queue(indexed_att_electra(vec![2], 0, 2, 0));

        let batch = q.dequeue();
        assert_eq!(batch.len(), 2);
        assert!(q.is_empty());

        q.requeue(batch);
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn attestation_queue_enqueue_after_dequeue() {
        let q = AttestationQueue::<E>::default();
        q.queue(indexed_att_electra(vec![1], 0, 1, 0));
        let _ = q.dequeue();

        q.queue(indexed_att_electra(vec![2], 0, 2, 0));
        assert_eq!(q.len(), 1);
    }

    // ── AttestationBatch ───────────────────────────────────────

    #[test]
    fn batch_queue_single() {
        let att = indexed_att_electra(vec![1], 0, 1, 0);
        let record = AttesterRecord::from(att.clone());
        let indexed_record = IndexedAttesterRecord::new(att, record);

        let mut batch = AttestationBatch::<E>::default();
        batch.queue(indexed_record);

        assert_eq!(batch.attestations.len(), 1);
        assert_eq!(batch.attesters.len(), 1);
    }

    #[test]
    fn batch_queue_multiple_validators_same_data() {
        let att = indexed_att_electra(vec![1, 2, 3], 0, 1, 0);
        let record = AttesterRecord::from(att.clone());
        let indexed_record = IndexedAttesterRecord::new(att, record);

        let mut batch = AttestationBatch::<E>::default();
        batch.queue(indexed_record);

        // 3 entries in attesters (one per validator)
        assert_eq!(batch.attesters.len(), 3);
        assert_eq!(batch.attestations.len(), 1);
    }

    #[test]
    fn batch_queue_different_data() {
        let att1 = indexed_att_electra(vec![1], 0, 1, 0);
        let record1 = AttesterRecord::from(att1.clone());
        let att2 = indexed_att_electra(vec![2], 0, 2, 0);
        let record2 = AttesterRecord::from(att2.clone());

        let mut batch = AttestationBatch::<E>::default();
        batch.queue(IndexedAttesterRecord::new(att1, record1));
        batch.queue(IndexedAttesterRecord::new(att2, record2));

        assert_eq!(batch.attestations.len(), 2);
        assert_eq!(batch.attesters.len(), 2);
    }

    #[test]
    fn batch_dedup_prefers_larger_aggregate() {
        let att_small = indexed_att_electra(vec![1], 0, 1, 0);
        let record_small = AttesterRecord::from(att_small.clone());

        let att_large = indexed_att_electra(vec![1, 2, 3], 0, 1, 0);
        let record_large = AttesterRecord::from(att_large.clone());

        let mut batch = AttestationBatch::<E>::default();
        batch.queue(IndexedAttesterRecord::new(att_small, record_small));
        batch.queue(IndexedAttesterRecord::new(att_large, record_large));

        // Validator 1 should map to the larger attestation
        let data_hash = batch.attesters.keys().find(|(v, _)| *v == 1).unwrap().1;
        let record = batch.attesters.get(&(1, data_hash)).unwrap();
        assert_eq!(record.indexed.attesting_indices_len(), 3);
    }

    #[test]
    fn batch_dedup_keeps_larger_when_queued_first() {
        let att_large = indexed_att_electra(vec![1, 2, 3], 0, 1, 0);
        let record_large = AttesterRecord::from(att_large.clone());

        let att_small = indexed_att_electra(vec![1], 0, 1, 0);
        let record_small = AttesterRecord::from(att_small.clone());

        let mut batch = AttestationBatch::<E>::default();
        batch.queue(IndexedAttesterRecord::new(att_large, record_large));
        batch.queue(IndexedAttesterRecord::new(att_small, record_small));

        // Validator 1 should still map to the larger attestation
        let data_hash = batch.attesters.keys().find(|(v, _)| *v == 1).unwrap().1;
        let record = batch.attesters.get(&(1, data_hash)).unwrap();
        assert_eq!(record.indexed.attesting_indices_len(), 3);
    }

    // ── group_by_validator_chunk_index ──────────────────────────

    #[test]
    fn group_by_validator_chunk_single_chunk() {
        let config = test_config();
        let att = indexed_att_electra(vec![1, 2, 3], 0, 1, 0);
        let record = AttesterRecord::from(att.clone());

        let mut batch = AttestationBatch::<E>::default();
        batch.queue(IndexedAttesterRecord::new(att, record));

        let grouped = batch.group_by_validator_chunk_index(&config);
        assert_eq!(grouped.subqueues.len(), 1);
        assert_eq!(grouped.subqueues[0].len(), 3);
    }

    #[test]
    fn group_by_validator_chunk_multiple_chunks() {
        let config = test_config();
        let att = indexed_att_electra(vec![0, 256, 512], 0, 1, 0);
        let record = AttesterRecord::from(att.clone());

        let mut batch = AttestationBatch::<E>::default();
        batch.queue(IndexedAttesterRecord::new(att, record));

        let grouped = batch.group_by_validator_chunk_index(&config);
        assert_eq!(grouped.subqueues.len(), 3);
        assert_eq!(grouped.subqueues[0].len(), 1);
        assert_eq!(grouped.subqueues[1].len(), 1);
        assert_eq!(grouped.subqueues[2].len(), 1);
    }

    #[test]
    fn group_by_validator_chunk_empty_batch() {
        let config = test_config();
        let batch = AttestationBatch::<E>::default();
        let grouped = batch.group_by_validator_chunk_index(&config);
        assert!(grouped.subqueues.is_empty());
    }
}
