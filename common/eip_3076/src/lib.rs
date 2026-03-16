use serde::{Deserialize, Serialize};
use std::cmp::max;
use std::collections::{HashMap, HashSet};
#[cfg(feature = "json")]
use std::io;
use types::{Epoch, Hash256, PublicKeyBytes, Slot};

#[derive(Debug)]
pub enum Error {
    MaxInconsistent,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "arbitrary-fuzz", derive(arbitrary::Arbitrary))]
pub struct InterchangeMetadata {
    #[serde(with = "serde_utils::quoted_u64::require_quotes")]
    pub interchange_format_version: u64,
    pub genesis_validators_root: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "arbitrary-fuzz", derive(arbitrary::Arbitrary))]
pub struct InterchangeData {
    pub pubkey: PublicKeyBytes,
    pub signed_blocks: Vec<SignedBlock>,
    pub signed_attestations: Vec<SignedAttestation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "arbitrary-fuzz", derive(arbitrary::Arbitrary))]
pub struct SignedBlock {
    #[serde(with = "serde_utils::quoted_u64::require_quotes")]
    pub slot: Slot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signing_root: Option<Hash256>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "arbitrary-fuzz", derive(arbitrary::Arbitrary))]
pub struct SignedAttestation {
    #[serde(with = "serde_utils::quoted_u64::require_quotes")]
    pub source_epoch: Epoch,
    #[serde(with = "serde_utils::quoted_u64::require_quotes")]
    pub target_epoch: Epoch,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signing_root: Option<Hash256>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "arbitrary-fuzz", derive(arbitrary::Arbitrary))]
pub struct Interchange {
    pub metadata: InterchangeMetadata,
    pub data: Vec<InterchangeData>,
}

impl Interchange {
    #[cfg(feature = "json")]
    pub fn from_json_str(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[cfg(feature = "json")]
    pub fn from_json_reader(mut reader: impl std::io::Read) -> Result<Self, io::Error> {
        // We read the entire file into memory first, as this is *a lot* faster than using
        // `serde_json::from_reader`. See https://github.com/serde-rs/json/issues/160
        let mut json_str = String::new();
        reader.read_to_string(&mut json_str)?;
        Ok(Interchange::from_json_str(&json_str)?)
    }

    #[cfg(feature = "json")]
    pub fn write_to(&self, writer: impl std::io::Write) -> Result<(), serde_json::Error> {
        serde_json::to_writer(writer, self)
    }

    /// Do these two `Interchange`s contain the same data (ignoring ordering)?
    pub fn equiv(&self, other: &Self) -> bool {
        let self_set = self.data.iter().collect::<HashSet<_>>();
        let other_set = other.data.iter().collect::<HashSet<_>>();
        self.metadata == other.metadata && self_set == other_set
    }

    /// The number of entries in `data`.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Is the `data` part of the interchange completely empty?
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Minify an interchange by constructing a synthetic block & attestation for each validator.
    pub fn minify(&self) -> Result<Self, Error> {
        // Map from pubkey to optional max block and max attestation.
        let mut validator_data =
            HashMap::<PublicKeyBytes, (Option<SignedBlock>, Option<SignedAttestation>)>::new();

        for data in self.data.iter() {
            // Existing maximum attestation and maximum block.
            let (max_block, max_attestation) = validator_data
                .entry(data.pubkey)
                .or_insert_with(|| (None, None));

            // Find maximum source and target epochs.
            let max_source_epoch = data
                .signed_attestations
                .iter()
                .map(|attestation| attestation.source_epoch)
                .max();
            let max_target_epoch = data
                .signed_attestations
                .iter()
                .map(|attestation| attestation.target_epoch)
                .max();

            match (max_source_epoch, max_target_epoch) {
                (Some(source_epoch), Some(target_epoch)) => {
                    if let Some(prev_max) = max_attestation {
                        prev_max.source_epoch = max(prev_max.source_epoch, source_epoch);
                        prev_max.target_epoch = max(prev_max.target_epoch, target_epoch);
                    } else {
                        *max_attestation = Some(SignedAttestation {
                            source_epoch,
                            target_epoch,
                            signing_root: None,
                        });
                    }
                }
                (None, None) => {}
                _ => return Err(Error::MaxInconsistent),
            };

            // Find maximum block slot.
            let max_block_slot = data.signed_blocks.iter().map(|block| block.slot).max();

            if let Some(max_slot) = max_block_slot {
                if let Some(prev_max) = max_block {
                    prev_max.slot = max(prev_max.slot, max_slot);
                } else {
                    *max_block = Some(SignedBlock {
                        slot: max_slot,
                        signing_root: None,
                    });
                }
            }
        }

        let data = validator_data
            .into_iter()
            .map(|(pubkey, (maybe_block, maybe_att))| InterchangeData {
                pubkey,
                signed_blocks: maybe_block.into_iter().collect(),
                signed_attestations: maybe_att.into_iter().collect(),
            })
            .collect();

        Ok(Self {
            metadata: self.metadata.clone(),
            data,
        })
    }
}

#[cfg(feature = "json")]
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::tempdir;
    use types::FixedBytesExtended;

