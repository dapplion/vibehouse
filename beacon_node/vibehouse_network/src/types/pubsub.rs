//! Handles the encoding and decoding of pubsub messages.

use crate::TopicHash;
use crate::types::{GossipEncoding, GossipKind, GossipTopic};
use snap::raw::{Decoder, Encoder, decompress_len};
use ssz::{Decode, Encode};
use std::io::{Error, ErrorKind};
use std::sync::Arc;
use types::{
    AttesterSlashing, AttesterSlashingBase, AttesterSlashingElectra, BlobSidecar,
    DataColumnSidecar, DataColumnSubnetId, EthSpec, ExecutionProof, ExecutionProofSubnetId,
    ForkContext, ForkName, LightClientFinalityUpdate, LightClientOptimisticUpdate,
    PayloadAttestationMessage, ProposerSlashing, SignedAggregateAndProof,
    SignedAggregateAndProofBase, SignedAggregateAndProofElectra, SignedBeaconBlock,
    SignedBeaconBlockAltair, SignedBeaconBlockBase, SignedBeaconBlockBellatrix,
    SignedBeaconBlockCapella, SignedBeaconBlockDeneb, SignedBeaconBlockElectra,
    SignedBeaconBlockFulu, SignedBeaconBlockGloas, SignedBlsToExecutionChange,
    SignedContributionAndProof, SignedExecutionPayloadBid, SignedExecutionPayloadEnvelope,
    SignedProposerPreferences, SignedVoluntaryExit, SingleAttestation, SubnetId,
    SyncCommitteeMessage, SyncSubnetId,
};

#[derive(Debug, Clone, PartialEq)]
pub enum PubsubMessage<E: EthSpec> {
    /// Gossipsub message providing notification of a new block.
    BeaconBlock(Arc<SignedBeaconBlock<E>>),
    /// Gossipsub message providing notification of a [`BlobSidecar`] along with the subnet id where it was received.
    BlobSidecar(Box<(u64, Arc<BlobSidecar<E>>)>),
    /// Gossipsub message providing notification of a [`DataColumnSidecar`] along with the subnet id where it was received.
    DataColumnSidecar(Box<(DataColumnSubnetId, Arc<DataColumnSidecar<E>>)>),
    /// Gossipsub message providing notification of a Aggregate attestation and associated proof.
    AggregateAndProofAttestation(Box<SignedAggregateAndProof<E>>),
    /// Gossipsub message providing notification of a `SingleAttestation` with its subnet id.
    Attestation(Box<(SubnetId, SingleAttestation)>),
    /// Gossipsub message providing notification of a voluntary exit.
    VoluntaryExit(Box<SignedVoluntaryExit>),
    /// Gossipsub message providing notification of a new proposer slashing.
    ProposerSlashing(Box<ProposerSlashing>),
    /// Gossipsub message providing notification of a new attester slashing.
    AttesterSlashing(Box<AttesterSlashing<E>>),
    /// Gossipsub message providing notification of partially aggregated sync committee signatures.
    SignedContributionAndProof(Box<SignedContributionAndProof<E>>),
    /// Gossipsub message providing notification of unaggregated sync committee signatures with its subnet id.
    SyncCommitteeMessage(Box<(SyncSubnetId, SyncCommitteeMessage)>),
    /// Gossipsub message for BLS to execution change messages.
    BlsToExecutionChange(Box<SignedBlsToExecutionChange>),
    /// Gossipsub message providing notification of a light client finality update.
    LightClientFinalityUpdate(Box<LightClientFinalityUpdate<E>>),
    /// Gossipsub message providing notification of a light client optimistic update.
    LightClientOptimisticUpdate(Box<LightClientOptimisticUpdate<E>>),
    /// Gossipsub message providing notification of an execution payload bid (gloas ePBS).
    ExecutionBid(Box<SignedExecutionPayloadBid<E>>),
    /// Gossipsub message providing notification of an execution payload envelope reveal (gloas ePBS).
    ExecutionPayload(Box<SignedExecutionPayloadEnvelope<E>>),
    /// Gossipsub message providing notification of a payload attestation from PTC (gloas ePBS).
    /// Per spec, the gossip topic carries individual `PayloadAttestationMessage` (not aggregated).
    PayloadAttestation(Box<PayloadAttestationMessage>),
    /// Gossipsub message providing notification of proposer preferences (gloas ePBS).
    ProposerPreferences(Box<types::SignedProposerPreferences>),
    /// Gossipsub message providing a ZK execution proof on a particular proof subnet.
    ExecutionProof(Box<(ExecutionProofSubnetId, Arc<ExecutionProof>)>),
}

// Implements the `DataTransform` trait of gossipsub to employ snappy compression
pub struct SnappyTransform {
    /// Sets the maximum size we allow gossipsub messages to decompress to.
    max_uncompressed_len: usize,
    /// Sets the maximum size we allow for compressed gossipsub message data.
    max_compressed_len: usize,
}

impl SnappyTransform {
    pub fn new(max_uncompressed_len: usize, max_compressed_len: usize) -> Self {
        SnappyTransform {
            max_uncompressed_len,
            max_compressed_len,
        }
    }
}

