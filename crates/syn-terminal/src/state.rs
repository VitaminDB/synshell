//! Вкладки и их сессии, горячие клавиши, оформление окна.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use syngui::input::{Key, Modifiers};
use syngui::prelude::*;
use syngui::widgets::visual::terminal::TerminalCommand;
use syngui::widgets::{TerminalConfig, TerminalSession};
use synshell_common::config::Rgba;
use synshell_common::Config;

use crate::schemes::{self, Scheme};
use crate::settings::{Settings, FONT_MAX, FONT_MIN};

#[derive(Clone)]
pub struct Tab {
    pub id: u64,
    pub session: TerminalSession,
    /// Заголовок, который поставила программа (OSC 0/2).
    pub title: RwSignal<String>,
    /// Текущая папка программы (опрашивается раз в секунду).
    pub cwd: RwSignal<Option<PathBuf>>,
    /// В терминале работает команда (не простаивает в приглашении).
    pub busy: RwSignal<bool>,
    /// Копировать/вставить — в виджет терминала.
    pub cmd: RwSignal<Option<TerminalCommand>>,
    /// Имя программы — подпись, пока нет ни заголовка, ни папки.
    pub program: String,
}

impl PartialEq for Tab {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id
    }
}

impl Tab {
    /// Подпись вкладки: заголовок программы, иначе папка, иначе имя программы.
    pub fn label(&self) -> String {
        let t = self.title.get();
        if !t.trim().is_empty() {
            return t;
        }
        match self.cwd.get() {
            Some(d) => short_path(&d),
            None => self.program.clone(),
        }
    }
}

