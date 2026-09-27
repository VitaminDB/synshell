//! Значки запуска (`app`), разделы (`group`) и папки (`folder`) — общие
//! для обычной панели и дока: какому приложению принадлежат окна, запуск
//! и активация, содержимое каталогов, всплывающие «стеки» раздела/папки
//! (сетка, список, веер как в macOS).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use syndesktop_common::config::Applet;
use syndesktop_common::ipc::{WindowInfo, WindowOp};
use syngui::animation::{Animation, Easing};
use syngui::input::MouseButton;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::Animated;

use crate::ctx::{PopupKind, ShellCtx};
use crate::panel::PanelCtx;
use crate::ui::{icon, mi, InputArea};
use crate::xdg;

// ─── Что запускает значок ────────────────────────────────────────────────────

/// Значок запуска: приложение из .desktop или своя команда.
#[derive(Clone, Debug, PartialEq)]
pub struct Launchable {
    /// id .desktop (для сопоставления окон) — пусто у своей команды.
    pub app_id: String,
    pub name: String,
    pub icon: Option<PathBuf>,
    /// Глиф Material Icons вместо картинки.
    pub glyph: Option<String>,
    pub command: String,
    pub terminal: bool,
}

impl Launchable {
    /// Ключ для анимаций и всплесков частиц.
    pub fn key(&self) -> String {
        if self.app_id.is_empty() {
            format!("cmd:{}", self.command)
        } else {
            self.app_id.clone()
        }
    }

    pub fn from_entry(e: &xdg::DesktopEntry) -> Self {
        Self {
            app_id: e.id.clone(),
            name: e.name.clone(),
            icon: xdg::lookup_icon(&e.icon),
            glyph: None,
            command: e.command(),
            terminal: e.terminal,
        }
    }

    /// Из апплета `app`: `app` (id .desktop), `command`, `icon`, `name`.
    pub fn from_applet(a: &Applet) -> Self {
        let id = a.str("app").unwrap_or("").trim().to_string();
        let mut l = match xdg::app_by_id(&id) {
            Some(e) => Self::from_entry(&e),
            None => Self {
                app_id: id.clone(),
                name: if id.is_empty() { a.str_or("command", "?").to_string() } else { id.clone() },
                icon: xdg::lookup_icon(&id).or_else(|| xdg::lookup_icon("application-x-executable")),
                glyph: None,
                command: id.clone(),
                terminal: false,
            },
        };
        if let Some(c) = a.str("command").filter(|c| !c.trim().is_empty()) {
            l.command = c.to_string();
            if id.is_empty() {
                l.app_id.clear();
            }
        }
        if let Some(n) = a.str("name").filter(|n| !n.trim().is_empty()) {
            l.name = n.to_string();
        }
        if let Some(i) = a.str("icon").filter(|i| !i.trim().is_empty()) {
            if i.chars().count() == 1 {
                l.glyph = Some(i.to_string());
            } else if let Some(p) = xdg::lookup_icon(i) {
                l.icon = Some(p);
            }
        }
        l
    }
}

/// Картинка значка (или глиф) с классом.
pub fn icon_widget(icon_path: &Option<PathBuf>, glyph: &Option<String>, class: &str, size: f32) -> Box<dyn Widget> {
    if let Some(g) = glyph {
        return Box::new(icon(g).class(format!("{class} {class}-glyph")).style("icon-size", StyleValue::px(size * 0.8)));
    }
    match icon_path {
        Some(p) => Box::new(
            Image::new(p.to_string_lossy())
                .fit(ImageFit::Contain)
                .placeholder(false)
                .class(class.to_string())
                .style("width", StyleValue::px(size))
                .style("height", StyleValue::px(size)),
        ),
        None => Box::new(icon(mi::WINDOW).class(format!("{class} {class}-glyph")).style("icon-size", StyleValue::px(size * 0.8))),
    }
}

// ─── Окна приложений ────────────────────────────────────────────────────────

