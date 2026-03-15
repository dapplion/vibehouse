pub mod altair {
    pub const TIMELY_SOURCE_FLAG_INDEX: usize = 0;
    pub const TIMELY_TARGET_FLAG_INDEX: usize = 1;
    pub const TIMELY_HEAD_FLAG_INDEX: usize = 2;
    pub const TIMELY_SOURCE_WEIGHT: u64 = 14;
    pub const TIMELY_TARGET_WEIGHT: u64 = 26;
    pub const TIMELY_HEAD_WEIGHT: u64 = 14;
    pub const SYNC_REWARD_WEIGHT: u64 = 2;
    pub const PROPOSER_WEIGHT: u64 = 8;
    pub const WEIGHT_DENOMINATOR: u64 = 64;
    pub const SYNC_COMMITTEE_SUBNET_COUNT: u64 = 4;
    pub const TARGET_AGGREGATORS_PER_SYNC_SUBCOMMITTEE: u64 = 16;

    pub const PARTICIPATION_FLAG_WEIGHTS: [u64; NUM_FLAG_INDICES] = [
        TIMELY_SOURCE_WEIGHT,
        TIMELY_TARGET_WEIGHT,
        TIMELY_HEAD_WEIGHT,
    ];

    pub const NUM_FLAG_INDICES: usize = 3;
}
pub mod bellatrix {
    pub const INTERVALS_PER_SLOT: u64 = 3;
}
pub mod deneb {
    pub use crate::VERSIONED_HASH_VERSION_KZG;
}

pub mod gloas {
    /// Gloas splits the slot into 4 intervals (proposer block, attestation, aggregate, PTC)
    /// instead of pre-Gloas 3 intervals.
    pub const INTERVALS_PER_SLOT: u64 = 4;

    /// Size of the Payload Timeliness Committee (PTC)
    pub const PTC_SIZE: u64 = 512;

    /// Builder index flag indicating a self-build (proposer builds their own payload)
    pub const BUILDER_INDEX_SELF_BUILD: u64 = u64::MAX;

    /// Bitwise flag which indicates that a ValidatorIndex should be treated as a BuilderIndex.
    /// When set on a withdrawal's validator_index, it means the withdrawal is for a builder.
    pub const BUILDER_INDEX_FLAG: u64 = 1u64 << 40;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Altair constants
    #[test]
    fn altair_flag_indices_distinct() {
        assert_ne!(
            altair::TIMELY_SOURCE_FLAG_INDEX,
            altair::TIMELY_TARGET_FLAG_INDEX
        );
        assert_ne!(
            altair::TIMELY_TARGET_FLAG_INDEX,
            altair::TIMELY_HEAD_FLAG_INDEX
        );
        assert_ne!(
            altair::TIMELY_SOURCE_FLAG_INDEX,
            altair::TIMELY_HEAD_FLAG_INDEX
        );
    }

    #[test]
    fn altair_weights_sum_to_denominator() {
        let total = altair::TIMELY_SOURCE_WEIGHT
            + altair::TIMELY_TARGET_WEIGHT
            + altair::TIMELY_HEAD_WEIGHT
            + altair::SYNC_REWARD_WEIGHT
            + altair::PROPOSER_WEIGHT;
        assert_eq!(total, altair::WEIGHT_DENOMINATOR);
    }

    #[test]
    fn altair_participation_flag_weights_array() {
        assert_eq!(
            altair::PARTICIPATION_FLAG_WEIGHTS.len(),
            altair::NUM_FLAG_INDICES
        );
        assert_eq!(
            altair::PARTICIPATION_FLAG_WEIGHTS[0],
            altair::TIMELY_SOURCE_WEIGHT
        );
        assert_eq!(
            altair::PARTICIPATION_FLAG_WEIGHTS[1],
            altair::TIMELY_TARGET_WEIGHT
        );
        assert_eq!(
            altair::PARTICIPATION_FLAG_WEIGHTS[2],
            altair::TIMELY_HEAD_WEIGHT
        );
    }

    #[test]
    fn altair_num_flag_indices() {
        assert_eq!(altair::NUM_FLAG_INDICES, 3);
    }

    #[test]
    fn altair_sync_committee_subnet_count() {
        assert_eq!(altair::SYNC_COMMITTEE_SUBNET_COUNT, 4);
    }

    // Bellatrix constants
    #[test]
    fn bellatrix_intervals_per_slot() {
        assert_eq!(bellatrix::INTERVALS_PER_SLOT, 3);
    }

    // Gloas constants
    #[test]
    fn gloas_intervals_per_slot() {
        assert_eq!(gloas::INTERVALS_PER_SLOT, 4);
    }

    #[test]
    fn gloas_ptc_size() {
        assert_eq!(gloas::PTC_SIZE, 512);
    }

    #[test]
    fn gloas_builder_index_self_build_is_max() {
        assert_eq!(gloas::BUILDER_INDEX_SELF_BUILD, u64::MAX);
    }

    #[test]
    fn gloas_builder_index_flag_bit_position() {
        assert_eq!(gloas::BUILDER_INDEX_FLAG, 1u64 << 40);
        // Should be a single bit set
        assert_eq!(gloas::BUILDER_INDEX_FLAG.count_ones(), 1);
    }

    #[test]
    fn gloas_builder_index_flag_does_not_overlap_validator_range() {
        // Validator indices are < 2^40, so the flag bit at position 40 doesn't overlap
        let max_validator_index: u64 = (1u64 << 40) - 1;
        assert_eq!(max_validator_index & gloas::BUILDER_INDEX_FLAG, 0);
    }
}
