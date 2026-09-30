//! Обои: предпросмотр в рамке экрана, обои и рабочие столы (одни на все,
//! свои у каждого стола, панорама), галерея картинок каталога с миниатюрами
//! и редактор кадра — сдвиг и масштаб пальцами, мышью и колесом, как в
//! Android и iOS.
//!
//! Кадр хранится долями (`zoom` относительно «заполнить экран», `center` —
//! точка картинки в центре), геометрию считает
//! `synshell_common::wallpaper` — та же, что у оболочки, поэтому
//! предпросмотр совпадает с рабочим столом.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use synshell_common::config::{Wallpaper, WallpaperFrame};
use synshell_common::paths::expand_tilde;
use synshell_common::wallpaper as wg;
use syngui::prelude::*;
use syngui::input::Key;
use syngui::widgets::{EventHook, KeyReply};
use syngui::{GestureDetector, ImageViewCommand, ImageViewInfo, ImageViewport};

use crate::op;
use crate::state;
use crate::store;
use crate::sys;
use crate::ui::*;

/// Сколько картинок каталога показывать в галерее.
const GALLERY_LIMIT: usize = 240;

/// Что правим: общие обои или обои стола (с 0).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Target {
    All,
    Workspace(u32),
}

/// Открытый редактор кадра: картинка и для чего она.
#[derive(Clone, PartialEq, Debug)]
struct Edit {
    path: PathBuf,
    target: Target,
}

#[derive(Clone, Copy)]
struct Sig {
    /// Стол под предпросмотром (с 0): его обои показываются и правятся.
    ws: RwSignal<u32>,
    /// Каталог галереи.
    dir: RwSignal<PathBuf>,
    edit: RwSignal<Option<Edit>>,
    /// Готовы новые миниатюры.
    thumbs: RwSignal<u64>,
}

thread_local! {
    static SIG: RefCell<Option<Sig>> = const { RefCell::new(None) };
}

static THUMBS_PENDING: AtomicBool = AtomicBool::new(false);

fn sig() -> Sig {
    SIG.with(|cell| {
        *cell.borrow_mut().get_or_insert_with(|| {
            let s = Sig {
                ws: use_signal(0u32),
                dir: use_signal(initial_dir(&store::config().wallpaper)),
                edit: use_signal(None),
                thumbs: use_signal(0u64),
            };
            let t = s.thumbs;
            // Миниатюры делаются в фоновых потоках; пачку готовых — одной
            // перерисовкой галереи.
            synshell_common::thumbs::set_notify(move || {
                if !THUMBS_PENDING.swap(true, Ordering::AcqRel) {
                    run_on_main_thread(move || {
                        THUMBS_PENDING.store(false, Ordering::Release);
                        t.set(t.get_untracked() + 1);
                    });
                }
            });
            s
        })
    })
}

/// Закрыть редактор кадра («назад» на телефоне). `true` — был открыт.
pub fn close_editor() -> bool {
    SIG.with(|cell| match *cell.borrow() {
        Some(s) if s.edit.get_untracked().is_some() => {
            s.edit.set(None);
            true
        }
        _ => false,
    })
}

/// Каталог галереи при открытии: где лежат текущие обои, иначе первый
/// из известных каталогов обоев.
fn initial_dir(w: &Wallpaper) -> PathBuf {
    let p = expand_tilde(w.path.trim());
    if !w.path.trim().is_empty() {
        if p.is_dir() {
            return p;
        }
        if let Some(parent) = p.parent().filter(|d| d.is_dir()) {
            return parent.to_path_buf();
        }
    }
    wg::wallpaper_dirs().into_iter().next().unwrap_or_else(|| wg::xdg_pictures_dir(&expand_tilde("~")))
}

/// Логический размер основного экрана: пропорции рамок и кадра.
fn screen() -> (f32, f32) {
    let outs = sys::outputs().unwrap_or_default();
    let o = outs.iter().find(|o| o.primary).or_else(|| outs.iter().find(|o| o.focused)).or(outs.first());
    match o {
        Some(o) if o.geometry[2] > 0 && o.geometry[3] > 0 => (o.geometry[2] as f32, o.geometry[3] as f32),
        _ if narrow() => (400.0, 880.0),
        _ => (1920.0, 1080.0),
    }
}

