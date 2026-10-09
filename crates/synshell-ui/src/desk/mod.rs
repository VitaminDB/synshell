//! Виджеты и значки рабочего стола (компьютер) и домашнего экрана
//! (телефон): `[[widget]]` в config.toml — что это (`type`), клетки сетки
//! (`x`, `y`, `w`, `h`), стол (`page`) и вид (`view`: карточка, кольцо,
//! шкала, график, столбики, круговая, радар, текст — графики syngui).
//!
//! Раскладку меняют прямо на столе: удержание (правый щелчок) по виджету —
//! его меню, «Изменить рабочий стол» — режим правки: виджеты перетаскиваются
//! по клеткам, уголок меняет размер, касание открывает настройки, «×»
//! убирает; внизу «Добавить виджет», «Добавить значок» и «Готово».
//!
//! Пока `[[widget]]` в конфиге нет, действует встроенная раскладка
//! ([`defaults`]): на телефоне — «Сводка» на первом столе (`[mobile]
//! resources_page`) и значки приложений на остальных (`[wallpaper]
//! desktop_icons`); на компьютере — пусто. Первая правка записывает её в
//! конфиг. На компьютере файлы `~/Desktop` (`desktop_icons`) встают
//! значками в свободные клетки сами, как в KDE; перетащенный значок
//! запоминает место.

pub mod data;
pub mod forms;
mod views;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use synshell_common::config::{DeskWidget, Edge};
use syngui::core::{Color, Point};
use syngui::prelude::*;
use syngui::containers::Positioned;
use syngui::GestureDetector;

use crate::ctx::{PopupKind, ShellCtx};
use crate::ui::{icon, mi, rx};

/// Зазор вокруг виджета в клетке, px.
const GAP: f32 = 5.0;

