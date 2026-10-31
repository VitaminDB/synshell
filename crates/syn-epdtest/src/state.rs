//! Состояние программы: режимы драйвера E Ink (читаются из sysfs, пишутся в фоне), вкладка,
//! таймеры тестов «Движение» и «Следы», выбор полного экрана.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use syngui::prelude::*;
use synshell_common::display::{Eink, Scheme};

/// Вкладки с тестами.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Gray,
    Color,
    Text,
    Motion,
    Ghost,
    Scroll,
}

impl Tab {
    pub const ALL: [Tab; 6] = [Tab::Gray, Tab::Color, Tab::Text, Tab::Motion, Tab::Ghost, Tab::Scroll];

    pub fn title(self) -> String {
        match self {
            Tab::Gray => t!("Серое"),
            Tab::Color => t!("Цвет"),
            Tab::Text => t!("Текст"),
            Tab::Motion => t!("Движение"),
            Tab::Ghost => t!("Следы"),
            Tab::Scroll => t!("Прокрутка"),
        }
    }

    /// Имя для `--tab` снимка.
    pub fn from_name(s: &str) -> Option<Tab> {
        Some(match s {
            "gray" => Tab::Gray,
            "color" => Tab::Color,
            "text" => Tab::Text,
            "motion" => Tab::Motion,
            "ghost" => Tab::Ghost,
            "scroll" => Tab::Scroll,
            _ => return None,
        })
    }
}

/// Волны UI_MODE_SET, которые стоит перебрать (`scheme ui <v>`).
pub const UI_WAVES: [u8; 6] = [0x10, 0x15, 0x12, 0x06, 0x07, 0x03];
/// Предел авторежима FPGA (`scheme auto <n>`) для кнопки «+».
pub const AUTO_MAX: u8 = 15;
/// Частоты теста «Движение», Гц.
pub const MOTION_HZ: [u32; 4] = [5, 10, 15, 20];
/// Период автоинверсии теста «Следы», с (0 — выключено).
pub const GHOST_EVERY: [u32; 4] = [0, 1, 2, 5];

/// Сколько раз холст «Движения» нарисован (фактические кадры приложения).
pub static DRAWN: AtomicU64 = AtomicU64::new(0);
/// Поколение таймеров: смена останавливает прежний поток.
static MOTION_GEN: AtomicU64 = AtomicU64::new(0);
static GHOST_GEN: AtomicU64 = AtomicU64::new(0);

/// Снимок драйвера.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Driver {
    /// Каталог управления найден.
    pub found: bool,
    pub modes: Vec<String>,
    /// Текущий `update_mode`; `None` — не совпадает ни с одним (выбран сырой `scheme`).
    pub mode: Option<String>,
    /// Сырое чтение `update_mode`.
    pub mode_raw: String,
    pub scheme: Option<Scheme>,
    pub scheme_raw: String,
    /// `None` — панель без цвета (или нет файла).
    pub color: Option<bool>,
}

impl Driver {
    pub fn read() -> Driver {
        let Some(e) = Eink::find() else { return Driver::default() };
        let raw = |n: &str| std::fs::read_to_string(e.dir().join(n)).map(|s| s.trim().to_string()).unwrap_or_default();
        Driver {
            found: true,
            modes: e.update_modes().iter().map(|m| m.as_str().to_string()).collect(),
            mode: e.update_mode().map(|m| m.as_str().to_string()),
            mode_raw: raw("update_mode"),
            scheme: e.scheme(),
            scheme_raw: raw("scheme"),
            color: if e.is_color_panel() || e.dir().join("color").is_file() { e.color() } else { None },
        }
    }
}

/// Замер кадров «Движения» за последнюю секунду.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rate {
    /// Тиков таймера в секунду.
    pub ticks: f32,
    /// Отрисовок холста в секунду.
    pub drawn: f32,
}

#[derive(Clone, Copy)]
pub struct St {
    pub tab: RwSignal<Tab>,
    pub drv: RwSignal<Driver>,
    /// Показана панель «Эксперт».
    pub expert: RwSignal<bool>,
    /// Значение «auto N» для кнопок − / +.
    pub auto_n: RwSignal<u8>,
    /// Последняя ошибка записи в драйвер.
    pub error: RwSignal<Option<String>>,
    pub window: RwSignal<syngui::window::WindowState>,

    pub motion_on: RwSignal<bool>,
    pub motion_hz: RwSignal<u32>,
    /// Номер кадра таймера «Движения».
    pub tick: RwSignal<u64>,
    /// Мс с пуска «Движения».
    pub stamp_ms: RwSignal<u64>,
    pub rate: RwSignal<Rate>,

    pub ghost_inv: RwSignal<bool>,
    pub ghost_every: RwSignal<u32>,
    pub ghost_count: RwSignal<u64>,
}

impl St {
    pub fn new() -> St {
        let drv = Driver::read();
        let auto_n = match drv.scheme {
            Some(Scheme::Auto(n)) => n,
            _ => 4,
        };
        let expert = drv.found && drv.mode.is_none();
        St {
            tab: use_signal(Tab::Gray),
            drv: use_signal(drv),
            expert: use_signal(expert),
            auto_n: use_signal(auto_n),
            error: use_signal(None),
            window: use_signal(Default::default()),
            motion_on: use_signal(false),
            motion_hz: use_signal(10),
            tick: use_signal(0),
            stamp_ms: use_signal(0),
            rate: use_signal(Rate::default()),
            ghost_inv: use_signal(false),
            ghost_every: use_signal(0),
            ghost_count: use_signal(0),
        }
    }

    /// Перечитать драйвер; сигнал меняется только при отличии (лишняя перерисовка на E Ink — волна).
    pub fn reload(self) {
        let d = Driver::read();
        if self.drv.get_untracked() != d {
            self.drv.set(d);
        }
    }