fn target_of(w: &Wallpaper, ws: u32) -> Target {
    if w.per_workspace() {
        Target::Workspace(ws)
    } else {
        Target::All
    }
}

/// Обои цели: свои обои стола, иначе общие.
fn target_frame(w: &Wallpaper, t: Target) -> WallpaperFrame {
    match t {
        Target::All => w.base_frame(),
        Target::Workspace(i) => w.workspace_frame(i).cloned().unwrap_or_else(|| w.base_frame()),
    }
}

fn target_base(t: Target) -> P {
    match t {
        Target::All => op!["wallpaper"],
        Target::Workspace(i) => op!["wallpaper", "workspace", (i + 1).to_string()],
    }
}

fn at(base: &P, key: &str) -> P {
    let mut p = base.clone();
    p.push(key.into());
    p
}

fn ws_name(ws: u32) -> String {
    let c = store::config();
    c.workspaces.names.get(ws as usize).filter(|n| !n.trim().is_empty()).cloned().unwrap_or_else(|| format!("Стол {}", ws + 1))
}

fn target_label(t: Target) -> String {
    match t {
        Target::All => "Все рабочие столы".into(),
        Target::Workspace(i) => ws_name(i),
    }
}

/// Записать картинку с кадром для цели.
fn apply(t: Target, path: &Path, zoom: f32, center: (f32, f32)) {
    let base = target_base(t);
    let round = |v: f32| (v as f64 * 1000.0).round() / 1000.0;
    set(&at(&base, "path"), home_short(path));
    set(&at(&base, "zoom"), round(zoom.max(1.0)));
    let mut arr = toml_edit::Array::new();
    arr.push(round(center.0));
    arr.push(round(center.1));
    set(&at(&base, "center"), toml_edit::Value::Array(arr));
    // Кадр виден только при заполнении экрана.
    if store::config().wallpaper.mode != "fill" {
        set(&op!["wallpaper", "mode"], "fill");
    }
}

/// Поставить путь цели без кадра (каталог для слайд-шоу, путь из поля).
fn set_path(t: Target, path: &str) {
    let base = target_base(t);
    set(&at(&base, "path"), path.to_string());
    if t != Target::All || !path.is_empty() {
        set(&at(&base, "zoom"), 1.0);
        let mut arr = toml_edit::Array::new();
        arr.push(0.5);
        arr.push(0.5);
        set(&at(&base, "center"), toml_edit::Value::Array(arr));
    }
}

fn mime_of(p: &Path) -> String {
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg".into(),
        e => format!("image/{e}"),
    }
}

fn thumb_of(p: &Path, px: u32) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(p).ok()?;
    synshell_common::thumbs::get(p, &mime_of(p), m.mtime(), px)
}

/// Файл для показа: HEIC/AVIF — через JPEG-копию; её ещё нет — `None`,
/// преобразование запущено, по готовности перерисуются миниатюры и превью
/// (`s.thumbs`).
fn shown(p: &Path) -> Option<PathBuf> {
    shown_notify(p, sig().thumbs)
}

/// То же, но по готовности растёт `ready` (у редактора кадра — свой сигнал:
/// общий `thumbs` растёт на каждую миниатюру галереи, и область кадра
/// пересоздавалась бы посреди жеста).
fn shown_notify(p: &Path, ready: RwSignal<u64>) -> Option<PathBuf> {
    let r = wg::displayable(p);
    if r.is_none() {
        wg::prepare(p, move || run_on_main_thread(move || ready.set(ready.get_untracked() + 1)));
    }
    r
}

fn img_size(p: &Path) -> Option<(f32, f32)> {
    syngui::gpu::image_file_size(&p.to_string_lossy()).map(|(w, h)| (w as f32, h as f32))
}