thread_local! {
    /// app_id окна → id .desktop (поиск по всем приложениям небыстрый).
    static DESKTOP_OF: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// id .desktop приложения окна (или сам `app_id`, если .desktop не нашёлся).
pub fn desktop_id_of(app_id: &str) -> String {
    if let Some(v) = DESKTOP_OF.with(|m| m.borrow().get(app_id).cloned()) {
        return v;
    }
    let v = xdg::app_for_window(app_id).map(|e| e.id).unwrap_or_else(|| app_id.to_string());
    DESKTOP_OF.with(|m| m.borrow_mut().insert(app_id.to_string(), v.clone()));
    v
}

/// Окно принадлежит приложению `app_id` (id .desktop).
pub fn window_is_app(w: &WindowInfo, app_id: &str) -> bool {
    if app_id.is_empty() || w.app_id.is_empty() {
        return false;
    }
    w.app_id == app_id || w.app_id.eq_ignore_ascii_case(app_id) || desktop_id_of(&w.app_id) == app_id
}

/// Окна приложения, видимые на панели (без skip_taskbar).
pub fn windows_of<'a>(app_id: &str, windows: &'a [WindowInfo]) -> Vec<&'a WindowInfo> {
    windows.iter().filter(|w| !w.skip_taskbar && window_is_app(w, app_id)).collect()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Запустить (новый экземпляр): команда, «прыжок» значка до появления
/// окна, всплеск частиц.
pub fn launch(ctx: ShellCtx, l: &Launchable) {
    if !l.app_id.is_empty() {
        if let Some(e) = xdg::app_by_id(&l.app_id) {
            if e.command() == l.command {
                crate::launcher::launch(ctx, &e);
            } else {
                spawn_command(ctx, l);
            }
        } else {
            spawn_command(ctx, l);
        }
    } else {
        spawn_command(ctx, l);
    }
    let key = l.key();
    ctx.burst(&key);
    let mut list = ctx.launching.get_untracked();
    list.retain(|(k, _)| *k != key);
    list.push((key, now_ms()));
    ctx.launching.set(list);
    watch_launching();
}

fn spawn_command(ctx: ShellCtx, l: &Launchable) {
    if l.terminal {
        crate::actions::spawn(&format!("{} -e {}", ctx.cfg().general.terminal, l.command));
    } else {
        crate::actions::spawn(&l.command);
    }
}

