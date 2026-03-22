/// Handles async task metrics
pub(crate) use metrics::*;
use std::sync::LazyLock;

pub(crate) static ASYNC_TASKS_COUNT: LazyLock<Result<IntGaugeVec>> = LazyLock::new(|| {
    try_create_int_gauge_vec(
        "async_tasks_count",
        "Total number of async tasks spawned using spawn",
        &["async_task_count"],
    )
});
pub(crate) static BLOCKING_TASKS_COUNT: LazyLock<Result<IntGaugeVec>> = LazyLock::new(|| {
    try_create_int_gauge_vec(
        "blocking_tasks_count",
        "Total number of async tasks spawned using spawn_blocking",
        &["blocking_task_count"],
    )
});
pub(crate) static BLOCKING_TASKS_HISTOGRAM: LazyLock<Result<HistogramVec>> = LazyLock::new(|| {
    try_create_histogram_vec(
        "blocking_tasks_histogram",
        "Time taken by blocking tasks",
        &["blocking_task_hist"],
    )
});
pub(crate) static BLOCK_ON_TASKS_COUNT: LazyLock<Result<IntGaugeVec>> = LazyLock::new(|| {
    try_create_int_gauge_vec(
        "block_on_tasks_count",
        "Total number of block_on_dangerous tasks spawned",
        &["name"],
    )
});
pub(crate) static BLOCK_ON_TASKS_HISTOGRAM: LazyLock<Result<HistogramVec>> = LazyLock::new(|| {
    try_create_histogram_vec(
        "block_on_tasks_histogram",
        "Time taken by block_on_dangerous tasks",
        &["name"],
    )
});
pub(crate) static TASKS_HISTOGRAM: LazyLock<Result<HistogramVec>> = LazyLock::new(|| {
    try_create_histogram_vec(
        "async_tasks_time_histogram",
        "Time taken by async tasks",
        &["async_task_hist"],
    )
});