    fn get_interchange() -> Interchange {
        Interchange {
            metadata: InterchangeMetadata {
                interchange_format_version: 5,
                genesis_validators_root: Hash256::from_low_u64_be(555),
            },
            data: vec![
                InterchangeData {
                    pubkey: PublicKeyBytes::deserialize(&[1u8; 48]).unwrap(),
                    signed_blocks: vec![SignedBlock {
                        slot: Slot::new(100),
                        signing_root: Some(Hash256::from_low_u64_be(1)),
                    }],
                    signed_attestations: vec![SignedAttestation {
                        source_epoch: Epoch::new(0),
                        target_epoch: Epoch::new(5),
                        signing_root: Some(Hash256::from_low_u64_be(2)),
                    }],
                },
                InterchangeData {
                    pubkey: PublicKeyBytes::deserialize(&[2u8; 48]).unwrap(),
                    signed_blocks: vec![],
                    signed_attestations: vec![],
                },
            ],
        }
    }

    #[test]
    fn test_roundtrip() {
        let temp_dir = tempdir().unwrap();
        let file_path = temp_dir.path().join("interchange.json");

        let interchange = get_interchange();

        let mut file = File::create(&file_path).unwrap();
        interchange.write_to(&mut file).unwrap();

        let file = File::open(&file_path).unwrap();
        let from_file = Interchange::from_json_reader(file).unwrap();

        assert_eq!(interchange, from_file);
    }

    #[test]
    fn test_empty_roundtrip() {
        let temp_dir = tempdir().unwrap();
        let file_path = temp_dir.path().join("empty.json");

        let empty = Interchange {
            metadata: InterchangeMetadata {
                interchange_format_version: 5,
                genesis_validators_root: Hash256::zero(),
            },
            data: vec![],
        };

        let mut file = File::create(&file_path).unwrap();
        empty.write_to(&mut file).unwrap();

        let file = File::open(&file_path).unwrap();
        let from_file = Interchange::from_json_reader(file).unwrap();

        assert_eq!(empty, from_file);
    }

    #[test]
    fn test_minify_roundtrip() {
        let interchange = get_interchange();

        let minified = interchange.minify().unwrap();

        let temp_dir = tempdir().unwrap();
        let file_path = temp_dir.path().join("minified.json");

        let mut file = File::create(&file_path).unwrap();
        minified.write_to(&mut file).unwrap();

        let file = File::open(&file_path).unwrap();
        let from_file = Interchange::from_json_reader(file).unwrap();

        assert_eq!(minified, from_file);
    }

    // --- len / is_empty ---