thread_local! {
    static LAUNCH_TIMER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Снимать «запускается», когда у приложения появилось окно (или прошло 8 с).
fn watch_launching() {
    if LAUNCH_TIMER.with(|t| t.replace(true)) {
        return;
    }
    syngui_layer::add_timer(Duration::from_millis(400), || {
        let ctx = ShellCtx::get();
        let list = ctx.launching.get_untracked();
        let windows = ctx.windows.get_untracked();
        let now = now_ms();
        let kept: Vec<(String, u64)> = list
            .iter()
            .filter(|(k, t)| now.saturating_sub(*t) < 8000 && !windows.iter().any(|w| window_is_app(w, k)))
            .cloned()
            .collect();
        if kept.len() != list.len() {
            ctx.launching.set(kept.clone());
        }
        if kept.is_empty() {
            LAUNCH_TIMER.with(|t| t.set(false));
            None
        } else {
            Some(Duration::from_millis(400))
        }
    });
}

pub fn is_launching(ctx: &ShellCtx, key: &str) -> bool {
    ctx.launching.get().iter().any(|(k, _)| k == key)
}

/// Клик по значку приложения: нет окон — запустить; окно в фокусе —
/// свернуть (одно) или перейти к следующему; иначе — поднять окно.
pub fn activate_or_launch(ctx: ShellCtx, l: &Launchable) {
    let windows = ctx.windows.get_untracked();
    let mine = windows_of(&l.app_id, &windows);
    if mine.is_empty() {
        launch(ctx, l);
        return;
    }
    if let Some(pos) = mine.iter().position(|w| w.focused) {
        if mine.len() == 1 {
            crate::actions::window_op(mine[0].id, WindowOp::ToggleMinimize);
        } else {
            let next = mine[(pos + 1) % mine.len()];
            crate::actions::window_op(next.id, WindowOp::Activate);
        }
        return;
    }
    // Не свёрнутое окно текущего стола, иначе любое.
    let ws = ctx.workspaces.get_untracked().iter().find(|w| w.active).map(|w| w.index);
    let pick = mine
        .iter()
        .find(|w| !w.minimized && (Some(w.workspace) == ws || w.sticky))
        .or_else(|| mine.first())
        .map(|w| w.id);
    if let Some(id) = pick {
        crate::actions::window_op(id, WindowOp::Activate);
    }
}

/// Колесо над значком: перебор окон приложения.
pub fn cycle_windows(ctx: ShellCtx, app_id: &str, dy: f32) {
    let windows = ctx.windows.get_untracked();
    let mine = windows_of(app_id, &windows);
    if mine.is_empty() {
        return;
    }
    let cur = mine.iter().position(|w| w.focused).map(|p| p as i64).unwrap_or(-1);
    let n = mine.len() as i64;
    let next = ((cur + if dy > 0.0 { -1 } else { 1 }) % n + n) % n;
    crate::actions::window_op(mine[next as usize].id, WindowOp::Activate);
}

// ─── Каталоги ───────────────────────────────────────────────────────────────

/// Каталог пользователя по XDG (`DOWNLOAD`, `DOCUMENTS`, `DESKTOP`…).
pub fn xdg_user_dir(kind: &str) -> Option<PathBuf> {
    let home = syndesktop_common::paths::expand_tilde("~");
    let conf = home.join(".config/user-dirs.dirs");
    let key = format!("XDG_{kind}_DIR=");
    let text = std::fs::read_to_string(conf).ok()?;
    for line in text.lines() {
        if let Some(v) = line.trim().strip_prefix(&key) {
            let v = v.trim_matches('"').replace("$HOME", &home.to_string_lossy());
            return Some(PathBuf::from(v));
        }
    }
    None
}

pub fn trash_dir() -> PathBuf {
    syndesktop_common::paths::data_home().join("Trash/files")
}

/// Путь папки из конфига: `~`, `xdg:DOWNLOAD`, `trash:`.
pub fn resolve_path(p: &str) -> PathBuf {
    let p = p.trim();
    if p == "trash:" || p == "trash:/" {
        return trash_dir();
    }
    if let Some(kind) = p.strip_prefix("xdg:") {
        if let Some(d) = xdg_user_dir(&kind.to_ascii_uppercase()) {
            return d;
        }
        let fallback = match kind.to_ascii_uppercase().as_str() {
            "DOWNLOAD" => "~/Downloads",
            "DOCUMENTS" => "~/Documents",
            "PICTURES" => "~/Pictures",
            "MUSIC" => "~/Music",
            "VIDEOS" => "~/Videos",
            "DESKTOP" => "~/Desktop",
            _ => "~",
        };
        return syndesktop_common::paths::expand_tilde(fallback);
    }
    syndesktop_common::paths::expand_tilde(p)
}

pub fn is_trash(p: &str) -> bool {
    p.trim().starts_with("trash:")
}

/// Значок каталога: особые папки — своими значками.
pub fn folder_icon_name(path: &Path, spec: &str) -> &'static str {
    if is_trash(spec) {
        return if std::fs::read_dir(trash_dir()).map(|mut d| d.next().is_some()).unwrap_or(false) {
            "user-trash-full"
        } else {
            "user-trash"
        };
    }
    let home = syndesktop_common::paths::expand_tilde("~");
    if path == home {
        return "user-home";
    }
    for (kind, name) in [
        ("DOWNLOAD", "folder-download"),
        ("DOCUMENTS", "folder-documents"),
        ("PICTURES", "folder-pictures"),
        ("MUSIC", "folder-music"),
        ("VIDEOS", "folder-videos"),
        ("DESKTOP", "user-desktop"),
    ] {
        if xdg_user_dir(kind).is_some_and(|d| d == path) {
            return name;
        }
    }
    "folder"
}

