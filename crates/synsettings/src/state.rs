//! Реактивное состояние окна: текущая страница, ревизии документа, тема.
//!
//! Две ревизии, чтобы ввод не терял фокус:
//! - `rev` — «структура поменялась» (добавили панель, перечитали файл):
//!   страница строится заново;
//! - `tick` — «поменялось любое значение»: от неё зависят только мелкие
//!   подписи (заголовки карточек правил, предпросмотры), не поля ввода.

use std::cell::RefCell;

use synshell_common::Config;
use syngui::prelude::*;

#[derive(Clone, Copy)]
pub struct Ctx {
    pub page: RwSignal<String>,
    pub rev: RwSignal<u64>,
    pub tick: RwSignal<u64>,
    pub theme: RwSignal<String>,
    pub error: RwSignal<Option<String>>,
    pub search: RwSignal<String>,
    /// Какое сочетание сейчас «слушает» нажатие (ключ строки или `+new`).
    pub capture: RwSignal<Option<String>>,
    /// Сообщение внизу окна («Сохранено», ошибки внешних команд).
    pub toast: RwSignal<String>,
    /// Узкое окно (телефон): вместо боковой панели — стек «список
    /// разделов → страница»; `true` — открыта страница.
    pub page_open: RwSignal<bool>,
    /// Подстраница внутри страницы (раздел «Оборудования»): ключ и заголовок
    /// для шапки. «Назад» сначала закрывает её; смена страницы — сбрасывает.
    pub sub: RwSignal<Option<(String, String)>>,
    pub sidebar_width: RwSignal<f32>,
}

pub const SIDEBAR_WIDTH: f32 = 264.0;
pub const SIDEBAR_MIN: f32 = 200.0;
pub const SIDEBAR_MAX: f32 = 480.0;

fn sidebar_file() -> std::path::PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| synshell_common::paths::home().join(".local/state"))
        .join("synshell/settings-sidebar")
}

fn load_sidebar_width() -> f32 {
    std::fs::read_to_string(sidebar_file())
        .ok()
        .and_then(|t| t.trim().parse::<f32>().ok())
        .filter(|w| w.is_finite())
        .map_or(SIDEBAR_WIDTH, |w| w.clamp(SIDEBAR_MIN, SIDEBAR_MAX))
}

pub fn save_sidebar_width() {
    let Some(ctx) = try_ctx() else { return };
    let w = ctx.sidebar_width.get_untracked().round();
    if w == load_sidebar_width() {
        return;
    }
    let f = sidebar_file();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, w.to_string());
}

