use rayon::{ThreadPool, ThreadPoolBuilder};
use std::sync::Arc;

const DEFAULT_LOW_PRIORITY_CPU_PERCENTAGE: usize = 25;
const DEFAULT_HIGH_PRIORITY_CPU_PERCENTAGE: usize = 80;
const MINIMUM_THREAD_COUNT: usize = 1;

pub enum RayonPoolType {
    HighPriority,
    LowPriority,
}

pub struct RayonPoolProvider {
    /// Smaller rayon thread pool for lower-priority, compute-intensive tasks.
    /// By default ~25% of CPUs or a minimum of 1 thread.
    low_priority_thread_pool: Arc<ThreadPool>,
    /// Larger rayon thread pool for high-priority, compute-intensive tasks.
    /// By default ~80% of CPUs or a minimum of 1 thread. Citical/highest
    /// priority tasks should use the global pool instead.
    high_priority_thread_pool: Arc<ThreadPool>,
}

impl Default for RayonPoolProvider {
    fn default() -> Self {
        let low_prio_threads =
            (num_cpus::get() * DEFAULT_LOW_PRIORITY_CPU_PERCENTAGE / 100).max(MINIMUM_THREAD_COUNT);
        let low_priority_thread_pool = Arc::new(
            ThreadPoolBuilder::new()
                .num_threads(low_prio_threads)
                .build()
                .expect("failed to build low-priority rayon pool"),
        );

        let high_prio_threads = (num_cpus::get() * DEFAULT_HIGH_PRIORITY_CPU_PERCENTAGE / 100)
            .max(MINIMUM_THREAD_COUNT);
        let high_priority_thread_pool = Arc::new(
            ThreadPoolBuilder::new()
                .num_threads(high_prio_threads)
                .build()
                .expect("failed to build high-priority rayon pool"),
        );
        Self {
            low_priority_thread_pool,
            high_priority_thread_pool,
        }
    }
}

impl RayonPoolProvider {
    /// Get a scoped thread pool by priority level.
    /// For critical/highest priority tasks, use the global pool instead.
    pub fn get_thread_pool(&self, rayon_pool_type: RayonPoolType) -> Arc<ThreadPool> {
        match rayon_pool_type {
            RayonPoolType::HighPriority => self.high_priority_thread_pool.clone(),
            RayonPoolType::LowPriority => self.low_priority_thread_pool.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_creates_pools() {
        let provider = RayonPoolProvider::default();

        let low = provider.get_thread_pool(RayonPoolType::LowPriority);
        let high = provider.get_thread_pool(RayonPoolType::HighPriority);

        assert!(
            low.current_num_threads() >= MINIMUM_THREAD_COUNT,
            "low priority pool has at least {} thread(s)",
            MINIMUM_THREAD_COUNT
        );
        assert!(
            high.current_num_threads() >= MINIMUM_THREAD_COUNT,
            "high priority pool has at least {} thread(s)",
            MINIMUM_THREAD_COUNT
        );
    }

    #[test]
    fn high_priority_has_more_threads_than_low() {
        let cpus = num_cpus::get();
        if cpus >= 4 {
            let provider = RayonPoolProvider::default();
            let low = provider.get_thread_pool(RayonPoolType::LowPriority);
            let high = provider.get_thread_pool(RayonPoolType::HighPriority);

            assert!(
                high.current_num_threads() >= low.current_num_threads(),
                "high priority pool ({}) should have >= threads than low priority pool ({})",
                high.current_num_threads(),
                low.current_num_threads()
            );
        }
    }

    #[test]
    fn thread_counts_match_percentages() {
        let cpus = num_cpus::get();
        let expected_low =
            (cpus * DEFAULT_LOW_PRIORITY_CPU_PERCENTAGE / 100).max(MINIMUM_THREAD_COUNT);
        let expected_high =
            (cpus * DEFAULT_HIGH_PRIORITY_CPU_PERCENTAGE / 100).max(MINIMUM_THREAD_COUNT);

        let provider = RayonPoolProvider::default();

        assert_eq!(
            provider
                .get_thread_pool(RayonPoolType::LowPriority)
                .current_num_threads(),
            expected_low,
            "low priority thread count"
        );
        assert_eq!(
            provider
                .get_thread_pool(RayonPoolType::HighPriority)
                .current_num_threads(),
            expected_high,
            "high priority thread count"
        );
    }

    #[test]
    fn get_thread_pool_returns_same_arc() {
        let provider = RayonPoolProvider::default();
        let pool1 = provider.get_thread_pool(RayonPoolType::HighPriority);
        let pool2 = provider.get_thread_pool(RayonPoolType::HighPriority);
        assert!(Arc::ptr_eq(&pool1, &pool2), "same Arc for same pool type");

        let low = provider.get_thread_pool(RayonPoolType::LowPriority);
        assert!(
            !Arc::ptr_eq(&pool1, &low),
            "different Arcs for different pool types"
        );
    }

    #[test]
    fn pools_execute_work() {
        let provider = RayonPoolProvider::default();
        let pool = provider.get_thread_pool(RayonPoolType::LowPriority);

        let result = pool.install(|| (0..100).map(|i| i * 2).sum::<i64>());
        assert_eq!(result, 9900);
    }
}