impl gossipsub::DataTransform for SnappyTransform {
    // Provides the snappy decompression from RawGossipsubMessages
    fn inbound_transform(
        &self,
        raw_message: gossipsub::RawMessage,
    ) -> Result<gossipsub::Message, std::io::Error> {
        // first check the size of the compressed payload
        if raw_message.data.len() > self.max_compressed_len {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "ssz_snappy encoded data > max_compressed_len",
            ));
        }
        // check the length of the uncompressed bytes
        let len = decompress_len(&raw_message.data)?;
        if len > self.max_uncompressed_len {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "ssz_snappy decoded data > MAX_PAYLOAD_SIZE",
            ));
        }

        let mut decoder = Decoder::new();
        let decompressed_data = decoder.decompress_vec(&raw_message.data)?;

        // Build the GossipsubMessage struct
        Ok(gossipsub::Message {
            source: raw_message.source,
            data: decompressed_data,
            sequence_number: raw_message.sequence_number,
            topic: raw_message.topic,
        })
    }

    /// Provides the snappy compression logic to gossipsub.
    fn outbound_transform(
        &self,
        _topic: &TopicHash,
        data: Vec<u8>,
    ) -> Result<Vec<u8>, std::io::Error> {
        // Currently we are not employing topic-based compression. Everything is expected to be
        // snappy compressed.
        if data.len() > self.max_uncompressed_len {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "ssz_snappy Encoded data > MAX_PAYLOAD_SIZE",
            ));
        }
        let mut encoder = Encoder::new();
        encoder.compress_vec(&data).map_err(Into::into)
    }
}

impl<E: EthSpec> PubsubMessage<E> {
    /// Returns the topics that each pubsub message will be sent across, given a supported
    /// gossipsub encoding and fork version.
    pub fn topics(&self, encoding: GossipEncoding, fork_version: [u8; 4]) -> Vec<GossipTopic> {
        vec![GossipTopic::new(self.kind(), encoding, fork_version)]
    }

    /// Returns the kind of gossipsub topic associated with the message.
    pub fn kind(&self) -> GossipKind {
        match self {
            PubsubMessage::BeaconBlock(_) => GossipKind::BeaconBlock,
            PubsubMessage::BlobSidecar(blob_sidecar_data) => {
                GossipKind::BlobSidecar(blob_sidecar_data.0)
            }
            PubsubMessage::DataColumnSidecar(column_sidecar_data) => {
                GossipKind::DataColumnSidecar(column_sidecar_data.0)
            }
            PubsubMessage::AggregateAndProofAttestation(_) => GossipKind::BeaconAggregateAndProof,
            PubsubMessage::Attestation(attestation_data) => {
                GossipKind::Attestation(attestation_data.0)
            }
            PubsubMessage::VoluntaryExit(_) => GossipKind::VoluntaryExit,
            PubsubMessage::ProposerSlashing(_) => GossipKind::ProposerSlashing,
            PubsubMessage::AttesterSlashing(_) => GossipKind::AttesterSlashing,
            PubsubMessage::SignedContributionAndProof(_) => GossipKind::SignedContributionAndProof,
            PubsubMessage::SyncCommitteeMessage(data) => GossipKind::SyncCommitteeMessage(data.0),
            PubsubMessage::BlsToExecutionChange(_) => GossipKind::BlsToExecutionChange,
            PubsubMessage::LightClientFinalityUpdate(_) => GossipKind::LightClientFinalityUpdate,
            PubsubMessage::LightClientOptimisticUpdate(_) => {
                GossipKind::LightClientOptimisticUpdate
            }
            PubsubMessage::ExecutionBid(_) => GossipKind::ExecutionBid,
            PubsubMessage::ExecutionPayload(_) => GossipKind::ExecutionPayload,
            PubsubMessage::PayloadAttestation(_) => GossipKind::PayloadAttestation,
            PubsubMessage::ProposerPreferences(_) => GossipKind::ProposerPreferences,
            PubsubMessage::ExecutionProof(data) => GossipKind::ExecutionProof(data.0),
        }
    }

