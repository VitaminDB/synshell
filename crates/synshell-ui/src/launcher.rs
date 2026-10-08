//! Меню запуска (как Kickoff): поиск по приложениям, избранное, недавние,
//! категории, калькулятор и запуск команды. Два вида: меню у кнопки и
//! сетка на весь экран. Отдельно — строка «Выполнить» (Alt+F2).

use std::path::PathBuf;
use syngui::input::{Key, MouseButton};
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui_layer::KeyInfo;

use crate::ctx::ShellCtx;
use crate::ui::{icon, mi, InputArea};
use crate::xdg::{self, DesktopEntry};

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    App(DesktopEntry),
    Calc { expr: String, value: String },
    Run(String),
}

impl Item {
    pub fn activate(&self, ctx: ShellCtx) {
        match self {
            Item::App(e) => launch(ctx, e),
            Item::Calc { value, .. } => {
                crate::actions::spawn(&format!("printf %s '{}' | wl-copy", value.replace('\'', "")));
            }
            Item::Run(cmd) => {
                let cfg = ctx.cfg();
                let cmd = cmd.strip_prefix('!').map(|c| format!("{} -e sh -c '{}'", cfg.general.terminal, c.replace('\'', "'\\''")));
                crate::actions::spawn(cmd.as_deref().unwrap_or(match self {
                    Item::Run(c) => c,
                    _ => unreachable!(),
                }));
            }
        }
        ctx.close_popup();
    }
}

/// Запустить приложение из .desktop.
pub fn launch(ctx: ShellCtx, e: &DesktopEntry) {
    let cfg = ctx.cfg();
    let cmd = e.command();
    if e.terminal {
        crate::actions::spawn(&format!("{} -e {cmd}", cfg.general.terminal));
    } else {
        crate::actions::spawn(&cmd);
    }
    remember(&e.id);
}

// ─── Недавние ────────────────────────────────────────────────────────────────

fn recent_file() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| synshell_common::paths::expand_tilde("~/.local/state"));
    base.join("synshell/recent-apps.json")
}