/// Экран с обоями `frame`: фон, картинка с кадром. `pano` — (столов,
/// сдвиг, стол): окно панорамы; кадр меняется плавно (`ms`).
fn screen_image(frame: &WallpaperFrame, mode: &str, scr: (f32, f32), pano: Option<(u32, f32, f32)>, ms: u32) -> W {
    let mut st = Stack::new().fit(StackFit::Expand).clip(true).child(DecoratedBox::new().class("wall-fill desk-bg"));
    if let Some(p) = wg::pick(&frame.path, 0).and_then(|p| shown(&p)) {
        let img = Image::new(p.to_string_lossy()).placeholder(false);
        let framed = mode == "fill" || pano.is_some();
        let img = match (framed, img_size(&p)) {
            (true, Some(size)) => {
                let [x, y, w, h] = wg::screen_uv(size, scr, frame.zoom, frame.center, pano);
                img.fit(ImageFit::Fill).crop_uv(x, y, w, h).crop_transition_ms(ms)
            }
            _ => img.fit(match mode {
                "fit" | "center" | "tile" => ImageFit::Contain,
                "stretch" => ImageFit::Fill,
                _ => ImageFit::Cover,
            }),
        };
        st = st.child(img.class("wall-image"));
    }
    boxed(st)
}

/// Открыть редактор кадра для картинки (снимок интерфейса, тесты).
pub fn open_editor(path: PathBuf) {
    let s = sig();
    let w = store::config().wallpaper;
    OPEN_ON_BUILD.with(|o| *o.borrow_mut() = Some(Edit { path, target: target_of(&w, s.ws.get_untracked()) }));
}

thread_local! {
    static OPEN_ON_BUILD: RefCell<Option<Edit>> = const { RefCell::new(None) };
}

// ─── Главная часть страницы ─────────────────────────────────────────────────

pub fn build() -> W {
    let s = sig();
    // Ушли со страницы и вернулись (или перестроили) — редактор закрыт.
    s.edit.set(OPEN_ON_BUILD.with(|o| o.borrow_mut().take()));
    let scr = screen();
    boxed(Reactive::new(move || -> Vec<W> {
        match s.edit.get() {
            Some(e) => vec![editor(e, scr, s)],
            None => vec![main_page(scr, s)],
        }
    }))
}

/// Рамка предпросмотра: обои выбранного стола.
fn preview(scr: (f32, f32), s: Sig) -> W {
    let max_h = if narrow() { 360.0 } else { 280.0 };
    let desktop = scr.0 >= scr.1;
    boxed(
        DecoratedBox::new().class(if desktop { "wall-hero-frame" } else { "wall-hero-frame wall-hero-phone" }).child(
            AspectRatio::new(scr.0 / scr.1).max_height(max_h).child(move || {
                state::ctx().tick.get();
                // Готова JPEG-копия HEIC/AVIF.
                s.thumbs.get();
                let ws = s.ws.get();
                let c = store::config();
                let w = &c.wallpaper;
                let frame = target_frame(w, target_of(w, ws));
                let pano = w.panorama().then_some((c.workspaces.count.max(1), w.panorama_shift, ws as f32));
                let mut st = Stack::new().fit(StackFit::Expand).clip(true).child(screen_image(&frame, &w.mode, scr, pano, 420));
                if desktop {
                    // Намёк на панель задач — видно, что это рабочий стол.
                    st = st.child(
                        Column::new()
                            .main_axis_alignment(MainAxisAlignment::End)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(DecoratedBox::new().class("wall-mock-bar")),
                    );
                }
                st
            }),
        ),
    )
}

/// Столы под предпросмотром: чей экран показать и править.
fn ws_chips(s: Sig) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        state::ctx().tick.get();
        let cur = s.ws.get();
        let c = store::config();
        let n = c.workspaces.count.max(1);
        let w = &c.wallpaper;
        if n < 2 || !(w.per_workspace() || w.panorama()) {
            return vec![];
        }
        let mut row = Flex::new().wrap().gap(6.0).class("wall-ws-row");
        for i in 0..n {
            let own = w.per_workspace() && w.workspace_frame(i).is_some();
            let mut inner = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(ws_name(i)).class("wall-ws-label"));
            if own {
                inner = inner.child(DecoratedBox::new().class("wall-ws-dot"));
            }
            let class = if i == cur { "wall-ws-chip selected" } else { "wall-ws-chip" };
            row = row.child(GestureDetector::new().on_click(move || s.ws.set(i)).child(DecoratedBox::new().class(class).child(inner)));
        }
        vec![boxed(row)]
    }))
}

