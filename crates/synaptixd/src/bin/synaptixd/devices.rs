use std::sync::Mutex;
use std::time::{Duration, Instant};

use synaptix_core::device::Device;
use synaptixd::DeviceKind;

struct Probe {
    npu: Option<bool>,
    npu_at: Option<Instant>,
    cuda: Option<bool>,
}

static PROBE: Mutex<Probe> = Mutex::new(Probe { npu: None, npu_at: None, cuda: None });

pub static NPU_LOCK: Mutex<()> = Mutex::new(());
pub static CUDA_LOCK: Mutex<()> = Mutex::new(());

/// Программа, которой служба уступила видеокарту (DaVinci Resolve и др.):
/// пока она запущена, CUDA для моделей недоступна.
static CUDA_YIELDED: Mutex<Option<String>> = Mutex::new(None);

pub fn cuda_yielded() -> Option<String> {
    CUDA_YIELDED.lock().unwrap().clone()
}

pub fn set_cuda_yielded(who: Option<String>) {
    *CUDA_YIELDED.lock().unwrap() = who;
}

pub fn init() {
    synaptix_kernels_cpu::ensure_registered();
}

fn cuda_ok() -> bool {
    #[cfg(feature = "cuda")]
    {
        if synaptix_core::device::cuda::get(0).is_ok() {
            synaptix_kernels_cuda::ensure_registered();
            return true;
        }
    }
    false
}

fn npu_ok() -> bool {
    #[cfg(feature = "npu")]
    {
        match synaptix_kernels_npu::register() {
            Ok(info) => {
                tracing::info!("NPU: {} ({})", info.model.name, info.model.id);
                return true;
            }
            Err(e) => tracing::info!("NPU недоступен: {e}"),
        }
    }
    false
}

pub fn available(kind: DeviceKind) -> bool {
    let mut p = PROBE.lock().unwrap();
    match kind {
        DeviceKind::Auto | DeviceKind::Cpu => true,
        DeviceKind::Gpu => false,
        DeviceKind::Cuda => cuda_yielded().is_none() && *p.cuda.get_or_insert_with(cuda_ok),
        DeviceKind::Npu => {
            let retry = p.npu == Some(false) && p.npu_at.is_none_or(|t| t.elapsed() > Duration::from_secs(30));
            if p.npu.is_none() || retry {
                p.npu = Some(npu_ok());
                p.npu_at = Some(Instant::now());
            }
            p.npu == Some(true)
        }
    }
}

pub fn list() -> Vec<DeviceKind> {
    [DeviceKind::Cpu, DeviceKind::Cuda, DeviceKind::Npu].into_iter().filter(|&k| available(k)).collect()
}

pub fn resolve(want: DeviceKind, supported: &[DeviceKind]) -> Option<DeviceKind> {
    let ok = |k: DeviceKind| supported.contains(&k) && available(k);
    match want {
        DeviceKind::Auto => [DeviceKind::Cuda, DeviceKind::Npu, DeviceKind::Cpu].into_iter().find(|&k| ok(k)),
        k if ok(k) => Some(k),
        _ => None,
    }
}

pub fn device(kind: DeviceKind) -> Device {
    match kind {
        DeviceKind::Cuda => Device::Cuda(0),
        DeviceKind::Npu => Device::Npu(0),
        _ => Device::Cpu,
    }
}

pub fn lock(kind: DeviceKind) -> Option<std::sync::MutexGuard<'static, ()>> {
    match kind {
        DeviceKind::Npu => Some(NPU_LOCK.lock().unwrap_or_else(|e| e.into_inner())),
        DeviceKind::Cuda => Some(CUDA_LOCK.lock().unwrap_or_else(|e| e.into_inner())),
        _ => None,
    }
}
