//! Hierarchical diff implementation.
use crate::{DBColumn, StoreConfig, StoreItem, metrics};
use bls::PublicKeyBytes;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use ssz::{Decode, Encode};
use ssz_derive::{Decode, Encode};
use std::cmp::Ordering;
use std::ops::RangeInclusive;
use std::str::FromStr;
use std::sync::LazyLock;
use superstruct::superstruct;
use types::historical_summary::HistoricalSummary;
use types::{BeaconState, ChainSpec, Epoch, EthSpec, Hash256, List, Slot, Validator};

static EMPTY_PUBKEY: LazyLock<PublicKeyBytes> = LazyLock::new(PublicKeyBytes::empty);

#[derive(Debug)]
pub enum Error {
    InvalidHierarchy,
    DiffDeletionsNotSupported,
    UnableToComputeDiff(xdelta3::Error),
    UnableToApplyDiff(xdelta3::Error),
    BalancesIncompleteChunk,
    Compression(std::io::Error),
    InvalidSszState(ssz::DecodeError),
    InvalidBalancesLength,
    LessThanStart(Slot, Slot),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
pub struct HierarchyConfig {
    /// A sequence of powers of two to define how frequently to store each layer of state diffs.
    /// The last value always represents the frequency of full state snapshots. Adding more
    /// exponents increases the number of diff layers. This value allows to customize the trade-off
    /// between reconstruction speed and disk space.
    ///
    /// Consider an example exponents value of `[5,13,21]`. This means we have 3 layers:
    /// - Full state stored every 2^21 slots (2097152 slots or 291 days)
    /// - First diff layer stored every 2^13 slots (8192 slots or 2.3 hours)
    /// - Second diff layer stored every 2^5 slots (32 slots or 1 epoch)
    ///
    /// To reconstruct a state at slot 3,000,003 we load each closest layer
    /// - Layer 0: 3000003 - (3000003 mod 2^21) = 2097152
    /// - Layer 1: 3000003 - (3000003 mod 2^13) = 2998272
    /// - Layer 2: 3000003 - (3000003 mod 2^5)  = 3000000
    ///
    /// Layer 0 is full state snapshot, apply layer 1 diff, then apply layer 2 diff and then replay
    /// blocks 3,000,001 to 3,000,003.
    pub exponents: Vec<u8>,
}

impl FromStr for HierarchyConfig {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let exponents = s
            .split(',')
            .map(|s| {
                s.parse()
                    .map_err(|e| format!("invalid hierarchy-exponents: {e:?}"))
            })
            .collect::<Result<Vec<u8>, _>>()?;

        if exponents.windows(2).any(|w| w[0] >= w[1]) {
            return Err("hierarchy-exponents must be in ascending order".to_string());
        }

        if exponents.is_empty() {
            return Err("empty exponents".to_string());
        }

        Ok(HierarchyConfig { exponents })
    }
}

impl std::fmt::Display for HierarchyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.exponents.iter().join(","))
    }
}

#[derive(Debug)]
pub struct HierarchyModuli {
    moduli: Vec<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum StorageStrategy {
    ReplayFrom(Slot),
    DiffFrom(Slot),
    Snapshot,
}

/// Hierarchical diff output and working buffer.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct HDiffBuffer {
    state: Vec<u8>,
    balances: Vec<u64>,
    inactivity_scores: Vec<u64>,
    validators: Vec<Validator>,
    historical_roots: Vec<Hash256>,
    historical_summaries: Vec<HistoricalSummary>,
}

/// Hierarchical state diff.
///
/// Splits the diff into two data sections:
///
/// - **balances**: The balance of each active validator is almost certain to change every epoch.
///   So this is the field in the state with most entropy. However the balance changes are small.
///   We can optimize the diff significantly by computing the balance difference first and then
///   compressing the result to squash those leading zero bytes.
///
/// - **everything else**: Instead of trying to apply heuristics and be clever on each field,
///   running a generic binary diff algorithm on the rest of fields yields very good results. With
///   this strategy the HDiff code is easily mantainable across forks, as new fields are covered
///   automatically. xdelta3 algorithm showed diff compute and apply times of ~200 ms on a mainnet
///   state from Apr 2023 (570k indexes), and a 92kB diff size.
#[superstruct(
    variants(V0),
    variant_attributes(derive(Debug, PartialEq, Encode, Decode))
)]
#[derive(Debug, PartialEq, Encode, Decode)]
#[ssz(enum_behaviour = "union")]
pub struct HDiff {
    state_diff: BytesDiff,
    balances_diff: CompressedU64Diff,
    /// inactivity_scores are small integers that change slowly epoch to epoch. And are 0 for all
    /// participants unless there's non-finality. Computing the diff and compressing the result is
    /// much faster than running them through a binary patch algorithm. In the default case where
    /// all values are 0 it should also result in a tiny output.
    inactivity_scores_diff: CompressedU64Diff,
    /// The validators array represents the vast majority of data in a BeaconState. Due to its big
    /// size we have seen the performance of xdelta3 degrade. Comparing each entry of the
    /// validators array manually significantly speeds up the computation of the diff (+10x faster)
    /// and result in the same minimal diff. As the `Validator` record is unlikely to change,
    /// maintaining this extra complexity should be okay.
    validators_diff: ValidatorsDiff,
    /// `historical_roots` is an unbounded forever growing (after Capella it's
    /// historical_summaries) list of unique roots. This data is pure entropy so there's no point
    /// in compressing it. As it's an append only list, the optimal diff + compression is just the
    /// list of new entries. The size of `historical_roots` and `historical_summaries` in
    /// non-trivial ~10 MB so throwing it to xdelta3 adds CPU cycles. With a bit of extra complexity
    /// we can save those completely.
    historical_roots: AppendOnlyDiff<Hash256>,
    /// See historical_roots
    historical_summaries: AppendOnlyDiff<HistoricalSummary>,
}

#[derive(Debug, PartialEq, Encode, Decode)]
pub struct BytesDiff {
    bytes: Vec<u8>,
}

#[derive(Debug, PartialEq, Encode, Decode)]
pub struct CompressedU64Diff {
    bytes: Vec<u8>,
}

#[derive(Debug, PartialEq, Encode, Decode)]
pub struct ValidatorsDiff {
    bytes: Vec<u8>,
}

#[derive(Debug, PartialEq, Encode, Decode)]
pub struct AppendOnlyDiff<T: Encode + Decode> {
    values: Vec<T>,
}