thread_local! {
    static EDITING: Cell<Option<RwSignal<bool>>> = const { Cell::new(None) };
    /// Рамка-подсказка при перетаскивании: x, y, w, h (px страницы), место свободно.
    static GHOST: Cell<Option<([f32; 4], bool)>> = const { Cell::new(None) };
    static DESKTOP_REV: Cell<Option<RwSignal<u64>>> = const { Cell::new(None) };
    static DESKTOP_FILES: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

/// Режим правки рабочего стола.
pub fn editing() -> RwSignal<bool> {
    EDITING.with(|e| match e.get() {
        Some(s) => s,
        None => {
            let s = use_signal(false);
            e.set(Some(s));
            s
        }
    })
}

pub fn start_editing() {
    ShellCtx::get().close_popup();
    editing().set(true);
}

pub fn stop_editing() {
    let ctx = ShellCtx::get();
    if ctx.popup.get_untracked().is_some_and(|p| matches!(p.kind, PopupKind::DeskAdd(_) | PopupKind::DeskEdit(_))) {
        ctx.close_popup();
    }
    editing().set(false);
}

// ─── Каталог ────────────────────────────────────────────────────────────────

/// Вид виджета: значение `view`, подпись.
pub type View = (&'static str, &'static str);

/// Тип виджета: `type`, подпись, значок, виды (первый — по умолчанию),
/// размер по умолчанию на телефоне и на компьютере (клеток).
pub struct Kind {
    pub kind: &'static str,
    pub label: &'static str,
    pub glyph: &'static str,
    pub views: &'static [View],
    pub phone: (u32, u32),
    pub desktop: (u32, u32),
    /// Нужны данные сборщика ([`data`]).
    pub data: bool,
}

const METRIC_VIEWS: &[View] = &[("card", n_!("Карточка")), ("ring", n_!("Кольцо")), ("gauge", n_!("Шкала")), ("line", n_!("График")), ("text", n_!("Число"))];

pub const KINDS: &[Kind] = &[
    Kind { kind: "clock", label: n_!("Часы"), glyph: "\u{E8B5}", views: &[("digital", n_!("Цифры")), ("big", n_!("Крупно")), ("analog", n_!("Стрелки"))], phone: (4, 1), desktop: (3, 1), data: false },
    Kind {
        kind: "cpu",
        label: n_!("Процессор"),
        glyph: mi::CPU,
        views: &[("card", n_!("Карточка")), ("ring", n_!("Кольцо")), ("gauge", n_!("Шкала")), ("line", n_!("График")), ("bars", n_!("Ядра")), ("radar", n_!("Радар")), ("text", n_!("Число"))],
        phone: (4, 3),
        desktop: (4, 3),
        data: true,
    },
    Kind {
        kind: "memory",
        label: n_!("Память"),
        glyph: mi::MEMORY,
        views: &[("card", n_!("Карточка")), ("ring", n_!("Кольцо")), ("gauge", n_!("Шкала")), ("line", n_!("График")), ("pie", n_!("Круговая")), ("text", n_!("Число"))],
        phone: (2, 2),
        desktop: (2, 2),
        data: true,
    },
    Kind { kind: "gpu", label: n_!("Графика"), glyph: "\u{E30A}", views: METRIC_VIEWS, phone: (2, 2), desktop: (2, 2), data: true },
    Kind { kind: "battery", label: n_!("Питание"), glyph: "\u{E1A4}", views: METRIC_VIEWS, phone: (2, 2), desktop: (2, 2), data: true },
    Kind {
        kind: "network",
        label: n_!("Сеть"),
        glyph: "\u{E80D}",
        views: &[("card", n_!("Карточка")), ("line", n_!("График")), ("bars", n_!("Столбики")), ("text", n_!("Скорость"))],
        phone: (2, 2),
        desktop: (3, 2),
        data: true,
    },
    Kind {
        kind: "temps",
        label: n_!("Температуры"),
        glyph: "\u{E1FF}",
        views: &[("bars", n_!("Столбики")), ("line", n_!("График")), ("gauge", n_!("Шкала")), ("text", n_!("Список"))],
        phone: (4, 2),
        desktop: (3, 2),
        data: true,
    },
    Kind { kind: "apps", label: n_!("Запущенные приложения"), glyph: mi::APPS, views: &[("rail", n_!("Лента"))], phone: (4, 2), desktop: (5, 2), data: true },
    Kind { kind: "launcher", label: n_!("Значки приложений"), glyph: mi::GRID, views: &[("grid", n_!("Сетка"))], phone: (4, 4), desktop: (4, 3), data: false },
    Kind { kind: "note", label: n_!("Заметка"), glyph: "\u{E244}", views: &[("note", n_!("Заметка"))], phone: (2, 2), desktop: (2, 2), data: false },
];

/// Значки — не из каталога виджетов, а со вкладки «Значки».
const ICON_KINDS: &[&str] = &["app", "file"];

pub fn kind_info(kind: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.kind == kind)
}

pub fn is_icon(kind: &str) -> bool {
    ICON_KINDS.contains(&kind)
}

/// Вид виджета (`view` или первый вид типа).
pub fn view_of(w: &DeskWidget) -> &str {
    let first = kind_info(&w.kind).and_then(|k| k.views.first()).map(|v| v.0).unwrap_or("");
    match w.str("view") {
        Some(v) if kind_info(&w.kind).is_some_and(|k| k.views.iter().any(|x| x.0 == v)) => v,
        _ => first,
    }
}

/// Подпись виджета для меню и настроек.
pub fn label_of(w: &DeskWidget) -> String {
    match w.kind.as_str() {
        "app" => w.str("app").and_then(crate::xdg::app_by_id).map(|e| e.name.clone()).unwrap_or_else(|| t!("Приложение").into()),
        "file" => w.str("path").map(|p| file_name(&synshell_common::paths::expand_tilde(p))).unwrap_or_else(|| t!("Файл").into()),
        k => kind_info(k).map(|k| syngui::i18n::t(k.label)).unwrap_or_else(|| k.to_string()),
    }
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned())
}

// ─── Раскладка ──────────────────────────────────────────────────────────────

/// Виджеты из конфига или встроенные (все форм-факторы).
pub fn items(ctx: &ShellCtx) -> Vec<DeskWidget> {
    let cfg = ctx.cfg();
    cfg.widgets.clone().unwrap_or_else(|| defaults(ctx))
}

