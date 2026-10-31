//! Состояние программы: профили экрана E Ink (общий файл `eink_profiles`, редактирование с живым
//! применением к драйверу), значения драйвера (читаются из sysfs, пишутся в фоне), вкладка,
//! таймеры тестов «Движение» и «Следы», выбор полного экрана.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use syngui::prelude::*;
use synshell_common::display::{Eink, Scheme, Tuning};
use synshell_common::eink_profiles::{self, Profile, Profiles, DEFAULT_ID};

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

/// Профили (копия файла; меняется только здесь и сохраняется целиком).
static STORE: Mutex<Option<Profiles>> = Mutex::new(None);
/// Последний ещё не записанный профиль (живое применение: ползунок шлёт десятки значений —
/// пишется последнее) и замок записи (порядок записей в драйвер — как у правок).
static PENDING: Mutex<Option<(Profile, Profiles)>> = Mutex::new(None);
static APPLY: Mutex<()> = Mutex::new(());
/// Текст поля имени в диалоге «Новый» / «Переименовать».
pub static NAME_INPUT: Mutex<String> = Mutex::new(String::new());

fn store<R>(f: impl FnOnce(&mut Profiles) -> R) -> R {
    let mut g = STORE.lock().unwrap();
    f(g.get_or_insert_with(eink_profiles::load))
}

/// Снимок драйвера.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Driver {
    /// Каталог управления найден.
    pub found: bool,
    pub scheme: Option<Scheme>,
    pub scheme_raw: String,
    /// Настройки, которые есть у драйвера, и их значения (`None` — не готов).
    pub tunings: Vec<(Tuning, Option<u32>)>,
    /// `None` — панель без цвета (или нет файла).
    pub color: Option<bool>,
}

impl Driver {
    pub fn read() -> Driver {
        let Some(e) = Eink::find() else { return Driver::default() };
        let raw = |n: &str| std::fs::read_to_string(e.dir().join(n)).map(|s| s.trim().to_string()).unwrap_or_default();
        Driver {
            found: true,
            scheme: e.scheme(),
            scheme_raw: raw("scheme"),
            tunings: Tuning::EDITOR.into_iter().filter(|t| e.has(*t)).map(|t| (t, e.tuning(t))).collect(),
            color: if e.is_color_panel() || e.dir().join("color").is_file() { e.color() } else { None },
        }
    }

    pub fn tuning(&self, t: Tuning) -> Option<u32> {
        self.tunings.iter().find(|(x, _)| *x == t).and_then(|(_, v)| *v)
    }

    pub fn has(&self, t: Tuning) -> bool {
        self.tunings.iter().any(|(x, _)| *x == t)
    }
}

