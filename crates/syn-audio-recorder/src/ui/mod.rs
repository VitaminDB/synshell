//! Окно диктофона: своя рамка (заголовок с кнопками окна, перетаскивание, изменение размера); широкое
//! окно — записи слева, запись справа; узкое (телефон) — экран записи и экран списка с верхней панелью.

pub mod list;
pub mod record;
pub mod sheets;
pub mod wave;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use syngui::async_runtime::run_on_main_thread;
use syngui::core::sync::Mutex;
use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::GestureDetector;
use syngui::video::AudioFilePlayer;
use syngui::widgets::{EventHook, KeyReply};
use syn_system::sound::{self, NodeKind};

use crate::engine;
use crate::store::{self, Recording, Settings};

pub type W = Box<dyn Widget>;

pub fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

pub mod icons {
    pub const MIC: &str = "\u{E029}";
    pub const MIC_OFF: &str = "\u{E02B}";
    pub const PAUSE: &str = "\u{E034}";
    pub const PLAY: &str = "\u{E037}";
    pub const DELETE: &str = "\u{E872}";
    pub const EDIT: &str = "\u{E3C9}";
    pub const FOLDER: &str = "\u{E2C8}";
    pub const SETTINGS: &str = "\u{E8B8}";
    pub const CLOSE: &str = "\u{E5CD}";
    pub const WAVE: &str = "\u{E1B8}";
    pub const BACK10: &str = "\u{E059}";
    pub const FWD10: &str = "\u{E056}";
    pub const CHECK: &str = "\u{E5CA}";
    pub const BACK: &str = "\u{E5C4}";
    pub const LIST: &str = "\u{E03D}";
    pub const NOISE: &str = "\u{EBEC}";
    pub const EXPAND: &str = "\u{E5CF}";
    pub const RADIO_ON: &str = "\u{E837}";
    pub const RADIO_OFF: &str = "\u{E836}";
    pub const WARNING: &str = "\u{E002}";
    pub const TRANSLATE: &str = "\u{E8E2}";
}

pub fn icon(g: &str) -> syngui::widget::StyledWidget<Icon> {
    Icon::new(g).class("ic")
}

pub fn tip(w: impl Widget + 'static, text: impl Into<String>) -> W {
    boxed(Tooltip::new(w, text).delay_ms(500))
}

pub fn icon_button(glyph: &str, tooltip: impl Into<String>, class: &str, on_click: impl FnMut() + Send + 'static) -> W {
    tip(Button::new("").icon(glyph).class(&format!("btn round flat {class}")).on_click(on_click), tooltip)
}

// ─── состояние ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Record,
    List,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MicState {
    Closed,
    Opening,
    Open,
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rec {
    Idle,
    Starting,
    Recording,
    Paused,
    Saving,
}

impl Rec {
    pub fn active(self) -> bool {
        matches!(self, Rec::Recording | Rec::Paused)
    }
}

/// Микрофон в списке выбора.
#[derive(Clone, Debug, PartialEq)]
pub struct MicItem {
    pub name: String,
    pub label: String,
    pub default: bool,
}

/// Шумоподавление телефона: `None` — устройство не умеет.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ns {
    pub on: bool,
    pub active: bool,
}

/// Открытый плеер.
#[derive(Clone)]
pub struct Player(pub Arc<Mutex<AudioFilePlayer>>);