/// Встроенная раскладка: телефон — «Сводка» на первом столе и значки
/// приложений на остальных (как было до виджетов), компьютер — пусто.
pub fn defaults(ctx: &ShellCtx) -> Vec<DeskWidget> {
    let cfg = ctx.cfg();
    if !ctx.is_phone() {
        return Vec::new();
    }
    let cols = phone_columns(&cfg);
    let mut v = Vec::new();
    let phone = |mut w: DeskWidget, page: u32| {
        w.page = page;
        w.form_factor = "phone".into();
        w
    };
    let half = (cols / 2).max(1);
    if cfg.mobile.resources_page {
        v.push(phone(DeskWidget::new("clock", 0, 0, cols, 1), 1));
        v.push(phone(DeskWidget::new("cpu", 0, 1, cols, 3), 1));
        v.push(phone(DeskWidget::new("memory", 0, 4, half, 2), 1));
        v.push(phone(DeskWidget::new("gpu", half, 4, cols - half, 2), 1));
        v.push(phone(DeskWidget::new("battery", 0, 6, half, 2), 1));
        v.push(phone(DeskWidget::new("network", half, 6, cols - half, 2), 1));
        v.push(phone(DeskWidget::new("apps", 0, 8, cols, 2), 1));
    }
    if cfg.wallpaper.desktop_icons {
        let first = if cfg.mobile.resources_page { 2 } else { 1 };
        for page in first..=cfg.workspaces.count.max(1) {
            v.push(phone(DeskWidget::new("launcher", 0, 0, 0, 0), page));
        }
    }
    v
}

fn phone_columns(cfg: &synshell_common::Config) -> u32 {
    cfg.mobile.home_columns.clamp(2, 8)
}

/// Записать раскладку в config.toml (оболочка перечитает его без
/// пересборки поверхностей — виджеты перестроятся сами).
pub fn save(list: Vec<DeskWidget>) {
    match synshell_common::config_edit::set_widgets(&list) {
        Ok(_) => crate::reload_after_write(),
        Err(e) => {
            log::error!("рабочий стол: не удалось сохранить виджеты: {e:#}");
            crate::osd::show(ShellCtx::get(), mi::INFO, None, t!("Не удалось сохранить config.toml").into());
        }
    }
}

/// Изменить виджет `index` и сохранить.
pub fn update(index: usize, f: impl FnOnce(&mut DeskWidget)) {
    let ctx = ShellCtx::get();
    let mut list = items(&ctx);
    if let Some(w) = list.get_mut(index) {
        f(w);
        save(list);
    }
}

pub fn remove(index: usize) {
    let ctx = ShellCtx::get();
    let mut list = items(&ctx);
    if index < list.len() {
        list.remove(index);
        save(list);
    }
}

/// Сетка вывода: колонок, строк (видимых), клетка и начало, px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub cols: u32,
    pub rows: u32,
    pub cw: f32,
    pub ch: f32,
    pub x0: f32,
    pub y0: f32,
    /// Размер страницы, px.
    pub width: f32,
    pub height: f32,
}

impl Grid {
    /// Клетки виджета с подстановкой «до края» (0) и обрезкой по ширине.
    pub fn cells(&self, w: &DeskWidget) -> (u32, u32, u32, u32) {
        let x = w.x.min(self.cols.saturating_sub(1));
        let ww = if w.w == 0 { self.cols - x } else { w.w.min(self.cols - x) }.max(1);
        let hh = if w.h == 0 { self.rows.saturating_sub(w.y).max(1) } else { w.h };
        (x, w.y, ww, hh)
    }

    pub fn rect(&self, (x, y, w, h): (u32, u32, u32, u32)) -> [f32; 4] {
        [self.x0 + x as f32 * self.cw, self.y0 + y as f32 * self.ch, w as f32 * self.cw, h as f32 * self.ch]
    }

    /// Клетка под точкой страницы (левый верхний угол виджета).
    pub fn cell_at(&self, px: f32, py: f32, w: u32) -> (u32, u32) {
        let x = ((px - self.x0) / self.cw).round().max(0.0) as u32;
        let y = ((py - self.y0) / self.ch).round().max(0.0) as u32;
        (x.min(self.cols.saturating_sub(w.max(1))), y)
    }
}