/// Диалог над редактором.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    New,
    Rename,
    Delete,
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
    /// Писать в драйвер и конфиг (в снимке — нет).
    pub live: bool,
    pub tab: RwSignal<Tab>,
    pub drv: RwSignal<Driver>,
    /// Имя выбранного (и применённого) профиля.
    pub active: RwSignal<String>,
    /// Состав списка профилей сменился (новый, переименование, удаление).
    pub list_rev: RwSignal<u32>,
    /// Значения выбранного профиля (с правками).
    pub cur: RwSignal<Profile>,
    /// Профиль, каким он был при выборе, — для «Вернуть».
    pub orig: RwSignal<Profile>,
    /// Пересобрать ползунки (выбор профиля, «Вернуть», задать/снять настройку).
    pub form_rev: RwSignal<u32>,
    pub dialog: RwSignal<Option<Dialog>>,
    pub dialog_err: RwSignal<Option<String>>,
    /// Показаны ползунки настроек.
    pub show_params: RwSignal<bool>,
    /// Значение «auto N» для кнопок − / +, пока выбрана волна ui.
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
    /// `live` — применять выбор к драйверу и конфигу; активный — из `[eink] profile`
    /// (нет — запомненный для экрана входа, иначе встроенный).
    pub fn new(live: bool) -> St {
        let drv = Driver::read();
        let name = eink_profiles::active_name(&synshell_common::Config::load().0.eink);
        let p = store(|s| s.resolve(Some(&name)));
        let auto_n = match p.scheme {
            Scheme::Auto(n) => n,
            _ => 5,
        };
        St {
            live,
            tab: use_signal(Tab::Gray),
            drv: use_signal(drv),
            active: use_signal(p.name.clone()),
            list_rev: use_signal(0),
            cur: use_signal(p.clone()),
            orig: use_signal(p),
            form_rev: use_signal(0),
            dialog: use_signal(None),
            dialog_err: use_signal(None),
            show_params: use_signal(true),
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

    /// Все профили по порядку (встроенный первым).
    pub fn profiles(self) -> Vec<Profile> {
        store(|s| s.all())
    }

    /// Перечитать драйвер; сигнал меняется только при отличии (лишняя перерисовка на E Ink — волна).
    pub fn reload(self) {
        let d = Driver::read();
        if self.drv.get_untracked() != d {
            self.drv.set(d);
        }
    }

    fn set_error(self, err: Option<String>) {
        if self.error.get_untracked() != err {
            self.error.set(err);
        }
    }

    /// Запись в драйвер в фоне (sysfs может ждать конца волны), затем перечитать.
    pub fn write(self, f: impl FnOnce(&Eink) -> std::io::Result<()> + Send + 'static) {
        if !self.live {
            return;
        }
        std::thread::spawn(move || {
            let r = {
                let _g = APPLY.lock().unwrap();
                match Eink::find() {
                    Some(e) => f(&e).map_err(|e| e.to_string()),
                    None => Err(t!("драйвер E Ink не найден")),
                }
            };
            run_on_main_thread(move || {
                if let Err(e) = &r {
                    tracing::warn!("запись в драйвер E Ink: {e}");
                }
                self.set_error(r.err());
                self.reload();
            });
        });
    }

    pub fn set_color(self, on: bool) {
        if self.live {
            if let Err(e) = synshell_common::config_edit::set_value(&["eink", "color"], on.into()) {
                tracing::warn!("[eink] color: {e:#}");
            }
        }
        self.write(move |e| e.set_color(on));
    }

    pub fn refresh(self) {
        self.write(|e| e.refresh());
    }

    // ── Профили ────────────────────────────────────────────────────────────────────────────

    /// Выбрать профиль: применить к драйверу и сделать активным (`[eink] profile`; synwm запомнит
    /// его для экрана входа).
    pub fn select(self, name: &str) {
        let p = store(|s| s.resolve(Some(name)));
        if let Scheme::Auto(n) = p.scheme {
            self.auto_n.set(n);
        }
        self.active.set(p.name.clone());
        self.cur.set(p.clone());
        self.orig.set(p.clone());
        self.form_rev.set(self.form_rev.get_untracked() + 1);
        self.close_dialog();
        if self.live {
            if let Err(e) = eink_profiles::set_active(&p.name) {
                tracing::warn!("[eink] profile: {e:#}");
            }
        }
        self.write(move |e| e.apply_profile(&p));
    }

    /// Правка выбранного профиля (встроенный не меняется): сразу в драйвер и в файл профилей.
    pub fn edit(self, f: impl FnOnce(&mut Profile)) {
        let mut p = self.cur.get_untracked();
        if p.is_default() {
            return;
        }
        f(&mut p);
        if p == self.cur.get_untracked() {
            return;
        }
        if let Scheme::Auto(n) = p.scheme {
            if self.auto_n.get_untracked() != n {
                self.auto_n.set(n);
            }
        }
        store(|s| s.update(&p));
        let p = store(|s| s.resolve(Some(&p.name)));
        self.cur.set(p.clone());
        self.commit(p);
    }

    /// Задать или снять настройку (ползунки пересобираются: меняется вид строки).
    pub fn put(self, t: Tuning, v: Option<u32>) {
        self.edit(|p| p.put(t, v));
        self.form_rev.set(self.form_rev.get_untracked() + 1);
    }

    pub fn set_scheme(self, s: Scheme) {
        self.edit(|p| p.scheme = s);
    }

    /// Вернуть профиль, каким он был при выборе.
    pub fn revert(self) {
        let o = self.orig.get_untracked();
        self.edit(|p| *p = o);
        self.form_rev.set(self.form_rev.get_untracked() + 1);
    }

    /// Живое применение и сохранение в фоне: пишется последняя правка.
    fn commit(self, p: Profile) {
        if !self.live {
            return;
        }
        let all = store(|s| s.clone());
        let start = PENDING.lock().unwrap().replace((p, all)).is_none();
        if !start {
            return;
        }
        std::thread::spawn(move || loop {
            let _g = APPLY.lock().unwrap();
            let Some((p, all)) = PENDING.lock().unwrap().take() else { break };
            let mut err = None;
            if let Err(e) = eink_profiles::save(&all) {
                err = Some(t!("профили не сохранены: {e}", e = e));
            }
            if let Some(e) = Eink::find() {
                if let Err(e) = e.apply_profile(&p) {
                    err = Some(e.to_string());
                }
            }
            run_on_main_thread(move || {
                self.set_error(err);
                self.reload();
            });
        });
    }

    fn save_store(self) {
        if !self.live {
            return;
        }
        let all = store(|s| s.clone());
        std::thread::spawn(move || {
            let _g = APPLY.lock().unwrap();
            let r = eink_profiles::save(&all).map_err(|e| t!("профили не сохранены: {e}", e = e));
            run_on_main_thread(move || self.set_error(r.err()));
        });
    }

    pub fn open_dialog(self, d: Dialog) {
        let name = match d {
            Dialog::New => store(|s| s.unique_name(&self.cur.get_untracked().title())),
            Dialog::Rename => self.cur.get_untracked().name,
            Dialog::Delete => String::new(),
        };
        *NAME_INPUT.lock().unwrap() = name;
        self.dialog_err.set(None);
        self.dialog.set(Some(d));
    }

    pub fn close_dialog(self) {
        if self.dialog.get_untracked().is_some() {
            self.dialog.set(None);
        }
        if self.dialog_err.get_untracked().is_some() {
            self.dialog_err.set(None);
        }
    }

    /// «Новый»: копия выбранного под именем `name`, сразу выбранная.
    pub fn create(self, name: &str) {
        let from = self.cur.get_untracked();
        match store(|s| s.add(name, &from)) {
            Ok(p) => {
                self.save_store();
                self.list_rev.set(self.list_rev.get_untracked() + 1);
                self.select(&p.name);
            }
            Err(e) => self.dialog_err.set(Some(e.0)),
        }
    }

    /// Переименовать выбранный; он остаётся активным под новым именем.
    pub fn rename(self, name: &str) {
        let old = self.active.get_untracked();
        match store(|s| s.rename(&old, name)) {
            Ok(new) => {
                self.save_store();
                for sig in [self.cur, self.orig] {
                    let mut p = sig.get_untracked();
                    p.name = new.clone();
                    sig.set(p);
                }
                self.active.set(new.clone());
                self.list_rev.set(self.list_rev.get_untracked() + 1);
                self.close_dialog();
                if self.live {
                    if let Err(e) = eink_profiles::set_active(&new) {
                        tracing::warn!("[eink] profile: {e:#}");
                    }
                }
            }
            Err(e) => self.dialog_err.set(Some(e.0)),
        }
    }

    /// Удалить выбранный; активным становится встроенный.
    pub fn delete(self) {
        let name = self.active.get_untracked();
        if store(|s| s.remove(&name)) {
            self.save_store();
            self.list_rev.set(self.list_rev.get_untracked() + 1);
        }
        self.select(DEFAULT_ID);
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
