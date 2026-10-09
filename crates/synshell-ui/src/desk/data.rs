//! Данные виджетов рабочего стола: снимок раз в секунду в фоновом потоке
//! (процессор и ядра, память, графика, температуры, питание, сеть,
//! запущенные приложения) с историей для графиков. Снимается, только пока
//! виджеты с данными видны ([`set_visible`]).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use syngui::prelude::*;

use crate::ShellCtx;

/// Сколько секунд истории держать (самый длинный график).
pub const HISTORY: usize = 300;

/// Снимок для виджетов.
#[derive(Clone, Default, PartialEq)]
pub struct Snap {
    pub cpu: f32,
    /// Ядра: загрузка, частота МГц, класс (0 — медленные).
    pub cores: Vec<(f32, Option<u32>, usize)>,
    pub cpu_temp: Option<f32>,
    pub gpu_temp: Option<f32>,
    /// Датчики температуры: имя, °C.
    pub temps: Vec<(String, f32)>,
    pub mem_percent: f32,
    pub mem_used_kb: u64,
    pub mem_total_kb: u64,
    pub gpu: Option<(Option<f32>, Option<u32>, Option<String>)>,
    pub battery: Option<synsystem::battery::Battery>,
    pub battery_info: Option<synsystem::battery::BatteryInfo>,
    /// Напряжение из АЦП, если драйвера батареи нет.
    pub adc_volt: Option<f32>,
    /// Байт/с принято, передано.
    pub net: (f64, f64),
    /// Приложения: окно, app_id, имя, память КБ, CPU %.
    pub apps: Vec<AppStat>,
    pub history: History,
}

/// История по секундам, старое — первым.
#[derive(Clone, Default, PartialEq)]
pub struct History {
    pub cpu: Vec<f32>,
    pub mem: Vec<f32>,
    pub gpu: Vec<f32>,
    pub battery: Vec<f32>,
    pub rx: Vec<f64>,
    pub tx: Vec<f64>,
    pub temp: Vec<f32>,
}

#[derive(Clone, PartialEq)]
pub struct AppStat {
    pub window: u64,
    pub app_id: String,
    pub title: String,
    pub mem_kb: u64,
    pub cpu: f32,
}

static VISIBLE: AtomicBool = AtomicBool::new(false);

/// Окна для ленты: (id окна, app_id, заголовок, pid) — из главного потока.
fn windows_slot() -> &'static Mutex<Vec<(u64, String, String, i32)>> {
    static W: OnceLock<Mutex<Vec<(u64, String, String, i32)>>> = OnceLock::new();
    W.get_or_init(|| Mutex::new(Vec::new()))
}

thread_local! {
    static SNAP: std::cell::Cell<Option<RwSignal<Snap>>> = const { std::cell::Cell::new(None) };
}

/// Сигнал снимка; первый вызов запускает сборщик.
pub fn snap() -> RwSignal<Snap> {
    SNAP.with(|s| match s.get() {
        Some(x) => x,
        None => {
            let x = use_signal(Snap::default());
            s.set(Some(x));
            track_windows(ShellCtx::get());
            start_sampler(x);
            x
        }
    })
}

/// Виджеты с данными на экране — снимать; нет — не тратить батарею.
pub fn set_visible(v: bool) {
    VISIBLE.store(v, Ordering::Relaxed);
}

fn push<T>(q: &mut VecDeque<T>, v: T) {
    q.push_back(v);
    while q.len() > HISTORY {
        q.pop_front();
    }
}

