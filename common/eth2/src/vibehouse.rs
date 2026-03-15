//! This module contains endpoints that are non-standard and only available on vibehouse servers.

mod attestation_performance;
mod block_packing_efficiency;
mod block_rewards;
mod custody;
pub mod sync_state;

use crate::{
    BeaconNodeHttpClient, DepositData, Error, Hash256, Slot,
    types::{AdminPeer, Epoch, GenericResponse, ValidatorId},
    vibehouse::sync_state::SyncState,
};
use proto_array::core::ProtoArray;
use serde::{Deserialize, Serialize};
use ssz::four_byte_option_impl;
use ssz_derive::{Decode, Encode};

pub use attestation_performance::{
    AttestationPerformance, AttestationPerformanceQuery, AttestationPerformanceStatistics,
};
pub use block_packing_efficiency::{
    BlockPackingEfficiency, BlockPackingEfficiencyQuery, ProposerInfo, UniqueAttestation,
};
pub use block_rewards::{AttestationRewards, BlockReward, BlockRewardMeta, BlockRewardsQuery};
pub use custody::CustodyInfo;

// Define "legacy" implementations of `Option<T>` which use four bytes for encoding the union
// selector.
four_byte_option_impl!(four_byte_option_u64, u64);
four_byte_option_impl!(four_byte_option_hash256, Hash256);

/// The results of validators voting during an epoch.
///
/// Provides information about the current and previous epochs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalValidatorInclusionData {
    /// The total effective balance of all active validators during the _current_ epoch.
    pub current_epoch_active_gwei: u64,
    /// The total effective balance of all validators who attested during the _current_ epoch and
    /// agreed with the state about the beacon block at the first slot of the _current_ epoch.
    pub current_epoch_target_attesting_gwei: u64,
    /// The total effective balance of all validators who attested during the _previous_ epoch and
    /// agreed with the state about the beacon block at the first slot of the _previous_ epoch.
    pub previous_epoch_target_attesting_gwei: u64,
    /// The total effective balance of all validators who attested during the _previous_ epoch and
    /// agreed with the state about the beacon block at the time of attestation.
    pub previous_epoch_head_attesting_gwei: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidatorInclusionData {
    /// True if the validator has been slashed, ever.
    pub is_slashed: bool,
    /// True if the validator can withdraw in the current epoch.
    pub is_withdrawable_in_current_epoch: bool,
    /// True if the validator was active and not slashed in the state's _current_ epoch.
    pub is_active_unslashed_in_current_epoch: bool,
    /// True if the validator was active and not slashed in the state's _previous_ epoch.
    pub is_active_unslashed_in_previous_epoch: bool,
    /// The validator's effective balance in the _current_ epoch.
    pub current_epoch_effective_balance_gwei: u64,
    /// True if the validator's beacon block root attestation for the first slot of the _current_
    /// epoch matches the block root known to the state.
    pub is_current_epoch_target_attester: bool,
    /// True if the validator's beacon block root attestation for the first slot of the _previous_
    /// epoch matches the block root known to the state.
    pub is_previous_epoch_target_attester: bool,
    /// True if the validator's beacon block root attestation in the _previous_ epoch at the
    /// attestation's slot (`attestation_data.slot`) matches the block root known to the state.
    pub is_previous_epoch_head_attester: bool,
}

/// Reports on the health of the Vibehouse instance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Health {
    #[serde(flatten)]
    pub system: SystemHealth,
    #[serde(flatten)]
    pub process: ProcessHealth,
}

