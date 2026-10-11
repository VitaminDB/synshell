//! Сборщик: раз в секунду в фоновом потоке снимает все ресурсы и копит историю для
//! графиков. Дорогое (видеопамять по процессам, модели synaptixd, размеры каталогов) —
//! реже и только для открытой страницы.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use syngui::prelude::*;
use synsystem::Sys;

use crate::state::Page;

/// Сколько секунд истории держать.
pub const HISTORY: usize = 300;
/// Сколько секунд истории у ядер (мини-графики).
pub const CORE_HISTORY: usize = 60;

/// История значения по секундам, старое первым.
#[derive(Clone, Default)]
pub struct Hist(pub VecDeque<f32>);

impl Hist {
    fn push(&mut self, v: f32, cap: usize) {
        self.0.push_back(v);
        while self.0.len() > cap {
            self.0.pop_front();
        }
    }

    /// Последние `n` значений.
    pub fn tail(&self, n: usize) -> Vec<f32> {
        let skip = self.0.len().saturating_sub(n);
        self.0.iter().skip(skip).copied().collect()
    }

    pub fn last(&self) -> f32 {
        self.0.back().copied().unwrap_or(0.0)
    }

    pub fn max(&self, n: usize) -> f32 {
        self.tail(n).into_iter().fold(0.0, f32::max)
    }
}

#[derive(Clone, Default)]
pub struct History {
    pub cpu: Hist,
    pub cores: Vec<Hist>,
    pub mem: Hist,
    pub swap: Hist,
    /// По GPU: загрузка, %.
    pub gpu: Vec<Hist>,
    /// По GPU: видеопамять, байт.
    pub vram: Vec<Hist>,
    pub npu: Hist,
    pub rx: Hist,
    pub tx: Hist,
    pub disk_read: Hist,
    pub disk_write: Hist,
    pub cpu_temp: Hist,
    pub gpu_temp: Hist,
    pub battery: Hist,
    pub power: Hist,
}

/// Интерфейс со скоростью.
#[derive(Clone, Default)]
pub struct Iface {
    pub info: synsystem::network::Interface,
    pub rx_bps: f64,
    pub tx_bps: f64,
    pub addrs: Vec<String>,
}

/// Модель нейросети synaptixd на устройстве.
#[derive(Clone)]
pub struct Model {
    pub title: String,
    pub device: String,
    pub state: synaptixd::LoadState,
    pub mem_bytes: u64,
    pub vram_bytes: u64,
    pub busy: bool,
    pub clients: u32,
}

/// Окно приложения (от композитора) с памятью и процессором его процессов.
#[derive(Clone, Default)]
pub struct App {
    pub window: u64,
    pub app_id: String,
    pub name: String,
    pub title: String,
    pub pid: Option<i32>,
    /// Процесс и потомки.
    pub pids: Vec<i32>,
    pub rss_kb: u64,
    pub cpu: f32,
}

#[derive(Clone, Default)]
pub struct Snap {
    pub ticks: u64,
    pub cpu: synsystem::cpu::CpuSnapshot,
    pub cpu_model: Option<String>,
    pub device_model: Option<String>,
    pub kernel: String,
    pub clusters: Vec<synsystem::cpu::Cluster>,
    pub load: Option<([f32; 3], u32)>,
    pub uptime: u64,
    pub mem: synsystem::memory::Detail,
    pub swaps: Vec<(String, String, u64, u64)>,
    /// PSI: процессор, память, ввод-вывод, % за 10 с.
    pub pressure: [Option<f32>; 3],
    pub gpus: Vec<synsystem::gpu::GpuDevice>,
    /// Видеопамять по процессам: pid, имя, байт.
    pub gpu_procs: Vec<(i32, String, u64)>,
    pub npu: Option<synsystem::npu::Npu>,
    pub npu_cache: Option<u64>,
    pub models: Option<Vec<Model>>,
    pub net: Vec<Iface>,
    pub disks: Vec<synsystem::disk::Disk>,
    pub mounts: Vec<synsystem::disk::Mount>,
    pub temps: Vec<synsystem::thermal::Sensor>,
    pub cpu_temp: Option<f32>,
    pub gpu_temp: Option<f32>,
    pub battery: Option<synsystem::battery::Battery>,
    pub batteries: Vec<synsystem::battery::BatteryInfo>,
    pub adc_volt: Option<f32>,
    pub procs: Vec<synsystem::procs::Process>,
    /// Окна (нет композитора synwm — None).
    pub apps: Option<Vec<App>>,
    pub hist: History,
}

