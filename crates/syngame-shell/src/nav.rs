//! Состояние лаунчера и навигация: вкладки, разделы (полки и сетки),
//! выделение, запуск игр, меню действий игры. Геймпад и клавиатура дают
//! одни и те же [`Nav`]; куда они идут — в быстрое меню, на главный экран
//! или (в игре) только Guide — решает [`dispatch`].

use std::cell::Cell;
use std::sync::Arc;
use std::time::Duration;

use synshell_common::ipc::WindowOp;
use synshell_ui::ShellCtx;
use syngui::prelude::*;
use syngui::input::Key;
use syngui_layer::KeyInfo;

pub use crate::gamepad::Nav;
use crate::library::{self, Game, Library, Source, UserState};

/// Чем пользователь управлял последним — от этого подсказки внизу.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Gamepad,
    Keyboard,
    Pointer,
}

/// Где фокус на главном экране.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Tabs,
    Content,
}

/// Раздел вкладки: полка (лента вбок) или сетка.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub key: String,
    pub title: String,
    pub items: Vec<Game>,
    pub grid: bool,
    /// Квадратные плитки со значком (приложения), а не обложки.
    pub icons: bool,
}

/// Строка навигации: кусок раздела (у сетки — ряд, у полки — вся полка).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavRow {
    pub section: usize,
    pub start: usize,
    pub len: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SheetItem {
    Play,
    Resume,
    Favorite(bool),
    CloseGame,
    Kill,
    Hide,
}

/// Меню действий игры (X / Options).
#[derive(Debug, Clone, PartialEq)]
pub struct Sheet {
    pub game: Game,
    pub items: Vec<SheetItem>,
    pub sel: usize,
}

#[derive(Clone, Copy)]
pub struct Gs {
    pub lib: RwSignal<Arc<Library>>,
    pub user: RwSignal<Arc<UserState>>,
    pub tab: RwSignal<usize>,
    pub zone: RwSignal<Zone>,
    pub sel: RwSignal<(usize, usize)>,
    pub input: RwSignal<Input>,
    pub sections: RwSignal<Arc<Vec<Section>>>,
    pub cols: RwSignal<usize>,
    /// Игра запускается: id и время нажатия (заставка до появления окна).
    pub launching: RwSignal<Option<(String, i64)>>,
    pub sheet: RwSignal<Option<Sheet>>,
    /// Подтверждение «Закрыть игру?» (id окна, имя).
    pub confirm: RwSignal<Option<(u64, String)>>,
}

thread_local! {
    static GS: Cell<Option<Gs>> = const { Cell::new(None) };
}

pub fn gs() -> Gs {
    GS.with(|g| g.get()).expect("состояние gameshell")
}

pub fn init(ctx: ShellCtx) -> Gs {
    let cfg = ctx.cfg();
    let user = UserState::load();
    let lib = Library::scan(&cfg.gameshell, &user);
    let g = Gs {
        lib: use_signal(Arc::new(lib)),
        user: use_signal(Arc::new(user)),
        tab: use_signal(0usize),
        zone: use_signal(Zone::Content),
        sel: use_signal((0usize, 0usize)),
        input: use_signal(Input::Pointer),
        sections: use_signal(Arc::new(Vec::new())),
        cols: use_signal(5usize),
        launching: use_signal(None),
        sheet: use_signal(None),
        confirm: use_signal(None),
    };
    GS.with(|s| s.set(Some(g)));
    let start = cfg.gameshell.start_tab.clone();
    if let Some(i) = tabs(&ctx, &g).iter().position(|t| t.0 == start) {
        g.tab.set(i);
    }
    // Разделы вкладки — пересчёт по библиотеке, избранному, окнам, вкладке.
    create_effect(move || {
        let lib = g.lib.get();
        let user = g.user.get();
        let _ = ctx.windows.get();
        // Вкладка могла исчезнуть (закрыли последнее окно — нет «Запущенных»).
        let ntabs = tabs(&ctx, &g).len().max(1);
        if g.tab.get_untracked() >= ntabs {
            g.tab.set(ntabs - 1);
        }
        let tab = g.tab.get();
        let _ = ctx.generation.get();
        let v = build_sections(&ctx, &g, &lib, &user, tab);
        // Выделение держится за игру, а не за номер строки: появилась полка
        // «Запущено» — выделенная игра остаётся выделенной.
        let keep = selected_pos(&g).and_then(|(si, i)| {
            let s = g.sections.get_untracked();
            s.get(si).map(|sec| (sec.key.clone(), sec.items.get(i).map(|x| x.id.clone())))
        });
        g.sections.set(Arc::new(v));
        if let Some((key, Some(id))) = keep {
            let s = g.sections.get_untracked();
            let found = s.iter().position(|sec| sec.key == key).and_then(|si| s[si].items.iter().position(|x| x.id == id).map(|i| (si, i)));
            if let Some((si, i)) = found {
                let rows = nav_rows(&s, g.cols.get_untracked());
                if let Some((r, row)) = rows.iter().enumerate().find(|(_, row)| row.section == si && i >= row.start && i < row.start + row.len) {
                    g.sel.set((r, i - row.start));
                }
            }
        }
        clamp_sel(&g);
    });
    g
}