impl PartialEq for Player {
    fn eq(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.0, &o.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub text: String,
    /// Удалённая запись: (в корзине, исходный путь) — кнопка «Отменить».
    pub undo: Option<(PathBuf, PathBuf)>,
    pub error: bool,
}

#[derive(Clone, Copy)]
pub struct St {
    pub settings: RwSignal<Settings>,
    pub page: RwSignal<Page>,
    pub mics: RwSignal<Vec<MicItem>>,
    pub ns: RwSignal<Option<Ns>>,
    pub mic: RwSignal<MicState>,
    pub rec: RwSignal<Rec>,
    /// Длительность дубля, десятые доли секунды.
    pub tenths: RwSignal<u64>,
    pub clip: RwSignal<bool>,
    pub recordings: RwSignal<Vec<Recording>>,
    pub scanning: RwSignal<bool>,
    pub selected: RwSignal<Option<PathBuf>>,
    pub player: RwSignal<Option<Player>>,
    pub pos: RwSignal<f64>,
    pub dur: RwSignal<f64>,
    pub playing: RwSignal<bool>,
    pub window: RwSignal<syngui::window::WindowState>,
    pub theme: RwSignal<String>,
    pub toast: RwSignal<Option<Toast>>,
    /// Открытый лист: выбор микрофона или настройки.
    pub sheet: RwSignal<Option<Sheet>>,
    pub rename: RwSignal<Option<PathBuf>>,
    /// Только что сохранённая запись: «Отменить» ещё несколько секунд удаляет её (с возвратом).
    pub just_saved: RwSignal<Option<PathBuf>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sheet {
    Mics,
    Settings,
}

static TOAST_ID: AtomicU64 = AtomicU64::new(0);
static OPEN_GEN: AtomicU64 = AtomicU64::new(0);
static SEEK_GEN: AtomicU64 = AtomicU64::new(0);
/// Перемотка ждёт декодер: тикер не перезаписывает позицию.
static SEEKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Окно не в фокусе дольше — монитор микрофона закрывается.
const IDLE_CLOSE: Duration = Duration::from_secs(120);

/// Повторное нажатие главной кнопки раньше — не «Стоп», а случайный двойной тап.
const STOP_GUARD: Duration = Duration::from_millis(1000);
/// Сколько после «Стоп» кнопка «Отменить» удаляет сохранённую запись.
const UNDO_SAVE: Duration = Duration::from_secs(8);

thread_local! {
    static STARTED_AT: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
    static SAVED_AT: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
    static UNFOCUSED_SINCE: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
    static SUSPENDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static LAST_NS: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

impl St {
    pub fn new() -> Self {
        St {
            settings: use_signal(Settings::load()),
            page: use_signal(Page::Record),
            mics: use_signal(Vec::new()),
            ns: use_signal(None),
            mic: use_signal(MicState::Closed),
            rec: use_signal(Rec::Idle),
            tenths: use_signal(0),
            clip: use_signal(false),
            recordings: use_signal(Vec::new()),
            scanning: use_signal(true),
            selected: use_signal(None),
            player: use_signal(None),
            pos: use_signal(0.0),
            dur: use_signal(0.0),
            playing: use_signal(false),
            window: use_signal(syngui::window::WindowState::default()),
            theme: use_signal(String::new()),
            toast: use_signal(None),
            sheet: use_signal(None),
            just_saved: use_signal(None),
            rename: use_signal(None),
        }
    }

    /// Запуск: папка, список, микрофоны, тикер.
    pub fn start(self) {
        let dir = self.settings.get_untracked().dir();
        let _ = std::fs::create_dir_all(&dir);
        std::thread::spawn(move || store::cleanup(&dir));
        self.rescan();
        self.refresh_mics();
        self.refresh_ns();
        self.start_ticker();
    }

    pub fn toast(self, text: impl Into<String>) {
        self.show_toast(text.into(), None, false);
    }

    pub fn error(self, text: impl Into<String>) {
        self.show_toast(text.into(), None, true);
    }

    fn show_toast(self, text: String, undo: Option<(PathBuf, PathBuf)>, error: bool) {
        // прежняя подсказка с «Отменить» уходит — удалённое удаляется насовсем
        if let Some(Toast { undo: Some((t, _)), .. }) = self.toast.get_untracked() {
            std::thread::spawn(move || store::purge(&t));
        }
        let id = TOAST_ID.fetch_add(1, Ordering::AcqRel) + 1;
        let secs = if undo.is_some() { 6 } else if error { 6 } else { 3 };
        self.toast.set(Some(Toast { id, text, undo, error }));
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(secs));
            run_on_main_thread(move || {
                if let Some(t) = self.toast.get_untracked().filter(|t| t.id == id) {
                    if let Some((tr, _)) = t.undo {
                        std::thread::spawn(move || store::purge(&tr));
                    }
                    self.toast.set(None);
                }
            });
        });
    }

