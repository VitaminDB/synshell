//! Реактивное состояние окна: текущая страница, ревизии документа, тема.
//!
//! Две ревизии, чтобы ввод не терял фокус:
//! - `rev` — «структура поменялась» (добавили панель, перечитали файл):
//!   страница строится заново;
//! - `tick` — «поменялось любое значение»: от неё зависят только мелкие
//!   подписи (заголовки карточек правил, предпросмотры), не поля ввода.

use std::cell::RefCell;

use syndesktop_common::config::Appearance;
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
}

thread_local! {
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

/// Создать сигналы (на главном потоке, до построения интерфейса).
pub fn init(start_page: &str) -> Ctx {
    let ctx = Ctx {
        page: use_signal(start_page.to_string()),
        rev: use_signal(0u64),
        tick: use_signal(0u64),
        theme: use_signal(theme_mss(&crate::store::config().appearance)),
        error: use_signal(crate::store::error()),
        search: use_signal(String::new()),
        capture: use_signal(None),
        toast: use_signal(String::new()),
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
pub fn theme_mss(a: &Appearance) -> String {
    let mut s = a.mss_variables();
    // Цвета, которых нет в общей палитре, — производные для окна настроек.
    let p = a.palette();
    let dark = a.color_scheme == syndesktop_common::config::ColorScheme::Dark;
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
    s.push_str(STYLES);
    if let Ok(user) = std::fs::read_to_string(syndesktop_common::paths::user_theme_file()) {
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
    let mss = theme_mss(&crate::store::config().appearance);
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