/// Значок файла по расширению.
fn file_icon_name(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "avif" | "heic" => "image-x-generic",
        "mp4" | "mkv" | "webm" | "avi" | "mov" => "video-x-generic",
        "mp3" | "flac" | "ogg" | "opus" | "wav" | "m4a" => "audio-x-generic",
        "pdf" => "application-pdf",
        "zip" | "tar" | "gz" | "xz" | "zst" | "7z" | "rar" | "bz2" => "package-x-generic",
        "sh" | "py" | "rs" | "js" | "ts" | "c" | "cpp" | "h" | "go" | "rb" | "lua" => "text-x-script",
        "doc" | "docx" | "odt" | "rtf" => "x-office-document",
        "xls" | "xlsx" | "ods" | "csv" => "x-office-spreadsheet",
        "ppt" | "pptx" | "odp" => "x-office-presentation",
        "html" | "htm" => "text-html",
        "iso" | "img" => "application-x-cd-image",
        "deb" | "rpm" | "pkg" | "appimage" => "application-x-executable",
        _ => "text-x-generic",
    }
}

/// Элемент раздела или папки.
#[derive(Clone, Debug, PartialEq)]
pub enum StackEntry {
    App(Launchable),
    File { path: PathBuf, name: String, is_dir: bool, icon: Option<PathBuf> },
}

impl StackEntry {
    pub fn name(&self) -> &str {
        match self {
            StackEntry::App(l) => &l.name,
            StackEntry::File { name, .. } => name,
        }
    }

    /// Строка из `items` раздела: путь (`/…`, `~/…`, `xdg:…`, `trash:`) или
    /// id приложения.
    pub fn parse(item: &str) -> Option<StackEntry> {
        let item = item.trim();
        if item.is_empty() {
            return None;
        }
        if item.starts_with('/') || item.starts_with('~') || item.starts_with("xdg:") || is_trash(item) {
            let path = resolve_path(item);
            return Some(file_entry(&path, Some(item)));
        }
        let mut a = Applet::new("app");
        a.options.insert("app".into(), toml::Value::String(item.to_string()));
        Some(StackEntry::App(Launchable::from_applet(&a)))
    }
}

fn file_entry(path: &Path, spec: Option<&str>) -> StackEntry {
    let is_dir = path.is_dir();
    let mut name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string_lossy().into_owned());
    let icon = if is_dir {
        xdg::lookup_icon(folder_icon_name(path, spec.unwrap_or(""))).or_else(|| xdg::lookup_icon("folder"))
    } else if path.extension().is_some_and(|e| e == "desktop") {
        match xdg::parse_desktop_file(path, String::new(), &[]) {
            Some(e) => {
                name = e.name.clone();
                xdg::lookup_icon(&e.icon)
            }
            None => xdg::lookup_icon("application-x-desktop"),
        }
    } else {
        xdg::lookup_icon(file_icon_name(path)).or_else(|| xdg::lookup_icon("text-x-generic"))
    };
    StackEntry::File { path: path.to_path_buf(), name, is_dir, icon }
}

/// Содержимое каталога: сначала папки, скрытые — нет, не больше 300.
pub fn list_dir(dir: &Path) -> Vec<StackEntry> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut items: Vec<(bool, String, PathBuf)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .map(|p| (!p.is_dir(), p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default(), p))
        .collect();
    items.sort();
    items.into_iter().take(300).map(|(_, _, p)| file_entry(&p, None)).collect()
}

/// Открыть файл или каталог программой по умолчанию.
pub fn open_path(path: &Path) {
    let q = path.to_string_lossy().replace('\'', "'\\''");
    crate::actions::spawn(&format!("xdg-open '{q}'"));
}

/// Очистить корзину (файлы и сведения об удалении).
pub fn empty_trash() {
    let base = syndesktop_common::paths::data_home().join("Trash");
    for sub in ["files", "info"] {
        if let Ok(rd) = std::fs::read_dir(base.join(sub)) {
            for e in rd.flatten() {
                let p = e.path();
                let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
            }
        }
    }
}

// ─── Значки раздела ─────────────────────────────────────────────────────────