/// Пересканировать библиотеку (новая игра в Steam, правка конфига).
pub fn rescan(ctx: &ShellCtx) {
    let g = gs();
    let cfg = ctx.cfg();
    let user = g.user.get_untracked();
    let lib = Library::scan(&cfg.gameshell, &user);
    if *g.lib.get_untracked().games != lib.games || *g.lib.get_untracked().apps != lib.apps {
        g.lib.set(Arc::new(lib));
    }
}

// ─── Вкладки и разделы ──────────────────────────────────────────────────────

/// Вкладки: ключ, подпись, значок.
pub fn tabs(ctx: &ShellCtx, g: &Gs) -> Vec<(String, String, &'static str)> {
    let cfg = ctx.cfg();
    let lib = g.lib.get_untracked();
    let user = g.user.get_untracked();
    let mut v = Vec::new();
    for t in &cfg.gameshell.tabs {
        let t = t.trim();
        match t {
            "home" => v.push((t.to_string(), t!("Главная").to_string(), "\u{E88A}")),
            "running" if !running(ctx, &lib).is_empty() => v.push((t.to_string(), t!("Запущенные").to_string(), "\u{E53B}")),
            "games" => v.push((t.to_string(), t!("Игры").to_string(), "\u{EA28}")),
            "favorites" if !user.favorites.is_empty() => v.push((t.to_string(), t!("Избранное").to_string(), "\u{E838}")),
            "steam" if lib.games.iter().any(|g| g.source == Source::Steam) => v.push((t.to_string(), "Steam".to_string(), "\u{E30A}")),
            "apps" => v.push((t.to_string(), t!("Приложения").to_string(), "\u{E5C3}")),
            _ => {
                if let Some(tag) = t.strip_prefix("tag:") {
                    if lib.games.iter().any(|g| g.tags.iter().any(|x| x == tag)) {
                        v.push((t.to_string(), tag.to_string(), "\u{E892}"));
                    }
                }
            }
        }
    }
    if v.is_empty() {
        v.push(("games".into(), t!("Игры").to_string(), "\u{EA28}"));
    }
    v
}

/// Запущенные игры и приложения (окна) — по библиотеке; без записи — как есть.
pub fn running(ctx: &ShellCtx, lib: &Library) -> Vec<(Game, u64)> {
    let mut out: Vec<(Game, u64)> = Vec::new();
    for w in ctx.windows.get_untracked().iter().filter(|w| !w.skip_taskbar) {
        if out.iter().any(|(_, id)| *id == w.id) {
            continue;
        }
        let g = lib.games.iter().chain(lib.apps.iter()).find(|g| library::window_matches(g, &w.app_id));
        if let Some(g) = g {
            if out.iter().any(|(x, _)| x.id == g.id) {
                continue;
            }
            out.push((g.clone(), w.id));
        } else {
            out.push((
                Game {
                    id: format!("window:{}", w.id),
                    name: if w.title.is_empty() { w.app_id.clone() } else { w.title.clone() },
                    source: Source::Desktop,
                    command: String::new(),
                    terminal: false,
                    cover: None,
                    hero: None,
                    logo: None,
                    header: None,
                    icon: synshell_common::xdg::lookup_icon(&w.app_id),
                    tags: Vec::new(),
                    app_id: w.app_id.clone(),
                    desktop_id: None,
                    is_game: false,
                    last_played: 0,
                },
                w.id,
            ));
        }
    }
    out
}