impl HDiffBuffer {
    pub fn from_state<E: EthSpec>(mut beacon_state: BeaconState<E>) -> Self {
        let _t = metrics::start_timer(&metrics::STORE_BEACON_HDIFF_BUFFER_FROM_STATE_TIME);
        // Set state.balances to empty list, and then serialize state as ssz
        let balances_list = std::mem::take(beacon_state.balances_mut());
        let inactivity_scores = if let Ok(inactivity_scores) = beacon_state.inactivity_scores_mut()
        {
            std::mem::take(inactivity_scores).to_vec()
        } else {
            // If this state is pre-altair consider the list empty. If the target state
            // is post altair, all its items will show up in the diff as is.
            vec![]
        };
        let validators = std::mem::take(beacon_state.validators_mut()).to_vec();
        let historical_roots = std::mem::take(beacon_state.historical_roots_mut()).to_vec();
        let historical_summaries =
            if let Ok(historical_summaries) = beacon_state.historical_summaries_mut() {
                std::mem::take(historical_summaries).to_vec()
            } else {
                // If this state is pre-capella consider the list empty. The diff will
                // include all items in the target state. If both states are
                // pre-capella the diff will be empty.
                vec![]
            };

        let state = beacon_state.as_ssz_bytes();
        let balances = balances_list.to_vec();

        HDiffBuffer {
            state,
            balances,
            inactivity_scores,
            validators,
            historical_roots,
            historical_summaries,
        }
    }

    pub fn as_state<E: EthSpec>(&self, spec: &ChainSpec) -> Result<BeaconState<E>, Error> {
        let _t = metrics::start_timer(&metrics::STORE_BEACON_HDIFF_BUFFER_INTO_STATE_TIME);
        let mut state =
            BeaconState::from_ssz_bytes(&self.state, spec).map_err(Error::InvalidSszState)?;

        *state.balances_mut() = List::try_from_iter(self.balances.iter().copied())
            .map_err(|_| Error::InvalidBalancesLength)?;

        if let Ok(inactivity_scores) = state.inactivity_scores_mut() {
            *inactivity_scores = List::try_from_iter(self.inactivity_scores.iter().copied())
                .map_err(|_| Error::InvalidBalancesLength)?;
        }

        *state.validators_mut() = List::try_from_iter(self.validators.iter().cloned())
            .map_err(|_| Error::InvalidBalancesLength)?;

        *state.historical_roots_mut() = List::try_from_iter(self.historical_roots.iter().copied())
            .map_err(|_| Error::InvalidBalancesLength)?;

        if let Ok(historical_summaries) = state.historical_summaries_mut() {
            *historical_summaries = List::try_from_iter(self.historical_summaries.iter().copied())
                .map_err(|_| Error::InvalidBalancesLength)?;
        }

        Ok(state)
    }

    /// Byte size of this instance
    pub fn size(&self) -> usize {
        self.state.len()
            + self.balances.len() * std::mem::size_of::<u64>()
            + self.inactivity_scores.len() * std::mem::size_of::<u64>()
            + self.validators.len() * std::mem::size_of::<Validator>()
            + self.historical_roots.len() * std::mem::size_of::<Hash256>()
            + self.historical_summaries.len() * std::mem::size_of::<HistoricalSummary>()
    }
}

impl HDiff {
    pub fn compute(
        source: &HDiffBuffer,
        target: &HDiffBuffer,
        config: &StoreConfig,
    ) -> Result<Self, Error> {
        let state_diff = BytesDiff::compute(&source.state, &target.state)?;
        let balances_diff = CompressedU64Diff::compute(&source.balances, &target.balances, config)?;
        let inactivity_scores_diff = CompressedU64Diff::compute(
            &source.inactivity_scores,
            &target.inactivity_scores,
            config,
        )?;
        let validators_diff =
            ValidatorsDiff::compute(&source.validators, &target.validators, config)?;
        let historical_roots =
            AppendOnlyDiff::compute(&source.historical_roots, &target.historical_roots)?;
        let historical_summaries =
            AppendOnlyDiff::compute(&source.historical_summaries, &target.historical_summaries)?;

        Ok(HDiff::V0(HDiffV0 {
            state_diff,
            balances_diff,
            inactivity_scores_diff,
            validators_diff,
            historical_roots,
            historical_summaries,
        }))
    }

    pub fn apply(&self, source: &mut HDiffBuffer, config: &StoreConfig) -> Result<(), Error> {
        let source_state = std::mem::take(&mut source.state);
        self.state_diff().apply(&source_state, &mut source.state)?;
        self.balances_diff().apply(&mut source.balances, config)?;
        self.inactivity_scores_diff()
            .apply(&mut source.inactivity_scores, config)?;
        self.validators_diff()
            .apply(&mut source.validators, config)?;
        self.historical_roots().apply(&mut source.historical_roots);
        self.historical_summaries()
            .apply(&mut source.historical_summaries);

        Ok(())
    }

    /// Byte size of this instance
    pub fn size(&self) -> usize {
        self.sizes().iter().sum()
    }

    pub fn sizes(&self) -> Vec<usize> {
        vec![
            self.state_diff().size(),
            self.balances_diff().size(),
            self.inactivity_scores_diff().size(),
            self.validators_diff().size(),
            self.historical_roots().size(),
            self.historical_summaries().size(),
        ]
    }
}

impl StoreItem for HDiff {
    fn db_column() -> DBColumn {
        DBColumn::BeaconStateDiff
    }

    fn as_store_bytes(&self) -> Vec<u8> {
        self.as_ssz_bytes()
    }

    fn from_store_bytes(bytes: &[u8]) -> Result<Self, crate::Error> {
        Ok(Self::from_ssz_bytes(bytes)?)
    }
}

impl BytesDiff {
    pub fn compute(source: &[u8], target: &[u8]) -> Result<Self, Error> {
        Self::compute_xdelta(source, target)
    }

    pub fn compute_xdelta(source_bytes: &[u8], target_bytes: &[u8]) -> Result<Self, Error> {
        // Start with a conservative estimate: diffs between similar beacon states are typically
        // much smaller than the states themselves. Use 1/4 of total size as initial guess,
        // with a minimum of 1024 bytes to handle tiny inputs.
        let total_len = source_bytes.len().saturating_add(target_bytes.len());
        let mut output_length = (total_len / 4).max(1024);
        let mut num_resizes = 0u32;
        loop {
            match xdelta3::encode_with_output_len(target_bytes, source_bytes, output_length as u32)
            {
                Ok(bytes) => {
                    metrics::observe(
                        &metrics::BEACON_HDIFF_BUFFER_COMPUTE_RESIZES,
                        num_resizes as f64,
                    );
                    return Ok(Self { bytes });
                }
                Err(xdelta3::Error::InsufficientOutputLength) => {
                    output_length = output_length.saturating_mul(2);
                    num_resizes = num_resizes.saturating_add(1);
                }
                Err(err) => {
                    return Err(Error::UnableToComputeDiff(err));
                }
            }
        }
    }

    pub fn apply(&self, source: &[u8], target: &mut Vec<u8>) -> Result<(), Error> {
        self.apply_xdelta(source, target)
    }