/// Значок раздела без своего `icon`: сетка 2×2 из первых значков, как
/// папка на iOS.
pub fn group_preview(items: &[StackEntry], size: f32) -> Box<dyn Widget> {
    let cell = (size * 0.42).round();
    let mut grid = Column::new().gap(size * 0.06).cross_axis_alignment(CrossAxisAlignment::Center);
    let mut it = items.iter();
    for _ in 0..2 {
        let mut row = Row::new().gap(size * 0.06).cross_axis_alignment(CrossAxisAlignment::Center);
        for _ in 0..2 {
            row = row.child(match it.next() {
                Some(StackEntry::App(l)) => icon_widget(&l.icon, &l.glyph, "group-preview-icon", cell),
                Some(StackEntry::File { icon: i, .. }) => icon_widget(i, &None, "group-preview-icon", cell),
                None => Box::new(DecoratedBox::new().class("group-preview-empty").style("width", StyleValue::px(cell)).style("height", StyleValue::px(cell))),
            });
        }
        grid = grid.child(row);
    }
    Box::new(
        DecoratedBox::new()
            .child(crate::ui::vcenter(grid))
            .class("group-preview")
            .style("width", StyleValue::px(size))
            .style("height", StyleValue::px(size)),
    )
}

/// Содержимое раздела `group` из апплета.
pub fn group_entries(a: &Applet) -> Vec<StackEntry> {
    a.strings("items").iter().filter_map(|s| StackEntry::parse(s)).collect()
}

/// Значок апплета `group`/`folder` (картинка, глиф или превью).
pub fn stack_icon(a: &Applet, size: f32, class: &str) -> Box<dyn Widget> {
    if let Some(i) = a.str("icon").filter(|i| !i.trim().is_empty()) {
        if i.chars().count() == 1 {
            return icon_widget(&None, &Some(i.to_string()), class, size);
        }
        if let Some(p) = xdg::lookup_icon(i) {
            return icon_widget(&Some(p), &None, class, size);
        }
    }
    if a.kind == "folder" {
        let spec = a.str_or("path", "~");
        let path = resolve_path(spec);
        return icon_widget(&xdg::lookup_icon(folder_icon_name(&path, spec)).or_else(|| xdg::lookup_icon("folder")), &None, class, size);
    }
    group_preview(&group_entries(a), size)
}

/// Подпись апплета `group`/`folder`.
pub fn stack_title(a: &Applet) -> String {
    if let Some(n) = a.str("name").filter(|n| !n.trim().is_empty()) {
        return n.to_string();
    }
    if a.kind == "folder" {
        let spec = a.str_or("path", "~");
        if is_trash(spec) {
            return "Корзина".into();
        }
        let p = resolve_path(spec);
        return p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| spec.to_string());
    }
    "Раздел".into()
}

// ─── Апплеты обычной панели ─────────────────────────────────────────────────

/// Значок запуска на панели: картинка (+ подпись), подчёркивание, если
/// у приложения есть окна.
pub fn app_applet(a: &Applet, pc: &PanelCtx, index: usize) -> Box<dyn Widget> {
    let ctx = ShellCtx::get();
    let l = Launchable::from_applet(a);
    let show_label = a.bool_or("label", false);
    let pc = pc.clone();
    let panel = pc.index;
    Box::new(crate::ui::rx(move || {
        let windows = ctx.windows.get();
        let mine = windows_of(&l.app_id, &windows);
        let launching = is_launching(&ctx, &l.key());
        let mut cls = String::from("applet applet-app");
        if !mine.is_empty() {
            cls.push_str(" applet-app-running");
        }
        if mine.iter().any(|w| w.focused) {
            cls.push_str(" applet-app-active");
        }
        if launching {
            cls.push_str(" applet-app-launching");
        }
        let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon_widget(&l.icon, &l.glyph, "applet-image", 22.0));
        if show_label {
            row = row.child(Text::new(l.name.clone()).max_lines(1).class("applet-label"));
        }
        let (l1, l2, l3) = (l.clone(), l.clone(), l.app_id.clone());
        let pc = pc.clone();
        Box::new(
            InputArea::new(DecoratedBox::new().child(crate::ui::vcenter(row)).class(cls))
                .pointer()
                .on_click(move |b, _, r| {
                    let ctx = ShellCtx::get();
                    match b {
                        MouseButton::Left => activate_or_launch(ctx, &l1),
                        MouseButton::Middle => launch(ctx, &l2),
                        MouseButton::Right => ctx.open_popup(PopupKind::ItemMenu { panel, index }, pc.anchor(r)),
                        _ => {}
                    }
                })
                .on_wheel(move |dy| cycle_windows(ShellCtx::get(), &l3, dy)),
        )
    }))
}

