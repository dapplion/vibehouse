use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use crate::GossipTopic;
use crate::types::GossipKind;

use tokio_util::time::delay_queue::{DelayQueue, Key};

/// Store of gossip messages that we failed to publish and will try again later. By default, all
/// messages are ignored. This behaviour can be changed using `GossipCacheBuilder::default_timeout`
/// to apply the same delay to every kind. Individual timeouts for specific kinds can be set and
/// will overwrite the default_timeout if present.
pub struct GossipCache {
    /// Expire timeouts for each topic-msg pair.
    expirations: DelayQueue<(GossipTopic, Vec<u8>)>,
    /// Messages cached for each topic.
    topic_msgs: HashMap<GossipTopic, HashMap<Vec<u8>, Key>>,
    /// Timeout for blocks.
    beacon_block: Option<Duration>,
    /// Timeout for blobs.
    blob_sidecar: Option<Duration>,
    /// Timeout for data columns.
    data_column_sidecar: Option<Duration>,
    /// Timeout for aggregate attestations.
    aggregates: Option<Duration>,
    /// Timeout for attestations.
    attestation: Option<Duration>,
    /// Timeout for voluntary exits.
    voluntary_exit: Option<Duration>,
    /// Timeout for proposer slashings.
    proposer_slashing: Option<Duration>,
    /// Timeout for attester slashings.
    attester_slashing: Option<Duration>,
    /// Timeout for aggregated sync committee signatures.
    signed_contribution_and_proof: Option<Duration>,
    /// Timeout for sync committee messages.
    sync_committee_message: Option<Duration>,
    /// Timeout for signed BLS to execution changes.
    bls_to_execution_change: Option<Duration>,
    /// Timeout for light client finality updates.
    light_client_finality_update: Option<Duration>,
    /// Timeout for light client optimistic updates.
    light_client_optimistic_update: Option<Duration>,
}

#[derive(Default)]
pub struct GossipCacheBuilder {
    default_timeout: Option<Duration>,
    /// Timeout for blocks.
    beacon_block: Option<Duration>,
    /// Timeout for blob sidecars.
    blob_sidecar: Option<Duration>,
    /// Timeout for data column sidecars.
    data_column_sidecar: Option<Duration>,
    /// Timeout for aggregate attestations.
    aggregates: Option<Duration>,
    /// Timeout for attestations.
    attestation: Option<Duration>,
    /// Timeout for voluntary exits.
    voluntary_exit: Option<Duration>,
    /// Timeout for proposer slashings.
    proposer_slashing: Option<Duration>,
    /// Timeout for attester slashings.
    attester_slashing: Option<Duration>,
    /// Timeout for aggregated sync committee signatures.
    signed_contribution_and_proof: Option<Duration>,
    /// Timeout for sync committee messages.
    sync_committee_message: Option<Duration>,
    /// Timeout for signed BLS to execution changes.
    bls_to_execution_change: Option<Duration>,
    /// Timeout for light client finality updates.
    light_client_finality_update: Option<Duration>,
    /// Timeout for light client optimistic updates.
    light_client_optimistic_update: Option<Duration>,
}