/// Место под панелями, которые его резервируют: [сверху, справа, снизу, слева].
fn reserved(ctx: &ShellCtx) -> [f32; 4] {
    let mut r = [0.0f32; 4];
    for p in ctx.cfg().panels.iter().filter(|p| p.shows_on(ctx.form_factor) && p.exclusive && !p.autohide) {
        let size = p.size as f32 + if p.floating { 8.0 } else { 0.0 };
        let i = match p.edge {
            Edge::Top => 0,
            Edge::Right => 1,
            Edge::Bottom => 2,
            Edge::Left => 3,
        };
        r[i] = r[i].max(size);
    }
    r
}

/// Сетка страницы вывода `output`.
pub fn grid(ctx: &ShellCtx, output: &str) -> Grid {
    let cfg = ctx.cfg();
    let (w, h) = crate::manager::output_size(Some(output));
    let r = reserved(ctx);
    if ctx.is_phone() {
        // Домашний экран уже не заходит под панели (exclusive zone 0).
        let (w, h) = (w - r[1] - r[3], h - r[0] - r[2]);
        let pad = 10.0;
        let cols = phone_columns(&cfg);
        let cw = ((w - 2.0 * pad) / cols as f32).floor().max(40.0);
        let ch = if cfg.desktop.phone_row > 0 { cfg.desktop.phone_row as f32 } else { cw };
        // сверху — точки столов
        let y0 = 34.0;
        let rows = ((h - y0 - pad) / ch).floor().max(1.0) as u32;
        Grid { cols, rows, cw, ch, x0: pad, y0, width: w, height: h }
    } else {
        let cell = (cfg.desktop.cell.max(32)) as f32;
        let pad = 12.0;
        let (aw, ah) = (w - r[1] - r[3] - 2.0 * pad, h - r[0] - r[2] - 2.0 * pad);
        let cols = (aw / cell).floor().max(1.0) as u32;
        let rows = (ah / cell).floor().max(1.0) as u32;
        Grid { cols, rows, cw: cell, ch: cell, x0: r[3] + pad, y0: r[0] + pad, width: w, height: h }
    }
}

/// Виджет на странице `page` (с 0) вывода `output`.
fn on_page(ctx: &ShellCtx, w: &DeskWidget, page: usize, output: &str) -> bool {
    if !w.shows_on(ctx.form_factor) || !(w.page == 0 || w.page as usize == page + 1) {
        return false;
    }
    if ctx.is_phone() || w.output.is_empty() {
        // Без своего монитора — на основном.
        return ctx.is_phone() || crate::manager::primary_output(ctx).as_deref().is_none_or(|p| p == output);
    }
    w.output == output
}

