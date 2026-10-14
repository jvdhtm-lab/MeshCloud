//! Local resource detection for building resource offers.

use mesh_protocol::{NodeCapabilities, ResourceLimits};

use crate::config::WorkerConfig;

/// Detect the capabilities this worker advertises, honoring config overrides.
pub fn detect_capabilities(config: &WorkerConfig) -> NodeCapabilities {
    NodeCapabilities {
        cpu_cores: config.cpu_cores.unwrap_or_else(detect_cpu_cores),
        ram_gb: config.ram_gb.unwrap_or_else(detect_ram_gb),
        storage_gb: config.storage_gb,
        gpu: config.gpu,
    }
}

/// Build the per-workload resource limits from configuration.
pub fn limits(config: &WorkerConfig) -> ResourceLimits {
    ResourceLimits {
        max_cpu_percent: config.max_cpu_percent,
        max_memory_mb: config.max_memory_mb,
    }
}

fn detect_cpu_cores() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1)
}

fn detect_ram_gb() -> u32 {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    let bytes = system.total_memory();
    (bytes / 1024 / 1024 / 1024).max(1) as u32
}