    #[test]
    fn test_len() {
        let interchange = get_interchange();
        assert_eq!(interchange.len(), 2);
    }

    #[test]
    fn test_is_empty_false() {
        let interchange = get_interchange();
        assert!(!interchange.is_empty());
    }

    #[test]
    fn test_is_empty_true() {
        let empty = Interchange {
            metadata: InterchangeMetadata {
                interchange_format_version: 5,
                genesis_validators_root: Hash256::zero(),
            },
            data: vec![],
        };
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
    }

    // --- equiv ---

    #[test]
    fn test_equiv_same_order() {
        let a = get_interchange();
        let b = get_interchange();
        assert!(a.equiv(&b));
    }

    #[test]
    fn test_equiv_different_order() {
        let a = get_interchange();
        let mut b = get_interchange();
        b.data.reverse();
        assert!(a.equiv(&b));
    }

    #[test]
    fn test_equiv_different_metadata() {
        let a = get_interchange();
        let mut b = get_interchange();
        b.metadata.interchange_format_version = 99;
        assert!(!a.equiv(&b));
    }

    #[test]
    fn test_equiv_different_data() {
        let a = get_interchange();
        let mut b = get_interchange();
        b.data.pop();
        assert!(!a.equiv(&b));
    }

    #[test]
    fn test_equiv_both_empty() {
        let meta = InterchangeMetadata {
            interchange_format_version: 5,
            genesis_validators_root: Hash256::zero(),
        };
        let a = Interchange {
            metadata: meta.clone(),
            data: vec![],
        };
        let b = Interchange {
            metadata: meta,
            data: vec![],
        };
        assert!(a.equiv(&b));
    }

    // --- minify edge cases ---

    #[test]
    fn test_minify_empty() {
        let interchange = Interchange {
            metadata: InterchangeMetadata {
                interchange_format_version: 5,
                genesis_validators_root: Hash256::zero(),
            },
            data: vec![],
        };
        let minified = interchange.minify().unwrap();
        assert!(minified.data.is_empty());
        assert_eq!(minified.metadata, interchange.metadata);
    }

    #[test]
    fn test_minify_picks_max_block_slot() {
        let pubkey = PublicKeyBytes::deserialize(&[1u8; 48]).unwrap();
        let interchange = Interchange {
            metadata: InterchangeMetadata {
                interchange_format_version: 5,
                genesis_validators_root: Hash256::zero(),
            },
            data: vec![InterchangeData {
                pubkey,
                signed_blocks: vec![
                    SignedBlock {
                        slot: Slot::new(10),
                        signing_root: Some(Hash256::from_low_u64_be(1)),
                    },
                    SignedBlock {
                        slot: Slot::new(50),
                        signing_root: Some(Hash256::from_low_u64_be(2)),
                    },
                    SignedBlock {
                        slot: Slot::new(30),
                        signing_root: Some(Hash256::from_low_u64_be(3)),
                    },
                ],
                signed_attestations: vec![],
            }],
        };
        let minified = interchange.minify().unwrap();
        assert_eq!(minified.data.len(), 1);
        assert_eq!(minified.data[0].signed_blocks.len(), 1);
        assert_eq!(minified.data[0].signed_blocks[0].slot, Slot::new(50));
        assert!(minified.data[0].signed_blocks[0].signing_root.is_none());
    }

