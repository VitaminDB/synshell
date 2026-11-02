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

/// Очередь к устройству с приоритетами: работы идут по одной, ждущий с
/// более высоким приоритетом проходит первым, а длинная генерация между
/// токенами уступает его ждущим ([`yield_point`]) — ASR и TTS в диалоге не
/// ждут конца фоновой генерации текста.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Prio {
    /// Фоновая работа (предгенерация контента): уступает всем.
    Background = 0,
    Normal = 1,
    /// Короткие ответы живому человеку: ASR, TTS, CTC/MFA, эмбеддинги.
    Interactive = 2,
}

impl Prio {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "background" | "low" => Some(Prio::Background),
            "normal" => Some(Prio::Normal),
            "interactive" | "high" => Some(Prio::Interactive),
            _ => None,
        }
    }
}

pub struct Gate {
    st: Mutex<GateState>,
    cv: std::sync::Condvar,
    kind: DeviceKind,
}

struct GateState {
    busy: bool,
    waiting: [usize; 3],
}

impl GateState {
    fn higher(&self, p: Prio) -> bool {
        self.waiting[p as usize + 1..].iter().any(|n| *n > 0)
    }
}

impl Gate {
    const fn new(kind: DeviceKind) -> Self {
        Self { st: Mutex::new(GateState { busy: false, waiting: [0; 3] }), cv: std::sync::Condvar::new(), kind }
    }

    fn acquire(&'static self, p: Prio) -> GateGuard {
        let mut s = self.st.lock().unwrap_or_else(|e| e.into_inner());
        s.waiting[p as usize] += 1;
        while s.busy || s.higher(p) {
            s = self.cv.wait(s).unwrap_or_else(|e| e.into_inner());
        }
        s.waiting[p as usize] -= 1;
        s.busy = true;
        HELD.with(|h| h.set(Some((self, p))));
        GateGuard { gate: self }
    }

    /// Уступить устройство ждущим с более высоким приоритетом и дождаться
    /// своей очереди. `true` — уступили.
    fn yield_to_higher(&'static self, p: Prio) -> bool {
        if !self.st.lock().unwrap_or_else(|e| e.into_inner()).higher(p) {
            return false;
        }
        // Своё в очереди устройства — досчитать до передачи.
        sync_device(self.kind);
        let mut s = self.st.lock().unwrap_or_else(|e| e.into_inner());
        s.busy = false;
        s.waiting[p as usize] += 1;
        self.cv.notify_all();
        let t0 = Instant::now();
        // Даём им зайти: пока кто-то выше ждёт или занято — ждём.
        while s.busy || s.higher(p) {
            s = self.cv.wait(s).unwrap_or_else(|e| e.into_inner());
        }
        s.waiting[p as usize] -= 1;
        s.busy = true;
        tracing::debug!("{:?}: уступил устройство на {:.0} мс", self.kind, t0.elapsed().as_secs_f64() * 1e3);
        true
    }
}

pub struct GateGuard {
    gate: &'static Gate,
}

impl Drop for GateGuard {
    fn drop(&mut self) {
        HELD.with(|h| h.set(None));
        let mut s = self.gate.st.lock().unwrap_or_else(|e| e.into_inner());
        s.busy = false;
        self.gate.cv.notify_all();
    }
}

static NPU_GATE: Gate = Gate::new(DeviceKind::Npu);
static CUDA_GATE: Gate = Gate::new(DeviceKind::Cuda);

thread_local! {
    /// Приоритет работы, которую сейчас выполняет поток (ставит диспетчер).
    static PRIO: std::cell::Cell<Prio> = const { std::cell::Cell::new(Prio::Normal) };
    /// Устройство, которое поток держит, и с каким приоритетом.
    static HELD: std::cell::Cell<Option<(&'static Gate, Prio)>> = const { std::cell::Cell::new(None) };
}

pub fn set_priority(p: Prio) {
    PRIO.with(|c| c.set(p));
}

/// Точка, где длинная работа может уступить устройство (между токенами).
pub fn yield_point() -> bool {
    match HELD.with(|h| h.get()) {
        Some((g, p)) => g.yield_to_higher(p),
        None => false,
    }
}

fn sync_device(kind: DeviceKind) {
    #[cfg(feature = "cuda")]
    if kind == DeviceKind::Cuda {
        let _ = synaptix_core::device::cuda::synchronize_all(0);
    }
    let _ = kind;
}

/// Программа, которой служба уступила видеокарту (DaVinci Resolve и др.):
/// пока она запущена, модели не грузятся на CUDA (см. [`resolve`]).
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
        DeviceKind::Cuda => *p.cuda.get_or_insert_with(cuda_ok),
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
    // Видеокарта уступлена программе — модели идут на NPU/CPU.
    let ok = |k: DeviceKind| supported.contains(&k) && available(k) && !(k == DeviceKind::Cuda && cuda_yielded().is_some());
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

/// Занять устройство с приоритетом текущей работы потока.
pub fn lock(kind: DeviceKind) -> Option<GateGuard> {
    let p = PRIO.with(|c| c.get());
    match kind {
        DeviceKind::Npu => Some(NPU_GATE.acquire(p)),
        DeviceKind::Cuda => Some(CUDA_GATE.acquire(p)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// Длинная фоновая работа с точками уступки пропускает интерактивную
    /// вперёд, а не держит устройство до конца.
    #[test]
    fn interactive_cuts_in() {
        static G: Gate = Gate::new(DeviceKind::Cpu);
        let done_bg = Arc::new(AtomicUsize::new(0));
        let d = done_bg.clone();
        let bg = std::thread::spawn(move || {
            let _g = G.acquire(Prio::Background);
            for i in 0..100 {
                std::thread::sleep(Duration::from_millis(10));
                d.store(i, Ordering::SeqCst);
                yield_point();
            }
        });
        std::thread::sleep(Duration::from_millis(50));
        let t0 = Instant::now();
        {
            let _g = G.acquire(Prio::Interactive);
        }
        let waited = t0.elapsed();
        assert!(waited < Duration::from_millis(200), "ждал {waited:?}");
        assert!(done_bg.load(Ordering::SeqCst) < 50);
        bg.join().unwrap();
    }
}
