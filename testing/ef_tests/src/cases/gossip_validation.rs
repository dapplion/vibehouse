use super::*;
use crate::bls_setting::BlsSetting;
use crate::decode::{ssz_decode_file, ssz_decode_file_with, ssz_decode_state, yaml_decode_file};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use types::{
    AttesterSlashing, BeaconState, ChainSpec, Checkpoint, EthSpec, Hash256, ProposerSlashing,
    PublicKey, SignedBeaconBlock, SignedVoluntaryExit, Slot,
};

/// Expected outcome of a gossip message per the spec.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GossipExpected {
    Valid,
    Reject,
    Ignore,
}

/// A single message step in a gossip validation test.
#[derive(Debug, Clone, Deserialize)]
pub struct GossipMessage {
    pub message: String,
    pub expected: GossipExpected,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Metadata for a gossip validation test case.
#[derive(Debug, Clone, Deserialize)]
pub struct GossipMeta {
    pub topic: String,
    pub messages: Vec<GossipMessage>,
    #[serde(default)]
    pub bls_setting: Option<BlsSetting>,
}

/// Metadata for beacon block gossip tests (richer than GossipMeta — includes blocks, timing,
/// finalized checkpoint).
#[derive(Debug, Clone, Deserialize)]
pub struct GossipBeaconBlockMeta {
    pub topic: String,
    #[serde(default)]
    pub blocks: Vec<GossipBlockRef>,
    #[serde(default)]
    pub finalized_checkpoint: Option<GossipCheckpoint>,
    pub current_time_ms: u64,
    pub messages: Vec<GossipBlockMessage>,
    #[serde(default)]
    pub bls_setting: Option<BlsSetting>,
}

/// A block reference in the test setup — processed before gossip messages.
#[derive(Debug, Clone, Deserialize)]
pub struct GossipBlockRef {
    pub block: String,
    #[serde(default)]
    pub failed: bool,
}

/// Finalized checkpoint override — may use `root` (hex string) or `block` (block reference).
#[derive(Debug, Clone, Deserialize)]
pub struct GossipCheckpoint {
    pub epoch: u64,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub block: Option<String>,
}

/// A gossip message with timing info.
#[derive(Debug, Clone, Deserialize)]
pub struct GossipBlockMessage {
    pub offset_ms: u64,
    pub message: String,
    pub expected: GossipExpected,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Gossip validation test for beacon blocks.
#[derive(Debug)]
pub struct GossipBeaconBlock<E: EthSpec> {
    pub meta: GossipBeaconBlockMeta,
    pub state: BeaconState<E>,
    pub blocks: Vec<(String, SignedBeaconBlock<E>)>,
}

/// Gossip validation test for proposer slashings.
#[derive(Debug, Clone)]
pub struct GossipProposerSlashing<E: EthSpec> {
    pub meta: GossipMeta,
    pub state: BeaconState<E>,
    pub slashings: Vec<(String, ProposerSlashing)>,
}

/// Gossip validation test for attester slashings.
#[derive(Debug, Clone)]
pub struct GossipAttesterSlashing<E: EthSpec> {
    pub meta: GossipMeta,
    pub state: BeaconState<E>,
    pub slashings: Vec<(String, AttesterSlashing<E>)>,
}

/// Gossip validation test for voluntary exits.
#[derive(Debug, Clone)]
pub struct GossipVoluntaryExit<E: EthSpec> {
    pub meta: GossipMeta,
    pub state: BeaconState<E>,
    pub exits: Vec<(String, SignedVoluntaryExit)>,
}

/// Load SSZ-encoded operations from the test directory.
///
/// Files matching the pattern `{prefix}_{hash}.ssz_snappy` are loaded.
fn load_operations<T: ssz::Decode>(path: &Path, prefix: &str) -> Result<Vec<(String, T)>, Error> {
    let mut ops = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|e| {
        Error::FailedToParseTest(format!("Failed to read dir {}: {e}", path.display()))
    })? {
        let entry = entry.map_err(|e| Error::FailedToParseTest(format!("{e}")))?;
        let file_name = entry.file_name().to_string_lossy().to_string();
        if file_name.starts_with(prefix) && file_name.ends_with(".ssz_snappy") {
            let name = file_name.strip_suffix(".ssz_snappy").unwrap().to_string();
            let op: T = ssz_decode_file(&entry.path())?;
            ops.push((name, op));
        }
    }
    Ok(ops)
}

impl<E: EthSpec> LoadCase for GossipBeaconBlock<E> {
    fn load_from_dir(path: &Path, fork_name: ForkName) -> Result<Self, Error> {
        let spec = &crate::testing_spec::<E>(fork_name);
        let meta: GossipBeaconBlockMeta = yaml_decode_file(&path.join("meta.yaml"))?;
        let state = ssz_decode_state(&path.join("state.ssz_snappy"), spec)?;

        // Load all block_*.ssz_snappy files.
        let mut blocks = Vec::new();
        for entry in std::fs::read_dir(path).map_err(|e| {
            Error::FailedToParseTest(format!("Failed to read dir {}: {e}", path.display()))
        })? {
            let entry = entry.map_err(|e| Error::FailedToParseTest(format!("{e}")))?;
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.starts_with("block_") && file_name.ends_with(".ssz_snappy") {
                let name = file_name.strip_suffix(".ssz_snappy").unwrap().to_string();
                let block: SignedBeaconBlock<E> = ssz_decode_file_with(&entry.path(), |bytes| {
                    SignedBeaconBlock::from_ssz_bytes(bytes, spec)
                })?;
                blocks.push((name, block));
            }
        }

        Ok(Self {
            meta,
            state,
            blocks,
        })
    }
}

impl<E: EthSpec> LoadCase for GossipProposerSlashing<E> {
    fn load_from_dir(path: &Path, fork_name: ForkName) -> Result<Self, Error> {
        let spec = &crate::testing_spec::<E>(fork_name);
        let meta: GossipMeta = yaml_decode_file(&path.join("meta.yaml"))?;
        let state = ssz_decode_state(&path.join("state.ssz_snappy"), spec)?;
        let slashings = load_operations::<ProposerSlashing>(path, "proposer_slashing_")?;
        Ok(Self {
            meta,
            state,
            slashings,
        })
    }
}

impl<E: EthSpec> LoadCase for GossipVoluntaryExit<E> {
    fn load_from_dir(path: &Path, fork_name: ForkName) -> Result<Self, Error> {
        let spec = &crate::testing_spec::<E>(fork_name);
        let meta: GossipMeta = yaml_decode_file(&path.join("meta.yaml"))?;
        let state = ssz_decode_state(&path.join("state.ssz_snappy"), spec)?;
        let exits = load_operations::<SignedVoluntaryExit>(path, "voluntary_exit_")?;
        Ok(Self { meta, state, exits })
    }
}

impl<E: EthSpec> LoadCase for GossipAttesterSlashing<E> {
    fn load_from_dir(path: &Path, fork_name: ForkName) -> Result<Self, Error> {
        use crate::decode::ssz_decode_file_with;
        use ssz::Decode as _;

        let spec = &crate::testing_spec::<E>(fork_name);
        let meta: GossipMeta = yaml_decode_file(&path.join("meta.yaml"))?;
        let state = ssz_decode_state(&path.join("state.ssz_snappy"), spec)?;

        // AttesterSlashing SSZ encoding differs by fork (Base vs Electra).
        let mut slashings = Vec::new();
        for entry in std::fs::read_dir(path).map_err(|e| {
            Error::FailedToParseTest(format!("Failed to read dir {}: {e}", path.display()))
        })? {
            let entry = entry.map_err(|e| Error::FailedToParseTest(format!("{e}")))?;
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.starts_with("attester_slashing_") && file_name.ends_with(".ssz_snappy") {
                let name = file_name.strip_suffix(".ssz_snappy").unwrap().to_string();
                let op: AttesterSlashing<E> = if fork_name.electra_enabled() {
                    ssz_decode_file_with(&entry.path(), |bytes| {
                        types::AttesterSlashingElectra::from_ssz_bytes(bytes)
                            .map(AttesterSlashing::Electra)
                    })?
                } else {
                    ssz_decode_file_with(&entry.path(), |bytes| {
                        types::AttesterSlashingBase::from_ssz_bytes(bytes)
                            .map(AttesterSlashing::Base)
                    })?
                };
                slashings.push((name, op));
            }
        }

        Ok(Self {
            meta,
            state,
            slashings,
        })
    }
}

/// Run a gossip validation test by processing messages in order and checking outcomes.
fn run_gossip_test<E, T, F>(
    meta: &GossipMeta,
    state: &BeaconState<E>,
    operations: &[(String, T)],
    spec: &ChainSpec,
    validate_fn: F,
) -> Result<(), Error>
where
    E: EthSpec,
    T: Clone + Debug,
    F: Fn(
        &T,
        &BeaconState<E>,
        &HashSet<u64>,
        &ChainSpec,
    ) -> Result<(GossipExpected, Vec<u64>), String>,
{
    // Track seen validator indices across messages (mimics ObservedOperations).
    let mut seen_indices: HashSet<u64> = HashSet::new();

    for (msg_idx, msg) in meta.messages.iter().enumerate() {
        // Find the operation by name.
        let (_, op) = operations
            .iter()
            .find(|(name, _)| *name == msg.message)
            .ok_or_else(|| {
                Error::FailedToParseTest(format!(
                    "Message '{}' not found in test directory",
                    msg.message
                ))
            })?;

        let result = validate_fn(op, state, &seen_indices, spec);

        match (&msg.expected, result) {
            (GossipExpected::Valid, Ok((GossipExpected::Valid, new_indices))) => {
                // Valid: add indices to seen set.
                seen_indices.extend(new_indices);
            }
            (GossipExpected::Ignore, Ok((GossipExpected::Ignore, _)))
            | (GossipExpected::Reject, Err(_)) => {
                // Correctly ignored (already seen) or rejected with validation error.
            }
            (expected, actual) => {
                return Err(Error::NotEqual(format!(
                    "Message {} (index {msg_idx}): expected {expected:?}, got {actual:?}",
                    msg.message,
                )));
            }
        }
    }

    Ok(())
}

impl<E: EthSpec> Case for GossipBeaconBlock<E> {
    fn result(&self, _case_index: usize, fork_name: ForkName) -> Result<(), Error> {
        struct BlockInfo {
            parent_root: Hash256,
            slot: Slot,
            failed: bool,
        }

        if let Some(bls_setting) = self.meta.bls_setting {
            bls_setting.check()?;
        }

        let spec = crate::testing_spec::<E>(fork_name);
        let seconds_per_slot = spec.seconds_per_slot;
        let genesis_time = self.state.genesis_time();

        // Build a lookup of block name → SignedBeaconBlock.
        let block_map: HashMap<&str, &SignedBeaconBlock<E>> = self
            .blocks
            .iter()
            .map(|(name, block)| (name.as_str(), block))
            .collect();

        // Build the store: track known block roots, their parent roots, slots, and validity.
        let mut known_blocks: HashMap<Hash256, BlockInfo> = HashMap::new();

        // Process pre-existing blocks from the test setup.
        for block_ref in &self.meta.blocks {
            let block = block_map.get(block_ref.block.as_str()).ok_or_else(|| {
                Error::FailedToParseTest(format!(
                    "Block ref '{}' not found in test directory",
                    block_ref.block
                ))
            })?;
            let root = block.canonical_root();
            known_blocks.insert(
                root,
                BlockInfo {
                    parent_root: block.message().parent_root(),
                    slot: block.message().slot(),
                    failed: block_ref.failed,
                },
            );
        }

        // Finalized checkpoint — from meta override or from state.
        let finalized_checkpoint = if let Some(ref cp) = self.meta.finalized_checkpoint {
            let root = if let Some(ref root_hex) = cp.root {
                // Hex-encoded root (e.g. "0xababab...").
                let hex_str = root_hex.strip_prefix("0x").unwrap_or(root_hex);
                let bytes = hex::decode(hex_str).map_err(|e| {
                    Error::FailedToParseTest(format!("Bad checkpoint root hex: {e}"))
                })?;
                if bytes.len() != 32 {
                    return Err(Error::FailedToParseTest(format!(
                        "Checkpoint root wrong length: {}",
                        bytes.len()
                    )));
                }
                Hash256::from_slice(&bytes)
            } else if let Some(ref block_ref) = cp.block {
                // Block reference name (e.g. "block_0x...").
                let block = block_map.get(block_ref.as_str()).ok_or_else(|| {
                    Error::FailedToParseTest(format!(
                        "Finalized checkpoint block '{block_ref}' not found",
                    ))
                })?;
                block.canonical_root()
            } else {
                return Err(Error::FailedToParseTest(
                    "Finalized checkpoint has neither root nor block".into(),
                ));
            };
            Checkpoint {
                epoch: types::Epoch::new(cp.epoch),
                root,
            }
        } else {
            self.state.finalized_checkpoint()
        };

        let finalized_slot = finalized_checkpoint.epoch.start_slot(E::slots_per_epoch());

        // Track seen proposer (slot, proposer_index) pairs.
        let mut seen_proposers: HashSet<(Slot, u64)> = HashSet::new();

        // Process each gossip message.
        for (msg_idx, msg) in self.meta.messages.iter().enumerate() {
            let block = block_map.get(msg.message.as_str()).ok_or_else(|| {
                Error::FailedToParseTest(format!(
                    "Message '{}' not found in test directory",
                    msg.message
                ))
            })?;

            let block_msg = block.message();
            let block_slot = block_msg.slot();
            let proposer_index = block_msg.proposer_index();
            let parent_root = block_msg.parent_root();

            let current_time_ms = self.meta.current_time_ms + msg.offset_ms;

            let result: Result<GossipExpected, String> = (|| {
                // 1. The block is not from a future slot (with clock disparity allowance).
                // Slot start time in ms = genesis_time * 1000 + block_slot * seconds_per_slot * 1000
                let slot_start_ms =
                    genesis_time * 1000 + block_slot.as_u64() * seconds_per_slot * 1000;
                let max_allowed_ms = current_time_ms + spec.maximum_gossip_clock_disparity;
                if slot_start_ms > max_allowed_ms {
                    return Ok(GossipExpected::Ignore);
                }

                // 2. The block is from a slot greater than the latest finalized slot.
                if block_slot <= finalized_slot {
                    return Ok(GossipExpected::Ignore);
                }

                // 3. The block's parent has been seen.
                let Some(parent_info) = known_blocks.get(&parent_root) else {
                    return Ok(GossipExpected::Ignore);
                };

                // 4. Check parent didn't fail validation.
                if parent_info.failed {
                    return Err("parent block failed validation".into());
                }

                // 5. Block slot must be higher than parent slot.
                if block_slot <= parent_info.slot {
                    return Err("block slot not higher than parent".into());
                }

                // 6. Proposer index must be in range.
                if proposer_index as usize >= self.state.validators().len() {
                    return Err("proposer index out of range".into());
                }

                // 7. Expected proposer check — advance state to the block's slot and compute.
                let expected_proposer = {
                    let mut state_clone = self.state.clone();
                    // Advance state slot if needed (without processing epochs fully,
                    // just update the slot for proposer computation).
                    while state_clone.slot() < block_slot {
                        // Use per_slot_processing to advance the state.
                        state_processing::per_slot_processing(&mut state_clone, None, &spec)
                            .map_err(|e| format!("slot processing failed: {e:?}"))?;
                    }
                    state_clone
                        .get_beacon_proposer_index(block_slot, &spec)
                        .map_err(|e| format!("proposer index computation failed: {e:?}"))?
                        as u64
                };
                if proposer_index != expected_proposer {
                    return Err(format!(
                        "wrong proposer: block has {proposer_index}, expected {expected_proposer}"
                    ));
                }

                // 8. Already seen proposer for this slot.
                if seen_proposers.contains(&(block_slot, proposer_index)) {
                    return Ok(GossipExpected::Ignore);
                }

                // 9. Verify block signature.
                if cfg!(not(feature = "fake_crypto")) {
                    use state_processing::per_block_processing::signature_sets::block_proposal_signature_set_from_parts;
                    use std::borrow::Cow;
                    let get_pubkey = |i: usize| -> Option<Cow<'_, PublicKey>> {
                        self.state
                            .validators()
                            .get(i)
                            .and_then(|v| v.pubkey.decompress().ok())
                            .map(Cow::Owned)
                    };
                    let sig_set = block_proposal_signature_set_from_parts(
                        *block,
                        None,
                        proposer_index,
                        &self.state.fork(),
                        self.state.genesis_validators_root(),
                        get_pubkey,
                        &spec,
                    )
                    .map_err(|e| format!("signature set error: {e:?}"))?;
                    if !sig_set.verify() {
                        return Err("invalid proposer signature".into());
                    }
                }

                // 10. Finalized checkpoint ancestry check.
                // Walk back from block's parent to see if finalized root is an ancestor.
                if finalized_checkpoint.epoch > types::Epoch::new(0)
                    || finalized_checkpoint.root != Hash256::ZERO
                {
                    let mut current = parent_root;
                    let mut found = current == finalized_checkpoint.root;
                    while !found {
                        if let Some(info) = known_blocks.get(&current) {
                            if current == finalized_checkpoint.root {
                                found = true;
                            } else {
                                current = info.parent_root;
                                if current == finalized_checkpoint.root {
                                    found = true;
                                }
                                // If we've reached a block with no known parent, stop.
                                if !known_blocks.contains_key(&current)
                                    && current != finalized_checkpoint.root
                                {
                                    break;
                                }
                            }
                        } else {
                            // Parent not in known blocks — check if it's the finalized root.
                            break;
                        }
                    }
                    if !found {
                        return Err("finalized checkpoint not ancestor of block".into());
                    }
                }

                Ok(GossipExpected::Valid)
            })();

            let expected = &msg.expected;
            match (expected, &result) {
                (GossipExpected::Valid, Ok(GossipExpected::Valid)) => {
                    // Valid — add to known blocks and mark proposer as seen.
                    seen_proposers.insert((block_slot, proposer_index));
                    let root = block.canonical_root();
                    known_blocks.insert(
                        root,
                        BlockInfo {
                            parent_root,
                            slot: block_slot,
                            failed: false,
                        },
                    );
                }
                (GossipExpected::Ignore, Ok(GossipExpected::Ignore))
                | (GossipExpected::Reject, Err(_)) => {}
                _ => {
                    return Err(Error::NotEqual(format!(
                        "Message {} (index {msg_idx}): expected {expected:?}, got {result:?}{}",
                        msg.message,
                        msg.reason
                            .as_ref()
                            .map(|r| format!(" (reason: {r})"))
                            .unwrap_or_default(),
                    )));
                }
            }
        }

