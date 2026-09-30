//! Экран ресурсов — первая страница домашнего экрана (`[mobile]
//! resources_page`): загрузка процессора (кольцо, ядра столбиками с частотой
//! и цветом класса ядра — LITTLE/big/prime, история за минуту), память,
//! графика, температуры, питание, сеть и внизу — лента запущенных
//! приложений с бейджами памяти (слева) и процессора (справа).
//!
//! Снимки — в фоновом потоке раз в секунду и только пока страница видна.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use synshell_common::ipc::WindowOp;
use synshell_ui::launchers;
use synshell_ui::ui::rx;
use synshell_ui::ShellCtx;
use syngui::core::Color;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::{PanAxis, SwipeDirection};
use syngui::containers::Keyed;
use syngui::GestureDetector;

/// Снимок для экрана.
#[derive(Clone, Default, PartialEq)]
pub struct Snap {
    pub cpu: f32,
    /// Ядра: загрузка, частота МГц, класс (0 — медленные).
    pub cores: Vec<(f32, Option<u32>, usize)>,
    pub history: Vec<f32>,
    pub cpu_temp: Option<f32>,
    pub gpu_temp: Option<f32>,
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

fn snap_signal() -> RwSignal<Snap> {
    SNAP.with(|s| match s.get() {
        Some(x) => x,
        None => {
            let x = use_signal(Snap::default());
            s.set(Some(x));
            start_sampler(x);
            x
        }
    })
}

/// Страница видна (домашний экран на странице ресурсов) — снимать.
pub fn set_visible(v: bool) {
    VISIBLE.store(v, Ordering::Relaxed);
}

fn start_sampler(sig: RwSignal<Snap>) {
    std::thread::Builder::new()
        .name("resources".into())
        .spawn(move || {
            let sys = synsystem::Sys::host();
            let mut cpu = synsystem::cpu::CpuSampler::new(sys.clone());
            let mut procs = synsystem::procs::ProcSampler::new(sys.clone());
            let mut history: VecDeque<f32> = VecDeque::with_capacity(60);
            let mut last_net: Option<(u64, u64)> = None;
            cpu.sample();
            loop {
                std::thread::sleep(Duration::from_secs(1));
                if !VISIBLE.load(Ordering::Relaxed) {
                    // Страница не видна — не тратить батарею; счётчики
                    // обновятся при показе.
                    last_net = None;
                    continue;
                }
                let c = cpu.sample();
                history.push_back(c.usage);
                while history.len() > 60 {
                    history.pop_front();
                }
                let temps = synsystem::thermal::sensors(&sys);
                let mem = synsystem::memory::read(&sys).unwrap_or_default();
                let (rx, tx) = synsystem::network::traffic(&sys);
                let net = match last_net {
                    Some((r0, t0)) => (rx.saturating_sub(r0) as f64, tx.saturating_sub(t0) as f64),
                    None => (0.0, 0.0),
                };
                last_net = Some((rx, tx));
                let wins = windows_slot().lock().map(|w| w.clone()).unwrap_or_default();
                let pids: Vec<i32> = wins.iter().map(|w| w.3).collect();
                let stats = procs.sample(&pids);
                let mut apps: Vec<AppStat> = wins
                    .iter()
                    .map(|(id, app, title, pid)| {
                        let s = stats.get(pid).copied().unwrap_or_default();
                        AppStat { window: *id, app_id: app.clone(), title: title.clone(), mem_kb: s.memory_kb, cpu: s.cpu_percent }
                    })
                    .collect();
                apps.sort_by(|a, b| b.mem_kb.cmp(&a.mem_kb));
                let snap = Snap {
                    cpu: c.usage,
                    cores: c.cores.iter().map(|k| (k.usage, k.cur_mhz, k.tier)).collect(),
                    history: history.iter().copied().collect(),
                    cpu_temp: synsystem::thermal::cpu_celsius(&temps),
                    gpu_temp: synsystem::thermal::gpu_celsius(&temps),
                    mem_percent: mem.used_percent(),
                    mem_used_kb: mem.used_kb(),
                    mem_total_kb: mem.total_kb,
                    gpu: synsystem::gpu::read(&sys).map(|g| (g.busy_percent, g.cur_mhz, g.name)),
                    battery: synsystem::battery::read(&sys),
                    battery_info: synsystem::battery::batteries(&sys).into_iter().next(),
                    adc_volt: synsystem::battery::adc_voltage(&sys),
                    net,
                    apps,
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

// ─── Виджеты ─────────────────────────────────────────────────────────────────

fn accent(ctx: &ShellCtx) -> Color {
    Color::from_hex(&ctx.cfg().appearance.palette().accent.hex())
}

/// Кольцо загрузки с подписью внутри.
fn ring(value: f32, label: String, sub: &str, size: f32, color: Color) -> impl Widget {
    let v = (value / 100.0).clamp(0.0, 1.0);
    let canvas = Canvas::new(move |c, _| {
        let r = size / 2.0 - 7.0;
        let (cx, cy) = (size / 2.0, size / 2.0);
        c.set_stroke_width(9.0);
        c.set_color(color.with_alpha(0.18));
        c.stroke_circle(cx, cy, r);
        if v > 0.001 {
            c.set_color(color);
            let start = -std::f32::consts::FRAC_PI_2;
            c.draw_arc(cx, cy, r, start, start + v * std::f32::consts::TAU);
        }
    })
    .size(size, size);
    Stack::new()
        .child(canvas)
        .child(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .gap(0.0)
                .child(Text::new(label).class("res-ring-value"))
                .child(Text::new(sub.to_string()).class("res-ring-unit"))
                .style("width", StyleValue::px(size))
                .style("height", StyleValue::px(size)),
        )
}

fn sparkline(values: Vec<f32>, color: Color, w: f32, h: f32) -> impl Widget {
    Canvas::new(move |c, _| {
        if values.len() < 2 {
            return;
        }
        let n = 60usize.max(values.len());
        let step = w / (n - 1) as f32;
        let off = n - values.len();
        let pts: Vec<(f32, f32)> = values.iter().enumerate().map(|(i, v)| ((off + i) as f32 * step, h - (v / 100.0).clamp(0.0, 1.0) * (h - 2.0) - 1.0)).collect();
        c.set_color(color.with_alpha(0.16));
        c.fill_area_strip(&pts, h);
        c.set_color(color);
        c.set_stroke_width(2.0);
        c.draw_polyline(&pts);
    })
    .size(w, h)
}

/// Столбики ядер: загрузка — высота, класс ядра — цвет, внизу частота.
fn cores(list: &[(f32, Option<u32>, usize)]) -> impl Widget {
    const H: f32 = 64.0;
    let mut row = Row::new().gap(4.0).main_axis_alignment(MainAxisAlignment::SpaceBetween).cross_axis_alignment(CrossAxisAlignment::End);
    // На десктопе с 24 ядрами — не больше 16 столбиков (самые загруженные).
    let mut items: Vec<(usize, &(f32, Option<u32>, usize))> = list.iter().enumerate().collect();
    if items.len() > 16 {
        items.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));
        items.truncate(16);
        items.sort_by_key(|x| x.0);
    }
    for (_, (usage, mhz, tier)) in items {
        let fill = (usage / 100.0).clamp(0.0, 1.0) * H;
        let bar = Stack::new()
            .child(DecoratedBox::new().class("res-core-bg").style("height", StyleValue::px(H)))
            .child(
                Column::new()
                    .main_axis_alignment(MainAxisAlignment::End)
                    .child(DecoratedBox::new().class(format!("res-core-fill res-tier-{}", tier.min(&3))).style("height", StyleValue::px(fill.max(2.0))))
                    .style("height", StyleValue::px(H)),
            );
        let label = mhz.map(|m| if m >= 1000 { format!("{:.1}", m as f32 / 1000.0) } else { format!("{:.2}", m as f32 / 1000.0) }).unwrap_or_default();
        row = row.child(
            Column::new()
                .gap(3.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(bar)
                .child(Text::new(label).class("res-core-mhz")),
        );
    }
    row
}

fn human_rate(b: f64) -> String {
    if b >= 1e6 {
        format!("{:.1} МБ/с", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} КБ/с", b / 1e3)
    } else {
        format!("{b:.0} Б/с")
    }
}

fn card<M>(title: &str, glyph: &str, body: impl syngui::IntoWidget<M>) -> impl Widget {
    DecoratedBox::new()
        .child(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(synshell_ui::ui::icon(glyph).class("res-card-icon"))
                        .child(Text::new(title.to_string()).class("res-card-title")),
                )
                .child(body),
        )
        .class("res-card")
}

fn kv(k: &str, v: String) -> impl Widget {
    Row::new().child(Text::new(k.to_string()).class("res-k grow")).child(Text::new(v).class("res-v"))
}

pub fn page(ctx: ShellCtx) -> impl Widget {
    let sig = snap_signal();
    track_windows(ctx);
    let color = accent(&ctx);
    let clock = rx(move || {
        let now = ctx.now.get();
        Box::new(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::End)
                .child(Text::new(synshell_ui::clock::format(now, "%H:%M")).class("res-clock"))
                .child(Text::new(synshell_ui::clock::format(now, "%a, %d %b")).class("res-date grow")),
        )
    });
    let cpu_card = rx(move || {
        let s = sig.get();
        let temp = s.cpu_temp.map(|t| format!("{t:.0} °C")).unwrap_or_default();
        let max_mhz = s.cores.iter().filter_map(|c| c.1).max();
        let top = Row::new()
            .gap(14.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(ring(s.cpu, format!("{:.0}%", s.cpu), "процессор", 96.0, color))
            .child(
                Column::new()
                    .gap(6.0)
                    .child(kv("Ядер", s.cores.len().to_string()))
                    .child(kv("Частота", max_mhz.map(|m| format!("до {m} МГц")).unwrap_or_default()))
                    .child(kv("Температура", temp))
                    .child(sparkline(s.history.clone(), color, 170.0, 34.0))
                    .class("grow"),
            );
        Box::new(card("Процессор", synshell_ui::ui::mi::CPU, Column::new().gap(12.0).child(top).child(cores(&s.cores))))
    });
    let mem_gpu = rx(move || {
        let s = sig.get();
        let mem = card(
            "Память",
            synshell_ui::ui::mi::MEMORY,
            Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(ring(s.mem_percent, format!("{:.0}%", s.mem_percent), &synsystem::memory::human_kb(s.mem_used_kb), 84.0, color))
                .child(Text::new(format!("из {}", synsystem::memory::human_kb(s.mem_total_kb))).class("res-ring-sub")),
        );
        let gpu: Box<dyn Widget> = match &s.gpu {
            Some((busy, mhz, name)) => Box::new(card(
                "Графика",
                "\u{E30A}",
                Column::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(ring(busy.unwrap_or(0.0), busy.map(|b| format!("{b:.0}%")).unwrap_or("—".into()), &mhz.map(|m| format!("{m} МГц")).unwrap_or_default(), 84.0, color))
                    .child(Text::new(format!("{}{}", name.clone().unwrap_or_default(), s.gpu_temp.map(|t| format!(" · {t:.0} °C")).unwrap_or_default())).max_lines(1).class("res-ring-sub")),
            )),
            None => Box::new(DecoratedBox::new()),
        };
        Box::new(Row::new().gap(10.0).child(DecoratedBox::new().child(mem).class("grow res-half")).child(DecoratedBox::new().child(gpu).class("grow res-half")))
    });
    let power = rx(move || {
        let s = sig.get();
        let body: Box<dyn Widget> = match (&s.battery, &s.battery_info) {
            (Some(b), info) => {
                let state = if b.charging { "Заряжается" } else if b.full { "Заряжена" } else { "Разряжается" };
                let mut col = Column::new()
                    .gap(6.0)
                    .child(
                        Row::new()
                            .gap(10.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(Text::new(format!("{}%", b.percent)).class("res-big"))
                            .child(Text::new(state).class("res-v grow")),
                    )
                    .child(synshell_ui::ui::meter(b.percent));
                if let Some(m) = b.minutes {
                    col = col.child(kv(if b.charging { "До полного" } else { "Осталось" }, format!("{}:{:02}", m / 60, m % 60)));
                }
                if let Some(w) = b.power_w.filter(|w| *w > 0.05) {
                    col = col.child(kv("Мощность", format!("{w:.2} Вт")));
                }
                if let Some(i) = info {
                    if let Some(v) = i.voltage_v {
                        col = col.child(kv("Напряжение", format!("{v:.2} В")));
                    }
                    if let Some(t) = i.temp_c {
                        col = col.child(kv("Температура", format!("{t:.1} °C")));
                    }
                }
                Box::new(col)
            }
            (None, _) => match s.adc_volt {
                // Драйвера батареи нет (телефон без ADSP) — напряжение с АЦП.
                Some(v) => Box::new(
                    Column::new()
                        .gap(4.0)
                        .child(Text::new(format!("{v:.2} В")).class("res-big"))
                        .child(Text::new("напряжение аккумулятора").class("res-k")),
                ),
                None => Box::new(Text::new("Драйвер аккумулятора не найден").class("res-v")),
            },
        };
        let net = card(
            "Сеть",
            "\u{E80D}",
            Column::new().gap(6.0).child(kv("↓", human_rate(s.net.0))).child(kv("↑", human_rate(s.net.1))),
        );
        // Питание и сеть — одной строкой, поровну.
        Box::new(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(DecoratedBox::new().child(card("Питание", "\u{E1A4}", body)).class("grow res-half"))
                .child(DecoratedBox::new().child(net).class("grow res-half")),
        )
    });
    let apps = rx(move || {
        let s = sig.get();
        if s.apps.is_empty() {
            return Box::new(Text::new("Нет запущенных приложений").class("res-empty")) as Box<dyn Widget>;
        }
        let mut row = Row::new().gap(12.0);
        for a in &s.apps {
            row = row.child(Keyed::new(a.window, 0, {
                let a = a.clone();
                move || Box::new(app_card(&a))
            }));
        }
        Box::new(ScrollView::new().horizontal().child(row.class("res-rail-row")).class("res-rail"))
    });
    ScrollView::new().vertical().child(
        Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(clock)
            .child(cpu_card)
            .child(mem_gpu)
            .child(power)
            .child(Text::new("Запущенные приложения").class("res-section"))
            .child(apps)
            .class("home-page res-page"),
    )
}

/// Карточка приложения в ленте: значок, слева бейдж памяти, справа —
/// процессора; тап — к окну, «×» в углу или свайп вверх — закрыть.
fn app_card(a: &AppStat) -> impl Widget {
    let entry = synshell_common::xdg::app_for_window(&a.app_id);
    let icon_path = entry.as_ref().and_then(|e| synshell_common::xdg::lookup_icon(&e.icon)).or_else(|| synshell_common::xdg::window_icon(&a.app_id));
    let name = entry.map(|e| e.name).unwrap_or_else(|| a.app_id.clone());
    let mem = if a.mem_kb >= 1024 * 1024 { format!("{:.1}G", a.mem_kb as f64 / 1048576.0) } else { format!("{}M", a.mem_kb / 1024) };
    let cpu_class = if a.cpu >= 50.0 { "res-badge res-badge-cpu res-badge-hot" } else { "res-badge res-badge-cpu" };
    let id = a.window;
    let close = GestureDetector::new()
        .on_click(move || synshell_ui::actions::window_op(id, WindowOp::Close))
        .child(DecoratedBox::new().child(synshell_ui::ui::icon(synshell_ui::ui::mi::CLOSE).class("res-app-close-icon")).class("res-app-close"));
    let card = GestureDetector::new()
        .pan_axis(PanAxis::Vertical)
        .on_click(move || synshell_ui::actions::window_op(id, WindowOp::Activate))
        .on_swipe(move |d, _| {
            if d == SwipeDirection::Up {
                synshell_ui::actions::window_op(id, WindowOp::Close);
            }
        })
        .child(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .gap(6.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(
                            Row::new()
                                .gap(4.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(DecoratedBox::new().child(Text::new(mem).class("res-badge-text")).class("res-badge res-badge-mem"))
                                .child(launchers::icon_widget(&icon_path, &None, "res-app-icon", 44.0))
                                .child(DecoratedBox::new().child(Text::new(format!("{:.0}%", a.cpu)).class("res-badge-text")).class(cpu_class)),
                        )
                        .child(Text::new(name).max_lines(1).class("res-app-name"))
                        .child(Text::new(a.title.clone()).max_lines(1).class("res-app-title")),
                )
                .class("res-app"),
        );
    Stack::new().child(card).child(Row::new().main_axis_alignment(MainAxisAlignment::End).child(close).class("res-app-close-row"))
}
