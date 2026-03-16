#![allow(clippy::needless_doctest_main)]
//! A wrapper around the `prometheus` crate that provides a global metrics registry
//! and functions to add and use the following components (more info at
//! [Prometheus docs](https://prometheus.io/docs/concepts/metric_types/)):
//!
//! - `Histogram`: used with `start_timer(..)` and `stop_timer(..)` to record durations (e.g.,
//!   block processing time).
//! - `IncCounter`: used to represent an ideally ever-growing, never-shrinking integer (e.g.,
//!   number of block processing requests).
//! - `IntGauge`: used to represent an varying integer (e.g., number of attestations per block).
//!
//! ## Important
//!
//! Metrics will fail if two items have the same `name`. All metrics must have a unique `name`.
//! Because we use a global registry there is no namespace per crate, it's one big global space.
//!
//! See the [Prometheus naming best practices](https://prometheus.io/docs/practices/naming/) when
//! choosing metric names.
//!
//! ## Example
//!
//! ```rust
//! use metrics::*;
//! use std::sync::LazyLock;
//!
//! // These metrics are "magically" linked to the global registry defined in `metrics`.
//! pub static RUN_COUNT: LazyLock<Result<IntCounter>> = LazyLock::new(|| try_create_int_counter(
//!     "runs_total",
//!     "Total number of runs"
//! ));
//! pub static CURRENT_VALUE: LazyLock<Result<IntGauge>> = LazyLock::new(|| try_create_int_gauge(
//!     "current_value",
//!     "The current value"
//! ));
//! pub static RUN_TIME: LazyLock<Result<Histogram>> =
//!     LazyLock::new(|| try_create_histogram("run_seconds", "Time taken (measured to high precision)"));
//!
//! fn main() {
//!     for i in 0..100 {
//!         inc_counter(&RUN_COUNT);
//!         let timer = start_timer(&RUN_TIME);
//!
//!         for j in 0..10 {
//!             set_gauge(&CURRENT_VALUE, j);
//!             println!("Howdy partner");
//!         }
//!
//!         stop_timer(timer);
//!     }
//! }
//! ```

use prometheus::{Error, HistogramOpts, Opts};
use std::time::Duration;

use prometheus::core::{Atomic, GenericGauge, GenericGaugeVec};
pub use prometheus::{
    DEFAULT_BUCKETS, Encoder, Gauge, GaugeVec, Histogram, HistogramTimer, HistogramVec, IntCounter,
    IntCounterVec, IntGauge, IntGaugeVec, Result, TextEncoder, exponential_buckets, linear_buckets,
    proto::{Metric, MetricFamily, MetricType},
};

/// Collect all the metrics for reporting.
pub fn gather() -> Vec<prometheus::proto::MetricFamily> {
    prometheus::gather()
}

/// Attempts to create an `IntCounter`, returning `Err` if the registry does not accept the counter
/// (potentially due to naming conflict).
pub fn try_create_int_counter(name: &str, help: &str) -> Result<IntCounter> {
    let opts = Opts::new(name, help);
    let counter = IntCounter::with_opts(opts)?;
    prometheus::register(Box::new(counter.clone()))?;
    Ok(counter)
}

/// Attempts to create an `IntGauge`, returning `Err` if the registry does not accept the counter
/// (potentially due to naming conflict).
pub fn try_create_int_gauge(name: &str, help: &str) -> Result<IntGauge> {
    let opts = Opts::new(name, help);
    let gauge = IntGauge::with_opts(opts)?;
    prometheus::register(Box::new(gauge.clone()))?;
    Ok(gauge)
}

/// Attempts to create a `Gauge`, returning `Err` if the registry does not accept the counter
/// (potentially due to naming conflict).
pub fn try_create_float_gauge(name: &str, help: &str) -> Result<Gauge> {
    let opts = Opts::new(name, help);
    let gauge = Gauge::with_opts(opts)?;
    prometheus::register(Box::new(gauge.clone()))?;
    Ok(gauge)
}

/// Attempts to create a `Histogram`, returning `Err` if the registry does not accept the counter
/// (potentially due to naming conflict).
pub fn try_create_histogram(name: &str, help: &str) -> Result<Histogram> {
    try_create_histogram_with_buckets(name, help, Ok(DEFAULT_BUCKETS.to_vec()))
}