    pub fn apply_xdelta(&self, source: &[u8], target: &mut Vec<u8>) -> Result<(), Error> {
        // Parse the VCDIFF header to get the exact target window size, avoiding the
        // guess-and-double allocation loop. Falls back to a heuristic if parsing fails.
        let mut output_length = vcdiff_target_window_size(&self.bytes)
            .unwrap_or(((source.len() + self.bytes.len()) * 3) / 2);
        let mut num_resizes = 0;
        loop {
            match xdelta3::decode_with_output_len(&self.bytes, source, output_length as u32) {
                Ok(result_buffer) => {
                    *target = result_buffer;

                    metrics::observe(
                        &metrics::BEACON_HDIFF_BUFFER_APPLY_RESIZES,
                        num_resizes as f64,
                    );
                    return Ok(());
                }
                Err(xdelta3::Error::InsufficientOutputLength) => {
                    // Double the output buffer length and try again.
                    output_length *= 2;
                    num_resizes += 1;
                }
                Err(err) => {
                    return Err(Error::UnableToApplyDiff(err));
                }
            }
        }
    }

    /// Byte size of this instance
    pub fn size(&self) -> usize {
        self.bytes.len()
    }
}

impl CompressedU64Diff {
    pub fn compute(xs: &[u64], ys: &[u64], config: &StoreConfig) -> Result<Self, Error> {
        if xs.len() > ys.len() {
            return Err(Error::DiffDeletionsNotSupported);
        }

        let uncompressed_bytes: Vec<u8> = ys
            .iter()
            .enumerate()
            .flat_map(|(i, y)| {
                // Diff from 0 if the entry is new.
                let x = xs.get(i).copied().unwrap_or(0);
                y.wrapping_sub(x).to_be_bytes()
            })
            .collect();

        Ok(CompressedU64Diff {
            bytes: config
                .compress_bytes(&uncompressed_bytes)
                .map_err(Error::Compression)?,
        })
    }

    pub fn apply(&self, xs: &mut Vec<u64>, config: &StoreConfig) -> Result<(), Error> {
        // Decompress balances diff.
        let balances_diff_bytes = config
            .decompress_bytes(&self.bytes)
            .map_err(Error::Compression)?;

        for (i, diff_bytes) in balances_diff_bytes
            .chunks(u64::BITS as usize / 8)
            .enumerate()
        {
            let diff = diff_bytes
                .try_into()
                .map(u64::from_be_bytes)
                .map_err(|_| Error::BalancesIncompleteChunk)?;

            if let Some(x) = xs.get_mut(i) {
                *x = x.wrapping_add(diff);
            } else {
                xs.push(diff);
            }
        }

        Ok(())
    }

    /// Byte size of this instance
    pub fn size(&self) -> usize {
        self.bytes.len()
    }
}

impl ValidatorsDiff {
    pub fn compute(
        xs: &[Validator],
        ys: &[Validator],
        config: &StoreConfig,
    ) -> Result<Self, Error> {
        if xs.len() > ys.len() {
            return Err(Error::DiffDeletionsNotSupported);
        }

        let uncompressed_bytes = ys
            .iter()
            .enumerate()
            .filter_map(|(i, y)| {
                let validator_diff = if let Some(x) = xs.get(i) {
                    if y == x {
                        return None;
                    } else {
                        let pubkey_changed = y.pubkey != x.pubkey;
                        // Note: If researchers attempt to change the Validator container, go quickly to
                        // All Core Devs and push hard to add another List in the BeaconState instead.
                        Validator {
                            // The pubkey can be changed on index re-use
                            pubkey: if pubkey_changed {
                                y.pubkey
                            } else {
                                PublicKeyBytes::empty()
                            },
                            // withdrawal_credentials can be set to zero initially but can never be
                            // changed INTO zero. On index re-use it can be set to zero, but in that
                            // case the pubkey will also change.
                            withdrawal_credentials: if pubkey_changed
                                || y.withdrawal_credentials != x.withdrawal_credentials
                            {
                                y.withdrawal_credentials
                            } else {
                                Hash256::ZERO
                            },
                            // effective_balance can increase and decrease
                            effective_balance: y
                                .effective_balance
                                .wrapping_sub(x.effective_balance),
                            // slashed can only change from false into true. In an index re-use it can
                            // switch back to false, but in that case the pubkey will also change.
                            slashed: y.slashed,
                            // activation_eligibility_epoch can never be zero under any case. It's
                            // set to either FAR_FUTURE_EPOCH or get_current_epoch(state) + 1
                            activation_eligibility_epoch: if y.activation_eligibility_epoch
                                != x.activation_eligibility_epoch
                            {
                                y.activation_eligibility_epoch
                            } else {
                                Epoch::new(0)
                            },
                            // activation_epoch can never be zero under any case. It's
                            // set to either FAR_FUTURE_EPOCH or epoch + 1 + MAX_SEED_LOOKAHEAD
                            activation_epoch: if y.activation_epoch != x.activation_epoch {
                                y.activation_epoch
                            } else {
                                Epoch::new(0)
                            },
                            // exit_epoch can never be zero under any case. It's set to either
                            // FAR_FUTURE_EPOCH or > epoch + 1 + MAX_SEED_LOOKAHEAD
                            exit_epoch: if y.exit_epoch != x.exit_epoch {
                                y.exit_epoch
                            } else {
                                Epoch::new(0)
                            },
                            // withdrawable_epoch can never be zero under any case. It's set to
                            // either FAR_FUTURE_EPOCH or > epoch + 1 + MAX_SEED_LOOKAHEAD
                            withdrawable_epoch: if y.withdrawable_epoch != x.withdrawable_epoch {
                                y.withdrawable_epoch
                            } else {
                                Epoch::new(0)
                            },
                        }
                    }
                } else {
                    y.clone()
                };

                Some(ValidatorDiffEntry {
                    index: i as u64,
                    validator_diff,
                })
            })
            .flat_map(|v_diff| v_diff.as_ssz_bytes())
            .collect::<Vec<u8>>();

        Ok(Self {
            bytes: config
                .compress_bytes(&uncompressed_bytes)
                .map_err(Error::Compression)?,
        })
    }