impl Snap {
    pub fn rx(&self) -> f64 {
        self.net.iter().map(|i| i.rx_bps).sum()
    }

    pub fn tx(&self) -> f64 {
        self.net.iter().map(|i| i.tx_bps).sum()
    }

    pub fn disk_read(&self) -> f64 {
        self.disks.iter().map(|d| d.read_bps).sum()
    }

    pub fn disk_write(&self) -> f64 {
        self.disks.iter().map(|d| d.write_bps).sum()
    }
}

/// Открытая страница — для сборщика (что снимать подробно).
static PAGE: AtomicU8 = AtomicU8::new(0);

pub fn set_page(p: Page) {
    PAGE.store(Page::ALL.iter().position(|x| *x == p).unwrap_or(0) as u8, Ordering::Relaxed);
}

fn page() -> Page {
    Page::ALL.get(PAGE.load(Ordering::Relaxed) as usize).copied().unwrap_or(Page::Overview)
}

/// Запустить сборщик: первый снимок сразу, дальше раз в секунду.
pub fn start(sig: RwSignal<Snap>) {
    std::thread::Builder::new()
        .name("sampler".into())
        .spawn(move || {
            let mut s = Sampler::new();
            // Сразу — то, что есть без разности (память, диски, датчики), загрузка — через 0,3 с.
            let first = s.tick();
            run_on_main_thread(move || sig.set_always(first));
            std::thread::sleep(Duration::from_millis(300));
            let second = s.tick();
            run_on_main_thread(move || sig.set_always(second));
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let snap = s.tick();
                run_on_main_thread(move || sig.set_always(snap));
            }
        })
        .ok();
}

/// Один снимок без истории (снимок экрана `--screenshot`): два замера с паузой — чтобы
/// загрузка процессора была не нулевой.
pub fn once() -> Snap {
    let mut s = Sampler::new();
    s.tick();
    std::thread::sleep(Duration::from_millis(400));
    s.tick()
}

struct Sampler {
    sys: Sys,
    cpu: synsystem::cpu::CpuSampler,
    procs: synsystem::procs::ProcessTable,
    npu: synsystem::npu::NpuSampler,
    disks: synsystem::disk::DiskSampler,
    last_net: HashMap<String, (u64, u64)>,
    last_at: Option<std::time::Instant>,
    snap: Snap,
}

impl Sampler {
    fn new() -> Self {
        let sys = Sys::host();
        let snap = Snap {
            cpu_model: synsystem::cpu::model(&sys),
            device_model: synsystem::cpu::device_model(&sys),
            kernel: kernel(),
            clusters: synsystem::cpu::clusters(&sys),
            ..Default::default()
        };
        Sampler {
            cpu: synsystem::cpu::CpuSampler::new(sys.clone()),
            procs: synsystem::procs::ProcessTable::new(sys.clone()),
            npu: synsystem::npu::NpuSampler::new(sys.clone()),
            disks: synsystem::disk::DiskSampler::new(sys.clone()),
            last_net: HashMap::new(),
            last_at: None,
            snap,
            sys,
        }
    }