/// `~/…` вместо домашней папки.
pub fn short_path(p: &Path) -> String {
    let home = synshell_common::paths::home();
    match p.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

/// Что спрашиваем перед закрытием.
#[derive(Clone, Copy, PartialEq)]
pub enum Ask {
    Tab(u64),
    Window,
}

#[derive(Clone, Copy)]
pub struct St {
    pub tabs: RwSignal<Vec<Tab>>,
    /// id активной вкладки.
    pub cur: RwSignal<u64>,
    pub settings: RwSignal<Settings>,
    pub settings_open: RwSignal<bool>,
    pub theme: RwSignal<String>,
    pub window: RwSignal<syngui::window::WindowState>,
    pub ask: RwSignal<Option<Ask>>,
    pub ask_open: RwSignal<bool>,
    /// Узкое окно (телефон): рамку рисует композитор, внизу ряд клавиш.
    pub phone: RwSignal<bool>,
    /// Размер шрифта сменили клавишами — ползунок в настройках пересобирается
    /// (от собственного перетаскивания — нет).
    pub font_rev: RwSignal<u32>,
    /// Контекстное меню терминала (правая кнопка, долгое нажатие).
    pub menu_open: RwSignal<bool>,
    pub menu_pos: RwSignal<syngui::core::Point>,
    /// Было ли выделение при открытии меню — «Копировать» доступен.
    pub menu_sel: RwSignal<bool>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// Конфиг synshell — для темы окна и схемы «как в системе».
static CONFIG: Mutex<Option<Config>> = Mutex::new(None);

pub fn set_config(cfg: Config) {
    if let Ok(mut g) = CONFIG.lock() {
        *g = Some(cfg);
    }
}

fn with_config<R>(f: impl FnOnce(&Config) -> R) -> R {
    let g = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    match g.as_ref() {
        Some(c) => f(c),
        None => f(&Config::default()),
    }
}

impl St {
    pub fn new(settings: Settings) -> Self {
        let st = Self {
            tabs: use_signal(Vec::new()),
            cur: use_signal(0),
            settings: use_signal(settings),
            settings_open: use_signal(false),
            theme: use_signal(String::new()),
            window: use_signal(syngui::window::WindowState::default()),
            ask: use_signal(None),
            ask_open: use_signal(false),
            phone: use_signal(false),
            font_rev: use_signal(0),
            menu_open: use_signal(false),
            menu_pos: use_signal(syngui::core::Point::zero()),
            menu_sel: use_signal(false),
        };
        st.retheme();
        st
    }

    pub fn scheme(&self) -> Scheme {
        self.scheme_of(&self.settings.get_untracked().scheme)
    }

    pub fn scheme_of(&self, id: &str) -> Scheme {
        with_config(|c| schemes::resolve(id, &c.appearance.palette(), c.appearance.is_dark()))
    }

    /// Пересобрать стили окна (сменились оформление или схема).
    pub fn retheme(&self) {
        let s = self.settings.get_untracked();
        let mss = with_config(|c| theme(c, &s));
        if self.theme.get_untracked() != mss {
            self.theme.set(mss);
        }
    }

    pub fn update(&self, f: impl FnOnce(&mut Settings)) {
        let mut s = self.settings.get_untracked();
        let old = s.clone();
        f(&mut s);
        if s != old {
            s.save();
            let restyle = s.scheme != old.scheme || s.opacity != old.opacity;
            self.settings.set(s);
            if restyle {
                self.retheme();
            }
        }
    }

    pub fn active(&self) -> Option<Tab> {
        let cur = self.cur.get();
        self.tabs.with(|t| t.iter().find(|t| t.id == cur).cloned())
    }

    pub fn active_untracked(&self) -> Option<Tab> {
        let cur = self.cur.get_untracked();
        self.tabs.with_untracked(|t| t.iter().find(|t| t.id == cur).cloned())
    }

    /// Новая вкладка. `cmd` — программа с аргументами (`-e`); `None` — оболочка.
    pub fn new_tab(&self, cmd: Option<Vec<String>>, cwd: Option<PathBuf>) {
        let s = self.settings.get_untracked();
        let mut argv = match cmd {
            Some(v) if !v.is_empty() => v,
            _ => s.shell().split_whitespace().map(String::from).collect(),
        };
        if argv.is_empty() {
            argv.push("/bin/sh".into());
        }
        let program = Path::new(&argv[0]).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| argv[0].clone());
        let cwd = cwd.filter(|d| d.is_dir()).or_else(|| Some(synshell_common::paths::home()));
        let config = TerminalConfig {
            command: argv[0].clone(),
            args: argv[1..].to_vec(),
            cwd: cwd.clone(),
            env: vec![("TERM_PROGRAM".into(), "syn-terminal".into()), ("TERM_PROGRAM_VERSION".into(), env!("CARGO_PKG_VERSION").into())],
            scrollback: s.scrollback,
            ..TerminalConfig::default()
        };
        let session = match TerminalSession::new(config) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("терминал: {e}");
                return;
            }
        };
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let tab = Tab {
            id,
            session: session.clone(),
            title: use_signal(String::new()),
            cwd: use_signal(cwd),
            busy: use_signal(false),
            cmd: use_signal(None),
            program,
        };
        session.set_title_signal(tab.title);
        // Программа завершилась (`exit`, Ctrl+D) — вкладка закрывается.
        let exited = use_signal(false);
        session.set_exit_signal(exited);
        let st = *self;
        syngui::signal::create_detached_effect(move || {
            if exited.get() {
                run_on_main_thread(move || st.remove_tab(id));
            }
        });
        // Новая вкладка встаёт за текущей.
        let cur = self.cur.get_untracked();
        self.tabs.update(|t| {
            let at = t.iter().position(|x| x.id == cur).map(|i| i + 1).unwrap_or(t.len());
            t.insert(at, tab);
        });
        self.cur.set(id);
    }

    /// Новая вкладка — в папке текущей (если так настроено).
    pub fn new_tab_here(&self) {
        let cwd = if self.settings.get_untracked().new_tab_here { self.active_untracked().and_then(|t| t.session.cwd()) } else { None };
        self.new_tab(None, cwd);
    }

    /// Закрыть вкладку; с работающей программой — сначала спросить.
    pub fn close_tab(&self, id: u64) {
        let Some(tab) = self.tabs.with_untracked(|t| t.iter().find(|t| t.id == id).cloned()) else { return };
        if self.settings.get_untracked().confirm_close && tab.session.is_busy() {
            self.ask.set(Some(Ask::Tab(id)));
            self.ask_open.set(true);
            return;
        }
        self.remove_tab(id);
    }

    /// Убрать вкладку без вопросов: процесс получает SIGHUP. Последняя — окно закрывается.
    pub fn remove_tab(&self, id: u64) {
        let tabs = self.tabs.get_untracked();
        let Some(i) = tabs.iter().position(|t| t.id == id) else { return };
        let rest: Vec<Tab> = tabs.iter().filter(|t| t.id != id).cloned().collect();
        if rest.is_empty() {
            self.tabs.set(rest);
            syngui::signal::quit_app();
            return;
        }
        if self.cur.get_untracked() == id {
            let next = rest[i.min(rest.len() - 1)].id;
            self.cur.set(next);
        }
        self.tabs.set(rest);
    }

    /// Закрыть окно: с работающими программами — спросить.
    pub fn request_quit(&self) -> bool {
        let busy = self.tabs.with_untracked(|t| t.iter().any(|t| t.session.is_busy()));
        if busy && self.settings.get_untracked().confirm_close {
            self.ask.set(Some(Ask::Window));
            self.ask_open.set(true);
            return false;
        }
        true
    }

    pub fn select(&self, id: u64) {
        if self.cur.get_untracked() != id {
            self.cur.set(id);
        }
    }

    /// Соседняя вкладка (по кругу).
    pub fn step(&self, delta: isize) {
        let tabs = self.tabs.get_untracked();
        let n = tabs.len() as isize;
        let cur = self.cur.get_untracked();
        if let Some(i) = tabs.iter().position(|t| t.id == cur) {
            if n > 1 {
                self.select(tabs[(i as isize + delta).rem_euclid(n) as usize].id);
            }
        }
    }

    /// Вкладка по номеру (с нуля); 8 — последняя, как в браузерах.
    pub fn select_nth(&self, n: usize) {
        let tabs = self.tabs.get_untracked();
        let tab = if n >= 8 { tabs.last() } else { tabs.get(n) };
        if let Some(t) = tab {
            self.select(t.id);
        }
    }

    /// Сдвинуть текущую вкладку влево (−1) или вправо (+1).
    pub fn move_tab(&self, delta: isize) {
        let cur = self.cur.get_untracked();
        self.tabs.update(|t| {
            if let Some(i) = t.iter().position(|x| x.id == cur) {
                let j = (i as isize + delta).clamp(0, t.len() as isize - 1) as usize;
                if i != j {
                    let tab = t.remove(i);
                    t.insert(j, tab);
                }
            }
        });
    }

    pub fn command(&self, c: TerminalCommand) {
        if let Some(t) = self.active_untracked() {
            t.cmd.set(Some(c));
        }
    }

    /// Открыть контекстное меню в точке окна.
    pub fn open_menu(&self, at: syngui::core::Point) {
        let sel = self.active_untracked().is_some_and(|t| t.session.has_selection());
        if self.menu_sel.get_untracked() != sel {
            self.menu_sel.set(sel);
        }
        self.menu_pos.set(at);
        self.menu_open.set(true);
    }

    /// Пункт контекстного меню.
    pub fn menu_selected(&self, id: &str) {
        self.menu_open.set(false);
        match id {
            "copy" => self.command(TerminalCommand::Copy),
            "paste" => self.command(TerminalCommand::Paste),
            "clear" => self.command(TerminalCommand::Clear),
            "new-tab" => self.new_tab_here(),
            "new-window" => self.new_window(),
            "settings" => self.settings_open.set(true),
            _ => {}
        }
    }

    /// Байты прямо в программу активной вкладки (ряд клавиш телефона).
    pub fn send(&self, bytes: &[u8]) {
        if let Some(t) = self.active_untracked() {
            t.session.write(bytes);
        }
    }

    pub fn zoom(&self, delta: f32) {
        self.update(|s| s.font_size = (s.font_size + delta).clamp(FONT_MIN, FONT_MAX));
        self.font_rev.update(|r| *r += 1);
    }

    /// Новое окно терминала в папке текущей вкладки.
    pub fn new_window(&self) {
        let cwd = self.active_untracked().and_then(|t| t.session.cwd());
        let exe = std::env::current_exe().unwrap_or_else(|_| "syn-terminal".into());
        let mut c = std::process::Command::new(exe);
        if let Some(d) = cwd {
            c.arg("--working-directory").arg(d);
        }
        if let Err(e) = c.spawn() {
            tracing::warn!("новое окно: {e}");
        }
    }

    /// Папка и занятость вкладок — для подписей (раз в секунду).
    pub fn poll(&self) {
        for t in self.tabs.get_untracked() {
            let cwd = t.session.cwd();
            if cwd.is_some() && t.cwd.get_untracked() != cwd {
                t.cwd.set(cwd);
            }
            let busy = t.session.is_busy();
            if t.busy.get_untracked() != busy {
                t.busy.set(busy);
            }
        }
    }
}