    pub fn apply(&self, xs: &mut Vec<Validator>, config: &StoreConfig) -> Result<(), Error> {
        let validator_diff_bytes = config
            .decompress_bytes(&self.bytes)
            .map_err(Error::Compression)?;

        for diff_bytes in
            validator_diff_bytes.chunks(<ValidatorDiffEntry as Decode>::ssz_fixed_len())
        {
            let ValidatorDiffEntry {
                index,
                validator_diff: diff,
            } = ValidatorDiffEntry::from_ssz_bytes(diff_bytes)
                .map_err(|_| Error::BalancesIncompleteChunk)?;

            if let Some(x) = xs.get_mut(index as usize) {
                // Note: a pubkey change implies index re-use. In that case over-write
                // withdrawal_credentials and slashed inconditionally as their default values
                // are valid values.
                let pubkey_changed = diff.pubkey != *EMPTY_PUBKEY;
                if pubkey_changed {
                    x.pubkey = diff.pubkey;
                }
                if pubkey_changed || diff.withdrawal_credentials != Hash256::ZERO {
                    x.withdrawal_credentials = diff.withdrawal_credentials;
                }
                if diff.effective_balance != 0 {
                    x.effective_balance = x.effective_balance.wrapping_add(diff.effective_balance);
                }
                if pubkey_changed || diff.slashed {
                    x.slashed = diff.slashed;
                }
                if diff.activation_eligibility_epoch != Epoch::new(0) {
                    x.activation_eligibility_epoch = diff.activation_eligibility_epoch;
                }
                if diff.activation_epoch != Epoch::new(0) {
                    x.activation_epoch = diff.activation_epoch;
                }
                if diff.exit_epoch != Epoch::new(0) {
                    x.exit_epoch = diff.exit_epoch;
                }
                if diff.withdrawable_epoch != Epoch::new(0) {
                    x.withdrawable_epoch = diff.withdrawable_epoch;
                }
            } else {
                xs.push(diff)
            }
        }

        Ok(())
    }

    /// Byte size of this instance
    pub fn size(&self) -> usize {
        self.bytes.len()
    }
}

#[derive(Debug, Encode, Decode)]
struct ValidatorDiffEntry {
    index: u64,
    validator_diff: Validator,
}

impl<T: Decode + Encode + Copy> AppendOnlyDiff<T> {
    pub fn compute(xs: &[T], ys: &[T]) -> Result<Self, Error> {
        match xs.len().cmp(&ys.len()) {
            Ordering::Less => Ok(Self {
                values: ys.iter().skip(xs.len()).copied().collect(),
            }),
            // Don't even create an iterator for this common case
            Ordering::Equal => Ok(Self { values: vec![] }),
            Ordering::Greater => Err(Error::DiffDeletionsNotSupported),
        }
    }

    pub fn apply(&self, xs: &mut Vec<T>) {
        xs.extend(self.values.iter().copied());
    }

    /// Byte size of this instance
    pub fn size(&self) -> usize {
        self.values.len() * size_of::<T>()
    }
}

impl Default for HierarchyConfig {
    fn default() -> Self {
        HierarchyConfig {
            exponents: vec![5, 9, 11, 13, 16, 18, 21],
        }
    }
}

impl HierarchyConfig {
    pub fn to_moduli(&self) -> Result<HierarchyModuli, Error> {
        self.validate()?;
        let moduli = self.exponents.iter().map(|n| 1 << n).collect();
        Ok(HierarchyModuli { moduli })
    }

    pub fn validate(&self) -> Result<(), Error> {
        if !self.exponents.is_empty()
            && self
                .exponents
                .iter()
                .tuple_windows()
                .all(|(small, big)| small < big && *big < u64::BITS as u8)
        {
            Ok(())
        } else {
            Err(Error::InvalidHierarchy)
        }
    }

    pub fn exponent_for_slot(slot: Slot) -> u32 {
        slot.as_u64().trailing_zeros()
    }
}

impl HierarchyModuli {
    /// * `slot` - Slot of the storage strategy
    /// * `start_slot` - Slot before which states are not available. Initial snapshot point, which
    ///   may not be aligned to the hierarchy moduli values. Given an example of
    ///   exponents `[5,13,21]`, to reconstruct state at slot 3,000,003: if start = 3,000,002
    ///   layer 2 diff will point to the start snapshot instead of the layer 1 diff at
    ///   2998272.
    pub fn storage_strategy(&self, slot: Slot, start_slot: Slot) -> Result<StorageStrategy, Error> {
        match slot.cmp(&start_slot) {
            Ordering::Less => return Err(Error::LessThanStart(slot, start_slot)),
            Ordering::Equal => return Ok(StorageStrategy::Snapshot),
            Ordering::Greater => {} // continue
        }

        // last = full snapshot interval
        let last = self.moduli.last().copied().ok_or(Error::InvalidHierarchy)?;
        // first = most frequent diff layer, need to replay blocks from this layer
        let first = self
            .moduli
            .first()
            .copied()
            .ok_or(Error::InvalidHierarchy)?;

        if slot % last == 0 {
            return Ok(StorageStrategy::Snapshot);
        }

        Ok(self
            .moduli
            .iter()
            .rev()
            .tuple_windows()
            .find_map(|(&n_big, &n_small)| {
                if slot % n_small == 0 {
                    // Diff from the previous layer.
                    let from = slot / n_big * n_big;
                    // Or from start point
                    let from = std::cmp::max(from, start_slot);
                    Some(StorageStrategy::DiffFrom(from))
                } else {
                    // Keep trying with next layer
                    None
                }
            })
            // Exhausted layers, need to replay from most frequent layer
            .unwrap_or_else(|| {
                let from = slot / first * first;
                // Or from start point
                let from = std::cmp::max(from, start_slot);
                StorageStrategy::ReplayFrom(from)
            }))
    }

    /// Return the smallest slot greater than or equal to `slot` at which a full snapshot should
    /// be stored.
    pub fn next_snapshot_slot(&self, slot: Slot) -> Result<Slot, Error> {
        let last = self.moduli.last().copied().ok_or(Error::InvalidHierarchy)?;
        if slot % last == 0 {
            Ok(slot)
        } else {
            Ok((slot / last + 1) * last)
        }
    }

    /// Return `true` if the database ops for this slot should be committed immediately.
    ///
    /// This is the case for all diffs aside from the ones in the leaf layer. To store a diff
    /// might require loading the state at the previous layer, in which case the diff for that
    /// layer must already have been stored.
    ///
    /// In future we may be able to handle this differently (with proper transaction semantics
    /// rather than write batches).
    pub fn should_commit_immediately(&self, slot: Slot) -> Result<bool, Error> {
        // If there's only 1 layer of snapshots, then commit only when writing a snapshot.
        self.moduli.get(1).map_or_else(
            || Ok(slot == self.next_snapshot_slot(slot)?),
            |second_layer_moduli| Ok(slot % *second_layer_moduli == 0),
        )
    }

    /// For each layer, returns the closest diff less than or equal to `slot`.
    pub fn closest_layer_points(&self, slot: Slot, start_slot: Slot) -> Vec<Slot> {
        let mut layers = self
            .moduli
            .iter()
            .map(|&n| {
                let from = slot / n * n;
                // Or from start point
                std::cmp::max(from, start_slot)
            })
            .collect::<Vec<_>>();

        // Remove duplication caused by the capping at `start_slot` (multiple
        // layers may have the same slot equal to `start_slot`), or shared multiples (a slot that is
        // a multiple of 2**n will also be a multiple of 2**m for all m < n).
        layers.dedup();

        layers
    }
}