/// Attempts to create a `Histogram` with specified buckets, returning `Err` if the registry does not accept the counter
/// (potentially due to naming conflict) or no valid buckets are provided.
pub fn try_create_histogram_with_buckets(
    name: &str,
    help: &str,
    buckets: Result<Vec<f64>>,
) -> Result<Histogram> {
    let opts = HistogramOpts::new(name, help).buckets(buckets?);
    let histogram = Histogram::with_opts(opts)?;
    prometheus::register(Box::new(histogram.clone()))?;
    Ok(histogram)
}

/// Attempts to create a `HistogramVec`, returning `Err` if the registry does not accept the counter
/// (potentially due to naming conflict).
pub fn try_create_histogram_vec(
    name: &str,
    help: &str,
    label_names: &[&str],
) -> Result<HistogramVec> {
    try_create_histogram_vec_with_buckets(name, help, Ok(DEFAULT_BUCKETS.to_vec()), label_names)
}

/// Attempts to create a `HistogramVec` with specified buckets, returning `Err` if the registry does not accept the counter
/// (potentially due to naming conflict) or no valid buckets are provided.
pub fn try_create_histogram_vec_with_buckets(
    name: &str,
    help: &str,
    buckets: Result<Vec<f64>>,
    label_names: &[&str],
) -> Result<HistogramVec> {
    let opts = HistogramOpts::new(name, help).buckets(buckets?);
    let histogram_vec = HistogramVec::new(opts, label_names)?;
    prometheus::register(Box::new(histogram_vec.clone()))?;
    Ok(histogram_vec)
}

/// Attempts to create a `IntGaugeVec`, returning `Err` if the registry does not accept the gauge
/// (potentially due to naming conflict).
pub fn try_create_int_gauge_vec(
    name: &str,
    help: &str,
    label_names: &[&str],
) -> Result<IntGaugeVec> {
    let opts = Opts::new(name, help);
    let counter_vec = IntGaugeVec::new(opts, label_names)?;
    prometheus::register(Box::new(counter_vec.clone()))?;
    Ok(counter_vec)
}

/// Attempts to create a `GaugeVec`, returning `Err` if the registry does not accept the gauge
/// (potentially due to naming conflict).
pub fn try_create_float_gauge_vec(
    name: &str,
    help: &str,
    label_names: &[&str],
) -> Result<GaugeVec> {
    let opts = Opts::new(name, help);
    let counter_vec = GaugeVec::new(opts, label_names)?;
    prometheus::register(Box::new(counter_vec.clone()))?;
    Ok(counter_vec)
}

/// Attempts to create a `IntCounterVec`, returning `Err` if the registry does not accept the gauge
/// (potentially due to naming conflict).
pub fn try_create_int_counter_vec(
    name: &str,
    help: &str,
    label_names: &[&str],
) -> Result<IntCounterVec> {
    let opts = Opts::new(name, help);
    let counter_vec = IntCounterVec::new(opts, label_names)?;
    prometheus::register(Box::new(counter_vec.clone()))?;
    Ok(counter_vec)
}

/// If `int_gauge_vec.is_ok()`, returns a gauge with the given `name`.
pub fn get_int_gauge(int_gauge_vec: &Result<IntGaugeVec>, name: &[&str]) -> Option<IntGauge> {
    if let Ok(int_gauge_vec) = int_gauge_vec {
        Some(int_gauge_vec.get_metric_with_label_values(name).ok()?)
    } else {
        None
    }
}

pub fn get_gauge<P: Atomic>(
    gauge_vec: &Result<GenericGaugeVec<P>>,
    name: &[&str],
) -> Option<GenericGauge<P>> {
    if let Ok(gauge_vec) = gauge_vec {
        Some(gauge_vec.get_metric_with_label_values(name).ok()?)
    } else {
        None
    }
}

pub fn set_gauge_entry<P: Atomic>(
    gauge_vec: &Result<GenericGaugeVec<P>>,
    name: &[&str],
    value: P::T,
) {
    if let Some(v) = get_gauge(gauge_vec, name) {
        v.set(value)
    };
}

/// If `int_gauge_vec.is_ok()`, sets the gauge with the given `name` to the given `value`
/// otherwise returns false.
pub fn set_int_gauge(int_gauge_vec: &Result<IntGaugeVec>, name: &[&str], value: i64) -> bool {
    if let Ok(int_gauge_vec) = int_gauge_vec {
        int_gauge_vec
            .get_metric_with_label_values(name)
            .map(|v| {
                v.set(value);
                true
            })
            .unwrap_or_else(|_| false)
    } else {
        false
    }
}