fn start_sampler(sig: RwSignal<Snap>) {
    std::thread::Builder::new()
        .name("desk-data".into())
        .spawn(move || {
            let sys = synsystem::Sys::host();
            let mut cpu = synsystem::cpu::CpuSampler::new(sys.clone());
            let mut procs = synsystem::procs::ProcSampler::new(sys.clone());
            let (mut h_cpu, mut h_mem, mut h_gpu, mut h_bat, mut h_rx, mut h_tx, mut h_temp) =
                (VecDeque::new(), VecDeque::new(), VecDeque::new(), VecDeque::new(), VecDeque::new(), VecDeque::new(), VecDeque::new());
            let mut last_net: Option<(u64, u64)> = None;
            cpu.sample();
            loop {
                std::thread::sleep(Duration::from_secs(1));
                if !VISIBLE.load(Ordering::Relaxed) {
                    // Счётчики сети обновятся при показе.
                    last_net = None;
                    continue;
                }
                let c = cpu.sample();
                let temps = synsystem::thermal::sensors(&sys);
                let mem = synsystem::memory::read(&sys).unwrap_or_default();
                let (rx, tx) = synsystem::network::traffic(&sys);
                let net = match last_net {
                    Some((r0, t0)) => (rx.saturating_sub(r0) as f64, tx.saturating_sub(t0) as f64),
                    None => (0.0, 0.0),
                };
                last_net = Some((rx, tx));
                let wins = windows_slot().lock().map(|w| w.clone()).unwrap_or_default();
                let groups: Vec<(u64, synsystem::procs::Source)> =
                    wins.iter().map(|(id, app, _, pid)| (*id, usage_source(&sys, app, *pid))).collect();
                let stats = procs.sample_groups(&groups);
                let mut apps: Vec<AppStat> = wins
                    .iter()
                    .map(|(id, app, title, _)| {
                        let s = stats.get(id).copied().unwrap_or_default();
                        AppStat { window: *id, app_id: app.clone(), title: title.clone(), mem_kb: s.memory_kb, cpu: s.cpu_percent }
                    })
                    .collect();
                apps.sort_by(|a, b| b.mem_kb.cmp(&a.mem_kb));
                let gpu = synsystem::gpu::read(&sys).map(|g| (g.busy_percent, g.cur_mhz, g.name));
                let battery = synsystem::battery::read(&sys);
                let cpu_temp = synsystem::thermal::cpu_celsius(&temps);
                push(&mut h_cpu, c.usage);
                push(&mut h_mem, mem.used_percent());
                push(&mut h_gpu, gpu.as_ref().and_then(|g| g.0).unwrap_or(0.0));
                push(&mut h_bat, battery.as_ref().map(|b| b.percent as f32).unwrap_or(0.0));
                push(&mut h_rx, net.0);
                push(&mut h_tx, net.1);
                push(&mut h_temp, cpu_temp.unwrap_or(0.0));
                let snap = Snap {
                    cpu: c.usage,
                    cores: c.cores.iter().map(|k| (k.usage, k.cur_mhz, k.tier)).collect(),
                    cpu_temp,
                    gpu_temp: synsystem::thermal::gpu_celsius(&temps),
                    temps: temp_list(&temps),
                    mem_percent: mem.used_percent(),
                    mem_used_kb: mem.used_kb(),
                    mem_total_kb: mem.total_kb,
                    gpu,
                    battery,
                    battery_info: synsystem::battery::batteries(&sys).into_iter().next(),
                    adc_volt: synsystem::battery::adc_voltage(&sys),
                    net,
                    apps,
                    history: History {
                        cpu: h_cpu.iter().copied().collect(),
                        mem: h_mem.iter().copied().collect(),
                        gpu: h_gpu.iter().copied().collect(),
                        battery: h_bat.iter().copied().collect(),
                        rx: h_rx.iter().copied().collect(),
                        tx: h_tx.iter().copied().collect(),
                        temp: h_temp.iter().copied().collect(),
                    },
                };
                syngui::async_runtime::run_on_main_thread(move || {
                    if sig.get_untracked() != snap {
                        sig.set(snap);
                    }
                });
            }
        })
        .ok();
}

/// Датчики для виджета температур: зоны одного узла (`cpu-1-2-usr`,
/// `cpu-0-0-usr` → `cpu`) — одной строкой с самой высокой, горячие первыми.
fn temp_list(temps: &[synsystem::thermal::Sensor]) -> Vec<(String, f32)> {
    let mut v: Vec<(String, f32)> = Vec::new();
    for t in temps {
        let base = t.name.strip_prefix("hwmon:").unwrap_or(&t.name);
        // Номер зоны или ядра в конце (`cpu-1-2-usr`, `Core 3`) — одна строка на узел.
        let cut = base.char_indices().find(|(i, c)| c.is_ascii_digit() && *i > 0 && base[..*i].ends_with(['-', '_', ' '])).map(|(i, _)| i).unwrap_or(base.len());
        let name = base[..cut].trim_end_matches(['-', '_', ' ']).to_string();
        let name = if name.is_empty() { base.to_string() } else { name };
        if let Some(x) = v.iter_mut().find(|x| x.0 == name) {
            x.1 = x.1.max(t.celsius);
        } else {
            v.push((name, t.celsius));
        }
    }
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v
}

/// Держать список окон для ленты в курсе (pid нужен фоновому потоку).
fn track_windows(ctx: ShellCtx) {
    create_effect(move || {
        let wins = ctx.windows.get();
        let v: Vec<(u64, String, String, i32)> = wins
            .iter()
            .filter(|w| !w.skip_taskbar)
            .filter_map(|w| Some((w.id, w.app_id.clone(), w.title.clone(), w.pid?)))
            .collect();
        if let Ok(mut slot) = windows_slot().lock() {
            *slot = v;
        }
    });
}

/// Что считать за приложение окна. Окна Android рисует один процесс hwcomposer контейнера
/// syndroid, его собственная память — не память Android: окно всего Android (app_id
/// `Waydroid`) — весь контейнер (cgroup syndroid), окно приложения (`waydroid.<пакет>`) — процессы
/// Android с именем пакета.
fn usage_source(sys: &synsystem::Sys, app_id: &str, pid: i32) -> synsystem::procs::Source {
    use synsystem::procs::Source;
    const ANDROID_CGROUP: &str = "/sys/fs/cgroup/syndroid";
    if app_id == "Waydroid" {
        return Source::Cgroup(ANDROID_CGROUP.into());
    }
    if let Some(pkg) = app_id.strip_prefix("waydroid.") {
        let pids = synsystem::procs::find_by_name(sys, pkg);
        if !pids.is_empty() {
            return Source::Pids(pids);
        }
    }
    Source::Pids(vec![pid])
}