    pub fn set_settings(self, f: impl FnOnce(&mut Settings)) {
        let mut s = self.settings.get_untracked();
        let old = s.clone();
        f(&mut s);
        if s == old {
            return;
        }
        s.save();
        let dir_changed = s.dir() != old.dir();
        let mic_changed = s.mic != old.mic;
        let monitor_off = old.vis_always && !s.vis_always;
        self.settings.set(s);
        if dir_changed {
            self.close_player();
            self.rescan();
        }
        if (mic_changed || monitor_off) && !self.rec.get_untracked().active() {
            // новый микрофон откроет тикер (если монитор включён)
            self.mic.set(MicState::Closed);
            std::thread::spawn(engine::close_mic);
        }
    }

    // ─── микрофоны и шумоподавление ────────────────────────────────────────

    pub fn refresh_mics(self) {
        std::thread::spawn(move || {
            let list: Vec<MicItem> = sound::nodes()
                .unwrap_or_default()
                .into_iter()
                .filter(|n| n.kind == NodeKind::Input)
                .map(|n| MicItem { label: if n.label.trim().is_empty() { n.name.clone() } else { n.label.clone() }, name: n.name, default: n.default })
                .collect();
            run_on_main_thread(move || {
                if self.mics.get_untracked() != list {
                    self.mics.set(list);
                }
            });
        });
    }

    pub fn refresh_ns(self) {
        LAST_NS.with(|c| c.set(Some(Instant::now())));
        std::thread::spawn(move || {
            let ns = sound::phone_audio().and_then(|p| p.mic_ns.map(|on| Ns { on, active: p.mic_ns_active }));
            run_on_main_thread(move || {
                if self.ns.get_untracked() != ns {
                    self.ns.set(ns);
                }
            });
        });
    }

    pub fn set_ns(self, on: bool) {
        self.ns.update(|n| {
            if let Some(n) = n {
                n.on = on;
            }
        });
        std::thread::spawn(move || {
            let r = sound::set_mic_ns(on);
            let ns = r.and_then(|p| p.mic_ns.map(|on| Ns { on, active: p.mic_ns_active }));
            run_on_main_thread(move || {
                if ns.is_none() {
                    self.error(t!("Не удалось переключить шумоподавление: служба звука не отвечает"));
                }
                self.ns.set(ns.or(self.ns.get_untracked()));
            });
        });
    }

    /// Подпись выбранного микрофона.
    pub fn mic_label(self) -> String {
        let s = self.settings.get_untracked();
        match s.mic_node() {
            None => t!("По умолчанию"),
            Some(n) => self.mics.get_untracked().into_iter().find(|m| m.name == n).map(|m| m.label).unwrap_or(n),
        }
    }

    /// Узел, с которого реально пишется (выбранный или системный по умолчанию).
    pub fn effective_mic(self) -> Option<String> {
        self.settings.get_untracked().mic_node().or_else(|| self.mics.get_untracked().into_iter().find(|m| m.default).map(|m| m.name))
    }

    /// Системное шумоподавление действует на выбранный микрофон (на отдельные микрофоны телефона — нет).
    pub fn ns_applies(self) -> bool {
        !self.effective_mic().is_some_and(|n| n.starts_with("palaudio.mic."))
    }

    fn open_mic_then(self, then: impl FnOnce(St) + Send + 'static) {
        let node = self.settings.get_untracked().mic_node();
        self.mic.set(MicState::Opening);
        std::thread::spawn(move || {
            let r = engine::open_mic(node);
            run_on_main_thread(move || match r {
                Ok(()) => {
                    self.mic.set(MicState::Open);
                    then(self);
                }
                Err(e) => {
                    tracing::warn!("микрофон: {e}");
                    self.mic.set(MicState::Error(e.clone()));
                    if self.rec.get_untracked() == Rec::Starting {
                        self.rec.set(Rec::Idle);
                        self.error(t!("Микрофон не открылся: {e}", e = e));
                    }
                }
            });
        });
    }

    // ─── запись ────────────────────────────────────────────────────────────

    /// Пробел и большая кнопка: начать, иначе пауза/продолжить.
    pub fn primary(self) {
        match self.rec.get_untracked() {
            Rec::Idle => self.record(),
            Rec::Recording | Rec::Paused => self.toggle_pause(),
            _ => {}
        }
    }