    fn tick(&mut self) -> Snap {
        let sys = &self.sys;
        let page = page();
        let s = &mut self.snap;
        let n = s.ticks;
        let every = |k: u64| n % k == 0;
        let now = std::time::Instant::now();
        let dt = self.last_at.map(|t| now.duration_since(t).as_secs_f64()).filter(|d| *d > 0.05);
        self.last_at = Some(now);

        s.cpu = self.cpu.sample();
        s.load = synsystem::cpu::load_average(sys);
        s.uptime = synsystem::cpu::uptime_secs(sys).unwrap_or(0);
        s.mem = synsystem::memory::detail(sys).unwrap_or_default();
        if every(5) {
            s.swaps = synsystem::memory::swaps(sys);
        }
        s.pressure = ["cpu", "memory", "io"].map(|r| synsystem::memory::pressure(sys, r));
        s.gpus = synsystem::gpu::devices(sys);
        if matches!(page, Page::Vram | Page::Gpu) && every(2) {
            let names: HashMap<i32, String> = s.procs.iter().map(|p| (p.pid, p.name.clone())).collect();
            s.gpu_procs = synsystem::gpu::process_memory(sys)
                .into_iter()
                .map(|(pid, b)| (pid, names.get(&pid).cloned().or_else(|| sys.read(format!("/proc/{pid}/comm"))).unwrap_or_else(|| pid.to_string()), b))
                .collect();
        }
        s.npu = self.npu.sample();
        if page == Page::Npu || page == Page::Gpu {
            if s.npu.as_ref().is_some_and(|n| n.vendor == "qualcomm") && (s.npu_cache.is_none() || every(30)) {
                s.npu_cache = Some(synsystem::npu::dir_size(sys, synsystem::npu::GRAPH_CACHE));
            }
            if every(3) || s.models.is_none() {
                s.models = models();
            }
        }
        // Сеть: скорость по разности счётчиков.
        let ifaces = synsystem::network::interfaces(sys);
        let addrs = if every(5) || s.net.is_empty() { Some(synsystem::network::addresses()) } else { None };
        let old: HashMap<String, Vec<String>> = s.net.iter().map(|i| (i.info.name.clone(), i.addrs.clone())).collect();
        s.net = ifaces
            .into_iter()
            .map(|info| {
                let (rx_bps, tx_bps) = match (dt, self.last_net.get(&info.name)) {
                    (Some(dt), Some(&(r0, t0))) => (info.rx_bytes.saturating_sub(r0) as f64 / dt, info.tx_bytes.saturating_sub(t0) as f64 / dt),
                    _ => (0.0, 0.0),
                };
                let a = match &addrs {
                    Some(m) => m.get(&info.name).cloned().unwrap_or_default(),
                    None => old.get(&info.name).cloned().unwrap_or_default(),
                };
                Iface { rx_bps, tx_bps, addrs: a, info }
            })
            .collect();
        self.last_net = s.net.iter().map(|i| (i.info.name.clone(), (i.info.rx_bytes, i.info.tx_bytes))).collect();
        s.disks = self.disks.sample();
        if every(5) || s.mounts.is_empty() {
            s.mounts = synsystem::disk::mounts(sys);
        }
        if every(2) || s.temps.is_empty() {
            let mut t = synsystem::thermal::sensors(sys);
            s.cpu_temp = synsystem::thermal::cpu_celsius(&t);
            s.gpu_temp = synsystem::thermal::gpu_celsius(&t).or_else(|| s.gpus.iter().find_map(|g| g.temp_c));
            t.sort_by(|a, b| b.celsius.total_cmp(&a.celsius));
            s.temps = t;
        }
        s.battery = synsystem::battery::read(sys);
        s.batteries = synsystem::battery::batteries(sys);
        s.adc_volt = if s.battery.is_none() { synsystem::battery::adc_voltage(sys) } else { None };
        if every(2) || s.procs.is_empty() {
            s.procs = self.procs.sample();
            s.apps = apps(&s.procs);
        }

        // История.
        let h = &mut s.hist;
        h.cpu.push(s.cpu.usage, HISTORY);
        h.cores.resize(s.cpu.cores.len(), Hist::default());
        for (hc, c) in h.cores.iter_mut().zip(&s.cpu.cores) {
            hc.push(c.usage, CORE_HISTORY);
        }
        h.mem.push(pct(s.mem.used(), s.mem.total), HISTORY);
        h.swap.push(pct(s.mem.swap_used(), s.mem.swap_total), HISTORY);
        h.gpu.resize(s.gpus.len(), Hist::default());
        h.vram.resize(s.gpus.len(), Hist::default());
        for (i, g) in s.gpus.iter().enumerate() {
            h.gpu[i].push(g.busy_percent.unwrap_or(0.0), HISTORY);
            h.vram[i].push(g.mem_used.unwrap_or(0) as f32, HISTORY);
        }
        h.npu.push(s.npu.as_ref().and_then(|n| n.busy_percent).unwrap_or(0.0), HISTORY);
        h.rx.push(s.net.iter().map(|i| i.rx_bps).sum::<f64>() as f32, HISTORY);
        h.tx.push(s.net.iter().map(|i| i.tx_bps).sum::<f64>() as f32, HISTORY);
        h.disk_read.push(s.disks.iter().map(|d| d.read_bps).sum::<f64>() as f32, HISTORY);
        h.disk_write.push(s.disks.iter().map(|d| d.write_bps).sum::<f64>() as f32, HISTORY);
        h.cpu_temp.push(s.cpu_temp.unwrap_or(0.0), HISTORY);
        h.gpu_temp.push(s.gpu_temp.unwrap_or(0.0), HISTORY);
        h.battery.push(s.battery.as_ref().map(|b| b.percent as f32).unwrap_or(0.0), HISTORY);
        h.power.push(s.battery.as_ref().and_then(|b| b.power_w).unwrap_or(0.0), HISTORY);
        s.ticks += 1;
        s.clone()
    }
}