pub fn recent() -> Vec<String> {
    std::fs::read_to_string(recent_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn remember(id: &str) {
    let mut list = recent();
    list.retain(|x| x != id);
    list.insert(0, id.to_string());
    list.truncate(16);
    let path = recent_file();
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(path, serde_json::to_string(&list).unwrap_or_default());
}

// ─── Поиск ───────────────────────────────────────────────────────────────────

fn score(e: &DesktopEntry, q: &str, keywords: bool) -> Option<u32> {
    let name = e.name.to_lowercase();
    if name == q {
        return Some(1000);
    }
    if name.starts_with(q) {
        return Some(900 - name.len().min(100) as u32);
    }
    if name.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(q)) {
        return Some(800);
    }
    if name.contains(q) {
        return Some(700);
    }
    let gn = e.generic_name.to_lowercase();
    if gn.contains(q) {
        return Some(600);
    }
    if keywords {
        if e.keywords.iter().any(|k| k.to_lowercase().starts_with(q)) {
            return Some(500);
        }
        let exe = e.exec.split_whitespace().next().unwrap_or("").rsplit('/').next().unwrap_or("").to_lowercase();
        if exe.starts_with(q) {
            return Some(450);
        }
        if e.id.to_lowercase().contains(q) {
            return Some(400);
        }
        if e.comment.to_lowercase().contains(q) {
            return Some(300);
        }
    }
    None
}

pub fn search(ctx: &ShellCtx, query: &str) -> Vec<Item> {
    let cfg = ctx.cfg();
    let q = query.trim().to_lowercase();
    let mut out = Vec::new();
    if q.is_empty() {
        return out;
    }
    if cfg.launcher.calculator {
        if let Some(v) = calc(query.trim()) {
            out.push(Item::Calc { expr: query.trim().into(), value: v });
        }
    }
    let apps = xdg::apps();
    let mut scored: Vec<(u32, &DesktopEntry)> = apps
        .iter()
        .filter(|e| !e.no_display)
        .filter_map(|e| score(e, &q, cfg.launcher.search_keywords).map(|s| (s, e)))
        .collect();
    let rec = recent();
    scored.sort_by(|a, b| {
        let ra = rec.iter().position(|r| *r == a.1.id).map(|p| 50 - p as u32).unwrap_or(0);
        let rb = rec.iter().position(|r| *r == b.1.id).map(|p| 50 - p as u32).unwrap_or(0);
        (b.0 + rb).cmp(&(a.0 + ra)).then_with(|| a.1.name.cmp(&b.1.name))
    });
    out.extend(scored.into_iter().take(30).map(|(_, e)| Item::App(e.clone())));
    if cfg.launcher.run_commands && !query.trim().is_empty() {
        out.push(Item::Run(query.trim().to_string()));
    }
    out
}

/// Приложения раздела меню: `favorites`, `recent`, `all` или категория.
fn section_items(ctx: &ShellCtx, key: &str) -> Vec<Item> {
    let apps = xdg::apps();
    // Программы Linux; приложения Android — только в разделах своих экземпляров (`android:<экземпляр>`)
    let visible = || apps.iter().filter(|e| !e.no_display && e.android.is_none());
    let v: Vec<DesktopEntry> = match key {
        "favorites" => ctx.cfg().launcher.favorites.iter().filter_map(|id| xdg::app_by_id(id)).collect(),
        "recent" => recent().iter().filter_map(|id| xdg::app_by_id(id)).collect(),
        "all" => visible().cloned().collect(),
        k if k.starts_with("android:") => {
            let inst = &k["android:".len()..];
            apps.iter().filter(|e| !e.no_display && e.android.as_ref().is_some_and(|a| a.instance == inst)).cloned().collect()
        }
        cat => visible().filter(|e| e.main_category() == cat).cloned().collect(),
    };
    v.into_iter().map(Item::App).collect()
}

// ─── Калькулятор ─────────────────────────────────────────────────────────────

/// Вычислить арифметическое выражение (`2+2*3`, `(1.5+2)^2`, `10%3`).
pub fn calc(s: &str) -> Option<String> {
    if !s.chars().any(|c| "+-*/^%(".contains(c)) || !s.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    if s.chars().any(|c| c.is_alphabetic()) {
        return None;
    }
    let toks: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).map(|c| if c == ',' { '.' } else { c }).collect();
    let mut p = Parser { t: &toks, i: 0 };
    let v = p.expr()?;
    if p.i != toks.len() || !v.is_finite() {
        return None;
    }
    let r = if (v - v.round()).abs() < 1e-9 && v.abs() < 1e15 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v:.10}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    Some(r)
}

struct Parser<'a> {
    t: &'a [char],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.t.get(self.i).copied()
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        while let Some(c) = self.peek() {
            match c {
                '+' => {
                    self.i += 1;
                    v += self.term()?
                }
                '-' => {
                    self.i += 1;
                    v -= self.term()?
                }
                _ => break,
            }
        }
        Some(v)
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.power()?;
        while let Some(c) = self.peek() {
            match c {
                '*' | '×' => {
                    self.i += 1;
                    v *= self.power()?
                }
                '/' | '÷' => {
                    self.i += 1;
                    v /= self.power()?
                }
                '%' => {
                    self.i += 1;
                    v %= self.power()?
                }
                _ => break,
            }
        }
        Some(v)
    }
    fn power(&mut self) -> Option<f64> {
        let b = self.unary()?;
        if self.peek() == Some('^') {
            self.i += 1;
            let e = self.power()?;
            return Some(b.powf(e));
        }
        Some(b)
    }
    fn unary(&mut self) -> Option<f64> {
        match self.peek()? {
            '-' => {
                self.i += 1;
                Some(-self.unary()?)
            }
            '+' => {
                self.i += 1;
                self.unary()
            }
            '(' => {
                self.i += 1;
                let v = self.expr()?;
                (self.peek() == Some(')')).then(|| self.i += 1)?;
                Some(v)
            }
            _ => {
                let start = self.i;
                while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.') {
                    self.i += 1;
                }
                self.t[start..self.i].iter().collect::<String>().parse().ok()
            }
        }
    }
}