fn overlaps(a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// Встанет ли `cells` на страницу `page` (1…; 0 — на все) без наложений,
/// кроме виджета `skip`.
pub fn fits(ctx: &ShellCtx, g: &Grid, list: &[DeskWidget], page: u32, cells: (u32, u32, u32, u32), skip: Option<usize>) -> bool {
    if cells.0 + cells.2 > g.cols {
        return false;
    }
    list.iter().enumerate().filter(|(i, w)| Some(*i) != skip && w.shows_on(ctx.form_factor)).all(|(_, w)| {
        let same = page == 0 || w.page == 0 || w.page == page;
        !same || !overlaps(cells, g.cells(w))
    })
}

/// Первое свободное место размера `w`×`h` (сверху вниз, слева направо).
pub fn free_spot(ctx: &ShellCtx, g: &Grid, list: &[DeskWidget], page: u32, w: u32, h: u32) -> (u32, u32) {
    let w = w.min(g.cols).max(1);
    for y in 0..(g.rows + 64) {
        for x in 0..=(g.cols - w) {
            if fits(ctx, g, list, page, (x, y, w, h), None) {
                return (x, y);
            }
        }
    }
    (0, g.rows)
}

/// Стол, на который добавлять: текущий (телефон) или все (компьютер).
pub fn current_page(ctx: &ShellCtx) -> u32 {
    if ctx.is_phone() {
        ctx.workspaces.get_untracked().iter().find(|w| w.active).map(|w| w.index + 1).unwrap_or(1)
    } else {
        0
    }
}

/// Добавить виджет или значок на свободное место стола `page`.
pub fn add(mut w: DeskWidget, page: u32) {
    let ctx = ShellCtx::get();
    let out = output_name(&ctx);
    let g = grid(&ctx, &out);
    let mut list = items(&ctx);
    let (dw, dh) = (w.w.max(1).min(g.cols), w.h.max(1));
    let (x, y) = free_spot(&ctx, &g, &list, page, dw, dh);
    w.x = x;
    w.y = y;
    w.w = dw;
    w.h = dh;
    w.page = page;
    if w.form_factor.is_empty() {
        w.form_factor = if ctx.is_phone() { "phone" } else { "desktop" }.into();
    }
    list.push(w);
    save(list);
}

/// Новый виджет типа `kind` с размером по умолчанию.
pub fn new_widget(ctx: &ShellCtx, kind: &str) -> DeskWidget {
    let (w, h) = kind_info(kind).map(|k| if ctx.is_phone() { k.phone } else { k.desktop }).unwrap_or((1, 1));
    DeskWidget::new(kind, 0, 0, w, h)
}

fn output_name(ctx: &ShellCtx) -> String {
    if ctx.is_phone() {
        syngui_layer::outputs().get_untracked().first().map(|o| o.name.clone()).unwrap_or_default()
    } else {
        crate::manager::primary_output(ctx).unwrap_or_default()
    }
}

/// Есть ли на странице виджеты, которым нужны данные сборщика.
pub fn has_data(ctx: &ShellCtx, page: usize) -> bool {
    let out = output_name(ctx);
    items(ctx).iter().any(|w| kind_info(&w.kind).is_some_and(|k| k.data) && on_page(ctx, w, page, &out))
}

// ─── Файлы ~/Desktop ────────────────────────────────────────────────────────

pub fn desktop_dir() -> PathBuf {
    // XDG_DESKTOP_DIR из user-dirs.dirs, иначе ~/Desktop.
    let home = synshell_common::paths::expand_tilde("~");
    let conf = home.join(".config/user-dirs.dirs");
    if let Ok(s) = std::fs::read_to_string(conf) {
        for line in s.lines() {
            if let Some(v) = line.strip_prefix("XDG_DESKTOP_DIR=") {
                let v = v.trim_matches('"').replace("$HOME", &home.to_string_lossy());
                return PathBuf::from(v);
            }
        }
    }
    home.join("Desktop")
}

fn scan_desktop() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(desktop_dir())
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Файлы `~/Desktop`; каталог перечитывается раз в 3 с (сигнал меняется,
/// только если состав другой).
fn desktop_files() -> (RwSignal<u64>, Vec<PathBuf>) {
    let rev = DESKTOP_REV.with(|r| match r.get() {
        Some(s) => s,
        None => {
            let s = use_signal(0u64);
            r.set(Some(s));
            DESKTOP_FILES.with(|f| *f.borrow_mut() = scan_desktop());
            syngui_layer::add_timer(std::time::Duration::from_secs(3), move || {
                let now = scan_desktop();
                let changed = DESKTOP_FILES.with(|f| *f.borrow() != now);
                if changed {
                    DESKTOP_FILES.with(|f| *f.borrow_mut() = now);
                    s.set(s.get_untracked() + 1);
                }
                Some(std::time::Duration::from_secs(3))
            });
            s
        }
    });
    (rev, DESKTOP_FILES.with(|f| f.borrow().clone()))
}

/// Что стоит в клетке: виджет из конфига (номер) или файл `~/Desktop`,
/// которому место выбрано само.
#[derive(Clone, Debug, PartialEq)]
pub enum Slot {
    Item(usize),
    Auto(PathBuf),
}

// ─── Страница ───────────────────────────────────────────────────────────────

/// Виджеты и значки страницы `page` (стол с 0) на выводе `output`: сетка
/// поверх обоев; в режиме правки — клетки, рамки и панель правки.
pub fn page_view(ctx: ShellCtx, page: usize, output: String) -> impl Widget {
    let edit = editing();
    let body = rx(move || {
        let _ = ctx.config.get();
        let _ = syngui_layer::outputs().get();
        let editing = edit.get();
        let cfg = ctx.cfg();
        let icons = !ctx.is_phone() && cfg.wallpaper.desktop_icons;
        let files = if icons {
            let (rev, files) = desktop_files();
            let _ = rev.get();
            files
        } else {
            Vec::new()
        };
        let g = grid(&ctx, &output);
        let list = items(&ctx);
        let mut placed: Vec<(Slot, DeskWidget)> =
            list.iter().enumerate().filter(|(_, w)| on_page(&ctx, w, page, &output)).map(|(i, w)| (Slot::Item(i), w.clone())).collect();
        // Файлы рабочего стола без своего места — в свободные клетки
        // сверху вниз, затем вправо (как в KDE).
        if icons {
            let pinned: Vec<PathBuf> = list.iter().filter(|w| w.kind == "file").filter_map(|w| w.str("path")).map(synshell_common::paths::expand_tilde).collect();
            let mut taken: Vec<DeskWidget> = placed.iter().map(|p| p.1.clone()).collect();
            let mut col = 0u32;
            let mut row = 0u32;
            for f in files.iter().filter(|f| !pinned.contains(f)) {
                let mut spot = None;
                while col < g.cols {
                    let c = (col, row, 1, 1);
                    let free = !taken.iter().any(|w| overlaps(c, g.cells(w)));
                    row += 1;
                    if row >= g.rows {
                        row = 0;
                        col += 1;
                    }
                    if free {
                        spot = Some(c);
                        break;
                    }
                }
                let Some((x, y, _, _)) = spot else { break };
                let w = DeskWidget::new("file", x, y, 1, 1).with("path", f.to_string_lossy().into_owned());
                taken.push(w.clone());
                placed.push((Slot::Auto(f.clone()), w));
            }
        }
        let bottom = placed.iter().map(|(_, w)| {
            let (_, y, _, h) = g.cells(w);
            y + h
        });
        let rows = bottom.max().unwrap_or(0).max(g.rows);
        let content_h = g.y0 + rows as f32 * g.ch + 12.0;
        let mut stack = Stack::new().child(
            DecoratedBox::new().style("width", syngui::mss::StyleValue::px(g.width)).style("height", syngui::mss::StyleValue::px(content_h.max(g.height))),
        );
        if editing {
            let taken: Vec<(u32, u32, u32, u32)> = placed.iter().map(|(_, w)| g.cells(w)).collect();
            stack = stack.child(Positioned::new(grid_canvas(g, rows, taken)).at(0.0, 0.0).dimensions(g.width, content_h.max(g.height)));
        }
        for (slot, w) in placed {
            stack = stack.child(place(ctx, g, slot, w, editing));
        }
        let page_w: Box<dyn Widget> = if ctx.is_phone() && content_h > g.height + 1.0 {
            Box::new(ScrollView::new().vertical().child(stack))
        } else {
            Box::new(stack)
        };
        page_w
    });
    // Панель правки — внизу: колонка с выравниванием снаружи `rx`
    // (Reactive отдаёт детям свободные ограничения).
    let bar = rx(move || if edit.get() { Box::new(edit_bar(ctx)) } else { Box::new(DecoratedBox::new()) });
    Stack::new()
        .fit(StackFit::Expand)
        .child(body)
        .child(Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center).child(bar))
}

/// Клетки сетки (режим правки) и рамка-подсказка при перетаскивании.
fn grid_canvas(g: Grid, rows: u32, taken: Vec<(u32, u32, u32, u32)>) -> impl Widget {
    Canvas::new(move |c, _| {
        c.set_color(Color::from_hex("#ffffff1c"));
        for y in 0..rows {
            for x in 0..g.cols {
                if taken.iter().any(|t| overlaps(*t, (x, y, 1, 1))) {
                    continue;
                }
                let [px, py, w, h] = g.rect((x, y, 1, 1));
                c.fill_rounded_rect(px + GAP, py + GAP, w - 2.0 * GAP, h - 2.0 * GAP, 10.0);
            }
        }
        if let Some(([x, y, w, h], ok)) = GHOST.with(|g| g.get()) {
            c.set_color(if ok { Color::from_hex("#ffffff40") } else { Color::from_hex("#ef535060") });
            c.fill_rounded_rect(x + GAP, y + GAP, w - 2.0 * GAP, h - 2.0 * GAP, 14.0);
        }
    })
    .animated(true)
}

/// Виджет на своём месте страницы; в режиме правки — с рамкой, ручкой
/// размера и «×», перетаскивается по клеткам.
fn place(ctx: ShellCtx, g: Grid, slot: Slot, w: DeskWidget, editing: bool) -> impl Widget {
    let cells = g.cells(&w);
    let [x, y, pw, ph] = g.rect(cells);
    let inner = (pw - 2.0 * GAP, ph - 2.0 * GAP);
    let content: Box<dyn Widget> = views::build(ctx, &slot, &w, inner.0, inner.1, editing);
    let off = use_signal(Point::new(x + GAP, y + GAP));
    if !editing {
        return Positioned::new(content).offset_signal(off).dimensions(inner.0, inner.1);
    }
    let page = w.page;
    // Ручка размера в правом нижнем углу.
    let resize = {
        let slot = slot.clone();
        let slot_end = slot.clone();
        GestureDetector::new()
            .on_pan_update(move |u| {
                let nw = ((pw + u.total.x) / g.cw).round().max(1.0) as u32;
                let nh = ((ph + u.total.y) / g.ch).round().max(1.0) as u32;
                let c = (cells.0, cells.1, nw.min(g.cols - cells.0), nh);
                let ok = match &slot {
                    Slot::Item(i) => fits(&ShellCtx::get(), &g, &items(&ShellCtx::get()), page, c, Some(*i)),
                    Slot::Auto(_) => false,
                };
                GHOST.with(|gh| gh.set(Some((g.rect(c), ok))));
            })
            .on_pan_end(move |_| {
                let ghost = GHOST.with(|gh| gh.take());
                if let (Some(([_, _, gw, gh], true)), Slot::Item(i)) = (ghost, &slot_end) {
                    let (nw, nh) = ((gw / g.cw).round() as u32, (gh / g.ch).round() as u32);
                    update(*i, |w| {
                        w.w = nw;
                        w.h = nh;
                    });
                }
            })
            .child(DecoratedBox::new().child(icon("\u{E5D0}").class("desk-resize-icon")).class("desk-resize"))
    };
    let remove_btn = {
        let slot = slot.clone();
        GestureDetector::new()
            .on_click(move || {
                if let Slot::Item(i) = &slot {
                    remove(*i);
                }
            })
            .child(DecoratedBox::new().child(icon(mi::CLOSE).class("desk-x-icon")).class("desk-x"))
    };
    let mut chrome = Stack::new()
        .fit(StackFit::Expand)
        .child(content)
        .child(DecoratedBox::new().class("desk-edit-frame"));
    if matches!(slot, Slot::Item(_)) {
        chrome = chrome
            .child(Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::End).child(resize))
            .child(Column::new().cross_axis_alignment(CrossAxisAlignment::End).child(remove_btn));
    }
    let (sx, sy) = (x + GAP, y + GAP);
    let tap_slot = slot.clone();
    let end_slot = slot.clone();
    let mover = GestureDetector::new()
        .on_pan_update(move |u| {
            let p = Point::new(sx + u.total.x, sy + u.total.y);
            off.set(p);
            let (cx, cy) = g.cell_at(p.x - GAP, p.y - GAP, cells.2);
            let c = (cx, cy, cells.2, cells.3);
            let ctx = ShellCtx::get();
            let ok = match &slot {
                Slot::Item(i) => fits(&ctx, &g, &items(&ctx), page, c, Some(*i)),
                Slot::Auto(_) => fits(&ctx, &g, &items(&ctx), page, c, None),
            };
            GHOST.with(|gh| gh.set(Some((g.rect(c), ok))));
        })
        .on_pan_end(move |_| {
            let ghost = GHOST.with(|gh| gh.take());
            match ghost {
                Some(([gx, gy, _, _], true)) => {
                    let (cx, cy) = g.cell_at(gx, gy, cells.2);
                    let ctx = ShellCtx::get();
                    match &end_slot {
                        Slot::Item(i) => update(*i, |w| {
                            w.x = cx;
                            w.y = cy;
                        }),
                        // Файл рабочего стола запоминает место.
                        Slot::Auto(p) => {
                            let mut list = items(&ctx);
                            let mut w = DeskWidget::new("file", cx, cy, 1, 1).with("path", p.to_string_lossy().into_owned());
                            w.form_factor = "desktop".into();
                            list.push(w);
                            save(list);
                        }
                    }
                }
                _ => off.set(Point::new(sx, sy)),
            }
        })
        .on_click(move || {
            if let Slot::Item(i) = &tap_slot {
                let ctx = ShellCtx::get();
                ctx.open_popup(PopupKind::DeskEdit(*i), crate::commands::centered());
            }
        })
        .child(chrome);
    Positioned::new(mover).offset_signal(off).dimensions(inner.0, inner.1)
}