/// System related health.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemHealth {
    /// Total virtual memory on the system
    pub sys_virt_mem_total: u64,
    /// Total virtual memory available for new processes.
    pub sys_virt_mem_available: u64,
    /// Total virtual memory used on the system.
    pub sys_virt_mem_used: u64,
    /// Total virtual memory not used on the system.
    pub sys_virt_mem_free: u64,
    /// Percentage of virtual memory used on the system.
    pub sys_virt_mem_percent: f32,
    /// Total cached virtual memory on the system.
    pub sys_virt_mem_cached: u64,
    /// Total buffered virtual memory on the system.
    pub sys_virt_mem_buffers: u64,

    /// System load average over 1 minute.
    pub sys_loadavg_1: f64,
    /// System load average over 5 minutes.
    pub sys_loadavg_5: f64,
    /// System load average over 15 minutes.
    pub sys_loadavg_15: f64,

    /// Total cpu cores.
    pub cpu_cores: u64,
    /// Total cpu threads.
    pub cpu_threads: u64,

    /// Total time spent in kernel mode.
    pub system_seconds_total: u64,
    /// Total time spent in user mode.
    pub user_seconds_total: u64,
    /// Total time spent in waiting for io.
    pub iowait_seconds_total: u64,
    /// Total idle cpu time.
    pub idle_seconds_total: u64,
    /// Total cpu time.
    pub cpu_time_total: u64,

    /// Total capacity of disk.
    pub disk_node_bytes_total: u64,
    /// Free space in disk.
    pub disk_node_bytes_free: u64,
    /// Number of disk reads.
    pub disk_node_reads_total: u64,
    /// Number of disk writes.
    pub disk_node_writes_total: u64,

    /// Total bytes received over all network interfaces.
    pub network_node_bytes_total_received: u64,
    /// Total bytes sent over all network interfaces.
    pub network_node_bytes_total_transmit: u64,

    /// Boot time
    pub misc_node_boot_ts_seconds: u64,
    /// OS
    pub misc_os: String,
}

/// Process specific health
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProcessHealth {
    /// The pid of this process.
    pub pid: u32,
    /// The number of threads used by this pid.
    pub pid_num_threads: i64,
    /// The total resident memory used by this pid.
    pub pid_mem_resident_set_size: u64,
    /// The total virtual memory used by this pid.
    pub pid_mem_virtual_memory_size: u64,
    /// The total shared memory used by this pid.
    pub pid_mem_shared_memory_size: u64,
    /// Number of cpu seconds consumed by this pid.
    pub pid_process_seconds_total: u64,
}

/// A fully parsed eth1 deposit contract log.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct DepositLog {
    pub deposit_data: DepositData,
    /// The block number of the log that included this `DepositData`.
    pub block_number: u64,
    /// The index included with the deposit log.
    pub index: u64,
    /// True if the signature is valid.
    pub signature_is_valid: bool,
}

impl BeaconNodeHttpClient {
    /// `GET vibehouse/health`
    pub async fn get_vibehouse_health(&self) -> Result<GenericResponse<Health>, Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("vibehouse")
            .push("health");