// ─── Вид ─────────────────────────────────────────────────────────────────────

pub fn item_icon(item: &Item, class: &str) -> Box<dyn Widget> {
    match item {
        Item::App(e) => match xdg::lookup_icon(&e.icon).or_else(|| xdg::lookup_icon("application-x-executable")) {
            Some(p) => Box::new(Image::new(p.to_string_lossy()).fit(ImageFit::Contain).placeholder(false).class(class.to_string())),
            None => Box::new(icon(mi::APPS).class(format!("{class}-glyph"))),
        },
        Item::Calc { .. } => Box::new(icon(mi::CALC).class(format!("{class}-glyph"))),
        Item::Run(_) => Box::new(icon(mi::TERMINAL).class(format!("{class}-glyph"))),
    }
}

pub fn item_text(item: &Item) -> (String, String) {
    match item {
        Item::App(e) => {
            // Приложение Android — всегда с подписью, из какого Android (не путать с программами Linux)
            let sub = if let Some(a) = &e.android {
                format!("Android · {}", a.title)
            } else if !e.generic_name.is_empty() && e.generic_name != e.name {
                e.generic_name.clone()
            } else {
                e.comment.clone()
            };
            (e.name.clone(), sub)
        }
        Item::Calc { expr, value } => (format!("= {value}"), format!("{expr} · Enter — скопировать")),
        Item::Run(cmd) => (format!("Выполнить «{cmd}»"), "Команда оболочки (! — в терминале)".into()),
    }
}

fn row(item: Item, index: usize, selected: RwSignal<usize>, ctx: ShellCtx) -> impl Widget {
    let (name, sub) = item_text(&item);
    let mut text = Column::new().gap(0.0).child(Text::new(name).max_lines(1).class("launcher-name"));
    if !sub.is_empty() {
        text = text.child(Text::new(sub).max_lines(1).class("launcher-sub"));
    }
    let content = Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(item_icon(&item, "launcher-icon")).child(text.class("grow"));
    let sel = selected;
    InputArea::new(
        DecoratedBox::new()
            .child(content)
            .class(if sel.get_untracked() == index { "launcher-row launcher-row-selected" } else { "launcher-row" }),
    )
    .pointer()
    .on_hover(move |h| {
        if h {
            sel.set(index);
        }
    })
    .on_click(move |b, _, _| {
        if b == MouseButton::Left {
            item.activate(ctx);
        }
    })
}

#[derive(Clone, Copy)]
struct State {
    query: RwSignal<String>,
    selected: RwSignal<usize>,
    section: RwSignal<String>,
    items: RwSignal<Vec<Item>>,
    /// Номер выдачи: растёт с каждым пересчётом `items` (версия содержимого
    /// для перетекания списка).
    items_version: RwSignal<u64>,
}

fn state(ctx: ShellCtx) -> State {
    let st = State {
        query: use_signal(String::new()),
        selected: use_signal(0usize),
        section: use_signal(if ctx.cfg().launcher.favorites.is_empty() { "all".to_string() } else { "favorites".to_string() }),
        items: use_signal(Vec::new()),
        items_version: use_signal(0u64),
    };
    let (query, section, items, selected, items_version) = (st.query, st.section, st.items, st.selected, st.items_version);
    create_effect(move || {
        let q = query.get();
        let s = section.get();
        let _ = ctx.apps_rev.get();
        let v = if q.trim().is_empty() { section_items(&ctx, &s) } else { search(&ctx, &q) };
        items.set(v);
        items_version.set(items_version.get_untracked() + 1);
        selected.set(0);
    });
    // Стрелки и Enter — до поля ввода.
    crate::popup::set_key_handler(move |k: &KeyInfo| {
        if !k.pressed {
            return false;
        }
        let n = items.get_untracked().len();
        match k.key {
            Key::Down => {
                if n > 0 {
                    selected.set((selected.get_untracked() + 1) % n);
                }
                true
            }
            Key::Up => {
                if n > 0 {
                    selected.set((selected.get_untracked() + n - 1) % n);
                }
                true
            }
            Key::Enter => {
                if let Some(it) = items.get_untracked().get(selected.get_untracked()) {
                    it.activate(ctx);
                }
                true
            }
            _ => false,
        }
    });
    st
}