    #[test]
    fn test_minify_picks_max_attestation_epochs() {
        let pubkey = PublicKeyBytes::deserialize(&[1u8; 48]).unwrap();
        let interchange = Interchange {
            metadata: InterchangeMetadata {
                interchange_format_version: 5,
                genesis_validators_root: Hash256::zero(),
            },
            data: vec![InterchangeData {
                pubkey,
                signed_blocks: vec![],
                signed_attestations: vec![
                    SignedAttestation {
                        source_epoch: Epoch::new(1),
                        target_epoch: Epoch::new(5),
                        signing_root: None,
                    },
                    SignedAttestation {
                        source_epoch: Epoch::new(10),
                        target_epoch: Epoch::new(3),
                        signing_root: None,
                    },
                    SignedAttestation {
                        source_epoch: Epoch::new(2),
                        target_epoch: Epoch::new(20),
                        signing_root: None,
                    },
                ],
            }],
        };
        let minified = interchange.minify().unwrap();
        assert_eq!(minified.data[0].signed_attestations.len(), 1);
        // max source = 10, max target = 20 (independently maximized)
        assert_eq!(
            minified.data[0].signed_attestations[0].source_epoch,
            Epoch::new(10)
        );
        assert_eq!(
            minified.data[0].signed_attestations[0].target_epoch,
            Epoch::new(20)
        );
    }

    #[test]
    fn test_minify_merges_duplicate_pubkeys() {
        let pubkey = PublicKeyBytes::deserialize(&[1u8; 48]).unwrap();
        let interchange = Interchange {
            metadata: InterchangeMetadata {
                interchange_format_version: 5,
                genesis_validators_root: Hash256::zero(),
            },
            data: vec![
                InterchangeData {
                    pubkey,
                    signed_blocks: vec![SignedBlock {
                        slot: Slot::new(10),
                        signing_root: None,
                    }],
                    signed_attestations: vec![SignedAttestation {
                        source_epoch: Epoch::new(1),
                        target_epoch: Epoch::new(2),
                        signing_root: None,
                    }],
                },
                InterchangeData {
                    pubkey,
                    signed_blocks: vec![SignedBlock {
                        slot: Slot::new(50),
                        signing_root: None,
                    }],
                    signed_attestations: vec![SignedAttestation {
                        source_epoch: Epoch::new(5),
                        target_epoch: Epoch::new(8),
                        signing_root: None,
                    }],
                },
            ],
        };
        let minified = interchange.minify().unwrap();
        // Duplicate pubkeys should be merged into one entry
        assert_eq!(minified.data.len(), 1);
        assert_eq!(minified.data[0].signed_blocks[0].slot, Slot::new(50));
        assert_eq!(
            minified.data[0].signed_attestations[0].source_epoch,
            Epoch::new(5)
        );
        assert_eq!(
            minified.data[0].signed_attestations[0].target_epoch,
            Epoch::new(8)
        );
    }

    #[test]
    fn test_minify_blocks_only_no_attestations() {
        let pubkey = PublicKeyBytes::deserialize(&[1u8; 48]).unwrap();
        let interchange = Interchange {
            metadata: InterchangeMetadata {
                interchange_format_version: 5,
                genesis_validators_root: Hash256::zero(),
            },
            data: vec![InterchangeData {
                pubkey,
                signed_blocks: vec![SignedBlock {
                    slot: Slot::new(42),
                    signing_root: None,
                }],
                signed_attestations: vec![],
            }],
        };
        let minified = interchange.minify().unwrap();
        assert_eq!(minified.data[0].signed_blocks.len(), 1);
        assert!(minified.data[0].signed_attestations.is_empty());
    }

    #[test]
    fn test_minify_attestations_only_no_blocks() {
        let pubkey = PublicKeyBytes::deserialize(&[1u8; 48]).unwrap();
        let interchange = Interchange {
            metadata: InterchangeMetadata {
                interchange_format_version: 5,
                genesis_validators_root: Hash256::zero(),
            },
            data: vec![InterchangeData {
                pubkey,
                signed_blocks: vec![],
                signed_attestations: vec![SignedAttestation {
                    source_epoch: Epoch::new(3),
                    target_epoch: Epoch::new(7),
                    signing_root: None,
                }],
            }],
        };
        let minified = interchange.minify().unwrap();
        assert!(minified.data[0].signed_blocks.is_empty());
        assert_eq!(minified.data[0].signed_attestations.len(), 1);
    }