fn build_sections(ctx: &ShellCtx, g: &Gs, lib: &Library, user: &UserState, tab: usize) -> Vec<Section> {
    let cfg = ctx.cfg();
    let tabs = tabs(ctx, g);
    let key = tabs.get(tab).map(|t| t.0.clone()).unwrap_or_else(|| "games".into());
    let fav: Vec<Game> = user.favorites.iter().filter_map(|id| lib.find(id).cloned()).collect();
    let grid = |key: &str, title: String, items: Vec<Game>, icons: bool| Section { key: key.into(), title, items, grid: true, icons };
    match key.as_str() {
        "home" => {
            let mut v = Vec::new();
            let run: Vec<Game> = running(ctx, lib).into_iter().map(|(g, _)| g).collect();
            if !run.is_empty() {
                v.push(Section { key: "running".into(), title: t!("Запущено").to_string(), items: run, grid: false, icons: false });
            }
            let recent = lib.recent(cfg.gameshell.recent_count.max(1) as usize);
            if !recent.is_empty() {
                v.push(Section { key: "recent".into(), title: t!("Недавние").to_string(), items: recent, grid: false, icons: false });
            }
            if !fav.is_empty() {
                v.push(Section { key: "fav".into(), title: t!("Избранное").to_string(), items: fav, grid: false, icons: false });
            }
            if !lib.games.is_empty() {
                v.push(Section { key: "all".into(), title: t!("Все игры").to_string(), items: lib.games.clone(), grid: false, icons: false });
            }
            v
        }
        // Окна: закрыть или убить кнопками на плитке (`procs` — свой ключ, не «Запущено» главной).
        "running" => vec![grid("procs", String::new(), running(ctx, lib).into_iter().map(|(g, _)| g).collect(), false)],
        "games" => vec![grid("games", String::new(), lib.games.clone(), false)],
        "favorites" => vec![grid("favorites", String::new(), fav, false)],
        "steam" => vec![grid("steam", String::new(), lib.games.iter().filter(|g| g.source == Source::Steam).cloned().collect(), false)],
        "apps" => vec![grid("apps", String::new(), lib.apps.clone(), true)],
        other => {
            let tag = other.strip_prefix("tag:").unwrap_or(other);
            vec![grid(other, String::new(), lib.games.iter().filter(|g| g.tags.iter().any(|x| x == tag)).cloned().collect(), false)]
        }
    }
}

pub fn nav_rows(sections: &[Section], cols: usize) -> Vec<NavRow> {
    let cols = cols.max(1);
    let mut v = Vec::new();
    for (si, s) in sections.iter().enumerate() {
        if s.items.is_empty() {
            continue;
        }
        if s.grid {
            let mut start = 0;
            while start < s.items.len() {
                let len = cols.min(s.items.len() - start);
                v.push(NavRow { section: si, start, len });
                start += len;
            }
        } else {
            v.push(NavRow { section: si, start: 0, len: s.items.len() });
        }
    }
    v
}

fn rows_now(g: &Gs) -> (Arc<Vec<Section>>, Vec<NavRow>) {
    let s = g.sections.get_untracked();
    let r = nav_rows(&s, g.cols.get_untracked());
    (s, r)
}

fn clamp_sel(g: &Gs) {
    let (_, rows) = rows_now(g);
    let (r, c) = g.sel.get_untracked();
    let fixed = match rows.get(r) {
        Some(row) => (r, c.min(row.len.saturating_sub(1))),
        None if rows.is_empty() => (0, 0),
        None => (rows.len() - 1, c.min(rows[rows.len() - 1].len.saturating_sub(1))),
    };
    if fixed != (r, c) {
        g.sel.set(fixed);
    }
}

/// Имя плитки для `reveal_named`.
pub fn tile_name(section: &str, index: usize) -> String {
    format!("gs-{section}-{index}")
}

/// Раздел и номер в нём выделенной плитки.
pub fn selected_pos(g: &Gs) -> Option<(usize, usize)> {
    let (_, rows) = rows_now(g);
    let (r, c) = g.sel.get_untracked();
    rows.get(r).map(|row| (row.section, row.start + c.min(row.len.saturating_sub(1))))
}

/// Выделенная игра (отслеживает выделение и разделы).
pub fn selected_game(g: &Gs) -> Option<Game> {
    let s = g.sections.get();
    let (r, c) = g.sel.get();
    let rows = nav_rows(&s, g.cols.get());
    let row = rows.get(r)?;
    s.get(row.section)?.items.get(row.start + c.min(row.len.saturating_sub(1))).cloned()
}

fn reveal(g: &Gs) {
    let s = g.sections.get_untracked();
    if let Some((si, i)) = selected_pos(g) {
        if let Some(sec) = s.get(si) {
            syngui::widgets::reveal_named(tile_name(&sec.key, i));
        }
    }
}