fn search_field(st: &State, placeholder: &str) -> impl Widget {
    let q = st.query;
    DecoratedBox::new()
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(icon(mi::SEARCH).class("search-icon"))
                .child(TextField::with_text(q.get_untracked()).placeholder(placeholder).autofocus(true).on_change(move |t| q.set(t.to_string())).class("search-field grow")),
        )
        .class("search-box")
}

/// Ключ раздела для перетекания списка: порядок разделов в боковой панели
/// задаёт направление (вниз по списку — новое въезжает снизу).
fn section_key(ctx: &ShellCtx, section: &str) -> u64 {
    let mut i = 2u64;
    for (key, _, _) in sidebar_entries(ctx) {
        if key == section {
            return i;
        }
        i += 1;
    }
    1
}

fn list(st: &State, ctx: ShellCtx) -> impl Widget {
    let (items, selected, section, query, version) = (st.items, st.selected, st.section, st.query, st.items_version);
    ScrollView::new()
        .vertical()
        .child(move || {
            // Снимок выдачи: уходящий список не должен подхватывать новую.
            let v = items.get();
            let ver = version.get();
            let key = if query.get().trim().is_empty() { section_key(&ctx, &section.get()) } else { u64::MAX };
            AnimatedSwitcher::new(key, move || {
                let sel = selected.get();
                let mut col = Column::new().gap(2.0);
                if v.is_empty() {
                    col = col.child(Text::new("Ничего не найдено").class("launcher-empty"));
                }
                for (i, it) in v.iter().cloned().enumerate() {
                    let _ = sel;
                    col = col.child(row(it, i, selected, ctx));
                }
                Box::new(col)
            })
            .version(ver)
            .slide(0.0, 22.0)
            .duration_ms(crate::anim::ms(&ctx, 240))
            .exit_duration_ms(crate::anim::ms(&ctx, 140))
            .animate_size(false)
        })
        .class("launcher-list")
}

/// Разделы боковой панели: ключ, подпись, значок.
fn sidebar_entries(ctx: &ShellCtx) -> Vec<(String, String, String)> {
    let cfg = ctx.cfg();
    let apps = xdg::apps();
    let mut entries: Vec<(String, String, String)> = Vec::new();
    if !cfg.launcher.favorites.is_empty() {
        entries.push(("favorites".into(), "Избранное".into(), mi::STAR.into()));
    }
    if cfg.launcher.show_recent {
        entries.push(("recent".into(), "Недавние".into(), mi::HISTORY.into()));
    }
    entries.push(("all".into(), "Все приложения".into(), mi::APPS.into()));
    if cfg.launcher.show_categories {
        for (key, label, glyph) in xdg::CATEGORIES {
            if apps.iter().any(|e| !e.no_display && e.android.is_none() && e.main_category() == *key) {
                entries.push((key.to_string(), label.to_string(), glyph.to_string()));
            }
        }
    }
    // Android — свои разделы, по экземпляру
    let instances = xdg::android_instances(&apps);
    let many = instances.len() > 1;
    for i in instances {
        let label = if many { format!("Android · {}", i.title) } else { "Android".into() };
        entries.push((format!("android:{}", i.instance), label, mi::ANDROID.into()));
    }
    entries
}

