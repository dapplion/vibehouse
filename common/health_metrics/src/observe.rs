use eth2::lighthouse::{Health, ProcessHealth, SystemHealth};

#[cfg(target_os = "linux")]
use procfs::{Current, CurrentSI};

pub trait Observe: Sized {
    fn observe() -> Result<Self, String>;
}

impl Observe for Health {
    #[cfg(not(target_os = "linux"))]
    fn observe() -> Result<Self, String> {
        Err("Health is only available on Linux".into())
    }

    #[cfg(target_os = "linux")]
    fn observe() -> Result<Self, String> {
        Ok(Self {
            process: ProcessHealth::observe()?,
            system: SystemHealth::observe()?,
        })
    }
}

impl Observe for SystemHealth {
    #[cfg(not(target_os = "linux"))]
    fn observe() -> Result<Self, String> {
        Err("Health is only available on Linux".into())
    }

    #[cfg(target_os = "linux")]
    fn observe() -> Result<Self, String> {
        let meminfo =
            procfs::Meminfo::current().map_err(|e| format!("Unable to get meminfo: {:?}", e))?;
        let loadavg = procfs::LoadAverage::current()
            .map_err(|e| format!("Unable to get loadavg: {:?}", e))?;
        let kernel_stats = procfs::KernelStats::current()
            .map_err(|e| format!("Unable to get kernel stats: {:?}", e))?;

        // Disk usage via statvfs
        let disk_usage = {
            let path = std::ffi::CString::new("/").unwrap();
            let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
            let ret = unsafe { libc::statvfs(path.as_ptr(), &mut stat) };
            if ret != 0 {
                return Err("Unable to get disk usage info".to_string());
            }
            (
                stat.f_blocks * stat.f_frsize, // total
                stat.f_bfree * stat.f_frsize,  // free
            )
        };

        // Sum disk I/O across all devices
        let disk_stats =
            procfs::diskstats().map_err(|e| format!("Unable to get disk counters: {:?}", e))?;
        let (disk_reads, disk_writes) =
            disk_stats.iter().fold((0u64, 0u64), |(reads, writes), d| {
                (
                    reads.saturating_add(d.reads),
                    writes.saturating_add(d.writes),
                )
            });

        // Sum network I/O across all interfaces
        let net_status = procfs::net::InterfaceDeviceStatus::current()
            .map_err(|e| format!("Unable to get network io counters: {:?}", e))?;
        let (net_recv, net_sent) =
            net_status
                .0
                .values()
                .fold((0u64, 0u64), |(recv, sent): (u64, u64), d| {
                    (
                        recv.saturating_add(d.recv_bytes),
                        sent.saturating_add(d.sent_bytes),
                    )
                });

        // CPU times: procfs reports in ticks, convert to seconds
        let tps = procfs::ticks_per_second();
        let cpu = &kernel_stats.total;
        let cpu_total_ticks = cpu.user
            + cpu.nice
            + cpu.system
            + cpu.idle
            + cpu.iowait.unwrap_or(0)
            + cpu.irq.unwrap_or(0)
            + cpu.softirq.unwrap_or(0)
            + cpu.steal.unwrap_or(0);

        // Memory percent: (total - available) / total * 100
        let mem_total = meminfo.mem_total;
        let mem_available = meminfo.mem_available.unwrap_or(meminfo.mem_free);
        let mem_used = mem_total.saturating_sub(mem_available);
        let mem_percent = if mem_total > 0 {
            (mem_used as f32 / mem_total as f32) * 100.0
        } else {
            0.0
        };

        Ok(Self {
            sys_virt_mem_total: mem_total,
            sys_virt_mem_available: mem_available,
            sys_virt_mem_used: mem_used,
            sys_virt_mem_free: meminfo.mem_free,
            sys_virt_mem_cached: meminfo.cached,
            sys_virt_mem_buffers: meminfo.buffers,
            sys_virt_mem_percent: mem_percent,
            sys_loadavg_1: loadavg.one as f64,
            sys_loadavg_5: loadavg.five as f64,
            sys_loadavg_15: loadavg.fifteen as f64,
            cpu_cores: num_cpus::get_physical() as u64,
            cpu_threads: num_cpus::get() as u64,
            system_seconds_total: cpu.system / tps,
            cpu_time_total: cpu_total_ticks / tps,
            user_seconds_total: cpu.user / tps,
            iowait_seconds_total: cpu.iowait.unwrap_or(0) / tps,
            idle_seconds_total: cpu.idle / tps,
            disk_node_bytes_total: disk_usage.0,
            disk_node_bytes_free: disk_usage.1,
            disk_node_reads_total: disk_reads,
            disk_node_writes_total: disk_writes,
            network_node_bytes_total_received: net_recv,
            network_node_bytes_total_transmit: net_sent,
            misc_node_boot_ts_seconds: kernel_stats.btime,
            misc_os: std::env::consts::OS.to_string(),
        })
    }
}

impl Observe for ProcessHealth {
    #[cfg(not(target_os = "linux"))]
    fn observe() -> Result<Self, String> {
        Err("Health is only available on Linux".into())
    }

    #[cfg(target_os = "linux")]
    fn observe() -> Result<Self, String> {
        let me = procfs::process::Process::myself()
            .map_err(|e| format!("Unable to get current process: {:?}", e))?;
        let stat = me
            .stat()
            .map_err(|e| format!("Unable to get stat: {:?}", e))?;
        let status = me
            .status()
            .map_err(|e| format!("Unable to get process status: {:?}", e))?;

        let page_size = procfs::page_size();
        let tps = procfs::ticks_per_second();

        // Process CPU time: utime + stime + cutime + cstime, in seconds
        let process_seconds = (stat.utime + stat.stime) / tps
            + (stat.cutime.unsigned_abs() + stat.cstime.unsigned_abs()) / tps;

        Ok(Self {
            pid: stat.pid as u32,
            pid_num_threads: stat.num_threads,
            pid_mem_resident_set_size: stat.rss * page_size,
            pid_mem_virtual_memory_size: stat.vsize,
            pid_mem_shared_memory_size: status.rssshmem.unwrap_or(0),
            pid_process_seconds_total: process_seconds,
        })
    }
}