const LAYOUTS: &[(&str, &str, &str)] = &[
    ("same", "Одни на все", "Одна картинка на всех рабочих столах."),
    ("workspace", "Свои у каждого", "У каждого стола своя картинка: выберите стол под предпросмотром и картинку ниже. Столы без своей картинки показывают общую."),
    ("panorama", "Панорама", "Одна широкая картинка на все столы: при переключении столов она плавно сдвигается вбок, как на домашнем экране телефона."),
];

fn controls(s: Sig) -> W {
    let c = store::config();
    let w = &c.wallpaper;
    let idx = LAYOUTS.iter().position(|(id, ..)| *id == w.layout).unwrap_or(0);
    let modes = SegmentedButton::new(LAYOUTS.iter().map(|(_, l, _)| *l).collect::<Vec<_>>()).selected(idx).on_change(|i| {
        let (id, ..) = LAYOUTS[i.min(LAYOUTS.len() - 1)];
        set(&op!["wallpaper", "layout"], id);
        if id == "panorama" && store::config().wallpaper.mode != "fill" {
            set(&op!["wallpaper", "mode"], "fill");
        }
        state::bump();
    });
    let mut col = Column::new()
        .gap(10.0)
        .class("wall-controls")
        .child(Text::new("Обои и рабочие столы").class("wall-controls-title"))
        .child(modes)
        .child(Text::new(LAYOUTS[idx].2).class("row-hint"));

    // Что правится сейчас и действия с ним.
    col = col.child(Reactive::new(move || -> Vec<W> {
        state::ctx().tick.get();
        let ws = s.ws.get();
        let c = store::config();
        let w = &c.wallpaper;
        let t = target_of(w, ws);
        let frame = target_frame(w, t);
        let path = expand_tilde(frame.path.trim());
        let mut out: Vec<W> = Vec::new();
        let name = if frame.path.trim().is_empty() {
            "Градиент (картинка не выбрана)".to_string()
        } else {
            path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| frame.path.clone())
        };
        let own = matches!(t, Target::Workspace(i) if w.workspace_frame(i).is_some());
        let whose = match t {
            Target::All => "Сейчас".to_string(),
            Target::Workspace(i) if own => format!("{}: своя картинка", ws_name(i)),
            Target::Workspace(i) => format!("{}: общая картинка", ws_name(i)),
        };
        out.push(boxed(
            Column::new()
                .gap(2.0)
                .class("wall-current")
                .child(Text::new(whose).class("row-hint"))
                .child(Text::new(name).max_lines(1).elide(Elide::Middle).class("row-label")),
        ));
        let mut buttons = Flex::new().wrap().gap(8.0);
        if path.is_file() {
            let p = path.clone();
            buttons = buttons.child(Button::new("Настроить кадр").icon(icons::CROP).class("btn primary").on_click(move || {
                s.edit.set(Some(Edit { path: p.clone(), target: t }));
            }));
        } else if path.is_dir() {
            out.push(boxed(Text::new("Каталог: картинки сменяются слайд-шоу").class("row-hint")));
        }
        if sys::has_file_dialog() {
            buttons = buttons.child(Button::new("Файл…").icon(icons::FILE).class("btn").on_click(move || {
                let start = s.dir.get_untracked().display().to_string();
                std::thread::spawn(move || {
                    if let Some(p) = sys::pick_path(false, &start) {
                        run_on_main_thread(move || s.edit.set(Some(Edit { path: PathBuf::from(p), target: t })));
                    }
                });
            }));
        }
        if own {
            buttons = buttons.child(Button::new("Как на других столах").icon(icons::UNDO).class("btn").on_click(move || {
                if let Target::Workspace(i) = t {
                    unset(&op!["wallpaper", "workspace", (i + 1).to_string()]);
                    state::bump();
                }
            }));
        }
        out.push(boxed(buttons));
        let mut col = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for w in out {
            col = col.child(w);
        }
        vec![boxed(col)]
    }));
    if w.panorama() {
        col = col.child(row_wide(
            "Сдвиг на один стол",
            "Доля ширины экрана, на которую уезжает картинка при переходе на соседний стол",
            slider(op!["wallpaper", "panorama_shift"], w.panorama_shift.clamp(0.05, 1.0) as f64, 0.1, 1.0, 0.05, 2),
        ));
    }
    boxed(col)
}