    /// This decodes `data` into a `PubsubMessage` given a topic.
    /* Note: This is assuming we are not hashing topics. If we choose to hash topics, these will
     * need to be modified.
     */
    pub fn decode(
        topic: &TopicHash,
        data: &[u8],
        fork_context: &ForkContext,
    ) -> Result<Self, String> {
        match GossipTopic::decode(topic.as_str()) {
            Err(_) => Err(format!("Unknown gossipsub topic: {:?}", topic)),
            Ok(gossip_topic) => {
                // All topics are currently expected to be compressed and decompressed with snappy.
                // This is done in the `SnappyTransform` struct.
                // Therefore compression has already been handled for us by the time we are
                // decoding the objects here.

                // the ssz decoders
                match gossip_topic.kind() {
                    GossipKind::BeaconAggregateAndProof => {
                        let signed_aggregate_and_proof = match fork_context
                            .get_fork_from_context_bytes(gossip_topic.fork_digest)
                        {
                            Some(&fork_name) => {
                                if fork_name.electra_enabled() {
                                    SignedAggregateAndProof::Electra(
                                        SignedAggregateAndProofElectra::from_ssz_bytes(data)
                                            .map_err(|e| format!("{:?}", e))?,
                                    )
                                } else {
                                    SignedAggregateAndProof::Base(
                                        SignedAggregateAndProofBase::from_ssz_bytes(data)
                                            .map_err(|e| format!("{:?}", e))?,
                                    )
                                }
                            }
                            None => {
                                return Err(format!(
                                    "Unknown gossipsub fork digest: {:?}",
                                    gossip_topic.fork_digest
                                ));
                            }
                        };
                        Ok(PubsubMessage::AggregateAndProofAttestation(Box::new(
                            signed_aggregate_and_proof,
                        )))
                    }
                    GossipKind::Attestation(subnet_id) => {
                        let attestation = SingleAttestation::from_ssz_bytes(data)
                            .map_err(|e| format!("{:?}", e))?;
                        Ok(PubsubMessage::Attestation(Box::new((
                            *subnet_id,
                            attestation,
                        ))))
                    }
                    GossipKind::BeaconBlock => {
                        let beacon_block = match fork_context
                            .get_fork_from_context_bytes(gossip_topic.fork_digest)
                        {
                            Some(ForkName::Base) => SignedBeaconBlock::<E>::Base(
                                SignedBeaconBlockBase::from_ssz_bytes(data)
                                    .map_err(|e| format!("{:?}", e))?,
                            ),
                            Some(ForkName::Altair) => SignedBeaconBlock::<E>::Altair(
                                SignedBeaconBlockAltair::from_ssz_bytes(data)
                                    .map_err(|e| format!("{:?}", e))?,
                            ),
                            Some(ForkName::Bellatrix) => SignedBeaconBlock::<E>::Bellatrix(
                                SignedBeaconBlockBellatrix::from_ssz_bytes(data)
                                    .map_err(|e| format!("{:?}", e))?,
                            ),
                            Some(ForkName::Capella) => SignedBeaconBlock::<E>::Capella(
                                SignedBeaconBlockCapella::from_ssz_bytes(data)
                                    .map_err(|e| format!("{:?}", e))?,
                            ),
                            Some(ForkName::Deneb) => SignedBeaconBlock::<E>::Deneb(
                                SignedBeaconBlockDeneb::from_ssz_bytes(data)
                                    .map_err(|e| format!("{:?}", e))?,
                            ),
                            Some(ForkName::Electra) => SignedBeaconBlock::<E>::Electra(
                                SignedBeaconBlockElectra::from_ssz_bytes(data)
                                    .map_err(|e| format!("{:?}", e))?,
                            ),
                            Some(ForkName::Fulu) => SignedBeaconBlock::<E>::Fulu(
                                SignedBeaconBlockFulu::from_ssz_bytes(data)
                                    .map_err(|e| format!("{:?}", e))?,
                            ),
                            Some(ForkName::Gloas) => SignedBeaconBlock::<E>::Gloas(
                                SignedBeaconBlockGloas::from_ssz_bytes(data)
                                    .map_err(|e| format!("{:?}", e))?,
                            ),
                            None => {
                                return Err(format!(
                                    "Unknown gossipsub fork digest: {:?}",
                                    gossip_topic.fork_digest
                                ));
                            }
                        };
                        Ok(PubsubMessage::BeaconBlock(Arc::new(beacon_block)))
                    }
                    GossipKind::BlobSidecar(blob_index) => {
                        if let Some(fork_name) =
                            fork_context.get_fork_from_context_bytes(gossip_topic.fork_digest)
                            && fork_name.deneb_enabled()
                        {
                            let blob_sidecar = Arc::new(
                                BlobSidecar::from_ssz_bytes(data)
                                    .map_err(|e| format!("{:?}", e))?,
                            );
                            return Ok(PubsubMessage::BlobSidecar(Box::new((
                                *blob_index,
                                blob_sidecar,
                            ))));
                        }

                        Err(format!(
                            "beacon_blobs_and_sidecar topic invalid for given fork digest {:?}",
                            gossip_topic.fork_digest
                        ))
                    }
                    GossipKind::DataColumnSidecar(subnet_id) => {
                        match fork_context.get_fork_from_context_bytes(gossip_topic.fork_digest) {
                            Some(fork) if fork.fulu_enabled() => {
                                let col_sidecar = Arc::new(
                                    DataColumnSidecar::any_from_ssz_bytes(data)
                                        .map_err(|e| format!("{:?}", e))?,
                                );
                                Ok(PubsubMessage::DataColumnSidecar(Box::new((
                                    *subnet_id,
                                    col_sidecar,
                                ))))
                            }
                            Some(_) | None => Err(format!(
                                "data_column_sidecar topic invalid for given fork digest {:?}",
                                gossip_topic.fork_digest
                            )),
                        }
                    }
                    GossipKind::VoluntaryExit => {
                        let voluntary_exit = SignedVoluntaryExit::from_ssz_bytes(data)
                            .map_err(|e| format!("{:?}", e))?;
                        Ok(PubsubMessage::VoluntaryExit(Box::new(voluntary_exit)))
                    }
                    GossipKind::ProposerSlashing => {
                        let proposer_slashing = ProposerSlashing::from_ssz_bytes(data)
                            .map_err(|e| format!("{:?}", e))?;
                        Ok(PubsubMessage::ProposerSlashing(Box::new(proposer_slashing)))
                    }
                    GossipKind::AttesterSlashing => {
                        let attester_slashing = match fork_context
                            .get_fork_from_context_bytes(gossip_topic.fork_digest)
                        {
                            Some(&fork_name) => {
                                if fork_name.electra_enabled() {
                                    AttesterSlashing::Electra(
                                        AttesterSlashingElectra::from_ssz_bytes(data)
                                            .map_err(|e| format!("{:?}", e))?,
                                    )
                                } else {
                                    AttesterSlashing::Base(
                                        AttesterSlashingBase::from_ssz_bytes(data)
                                            .map_err(|e| format!("{:?}", e))?,
                                    )
                                }
                            }
                            None => {
                                return Err(format!(
                                    "Unknown gossipsub fork digest: {:?}",
                                    gossip_topic.fork_digest
                                ));
                            }
                        };
                        Ok(PubsubMessage::AttesterSlashing(Box::new(attester_slashing)))
                    }
                    GossipKind::SignedContributionAndProof => {
                        let sync_aggregate = SignedContributionAndProof::from_ssz_bytes(data)
                            .map_err(|e| format!("{:?}", e))?;
                        Ok(PubsubMessage::SignedContributionAndProof(Box::new(
                            sync_aggregate,
                        )))
                    }
                    GossipKind::SyncCommitteeMessage(subnet_id) => {
                        let sync_committee = SyncCommitteeMessage::from_ssz_bytes(data)
                            .map_err(|e| format!("{:?}", e))?;
                        Ok(PubsubMessage::SyncCommitteeMessage(Box::new((
                            *subnet_id,
                            sync_committee,
                        ))))
                    }
                    GossipKind::BlsToExecutionChange => {
                        let bls_to_execution_change =
                            SignedBlsToExecutionChange::from_ssz_bytes(data)
                                .map_err(|e| format!("{:?}", e))?;
                        Ok(PubsubMessage::BlsToExecutionChange(Box::new(
                            bls_to_execution_change,
                        )))
                    }
                    GossipKind::LightClientFinalityUpdate => {
                        let light_client_finality_update = match fork_context
                            .get_fork_from_context_bytes(gossip_topic.fork_digest)
                        {
                            Some(&fork_name) => {
                                LightClientFinalityUpdate::from_ssz_bytes(data, fork_name)
                                    .map_err(|e| format!("{:?}", e))?
                            }
                            None => {
                                return Err(format!(
                                    "light_client_finality_update topic invalid for given fork digest {:?}",
                                    gossip_topic.fork_digest
                                ));
                            }
                        };
                        Ok(PubsubMessage::LightClientFinalityUpdate(Box::new(
                            light_client_finality_update,
                        )))
                    }
                    GossipKind::LightClientOptimisticUpdate => {
                        let light_client_optimistic_update = match fork_context
                            .get_fork_from_context_bytes(gossip_topic.fork_digest)
                        {
                            Some(&fork_name) => {
                                LightClientOptimisticUpdate::from_ssz_bytes(data, fork_name)
                                    .map_err(|e| format!("{:?}", e))?
                            }
                            None => {
                                return Err(format!(
                                    "light_client_optimistic_update topic invalid for given fork digest {:?}",
                                    gossip_topic.fork_digest
                                ));
                            }
                        };
                        Ok(PubsubMessage::LightClientOptimisticUpdate(Box::new(
                            light_client_optimistic_update,
                        )))
                    }
                    GossipKind::ExecutionBid => {
                        match fork_context.get_fork_from_context_bytes(gossip_topic.fork_digest) {
                            Some(fork) if fork.gloas_enabled() => {
                                let execution_bid = SignedExecutionPayloadBid::from_ssz_bytes(data)
                                    .map_err(|e| format!("{:?}", e))?;
                                Ok(PubsubMessage::ExecutionBid(Box::new(execution_bid)))
                            }
                            Some(_) | None => Err(format!(
                                "execution_bid topic invalid for given fork digest {:?}",
                                gossip_topic.fork_digest
                            )),
                        }
                    }
                    GossipKind::ExecutionPayload => {
                        match fork_context.get_fork_from_context_bytes(gossip_topic.fork_digest) {
                            Some(fork) if fork.gloas_enabled() => {
                                let execution_payload =
                                    SignedExecutionPayloadEnvelope::from_ssz_bytes(data)
                                        .map_err(|e| format!("{:?}", e))?;
                                Ok(PubsubMessage::ExecutionPayload(Box::new(execution_payload)))
                            }
                            Some(_) | None => Err(format!(
                                "execution_payload topic invalid for given fork digest {:?}",
                                gossip_topic.fork_digest
                            )),
                        }
                    }
                    GossipKind::PayloadAttestation => {
                        match fork_context.get_fork_from_context_bytes(gossip_topic.fork_digest) {
                            Some(fork) if fork.gloas_enabled() => {
                                let message = PayloadAttestationMessage::from_ssz_bytes(data)
                                    .map_err(|e| format!("{:?}", e))?;
                                Ok(PubsubMessage::PayloadAttestation(Box::new(message)))
                            }
                            Some(_) | None => Err(format!(
                                "payload_attestation topic invalid for given fork digest {:?}",
                                gossip_topic.fork_digest
                            )),
                        }
                    }
                    GossipKind::ProposerPreferences => {
                        match fork_context.get_fork_from_context_bytes(gossip_topic.fork_digest) {
                            Some(fork) if fork.gloas_enabled() => {
                                let preferences = SignedProposerPreferences::from_ssz_bytes(data)
                                    .map_err(|e| format!("{:?}", e))?;
                                Ok(PubsubMessage::ProposerPreferences(Box::new(preferences)))
                            }
                            Some(_) | None => Err(format!(
                                "proposer_preferences topic invalid for given fork digest {:?}",
                                gossip_topic.fork_digest
                            )),
                        }
                    }
                    GossipKind::ExecutionProof(subnet_id) => {
                        match fork_context.get_fork_from_context_bytes(gossip_topic.fork_digest) {
                            Some(fork) if fork.gloas_enabled() => {
                                let proof = Arc::new(
                                    ExecutionProof::from_ssz_bytes(data)
                                        .map_err(|e| format!("{:?}", e))?,
                                );
                                Ok(PubsubMessage::ExecutionProof(Box::new((*subnet_id, proof))))
                            }
                            Some(_) | None => Err(format!(
                                "execution_proof topic invalid for given fork digest {:?}",
                                gossip_topic.fork_digest
                            )),
                        }
                    }
                }
            }
        }
    }

