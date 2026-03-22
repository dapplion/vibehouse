pub(crate) use metrics::{IntGauge, Result, set_gauge, try_create_int_gauge};
use std::sync::LazyLock;

pub(crate) static SYNC_SLOTS_PER_SECOND: LazyLock<Result<IntGauge>> = LazyLock::new(|| {
    try_create_int_gauge(
        "sync_slots_per_second",
        "The number of blocks being imported per second",
    )
});

pub(crate) static IS_SYNCED: LazyLock<Result<IntGauge>> = LazyLock::new(|| {
    try_create_int_gauge(
        "sync_eth2_synced",
        "Metric to check if the beacon chain is synced to head. 0 if not synced and non-zero if synced",
    )
});

pub(crate) static NOTIFIER_HEAD_SLOT: LazyLock<Result<IntGauge>> = LazyLock::new(|| {
    try_create_int_gauge(
        "notifier_head_slot",
        "The head slot sourced from the beacon chain notifier",
    )
});