/// Раздел или папка на панели: значок, по клику (или наведению) —
/// всплывающий стек.
pub fn stack_applet(a: &Applet, pc: &PanelCtx, index: usize) -> Box<dyn Widget> {
    let hover_open = a.str_or("open", "click") == "hover";
    let label = a.bool_or("label", false).then(|| stack_title(a));
    let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(stack_icon(a, 22.0, "applet-image"));
    if let Some(l) = label {
        row = row.child(Text::new(l).max_lines(1).class("applet-label"));
    }
    let pc2 = pc.clone();
    let pc3 = pc.clone();
    let panel = pc.index;
    let hover_timer: std::sync::Arc<std::sync::Mutex<Option<u64>>> = Default::default();
    Box::new(
        InputArea::new(DecoratedBox::new().child(crate::ui::vcenter(row)).class(format!("applet applet-{}", a.kind)))
            .pointer()
            .on_click(move |b, _, r| {
                let ctx = ShellCtx::get();
                match b {
                    MouseButton::Left => ctx.open_popup(PopupKind::Stack { panel, index, hover: false }, pc2.anchor(r)),
                    MouseButton::Right => ctx.open_popup(PopupKind::ItemMenu { panel, index }, pc2.anchor(r)),
                    _ => {}
                }
            })
            .on_hover(move |inside| {
                if !hover_open {
                    return;
                }
                hover_open_stack(&hover_timer, inside, panel, index, &pc3, None);
            }),
    )
}

/// Открытие стека наведением с задержкой (случайный пролёт курсора не
/// открывает). `rect` — якорь (иначе — границы апплета панели).
pub fn hover_open_stack(
    timer: &std::sync::Arc<std::sync::Mutex<Option<u64>>>,
    inside: bool,
    panel: usize,
    index: usize,
    pc: &PanelCtx,
    rect: Option<Rect>,
) {
    if let Some(t) = timer.lock().unwrap().take() {
        syngui_layer::cancel_timer(t);
    }
    if !inside {
        return;
    }
    let ctx = ShellCtx::get();
    if ctx.editing.get_untracked().is_some() {
        return;
    }
    let kind = PopupKind::Stack { panel, index, hover: true };
    if ctx.popup.get_untracked().is_some_and(|p| p.kind == kind) {
        return;
    }
    let pc = pc.clone();
    let anchor_rect = rect.or_else(|| pc.bounds_of(index));
    let t = syngui_layer::add_timer(Duration::from_millis(280), move || {
        let ctx = ShellCtx::get();
        let r = anchor_rect.unwrap_or(Rect::zero());
        ctx.popup.set(Some(crate::ctx::Popup { kind: kind.clone(), anchor: pc.anchor(r) }));
        None
    });
    *timer.lock().unwrap() = Some(t);
}

// ─── Всплывающий стек ───────────────────────────────────────────────────────

/// Вид стека: `grid`, `list`, `fan` (веер над доком снизу).
pub fn stack_view_kind(a: &Applet, edge: syndesktop_common::config::Edge) -> &'static str {
    match a.str_or("view", "grid") {
        "fan" if edge == syndesktop_common::config::Edge::Bottom => "fan",
        "list" | "fan" => "list",
        _ => "grid",
    }
}

/// Ширина карточки стека.
pub fn stack_width(ctx: &ShellCtx, panel: usize, index: usize) -> f32 {
    let cfg = ctx.cfg();
    let Some(p) = cfg.panels.get(panel) else { return 320.0 };
    let Some(a) = p.applets.get(index) else { return 320.0 };
    let n = if a.kind == "folder" { list_dir(&resolve_path(a.str_or("path", "~"))).len() } else { group_entries(a).len() };
    match stack_view_kind(a, p.edge) {
        "fan" => FAN_SIDE * 2.0 + 60.0,
        "list" => 320.0,
        _ => grid_cols(n) * 94.0 + 30.0,
    }
}

/// Ширина половины веера (подписи слева от значков).
const FAN_SIDE: f32 = 200.0;

/// Колонок в сетке стека.
fn grid_cols(n: usize) -> f32 {
    (n as f32).sqrt().ceil().clamp(3.0, 6.0)
}