    /// Encodes a `PubsubMessage` based on the topic encodings. The first known encoding is used. If
    /// no encoding is known, and error is returned.
    pub fn encode(&self, _encoding: GossipEncoding) -> Vec<u8> {
        // Currently do not employ encoding strategies based on the topic. All messages are ssz
        // encoded.
        // Also note, that the compression is handled by the `SnappyTransform` struct. Gossipsub will compress the
        // messages for us.
        match &self {
            PubsubMessage::BeaconBlock(data) => data.as_ssz_bytes(),
            PubsubMessage::BlobSidecar(data) => data.1.as_ssz_bytes(),
            PubsubMessage::DataColumnSidecar(data) => data.1.as_ssz_bytes(),
            PubsubMessage::AggregateAndProofAttestation(data) => data.as_ssz_bytes(),
            PubsubMessage::VoluntaryExit(data) => data.as_ssz_bytes(),
            PubsubMessage::ProposerSlashing(data) => data.as_ssz_bytes(),
            PubsubMessage::AttesterSlashing(data) => data.as_ssz_bytes(),
            PubsubMessage::Attestation(data) => data.1.as_ssz_bytes(),
            PubsubMessage::SignedContributionAndProof(data) => data.as_ssz_bytes(),
            PubsubMessage::SyncCommitteeMessage(data) => data.1.as_ssz_bytes(),
            PubsubMessage::BlsToExecutionChange(data) => data.as_ssz_bytes(),
            PubsubMessage::LightClientFinalityUpdate(data) => data.as_ssz_bytes(),
            PubsubMessage::LightClientOptimisticUpdate(data) => data.as_ssz_bytes(),
            PubsubMessage::ExecutionBid(data) => data.as_ssz_bytes(),
            PubsubMessage::ExecutionPayload(data) => data.as_ssz_bytes(),
            PubsubMessage::PayloadAttestation(data) => data.as_ssz_bytes(),
            PubsubMessage::ProposerPreferences(data) => data.as_ssz_bytes(),
            PubsubMessage::ExecutionProof(data) => data.1.as_ssz_bytes(),
        }
    }
}

