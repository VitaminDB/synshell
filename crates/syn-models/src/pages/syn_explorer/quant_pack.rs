//! Квантование весов при упаковке — у службы нейросетей: квантующий поток
//! живёт в `synaptix::facade::pack`, служба исполняет его операцией `pack`
//! на своей карте (Synthos своего CUDA не держит).

pub use synaptix::facade::pack::QuantDecision;

/// Есть ли карта NVIDIA, на которой служба посчитает квант: драйвер
/// загружен (без NVML и CUDA-контекста в «Моделях»).
pub fn cuda_available() -> bool {
    std::path::Path::new("/proc/driver/nvidia/version").exists()
}