impl StorageStrategy {
    /// For the state stored with this `StorageStrategy` at `slot`, return the range of slots which
    /// should be checked for ancestor states in the historic state cache.
    ///
    /// The idea is that for states which need to be built by replaying blocks we should scan
    /// for any viable ancestor state between their `from` slot and `slot`. If we find such a
    /// state it will save us from the slow reconstruction of the `from` state using diffs.
    ///
    /// Similarly for `DiffFrom` and `Snapshot` states, loading the prior state and replaying 1
    /// block is often going to be faster than loading and applying diffs/snapshots, so we may as
    /// well check the cache for that 1 slot prior (in case the caller is iterating sequentially).
    pub fn replay_from_range(
        &self,
        slot: Slot,
    ) -> std::iter::Map<RangeInclusive<u64>, fn(u64) -> Slot> {
        match self {
            Self::ReplayFrom(from) => from.as_u64()..=slot.as_u64(),
            Self::Snapshot | Self::DiffFrom(_) => {
                if slot > 0 {
                    (slot - 1).as_u64()..=slot.as_u64()
                } else {
                    slot.as_u64()..=slot.as_u64()
                }
            }
        }
        .map(Slot::from)
    }

    /// Returns the slot that storage_strategy points to.
    pub fn diff_base_slot(&self) -> Option<Slot> {
        match self {
            Self::ReplayFrom(from) => Some(*from),
            Self::DiffFrom(from) => Some(*from),
            Self::Snapshot => None,
        }
    }

    pub fn is_replay_from(&self) -> bool {
        matches!(self, Self::ReplayFrom(_))
    }

    pub fn is_diff_from(&self) -> bool {
        matches!(self, Self::DiffFrom(_))
    }

    pub fn is_snapshot(&self) -> bool {
        matches!(self, Self::Snapshot)
    }
}

/// Read a VCDIFF variable-length integer (RFC 3284 Section 2).
///
/// Each byte has 7 data bits (MSB-first) and 1 continuation bit (bit 7).
/// The last byte has bit 7 = 0.
fn read_vcdiff_integer(bytes: &[u8], offset: &mut usize) -> Option<usize> {
    let mut result: usize = 0;
    loop {
        let byte = *bytes.get(*offset)?;
        *offset += 1;
        result = result.checked_shl(7)?.checked_add((byte & 0x7F) as usize)?;
        if byte & 0x80 == 0 {
            return Some(result);
        }
    }
}