// ─── горячие клавиши ───────────────────────────────────────────────────────

/// Коды evdev клавиш, которых нет в `Key` или которые зависят от раскладки.
mod sc {
    pub const N1: u32 = 2;
    pub const N9: u32 = 10;
    pub const N0: u32 = 11;
    pub const MINUS: u32 = 12;
    pub const EQUAL: u32 = 13;
    pub const TAB: u32 = 15;
    pub const W: u32 = 17;
    pub const T: u32 = 20;
    pub const C: u32 = 46;
    pub const V: u32 = 47;
    pub const N: u32 = 49;
    pub const COMMA: u32 = 51;
    pub const KP_MINUS: u32 = 74;
    pub const KP_PLUS: u32 = 78;
    pub const PAGE_UP: u32 = 104;
    pub const PAGE_DOWN: u32 = 109;
}

/// Клавиша по физическому коду (раскладка не важна), иначе по `Key`.
fn code_of(key: Key) -> Option<u32> {
    if let Some(c) = syngui::input::current_scancode() {
        return Some(c);
    }
    Some(match key {
        Key::T => sc::T,
        Key::W => sc::W,
        Key::C => sc::C,
        Key::V => sc::V,
        Key::N => sc::N,
        Key::Tab => sc::TAB,
        Key::PageUp => sc::PAGE_UP,
        Key::PageDown => sc::PAGE_DOWN,
        Key::Num0 => sc::N0,
        Key::Num1 => sc::N1,
        Key::Num2 => sc::N1 + 1,
        Key::Num3 => sc::N1 + 2,
        Key::Num4 => sc::N1 + 3,
        Key::Num5 => sc::N1 + 4,
        Key::Num6 => sc::N1 + 5,
        Key::Num7 => sc::N1 + 6,
        Key::Num8 => sc::N1 + 7,
        Key::Num9 => sc::N9,
        _ => return None,
    })
}