    pub fn record(self) {
        if self.rec.get_untracked() != Rec::Idle {
            return;
        }
        if let Some(p) = self.player.get_untracked() {
            p.0.lock().unwrap_or_else(|e| e.into_inner()).pause();
            self.playing.set(false);
        }
        self.rec.set(Rec::Starting);
        self.tenths.set(0);
        self.just_saved.set(None);
        STARTED_AT.with(|c| c.set(Some(Instant::now())));
        let go = move |st: St| {
            let s = st.settings.get_untracked();
            match engine::start(&s, &st.mic_label()) {
                Ok(p) => {
                    tracing::info!("запись: {}", p.display());
                    st.rec.set(Rec::Recording);
                }
                Err(e) => {
                    st.rec.set(Rec::Idle);
                    st.error(e);
                }
            }
        };
        let want = self.settings.get_untracked().mic_node();
        if engine::mic_open() == Some(want) {
            go(self);
        } else {
            self.open_mic_then(go);
        }
    }

    pub fn toggle_pause(self) {
        match self.rec.get_untracked() {
            Rec::Recording => {
                engine::set_paused(true);
                self.rec.set(Rec::Paused);
            }
            Rec::Paused => {
                engine::set_paused(false);
                self.rec.set(Rec::Recording);
            }
            _ => {}
        }
    }

    /// Главная кнопка: запись или «Стоп» (повтор в первую секунду — двойной тап, не стоп).
    pub fn main_button(self) {
        match self.rec.get_untracked() {
            Rec::Idle => self.record(),
            Rec::Recording | Rec::Paused => {
                if STARTED_AT.with(|c| c.get()).is_some_and(|t| t.elapsed() < STOP_GUARD) {
                    tracing::info!("стоп в первую секунду записи — пропущен (двойной тап)");
                    return;
                }
                self.stop()
            }
            _ => {}
        }
    }

    pub fn stop(self) {
        if !self.rec.get_untracked().active() {
            return;
        }
        self.rec.set(Rec::Saving);
        std::thread::spawn(move || {
            let r = engine::stop();
            run_on_main_thread(move || {
                self.rec.set(Rec::Idle);
                match r {
                    Ok(Some(p)) => {
                        let m = store::read_meta(&p).unwrap_or_default();
                        self.toast(t!("Сохранено: {t}", t = m.title));
                        // новая запись — в плеер (не играет, пока не нажать)
                        OPEN_GEN.fetch_add(1, Ordering::AcqRel);
                        self.drop_player();
                        self.pos.set(0.0);
                        self.dur.set(m.duration);
                        self.selected.set(Some(p.clone()));
                        self.just_saved.set(Some(p));
                        SAVED_AT.with(|c| c.set(Some(Instant::now())));
                    }
                    Ok(None) => self.toast(t!("Пустая запись не сохранена")),
                    Err(e) => self.error(t!("Ошибка записи: {e}", e = e)),
                }
                self.tenths.set(0);
                self.rescan();
            });
        });
    }

    pub fn cancel(self) {
        if !self.rec.get_untracked().active() {
            // сразу после «Стоп» — удалить только что сохранённую (в корзину, с «Отменить» в подсказке)
            if let Some(p) = self.just_saved.get_untracked() {
                self.just_saved.set(None);
                self.delete(p);
            }
            return;
        }
        engine::cancel();
        self.rec.set(Rec::Idle);
        self.tenths.set(0);
        self.toast(t!("Запись отменена"));
    }

    // ─── записи ────────────────────────────────────────────────────────────