/// Parse the target window size from a VCDIFF-encoded delta (RFC 3284).
///
/// The VCDIFF format stores the target window length in the window header,
/// which lets us allocate the exact output buffer size without guessing.
fn vcdiff_target_window_size(delta: &[u8]) -> Option<usize> {
    const VCDIFF_MAGIC: [u8; 4] = [0xD6, 0xC3, 0xC4, 0x00];

    if delta.len() < 5 || delta[..4] != VCDIFF_MAGIC {
        return None;
    }

    let mut offset = 4;
    let hdr_indicator = *delta.get(offset)?;
    offset += 1;

    // Skip secondary compressor ID if VCD_DECOMPRESS is set
    if hdr_indicator & 0x01 != 0 {
        offset += 1;
    }
    // Skip code table data if VCD_CODETABLE is set
    if hdr_indicator & 0x02 != 0 {
        let code_table_len = read_vcdiff_integer(delta, &mut offset)?;
        offset += code_table_len;
    }
    // Skip application data if VCD_APPHEADER is set
    if hdr_indicator & 0x04 != 0 {
        let app_data_len = read_vcdiff_integer(delta, &mut offset)?;
        offset += app_data_len;
    }

    // Window section
    let win_indicator = *delta.get(offset)?;
    offset += 1;

    // Skip source/target segment info if VCD_SOURCE or VCD_TARGET is set
    if win_indicator & 0x03 != 0 {
        let _segment_size = read_vcdiff_integer(delta, &mut offset)?;
        let _segment_position = read_vcdiff_integer(delta, &mut offset)?;
    }

    // Delta encoding length (skip)
    let _delta_encoding_len = read_vcdiff_integer(delta, &mut offset)?;

    // Target window length — the exact output size
    read_vcdiff_integer(delta, &mut offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng, rng, rngs::SmallRng};

    #[test]
    fn default_storage_strategy() {
        let config = HierarchyConfig::default();
        config.validate().unwrap();
        let sslot = Slot::new(0);

        let moduli = config.to_moduli().unwrap();

        // Full snapshots at multiples of 2^21.
        let snapshot_freq = Slot::new(1 << 21);
        assert_eq!(
            moduli.storage_strategy(Slot::new(0), sslot).unwrap(),
            StorageStrategy::Snapshot
        );
        assert_eq!(
            moduli.storage_strategy(snapshot_freq, sslot).unwrap(),
            StorageStrategy::Snapshot
        );
        assert_eq!(
            moduli.storage_strategy(snapshot_freq * 3, sslot).unwrap(),
            StorageStrategy::Snapshot
        );

        // Diffs should be from the previous layer (the snapshot in this case), and not the previous diff in the same layer.
        let first_layer = Slot::new(1 << 18);
        assert_eq!(
            moduli.storage_strategy(first_layer * 2, sslot).unwrap(),
            StorageStrategy::DiffFrom(Slot::new(0))
        );

        let replay_strategy_slot = first_layer + 1;
        assert_eq!(
            moduli
                .storage_strategy(replay_strategy_slot, sslot)
                .unwrap(),
            StorageStrategy::ReplayFrom(first_layer)
        );
    }

    #[test]
    fn next_snapshot_slot() {
        let config = HierarchyConfig::default();
        config.validate().unwrap();

        let moduli = config.to_moduli().unwrap();
        let snapshot_freq = Slot::new(1 << 21);

        assert_eq!(
            moduli.next_snapshot_slot(snapshot_freq).unwrap(),
            snapshot_freq
        );
        assert_eq!(
            moduli.next_snapshot_slot(snapshot_freq + 1).unwrap(),
            snapshot_freq * 2
        );
        assert_eq!(
            moduli.next_snapshot_slot(snapshot_freq * 2 - 1).unwrap(),
            snapshot_freq * 2
        );
        assert_eq!(
            moduli.next_snapshot_slot(snapshot_freq * 2).unwrap(),
            snapshot_freq * 2
        );
        assert_eq!(
            moduli.next_snapshot_slot(snapshot_freq * 100).unwrap(),
            snapshot_freq * 100
        );
    }

    #[test]
    fn compressed_u64_vs_bytes_diff() {
        let x_values = vec![99u64, 55, 123, 6834857, 0, 12];
        let y_values = vec![98u64, 55, 312, 1, 1, 2, 4, 5];
        let config = &StoreConfig::default();

        let to_bytes =
            |nums: &[u64]| -> Vec<u8> { nums.iter().flat_map(|x| x.to_be_bytes()).collect() };

        let x_bytes = to_bytes(&x_values);
        let y_bytes = to_bytes(&y_values);

        let u64_diff = CompressedU64Diff::compute(&x_values, &y_values, config).unwrap();

        let mut y_from_u64_diff = x_values;
        u64_diff.apply(&mut y_from_u64_diff, config).unwrap();

        assert_eq!(y_values, y_from_u64_diff);

        let bytes_diff = BytesDiff::compute(&x_bytes, &y_bytes).unwrap();

        let mut y_from_bytes = vec![];
        bytes_diff.apply(&x_bytes, &mut y_from_bytes).unwrap();

        assert_eq!(y_bytes, y_from_bytes);

        // U64 diff wins by more than a factor of 3
        assert!(u64_diff.bytes.len() < 3 * bytes_diff.bytes.len());
    }

    #[test]
    fn compressed_validators_diff() {
        assert_eq!(<ValidatorDiffEntry as Decode>::ssz_fixed_len(), 129);

        let mut rng = rng();
        let config = &StoreConfig::default();
        let xs = (0..10)
            .map(|_| rand_validator(&mut rng))
            .collect::<Vec<_>>();
        let mut ys = xs.clone();
        ys[5] = rand_validator(&mut rng);
        ys.push(rand_validator(&mut rng));
        let diff = ValidatorsDiff::compute(&xs, &ys, config).unwrap();

        let mut xs_out = xs.clone();
        diff.apply(&mut xs_out, config).unwrap();
        assert_eq!(xs_out, ys);
    }

    fn rand_validator(mut rng: impl Rng) -> Validator {
        let mut pubkey = [0u8; 48];
        rng.fill_bytes(&mut pubkey);
        let withdrawal_credentials: [u8; 32] = rng.random();

        Validator {
            pubkey: PublicKeyBytes::from_ssz_bytes(&pubkey).unwrap(),
            withdrawal_credentials: withdrawal_credentials.into(),
            slashed: false,
            effective_balance: 32_000_000_000,
            activation_eligibility_epoch: Epoch::max_value(),
            activation_epoch: Epoch::max_value(),
            exit_epoch: Epoch::max_value(),
            withdrawable_epoch: Epoch::max_value(),
        }
    }

    // This test checks that the hdiff algorithm doesn't accidentally change between releases.
    // If it does, we need to ensure appropriate backwards compatibility measures are implemented
    // before this test is updated.
    #[test]
    fn hdiff_version_stability() {
        let mut rng = SmallRng::seed_from_u64(0xffeeccdd00aa);

        let pre_balances = vec![32_000_000_000, 16_000_000_000, 0];
        let post_balances = vec![31_000_000_000, 17_000_000, 0, 0];

        let pre_inactivity_scores = vec![1, 1, 1];
        let post_inactivity_scores = vec![0, 0, 0, 1];

        let pre_validators = (0..3).map(|_| rand_validator(&mut rng)).collect::<Vec<_>>();
        let post_validators = pre_validators.clone();

        let pre_historical_roots = vec![Hash256::repeat_byte(0xff)];
        let post_historical_roots = vec![Hash256::repeat_byte(0xff), Hash256::repeat_byte(0xee)];

        let pre_historical_summaries = vec![HistoricalSummary::default()];
        let post_historical_summaries = pre_historical_summaries.clone();

        let pre_buffer = HDiffBuffer {
            state: vec![0, 1, 2, 3, 3, 2, 1, 0],
            balances: pre_balances,
            inactivity_scores: pre_inactivity_scores,
            validators: pre_validators,
            historical_roots: pre_historical_roots,
            historical_summaries: pre_historical_summaries,
        };
        let post_buffer = HDiffBuffer {
            state: vec![0, 1, 3, 2, 2, 3, 1, 1],
            balances: post_balances,
            inactivity_scores: post_inactivity_scores,
            validators: post_validators,
            historical_roots: post_historical_roots,
            historical_summaries: post_historical_summaries,
        };

        let config = StoreConfig::default();
        let hdiff = HDiff::compute(&pre_buffer, &post_buffer, &config).unwrap();
        let hdiff_ssz = hdiff.as_ssz_bytes();

        // First byte should match enum version.
        assert_eq!(hdiff_ssz[0], 0);

        // Should roundtrip.
        assert_eq!(HDiff::from_ssz_bytes(&hdiff_ssz).unwrap(), hdiff);

        // Should roundtrip as V0 with enum selector stripped.
        assert_eq!(
            HDiff::V0(HDiffV0::from_ssz_bytes(&hdiff_ssz[1..]).unwrap()),
            hdiff
        );

        assert_eq!(
            hdiff_ssz,
            vec![
                0u8, 24, 0, 0, 0, 49, 0, 0, 0, 85, 0, 0, 0, 114, 0, 0, 0, 127, 0, 0, 0, 163, 0, 0,
                0, 4, 0, 0, 0, 214, 195, 196, 0, 0, 0, 14, 8, 0, 8, 1, 0, 0, 1, 3, 2, 2, 3, 1, 1,
                9, 4, 0, 0, 0, 40, 181, 47, 253, 0, 72, 189, 0, 0, 136, 255, 255, 255, 255, 196,
                101, 54, 0, 255, 255, 255, 252, 71, 86, 198, 64, 0, 1, 0, 59, 176, 4, 4, 0, 0, 0,
                40, 181, 47, 253, 0, 72, 133, 0, 0, 80, 255, 255, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 10,
                192, 2, 4, 0, 0, 0, 40, 181, 47, 253, 32, 0, 1, 0, 0, 4, 0, 0, 0, 238, 238, 238,
                238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238,
                238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 4, 0, 0, 0
            ]
        );
    }

    // Test that the diffs and snapshots required for storage of split states are retained in the
    // hot DB as the split slot advances, if we begin from an initial configuration where this
    // invariant holds.
    fn test_slots_retained_invariant(hierarchy: HierarchyModuli, start_slot: u64, epoch_jump: u64) {
        let start_slot = Slot::new(start_slot);
        let mut finalized_slot = start_slot;

        // Initially we have just one snapshot stored at the `start_slot`. This is what checkpoint
        // sync sets up (or the V24 migration).
        let mut retained_slots = vec![finalized_slot];

        // Iterate until we've reached two snapshots in the future.
        let stop_at = hierarchy
            .next_snapshot_slot(hierarchy.next_snapshot_slot(start_slot).unwrap() + 1)
            .unwrap();

        while finalized_slot <= stop_at {
            // Jump multiple epocsh at a time because inter-epoch states are not interesting and
            // would take too long to iterate over.
            let new_finalized_slot = finalized_slot + 32 * epoch_jump;

            let new_retained_slots = hierarchy.closest_layer_points(new_finalized_slot, start_slot);

            for slot in &new_retained_slots {
                // All new retained slots must either be already stored prior to the old finalized
                // slot, OR newer than the finalized slot (i.e. stored in the hot DB as part of
                // regular state storage).
                assert!(retained_slots.contains(slot) || *slot >= finalized_slot);
            }

            retained_slots = new_retained_slots;
            finalized_slot = new_finalized_slot;
        }
    }

    #[test]
    fn slots_retained_invariant() {
        let cases = [
            // Default hierarchy with a start_slot between the 2^13 and 2^16 layers.
            (
                HierarchyConfig::default().to_moduli().unwrap(),
                2 * (1 << 14) - 5 * 32,
                1,
            ),
            // Default hierarchy with a start_slot between the 2^13 and 2^16 layers, with 8 epochs
            // finalizing at a time (should not make any difference).
            (
                HierarchyConfig::default().to_moduli().unwrap(),
                2 * (1 << 14) - 5 * 32,
                8,
            ),
            // Very dense hierarchy config.
            (
                HierarchyConfig::from_str("5,7")
                    .unwrap()
                    .to_moduli()
                    .unwrap(),
                32,
                1,
            ),
            // Very dense hierarchy config that skips a whole snapshot on its first finalization.
            (
                HierarchyConfig::from_str("5,7")
                    .unwrap()
                    .to_moduli()
                    .unwrap(),
                32,
                1 << 7,
            ),
        ];

        for (hierarchy, start_slot, epoch_jump) in cases {
            test_slots_retained_invariant(hierarchy, start_slot, epoch_jump);
        }
    }

    #[test]
    fn closest_layer_points_unique() {
        let hierarchy = HierarchyConfig::default().to_moduli().unwrap();

        let start_slot = Slot::new(0);
        let end_slot = hierarchy.next_snapshot_slot(Slot::new(1)).unwrap();

        for slot in (0..end_slot.as_u64()).map(Slot::new) {
            let closest_layer_points = hierarchy.closest_layer_points(slot, start_slot);
            assert!(closest_layer_points.is_sorted_by(|a, b| a > b));
        }
    }

    // ── HierarchyConfig parsing (FromStr) ──

    #[test]
    fn hierarchy_config_from_str_valid() {
        let config = HierarchyConfig::from_str("5,13,21").unwrap();
        assert_eq!(config.exponents, vec![5, 13, 21]);
    }

    #[test]
    fn hierarchy_config_from_str_single() {
        let config = HierarchyConfig::from_str("10").unwrap();
        assert_eq!(config.exponents, vec![10]);
    }

    #[test]
    fn hierarchy_config_from_str_two_layers() {
        let config = HierarchyConfig::from_str("5,7").unwrap();
        assert_eq!(config.exponents, vec![5, 7]);
    }

    #[test]
    fn hierarchy_config_from_str_empty_error() {
        assert!(HierarchyConfig::from_str("").is_err());
    }

    #[test]
    fn hierarchy_config_from_str_not_ascending_error() {
        assert!(HierarchyConfig::from_str("5,3,21").is_err());
    }

    #[test]
    fn hierarchy_config_from_str_equal_values_error() {
        assert!(HierarchyConfig::from_str("5,5,21").is_err());
    }

    #[test]
    fn hierarchy_config_from_str_descending_error() {
        assert!(HierarchyConfig::from_str("21,13,5").is_err());
    }

    #[test]
    fn hierarchy_config_from_str_non_numeric_error() {
        assert!(HierarchyConfig::from_str("abc").is_err());
    }

    #[test]
    fn hierarchy_config_display_roundtrip() {
        let config = HierarchyConfig::from_str("5,13,21").unwrap();
        assert_eq!(format!("{}", config), "5,13,21");
        let reparsed = HierarchyConfig::from_str(&format!("{}", config)).unwrap();
        assert_eq!(reparsed.exponents, config.exponents);
    }

    // ── HierarchyConfig::validate ──

    #[test]
    fn hierarchy_config_validate_default_ok() {
        assert!(HierarchyConfig::default().validate().is_ok());
    }

    #[test]
    fn hierarchy_config_validate_empty_error() {
        let config = HierarchyConfig { exponents: vec![] };
        assert!(config.validate().is_err());
    }

    #[test]
    fn hierarchy_config_validate_non_ascending_error() {
        let config = HierarchyConfig {
            exponents: vec![10, 5],
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn hierarchy_config_validate_too_large_exponent_error() {
        let config = HierarchyConfig {
            exponents: vec![5, 64], // 64 >= u64::BITS
        };
        assert!(config.validate().is_err());
    }

    // ── HierarchyConfig::exponent_for_slot ──

    #[test]
    fn exponent_for_slot_powers_of_two() {
        assert_eq!(HierarchyConfig::exponent_for_slot(Slot::new(1)), 0);
        assert_eq!(HierarchyConfig::exponent_for_slot(Slot::new(2)), 1);
        assert_eq!(HierarchyConfig::exponent_for_slot(Slot::new(4)), 2);
        assert_eq!(HierarchyConfig::exponent_for_slot(Slot::new(32)), 5);
        assert_eq!(HierarchyConfig::exponent_for_slot(Slot::new(1 << 13)), 13);
        assert_eq!(HierarchyConfig::exponent_for_slot(Slot::new(1 << 21)), 21);
    }

    #[test]
    fn exponent_for_slot_zero() {
        // trailing_zeros of 0 is 64
        assert_eq!(HierarchyConfig::exponent_for_slot(Slot::new(0)), 64);
    }

    #[test]
    fn exponent_for_slot_odd() {
        // Odd numbers have trailing_zeros = 0
        assert_eq!(HierarchyConfig::exponent_for_slot(Slot::new(3)), 0);
        assert_eq!(HierarchyConfig::exponent_for_slot(Slot::new(7)), 0);
        assert_eq!(HierarchyConfig::exponent_for_slot(Slot::new(33)), 0);
    }

    #[test]
    fn exponent_for_slot_mixed() {
        // 12 = 0b1100, trailing_zeros = 2
        assert_eq!(HierarchyConfig::exponent_for_slot(Slot::new(12)), 2);
        // 48 = 0b110000, trailing_zeros = 4
        assert_eq!(HierarchyConfig::exponent_for_slot(Slot::new(48)), 4);
    }

    // ── HierarchyModuli::should_commit_immediately ──

    #[test]
    fn should_commit_immediately_snapshot_layer() {
        let moduli = HierarchyConfig::from_str("5,13,21")
            .unwrap()
            .to_moduli()
            .unwrap();
        // Slot at full snapshot interval (2^21) should commit immediately
        assert!(
            moduli
                .should_commit_immediately(Slot::new(1 << 21))
                .unwrap()
        );
    }

    #[test]
    fn should_commit_immediately_second_layer() {
        let moduli = HierarchyConfig::from_str("5,13,21")
            .unwrap()
            .to_moduli()
            .unwrap();
        // Slot at 2^13 boundary should commit (it's the second layer)
        assert!(
            moduli
                .should_commit_immediately(Slot::new(1 << 13))
                .unwrap()
        );
    }

    #[test]
    fn should_commit_immediately_leaf_layer() {
        let moduli = HierarchyConfig::from_str("5,13,21")
            .unwrap()
            .to_moduli()
            .unwrap();
        // Slot at 2^5 boundary (leaf layer) should NOT commit immediately
        assert!(!moduli.should_commit_immediately(Slot::new(32)).unwrap());
    }

    #[test]
    fn should_commit_immediately_non_aligned_slot() {
        let moduli = HierarchyConfig::from_str("5,13,21")
            .unwrap()
            .to_moduli()
            .unwrap();
        assert!(!moduli.should_commit_immediately(Slot::new(33)).unwrap());
    }

    #[test]
    fn should_commit_immediately_single_layer() {
        let moduli = HierarchyConfig::from_str("10")
            .unwrap()
            .to_moduli()
            .unwrap();
        // With single layer, commit only at snapshot boundaries
        assert!(
            moduli
                .should_commit_immediately(Slot::new(1 << 10))
                .unwrap()
        );
        assert!(!moduli.should_commit_immediately(Slot::new(1)).unwrap());
    }

    // ── storage_strategy edge cases ──

    #[test]
    fn storage_strategy_slot_less_than_start_error() {
        let moduli = HierarchyConfig::default().to_moduli().unwrap();
        let result = moduli.storage_strategy(Slot::new(5), Slot::new(10));
        assert!(result.is_err());
    }

    #[test]
    fn storage_strategy_equal_to_start_is_snapshot() {
        let moduli = HierarchyConfig::default().to_moduli().unwrap();
        let result = moduli
            .storage_strategy(Slot::new(100), Slot::new(100))
            .unwrap();
        assert_eq!(result, StorageStrategy::Snapshot);
    }

    #[test]
    fn storage_strategy_start_slot_affects_diff_from() {
        let moduli = HierarchyConfig::from_str("5,13")
            .unwrap()
            .to_moduli()
            .unwrap();
        // With start_slot at 8000, a diff should reference start_slot when it's
        // closer than the natural grid point
        let start = Slot::new(8000);
        let slot = Slot::new(1 << 13); // 8192 — at layer boundary
        let result = moduli.storage_strategy(slot, start).unwrap();
        // Diff should be from max(8192/8192*8192=0 → but start=8000, so from 8000)
        // Actually slot 8192 % 8192 == 0, so this is a snapshot
        assert_eq!(result, StorageStrategy::Snapshot);

        // Slot just past the leaf boundary after start
        let slot = Slot::new(8032); // 8032 % 32 == 0
        let result = moduli.storage_strategy(slot, start).unwrap();
        // Natural diff_from = 8032/8192*8192 = 0, clamped to start=8000
        assert_eq!(result, StorageStrategy::DiffFrom(start));
    }

    // ── next_snapshot_slot edge cases ──

    #[test]
    fn next_snapshot_slot_zero() {
        let moduli = HierarchyConfig::default().to_moduli().unwrap();
        assert_eq!(
            moduli.next_snapshot_slot(Slot::new(0)).unwrap(),
            Slot::new(0)
        );
    }

    // ── VCDIFF header parsing ──

    #[test]
    fn vcdiff_target_size_from_known_delta() {
        // Encode a known source → target pair and verify the parsed size matches.
        let source = vec![1u8, 2, 3, 4, 5, 6, 7];
        let target = vec![1u8, 2, 3, 4, 5, 6, 7];
        let delta = xdelta3::encode(&target, &source).unwrap();
        let parsed_size = vcdiff_target_window_size(&delta);
        assert_eq!(parsed_size, Some(target.len()));
    }

    #[test]
    fn vcdiff_target_size_different_lengths() {
        // Target is larger than source.
        let source = vec![0u8; 100];
        let target = vec![42u8; 500];
        let delta = xdelta3::encode(&target, &source).unwrap();
        assert_eq!(vcdiff_target_window_size(&delta), Some(500));

        // Target is smaller than source.
        let source = vec![0u8; 500];
        let target = vec![42u8; 100];
        let delta = xdelta3::encode(&target, &source).unwrap();
        assert_eq!(vcdiff_target_window_size(&delta), Some(100));
    }

    #[test]
    fn vcdiff_target_size_large_similar_data() {
        // Simulate beacon state-like data: large, mostly similar.
        let mut rng = SmallRng::seed_from_u64(42);
        let source: Vec<u8> = (0..10_000).map(|_| rng.random()).collect();
        let mut target = source.clone();
        // Modify a few bytes.
        for i in (0..target.len()).step_by(1000) {
            target[i] = target[i].wrapping_add(1);
        }
        let delta = xdelta3::encode(&target, &source).unwrap();
        assert_eq!(vcdiff_target_window_size(&delta), Some(10_000));
    }

    #[test]
    fn vcdiff_target_size_empty_source() {
        let source = vec![];
        let target = vec![1u8, 2, 3];
        let delta = xdelta3::encode_with_output_len(&target, &source, 1024).unwrap();
        assert_eq!(vcdiff_target_window_size(&delta), Some(3));
    }

    #[test]
    fn vcdiff_target_size_invalid_input() {
        assert_eq!(vcdiff_target_window_size(&[]), None);
        assert_eq!(vcdiff_target_window_size(&[0, 0, 0, 0, 0]), None);
        assert_eq!(vcdiff_target_window_size(&[0xD6, 0xC3]), None);
    }

    #[test]
    fn vcdiff_roundtrip_with_bytes_diff() {
        // Verify that BytesDiff::apply produces the correct result when using
        // the VCDIFF header parser for buffer allocation.
        let mut rng = SmallRng::seed_from_u64(99);
        let source: Vec<u8> = (0..5_000).map(|_| rng.random()).collect();
        let mut target_expected = source.clone();
        for i in (0..target_expected.len()).step_by(500) {
            target_expected[i] = target_expected[i].wrapping_add(7);
        }

        let diff = BytesDiff::compute(&source, &target_expected).unwrap();
        let mut target_actual = Vec::new();
        diff.apply(&source, &mut target_actual).unwrap();
        assert_eq!(target_actual, target_expected);
    }
}