fn pct(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        used as f32 / total as f32 * 100.0
    }
}

/// Окна композитора с суммой по процессу окна и его потомкам.
fn apps(procs: &[synsystem::procs::Process]) -> Option<Vec<App>> {
    use synshell_common::ipc::{Client, Request, Response};
    let Ok(Response::Windows { windows }) = Client::connect().ok()?.request(&Request::Windows) else { return None };
    let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
    for p in procs {
        children.entry(p.ppid).or_default().push(p.pid);
    }
    let by_pid: HashMap<i32, &synsystem::procs::Process> = procs.iter().map(|p| (p.pid, p)).collect();
    let mut out: Vec<App> = windows
        .into_iter()
        .filter(|w| !w.skip_taskbar)
        .map(|w| {
            let mut pids = Vec::new();
            if let Some(p) = w.pid {
                let mut stack = vec![p];
                while let Some(x) = stack.pop() {
                    if pids.len() > 256 {
                        break;
                    }
                    pids.push(x);
                    stack.extend(children.get(&x).into_iter().flatten().copied());
                }
            }
            let (rss_kb, cpu) = pids.iter().filter_map(|p| by_pid.get(p)).fold((0, 0.0), |a, p| (a.0 + p.rss_kb, a.1 + p.cpu_percent));
            let name = synshell_common::xdg::app_for_window(&w.app_id).map(|e| e.name).filter(|n| !n.is_empty()).unwrap_or_else(|| w.app_id.clone());
            App { window: w.id, app_id: w.app_id, name, title: w.title, pid: w.pid, pids, rss_kb, cpu }
        })
        .collect();
    out.sort_by(|a, b| b.rss_kb.cmp(&a.rss_kb));
    Some(out)
}

/// Модели synaptixd (нет службы — None).
fn models() -> Option<Vec<Model>> {
    let mut c = synaptixd::Client::connect().ok()?;
    let list = c.list().ok()?;
    Some(
        list.into_iter()
            .map(|m| Model {
                title: m.title,
                device: m.device.map(|d| d.id().to_string()).unwrap_or_default(),
                state: m.state,
                mem_bytes: m.mem_bytes,
                vram_bytes: m.vram_bytes,
                busy: m.busy,
                clients: m.clients,
            })
            .collect(),
    )
}

/// Версия ядра (uname -r).
fn kernel() -> String {
    // SAFETY: uname заполняет переданную структуру.
    let mut u: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut u) } != 0 {
        return String::new();
    }
    let f = |a: &[libc::c_char]| unsafe { std::ffi::CStr::from_ptr(a.as_ptr()) }.to_string_lossy().into_owned();
    format!("{} {} ({})", f(&u.sysname), f(&u.release), f(&u.machine))
}