    pub fn rescan(self) {
        let dir = self.settings.get_untracked().dir();
        self.scanning.set(true);
        std::thread::spawn(move || {
            let list = store::scan(&dir);
            let pending: Vec<PathBuf> = list.iter().filter(|r| r.pending).map(|r| r.path.clone()).collect();
            run_on_main_thread(move || {
                self.recordings.set(list);
                self.scanning.set(false);
            });
            // файлы без сведений (скопированные в папку) — длительность и волна
            for p in pending {
                let Some(mut m) = engine::analyze(&p) else { continue };
                m.title = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                m.created = std::fs::metadata(&p).ok().and_then(|md| md.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
                store::write_meta(&p, &m);
                run_on_main_thread(move || {
                    self.recordings.update(|v| {
                        if let Some(r) = v.iter_mut().find(|r| r.path == p) {
                            r.duration = m.duration;
                            r.peaks = m.peaks.clone();
                            r.pending = false;
                        }
                    });
                });
            }
        });
    }

    /// Выбрать запись и играть её.
    pub fn play(self, path: PathBuf) {
        if self.rec.get_untracked().active() {
            self.toast(t!("Идёт запись — прослушать можно после остановки"));
            return;
        }
        if self.selected.get_untracked().as_ref() == Some(&path) {
            if self.player.get_untracked().is_some() {
                self.toggle_play();
                return;
            }
        }
        let gen = OPEN_GEN.fetch_add(1, Ordering::AcqRel) + 1;
        self.drop_player();
        self.selected.set(Some(path.clone()));
        self.pos.set(0.0);
        self.dur.set(self.recordings.get_untracked().iter().find(|r| r.path == path).map(|r| r.duration).unwrap_or(0.0));
        std::thread::spawn(move || {
            let r = AudioFilePlayer::open(&path.to_string_lossy());
            run_on_main_thread(move || {
                if OPEN_GEN.load(Ordering::Acquire) != gen {
                    if let Ok(p) = r {
                        std::thread::spawn(move || drop(p));
                    }
                    return;
                }
                match r {
                    Ok(p) => {
                        if p.duration_sec() > 0.0 {
                            self.dur.set(p.duration_sec());
                        }
                        self.playing.set(true);
                        self.player.set(Some(Player(Arc::new(Mutex::new(p)))));
                    }
                    Err(e) => self.error(t!("Не удалось открыть запись: {e}", e = e)),
                }
            });
        });
    }

    fn drop_player(self) {
        if let Some(old) = self.player.get_untracked() {
            self.player.set(None);
            std::thread::spawn(move || drop(old));
        }
        self.playing.set(false);
    }

    pub fn close_player(self) {
        OPEN_GEN.fetch_add(1, Ordering::AcqRel);
        self.drop_player();
        self.selected.set(None);
    }

    pub fn toggle_play(self) {
        let Some(p) = self.player.get_untracked() else {
            if let Some(s) = self.selected.get_untracked() {
                self.play(s);
            }
            return;
        };
        let mut g = p.0.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_ended() {
            let _ = g.seek(0.0);
            g.play();
            self.pos.set(0.0);
        } else if g.is_paused() {
            g.play();
        } else {
            g.pause();
        }
        self.playing.set(!g.is_paused());
    }

    /// Перемотка: позиция сразу на экране, декодер — через 120 мс после последнего сдвига.
    pub fn seek(self, sec: f64) {
        let d = self.dur.get_untracked();
        let sec = if d > 0.0 { sec.clamp(0.0, d) } else { sec.max(0.0) };
        self.pos.set(sec);
        SEEKING.store(true, Ordering::Release);
        let gen = SEEK_GEN.fetch_add(1, Ordering::AcqRel) + 1;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            run_on_main_thread(move || {
                if SEEK_GEN.load(Ordering::Acquire) != gen {
                    return;
                }
                if let Some(p) = self.player.get_untracked() {
                    let mut g = p.0.lock().unwrap_or_else(|e| e.into_inner());
                    if let Err(e) = g.seek(sec) {
                        tracing::warn!("перемотка: {e}");
                    }
                    if g.is_ended() || !self.playing.get_untracked() {
                        g.pause();
                    }
                }
                SEEKING.store(false, Ordering::Release);
            });
        });
    }

    pub fn skip(self, delta: f64) {
        self.seek(self.pos.get_untracked() + delta);
    }

    pub fn delete(self, path: PathBuf) {
        if self.selected.get_untracked().as_ref() == Some(&path) {
            self.close_player();
        }
        let title = self.recordings.get_untracked().iter().find(|r| r.path == path).map(|r| r.title.clone()).unwrap_or_default();
        match store::trash(&path) {
            Ok(t) => {
                self.recordings.update(|v| v.retain(|r| r.path != path));
                self.show_toast(t!("Удалено: {t}", t = title), Some((t, path)), false);
            }
            Err(e) => self.error(t!("Не удалось удалить: {e}", e = e)),
        }
    }

    pub fn undo_delete(self) {
        let Some(Toast { undo: Some((t, orig)), .. }) = self.toast.get_untracked() else { return };
        self.toast.set(None);
        match store::untrash(&t, &orig) {
            Ok(()) => self.rescan(),
            Err(e) => self.error(t!("Не удалось вернуть: {e}", e = e)),
        }
    }

    pub fn do_rename(self, path: PathBuf, title: String) {
        let title = title.trim().to_string();
        if title.is_empty() {
            return;
        }
        let was_selected = self.selected.get_untracked().as_ref() == Some(&path);
        if was_selected {
            self.close_player();
        }
        match store::rename(&path, &title) {
            Ok(n) => {
                self.recordings.update(|v| {
                    if let Some(r) = v.iter_mut().find(|r| r.path == path) {
                        r.path = n.clone();
                        r.title = title.clone();
                    }
                });
                if was_selected {
                    self.selected.set(Some(n));
                }
            }
            Err(e) => self.error(t!("Не удалось переименовать: {e}", e = e)),
        }
    }

    pub fn open_folder(self) {
        let dir = self.settings.get_untracked().dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Err(e) = std::process::Command::new("xdg-open").arg(&dir).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
            self.error(t!("Не удалось открыть папку: {e}", e = e));
        }
    }

    // ─── тикер ─────────────────────────────────────────────────────────────

    fn start_ticker(self) {
        std::thread::Builder::new()
            .name("sar-tick".into())
            .spawn(move || {
                let mut n = 0u64;
                loop {
                    std::thread::sleep(Duration::from_millis(100));
                    n += 1;
                    let slow = n % 10 == 0;
                    run_on_main_thread(move || self.tick(slow));
                }
            })
            .ok();
    }

    fn tick(self, slow: bool) {
        // таймер и перегрузка
        let (secs, clip) = {
            let v = engine::vis();
            (v.elapsed(), v.clip_at.is_some_and(|t| t.elapsed() < Duration::from_millis(1500)))
        };
        let rec = self.rec.get_untracked();
        if rec.active() {
            let t = (secs * 10.0) as u64;
            if self.tenths.get_untracked() != t {
                self.tenths.set(t);
            }
            if let Some(e) = engine::take_error() {
                self.error(e);
                self.stop();
            }
        }
        if self.clip.get_untracked() != clip {
            self.clip.set(clip);
        }
        // плеер
        if let Some(p) = self.player.get_untracked() {
            let (pos, ended, paused) = {
                let g = p.0.lock().unwrap_or_else(|e| e.into_inner());
                (g.position_sec(), g.is_ended(), g.is_paused())
            };
            if self.playing.get_untracked() && !SEEKING.load(Ordering::Acquire) {
                if (pos - self.pos.get_untracked()).abs() > 0.05 {
                    self.pos.set(pos);
                }
                if ended || paused {
                    self.playing.set(false);
                    if ended {
                        self.pos.set(self.dur.get_untracked());
                    }
                }
            }
        }
        if !slow {
            return;
        }
        if self.just_saved.get_untracked().is_some() && SAVED_AT.with(|c| c.get()).is_none_or(|t| t.elapsed() > UNDO_SAVE) {
            self.just_saved.set(None);
        }
        // монитор: открыт, пока окно активно; закрывается после 2 мин без фокуса и при сворачивании
        let w = self.window.get_untracked();
        let focused = w.focused && !SUSPENDED.with(|c| c.get());
        UNFOCUSED_SINCE.with(|c| match (focused, c.get()) {
            (true, _) => c.set(None),
            (false, None) => c.set(Some(Instant::now())),
            _ => {}
        });
        let away = SUSPENDED.with(|c| c.get()) || UNFOCUSED_SINCE.with(|c| c.get()).is_some_and(|t| t.elapsed() > IDLE_CLOSE);
        let s = self.settings.get_untracked();
        let mic = self.mic.get_untracked();
        // уровень нужен только на экране записи и не во время прослушивания: открытый микрофон держит поток
        // записи (на телефоне — единственный поток PAL: другой микрофон в других программах был бы тишиной)
        let shown = self.page.get_untracked() == Page::Record && !self.playing.get_untracked();
        if rec == Rec::Idle {
            if (away || !s.vis_always || !shown) && mic == MicState::Open {
                self.mic.set(MicState::Closed);
                std::thread::spawn(engine::close_mic);
            } else if s.vis_always && focused && shown && mic == MicState::Closed {
                self.open_mic_then(|_| {});
            }
        }
        // шумоподавление: состояние могли поменять в «Параметрах»
        if focused && LAST_NS.with(|c| c.get()).is_none_or(|t| t.elapsed() > Duration::from_secs(4)) {
            self.refresh_ns();
        }
    }
}