impl<E: EthSpec> std::fmt::Display for PubsubMessage<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PubsubMessage::BeaconBlock(block) => write!(
                f,
                "Beacon Block: slot: {}, proposer_index: {}",
                block.slot(),
                block.message().proposer_index()
            ),
            PubsubMessage::BlobSidecar(data) => write!(
                f,
                "BlobSidecar: slot: {}, blob index: {}",
                data.1.slot(),
                data.1.index,
            ),
            PubsubMessage::DataColumnSidecar(data) => write!(
                f,
                "DataColumnSidecar: slot: {}, column index: {}",
                data.1.slot(),
                data.1.index(),
            ),
            PubsubMessage::AggregateAndProofAttestation(att) => write!(
                f,
                "Aggregate and Proof: slot: {}, index: {:?}, aggregator_index: {}",
                att.message().aggregate().data().slot,
                att.message().aggregate().committee_index(),
                att.message().aggregator_index(),
            ),
            PubsubMessage::Attestation(data) => write!(
                f,
                "SingleAttestation: subnet_id: {}, attestation_slot: {}, committee_index: {:?}, attester_index: {:?}",
                *data.0, data.1.data.slot, data.1.committee_index, data.1.attester_index,
            ),
            PubsubMessage::VoluntaryExit(_data) => write!(f, "Voluntary Exit"),
            PubsubMessage::ProposerSlashing(_data) => write!(f, "Proposer Slashing"),
            PubsubMessage::AttesterSlashing(_data) => write!(f, "Attester Slashing"),
            PubsubMessage::SignedContributionAndProof(_) => {
                write!(f, "Signed Contribution and Proof")
            }
            PubsubMessage::SyncCommitteeMessage(data) => {
                write!(f, "Sync committee message: subnet_id: {}", *data.0)
            }
            PubsubMessage::BlsToExecutionChange(data) => {
                write!(
                    f,
                    "Signed BLS to execution change: validator_index: {}, address: {:?}",
                    data.message.validator_index, data.message.to_execution_address
                )
            }
            PubsubMessage::LightClientFinalityUpdate(_data) => {
                write!(f, "Light CLient Finality Update")
            }
            PubsubMessage::LightClientOptimisticUpdate(_data) => {
                write!(f, "Light CLient Optimistic Update")
            }
            PubsubMessage::ExecutionBid(data) => write!(
                f,
                "Execution Bid: slot: {}, builder_index: {}, value: {}",
                data.message.slot, data.message.builder_index, data.message.value
            ),
            PubsubMessage::ExecutionPayload(data) => write!(
                f,
                "Execution Payload: slot: {}, builder_index: {}",
                data.message.slot, data.message.builder_index
            ),
            PubsubMessage::PayloadAttestation(data) => write!(
                f,
                "Payload Attestation: slot: {}, beacon_block_root: {:?}, validator_index: {}",
                data.data.slot, data.data.beacon_block_root, data.validator_index
            ),
            PubsubMessage::ProposerPreferences(data) => write!(
                f,
                "Proposer Preferences: slot: {}, validator_index: {}",
                data.message.proposal_slot, data.message.validator_index
            ),
            PubsubMessage::ExecutionProof(data) => write!(
                f,
                "Execution Proof: subnet_id: {}, block_root: {:?}",
                *data.0, data.1.block_root
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::test_utils::TestRandom;
    use types::{
        Epoch, ExecutionBlockHash, ExecutionProofSubnetId, ForkContext, Hash256, MainnetEthSpec,
        Slot,
    };

    type E = MainnetEthSpec;

    /// Create a ForkContext with all forks enabled including Gloas.
    fn gloas_fork_context() -> ForkContext {
        let mut spec = E::default_spec();
        spec.altair_fork_epoch = Some(Epoch::new(1));
        spec.bellatrix_fork_epoch = Some(Epoch::new(2));
        spec.capella_fork_epoch = Some(Epoch::new(3));
        spec.deneb_fork_epoch = Some(Epoch::new(4));
        spec.electra_fork_epoch = Some(Epoch::new(5));
        spec.fulu_fork_epoch = Some(Epoch::new(6));
        spec.gloas_fork_epoch = Some(Epoch::new(7));
        let genesis_root = Hash256::ZERO;
        let slot = Slot::new(7 * E::slots_per_epoch());
        ForkContext::new::<E>(slot, genesis_root, &spec)
    }

    /// Create a ForkContext where the latest fork is Fulu (pre-Gloas).
    fn pre_gloas_fork_context() -> ForkContext {
        let mut spec = E::default_spec();
        spec.altair_fork_epoch = Some(Epoch::new(1));
        spec.bellatrix_fork_epoch = Some(Epoch::new(2));
        spec.capella_fork_epoch = Some(Epoch::new(3));
        spec.deneb_fork_epoch = Some(Epoch::new(4));
        spec.electra_fork_epoch = Some(Epoch::new(5));
        spec.fulu_fork_epoch = Some(Epoch::new(6));
        let genesis_root = Hash256::ZERO;
        let slot = Slot::new(6 * E::slots_per_epoch());
        ForkContext::new::<E>(slot, genesis_root, &spec)
    }

    /// Build a topic hash for the given gossip kind at the current fork digest.
    fn gloas_topic(fork_context: &ForkContext, kind: GossipKind) -> TopicHash {
        let topic = GossipTopic::new(
            kind,
            GossipEncoding::SSZSnappy,
            fork_context.current_fork_digest(),
        );
        TopicHash::from_raw(topic.to_string())
    }

    // ── ExecutionBid round-trip ──

    #[test]
    fn encode_decode_execution_bid() {
        let fork_context = gloas_fork_context();
        let mut rng = rand::rng();
        let bid = SignedExecutionPayloadBid::<E>::random_for_test(&mut rng);
        let msg = PubsubMessage::<E>::ExecutionBid(Box::new(bid.clone()));

        let encoded = msg.encode(GossipEncoding::SSZSnappy);
        let topic = gloas_topic(&fork_context, GossipKind::ExecutionBid);
        let decoded = PubsubMessage::<E>::decode(&topic, &encoded, &fork_context)
            .expect("should decode ExecutionBid");

        assert_eq!(decoded, msg);
    }

    #[test]
    fn execution_bid_kind() {
        let mut rng = rand::rng();
        let bid = SignedExecutionPayloadBid::<E>::random_for_test(&mut rng);
        let msg = PubsubMessage::<E>::ExecutionBid(Box::new(bid));
        assert_eq!(msg.kind(), GossipKind::ExecutionBid);
    }

    #[test]
    fn execution_bid_rejected_pre_gloas() {
        let fork_context = pre_gloas_fork_context();
        let mut rng = rand::rng();
        let bid = SignedExecutionPayloadBid::<E>::random_for_test(&mut rng);
        let msg = PubsubMessage::<E>::ExecutionBid(Box::new(bid));
        let encoded = msg.encode(GossipEncoding::SSZSnappy);
        let topic = gloas_topic(&fork_context, GossipKind::ExecutionBid);
        let result = PubsubMessage::<E>::decode(&topic, &encoded, &fork_context);
        assert!(result.is_err());
    }

    // ── ExecutionPayload (envelope) round-trip ──

    #[test]
    fn encode_decode_execution_payload_envelope() {
        let fork_context = gloas_fork_context();
        let mut rng = rand::rng();
        let envelope = SignedExecutionPayloadEnvelope::<E>::random_for_test(&mut rng);
        let msg = PubsubMessage::<E>::ExecutionPayload(Box::new(envelope.clone()));

        let encoded = msg.encode(GossipEncoding::SSZSnappy);
        let topic = gloas_topic(&fork_context, GossipKind::ExecutionPayload);
        let decoded = PubsubMessage::<E>::decode(&topic, &encoded, &fork_context)
            .expect("should decode ExecutionPayload");

        assert_eq!(decoded, msg);
    }

    #[test]
    fn execution_payload_kind() {
        let mut rng = rand::rng();
        let envelope = SignedExecutionPayloadEnvelope::<E>::random_for_test(&mut rng);
        let msg = PubsubMessage::<E>::ExecutionPayload(Box::new(envelope));
        assert_eq!(msg.kind(), GossipKind::ExecutionPayload);
    }

    #[test]
    fn execution_payload_rejected_pre_gloas() {
        let fork_context = pre_gloas_fork_context();
        let mut rng = rand::rng();
        let envelope = SignedExecutionPayloadEnvelope::<E>::random_for_test(&mut rng);
        let msg = PubsubMessage::<E>::ExecutionPayload(Box::new(envelope));
        let encoded = msg.encode(GossipEncoding::SSZSnappy);
        let topic = gloas_topic(&fork_context, GossipKind::ExecutionPayload);
        let result = PubsubMessage::<E>::decode(&topic, &encoded, &fork_context);
        assert!(result.is_err());
    }

    // ── PayloadAttestation (PayloadAttestationMessage) round-trip ──

    #[test]
    fn encode_decode_payload_attestation() {
        let fork_context = gloas_fork_context();
        let mut rng = rand::rng();
        let message = PayloadAttestationMessage::random_for_test(&mut rng);
        let msg = PubsubMessage::<E>::PayloadAttestation(Box::new(message.clone()));

        let encoded = msg.encode(GossipEncoding::SSZSnappy);
        let topic = gloas_topic(&fork_context, GossipKind::PayloadAttestation);
        let decoded = PubsubMessage::<E>::decode(&topic, &encoded, &fork_context)
            .expect("should decode PayloadAttestationMessage");

        assert_eq!(decoded, msg);
    }

    #[test]
    fn payload_attestation_kind() {
        let mut rng = rand::rng();
        let message = PayloadAttestationMessage::random_for_test(&mut rng);
        let msg = PubsubMessage::<E>::PayloadAttestation(Box::new(message));
        assert_eq!(msg.kind(), GossipKind::PayloadAttestation);
    }

    #[test]
    fn payload_attestation_rejected_pre_gloas() {
        let fork_context = pre_gloas_fork_context();
        let mut rng = rand::rng();
        let message = PayloadAttestationMessage::random_for_test(&mut rng);
        let msg = PubsubMessage::<E>::PayloadAttestation(Box::new(message));
        let encoded = msg.encode(GossipEncoding::SSZSnappy);
        let topic = gloas_topic(&fork_context, GossipKind::PayloadAttestation);
        let result = PubsubMessage::<E>::decode(&topic, &encoded, &fork_context);
        assert!(result.is_err());
    }

    // ── ProposerPreferences round-trip ──

    #[test]
    fn encode_decode_proposer_preferences() {
        let fork_context = gloas_fork_context();
        let mut rng = rand::rng();
        let prefs = SignedProposerPreferences::random_for_test(&mut rng);
        let msg = PubsubMessage::<E>::ProposerPreferences(Box::new(prefs.clone()));

        let encoded = msg.encode(GossipEncoding::SSZSnappy);
        let topic = gloas_topic(&fork_context, GossipKind::ProposerPreferences);
        let decoded = PubsubMessage::<E>::decode(&topic, &encoded, &fork_context)
            .expect("should decode ProposerPreferences");

        assert_eq!(decoded, msg);
    }

    #[test]
    fn proposer_preferences_kind() {
        let mut rng = rand::rng();
        let prefs = SignedProposerPreferences::random_for_test(&mut rng);
        let msg = PubsubMessage::<E>::ProposerPreferences(Box::new(prefs));
        assert_eq!(msg.kind(), GossipKind::ProposerPreferences);
    }

    #[test]
    fn proposer_preferences_rejected_pre_gloas() {
        let fork_context = pre_gloas_fork_context();
        let mut rng = rand::rng();
        let prefs = SignedProposerPreferences::random_for_test(&mut rng);
        let msg = PubsubMessage::<E>::ProposerPreferences(Box::new(prefs));
        let encoded = msg.encode(GossipEncoding::SSZSnappy);
        let topic = gloas_topic(&fork_context, GossipKind::ProposerPreferences);
        let result = PubsubMessage::<E>::decode(&topic, &encoded, &fork_context);
        assert!(result.is_err());
    }

    // ── ExecutionProof round-trip ──

    #[test]
    fn encode_decode_execution_proof() {
        let fork_context = gloas_fork_context();
        let subnet_id = ExecutionProofSubnetId::new(0).unwrap();
        let proof = ExecutionProof::new(
            Hash256::random(),
            ExecutionBlockHash::from_root(Hash256::random()),
            subnet_id,
            1,
            vec![0xde, 0xad, 0xbe, 0xef],
        );
        let msg =
            PubsubMessage::<E>::ExecutionProof(Box::new((subnet_id, Arc::new(proof.clone()))));

        let encoded = msg.encode(GossipEncoding::SSZSnappy);
        let topic = gloas_topic(&fork_context, GossipKind::ExecutionProof(subnet_id));
        let decoded = PubsubMessage::<E>::decode(&topic, &encoded, &fork_context)
            .expect("should decode ExecutionProof");

        match decoded {
            PubsubMessage::ExecutionProof(data) => {
                assert_eq!(*data.0, *subnet_id);
                assert_eq!(*data.1, proof);
            }
            _ => panic!("expected ExecutionProof variant"),
        }
    }

    #[test]
    fn execution_proof_kind() {
        let subnet_id = ExecutionProofSubnetId::new(0).unwrap();
        let proof = ExecutionProof::new(
            Hash256::random(),
            ExecutionBlockHash::from_root(Hash256::random()),
            subnet_id,
            1,
            vec![],
        );
        let msg = PubsubMessage::<E>::ExecutionProof(Box::new((subnet_id, Arc::new(proof))));
        assert_eq!(msg.kind(), GossipKind::ExecutionProof(subnet_id));
    }

    #[test]
    fn execution_proof_rejected_pre_gloas() {
        let fork_context = pre_gloas_fork_context();
        let subnet_id = ExecutionProofSubnetId::new(0).unwrap();
        let proof = ExecutionProof::new(
            Hash256::random(),
            ExecutionBlockHash::from_root(Hash256::random()),
            subnet_id,
            1,
            vec![0x01],
        );
        let msg = PubsubMessage::<E>::ExecutionProof(Box::new((subnet_id, Arc::new(proof))));
        let encoded = msg.encode(GossipEncoding::SSZSnappy);
        let topic = gloas_topic(&fork_context, GossipKind::ExecutionProof(subnet_id));
        let result = PubsubMessage::<E>::decode(&topic, &encoded, &fork_context);
        assert!(result.is_err());
    }

    // ── Gloas BeaconBlock round-trip ──

    #[test]
    fn encode_decode_gloas_beacon_block() {
        let fork_context = gloas_fork_context();
        let mut rng = rand::rng();
        let block = SignedBeaconBlockGloas::<E>::random_for_test(&mut rng);
        let signed = SignedBeaconBlock::<E>::Gloas(block);
        let msg = PubsubMessage::<E>::BeaconBlock(Arc::new(signed.clone()));

        let encoded = msg.encode(GossipEncoding::SSZSnappy);
        let topic = gloas_topic(&fork_context, GossipKind::BeaconBlock);
        let decoded = PubsubMessage::<E>::decode(&topic, &encoded, &fork_context)
            .expect("should decode Gloas BeaconBlock");

        match decoded {
            PubsubMessage::BeaconBlock(decoded_block) => {
                assert_eq!(*decoded_block, signed);
            }
            _ => panic!("expected BeaconBlock variant"),
        }
    }

    // ── Invalid SSZ data ──

    #[test]
    fn execution_bid_invalid_ssz() {
        let fork_context = gloas_fork_context();
        let topic = gloas_topic(&fork_context, GossipKind::ExecutionBid);
        let result = PubsubMessage::<E>::decode(&topic, &[0xff, 0x00], &fork_context);
        assert!(result.is_err());
    }

    #[test]
    fn execution_payload_invalid_ssz() {
        let fork_context = gloas_fork_context();
        let topic = gloas_topic(&fork_context, GossipKind::ExecutionPayload);
        let result = PubsubMessage::<E>::decode(&topic, &[0xff, 0x00], &fork_context);
        assert!(result.is_err());
    }

    #[test]
    fn payload_attestation_invalid_ssz() {
        let fork_context = gloas_fork_context();
        let topic = gloas_topic(&fork_context, GossipKind::PayloadAttestation);
        let result = PubsubMessage::<E>::decode(&topic, &[0xff, 0x00], &fork_context);
        assert!(result.is_err());
    }

    #[test]
    fn proposer_preferences_invalid_ssz() {
        let fork_context = gloas_fork_context();
        let topic = gloas_topic(&fork_context, GossipKind::ProposerPreferences);
        let result = PubsubMessage::<E>::decode(&topic, &[0xff, 0x00], &fork_context);
        assert!(result.is_err());
    }

    #[test]
    fn execution_proof_invalid_ssz() {
        let fork_context = gloas_fork_context();
        let subnet_id = ExecutionProofSubnetId::new(0).unwrap();
        let topic = gloas_topic(&fork_context, GossipKind::ExecutionProof(subnet_id));
        let result = PubsubMessage::<E>::decode(&topic, &[0xff, 0x00], &fork_context);
        assert!(result.is_err());
    }
}