/// Выделить плитку (наведение, касание).
pub fn select_item(section: usize, index: usize) {
    let g = gs();
    let (_, rows) = rows_now(&g);
    if let Some((r, row)) = rows.iter().enumerate().find(|(_, row)| row.section == section && index >= row.start && index < row.start + row.len) {
        g.sel.set((r, index - row.start));
        g.zone.set(Zone::Content);
    }
}

// ─── Действия ───────────────────────────────────────────────────────────────

/// Главный экран виден: окон поверх нет (все свёрнуты).
pub fn home_visible(ctx: &ShellCtx) -> bool {
    let active = ctx.active_workspace().unwrap_or(0);
    !ctx.windows.get_untracked().iter().any(|w| !w.minimized && !w.skip_taskbar && (w.workspace == active || w.sticky))
}

/// Свернуть все окна — на главный экран.
pub fn go_home() {
    let ctx = ShellCtx::get();
    for w in ctx.windows.get_untracked().iter().filter(|w| !w.minimized) {
        synshell_ui::actions::window_op(w.id, WindowOp::Minimize);
    }
    crate::overlay::close();
}

pub fn launch(game: &Game) {
    let ctx = ShellCtx::get();
    let g = gs();
    // Уже запущена — поднять окно.
    if let Some(wid) = game.id.strip_prefix("window:").and_then(|s| s.parse::<u64>().ok()) {
        synshell_ui::actions::window_op(wid, WindowOp::Activate);
        return;
    }
    if let Some((_, wid)) = running(&ctx, &g.lib.get_untracked()).into_iter().find(|(x, _)| x.id == game.id) {
        synshell_ui::actions::window_op(wid, WindowOp::Activate);
        return;
    }
    if game.command.trim().is_empty() {
        return;
    }
    let cfg = ctx.cfg();
    if game.terminal {
        synshell_ui::actions::spawn(&format!("{} -e {}", cfg.general.terminal, game.command));
    } else {
        synshell_ui::actions::spawn(&game.command);
    }
    // Запомнить запуск.
    let mut user = (*g.user.get_untracked()).clone();
    user.played.insert(game.id.clone(), library::now());
    user.save();
    g.user.set(Arc::new(user));
    let id = game.id.clone();
    let at = synshell_ui::clock::unix_now();
    g.launching.set(Some((id, at)));
    // Заставка запуска — до окна или не дольше 25 с.
    let windows_before = ctx.windows.get_untracked().len();
    let started = std::time::Instant::now();
    syngui_layer::add_timer(Duration::from_millis(300), move || {
        let ctx = ShellCtx::get();
        let g = gs();
        if g.launching.get_untracked().as_ref().map(|l| l.1) != Some(at) {
            return None;
        }
        if ctx.windows.get_untracked().len() > windows_before || !home_visible(&ctx) || started.elapsed() > Duration::from_secs(25) {
            g.launching.set(None);
            rescan(&ctx);
            return None;
        }
        Some(Duration::from_millis(300))
    });
}

pub fn toggle_favorite(game: &Game) {
    let g = gs();
    let mut user = (*g.user.get_untracked()).clone();
    let on = user.toggle_favorite(&game.id);
    user.save();
    g.user.set(Arc::new(user));
    let ctx = ShellCtx::get();
    let label = if on { t!("«{n}» — в избранном", n = game.name) } else { t!("«{n}» убрана из избранного", n = game.name) };
    synshell_ui::osd::show(ctx, "\u{E838}", None, label.to_string());
}

/// Окно запущенной игры (первое).
pub fn window_of(game: &Game) -> Option<u64> {
    if let Some(w) = game.id.strip_prefix("window:").and_then(|s| s.parse().ok()) {
        return Some(w);
    }
    let ctx = ShellCtx::get();
    running(&ctx, &gs().lib.get_untracked()).into_iter().find(|(x, _)| x.id == game.id).map(|(_, w)| w)
}

/// «Закрыть игру» (с подтверждением, если `[gameshell] confirm_close`).
pub fn close_window(wid: u64, name: String) {
    let g = gs();
    if ShellCtx::get().cfg().gameshell.confirm_close && g.confirm.get_untracked().is_none() {
        g.confirm.set(Some((wid, name)));
        return;
    }
    g.confirm.set(None);
    synshell_ui::actions::window_op(wid, WindowOp::Close);
    // Не закрылось за полторы секунды — приложение, видимо, спрашивает само
    // (несохранённое, запущенная команда): показать его окно с вопросом.
    syngui_layer::add_timer(Duration::from_millis(1500), move || {
        if ShellCtx::get().windows.get_untracked().iter().any(|w| w.id == wid) {
            synshell_ui::actions::window_op(wid, WindowOp::Activate);
        }
        None
    });
}