        self.get(path).await
    }

    /// `GET vibehouse/syncing`
    pub async fn get_vibehouse_syncing(&self) -> Result<GenericResponse<SyncState>, Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("vibehouse")
            .push("syncing");

        self.get(path).await
    }

    /// `GET vibehouse/custody/info`
    pub async fn get_vibehouse_custody_info(&self) -> Result<CustodyInfo, Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("vibehouse")
            .push("custody")
            .push("info");

        self.get(path).await
    }

    /// `POST vibehouse/custody/backfill`
    pub async fn post_vibehouse_custody_backfill(&self) -> Result<(), Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("vibehouse")
            .push("custody")
            .push("backfill");

        self.post(path, &()).await
    }

    /*
     * Note:
     *
     * The `vibehouse/peers` endpoints do not have functions here. We are yet to implement
     * `Deserialize` on the `PeerInfo` struct since it contains use of `Instant`. This could be
     * fairly simply achieved, if desired.
     */

    /// `GET vibehouse/proto_array`
    pub async fn get_vibehouse_proto_array(&self) -> Result<GenericResponse<ProtoArray>, Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("vibehouse")
            .push("proto_array");

        self.get(path).await
    }

    /// `GET vibehouse/validator_inclusion/{epoch}/global`
    pub async fn get_vibehouse_validator_inclusion_global(
        &self,
        epoch: Epoch,
    ) -> Result<GenericResponse<GlobalValidatorInclusionData>, Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("vibehouse")
            .push("validator_inclusion")
            .push(&epoch.to_string())
            .push("global");

        self.get(path).await
    }

    /// `GET vibehouse/validator_inclusion/{epoch}/{validator_id}`
    pub async fn get_vibehouse_validator_inclusion(
        &self,
        epoch: Epoch,
        validator_id: ValidatorId,
    ) -> Result<GenericResponse<Option<ValidatorInclusionData>>, Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("vibehouse")
            .push("validator_inclusion")
            .push(&epoch.to_string())
            .push(&validator_id.to_string());

        self.get(path).await
    }

    /// `POST vibehouse/database/reconstruct`
    pub async fn post_vibehouse_database_reconstruct(&self) -> Result<String, Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("vibehouse")
            .push("database")
            .push("reconstruct");

        self.post_with_response(path, &()).await
    }

    /// `POST vibehouse/add_peer`
    pub async fn post_vibehouse_add_peer(&self, req: AdminPeer) -> Result<(), Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("vibehouse")
            .push("add_peer");

        self.post_with_response(path, &req).await
    }

    /// `POST vibehouse/remove_peer`
    pub async fn post_vibehouse_remove_peer(&self, req: AdminPeer) -> Result<(), Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("vibehouse")
            .push("remove_peer");

        self.post_with_response(path, &req).await
    }

    /*
     Analysis endpoints.
    */

    /// `GET` vibehouse/analysis/block_rewards?start_slot,end_slot
    pub async fn get_vibehouse_analysis_block_rewards(
        &self,
        start_slot: Slot,
        end_slot: Slot,
    ) -> Result<Vec<BlockReward>, Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("vibehouse")
            .push("analysis")
            .push("block_rewards");

        path.query_pairs_mut()
            .append_pair("start_slot", &start_slot.to_string())
            .append_pair("end_slot", &end_slot.to_string());

        self.get(path).await
    }

    /// `GET` vibehouse/analysis/block_packing?start_epoch,end_epoch
    pub async fn get_vibehouse_analysis_block_packing(
        &self,
        start_epoch: Epoch,
        end_epoch: Epoch,
    ) -> Result<Vec<BlockPackingEfficiency>, Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("vibehouse")
            .push("analysis")
            .push("block_packing_efficiency");

        path.query_pairs_mut()
            .append_pair("start_epoch", &start_epoch.to_string())
            .append_pair("end_epoch", &end_epoch.to_string());

        self.get(path).await
    }

    /// `GET` vibehouse/analysis/attestation_performance/{index}?start_epoch,end_epoch
    pub async fn get_vibehouse_analysis_attestation_performance(
        &self,
        start_epoch: Epoch,
        end_epoch: Epoch,
        target: String,
    ) -> Result<Vec<AttestationPerformance>, Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("vibehouse")
            .push("analysis")
            .push("attestation_performance")
            .push(&target);

        path.query_pairs_mut()
            .append_pair("start_epoch", &start_epoch.to_string())
            .append_pair("end_epoch", &end_epoch.to_string());

        self.get(path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::FixedBytesExtended;

    #[test]
    fn global_validator_inclusion_data_serde_roundtrip() {
        let data = GlobalValidatorInclusionData {
            current_epoch_active_gwei: 1_000_000,
            current_epoch_target_attesting_gwei: 800_000,
            previous_epoch_target_attesting_gwei: 750_000,
            previous_epoch_head_attesting_gwei: 700_000,
        };
        let json = serde_json::to_string(&data).unwrap();
        let decoded: GlobalValidatorInclusionData = serde_json::from_str(&json).unwrap();
        assert_eq!(data, decoded);
    }

    #[test]
    fn global_validator_inclusion_data_zero_values() {
        let data = GlobalValidatorInclusionData {
            current_epoch_active_gwei: 0,
            current_epoch_target_attesting_gwei: 0,
            previous_epoch_target_attesting_gwei: 0,
            previous_epoch_head_attesting_gwei: 0,
        };
        let json = serde_json::to_string(&data).unwrap();
        let decoded: GlobalValidatorInclusionData = serde_json::from_str(&json).unwrap();
        assert_eq!(data, decoded);
    }

    #[test]
    fn validator_inclusion_data_serde_roundtrip() {
        let data = ValidatorInclusionData {
            is_slashed: false,
            is_withdrawable_in_current_epoch: true,
            is_active_unslashed_in_current_epoch: true,
            is_active_unslashed_in_previous_epoch: false,
            current_epoch_effective_balance_gwei: 32_000_000_000,
            is_current_epoch_target_attester: true,
            is_previous_epoch_target_attester: false,
            is_previous_epoch_head_attester: true,
        };
        let json = serde_json::to_string(&data).unwrap();
        let decoded: ValidatorInclusionData = serde_json::from_str(&json).unwrap();
        assert_eq!(data, decoded);
    }

    #[test]
    fn validator_inclusion_data_slashed_validator() {
        let data = ValidatorInclusionData {
            is_slashed: true,
            is_withdrawable_in_current_epoch: false,
            is_active_unslashed_in_current_epoch: false,
            is_active_unslashed_in_previous_epoch: false,
            current_epoch_effective_balance_gwei: 16_000_000_000,
            is_current_epoch_target_attester: false,
            is_previous_epoch_target_attester: false,
            is_previous_epoch_head_attester: false,
        };
        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("\"is_slashed\":true"));
        let decoded: ValidatorInclusionData = serde_json::from_str(&json).unwrap();
        assert_eq!(data, decoded);
    }

    #[test]
    fn system_health_serde_roundtrip() {
        let health = SystemHealth {
            sys_virt_mem_total: 16_000_000_000,
            sys_virt_mem_available: 8_000_000_000,
            sys_virt_mem_used: 7_000_000_000,
            sys_virt_mem_free: 1_000_000_000,
            sys_virt_mem_percent: 43.75,
            sys_virt_mem_cached: 4_000_000_000,
            sys_virt_mem_buffers: 500_000_000,
            sys_loadavg_1: 1.5,
            sys_loadavg_5: 1.2,
            sys_loadavg_15: 0.9,
            cpu_cores: 4,
            cpu_threads: 8,
            system_seconds_total: 100_000,
            user_seconds_total: 200_000,
            iowait_seconds_total: 5_000,
            idle_seconds_total: 300_000,
            cpu_time_total: 605_000,
            disk_node_bytes_total: 500_000_000_000,
            disk_node_bytes_free: 200_000_000_000,
            disk_node_reads_total: 1_000_000,
            disk_node_writes_total: 500_000,
            network_node_bytes_total_received: 10_000_000_000,
            network_node_bytes_total_transmit: 5_000_000_000,
            misc_node_boot_ts_seconds: 1_700_000_000,
            misc_os: "Linux".to_string(),
        };
        let json = serde_json::to_string(&health).unwrap();
        let decoded: SystemHealth = serde_json::from_str(&json).unwrap();
        assert_eq!(health, decoded);
    }

    #[test]
    fn process_health_serde_roundtrip() {
        let health = ProcessHealth {
            pid: 12345,
            pid_num_threads: 42,
            pid_mem_resident_set_size: 500_000_000,
            pid_mem_virtual_memory_size: 1_000_000_000,
            pid_mem_shared_memory_size: 100_000_000,
            pid_process_seconds_total: 3600,
        };
        let json = serde_json::to_string(&health).unwrap();
        let decoded: ProcessHealth = serde_json::from_str(&json).unwrap();
        assert_eq!(health, decoded);
    }

    #[test]
    fn health_serde_roundtrip_flattened() {
        let health = Health {
            system: SystemHealth {
                sys_virt_mem_total: 16_000_000_000,
                sys_virt_mem_available: 8_000_000_000,
                sys_virt_mem_used: 7_000_000_000,
                sys_virt_mem_free: 1_000_000_000,
                sys_virt_mem_percent: 43.75,
                sys_virt_mem_cached: 4_000_000_000,
                sys_virt_mem_buffers: 500_000_000,
                sys_loadavg_1: 1.5,
                sys_loadavg_5: 1.2,
                sys_loadavg_15: 0.9,
                cpu_cores: 4,
                cpu_threads: 8,
                system_seconds_total: 100_000,
                user_seconds_total: 200_000,
                iowait_seconds_total: 5_000,
                idle_seconds_total: 300_000,
                cpu_time_total: 605_000,
                disk_node_bytes_total: 500_000_000_000,
                disk_node_bytes_free: 200_000_000_000,
                disk_node_reads_total: 1_000_000,
                disk_node_writes_total: 500_000,
                network_node_bytes_total_received: 10_000_000_000,
                network_node_bytes_total_transmit: 5_000_000_000,
                misc_node_boot_ts_seconds: 1_700_000_000,
                misc_os: "Linux".to_string(),
            },
            process: ProcessHealth {
                pid: 1,
                pid_num_threads: 10,
                pid_mem_resident_set_size: 100_000,
                pid_mem_virtual_memory_size: 200_000,
                pid_mem_shared_memory_size: 50_000,
                pid_process_seconds_total: 100,
            },
        };
        let json = serde_json::to_string(&health).unwrap();
        // Health uses #[serde(flatten)] so system and process fields are at top level
        assert!(json.contains("\"pid\":1"));
        assert!(json.contains("\"sys_virt_mem_total\":16000000000"));
        let decoded: Health = serde_json::from_str(&json).unwrap();
        assert_eq!(health, decoded);
    }

    #[test]
    fn deposit_log_serde_roundtrip() {
        let log = DepositLog {
            deposit_data: DepositData {
                pubkey: types::PublicKeyBytes::empty(),
                withdrawal_credentials: Hash256::zero(),
                amount: 32_000_000_000,
                signature: types::SignatureBytes::empty(),
            },
            block_number: 12345678,
            index: 42,
            signature_is_valid: true,
        };
        let json = serde_json::to_string(&log).unwrap();
        let decoded: DepositLog = serde_json::from_str(&json).unwrap();
        assert_eq!(log, decoded);
    }

    #[test]
    fn deposit_log_ssz_roundtrip() {
        use ssz::{Decode, Encode};
        let log = DepositLog {
            deposit_data: DepositData {
                pubkey: types::PublicKeyBytes::empty(),
                withdrawal_credentials: Hash256::repeat_byte(0xab),
                amount: 32_000_000_000,
                signature: types::SignatureBytes::empty(),
            },
            block_number: 999,
            index: 0,
            signature_is_valid: false,
        };
        let bytes = log.as_ssz_bytes();
        let decoded = DepositLog::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(log, decoded);
    }

    #[test]
    fn deposit_log_invalid_signature() {
        let log = DepositLog {
            deposit_data: DepositData {
                pubkey: types::PublicKeyBytes::empty(),
                withdrawal_credentials: Hash256::zero(),
                amount: 0,
                signature: types::SignatureBytes::empty(),
            },
            block_number: 0,
            index: 0,
            signature_is_valid: false,
        };
        let json = serde_json::to_string(&log).unwrap();
        assert!(json.contains("\"signature_is_valid\":false"));
    }

    #[test]
    fn global_validator_inclusion_data_clone() {
        let data = GlobalValidatorInclusionData {
            current_epoch_active_gwei: 100,
            current_epoch_target_attesting_gwei: 80,
            previous_epoch_target_attesting_gwei: 75,
            previous_epoch_head_attesting_gwei: 70,
        };
        let cloned = data.clone();
        assert_eq!(data, cloned);
    }

    #[test]
    fn validator_inclusion_data_clone() {
        let data = ValidatorInclusionData {
            is_slashed: true,
            is_withdrawable_in_current_epoch: true,
            is_active_unslashed_in_current_epoch: false,
            is_active_unslashed_in_previous_epoch: false,
            current_epoch_effective_balance_gwei: 32_000_000_000,
            is_current_epoch_target_attester: false,
            is_previous_epoch_target_attester: false,
            is_previous_epoch_head_attester: false,
        };
        let cloned = data.clone();
        assert_eq!(data, cloned);
    }

    #[test]
    fn health_clone() {
        let health = Health {
            system: SystemHealth {
                sys_virt_mem_total: 1,
                sys_virt_mem_available: 1,
                sys_virt_mem_used: 0,
                sys_virt_mem_free: 1,
                sys_virt_mem_percent: 0.0,
                sys_virt_mem_cached: 0,
                sys_virt_mem_buffers: 0,
                sys_loadavg_1: 0.0,
                sys_loadavg_5: 0.0,
                sys_loadavg_15: 0.0,
                cpu_cores: 1,
                cpu_threads: 1,
                system_seconds_total: 0,
                user_seconds_total: 0,
                iowait_seconds_total: 0,
                idle_seconds_total: 0,
                cpu_time_total: 0,
                disk_node_bytes_total: 0,
                disk_node_bytes_free: 0,
                disk_node_reads_total: 0,
                disk_node_writes_total: 0,
                network_node_bytes_total_received: 0,
                network_node_bytes_total_transmit: 0,
                misc_node_boot_ts_seconds: 0,
                misc_os: "test".to_string(),
            },
            process: ProcessHealth {
                pid: 1,
                pid_num_threads: 1,
                pid_mem_resident_set_size: 0,
                pid_mem_virtual_memory_size: 0,
                pid_mem_shared_memory_size: 0,
                pid_process_seconds_total: 0,
            },
        };
        let cloned = health.clone();
        assert_eq!(health, cloned);
    }
}