// ─── окно ──────────────────────────────────────────────────────────────────

/// Широкое окно — записи сбоку и заголовок с кнопками окна.
fn wide() -> RwSignal<bool> {
    let narrow = viewport_below(760.0);
    let w = use_signal(!narrow.get_untracked());
    create_effect(move || {
        let v = !narrow.get();
        if w.get_untracked() != v {
            w.set(v);
        }
    });
    w
}

pub fn brand_mark() -> W {
    boxed(DecoratedBox::new().class("brand").child(icon(icons::MIC).class("ic brand-ic")))
}

/// Точка «идёт запись» в заголовке.
fn rec_badge(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let r = st.rec.get();
        if !r.active() {
            return vec![];
        }
        let _ = st.tenths.get();
        let (hms, _) = store::timer(st.tenths.get_untracked() as f64 / 10.0);
        vec![boxed(
            DecoratedBox::new().class(if r == Rec::Paused { "rec-badge paused" } else { "rec-badge" }).child(
                Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("rec-dot")).child(Text::new(hms).class("rec-badge-text")),
            ),
        )]
    }))
}

fn titlebar(st: St) -> W {
    let controls = Reactive::new(move || -> Vec<W> {
        let w = st.window.get();
        vec![boxed(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    boxed(
        DecoratedBox::new().class("titlebar").child(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(DecoratedBox::new().class("tb-left").child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(brand_mark()).child(Text::new(t!("Диктофон")).class("brand-title"))))
                .child(DecoratedBox::new().class("grow drag-wrap").child(WindowDragRegion::new().child(DecoratedBox::new().class("drag-space").child(rec_badge(st)))))
                .child(DecoratedBox::new().class("tb-tools").child(Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon_button(icons::SETTINGS, t!("Настройки"), "tb-btn", move || st.sheet.set(Some(Sheet::Settings))))))
                .child(DecoratedBox::new().class("window-controls").child(controls)),
        ),
    )
}

/// Верхняя панель узкого окна.
fn appbar(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let page = st.page.get();
        let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
        let title = match page {
            Page::Record => {
                row = row.child(DecoratedBox::new().class("appbar-mark").child(brand_mark()));
                t!("Диктофон")
            }
            Page::List => {
                row = row.child(icon_button(icons::BACK, t!("Назад"), "tb-btn", move || st.page.set(Page::Record)));
                t!("Записи")
            }
        };
        row = row.child(DecoratedBox::new().class("grow").child(WindowDragRegion::new().child(DecoratedBox::new().class("appbar-drag").child(Text::new(title).max_lines(1).elide(Elide::End).class("appbar-title")))));
        match page {
            Page::Record => {
                row = row.child(rec_badge(st));
                row = row.child(list_button(st));
            }
            Page::List => {
                row = row.child(icon_button(icons::FOLDER, t!("Открыть папку"), "tb-btn", move || st.open_folder()));
            }
        }
        row = row.child(icon_button(icons::SETTINGS, t!("Настройки"), "tb-btn", move || st.sheet.set(Some(Sheet::Settings))));
        vec![boxed(DecoratedBox::new().class("appbar").child(row))]
    }))
}

