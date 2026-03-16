use std::time::{SystemTime, UNIX_EPOCH};

use eth2::vibehouse::{ProcessHealth, SystemHealth};
use serde::{Deserialize, Serialize};

pub const VERSION: u64 = 1;
pub const CLIENT_NAME: &str = "vibehouse";

/// An API error serializable to JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorMessage {
    pub code: u16,
    pub message: String,
    #[serde(default)]
    pub stacktraces: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitoringMetrics {
    #[serde(flatten)]
    pub metadata: Metadata,
    #[serde(flatten)]
    pub process_metrics: Process,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessType {
    BeaconNode,
    Validator,
    System,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    version: u64,
    timestamp: u128,
    process: ProcessType,
}

impl Metadata {
    pub fn new(process: ProcessType) -> Self {
        Self {
            version: VERSION,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time should be greater than unix epoch")
                .as_millis(),
            process,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Process {
    Beacon(BeaconProcessMetrics),
    System(SystemMetrics),
    Validator(ValidatorProcessMetrics),
}

/// Common metrics for all processes.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessMetrics {
    cpu_process_seconds_total: u64,
    memory_process_bytes: u64,

    client_name: String,
    client_version: String,
    client_build: u64,
}

impl From<ProcessHealth> for ProcessMetrics {
    fn from(health: ProcessHealth) -> Self {
        Self {
            cpu_process_seconds_total: health.pid_process_seconds_total,
            memory_process_bytes: health.pid_mem_resident_set_size,
            client_name: CLIENT_NAME.to_string(),
            client_version: client_version().unwrap_or_default(),
            client_build: client_build(),
        }
    }
}

/// Metrics related to the system.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemMetrics {
    cpu_cores: u64,
    cpu_threads: u64,
    cpu_node_system_seconds_total: u64,
    cpu_node_user_seconds_total: u64,
    cpu_node_iowait_seconds_total: u64,
    cpu_node_idle_seconds_total: u64,

    memory_node_bytes_total: u64,
    memory_node_bytes_free: u64,
    memory_node_bytes_cached: u64,
    memory_node_bytes_buffers: u64,

    disk_node_bytes_total: u64,
    disk_node_bytes_free: u64,

    disk_node_io_seconds: u64,
    disk_node_reads_total: u64,
    disk_node_writes_total: u64,

    network_node_bytes_total_receive: u64,
    network_node_bytes_total_transmit: u64,

    misc_node_boot_ts_seconds: u64,
    misc_os: String,
}

impl From<SystemHealth> for SystemMetrics {
    fn from(health: SystemHealth) -> Self {
        // Export format uses 3 letter os names
        let misc_os = health.misc_os.get(0..3).unwrap_or("unk").to_string();
        Self {
            cpu_cores: health.cpu_cores,
            cpu_threads: health.cpu_threads,
            cpu_node_system_seconds_total: health.cpu_time_total,
            cpu_node_user_seconds_total: health.user_seconds_total,
            cpu_node_iowait_seconds_total: health.iowait_seconds_total,
            cpu_node_idle_seconds_total: health.idle_seconds_total,

            memory_node_bytes_total: health.sys_virt_mem_total,
            memory_node_bytes_free: health.sys_virt_mem_free,
            memory_node_bytes_cached: health.sys_virt_mem_cached,
            memory_node_bytes_buffers: health.sys_virt_mem_buffers,

            disk_node_bytes_total: health.disk_node_bytes_total,
            disk_node_bytes_free: health.disk_node_bytes_free,

            // Unavaliable for now
            disk_node_io_seconds: 0,
            disk_node_reads_total: health.disk_node_reads_total,
            disk_node_writes_total: health.disk_node_writes_total,

            network_node_bytes_total_receive: health.network_node_bytes_total_received,
            network_node_bytes_total_transmit: health.network_node_bytes_total_transmit,

            misc_node_boot_ts_seconds: health.misc_node_boot_ts_seconds,
            misc_os,
        }
    }
}

/// All beacon process metrics.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeaconProcessMetrics {
    #[serde(flatten)]
    pub common: ProcessMetrics,
    #[serde(flatten)]
    pub beacon: serde_json::Value,
}

/// All validator process metrics
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidatorProcessMetrics {
    #[serde(flatten)]
    pub common: ProcessMetrics,
    #[serde(flatten)]
    pub validator: serde_json::Value,
}

/// Returns the client version
fn client_version() -> Option<String> {
    let re = regex::Regex::new(r"\d+\.\d+\.\d+").expect("Regex is valid");
    re.find(vibehouse_version::VERSION)
        .map(|m| m.as_str().to_string())
}

/// Returns the client build
/// Note: Vibehouse does not support build numbers, this is effectively a null-value.
fn client_build() -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_system_health() -> SystemHealth {
        SystemHealth {
            sys_virt_mem_total: 16_000_000,
            sys_virt_mem_available: 12_000_000,
            sys_virt_mem_used: 4_000_000,
            sys_virt_mem_free: 8_000_000,
            sys_virt_mem_percent: 25.0,
            sys_virt_mem_cached: 4_000_000,
            sys_virt_mem_buffers: 1_000_000,
            sys_loadavg_1: 1.0,
            sys_loadavg_5: 0.8,
            sys_loadavg_15: 0.5,
            cpu_cores: 4,
            cpu_threads: 8,
            system_seconds_total: 200,
            user_seconds_total: 50,
            iowait_seconds_total: 5,
            idle_seconds_total: 45,
            cpu_time_total: 100,
            disk_node_bytes_total: 500_000_000,
            disk_node_bytes_free: 250_000_000,
            disk_node_reads_total: 1000,
            disk_node_writes_total: 2000,
            network_node_bytes_total_received: 5000,
            network_node_bytes_total_transmit: 3000,
            misc_node_boot_ts_seconds: 1700000000,
            misc_os: "linux".to_string(),
        }
    }

    #[test]
    fn error_message_serde_roundtrip() {
        let msg = ErrorMessage {
            code: 404,
            message: "Not found".to_string(),
            stacktraces: vec!["frame1".to_string(), "frame2".to_string()],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: ErrorMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn error_message_stacktraces_default_empty() {
        let json = r#"{"code":500,"message":"error"}"#;
        let msg: ErrorMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.code, 500);
        assert!(msg.stacktraces.is_empty());
    }

    #[test]
    fn process_type_serde_lowercase() {
        assert_eq!(
            serde_json::to_string(&ProcessType::BeaconNode).unwrap(),
            r#""beaconnode""#
        );
        assert_eq!(
            serde_json::to_string(&ProcessType::Validator).unwrap(),
            r#""validator""#
        );
        assert_eq!(
            serde_json::to_string(&ProcessType::System).unwrap(),
            r#""system""#
        );
    }

    #[test]
    fn process_type_deserialize() {
        assert_eq!(
            serde_json::from_str::<ProcessType>(r#""beaconnode""#).unwrap(),
            ProcessType::BeaconNode
        );
        assert_eq!(
            serde_json::from_str::<ProcessType>(r#""validator""#).unwrap(),
            ProcessType::Validator
        );
    }

    #[test]
    fn metadata_new_sets_version_and_timestamp() {
        let m = Metadata::new(ProcessType::BeaconNode);
        assert_eq!(m.version, VERSION);
        assert_eq!(m.process, ProcessType::BeaconNode);
        assert!(m.timestamp > 0);
    }

    #[test]
    fn metadata_serde_roundtrip() {
        let m = Metadata::new(ProcessType::System);
        let json = serde_json::to_string(&m).unwrap();
        let decoded: Metadata = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, m);
    }

    #[test]
    fn process_metrics_from_process_health() {
        let health = ProcessHealth {
            pid: 1234,
            pid_num_threads: 10,
            pid_mem_resident_set_size: 1024 * 1024,
            pid_mem_virtual_memory_size: 2048 * 1024,
            pid_mem_shared_memory_size: 512 * 1024,
            pid_process_seconds_total: 42,
        };
        let pm: ProcessMetrics = health.into();
        assert_eq!(pm.cpu_process_seconds_total, 42);
        assert_eq!(pm.memory_process_bytes, 1024 * 1024);
        assert_eq!(pm.client_name, CLIENT_NAME);
        assert_eq!(pm.client_build, 0);
    }

    #[test]
    fn system_metrics_from_system_health() {
        let sm: SystemMetrics = sample_system_health().into();
        assert_eq!(sm.cpu_cores, 4);
        assert_eq!(sm.cpu_threads, 8);
        assert_eq!(sm.cpu_node_system_seconds_total, 100);
        assert_eq!(sm.cpu_node_user_seconds_total, 50);
        assert_eq!(sm.memory_node_bytes_total, 16_000_000);
        assert_eq!(sm.disk_node_io_seconds, 0);
        assert_eq!(sm.network_node_bytes_total_receive, 5000);
        assert_eq!(sm.misc_os, "lin");
    }

    #[test]
    fn system_metrics_short_os_falls_back_to_unk() {
        let mut health = sample_system_health();
        health.misc_os = "ab".to_string();
        let sm: SystemMetrics = health.into();
        assert_eq!(sm.misc_os, "unk");
    }

    #[test]
    fn system_metrics_empty_os_falls_back_to_unk() {
        let mut health = sample_system_health();
        health.misc_os = String::new();
        let sm: SystemMetrics = health.into();
        assert_eq!(sm.misc_os, "unk");
    }

    #[test]
    fn system_metrics_exact_3_char_os() {
        let mut health = sample_system_health();
        health.misc_os = "win".to_string();
        let sm: SystemMetrics = health.into();
        assert_eq!(sm.misc_os, "win");
    }

    #[test]
    fn process_metrics_default() {
        let pm = ProcessMetrics::default();
        assert_eq!(pm.cpu_process_seconds_total, 0);
        assert_eq!(pm.memory_process_bytes, 0);
        assert!(pm.client_name.is_empty());
    }

    #[test]
    fn system_metrics_serde_roundtrip() {
        let sm = SystemMetrics {
            cpu_cores: 8,
            cpu_threads: 16,
            misc_os: "lin".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&sm).unwrap();
        let decoded: SystemMetrics = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, sm);
    }

    #[test]
    fn beacon_process_metrics_serde_roundtrip() {
        let bpm = BeaconProcessMetrics {
            common: ProcessMetrics::default(),
            beacon: serde_json::json!({"sync_eth2_synced": true}),
        };
        let json = serde_json::to_string(&bpm).unwrap();
        let decoded: BeaconProcessMetrics = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, bpm);
    }

    #[test]
    fn validator_process_metrics_serde_roundtrip() {
        let vpm = ValidatorProcessMetrics {
            common: ProcessMetrics::default(),
            validator: serde_json::json!({"validator_active": 5}),
        };
        let json = serde_json::to_string(&vpm).unwrap();
        let decoded: ValidatorProcessMetrics = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, vpm);
    }

    #[test]
    fn client_version_extracts_semver() {
        if let Some(v) = client_version() {
            assert!(v.contains('.'), "Expected semver, got: {}", v);
        }
    }

    #[test]
    fn client_build_is_zero() {
        assert_eq!(client_build(), 0);
    }

    #[test]
    fn constants() {
        assert_eq!(VERSION, 1);
        assert_eq!(CLIENT_NAME, "vibehouse");
    }

    #[test]
    fn error_message_clone_eq() {
        let msg = ErrorMessage {
            code: 200,
            message: "ok".to_string(),
            stacktraces: vec![],
        };
        assert_eq!(msg.clone(), msg);
    }
}
