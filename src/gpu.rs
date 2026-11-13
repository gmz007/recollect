//! Cheap GPU detection used to pick a default model (ADR 0002).

use eframe::wgpu;
use std::sync::atomic::{AtomicBool, Ordering};

static FORCE_CPU: AtomicBool = AtomicBool::new(false);

/// Disables GPU use for this process (CLI `--cpu`, tests, benchmarks).
pub fn force_cpu(v: bool) {
    FORCE_CPU.store(v, Ordering::Relaxed);
}

pub fn cpu_forced() -> bool {
    FORCE_CPU.load(Ordering::Relaxed) || std::env::var_os("RECOLLECT_NO_GPU").is_some()
}

/// True if a discrete GPU is visible through Vulkan, Metal or DirectX 12.
/// Software rasterizers and integrated GPUs return false.
pub fn has_discrete_gpu() -> bool {
    if cpu_forced() {
        return false;
    }
    let desc = wgpu::InstanceDescriptor::new_without_display_handle();
    let instance = wgpu::Instance::new(desc);
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::PRIMARY));
    adapters.iter().any(|a| {
        let info = a.get_info();
        tracing::debug!(
            "GPU adapter: {} ({:?}, {:?})",
            info.name,
            info.device_type,
            info.backend
        );
        info.device_type == wgpu::DeviceType::DiscreteGpu
    })
}