fn hero(scr: (f32, f32), s: Sig) -> W {
    let left = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(preview(scr, s)).child(ws_chips(s));
    if narrow() {
        boxed(Column::new().gap(14.0).class("group-card wall-hero").cross_axis_alignment(CrossAxisAlignment::Stretch).child(left).child(controls(s)))
    } else {
        boxed(
            Row::new()
                .gap(20.0)
                .class("group-card wall-hero")
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .child(DecoratedBox::new().class("wall-hero-left").child(left))
                .child(DecoratedBox::new().class("grow").child(controls(s))),
        )
    }
}

/// Кнопки каталогов галереи.
fn dir_chips(s: Sig) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let cur = s.dir.get();
        let home = expand_tilde("~");
        let mut dirs = wg::wallpaper_dirs();
        if !dirs.contains(&cur) {
            dirs.push(cur.clone());
        }
        let mut row = Flex::new().wrap().gap(6.0);
        for d in dirs {
            let label = if d == wg::xdg_pictures_dir(&home) {
                "Изображения".to_string()
            } else if d.starts_with("/usr/share") {
                format!("Системные: {}", d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
            } else {
                d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| d.display().to_string())
            };
            let class = if d == cur { "wall-dir-chip selected" } else { "wall-dir-chip" };
            let dd = d.clone();
            row = row.child(
                GestureDetector::new().on_click(move || s.dir.set(dd.clone())).child(
                    DecoratedBox::new().class(class).child(
                        Row::new()
                            .gap(6.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(Icon::new(icons::FOLDER).class("wall-dir-icon"))
                            .child(Text::new(label).max_lines(1).class("wall-dir-label")),
                    ),
                ),
            );
        }
        if sys::has_file_dialog() {
            row = row.child(Button::new("Другой каталог…").icon(icons::ADD).class("btn small").on_click(move || {
                let start = s.dir.get_untracked().display().to_string();
                std::thread::spawn(move || {
                    if let Some(p) = sys::pick_path(true, &start) {
                        run_on_main_thread(move || s.dir.set(PathBuf::from(p)));
                    }
                });
            }));
        }
        vec![boxed(row)]
    }))
}

/// Сетка миниатюр каталога; касание — редактор кадра этой картинки.
fn gallery(scr: (f32, f32), s: Sig) -> W {
    let vp = syngui::viewport::viewport_size();
    boxed(Reactive::new(move || -> Vec<W> {
        state::ctx().tick.get();
        s.thumbs.get();
        let dir = s.dir.get();
        let ws = s.ws.get_untracked();
        // Ширина ячейки: на телефоне — три в ряд по ширине окна.
        let cell = if narrow() { ((vp.get().width - 24.0 - 2.0 * 26.0 - 2.0 * 8.0) / 3.0).floor().max(84.0) } else { 172.0 };
        let c = store::config();
        let w = &c.wallpaper;
        let t = target_of(w, ws);
        let current = expand_tilde(target_frame(w, t).path.trim());
        let files = wg::gallery(&dir);
        if files.is_empty() {
            return vec![note("В этом каталоге нет картинок (PNG, JPEG, WebP, HEIC, AVIF, BMP, GIF).")];
        }
        // Высокий экран (телефон) — миниатюры пониже, чтобы влезало больше.
        let ratio = (scr.0 / scr.1).max(0.62);
        let mut grid = Flex::new().wrap().gap(8.0).class("wall-grid");
        for p in files.into_iter().take(GALLERY_LIMIT) {
            let selected = p == current;
            let thumb: W = match thumb_of(&p, (cell * 2.0) as u32) {
                Some(t) => boxed(Image::new(t.to_string_lossy()).fit(ImageFit::Cover).placeholder(false).class("wall-thumb-img")),
                None => boxed(DecoratedBox::new().class("wall-thumb-loading")),
            };
            let mut st = Stack::new().fit(StackFit::Expand).clip(true).child(thumb);
            if selected {
                st = st.child(
                    Column::new().main_axis_alignment(MainAxisAlignment::Start).cross_axis_alignment(CrossAxisAlignment::End).child(
                        DecoratedBox::new().class("wall-thumb-check").child(Icon::new(icons::CHECK).class("wall-thumb-check-icon")),
                    ),
                );
            }
            let class = if selected { "wall-thumb selected" } else { "wall-thumb" };
            let pp = p.clone();
            grid = grid.child(
                GestureDetector::new()
                    .on_click(move || s.edit.set(Some(Edit { path: pp.clone(), target: t })))
                    .child(DecoratedBox::new().class(class).style("width", cell).child(AspectRatio::new(ratio).child(st))),
            );
        }
        vec![boxed(grid)]
    }))
}