/// Убить процесс окна (SIGKILL) — когда приложение не закрывается.
pub fn kill_window(wid: u64) {
    synshell_ui::actions::window_op(wid, WindowOp::Kill);
}

fn hide_game(game: &Game) {
    let ctx = ShellCtx::get();
    let mut hide = ctx.cfg().gameshell.hide.clone();
    if !hide.contains(&game.id) {
        hide.push(game.id.clone());
    }
    let arr: toml_edit::Array = hide.iter().map(|s| s.as_str()).collect();
    match synshell_common::config_edit::set_value(&["gameshell", "hide"], toml_edit::Value::Array(arr)) {
        Ok(_) => synshell_ui::reload_after_write(),
        Err(e) => log::warn!("не удалось спрятать игру: {e}"),
    }
}

fn open_sheet(game: Game) {
    let g = gs();
    let running = window_of(&game).is_some();
    let fav = g.user.get_untracked().is_favorite(&game.id);
    let mut items = vec![if running { SheetItem::Resume } else { SheetItem::Play }, SheetItem::Favorite(fav)];
    if running {
        items.push(SheetItem::CloseGame);
        items.push(SheetItem::Kill);
    }
    if !game.id.starts_with("window:") {
        items.push(SheetItem::Hide);
    }
    g.sheet.set(Some(Sheet { game, items, sel: 0 }));
}

pub fn sheet_activate(item: &SheetItem, game: &Game) {
    let g = gs();
    g.sheet.set(None);
    match item {
        SheetItem::Play | SheetItem::Resume => launch(game),
        SheetItem::Favorite(_) => toggle_favorite(game),
        SheetItem::CloseGame => {
            if let Some(w) = window_of(game) {
                close_window(w, game.name.clone());
            }
        }
        SheetItem::Kill => {
            if let Some(w) = window_of(game) {
                kill_window(w);
            }
        }
        SheetItem::Hide => hide_game(game),
    }
}

// ─── Ввод ───────────────────────────────────────────────────────────────────

/// Геймпад (из потока evdev, уже в главном потоке).
pub fn gamepad(n: Nav) {
    dispatch(n, Input::Gamepad);
}

/// Клавиша главного экрана / быстрого меню; `true` — съедена.
pub fn key(k: &KeyInfo) -> bool {
    if !k.pressed {
        return false;
    }
    let shift = k.modifiers.shift;
    let n = match k.key {
        Key::Up => Nav::Up,
        Key::Down => Nav::Down,
        Key::Left => Nav::Left,
        Key::Right => Nav::Right,
        Key::Enter | Key::Space => Nav::Accept,
        Key::Escape | Key::Backspace => Nav::Back,
        Key::Tab if shift => Nav::PrevTab,
        Key::Tab | Key::PageDown => Nav::NextTab,
        Key::PageUp => Nav::PrevTab,
        _ => match k.keysym.as_str() {
            "q" | "Q" | "bracketleft" => Nav::PrevTab,
            "e" | "E" | "bracketright" => Nav::NextTab,
            "f" | "F" | "y" | "Y" => Nav::Favorite,
            "x" | "X" | "Menu" => Nav::Options,
            "m" | "M" | "F1" => Nav::Menu,
            "Return" | "KP_Enter" => Nav::Accept,
            _ => return false,
        },
    };
    dispatch(n, Input::Keyboard);
    true
}

pub fn dispatch(n: Nav, input: Input) {
    let ctx = ShellCtx::get();
    let g = gs();
    if g.input.get_untracked() != input {
        g.input.set(input);
    }
    if crate::overlay::is_open() {
        crate::overlay::handle(n);
        return;
    }
    if !home_visible(&ctx) {
        // В игре геймпад принадлежит игре — кроме Guide.
        if n == Nav::Guide {
            guide();
        }
        return;
    }
    home(n);
}

/// Guide, Super, жест «домой».
pub fn guide() {
    let ctx = ShellCtx::get();
    if crate::overlay::is_open() {
        crate::overlay::close();
        return;
    }
    if home_visible(&ctx) {
        crate::overlay::open(crate::overlay::Section::Main);
        return;
    }
    match ctx.cfg().gameshell.guide_button.trim() {
        "home" => go_home(),
        "none" => {}
        _ => crate::overlay::open(crate::overlay::Section::Main),
    }
}