/// Горячие клавиши окна. `true` — клавиша обработана и в терминал не идёт.
pub fn shortcut(st: St, key: Key, m: Modifiers) -> bool {
    let Some(code) = code_of(key) else { return false };
    let ctrl_shift = m.ctrl && m.shift && !m.alt && !m.meta;
    let ctrl_only = m.ctrl && !m.shift && !m.alt && !m.meta;
    match code {
        sc::T if ctrl_shift => st.new_tab_here(),
        sc::W if ctrl_shift => {
            if let Some(t) = st.active_untracked() {
                st.close_tab(t.id);
            }
        }
        sc::N if ctrl_shift => st.new_window(),
        sc::C if ctrl_shift => st.command(TerminalCommand::Copy),
        sc::V if ctrl_shift => st.command(TerminalCommand::Paste),
        sc::PAGE_UP if ctrl_only => st.step(-1),
        sc::PAGE_DOWN if ctrl_only => st.step(1),
        sc::PAGE_UP if ctrl_shift => st.move_tab(-1),
        sc::PAGE_DOWN if ctrl_shift => st.move_tab(1),
        sc::TAB if ctrl_only => st.step(1),
        sc::TAB if ctrl_shift => st.step(-1),
        sc::N1..=sc::N9 if ctrl_only => st.select_nth((code - sc::N1) as usize),
        sc::EQUAL | sc::KP_PLUS if m.ctrl && !m.alt => st.zoom(1.0),
        sc::MINUS | sc::KP_MINUS if ctrl_only => st.zoom(-1.0),
        sc::N0 if ctrl_only => {
            let def = Settings::default().font_size;
            st.update(|s| s.font_size = def);
            st.font_rev.update(|r| *r += 1);
        }
        sc::COMMA if ctrl_only => st.settings_open.set(!st.settings_open.get_untracked()),
        _ => return false,
    }
    true
}

// ─── оформление ────────────────────────────────────────────────────────────

/// Таблица стилей окна: палитра `[appearance]`, цвета схемы, `syn-terminal.mss`,
/// слой Проводника темы (`files.mss` — общая рамка окна), `theme.mss`.
pub fn theme(cfg: &Config, s: &Settings) -> String {
    use synshell_common::style::{px, AppStyle};
    let a = &cfg.appearance;
    let p = a.palette();
    let dark = a.is_dark();
    let sc = schemes::resolve(&s.scheme, &p, dark);
    let term_dark = sc.is_dark();
    // Заголовок с вкладками — чуть темнее (на светлой схеме — чуть серее) фона
    // терминала; активная вкладка сливается с терминалом.
    let titlebar = if term_dark { sc.bg.mix(Rgba::rgb(0, 0, 0), 0.32) } else { sc.bg.mix(Rgba::rgb(0, 0, 0), 0.07) };
    let alpha = s.opacity.clamp(0.5, 1.0);
    let mut out = AppStyle::new(cfg, "syn-terminal")
        .font_selector("Text, Button")
        .vars(&[
            ("term-bg", sc.bg.with_alpha(alpha).hex()),
            ("term-fg", sc.fg.hex()),
            ("term-cursor", sc.cursor.hex()),
            ("term-muted", sc.fg.mix(sc.bg, 0.45).hex()),
            ("term-hover", sc.fg.with_alpha(0.08).hex()),
            ("term-pressed", sc.fg.with_alpha(0.14).hex()),
            ("titlebar", titlebar.with_alpha(alpha).hex()),
            ("tab-active", sc.bg.with_alpha(alpha).hex()),
            ("divider", sc.fg.with_alpha(0.12).hex()),
            ("scrim", (if dark { "#00000080" } else { "#00000040" }).to_string()),
            ("window-radius", px(cfg.decorations.corner_radius.max(0.0))),
        ])
        .push(include_str!("../styles/syn-terminal.mss"))
        .push(a.theme_files_mss())
        .build();
    // Пользовательский `theme.mss` — поверх всего, как и раньше.
    if let Ok(user) = std::fs::read_to_string(synshell_common::paths::user_theme_file()) {
        out.push('\n');
        out.push_str(&user);
    }
    out
}