/// Содержимое стека раздела/папки.
pub fn stack_view(ctx: ShellCtx, panel: usize, index: usize) -> Box<dyn Widget> {
    let cfg = ctx.cfg();
    let Some(p) = cfg.panels.get(panel) else { return Box::new(Text::new("Нет панели")) };
    let Some(a) = p.applets.get(index).cloned() else { return Box::new(Text::new("Нет значка")) };
    let view = stack_view_kind(&a, p.edge);
    let title = stack_title(&a);
    if a.kind == "folder" {
        let spec = a.str_or("path", "~").to_string();
        let root = resolve_path(&spec);
        let cwd = use_signal(root.clone());
        let trash = is_trash(&spec);
        return Box::new(
            Column::new()
                .gap(8.0)
                .class(format!("stack stack-{view}"))
                .child(move || {
                    let dir = cwd.get();
                    let mut head = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).class("stack-head");
                    if dir != root {
                        let parent = dir.parent().map(Path::to_path_buf);
                        head = head.child(
                            InputArea::new(crate::ui::boxed("stack-back", icon("\u{E5C4}")))
                                .pointer()
                                .on_click(move |_, _, _| {
                                    if let Some(p) = &parent {
                                        cwd.set(p.clone());
                                    }
                                }),
                        );
                    }
                    let name = if dir == root { title.clone() } else { dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() };
                    head.child(Text::new(name).max_lines(1).class("popup-title grow"))
                })
                .child(crate::ui::rx(move || {
                    let dir = cwd.get();
                    let entries = list_dir(&dir);
                    if entries.is_empty() {
                        return Box::new(Text::new(if trash { "Корзина пуста" } else { "Папка пуста" }).class("stack-empty")) as Box<dyn Widget>;
                    }
                    entries_view(entries, view, Some(cwd))
                }))
                .child({
                    let root2 = resolve_path(&spec);
                    let mut foot = Row::new().gap(8.0).class("stack-foot").child(crate::popup::menu_item("\u{E89E}", "Открыть в файловом менеджере", move || {
                        let d = cwd.get_untracked();
                        open_path(if d.exists() { &d } else { &root2 });
                    }));
                    if trash {
                        foot = foot.child(crate::popup::menu_item("\u{E872}", "Очистить", empty_trash));
                    }
                    foot
                }),
        );
    }
    let entries = group_entries(&a);
    let mut col = Column::new().gap(8.0).class(format!("stack stack-{view}"));
    if view != "fan" {
        col = col.child(Text::new(title).max_lines(1).class("popup-title stack-title"));
    }
    if entries.is_empty() {
        return Box::new(col.child(Text::new("Раздел пуст — добавьте приложения в режиме редактирования").class("stack-empty")));
    }
    Box::new(col.child(entries_view(entries, view, None)))
}

/// Появление элемента стека: с задержкой по номеру, «выпрыгивая».
fn pop_in(child: impl Widget + 'static, i: usize, from_y: f32) -> Animated {
    let delay = (i as u32 * 22).min(400);
    Animated::new(child)
        .opacity(Animation::tween(Easing::EaseOutCubic).from(0.0).to(1.0).duration_ms(180).delay_ms(delay).build())
        .scale(Animation::tween(Easing::EaseOutBack).from(0.6).to(1.0).duration_ms(260).delay_ms(delay).build())
        .translate_y(Animation::tween(Easing::EaseOutCubic).from(from_y).to(0.0).duration_ms(260).delay_ms(delay).build())
}

fn activate_entry(e: &StackEntry, cwd: Option<RwSignal<PathBuf>>) {
    let ctx = ShellCtx::get();
    match e {
        StackEntry::App(l) => {
            activate_or_launch(ctx, l);
            ctx.close_popup();
        }
        StackEntry::File { path, is_dir: true, .. } if cwd.is_some() => {
            if let Some(c) = cwd {
                c.set(path.clone());
            }
        }
        StackEntry::File { path, .. } => {
            open_path(path);
            ctx.close_popup();
        }
    }
}

