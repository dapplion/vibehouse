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
    /// Size of the Payload Timeliness Committee (PTC)
    pub const PTC_SIZE: u64 = 512;

    /// Builder index flag indicating a self-build (proposer builds their own payload)
    pub const BUILDER_INDEX_SELF_BUILD: u64 = u64::MAX;

    /// Payload status tracking for ePBS fork choice.
    ///
    /// In Gloas, the beacon block contains a bid but not the actual execution payload.
    /// The payload is revealed separately. Fork choice needs to track whether the payload
    /// has been received yet.
    pub type PayloadStatus = u8;

    /// Payload has not yet been received (Gloas blocks start in this state)
    pub const PAYLOAD_STATUS_PENDING: PayloadStatus = 0;

    /// Slot passed without a payload (empty slot or pre-Gloas block)
    pub const PAYLOAD_STATUS_EMPTY: PayloadStatus = 1;

    /// Valid execution payload has been received and validated
    pub const PAYLOAD_STATUS_FULL: PayloadStatus = 2;
}