fn main_page(scr: (f32, f32), s: Sig) -> W {
    let c = store::config();
    let w = &c.wallpaper;
    let ws = s.ws.get_untracked().min(c.workspaces.count.max(1) - 1);
    let t = target_of(w, ws);

    let slideshow_dir = Button::new("Слайд-шоу из этого каталога").icon(icons::SLIDESHOW).class("btn small").on_click(move || {
        let t = target_of(&store::config().wallpaper, s.ws.get_untracked());
        set_path(t, &home_short(&s.dir.get_untracked()));
        state::bump();
    });
    let pictures = group(
        "Картинки",
        vec![
            boxed(Column::new().gap(10.0).class("wall-gallery-head").child(dir_chips(s)).child({
                let dir_label = move || Text::new(home_short(&s.dir.get())).elide(Elide::Middle).class("row-hint grow");
                let w: W = if narrow() {
                    boxed(Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Start).child(dir_label).child(slideshow_dir))
                } else {
                    boxed(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(dir_label).child(slideshow_dir))
                };
                w
            })),
            boxed(DecoratedBox::new().class("wall-gallery").child(gallery(scr, s))),
        ],
    );

    let path_field = TextField::with_text(target_frame(w, t).path)
        .placeholder("Путь к картинке или каталогу")
        .width(if narrow() { 260.0 } else { 380.0 })
        .on_change(move |v| set_path(t, v));

    let mut per_output: Vec<W> = w
        .per_output
        .iter()
        .map(|(out, path)| {
            let out_s = out.clone();
            row(
                out,
                "",
                Row::new()
                    .gap(8.0)
                    .child(text(op!["wallpaper", "per_output", out.as_str()], path, "Путь", 260.0))
                    .child(danger_icon_button(icons::DELETE, move || {
                        unset(&op!["wallpaper", "per_output", out_s.as_str()]);
                        state::bump();
                    })),
            )
        })
        .collect();
    let new_out = use_signal(String::new());
    per_output.push(row(
        "Добавить монитор",
        "Имя вывода: eDP-1, HDMI-A-1…",
        Row::new()
            .gap(8.0)
            .child(TextField::new().placeholder("HDMI-A-1").width(160.0).on_change(move |s| new_out.set(s.to_string())))
            .child(icon_button(icons::ADD, move || {
                let n = new_out.get_untracked().trim().to_string();
                if !n.is_empty() {
                    set(&op!["wallpaper", "per_output", n], "");
                    state::bump();
                }
            })),
    ));

    let mut image_rows = vec![row_wide(
        match t {
            Target::All => "Картинка или каталог".to_string(),
            Target::Workspace(_) => format!("Картинка или каталог — {}", ws_name(ws)),
        }
        .as_str(),
        "Каталог — слайд-шоу; пусто — градиент из цветов ниже",
        path_field,
    )];
    if !w.panorama() {
        image_rows.push(choice_row(
            "Заполнение",
            "Кадр настраивается при «Заполнить»",
            op!["wallpaper", "mode"],
            &w.mode,
            &[("fill", "Заполнить (кадр)"), ("fit", "Вписать"), ("stretch", "Растянуть"), ("center", "По центру"), ("tile", "Мозаика")],
        ));
    }
    image_rows.push(int_row(
        "Слайд-шоу",
        "Смена картинки из каталога, минут (0 — выкл.)",
        op!["wallpaper", "slideshow_minutes"],
        w.slideshow_minutes as i64,
        0,
        1440,
        5,
    ));

    page(
        "Обои",
        "Фон рабочих столов: картинка с кадром, свои обои для каждого стола или панорама.",
        vec![
            hero(scr, s),
            pictures,
            group("Изображение", image_rows),
            group(
                "Цвет",
                vec![
                    row("Основной цвет", "", color_field(op!["wallpaper", "color"], &w.color, "#1b2233")),
                    row("Второй цвет", "Пусто — сплошной цвет", color_field(op!["wallpaper", "color2"], &w.color2, "")),
                ],
            ),
            group(
                "Рабочий стол",
                vec![switch_row(
                    "Значки на рабочем столе",
                    "Файлы из ~/Desktop; на телефоне — сетка приложений на домашнем экране",
                    op!["wallpaper", "desktop_icons"],
                    w.desktop_icons,
                )],
            ),
            group("Свои обои для мониторов", per_output),
        ],
    )
}