fn entry_icon(e: &StackEntry, size: f32, class: &str) -> Box<dyn Widget> {
    match e {
        StackEntry::App(l) => icon_widget(&l.icon, &l.glyph, class, size),
        StackEntry::File { icon: i, .. } => icon_widget(i, &None, class, size),
    }
}

/// Высота прокрутки стека: по содержимому, но не выше `MAX_STACK_H`.
const MAX_STACK_H: f32 = 430.0;

fn entries_view(entries: Vec<StackEntry>, view: &str, cwd: Option<RwSignal<PathBuf>>) -> Box<dyn Widget> {
    let n = entries.len();
    match view {
        "list" => {
            let mut col = Column::new().gap(2.0).class("stack-list");
            for (i, e) in entries.into_iter().enumerate() {
                let row = Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(entry_icon(&e, 28.0, "stack-list-icon"))
                    .child(Text::new(e.name().to_string()).max_lines(1).class("stack-list-label grow"));
                let e2 = e.clone();
                col = col.child(pop_in(
                    InputArea::new(DecoratedBox::new().child(row).class("stack-list-item")).pointer().on_click(move |b, _, _| {
                        if b == MouseButton::Left {
                            activate_entry(&e2, cwd);
                        }
                    }),
                    i,
                    6.0,
                ));
            }
            let h = (n as f32 * 42.0 + 4.0).min(MAX_STACK_H);
            Box::new(DecoratedBox::new().child(ScrollView::new().vertical().child(col).class("stack-scroll")).style("height", StyleValue::px(h)))
        }
        "fan" => {
            // Веер: снизу вверх, дугой вправо, с лёгким поворотом — как
            // «Стопки» в Dock macOS. Первый элемент — ближе к доку; значки
            // — ровно над значком раздела, подписи слева.
            let n = entries.len().min(14);
            let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).class("stack-fan");
            for (i, e) in entries.into_iter().take(n).enumerate().collect::<Vec<_>>().into_iter().rev() {
                let t = i as f32 / (n.max(2) - 1) as f32;
                let dx = t * t * 64.0;
                let rot = -t * 8.0;
                let row = Row::new()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(
                        Row::new()
                            .main_axis_alignment(MainAxisAlignment::End)
                            .child(DecoratedBox::new().child(Text::new(e.name().to_string()).max_lines(1).class("stack-fan-label")).class("stack-fan-label-box"))
                            .style("width", StyleValue::px(FAN_SIDE)),
                    )
                    .child(DecoratedBox::new().child(entry_icon(&e, 44.0, "stack-fan-icon")).class("stack-fan-icon-box"))
                    .child(DecoratedBox::new().style("width", StyleValue::px(FAN_SIDE)));
                let e2 = e.clone();
                let item = InputArea::new(
                    DecoratedBox::new()
                        .child(row)
                        .class("stack-fan-item")
                        .style("translate-x", StyleValue::px(dx))
                        .style("rotate", StyleValue::Number(rot))
                        .style("transform-origin", StyleValue::String("center".into())),
                )
                .pointer()
                .on_click(move |b, _, _| {
                    if b == MouseButton::Left {
                        activate_entry(&e2, cwd);
                    }
                });
                col = col.child(pop_in(item, i, 40.0 + i as f32 * 6.0));
            }
            Box::new(col)
        }
        _ => {
            let mut flex = Flex::new().wrap().gap(4.0).class("stack-grid");
            for (i, e) in entries.into_iter().enumerate() {
                let cell = Column::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(entry_icon(&e, 48.0, "stack-grid-icon"))
                    .child(Text::new(e.name().to_string()).max_lines(2).class("stack-grid-label"));
                let e2 = e.clone();
                flex = flex.child(pop_in(
                    InputArea::new(DecoratedBox::new().child(cell).class("stack-grid-item")).pointer().on_click(move |b, _, _| {
                        if b == MouseButton::Left {
                            activate_entry(&e2, cwd);
                        }
                    }),
                    i,
                    10.0,
                ));
            }
            let rows = (n as f32 / grid_cols(n)).ceil();
            let h = (rows * 108.0 + 4.0).min(MAX_STACK_H);
            Box::new(DecoratedBox::new().child(ScrollView::new().vertical().child(flex).class("stack-scroll")).style("height", StyleValue::px(h)))
        }
    }
}