fn home(n: Nav) {
    let g = gs();
    // Подтверждение закрытия.
    if let Some((wid, name)) = g.confirm.get_untracked() {
        match n {
            Nav::Accept => close_window(wid, name),
            Nav::Back | Nav::Options => g.confirm.set(None),
            _ => {}
        }
        return;
    }
    // Заставка запуска: «назад» убирает её (игра всё равно запускается).
    if g.launching.get_untracked().is_some() {
        if n == Nav::Back {
            g.launching.set(None);
        }
        return;
    }
    // Меню действий игры.
    if let Some(mut sh) = g.sheet.get_untracked() {
        match n {
            Nav::Up => {
                sh.sel = sh.sel.saturating_sub(1);
                g.sheet.set(Some(sh));
            }
            Nav::Down => {
                sh.sel = (sh.sel + 1).min(sh.items.len().saturating_sub(1));
                g.sheet.set(Some(sh));
            }
            Nav::Accept => {
                if let Some(it) = sh.items.get(sh.sel).cloned() {
                    sheet_activate(&it, &sh.game);
                }
            }
            Nav::Back | Nav::Options => g.sheet.set(None),
            _ => {}
        }
        return;
    }
    let ctx = ShellCtx::get();
    let ntabs = tabs(&ctx, &g).len().max(1);
    let switch_tab = |d: i32| {
        let t = (g.tab.get_untracked() as i32 + d).rem_euclid(ntabs as i32) as usize;
        g.tab.set(t);
        g.sel.set((0, 0));
        syngui::widgets::reveal_named(format!("gs-tab-{t}"));
    };
    let (_, rows) = rows_now(&g);
    let (r, c) = g.sel.get_untracked();
    match (g.zone.get_untracked(), n) {
        (_, Nav::PrevTab) => switch_tab(-1),
        (_, Nav::NextTab) => switch_tab(1),
        (_, Nav::Menu) | (_, Nav::Guide) => crate::overlay::open(crate::overlay::Section::Main),
        (Zone::Tabs, Nav::Left) => switch_tab(-1),
        (Zone::Tabs, Nav::Right) => switch_tab(1),
        (Zone::Tabs, Nav::Down) | (Zone::Tabs, Nav::Accept) => {
            if !rows.is_empty() {
                g.zone.set(Zone::Content);
                g.sel.set((0, 0));
            }
        }
        (Zone::Tabs, _) => {}
        (Zone::Content, Nav::Up) => {
            if r == 0 {
                g.zone.set(Zone::Tabs);
            } else {
                let len = rows[r - 1].len;
                g.sel.set((r - 1, c.min(len.saturating_sub(1))));
            }
        }
        (Zone::Content, Nav::Down) => {
            if r + 1 < rows.len() {
                let len = rows[r + 1].len;
                g.sel.set((r + 1, c.min(len.saturating_sub(1))));
            }
        }
        (Zone::Content, Nav::Left) => {
            if c > 0 {
                g.sel.set((r, c - 1));
            }
        }
        (Zone::Content, Nav::Right) => {
            if rows.get(r).is_some_and(|row| c + 1 < row.len) {
                g.sel.set((r, c + 1));
            }
        }
        (Zone::Content, Nav::Accept) => {
            if let Some(game) = selected_game_untracked(&g) {
                launch(&game);
            }
        }
        (Zone::Content, Nav::Favorite) => {
            if let Some(game) = selected_game_untracked(&g) {
                if !game.id.starts_with("window:") {
                    toggle_favorite(&game);
                }
            }
        }
        (Zone::Content, Nav::Options) => {
            if let Some(game) = selected_game_untracked(&g) {
                open_sheet(game);
            }
        }
        (Zone::Content, Nav::Back) => {
            // Назад: к началу вкладки, затем к стартовой вкладке.
            let start = tabs(&ctx, &g).iter().position(|t| t.0 == ctx.cfg().gameshell.start_tab.trim()).unwrap_or(0);
            if (r, c) != (0, 0) {
                g.sel.set((0, 0));
            } else if g.tab.get_untracked() != start {
                g.tab.set(start);
            }
        }
    }
    if g.zone.get_untracked() == Zone::Content {
        reveal(&g);
    }
}

fn selected_game_untracked(g: &Gs) -> Option<Game> {
    let s = g.sections.get_untracked();
    let (si, i) = selected_pos(g)?;
    s.get(si)?.items.get(i).cloned()
}