thread_local! {
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

/// Создать сигналы (на главном потоке, до построения интерфейса).
pub fn init(start: &str) -> Ctx {
    let (start_page, sub) = match start.split_once('/') {
        Some((p, _)) => (p, Some((start.to_string(), String::new()))),
        None => (start, None),
    };
    let ctx = Ctx {
        page: use_signal(start_page.to_string()),
        rev: use_signal(0u64),
        tick: use_signal(0u64),
        theme: use_signal(theme_mss(&crate::store::config())),
        error: use_signal(crate::store::error()),
        search: use_signal(String::new()),
        capture: use_signal(None),
        toast: use_signal(String::new()),
        page_open: use_signal(false),
        sub: use_signal(sub),
        sidebar_width: use_signal(load_sidebar_width()),
    };
    CTX.with(|c| *c.borrow_mut() = Some(ctx));
    ctx
}

pub fn ctx() -> Ctx {
    CTX.with(|c| c.borrow().expect("state::init не вызван"))
}

fn try_ctx() -> Option<Ctx> {
    CTX.with(|c| *c.borrow())
}

/// Полная таблица стилей окна: палитра из `[appearance]` + свой MSS.
pub fn theme_mss(c: &Config) -> String {
    // «Анимации» сразу и в самом окне настроек (переключатель на странице анимаций)
    syngui::animation::set_enabled(c.animations.enabled && !synshell_common::config::eink());
    syngui::animation::set_speed(c.animations.speed);
    let a = &c.appearance;
    let mut s = a.mss_variables();
    // Цвета, которых нет в общей палитре, — производные для окна настроек.
    let p = a.palette();
    let dark = a.is_dark();
    let sidebar = if dark { p.bg.mix(p.surface, 0.45) } else { p.surface_alt.mix(p.bg, 0.4) };
    let card = if dark { p.surface } else { p.surface };
    let input = if dark { p.bg.mix(p.surface, 0.35) } else { p.surface };
    s.push_str(&format!(
        ":root {{\n  --sidebar-bg: {};\n  --card-bg: {};\n  --input-bg: {};\n  --shadow: {};\n  --accent-hover: {};\n  --danger-soft: {};\n}}\n",
        sidebar.hex(),
        card.hex(),
        input.hex(),
        if dark { "#00000066" } else { "#1a203014" },
        p.accent.mix(p.fg, 0.15).hex(),
        p.danger.with_alpha(0.16).hex(),
    ));
    s.push_str(&a.theme_mss_variables());
    s.push_str(STYLES);
    s.push_str(&crate::pages::themes_gallery_mss(a.color_scheme));
    s.push_str(a.theme_mss(true));
    // Фон превью обоев и рабочего стола на страницах оформления.
    s.push_str(&format!("\n.desk-bg {{ background: {}; }}\n", a.wallpaper_background(&c.wallpaper)));
    if let Ok(user) = std::fs::read_to_string(synshell_common::paths::user_theme_file()) {
        // Пользовательская тема оболочки влияет и на настройки — один вид.
        s.push('\n');
        s.push_str(&user);
    }
    s
}

pub const STYLES: &str = include_str!("../styles/settings.mss");

/// Вызывается хранилищем после любой правки.
pub fn notify_changed() {
    let Some(ctx) = try_ctx() else { return };
    ctx.tick.set(ctx.tick.get_untracked() + 1);
    let err = crate::store::error();
    if ctx.error.get_untracked() != err {
        ctx.error.set(err);
    }
    let mss = theme_mss(&crate::store::config());
    if ctx.theme.get_untracked() != mss {
        ctx.theme.set(mss);
    }
}

/// Пересобрать таблицу стилей окна (без ревизии значений).
pub fn refresh_theme() {
    let Some(ctx) = try_ctx() else { return };
    let mss = theme_mss(&crate::store::config());
    if ctx.theme.get_untracked() != mss {
        ctx.theme.set(mss);
    }
}

/// Структура документа поменялась — перестроить страницу.
pub fn bump() {
    let Some(ctx) = try_ctx() else { return };
    ctx.rev.set(ctx.rev.get_untracked() + 1);
}

pub fn toast(msg: impl Into<String>) {
    if let Some(ctx) = try_ctx() {
        ctx.toast.set(msg.into());
    }
}

/// Открыть страницу `id` (подстраница сбрасывается).
pub fn open_page(ctx: Ctx, id: &str) {
    quiet(|| {
        if ctx.sub.get_untracked().is_some() {
            ctx.sub.set(None);
        }
    });
    ctx.page.set(id.to_string());
}

type Loc = (String, Option<(String, String)>);

#[derive(Default)]
struct History {
    back: Vec<Loc>,
    fwd: Vec<Loc>,
    cur: Option<Loc>,
    quiet: bool,
}

thread_local! {
    static HISTORY: RefCell<History> = RefCell::new(History::default());
}

pub fn history_rev() -> RwSignal<u64> {
    thread_local! {
        static REV: std::cell::Cell<Option<RwSignal<u64>>> = const { std::cell::Cell::new(None) };
    }
    REV.with(|r| match r.get() {
        Some(s) => s,
        None => {
            let s = use_signal(0u64);
            r.set(Some(s));
            s
        }
    })
}

fn quiet(f: impl FnOnce()) {
    HISTORY.with(|h| h.borrow_mut().quiet = true);
    f();
    HISTORY.with(|h| h.borrow_mut().quiet = false);
}

fn loc(ctx: Ctx) -> Loc {
    (ctx.page.get_untracked(), ctx.sub.get_untracked())
}

pub fn track_history(ctx: Ctx) {
    let rev = history_rev();
    create_detached_effect(move || {
        let page = ctx.page.get();
        let sub = ctx.sub.get();
        let changed = HISTORY.with(|h| {
            let mut h = h.borrow_mut();
            if h.quiet {
                return false;
            }
            let now = (page, sub);
            match h.cur.replace(now.clone()) {
                Some(prev) if prev != now => {
                    h.back.push(prev);
                    h.fwd.clear();
                    true
                }
                _ => false,
            }
        });
        if changed {
            rev.set(rev.get_untracked() + 1);
        }
    });
}

pub fn can_go(back: bool) -> bool {
    HISTORY.with(|h| {
        let h = h.borrow();
        if back { !h.back.is_empty() } else { !h.fwd.is_empty() }
    })
}

pub fn history_go(ctx: Ctx, back: bool) -> bool {
    let cur = loc(ctx);
    let Some(target) = HISTORY.with(|h| {
        let mut h = h.borrow_mut();
        let t = if back { h.back.pop() } else { h.fwd.pop() }?;
        if back { h.fwd.push(cur) } else { h.back.push(cur) }
        h.cur = Some(t.clone());
        Some(t)
    }) else {
        return false;
    };
    quiet(|| {
        if ctx.sub.get_untracked().is_some() {
            ctx.sub.set(None);
        }
        if ctx.page.get_untracked() != target.0 {
            ctx.page.set(target.0.clone());
        }
        if target.1.is_some() {
            ctx.sub.set(target.1.clone());
        }
    });
    let rev = history_rev();
    rev.set(rev.get_untracked() + 1);
    true
}