/// If `int_counter_vec.is_ok()`, returns a counter with the given `name`.
pub fn get_int_counter(
    int_counter_vec: &Result<IntCounterVec>,
    name: &[&str],
) -> Option<IntCounter> {
    if let Ok(int_counter_vec) = int_counter_vec {
        Some(int_counter_vec.get_metric_with_label_values(name).ok()?)
    } else {
        None
    }
}

/// Increments the `int_counter_vec` with the given `name`.
pub fn inc_counter_vec(int_counter_vec: &Result<IntCounterVec>, name: &[&str]) {
    if let Some(counter) = get_int_counter(int_counter_vec, name) {
        counter.inc()
    }
}

pub fn inc_counter_vec_by(int_counter_vec: &Result<IntCounterVec>, name: &[&str], amount: u64) {
    if let Some(counter) = get_int_counter(int_counter_vec, name) {
        counter.inc_by(amount);
    }
}

/// If `histogram_vec.is_ok()`, returns a histogram with the given `name`.
pub fn get_histogram(histogram_vec: &Result<HistogramVec>, name: &[&str]) -> Option<Histogram> {
    if let Ok(histogram_vec) = histogram_vec {
        Some(histogram_vec.get_metric_with_label_values(name).ok()?)
    } else {
        None
    }
}

/// Starts a timer on `vec` with the given `name`.
pub fn start_timer_vec(vec: &Result<HistogramVec>, name: &[&str]) -> Option<HistogramTimer> {
    get_histogram(vec, name).map(|h| h.start_timer())
}

/// Starts a timer for the given `Histogram`, stopping when it gets dropped or given to `stop_timer(..)`.
pub fn start_timer(histogram: &Result<Histogram>) -> Option<HistogramTimer> {
    if let Ok(histogram) = histogram {
        Some(histogram.start_timer())
    } else {
        None
    }
}

/// Starts a timer on `vec` with the given `name`.
pub fn observe_timer_vec(vec: &Result<HistogramVec>, name: &[&str], duration: Duration) {
    if let Some(h) = get_histogram(vec, name) {
        h.observe(duration_to_f64(duration))
    }
}

/// Stops a timer created with `start_timer(..)`.
pub fn stop_timer(timer: Option<HistogramTimer>) {
    if let Some(t) = timer {
        t.observe_duration()
    }
}

/// Stops a timer created with `start_timer(..)`.
///
/// Return the duration that the timer was running for, or 0.0 if it was `None` due to incorrect
/// initialisation.
pub fn stop_timer_with_duration(timer: Option<HistogramTimer>) -> Duration {
    Duration::from_secs_f64(timer.map_or(0.0, |t| t.stop_and_record()))
}

pub fn observe_vec(vec: &Result<HistogramVec>, name: &[&str], value: f64) {
    if let Some(h) = get_histogram(vec, name) {
        h.observe(value)
    }
}

pub fn inc_counter(counter: &Result<IntCounter>) {
    if let Ok(counter) = counter {
        counter.inc();
    }
}

pub fn inc_counter_by(counter: &Result<IntCounter>, value: u64) {
    if let Ok(counter) = counter {
        counter.inc_by(value);
    }
}

pub fn set_gauge_vec(int_gauge_vec: &Result<IntGaugeVec>, name: &[&str], value: i64) {
    if let Some(gauge) = get_int_gauge(int_gauge_vec, name) {
        gauge.set(value);
    }
}

pub fn inc_gauge_vec(int_gauge_vec: &Result<IntGaugeVec>, name: &[&str]) {
    if let Some(gauge) = get_int_gauge(int_gauge_vec, name) {
        gauge.inc();
    }
}

pub fn dec_gauge_vec(int_gauge_vec: &Result<IntGaugeVec>, name: &[&str]) {
    if let Some(gauge) = get_int_gauge(int_gauge_vec, name) {
        gauge.dec();
    }
}

pub fn set_gauge(gauge: &Result<IntGauge>, value: i64) {
    if let Ok(gauge) = gauge {
        gauge.set(value);
    }
}