fn sidebar(st: &State, ctx: ShellCtx) -> impl Widget {
    let section = st.section;
    let query = st.query;
    let entries = sidebar_entries(&ctx);
    ScrollView::new()
        .vertical()
        .child(move || {
            let cur = section.get();
            let active_search = !query.get().trim().is_empty();
            let mut col = Column::new().gap(2.0);
            for (key, label, glyph) in entries.clone() {
                let k = key.clone();
                col = col.child(
                    InputArea::new(
                        DecoratedBox::new()
                            .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(&glyph).class("menu-icon")).child(Text::new(label).class("menu-label")))
                            .class(if cur == key && !active_search { "cat-item cat-item-active" } else { "cat-item" }),
                    )
                    .pointer()
                    .on_hover(move |h| {
                        // Как в Kickoff: раздел открывается наведением.
                        if h {
                            section.set(k.clone());
                        }
                    })
                    .on_click(move |_, _, _| {}),
                );
            }
            col
        })
        .class("launcher-sidebar")
}

fn footer(ctx: ShellCtx, settings: RwSignal<bool>) -> impl Widget {
    let user = std::env::var("USER").unwrap_or_default();
    let host = std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).unwrap_or_default();
    let btn = |glyph: &'static str, tip: &'static str, f: fn()| {
        InputArea::new(DecoratedBox::new().child(icon(glyph)).class("footer-btn")).pointer().on_click(move |b, _, _| {
            if b == MouseButton::Left {
                let _ = tip;
                f();
            }
        })
    };
    let _ = ctx;
    Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            Column::new()
                .gap(0.0)
                .child(Text::new(user).class("launcher-user"))
                .child(Text::new(host).class("launcher-sub"))
                .class("grow"),
        )
        .child(crate::menu_prefs::button(settings, "footer-btn", ""))
        .child(btn(mi::SETTINGS, "Параметры", || {
            ShellCtx::get().close_popup();
            crate::actions::spawn("synsettings");
        }))
        .child(btn(mi::LOCK, "Блокировать", || crate::commands::handle("lock")))
        .child(btn(mi::POWER, "Питание", || {
            let c = ShellCtx::get();
            c.popup.set(Some(crate::ctx::Popup {
                kind: crate::ctx::PopupKind::Power,
                anchor: crate::ctx::PopupAnchor { output: None, rect: None, edge: synshell_common::config::Edge::Bottom, attached: false },
            }));
        }))
        .class("launcher-footer")
}

/// Меню у кнопки. Размер — уголком (живьём, запоминается), настройки — кнопкой внизу.
pub fn menu(ctx: ShellCtx) -> impl Widget {
    let st = state(ctx);
    let size = crate::menu_prefs::size_signal(&ctx);
    let settings = use_signal(crate::menu_prefs::take_reopen_settings());
    let top = ctx.popup.get_untracked().is_some_and(|p| p.anchor.edge == synshell_common::config::Edge::Top);
    crate::ui::rx(move || {
        let (w, h) = size.get();
        let inner: Box<dyn Widget> = if settings.get() {
            Box::new(crate::menu_prefs::view(ctx, size, move || settings.set(false)).style("height", StyleValue::px(h - 30.0)))
        } else {
            Box::new(
                Column::new()
                    .gap(10.0)
                    .child(search_field(&st, "Поиск приложений, команд, вычислений…"))
                    .child(
                        Row::new()
                            .gap(8.0)
                            .child(DecoratedBox::new().child(sidebar(&st, ctx)).class("launcher-side-wrap"))
                            .child(list(&st, ctx))
                            .style("height", StyleValue::px((h - 130.0).max(200.0))),
                    )
                    .child(footer(ctx, settings)),
            )
        };
        let gy = if top { h - 52.0 } else { -6.0 };
        Box::new(
            Stack::new()
                .child(Column::new().child(inner).style("width", StyleValue::px(w - 30.0)))
                .child(syngui::containers::Positioned::new(crate::menu_prefs::grip(ctx, size)).at(w - 30.0 - 16.0, gy)),
        )
    })
}