/// Панель режима правки: добавить виджет или значок, готово.
fn edit_bar(ctx: ShellCtx) -> impl Widget {
    let button = |glyph: &'static str, label: &'static str, f: Box<dyn Fn() + Send + Sync>| {
        GestureDetector::new().on_click(move || f()).child(
            DecoratedBox::new()
                .child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(glyph).class("desk-bar-icon")).child(Text::new(syngui::i18n::t(label)).class("desk-bar-label")))
                .class("desk-bar-btn"),
        )
    };
    let bar = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(button(
            "\u{E145}",
            n_!("Виджет"),
            Box::new(move || {
                let ctx = ShellCtx::get();
                ctx.open_popup(PopupKind::DeskAdd(current_page(&ctx)), crate::commands::centered());
            }),
        ))
        .child(button(
            mi::APPS,
            n_!("Значок"),
            Box::new(move || {
                let ctx = ShellCtx::get();
                forms::set_add_tab(1);
                ctx.open_popup(PopupKind::DeskAdd(current_page(&ctx)), crate::commands::centered());
            }),
        ))
        .child(button("\u{E876}", n_!("Готово"), Box::new(stop_editing)))
        .class("desk-bar");
    let _ = ctx;
    DecoratedBox::new().child(bar).class("desk-bar-place")
}