        Ok(())
    }
}

impl<E: EthSpec> Case for GossipProposerSlashing<E> {
    fn result(&self, _case_index: usize, _fork_name: ForkName) -> Result<(), Error> {
        if let Some(bls_setting) = self.meta.bls_setting {
            bls_setting.check()?;
        }

        let spec = E::default_spec();

        run_gossip_test(
            &self.meta,
            &self.state,
            &self.slashings,
            &spec,
            |slashing, state, seen, spec| {
                use state_processing::per_block_processing::{
                    VerifySignatures, verify_proposer_slashing,
                };

                let proposer_index = slashing.signed_header_1.message.proposer_index;

                // Check already-seen (spec: proposer index not already in observed set).
                if seen.contains(&proposer_index) {
                    return Ok((GossipExpected::Ignore, vec![]));
                }

                // Validate the slashing.
                let verify_sigs = if cfg!(feature = "fake_crypto") {
                    VerifySignatures::False
                } else {
                    VerifySignatures::True
                };
                verify_proposer_slashing(slashing, state, verify_sigs, spec)
                    .map(|()| (GossipExpected::Valid, vec![proposer_index]))
                    .map_err(|e| format!("{e:?}"))
            },
        )
    }
}

impl<E: EthSpec> Case for GossipAttesterSlashing<E> {
    fn result(&self, _case_index: usize, _fork_name: ForkName) -> Result<(), Error> {
        if let Some(bls_setting) = self.meta.bls_setting {
            bls_setting.check()?;
        }

        let spec = E::default_spec();

        run_gossip_test(
            &self.meta,
            &self.state,
            &self.slashings,
            &spec,
            |slashing, state, seen, spec| {
                use state_processing::per_block_processing::{
                    VerifySignatures, verify_attester_slashing,
                };

                // Compute the intersection of attesting indices (observable validators).
                let indices_1: HashSet<u64> = slashing
                    .attestation_1()
                    .attesting_indices_iter()
                    .copied()
                    .collect();
                let indices_2: HashSet<u64> = slashing
                    .attestation_2()
                    .attesting_indices_iter()
                    .copied()
                    .collect();

                let intersection: Vec<u64> = indices_1.intersection(&indices_2).copied().collect();

                // Check already-seen: at least one intersection index must be new.
                // Empty intersection also counts as "all seen" → IGNORE.
                if intersection.iter().all(|idx| seen.contains(idx)) {
                    return Ok((GossipExpected::Ignore, vec![]));
                }

                // Validate the slashing.
                let verify_sigs = if cfg!(feature = "fake_crypto") {
                    VerifySignatures::False
                } else {
                    VerifySignatures::True
                };
                verify_attester_slashing(state, slashing.to_ref(), verify_sigs, spec)
                    .map(|_slashable_indices| (GossipExpected::Valid, intersection))
                    .map_err(|e| format!("{e:?}"))
            },
        )
    }
}

impl<E: EthSpec> Case for GossipVoluntaryExit<E> {
    fn result(&self, _case_index: usize, _fork_name: ForkName) -> Result<(), Error> {
        if let Some(bls_setting) = self.meta.bls_setting {
            bls_setting.check()?;
        }

        let spec = E::default_spec();

        run_gossip_test(
            &self.meta,
            &self.state,
            &self.exits,
            &spec,
            |signed_exit, state, seen, spec| {
                use state_processing::per_block_processing::verify_exit;

                let validator_index = signed_exit.message.validator_index;

                // Check already-seen (spec: validator_index not already in observed set).
                if seen.contains(&validator_index) {
                    return Ok((GossipExpected::Ignore, vec![]));
                }

                // Validate the exit.
                let verify_sigs = if cfg!(feature = "fake_crypto") {
                    state_processing::per_block_processing::VerifySignatures::False
                } else {
                    state_processing::per_block_processing::VerifySignatures::True
                };
                verify_exit(state, None, signed_exit, verify_sigs, spec)
                    .map(|_is_builder| (GossipExpected::Valid, vec![validator_index]))
                    .map_err(|e| format!("{e:?}"))
            },
        )
    }
}