pub fn set_float_gauge(gauge: &Result<Gauge>, value: f64) {
    if let Ok(gauge) = gauge {
        gauge.set(value);
    }
}

pub fn set_float_gauge_vec(gauge_vec: &Result<GaugeVec>, name: &[&str], value: f64) {
    if let Some(gauge) = get_gauge(gauge_vec, name) {
        gauge.set(value);
    }
}

pub fn inc_gauge(gauge: &Result<IntGauge>) {
    if let Ok(gauge) = gauge {
        gauge.inc();
    }
}

pub fn dec_gauge(gauge: &Result<IntGauge>) {
    if let Ok(gauge) = gauge {
        gauge.dec();
    }
}

pub fn maybe_set_gauge(gauge: &Result<IntGauge>, value_opt: Option<i64>) {
    if let Some(value) = value_opt {
        set_gauge(gauge, value)
    }
}

pub fn maybe_set_float_gauge(gauge: &Result<Gauge>, value_opt: Option<f64>) {
    if let Some(value) = value_opt {
        set_float_gauge(gauge, value)
    }
}

/// Sets the value of a `Histogram` manually.
pub fn observe(histogram: &Result<Histogram>, value: f64) {
    if let Ok(histogram) = histogram {
        histogram.observe(value);
    }
}

pub fn observe_duration(histogram: &Result<Histogram>, duration: Duration) {
    if let Ok(histogram) = histogram {
        histogram.observe(duration_to_f64(duration))
    }
}

fn duration_to_f64(duration: Duration) -> f64 {
    // This conversion was taken from here:
    //
    // https://docs.rs/prometheus/0.5.0/src/prometheus/histogram.rs.html#550-555
    let nanos = f64::from(duration.subsec_nanos()) / 1e9;
    duration.as_secs() as f64 + nanos
}

/// Create buckets using divisors of 10 multiplied by powers of 10, e.g.,
/// […, 0.1, 0.2, 0.5, 1, 2, 5, 10, 20, 50, …]
///
/// The buckets go from `10^min_power` to `5 × 10^max_power`, inclusively.
/// The total number of buckets is `3 * (max_power - min_power + 1)`.
///
/// assert_eq!(vec![0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0], decimal_buckets(-1, 1));
/// assert_eq!(vec![1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0], decimal_buckets(0, 2));
pub fn decimal_buckets(min_power: i32, max_power: i32) -> Result<Vec<f64>> {
    if max_power < min_power {
        return Err(Error::Msg(format!(
            "decimal_buckets min_power needs to be <= max_power, given {} and {}",
            min_power, max_power
        )));
    }

    let mut buckets = Vec::with_capacity(3 * (max_power - min_power + 1) as usize);
    for n in min_power..=max_power {
        for m in &[1f64, 2f64, 5f64] {
            buckets.push(m * 10f64.powi(n))
        }
    }
    Ok(buckets)
}

/// Would be nice to use the `Try` trait bound and have a single implementation, but try_trait_v2
/// is not a stable feature yet.
pub trait TryExt {
    fn discard_timer_on_break(self, timer: &mut Option<HistogramTimer>) -> Self;
}

impl<T, E> TryExt for std::result::Result<T, E> {
    fn discard_timer_on_break(self, timer_opt: &mut Option<HistogramTimer>) -> Self {
        if self.is_err()
            && let Some(timer) = timer_opt.take()
        {
            timer.stop_and_discard();
        }
        self
    }
}

