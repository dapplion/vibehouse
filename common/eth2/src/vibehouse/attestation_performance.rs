use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use types::Epoch;

#[derive(Debug, Default, PartialEq, Clone, Serialize, Deserialize)]
pub struct AttestationPerformanceStatistics {
    pub active: bool,
    pub head: bool,
    pub target: bool,
    pub source: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay: Option<u64>,
}

#[derive(Debug, Default, PartialEq, Clone, Serialize, Deserialize)]
pub struct AttestationPerformance {
    pub index: u64,
    pub epochs: HashMap<u64, AttestationPerformanceStatistics>,
}

impl AttestationPerformance {
    pub fn initialize(indices: Vec<u64>) -> Vec<Self> {
        let mut vec = Vec::with_capacity(indices.len());
        for index in indices {
            vec.push(Self {
                index,
                ..Default::default()
            })
        }
        vec
    }
}

/// Query parameters for the `/vibehouse/analysis/attestation_performance` endpoint.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct AttestationPerformanceQuery {
    pub start_epoch: Epoch,
    pub end_epoch: Epoch,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statistics_default() {
        let stats = AttestationPerformanceStatistics::default();
        assert!(!stats.active);
        assert!(!stats.head);
        assert!(!stats.target);
        assert!(!stats.source);
        assert_eq!(stats.delay, None);
    }

    #[test]
    fn statistics_serde_roundtrip() {
        let stats = AttestationPerformanceStatistics {
            active: true,
            head: true,
            target: false,
            source: true,
            delay: Some(3),
        };
        let json = serde_json::to_string(&stats).unwrap();
        let decoded: AttestationPerformanceStatistics = serde_json::from_str(&json).unwrap();
        assert_eq!(stats, decoded);
    }

    #[test]
    fn statistics_delay_none_skipped() {
        let stats = AttestationPerformanceStatistics {
            active: true,
            head: true,
            target: true,
            source: true,
            delay: None,
        };
        let json = serde_json::to_string(&stats).unwrap();
        assert!(!json.contains("delay"));
    }

    #[test]
    fn performance_initialize() {
        let perfs = AttestationPerformance::initialize(vec![10, 20, 30]);
        assert_eq!(perfs.len(), 3);
        assert_eq!(perfs[0].index, 10);
        assert_eq!(perfs[1].index, 20);
        assert_eq!(perfs[2].index, 30);
        for p in &perfs {
            assert!(p.epochs.is_empty());
        }
    }

    #[test]
    fn performance_initialize_empty() {
        let perfs = AttestationPerformance::initialize(vec![]);
        assert!(perfs.is_empty());
    }

    #[test]
    fn performance_serde_roundtrip() {
        let mut perf = AttestationPerformance {
            index: 5,
            epochs: HashMap::new(),
        };
        perf.epochs.insert(
            10,
            AttestationPerformanceStatistics {
                active: true,
                head: true,
                target: true,
                source: true,
                delay: Some(1),
            },
        );
        let json = serde_json::to_string(&perf).unwrap();
        let decoded: AttestationPerformance = serde_json::from_str(&json).unwrap();
        assert_eq!(perf, decoded);
    }

    #[test]
    fn query_serde_roundtrip() {
        let q = AttestationPerformanceQuery {
            start_epoch: Epoch::new(5),
            end_epoch: Epoch::new(10),
        };
        let json = serde_json::to_string(&q).unwrap();
        let decoded: AttestationPerformanceQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(q, decoded);
    }
}