/// Открыть окно по центру после того, как пункт меню закроет своё
/// (`menu_item` закрывает всплывающее окно уже после обработчика).
pub fn open_later(kind: PopupKind) {
    syngui_layer::add_timer(std::time::Duration::from_millis(30), move || {
        let ctx = ShellCtx::get();
        ctx.popup.set(Some(crate::ctx::Popup { kind: kind.clone(), anchor: crate::commands::centered() }));
        None
    });
}

/// Открыть меню виджета у точки нажатия.
pub fn open_menu(slot: &Slot) {
    let ctx = ShellCtx::get();
    let kind = match slot {
        Slot::Item(i) => PopupKind::DeskItem(*i),
        Slot::Auto(p) => PopupKind::DeskFile(p.to_string_lossy().into_owned()),
    };
    crate::edit::open_at_press(&ctx, kind);
}

/// Синхронизировать сбор данных с тем, что на экране (компьютер: виджеты
/// активного стола видны всегда, обои под окнами не прячутся).
pub fn track_visibility_desktop(ctx: ShellCtx) {
    create_effect(move || {
        let _ = ctx.config.get();
        let ws = ctx.workspaces.get();
        let page = ws.iter().find(|w| w.active).map(|w| w.index as usize).unwrap_or(0);
        data::set_visible(has_data(&ctx, page));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Grid {
        Grid { cols: 4, rows: 10, cw: 100.0, ch: 100.0, x0: 10.0, y0: 34.0, width: 420.0, height: 1100.0 }
    }

    #[test]
    fn zero_size_stretches_to_edges() {
        let g = grid();
        assert_eq!(g.cells(&DeskWidget::new("launcher", 0, 0, 0, 0)), (0, 0, 4, 10));
        assert_eq!(g.cells(&DeskWidget::new("launcher", 1, 3, 0, 0)), (1, 3, 3, 7));
        // шире сетки — обрезается по правому краю
        assert_eq!(g.cells(&DeskWidget::new("cpu", 2, 0, 5, 2)), (2, 0, 2, 2));
    }

    #[test]
    fn overlap_and_cell_at() {
        assert!(overlaps((0, 0, 2, 2), (1, 1, 2, 2)));
        assert!(!overlaps((0, 0, 2, 2), (2, 0, 2, 2)));
        assert!(!overlaps((0, 0, 4, 1), (0, 1, 4, 3)));
        let g = grid();
        // левый верхний угол виджета шириной 2 у правого края — не дальше колонки 2
        assert_eq!(g.cell_at(390.0, 34.0, 2), (2, 0));
        assert_eq!(g.cell_at(160.0, 290.0, 1), (2, 3));
    }
}