    /// Запись в драйвер в фоне (sysfs может ждать конца волны), затем перечитать.
    pub fn write(self, f: impl FnOnce(&Eink) -> std::io::Result<()> + Send + 'static) {
        std::thread::spawn(move || {
            let r = match Eink::find() {
                Some(e) => f(&e).map_err(|e| e.to_string()),
                None => Err(t!("драйвер E Ink не найден")),
            };
            run_on_main_thread(move || {
                if let Err(e) = &r {
                    tracing::warn!("запись в драйвер E Ink: {e}");
                }
                let err = r.err();
                if self.error.get_untracked() != err {
                    self.error.set(err);
                }
                self.reload();
            });
        });
    }

    pub fn set_mode(self, name: String) {
        self.write(move |e| match synshell_common::display::UpdateMode::from_name(&name) {
            Some(m) => e.set_update_mode(&m),
            None => Ok(()),
        });
    }

    pub fn set_scheme(self, s: Scheme) {
        if let Scheme::Auto(n) = s {
            if self.auto_n.get_untracked() != n {
                self.auto_n.set(n);
            }
        }
        self.write(move |e| e.set_scheme(s));
    }

    pub fn set_color(self, on: bool) {
        self.write(move |e| e.set_color(on));
    }

    pub fn refresh(self) {
        self.write(|e| e.refresh());
    }

    // ── «Движение» ─────────────────────────────────────────────────────────────────────────

    /// Пуск/перезапуск таймера с частотой `motion_hz`; кадр — `tick` +1 на главном потоке.
    pub fn motion_start(self) {
        let gen = MOTION_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        let hz = self.motion_hz.get_untracked().max(1);
        if !self.motion_on.get_untracked() {
            self.motion_on.set(true);
        }
        let start = Instant::now();
        let base_tick = self.tick.get_untracked();
        let base_ms = self.stamp_ms.get_untracked();
        std::thread::spawn(move || {
            let period = Duration::from_secs_f64(1.0 / hz as f64);
            let mut next = Instant::now();
            let mut n = 0u64;
            // окно замера: тики и отрисовки за последнюю секунду
            let mut win = (Instant::now(), 0u64, DRAWN.load(Ordering::Relaxed));
            loop {
                next += period;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else {
                    // не успели — без догонялок пачкой
                    next = now;
                }
                if MOTION_GEN.load(Ordering::SeqCst) != gen {
                    return;
                }
                n += 1;
                let ms = base_ms + start.elapsed().as_millis() as u64;
                let el = win.0.elapsed().as_secs_f32();
                let rate = if el >= 1.0 {
                    let drawn = DRAWN.load(Ordering::Relaxed);
                    let r = Rate { ticks: (n - win.1) as f32 / el, drawn: (drawn - win.2) as f32 / el };
                    win = (Instant::now(), n, drawn);
                    Some(r)
                } else {
                    None
                };
                run_on_main_thread(move || {
                    if MOTION_GEN.load(Ordering::SeqCst) != gen {
                        return;
                    }
                    self.tick.set(base_tick + n);
                    self.stamp_ms.set(ms);
                    if let Some(r) = rate {
                        self.rate.set(r);
                    }
                });
            }
        });
    }

    pub fn motion_stop(self) {
        MOTION_GEN.fetch_add(1, Ordering::SeqCst);
        if self.motion_on.get_untracked() {
            self.motion_on.set(false);
        }
    }

    pub fn motion_hz(self, hz: u32) {
        self.motion_hz.set(hz);
        if self.motion_on.get_untracked() {
            self.motion_start();
        }
    }

    // ── «Следы» ────────────────────────────────────────────────────────────────────────────

    pub fn ghost_flip(self) {
        self.ghost_inv.set(!self.ghost_inv.get_untracked());
        self.ghost_count.set(self.ghost_count.get_untracked() + 1);
    }

    /// Автоинверсия раз в `secs` с (0 — выключить).
    pub fn ghost_every(self, secs: u32) {
        let gen = GHOST_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        self.ghost_every.set(secs);
        if secs == 0 {
            return;
        }
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(secs as u64));
            if GHOST_GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            run_on_main_thread(move || {
                if GHOST_GEN.load(Ordering::SeqCst) == gen {
                    self.ghost_flip();
                }
            });
        });
    }

    /// Уход с вкладки: таймеры теста останавливаются (лишние волны на панели не нужны).
    pub fn go(self, tab: Tab) {
        if self.tab.get_untracked() == tab {
            return;
        }
        if tab != Tab::Motion {
            self.motion_stop();
        }
        if tab != Tab::Ghost && self.ghost_every.get_untracked() != 0 {
            self.ghost_every(0);
        }
        self.tab.set(tab);
    }
}

// ── Полный экран: выбор пользователя запоминается ─────────────────────────────────────────

fn fullscreen_file() -> std::path::PathBuf {
    synshell_common::paths::state_home().join("synshell").join("syn-epdtest")
}

/// Запомненный выбор; по умолчанию — во весь экран.
pub fn fullscreen_saved() -> bool {
    std::fs::read_to_string(fullscreen_file()).map(|s| s.trim() != "fullscreen=0").unwrap_or(true)
}

fn save_fullscreen(on: bool) {
    let p = fullscreen_file();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, if on { "fullscreen=1\n" } else { "fullscreen=0\n" });
}

pub fn toggle_fullscreen(st: St) {
    set_fullscreen(st, !st.window.get_untracked().fullscreen);
}

pub fn set_fullscreen(_st: St, on: bool) {
    syngui::signal::set_fullscreen(on);
    save_fullscreen(on);
}