    #[test]
    fn test_minify_no_blocks_no_attestations() {
        let pubkey = PublicKeyBytes::deserialize(&[1u8; 48]).unwrap();
        let interchange = Interchange {
            metadata: InterchangeMetadata {
                interchange_format_version: 5,
                genesis_validators_root: Hash256::zero(),
            },
            data: vec![InterchangeData {
                pubkey,
                signed_blocks: vec![],
                signed_attestations: vec![],
            }],
        };
        let minified = interchange.minify().unwrap();
        assert_eq!(minified.data.len(), 1);
        assert!(minified.data[0].signed_blocks.is_empty());
        assert!(minified.data[0].signed_attestations.is_empty());
    }

    #[test]
    fn test_minify_multiple_validators() {
        let pk1 = PublicKeyBytes::deserialize(&[1u8; 48]).unwrap();
        let pk2 = PublicKeyBytes::deserialize(&[2u8; 48]).unwrap();
        let interchange = Interchange {
            metadata: InterchangeMetadata {
                interchange_format_version: 5,
                genesis_validators_root: Hash256::zero(),
            },
            data: vec![
                InterchangeData {
                    pubkey: pk1,
                    signed_blocks: vec![SignedBlock {
                        slot: Slot::new(10),
                        signing_root: None,
                    }],
                    signed_attestations: vec![],
                },
                InterchangeData {
                    pubkey: pk2,
                    signed_blocks: vec![SignedBlock {
                        slot: Slot::new(20),
                        signing_root: None,
                    }],
                    signed_attestations: vec![],
                },
            ],
        };
        let minified = interchange.minify().unwrap();
        assert_eq!(minified.data.len(), 2);
    }

    #[test]
    fn test_minify_clears_signing_roots() {
        let interchange = get_interchange();
        let minified = interchange.minify().unwrap();
        for data in &minified.data {
            for block in &data.signed_blocks {
                assert!(
                    block.signing_root.is_none(),
                    "minify should clear block signing roots"
                );
            }
            for att in &data.signed_attestations {
                assert!(
                    att.signing_root.is_none(),
                    "minify should clear attestation signing roots"
                );
            }
        }
    }

    #[test]
    fn test_minify_preserves_metadata() {
        let interchange = get_interchange();
        let minified = interchange.minify().unwrap();
        assert_eq!(interchange.metadata, minified.metadata);
    }

    // --- serde ---

    #[test]
    fn test_signed_block_with_signing_root_serde() {
        let block = SignedBlock {
            slot: Slot::new(42),
            signing_root: Some(Hash256::from_low_u64_be(99)),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("signing_root"));
        let parsed: SignedBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, parsed);
    }

    #[test]
    fn test_signed_block_without_signing_root_serde() {
        let block = SignedBlock {
            slot: Slot::new(42),
            signing_root: None,
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(!json.contains("signing_root"));
        let parsed: SignedBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, parsed);
    }

    #[test]
    fn test_signed_attestation_serde() {
        let att = SignedAttestation {
            source_epoch: Epoch::new(10),
            target_epoch: Epoch::new(20),
            signing_root: Some(Hash256::from_low_u64_be(5)),
        };
        let json = serde_json::to_string(&att).unwrap();
        let parsed: SignedAttestation = serde_json::from_str(&json).unwrap();
        assert_eq!(att, parsed);
    }

    #[test]
    fn test_deny_unknown_fields() {
        let json = r#"{"slot": "1", "signing_root": null, "extra_field": true}"#;
        let result = serde_json::from_str::<SignedBlock>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_from_json_str_valid() {
        let interchange = get_interchange();
        let json = serde_json::to_string(&interchange).unwrap();
        let parsed = Interchange::from_json_str(&json).unwrap();
        assert_eq!(interchange, parsed);
    }

    #[test]
    fn test_from_json_str_invalid() {
        let result = Interchange::from_json_str("not valid json");
        assert!(result.is_err());
    }
}