#[allow(dead_code)]
impl GossipCacheBuilder {
    /// By default, all timeouts all disabled. Setting a default timeout will enable all timeout
    /// that are not already set.
    pub fn default_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = Some(timeout);
        self
    }
    /// Timeout for blocks.
    pub fn beacon_block_timeout(mut self, timeout: Duration) -> Self {
        self.beacon_block = Some(timeout);
        self
    }

    /// Timeout for aggregate attestations.
    pub fn aggregates_timeout(mut self, timeout: Duration) -> Self {
        self.aggregates = Some(timeout);
        self
    }

    /// Timeout for attestations.
    pub fn attestation_timeout(mut self, timeout: Duration) -> Self {
        self.attestation = Some(timeout);
        self
    }

    /// Timeout for voluntary exits.
    pub fn voluntary_exit_timeout(mut self, timeout: Duration) -> Self {
        self.voluntary_exit = Some(timeout);
        self
    }

    /// Timeout for proposer slashings.
    pub fn proposer_slashing_timeout(mut self, timeout: Duration) -> Self {
        self.proposer_slashing = Some(timeout);
        self
    }

    /// Timeout for attester slashings.
    pub fn attester_slashing_timeout(mut self, timeout: Duration) -> Self {
        self.attester_slashing = Some(timeout);
        self
    }

    /// Timeout for aggregated sync committee signatures.
    pub fn signed_contribution_and_proof_timeout(mut self, timeout: Duration) -> Self {
        self.signed_contribution_and_proof = Some(timeout);
        self
    }

    /// Timeout for sync committee messages.
    pub fn sync_committee_message_timeout(mut self, timeout: Duration) -> Self {
        self.sync_committee_message = Some(timeout);
        self
    }

    /// Timeout for BLS to execution change messages.
    pub fn bls_to_execution_change_timeout(mut self, timeout: Duration) -> Self {
        self.bls_to_execution_change = Some(timeout);
        self
    }

    /// Timeout for light client finality update messages.
    pub fn light_client_finality_update_timeout(mut self, timeout: Duration) -> Self {
        self.light_client_finality_update = Some(timeout);
        self
    }

    /// Timeout for light client optimistic update messages.
    pub fn light_client_optimistic_update_timeout(mut self, timeout: Duration) -> Self {
        self.light_client_optimistic_update = Some(timeout);
        self
    }

    pub fn build(self) -> GossipCache {
        let GossipCacheBuilder {
            default_timeout,
            beacon_block,
            blob_sidecar,
            data_column_sidecar,
            aggregates,
            attestation,
            voluntary_exit,
            proposer_slashing,
            attester_slashing,
            signed_contribution_and_proof,
            sync_committee_message,
            bls_to_execution_change,
            light_client_finality_update,
            light_client_optimistic_update,
        } = self;
        GossipCache {
            expirations: DelayQueue::default(),
            topic_msgs: HashMap::default(),
            beacon_block: beacon_block.or(default_timeout),
            blob_sidecar: blob_sidecar.or(default_timeout),
            data_column_sidecar: data_column_sidecar.or(default_timeout),
            aggregates: aggregates.or(default_timeout),
            attestation: attestation.or(default_timeout),
            voluntary_exit: voluntary_exit.or(default_timeout),
            proposer_slashing: proposer_slashing.or(default_timeout),
            attester_slashing: attester_slashing.or(default_timeout),
            signed_contribution_and_proof: signed_contribution_and_proof.or(default_timeout),
            sync_committee_message: sync_committee_message.or(default_timeout),
            bls_to_execution_change: bls_to_execution_change.or(default_timeout),
            light_client_finality_update: light_client_finality_update.or(default_timeout),
            light_client_optimistic_update: light_client_optimistic_update.or(default_timeout),
        }
    }
}

impl GossipCache {
    /// Get a builder of a `GossipCache`. Topic kinds for which no timeout is defined will be
    /// ignored if added in `insert`.
    pub fn builder() -> GossipCacheBuilder {
        GossipCacheBuilder::default()
    }

    // Insert a message to be sent later.
    pub fn insert(&mut self, topic: GossipTopic, data: Vec<u8>) {
        let expire_timeout = match topic.kind() {
            GossipKind::BeaconBlock => self.beacon_block,
            GossipKind::BlobSidecar(_) => self.blob_sidecar,
            GossipKind::DataColumnSidecar(_) => self.data_column_sidecar,
            GossipKind::BeaconAggregateAndProof => self.aggregates,
            GossipKind::Attestation(_) => self.attestation,
            GossipKind::VoluntaryExit => self.voluntary_exit,
            GossipKind::ProposerSlashing => self.proposer_slashing,
            GossipKind::AttesterSlashing => self.attester_slashing,
            GossipKind::SignedContributionAndProof => self.signed_contribution_and_proof,
            GossipKind::SyncCommitteeMessage(_) => self.sync_committee_message,
            GossipKind::BlsToExecutionChange => self.bls_to_execution_change,
            GossipKind::LightClientFinalityUpdate => self.light_client_finality_update,
            GossipKind::LightClientOptimisticUpdate => self.light_client_optimistic_update,
            GossipKind::ExecutionBid => None, // gloas ePBS: bids are time-sensitive, no caching
            GossipKind::ExecutionPayload => None, // gloas ePBS: payloads are time-sensitive, no caching
            GossipKind::PayloadAttestation => None, // gloas ePBS: attestations are time-sensitive, no caching
            GossipKind::ProposerPreferences => None, // gloas ePBS: preferences are time-sensitive, no caching
            GossipKind::ExecutionProof(_) => None,   // proofs are time-sensitive, no caching
        };
        let Some(expire_timeout) = expire_timeout else {
            return;
        };
        match self
            .topic_msgs
            .entry(topic.clone())
            .or_default()
            .entry(data.clone())
        {
            Entry::Occupied(key) => self.expirations.reset(key.get(), expire_timeout),
            Entry::Vacant(entry) => {
                let key = self.expirations.insert((topic, data), expire_timeout);
                entry.insert(key);
            }
        }
    }