/// Строка «Выполнить»: карточка перетекает по высоте вслед за выдачей.
pub fn run_prompt(ctx: ShellCtx) -> impl Widget {
    let st = state(ctx);
    let items = st.items;
    let results = crate::ui::rx(move || {
        let v = items.get();
        let mut col = Column::new().gap(2.0);
        for (i, it) in v.into_iter().take(6).enumerate() {
            col = col.child(row(it, i, st.selected, ctx));
        }
        Box::new(col)
    });
    Column::new()
        .gap(8.0)
        .child(search_field(&st, "Команда или приложение…"))
        .child(AnimatedSize::new(results).axis(AnimationAxis::Height).spring(420.0, 40.0).class("popup-morph"))
}

/// Сетка на весь экран. Проявляется и уходит по сигналу `open`; после
/// ухода закрывается поверхность `sid`.
pub fn fullscreen(ctx: ShellCtx, open: RwSignal<bool>, sid: std::sync::Arc<std::sync::Mutex<Option<syngui_layer::SurfaceId>>>) -> impl Widget {
    let dur = crate::anim::ms(&ctx, 260);
    Presence::signal(open, move || Box::new(fullscreen_view(ctx)))
        .enter(Motion::fade().scale(1.04))
        .exit(Motion::fade().scale(1.02))
        .duration_ms(dur)
        .initial(dur > 0)
        .on_exit_complete(move || {
            if let Some(id) = *sid.lock().unwrap() {
                syngui_layer::close_surface(id);
            }
        })
}

fn fullscreen_view(ctx: ShellCtx) -> impl Widget {
    let st = state(ctx);
    st.section.set("all".into());
    let cols = ctx.cfg().launcher.columns.max(2) as usize;
    let (items, selected) = (st.items, st.selected);
    let backdrop = InputArea::new(DecoratedBox::new().class("launcher-fs-backdrop")).on_press(|_, _, _| ShellCtx::get().close_popup());
    let grid = ScrollView::new().vertical().child(move || {
        let v = items.get();
        let sel = selected.get();
        let mut col = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for (r, chunk) in v.chunks(cols).enumerate() {
            let mut row = Row::new().gap(12.0);
            for (c, it) in chunk.iter().enumerate() {
                let idx = r * cols + c;
                let (name, _) = item_text(it);
                let it2 = it.clone();
                row = row.child(
                    InputArea::new(
                        DecoratedBox::new()
                            .child(
                                Column::new()
                                    .gap(6.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .child(item_icon(it, "grid-icon"))
                                    .child(Text::new(name).max_lines(2).class("grid-name")),
                            )
                            .class(if idx == sel { "grid-cell grid-cell-selected" } else { "grid-cell" }),
                    )
                    .pointer()
                    .on_click(move |b, _, _| {
                        if b == MouseButton::Left {
                            it2.activate(ShellCtx::get());
                        }
                    }),
                );
            }
            col = col.child(row);
        }
        col
    });
    let content = Column::new()
        .gap(24.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(search_field(&st, "Поиск")).class("launcher-fs-search"))
        .child(InputArea::new(grid.class("launcher-fs-grid")).absorb())
        .class("launcher-fs");
    Stack::new().fit(StackFit::Expand).child(backdrop).child(content)
}

#[cfg(test)]
mod tests {
    use super::calc;

    #[test]
    fn calculator() {
        assert_eq!(calc("2+2*3").as_deref(), Some("8"));
        assert_eq!(calc("(1.5+2)^2").as_deref(), Some("12.25"));
        assert_eq!(calc("10 % 3").as_deref(), Some("1"));
        assert_eq!(calc("1/3").as_deref(), Some("0.3333333333"));
        assert_eq!(calc("firefox"), None);
        assert_eq!(calc("2+"), None);
    }
}
