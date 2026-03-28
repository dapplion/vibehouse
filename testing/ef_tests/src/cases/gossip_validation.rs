use super::*;
use crate::bls_setting::BlsSetting;
use crate::decode::{ssz_decode_file, ssz_decode_state, yaml_decode_file};
use serde::Deserialize;
use std::collections::HashSet;
use std::fmt::Debug;
use types::{AttesterSlashing, BeaconState, ChainSpec, EthSpec, ProposerSlashing};

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