impl<T> TryExt for Option<T> {
    fn discard_timer_on_break(self, timer_opt: &mut Option<HistogramTimer>) -> Self {
        if self.is_none()
            && let Some(timer) = timer_opt.take()
        {
            timer.stop_and_discard();
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── decimal_buckets ─────────────────────────────────────────

    #[test]
    fn decimal_buckets_standard_range() {
        let buckets = decimal_buckets(-1, 1).unwrap();
        let expected = vec![0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0];
        assert_eq!(buckets.len(), expected.len());
        for (a, b) in buckets.iter().zip(expected.iter()) {
            assert!((a - b).abs() < 1e-10, "expected {b}, got {a}");
        }
    }

    #[test]
    fn decimal_buckets_zero_to_two() {
        let buckets = decimal_buckets(0, 2).unwrap();
        let expected = vec![1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0];
        assert_eq!(buckets.len(), expected.len());
        for (a, b) in buckets.iter().zip(expected.iter()) {
            assert!((a - b).abs() < 1e-10, "expected {b}, got {a}");
        }
    }

    #[test]
    fn decimal_buckets_single_power() {
        let buckets = decimal_buckets(0, 0).unwrap();
        assert_eq!(buckets, vec![1.0, 2.0, 5.0]);
    }

    #[test]
    fn decimal_buckets_negative_range() {
        let buckets = decimal_buckets(-3, -2).unwrap();
        let expected = [0.001, 0.002, 0.005, 0.01, 0.02, 0.05];
        assert_eq!(buckets.len(), expected.len());
        for (a, b) in buckets.iter().zip(expected.iter()) {
            assert!((a - b).abs() < 1e-15, "expected {b}, got {a}");
        }
    }

    #[test]
    fn decimal_buckets_invalid_range() {
        assert!(decimal_buckets(2, 1).is_err());
        assert!(decimal_buckets(0, -1).is_err());
    }

    #[test]
    fn decimal_buckets_count() {
        // 3 * (max - min + 1) buckets
        let buckets = decimal_buckets(-2, 3).unwrap();
        assert_eq!(buckets.len(), 3 * 6);
    }

    // ── duration_to_f64 ─────────────────────────────────────────

    #[test]
    fn duration_to_f64_zero() {
        assert_eq!(duration_to_f64(Duration::from_secs(0)), 0.0);
    }

    #[test]
    fn duration_to_f64_whole_seconds() {
        assert_eq!(duration_to_f64(Duration::from_secs(42)), 42.0);
    }

    #[test]
    fn duration_to_f64_fractional() {
        let d = Duration::from_millis(1500);
        assert!((duration_to_f64(d) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn duration_to_f64_nanos() {
        let d = Duration::new(1, 500_000_000); // 1.5s
        assert!((duration_to_f64(d) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn duration_to_f64_large() {
        let d = Duration::from_secs(86400); // 1 day
        assert_eq!(duration_to_f64(d), 86400.0);
    }

    // ── stop_timer ──────────────────────────────────────────────

    #[test]
    fn stop_timer_none_is_noop() {
        // Should not panic
        stop_timer(None);
    }

    #[test]
    fn stop_timer_with_duration_none_returns_zero() {
        let d = stop_timer_with_duration(None);
        assert_eq!(d, Duration::from_secs(0));
    }

    // ── TryExt for Result ───────────────────────────────────────

    #[test]
    fn try_ext_result_ok_preserves_timer() {
        let histogram =
            try_create_histogram("test_try_ext_result_ok_preserves", "test histogram").unwrap();
        let mut timer = Some(histogram.start_timer());
        let result: std::result::Result<i32, &str> = Ok(42);
        let result = result.discard_timer_on_break(&mut timer);
        assert_eq!(result.unwrap(), 42);
        assert!(timer.is_some(), "timer should still be present on Ok");
    }

    #[test]
    fn try_ext_result_err_discards_timer() {
        let histogram =
            try_create_histogram("test_try_ext_result_err_discards", "test histogram").unwrap();
        let mut timer = Some(histogram.start_timer());
        let result: std::result::Result<i32, &str> = Err("fail");
        let result = result.discard_timer_on_break(&mut timer);
        assert!(result.is_err());
        assert!(timer.is_none(), "timer should be taken on Err");
    }

    #[test]
    fn try_ext_result_err_no_timer_is_noop() {
        let mut timer: Option<HistogramTimer> = None;
        let result: std::result::Result<i32, &str> = Err("fail");
        let _ = result.discard_timer_on_break(&mut timer);
        assert!(timer.is_none());
    }

    // ── TryExt for Option ───────────────────────────────────────

    #[test]
    fn try_ext_option_some_preserves_timer() {
        let histogram =
            try_create_histogram("test_try_ext_option_some_preserves", "test histogram").unwrap();
        let mut timer = Some(histogram.start_timer());
        let opt: Option<i32> = Some(42);
        let opt = opt.discard_timer_on_break(&mut timer);
        assert_eq!(opt.unwrap(), 42);
        assert!(timer.is_some(), "timer should still be present on Some");
    }

    #[test]
    fn try_ext_option_none_discards_timer() {
        let histogram =
            try_create_histogram("test_try_ext_option_none_discards", "test histogram").unwrap();
        let mut timer = Some(histogram.start_timer());
        let opt: Option<i32> = None;
        let opt = opt.discard_timer_on_break(&mut timer);
        assert!(opt.is_none());
        assert!(timer.is_none(), "timer should be taken on None");
    }

    // ── metric creation functions ───────────────────────────────

    #[test]
    fn try_create_int_counter_success() {
        let counter = try_create_int_counter("test_counter_create", "A test counter");
        assert!(counter.is_ok());
    }

    #[test]
    fn try_create_int_counter_duplicate_fails() {
        let _ = try_create_int_counter("test_counter_dup", "first");
        let result = try_create_int_counter("test_counter_dup", "second");
        assert!(result.is_err());
    }

    #[test]
    fn try_create_int_gauge_success() {
        let gauge = try_create_int_gauge("test_gauge_create", "A test gauge");
        assert!(gauge.is_ok());
    }

    #[test]
    fn try_create_float_gauge_success() {
        let gauge = try_create_float_gauge("test_float_gauge_create", "A test float gauge");
        assert!(gauge.is_ok());
    }

    #[test]
    fn try_create_histogram_success() {
        let h = try_create_histogram("test_histogram_create", "A test histogram");
        assert!(h.is_ok());
    }

    #[test]
    fn try_create_histogram_with_buckets_success() {
        let buckets = Ok(vec![0.1, 0.5, 1.0, 5.0, 10.0]);
        let h = try_create_histogram_with_buckets("test_histogram_buckets_create", "test", buckets);
        assert!(h.is_ok());
    }

    #[test]
    fn try_create_histogram_vec_success() {
        let h = try_create_histogram_vec("test_histogram_vec_create", "test", &["label1"]);
        assert!(h.is_ok());
    }

    #[test]
    fn try_create_int_gauge_vec_success() {
        let g = try_create_int_gauge_vec("test_int_gauge_vec_create", "test", &["label1"]);
        assert!(g.is_ok());
    }

    #[test]
    fn try_create_float_gauge_vec_success() {
        let g = try_create_float_gauge_vec("test_float_gauge_vec_create", "test", &["label1"]);
        assert!(g.is_ok());
    }

    #[test]
    fn try_create_int_counter_vec_success() {
        let c = try_create_int_counter_vec("test_int_counter_vec_create", "test", &["label1"]);
        assert!(c.is_ok());
    }

    // ── gauge/counter getter and setter functions ───────────────

    #[test]
    fn set_and_get_int_gauge_vec() {
        let gauge_vec = try_create_int_gauge_vec("test_set_get_igv", "test", &["name"]);
        set_gauge_vec(&gauge_vec, &["foo"], 42);
        let gauge = get_int_gauge(&gauge_vec, &["foo"]);
        assert_eq!(gauge.unwrap().get(), 42);
    }

    #[test]
    fn get_int_gauge_err_returns_none() {
        let gauge_vec: Result<IntGaugeVec> = Err(Error::Msg("not initialized".into()));
        assert!(get_int_gauge(&gauge_vec, &["foo"]).is_none());
    }

    #[test]
    fn inc_dec_gauge_vec() {
        let gauge_vec = try_create_int_gauge_vec("test_inc_dec_gv", "test", &["name"]);
        inc_gauge_vec(&gauge_vec, &["bar"]);
        inc_gauge_vec(&gauge_vec, &["bar"]);
        dec_gauge_vec(&gauge_vec, &["bar"]);
        let gauge = get_int_gauge(&gauge_vec, &["bar"]);
        assert_eq!(gauge.unwrap().get(), 1);
    }

    #[test]
    fn set_int_gauge_returns_true_on_success() {
        let gauge_vec = try_create_int_gauge_vec("test_set_ig_ret", "test", &["name"]);
        assert!(set_int_gauge(&gauge_vec, &["x"], 99));
        let gauge = get_int_gauge(&gauge_vec, &["x"]);
        assert_eq!(gauge.unwrap().get(), 99);
    }

    #[test]
    fn set_int_gauge_err_returns_false() {
        let gauge_vec: Result<IntGaugeVec> = Err(Error::Msg("err".into()));
        assert!(!set_int_gauge(&gauge_vec, &["x"], 99));
    }

    #[test]
    fn inc_counter_vec_increments() {
        let counter_vec = try_create_int_counter_vec("test_inc_cv", "test", &["name"]);
        inc_counter_vec(&counter_vec, &["a"]);
        inc_counter_vec(&counter_vec, &["a"]);
        let counter = get_int_counter(&counter_vec, &["a"]);
        assert_eq!(counter.unwrap().get(), 2);
    }

    #[test]
    fn inc_counter_vec_by_amount() {
        let counter_vec = try_create_int_counter_vec("test_inc_cv_by", "test", &["name"]);
        inc_counter_vec_by(&counter_vec, &["b"], 10);
        let counter = get_int_counter(&counter_vec, &["b"]);
        assert_eq!(counter.unwrap().get(), 10);
    }

    #[test]
    fn get_int_counter_err_returns_none() {
        let counter_vec: Result<IntCounterVec> = Err(Error::Msg("err".into()));
        assert!(get_int_counter(&counter_vec, &["x"]).is_none());
    }

    // ── simple gauge/counter operations ─────────────────────────

    #[test]
    fn set_gauge_and_inc_dec() {
        let gauge = try_create_int_gauge("test_gauge_ops", "test");
        set_gauge(&gauge, 10);
        assert_eq!(gauge.as_ref().unwrap().get(), 10);
        inc_gauge(&gauge);
        assert_eq!(gauge.as_ref().unwrap().get(), 11);
        dec_gauge(&gauge);
        assert_eq!(gauge.as_ref().unwrap().get(), 10);
    }

    #[test]
    fn set_float_gauge_works() {
        let gauge = try_create_float_gauge("test_float_gauge_ops", "test");
        set_float_gauge(&gauge, 3.25);
        assert!((gauge.as_ref().unwrap().get() - 3.25).abs() < 1e-10);
    }

    #[test]
    fn inc_counter_works() {
        let counter = try_create_int_counter("test_inc_counter_ops", "test");
        inc_counter(&counter);
        inc_counter(&counter);
        assert_eq!(counter.as_ref().unwrap().get(), 2);
    }

    #[test]
    fn inc_counter_by_works() {
        let counter = try_create_int_counter("test_inc_counter_by_ops", "test");
        inc_counter_by(&counter, 5);
        assert_eq!(counter.as_ref().unwrap().get(), 5);
    }

    // ── maybe_set functions ─────────────────────────────────────

    #[test]
    fn maybe_set_gauge_some() {
        let gauge = try_create_int_gauge("test_maybe_set_some", "test");
        maybe_set_gauge(&gauge, Some(42));
        assert_eq!(gauge.as_ref().unwrap().get(), 42);
    }

    #[test]
    fn maybe_set_gauge_none_is_noop() {
        let gauge = try_create_int_gauge("test_maybe_set_none", "test");
        set_gauge(&gauge, 10);
        maybe_set_gauge(&gauge, None);
        assert_eq!(gauge.as_ref().unwrap().get(), 10);
    }

    #[test]
    fn maybe_set_float_gauge_some() {
        let gauge = try_create_float_gauge("test_maybe_set_float_some", "test");
        maybe_set_float_gauge(&gauge, Some(2.75));
        assert!((gauge.as_ref().unwrap().get() - 2.75).abs() < 1e-10);
    }

    #[test]
    fn maybe_set_float_gauge_none_is_noop() {
        let gauge = try_create_float_gauge("test_maybe_set_float_none", "test");
        set_float_gauge(&gauge, 1.0);
        maybe_set_float_gauge(&gauge, None);
        assert!((gauge.as_ref().unwrap().get() - 1.0).abs() < 1e-10);
    }

    // ── observe functions ───────────────────────────────────────

    #[test]
    fn observe_histogram_works() {
        let h = try_create_histogram("test_observe_h", "test");
        observe(&h, 1.5);
        observe(&h, 2.5);
        assert_eq!(h.as_ref().unwrap().get_sample_count(), 2);
    }

    #[test]
    fn observe_duration_works() {
        let h = try_create_histogram("test_observe_dur", "test");
        observe_duration(&h, Duration::from_millis(100));
        assert_eq!(h.as_ref().unwrap().get_sample_count(), 1);
    }

    // ── histogram vec operations ────────────────────────────────

    #[test]
    fn start_timer_vec_returns_some() {
        let hv = try_create_histogram_vec("test_start_timer_vec", "test", &["op"]);
        let timer = start_timer_vec(&hv, &["read"]);
        assert!(timer.is_some());
        stop_timer(timer);
    }

    #[test]
    fn observe_timer_vec_records() {
        let hv = try_create_histogram_vec("test_observe_timer_vec", "test", &["op"]);
        observe_timer_vec(&hv, &["write"], Duration::from_millis(50));
        let h = get_histogram(&hv, &["write"]);
        assert_eq!(h.unwrap().get_sample_count(), 1);
    }

    #[test]
    fn observe_vec_records() {
        let hv = try_create_histogram_vec("test_observe_vec", "test", &["op"]);
        observe_vec(&hv, &["sync"], 0.5);
        let h = get_histogram(&hv, &["sync"]);
        assert_eq!(h.unwrap().get_sample_count(), 1);
    }

    #[test]
    fn get_histogram_err_returns_none() {
        let hv: Result<HistogramVec> = Err(Error::Msg("err".into()));
        assert!(get_histogram(&hv, &["x"]).is_none());
    }

    // ── start_timer for plain Histogram ─────────────────────────

    #[test]
    fn start_timer_ok_returns_some() {
        let h = try_create_histogram("test_start_timer_ok", "test");
        let timer = start_timer(&h);
        assert!(timer.is_some());
        stop_timer(timer);
    }

    #[test]
    fn start_timer_err_returns_none() {
        let h: Result<Histogram> = Err(Error::Msg("err".into()));
        let timer = start_timer(&h);
        assert!(timer.is_none());
    }

    // ── gather ──────────────────────────────────────────────────

    #[test]
    fn gather_returns_families() {
        // Register a metric so gather has something to return
        let _ = try_create_int_counter("test_gather_check", "test");
        let families = gather();
        assert!(!families.is_empty());
    }

    // ── set_float_gauge_vec ─────────────────────────────────────

    #[test]
    fn set_float_gauge_vec_works() {
        let gv = try_create_float_gauge_vec("test_set_fgv", "test", &["name"]);
        set_float_gauge_vec(&gv, &["a"], 1.23);
        let g = get_gauge(&gv, &["a"]);
        assert!((g.unwrap().get() - 1.23).abs() < 1e-10);
    }

    // ── set_gauge_entry ─────────────────────────────────────────

    #[test]
    fn set_gauge_entry_works() {
        let gv = try_create_int_gauge_vec("test_set_gauge_entry", "test", &["name"]);
        set_gauge_entry(&gv, &["x"], 77);
        let g = get_int_gauge(&gv, &["x"]);
        assert_eq!(g.unwrap().get(), 77);
    }

    // ── operations on Err metrics are no-ops ────────────────────

    #[test]
    fn operations_on_err_gauges_are_noop() {
        let gauge: Result<IntGauge> = Err(Error::Msg("err".into()));
        set_gauge(&gauge, 10);
        inc_gauge(&gauge);
        dec_gauge(&gauge);
        maybe_set_gauge(&gauge, Some(5));
        // Should not panic
    }

    #[test]
    fn operations_on_err_float_gauges_are_noop() {
        let gauge: Result<Gauge> = Err(Error::Msg("err".into()));
        set_float_gauge(&gauge, 1.0);
        maybe_set_float_gauge(&gauge, Some(2.0));
        // Should not panic
    }

    #[test]
    fn operations_on_err_counters_are_noop() {
        let counter: Result<IntCounter> = Err(Error::Msg("err".into()));
        inc_counter(&counter);
        inc_counter_by(&counter, 5);
        // Should not panic
    }

    #[test]
    fn operations_on_err_histograms_are_noop() {
        let h: Result<Histogram> = Err(Error::Msg("err".into()));
        observe(&h, 1.0);
        observe_duration(&h, Duration::from_secs(1));
        // Should not panic
    }

    #[test]
    fn operations_on_err_counter_vecs_are_noop() {
        let cv: Result<IntCounterVec> = Err(Error::Msg("err".into()));
        inc_counter_vec(&cv, &["x"]);
        inc_counter_vec_by(&cv, &["x"], 5);
        // Should not panic
    }

    #[test]
    fn operations_on_err_gauge_vecs_are_noop() {
        let gv: Result<IntGaugeVec> = Err(Error::Msg("err".into()));
        set_gauge_vec(&gv, &["x"], 10);
        inc_gauge_vec(&gv, &["x"]);
        dec_gauge_vec(&gv, &["x"]);
        // Should not panic
    }
}