// ─── Редактор кадра ─────────────────────────────────────────────────────────

fn editor(e: Edit, scr: (f32, f32), s: Sig) -> W {
    let c = store::config();
    let w = &c.wallpaper;
    let n = c.workspaces.count.max(1);
    let pano = w.panorama();
    let shift = w.panorama_shift;
    // Панорама кадрируется под холст всех столов, обычные обои — под экран.
    let view = if pano { wg::panorama_canvas(scr, n, shift) } else { scr };
    let frame = target_frame(w, e.target);
    let same = expand_tilde(frame.path.trim()) == e.path;
    let (zoom, center) = if same { (frame.zoom, frame.center) } else { (1.0, [0.5, 0.5]) };

    let info = use_signal(ImageViewInfo::default());
    let cmd = use_signal((0u64, ImageViewCommand::Fill));
    let send = move |c: ImageViewCommand| cmd.set((cmd.get_untracked().0 + 1, c));
    let src = e.path.clone();
    let converted = use_signal(0u64);
    let viewport = Reactive::new(move || -> Vec<W> {
        let (seq, c) = cmd.get();
        // HEIC/AVIF: пока готовится JPEG-копия — заглушка.
        converted.get();
        let Some(path) = shown_notify(&src, converted) else {
            return vec![boxed(DecoratedBox::new().class("wall-thumb-loading wall-editor-view"))];
        };
        vec![boxed(
            ImageViewport::new(path.to_string_lossy().to_string())
                .crop_mode(true)
                .max_scale(8.0)
                .crop(zoom, center[0], center[1])
                .info(info)
                .command(seq, c)
                .class("wall-editor-view"),
        )]
    });

    // Панорама: окно выбранного стола на холсте, остальное притемнено.
    let overlay = Reactive::new(move || -> Vec<W> {
        if !pano || n < 2 {
            return vec![];
        }
        let ws = s.ws.get().min(n - 1) as f32;
        let span = wg::panorama_span(n, shift);
        let a = ws * wg::clamp_shift(shift) / span;
        let b = 1.0 / span;
        let rest = (1.0 - a - b).max(0.0);
        let mut row = Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        if a > 0.001 {
            row = row.child(DecoratedBox::new().class("wall-pano-dim").style("flex-grow", a));
        }
        row = row.child(DecoratedBox::new().class("wall-pano-window").style("flex-grow", b));
        if rest > 0.001 {
            row = row.child(DecoratedBox::new().class("wall-pano-dim").style("flex-grow", rest));
        }
        vec![boxed(row)]
    });

    // Сцена не выше окна: под ней — столы, подсказка и кнопки.
    let vh = syngui::viewport::viewport_size().get_untracked().height;
    let max_h = (vh - if narrow() { 330.0 } else { 260.0 }).max(160.0);
    let stage = DecoratedBox::new().class("wall-editor-stage").child(
        AspectRatio::new(view.0 / view.1)
            .max_height(max_h)
            .class("wall-editor-frame")
            .child(Stack::new().fit(StackFit::Expand).clip(true).child(viewport).child(overlay)),
    );

    let target = e.target;
    let title = match (pano, target) {
        (true, _) => format!("Панорама на {n} столов"),
        (false, t) => target_label(t),
    };
    let mut header = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).class("wall-editor-bar");
    // На телефоне «назад» уже есть в строке заголовка окна.
    if !narrow() {
        header = header.child(icon_button(icons::BACK, move || s.edit.set(None)));
    }
    let header = header.child(
            Column::new()
                .gap(2.0)
                .class("grow")
                .child(Text::new("Кадр обоев").class("wall-editor-title"))
                .child(Text::new(format!("{title} · {}", e.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())).max_lines(1).elide(Elide::Middle).class("row-hint")),
        );

    let zoom_label = DecoratedBox::new().class("wall-zoom-box").child(move || Text::new(format!("{:.1}×", info.get().zoom.max(1.0))).class("wall-zoom-label"));
    let zoom_row = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Button::new("").icon(icons::ZOOM_OUT).class("icon-btn").on_click(move || send(ImageViewCommand::ZoomOut)))
        .child(zoom_label)
        .child(Button::new("").icon(icons::ZOOM_IN).class("icon-btn").on_click(move || send(ImageViewCommand::ZoomIn)))
        .child(Button::new("Заполнить").icon(icons::FIT).class("btn small").on_click(move || send(ImageViewCommand::Fill)));

    let apply_path = e.path.clone();
    let actions = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Button::new("Отмена").class("btn").on_click(move || s.edit.set(None)))
        .child(Button::new("Установить").icon(icons::CHECK).class("btn primary").on_click(move || {
            let i = info.get_untracked();
            apply(target, &apply_path, if i.ready { i.zoom } else { zoom }, if i.ready { i.center } else { (center[0], center[1]) });
            s.edit.set(None);
            state::bump();
            state::toast(match target {
                Target::All => "Обои установлены".to_string(),
                Target::Workspace(ws) => format!("Обои установлены: {}", ws_name(ws)),
            });
        }));

    let footer: W = if narrow() {
        boxed(
            Column::new()
                .gap(10.0)
                .class("wall-editor-footer")
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(zoom_row))
                .child(Row::new().main_axis_alignment(MainAxisAlignment::End).child(actions)),
        )
    } else {
        boxed(Row::new().gap(12.0).class("wall-editor-footer").cross_axis_alignment(CrossAxisAlignment::Center).child(zoom_row).child(DecoratedBox::new().class("grow")).child(actions))
    };

    let hint = if narrow() {
        "Двигайте картинку пальцем, масштаб — двумя пальцами, двойное касание — ×2"
    } else {
        "Двигайте картинку мышью, масштаб — колесом или двумя пальцами, двойной щелчок — ×2"
    };
    let mut col = Column::new()
        .gap(12.0)
        .class(if narrow() { "wall-editor wall-editor-narrow grow" } else { "wall-editor grow" })
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(header)
        .child(stage);
    if pano {
        col = col.child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(ws_chips_pano(s, n)));
    }
    col = col.child(Text::new(hint).class("row-hint wall-editor-hint")).child(footer);
    boxed(EventHook::new()
        .on_key_down(move |k, _| {
            if matches!(k, Key::Escape) {
                s.edit.set(None);
                KeyReply::Handled
            } else {
                KeyReply::Ignore
            }
        })
        .child(col))
}

/// Столы под холстом панорамы: какое окно подсветить.
fn ws_chips_pano(s: Sig, n: u32) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let cur = s.ws.get();
        let mut row = Flex::new().wrap().gap(6.0);
        for i in 0..n {
            let class = if i == cur { "wall-ws-chip selected" } else { "wall-ws-chip" };
            row = row.child(GestureDetector::new().on_click(move || s.ws.set(i)).child(DecoratedBox::new().class(class).child(Text::new(ws_name(i)).class("wall-ws-label"))));
        }
        vec![boxed(row)]
    }))
}