    // Get the registered messages for this topic.
    pub fn retrieve(&mut self, topic: &GossipTopic) -> Option<impl Iterator<Item = Vec<u8>> + '_> {
        if let Some(msgs) = self.topic_msgs.remove(topic) {
            for (_, key) in msgs.iter() {
                self.expirations.remove(key);
            }
            Some(msgs.into_keys())
        } else {
            None
        }
    }
}

impl futures::stream::Stream for GossipCache {
    type Item = Result<GossipTopic, String>; // We don't care to retrieve the expired data.

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.expirations.poll_expired(cx) {
            Poll::Ready(Some(expired)) => {
                let expected_key = expired.key();
                let (topic, data) = expired.into_inner();
                let topic_msg = self.topic_msgs.get_mut(&topic);
                debug_assert!(
                    topic_msg.is_some(),
                    "Topic for registered message is not present."
                );
                if let Some(msgs) = topic_msg {
                    let key = msgs.remove(&data);
                    debug_assert_eq!(key, Some(expected_key));
                    if msgs.is_empty() {
                        // no more messages for this topic.
                        self.topic_msgs.remove(&topic);
                    }
                }
                Poll::Ready(Some(Ok(topic)))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream::StreamExt;

    #[tokio::test]
    async fn test_stream() {
        let mut cache = GossipCache::builder()
            .default_timeout(Duration::from_millis(300))
            .build();
        let test_topic = GossipTopic::new(
            GossipKind::Attestation(1u64.into()),
            crate::types::GossipEncoding::SSZSnappy,
            [0u8; 4],
        );
        cache.insert(test_topic, vec![]);
        tokio::time::sleep(Duration::from_millis(300)).await;
        while cache.next().await.is_some() {}
        assert!(cache.expirations.is_empty());
        assert!(cache.topic_msgs.is_empty());
    }

    fn make_topic(kind: GossipKind) -> GossipTopic {
        GossipTopic::new(kind, crate::types::GossipEncoding::SSZSnappy, [0u8; 4])
    }

    #[test]
    fn builder_default_timeout_applies_to_all() {
        let cache = GossipCache::builder()
            .default_timeout(Duration::from_secs(5))
            .build();
        assert_eq!(cache.beacon_block, Some(Duration::from_secs(5)));
        assert_eq!(cache.aggregates, Some(Duration::from_secs(5)));
        assert_eq!(cache.attestation, Some(Duration::from_secs(5)));
        assert_eq!(cache.voluntary_exit, Some(Duration::from_secs(5)));
        assert_eq!(cache.proposer_slashing, Some(Duration::from_secs(5)));
        assert_eq!(cache.attester_slashing, Some(Duration::from_secs(5)));
        assert_eq!(
            cache.signed_contribution_and_proof,
            Some(Duration::from_secs(5))
        );
        assert_eq!(cache.sync_committee_message, Some(Duration::from_secs(5)));
        assert_eq!(cache.bls_to_execution_change, Some(Duration::from_secs(5)));
    }

    #[test]
    fn builder_specific_timeout_overrides_default() {
        let cache = GossipCache::builder()
            .default_timeout(Duration::from_secs(5))
            .beacon_block_timeout(Duration::from_secs(10))
            .build();
        assert_eq!(cache.beacon_block, Some(Duration::from_secs(10)));
        assert_eq!(cache.aggregates, Some(Duration::from_secs(5)));
    }

    #[test]
    fn builder_no_default_leaves_all_none() {
        let cache = GossipCache::builder().build();
        assert_eq!(cache.beacon_block, None);
        assert_eq!(cache.aggregates, None);
        assert_eq!(cache.attestation, None);
    }

    #[test]
    fn insert_ignored_when_no_timeout() {
        let mut cache = GossipCache::builder().build();
        let topic = make_topic(GossipKind::BeaconBlock);
        cache.insert(topic.clone(), vec![1, 2, 3]);
        // No timeout for beacon_block → message should be dropped
        assert!(cache.topic_msgs.is_empty());
        assert!(cache.expirations.is_empty());
    }

    #[tokio::test]
    async fn insert_stores_when_timeout_set() {
        let mut cache = GossipCache::builder()
            .beacon_block_timeout(Duration::from_secs(10))
            .build();
        let topic = make_topic(GossipKind::BeaconBlock);
        cache.insert(topic.clone(), vec![1, 2, 3]);
        assert_eq!(cache.topic_msgs.len(), 1);
        assert!(cache.topic_msgs.contains_key(&topic));
    }

    #[tokio::test]
    async fn retrieve_returns_cached_messages() {
        let mut cache = GossipCache::builder()
            .beacon_block_timeout(Duration::from_secs(10))
            .build();
        let topic = make_topic(GossipKind::BeaconBlock);
        cache.insert(topic.clone(), vec![1, 2, 3]);
        cache.insert(topic.clone(), vec![4, 5, 6]);

        let msgs: Vec<Vec<u8>> = cache.retrieve(&topic).unwrap().collect();
        assert_eq!(msgs.len(), 2);
        assert!(msgs.contains(&vec![1, 2, 3]));
        assert!(msgs.contains(&vec![4, 5, 6]));

        // After retrieval, topic should be removed
        assert!(cache.topic_msgs.is_empty());
        assert!(cache.expirations.is_empty());
    }

    #[test]
    fn retrieve_returns_none_for_unknown_topic() {
        let mut cache = GossipCache::builder().build();
        let topic = make_topic(GossipKind::BeaconBlock);
        assert!(cache.retrieve(&topic).is_none());
    }

    #[tokio::test]
    async fn duplicate_insert_resets_timer() {
        let mut cache = GossipCache::builder()
            .attestation_timeout(Duration::from_secs(10))
            .build();
        let topic = make_topic(GossipKind::Attestation(0u64.into()));
        let data = vec![1, 2, 3];

        cache.insert(topic.clone(), data.clone());
        // Re-inserting same data for same topic should reset the timer, not add duplicate
        cache.insert(topic.clone(), data.clone());

        let msgs: Vec<Vec<u8>> = cache.retrieve(&topic).unwrap().collect();
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn epbs_gossip_kinds_are_not_cached() {
        let mut cache = GossipCache::builder()
            .default_timeout(Duration::from_secs(10))
            .build();

        // ePBS types should not be cached
        let bid_topic = make_topic(GossipKind::ExecutionBid);
        cache.insert(bid_topic.clone(), vec![1]);
        assert!(cache.topic_msgs.is_empty());

        let payload_topic = make_topic(GossipKind::ExecutionPayload);
        cache.insert(payload_topic.clone(), vec![2]);
        assert!(cache.topic_msgs.is_empty());

        let pa_topic = make_topic(GossipKind::PayloadAttestation);
        cache.insert(pa_topic.clone(), vec![3]);
        assert!(cache.topic_msgs.is_empty());

        let pref_topic = make_topic(GossipKind::ProposerPreferences);
        cache.insert(pref_topic.clone(), vec![4]);
        assert!(cache.topic_msgs.is_empty());
    }

    #[tokio::test]
    async fn different_topics_stored_independently() {
        let mut cache = GossipCache::builder()
            .beacon_block_timeout(Duration::from_secs(10))
            .voluntary_exit_timeout(Duration::from_secs(10))
            .build();

        let block_topic = make_topic(GossipKind::BeaconBlock);
        let exit_topic = make_topic(GossipKind::VoluntaryExit);

        cache.insert(block_topic.clone(), vec![1]);
        cache.insert(exit_topic.clone(), vec![2]);

        assert_eq!(cache.topic_msgs.len(), 2);

        let block_msgs: Vec<Vec<u8>> = cache.retrieve(&block_topic).unwrap().collect();
        assert_eq!(block_msgs, vec![vec![1]]);

        // Exit topic should still be present
        assert_eq!(cache.topic_msgs.len(), 1);
        let exit_msgs: Vec<Vec<u8>> = cache.retrieve(&exit_topic).unwrap().collect();
        assert_eq!(exit_msgs, vec![vec![2]]);
    }
}