/// «Записи» с числом — на узком окне.
fn list_button(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let n = st.recordings.get().len();
        let b = GestureDetector::new().on_click(move || st.page.set(Page::List)).child(
            DecoratedBox::new().class("list-chip").child(
                Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(icons::LIST).class("ic list-chip-ic")).child(Text::new(n.to_string()).class("list-chip-text")),
            ),
        );
        vec![tip(b, t!("Записи"))]
    }))
}

fn toast_layer(st: St) -> W {
    let toast = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let Some(t) = st.toast.get() else { return vec![] };
        let mut row = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if t.error {
            row = row.child(icon(icons::WARNING).class("ic toast-warn"));
        }
        row = row.child(Text::new(t.text.clone()).max_lines(3).class("toast-text"));
        if t.undo.is_some() {
            row = row.child(Button::new(t!("Отменить")).class("btn toast-btn").on_click(move || st.undo_delete()));
        }
        vec![boxed(DecoratedBox::new().class(if t.error { "toast error" } else { "toast" }).child(row))]
    });
    boxed(Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center).class("toast-wrap").child(toast))
}

/// Клавиши: Пробел — запись/пауза, Ctrl+S или Enter — стоп и сохранить, Esc — закрыть лист/назад,
/// K — играть/пауза записи в списке, F11 — во весь экран.
fn keys(st: St, inner: impl Widget + 'static) -> W {
    boxed(
        EventHook::new()
            .on_back(move || {
                if st.sheet.get_untracked().is_some() {
                    st.sheet.set(None);
                    return true;
                }
                if st.rename.get_untracked().is_some() {
                    st.rename.set(None);
                    return true;
                }
                if st.page.get_untracked() == Page::List {
                    st.page.set(Page::Record);
                    return true;
                }
                false
            })
            .on_suspend(move || {
                SUSPENDED.with(|c| c.set(true));
                if st.rec.get_untracked() == Rec::Idle && st.mic.get_untracked() == MicState::Open {
                    st.mic.set(MicState::Closed);
                    std::thread::spawn(engine::close_mic);
                }
            })
            .on_resume(move || SUSPENDED.with(|c| c.set(false)))
            .on_key_down(move |k, m| {
                let modal = st.sheet.get_untracked().is_some() || st.rename.get_untracked().is_some();
                if k == Key::F11 {
                    syngui::signal::toggle_fullscreen();
                    return KeyReply::Handled;
                }
                if k == Key::Escape {
                    if st.rename.get_untracked().is_some() {
                        st.rename.set(None);
                        return KeyReply::Handled;
                    }
                    if st.sheet.get_untracked().is_some() {
                        st.sheet.set(None);
                        return KeyReply::Handled;
                    }
                    if st.page.get_untracked() == Page::List {
                        st.page.set(Page::Record);
                        return KeyReply::Handled;
                    }
                    return KeyReply::Ignore;
                }
                if modal || m.alt {
                    return KeyReply::Ignore;
                }
                match k {
                    Key::S if m.ctrl => {
                        st.stop();
                        KeyReply::Handled
                    }
                    _ if m.ctrl => KeyReply::Ignore,
                    Key::Space => {
                        st.primary();
                        KeyReply::Handled
                    }
                    Key::Enter if st.rec.get_untracked().active() => {
                        st.stop();
                        KeyReply::Handled
                    }
                    Key::K => {
                        st.toggle_play();
                        KeyReply::Handled
                    }
                    Key::Left if st.player.get_untracked().is_some() => {
                        st.skip(-5.0);
                        KeyReply::Handled
                    }
                    Key::Right if st.player.get_untracked().is_some() => {
                        st.skip(5.0);
                        KeyReply::Handled
                    }
                    _ => KeyReply::Ignore,
                }
            })
            .child(inner),
    )
}

pub fn root(st: St) -> W {
    let wide = wide();
    boxed(Reactive::new(move || -> Vec<W> {
        let w = wide.get();
        let body: W = if w {
            boxed(Row::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow").child(list::sidebar(st)).child(DecoratedBox::new().class("grow body").child(record::page(st, true))))
        } else {
            boxed(Reactive::new(move || -> Vec<W> {
                vec![match st.page.get() {
                    Page::Record => boxed(DecoratedBox::new().class("grow body").child(record::page(st, false))),
                    Page::List => boxed(DecoratedBox::new().class("grow body").child(list::page(st))),
                }]
            }))
        };
        let main = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class(if w { "window" } else { "window phone" }).child(if w { titlebar(st) } else { appbar(st) }).child(DecoratedBox::new().class("grow").child(body));
        let stack = Stack::new().fit(StackFit::Expand).child(main).child(toast_layer(st)).child(sheets::layer(st, w));
        let frame = DecoratedBox::new().class(if w { "window-frame" } else { "window-frame flat" }).clip(true).child(keys(st, stack));
        vec![boxed(WindowResizeRegion::new().over_content(true).inset(5.0).enabled(w).child(frame))]
    }))
}
