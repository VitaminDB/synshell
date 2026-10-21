//! Окно истории буфера: поиск, фильтры, карточки записей.
//!
//! Карточка показывает запись так, как её удобнее узнать: Markdown —
//! отрисованным, код — моноширинным, ссылку — доменом, цвет — образцом,
//! картинку — самой картинкой, видео — кадром с кнопкой, ярлык приложения —
//! его значком, файлы — миниатюрами или значками типа. Сверху — откуда
//! скопировано (значок и имя программы) и когда.
//!
//! Щелчок (Enter) — запись снова в буфере, окно закрывается; средняя
//! кнопка (Shift+Delete) — удалить; булавка — закрепить (закреплённые не
//! вытесняются и не очищаются).

use std::path::{Path, PathBuf};

use syngui::input::{Key, MouseButton};
use syngui::prelude::*;
use syngui::StyleValue;
use syngui::widgets::visual::MarkdownView;
use syngui_layer::KeyInfo;
use synshell_common::{mime, xdg};

use super::{entries, text_kind, thumbs_rev, Entry, Kind, TextKind};
use crate::ctx::ShellCtx;
use crate::ui::{icon, rx, InputArea};

mod gl {
    pub const CLIPBOARD: &str = "\u{E14F}";
    pub const PIN: &str = "\u{F10D}";
    pub const DELETE: &str = "\u{E872}";
    pub const SWEEP: &str = "\u{E16C}";
    pub const SEARCH: &str = "\u{E8B6}";
    pub const LINK: &str = "\u{E157}";
    pub const IMAGE: &str = "\u{E3F4}";
    pub const FILE: &str = "\u{E873}";
    pub const FOLDER: &str = "\u{E2C7}";
    pub const PLAY: &str = "\u{E037}";
    pub const PHONE: &str = "\u{E32C}";
    pub const OPEN: &str = "\u{E89E}";
    pub const APPS: &str = "\u{E5C3}";
}

/// Ширина содержимого карточки (окно 460 минус поля) — под неё
/// рассчитывается высота картинок.
const INNER_W: f32 = 404.0;

#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Text,
    Links,
    Images,
    Files,
    Pinned,
}

impl Filter {
    const ALL: [Filter; 6] = [Filter::All, Filter::Text, Filter::Links, Filter::Images, Filter::Files, Filter::Pinned];

    fn label(self) -> String {
        match self {
            Filter::All => t!("Все"),
            Filter::Text => t!("Текст"),
            Filter::Links => t!("Ссылки"),
            Filter::Images => t!("Картинки"),
            Filter::Files => t!("Файлы"),
            Filter::Pinned => t!("Закреплённые"),
        }
        .into()
    }

    fn accepts(self, e: &Entry) -> bool {
        match self {
            Filter::All => true,
            Filter::Pinned => e.pinned,
            Filter::Text => e.kind == Kind::Text && !matches!(text_kind(&e.text), TextKind::Link(_)),
            Filter::Links => e.kind == Kind::Text && matches!(text_kind(&e.text), TextKind::Link(_)),
            Filter::Images => e.kind == Kind::Image || (e.kind == Kind::Files && e.paths().iter().all(|p| media_mime(p).is_some())),
            Filter::Files => e.kind == Kind::Files,
        }
    }
}

/// MIME картинки или видео по имени (иначе `None`).
fn media_mime(p: &Path) -> Option<String> {
    let m = mime::guess_by_name(&p.file_name()?.to_string_lossy())?;
    (m.starts_with("image/") || m.starts_with("video/")).then_some(m)
}

fn matches(e: &Entry, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    let q = q.to_lowercase();
    e.text.to_lowercase().contains(&q) || e.source.as_deref().and_then(xdg::app_by_id).is_some_and(|a| a.name.to_lowercase().contains(&q))
}

pub fn view(ctx: ShellCtx) -> impl Widget {
    let query = use_signal(String::new());
    let filter = use_signal(Filter::All);
    let selected = use_signal(0usize);
    let list = entries();
    let visible = move || -> Vec<Entry> {
        let q = query.get_untracked();
        let f = filter.get_untracked();
        list.get_untracked().into_iter().filter(|e| f.accepts(e) && matches(e, q.trim())).collect()
    };
    crate::popup::set_key_handler(move |k: &KeyInfo| {
        if !k.pressed {
            return false;
        }
        let v = visible();
        let n = v.len();
        match k.key {
            Key::Down if n > 0 => {
                let i = (selected.get_untracked() + 1).min(n - 1);
                selected.set(i);
                syngui::widgets::reveal_named(card_name(v[i].id));
                true
            }
            Key::Up if n > 0 => {
                let i = selected.get_untracked().saturating_sub(1);
                selected.set(i);
                syngui::widgets::reveal_named(card_name(v[i].id));
                true
            }
            Key::Enter => {
                if let Some(e) = v.get(selected.get_untracked()) {
                    pick(e.id);
                }
                true
            }
            Key::Delete if k.modifiers.shift => {
                if let Some(e) = v.get(selected.get_untracked()) {
                    super::remove(e.id);
                    selected.set(selected.get_untracked().min(n.saturating_sub(2)));
                }
                true
            }
            Key::Tab => {
                let i = Filter::ALL.iter().position(|f| *f == filter.get_untracked()).unwrap_or(0);
                let next = if k.modifiers.shift { (i + Filter::ALL.len() - 1) % Filter::ALL.len() } else { (i + 1) % Filter::ALL.len() };
                filter.set(Filter::ALL[next]);
                selected.set(0);
                true
            }
            _ => false,
        }
    });

    let header = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("clip-head-badge").child(icon(gl::CLIPBOARD).class("clip-head-icon")))
        .child(
            Column::new()
                .gap(1.0)
                .class("grow")
                .child(Text::new(t!("Буфер обмена")).class("popup-title"))
                .child(rx(move || {
                    let v = list.get();
                    let pinned = v.iter().filter(|e| e.pinned).count();
                    let s = if pinned > 0 { t!("Записей: {n} · закреплено: {p}", n = v.len(), p = pinned) } else { t!("Записей: {n}", n = v.len()) };
                    Box::new(Text::new(s).class("clip-subtitle"))
                })),
        )
        .child(
            InputArea::new(DecoratedBox::new().class("clip-clear").child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(gl::SWEEP).class("clip-clear-icon")).child(Text::new(t!("Очистить")).class("clip-clear-label"))))
                .pointer()
                .on_click(|b, _, _| {
                    if b == MouseButton::Left {
                        super::clear();
                    }
                }),
        );

    let search = DecoratedBox::new()
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(icon(gl::SEARCH).class("search-icon"))
                .child(
                    TextField::new()
                        .placeholder(t!("Поиск в истории"))
                        .autofocus(!ctx.is_phone())
                        .on_change(move |t| {
                            query.set(t.to_string());
                            selected.set(0);
                        })
                        .class("search-field grow"),
                ),
        )
        .class("search-box");

    let chips = rx(move || {
        let cur = filter.get();
        let mut row = Row::new().gap(5.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for f in Filter::ALL {
            // Закреплённые — одной булавкой: шесть подписей в строку не входят.
            let label: Box<dyn Widget> = if f == Filter::Pinned {
                Box::new(icon(gl::PIN).class("clip-chip-icon"))
            } else {
                Box::new(Text::new(f.label()).class("clip-chip-text"))
            };
            row = row.child(
                InputArea::new(DecoratedBox::new().class(if f == cur { "clip-chip clip-chip-on" } else { "clip-chip" }).child(label))
                    .pointer()
                    .on_click(move |b, _, _| {
                        if b == MouseButton::Left {
                            filter.set(f);
                            selected.set(0);
                        }
                    }),
            );
        }
        Box::new(row)
    });

    let items = rx(move || {
        let q = query.get();
        let f = filter.get();
        let sel = selected.get();
        if let Some(r) = thumbs_rev() {
            r.get();
        }
        let now = ctx.now.get();
        let v: Vec<Entry> = list.get().into_iter().filter(|e| f.accepts(e) && matches(e, q.trim())).collect();
        if v.is_empty() {
            let (title, hint) = if list.with_untracked(|l| l.is_empty()) {
                (t!("Здесь будет всё, что вы скопируете"), t!("Текст, ссылки, картинки и файлы — Super+V откроет историю"))
            } else {
                (t!("Ничего не нашлось"), t!("Попробуйте другой запрос или фильтр"))
            };
            return Box::new(
                Column::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class("clip-empty")
                    .child(icon(gl::CLIPBOARD).class("clip-empty-icon"))
                    .child(Text::new(title).class("clip-empty-title"))
                    .child(Text::new(hint).class("clip-empty-hint")),
            );
        }
        let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for (i, e) in v.into_iter().enumerate() {
            let name = card_name(e.id);
            col = col.child(syngui::widgets::Named::new(name, card(e, i == sel, now)));
        }
        Box::new(col)
    });

    let h = if ctx.is_phone() { 460.0 } else { 540.0 };
    Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(header)
        .child(search)
        .child(chips)
        .child(DecoratedBox::new().child(ScrollView::new().vertical().child(items).class("clip-scroll")).style("height", StyleValue::px(h)))
        .child(Text::new(t!("Enter — в буфер · ↑↓ — выбор · Tab — фильтр · Shift+Del — удалить")).class("clip-footer"))
}

fn card_name(id: u64) -> String {
    format!("clip-card-{id}")
}

/// Запись снова в буфер, окно закрыть.
fn pick(id: u64) {
    super::activate(id);
    ShellCtx::get().close_popup();
}

fn ago(now: i64, then: i64) -> String {
    let s = (now - then).max(0);
    match s {
        0..=44 => t!("только что").into(),
        45..=3599 => t!("{n} мин назад", n = (s / 60).max(1)),
        3600..=86399 => t!("{n} ч назад", n = s / 3600),
        86400..=172799 => t!("вчера").into(),
        _ => crate::clock::format(then, "%e %b"),
    }
}

/// Значок и имя источника.
fn source_of(e: &Entry) -> Option<(Box<dyn Widget>, String)> {
    let src = e.source.as_deref()?;
    if src == "synlink" {
        return Some((Box::new(icon(gl::PHONE).class("clip-src-glyph")), t!("Другое устройство").into()));
    }
    let app = xdg::app_by_id(src)?;
    let l = crate::launchers::Launchable::from_entry(&app);
    Some((crate::launchers::icon_widget(&l.icon, &None, "clip-src-icon", 16.0), app.name))
}

fn kind_label(e: &Entry) -> String {
    match e.kind {
        Kind::Image => t!("Картинка").into(),
        Kind::Files => {
            let n = e.paths().len();
            if n == 1 {
                let p = &e.paths()[0];
                if p.extension().is_some_and(|x| x == "desktop") {
                    t!("Приложение").into()
                } else if p.is_dir() {
                    t!("Папка").into()
                } else if let Some(m) = media_mime(p) {
                    if m.starts_with("video/") { t!("Видео").into() } else { t!("Картинка").into() }
                } else {
                    t!("Файл").into()
                }
            } else {
                t!("Файлов: {n}", n = n)
            }
        }
        Kind::Text => match text_kind(&e.text) {
            TextKind::Link(_) => t!("Ссылка").into(),
            TextKind::Color(_) => t!("Цвет").into(),
            TextKind::Markdown => "Markdown".into(),
            TextKind::Code => t!("Код").into(),
            TextKind::Plain => {
                let n = e.text.chars().count();
                t!("Текст · {n} симв.", n = n)
            }
        },
    }
}

fn action(glyph: &'static str, cls: &'static str, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    InputArea::new(DecoratedBox::new().class(format!("clip-act {cls}")).child(icon(glyph).class("clip-act-icon")))
        .pointer()
        .absorb()
        .on_click(move |b, _, _| {
            if b == MouseButton::Left {
                f();
            }
        })
}

fn card(e: Entry, selected: bool, now: i64) -> impl Widget {
    let id = e.id;
    let mut head = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
    let src_name = match source_of(&e) {
        Some((w, name)) => {
            head = head.child(w);
            Some(name)
        }
        None => None,
    };
    let mut meta = String::new();
    if let Some(n) = src_name {
        meta.push_str(&n);
        meta.push_str(" · ");
    }
    meta.push_str(&ago(now, e.time));
    head = head
        .child(Text::new(meta).max_lines(1).class("clip-meta grow"))
        .child(DecoratedBox::new().class("clip-kind").child(Text::new(kind_label(&e)).class("clip-kind-text")));
    if e.count > 1 {
        head = head.child(Text::new(format!("×{}", e.count)).class("clip-count"));
    }
    if let Some(target) = open_target(&e) {
        head = head.child(action(gl::OPEN, "clip-act-open", move || {
            crate::actions::spawn(&format!("xdg-open {}", quote(&target)));
            ShellCtx::get().close_popup();
        }));
    }
    head = head
        .child(action(gl::PIN, if e.pinned { "clip-act-pin clip-act-on" } else { "clip-act-pin" }, move || super::toggle_pin(id)))
        .child(action(gl::DELETE, "clip-act-del", move || super::remove(id)));

    let body = preview(&e);
    let mut cls = String::from("clip-card");
    if selected {
        cls.push_str(" clip-card-sel");
    }
    if e.pinned {
        cls.push_str(" clip-card-pinned");
    }
    InputArea::new(DecoratedBox::new().class(cls).child(Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head).child(body)))
        .pointer()
        .absorb()
        .on_click(move |b, _, _| match b {
            MouseButton::Left => pick(id),
            MouseButton::Middle => super::remove(id),
            _ => {}
        })
}

/// Что открыть кнопкой «Открыть»: ссылка или единственный файл.
fn open_target(e: &Entry) -> Option<String> {
    match e.kind {
        Kind::Text => match text_kind(&e.text) {
            TextKind::Link(u) if u.starts_with("www.") => Some(format!("https://{u}")),
            TextKind::Link(u) => Some(u),
            _ => None,
        },
        Kind::Files => {
            let p = e.paths();
            (p.len() == 1 && p[0].exists()).then(|| p[0].to_string_lossy().into_owned())
        }
        Kind::Image => None,
    }
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn preview(e: &Entry) -> Box<dyn Widget> {
    match e.kind {
        Kind::Text => text_preview(&e.text),
        Kind::Image => image_preview(e),
        Kind::Files => files_preview(&e.paths()),
    }
}

/// Первые `lines` строк (и не больше `chars` символов).
fn head_of(t: &str, lines: usize, chars: usize) -> (String, bool) {
    let mut out = String::new();
    let mut cut = false;
    for (i, l) in t.lines().enumerate() {
        if i >= lines || out.len() + l.len() > chars {
            cut = true;
            break;
        }
        if i > 0 {
            out.push('\n');
        }
        out.push_str(l);
    }
    if out.is_empty() {
        out = t.chars().take(chars).collect();
        cut = t.chars().count() > chars;
    }
    (out, cut)
}

fn text_preview(t: &str) -> Box<dyn Widget> {
    match text_kind(t) {
        TextKind::Link(u) => {
            let rest = u.split_once("://").map(|(_, r)| r).unwrap_or(&u);
            let host = rest.split(['/', '?', '#']).next().unwrap_or(rest).trim_start_matches("www.").to_string();
            Box::new(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class("clip-link-badge").child(icon(gl::LINK).class("clip-link-icon")))
                    .child(Column::new().gap(2.0).class("grow").child(Text::new(host).max_lines(1).class("clip-link-host")).child(Text::new(u.clone()).max_lines(2).class("clip-link-url"))),
            )
        }
        TextKind::Color(c) => {
            let col = syngui::core::Color::from_srgb(c[0], c[1], c[2], c[3] as f32 / 255.0);
            let rgb = if c[3] == 255 { format!("rgb({}, {}, {})", c[0], c[1], c[2]) } else { format!("rgba({}, {}, {}, {:.2})", c[0], c[1], c[2], c[3] as f32 / 255.0) };
            Box::new(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class("clip-swatch").style("background-color", col))
                    .child(Column::new().gap(2.0).child(Text::new(t.trim().to_string()).class("clip-color-hex")).child(Text::new(rgb).class("clip-color-rgb"))),
            )
        }
        TextKind::Markdown => {
            let (src, cut) = head_of(t, 14, 1600);
            let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(DecoratedBox::new().class("clip-md").clip(true).child(MarkdownView::new(src).class("clip-md-view")));
            if cut {
                col = col.child(Text::new("…").class("clip-more"));
            }
            Box::new(col)
        }
        TextKind::Code => {
            let (src, cut) = head_of(t, 8, 900);
            let src = if cut { format!("{src}\n…") } else { src };
            Box::new(DecoratedBox::new().class("clip-code-box").clip(true).child(Text::new(src).class("clip-code")))
        }
        TextKind::Plain => {
            let (src, _) = head_of(t.trim(), 6, 600);
            Box::new(Text::new(src).max_lines(5).class("clip-text"))
        }
    }
}

/// Высота картинки шириной [`INNER_W`].
fn fit_height(w: u32, h: u32, max: f32) -> f32 {
    if w == 0 || h == 0 {
        return max;
    }
    (INNER_W * h as f32 / w as f32).clamp(56.0, max)
}

fn image_preview(e: &Entry) -> Box<dyn Widget> {
    let Some(info) = &e.image else { return Box::new(Text::new(t!("Картинка")).class("clip-text")) };
    let h = fit_height(info.width, info.height, 240.0);
    let fmt = info.mime.trim_start_matches("image/").to_uppercase();
    Box::new(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                DecoratedBox::new()
                    .class("clip-image-box")
                    .clip(true)
                    .style("height", StyleValue::px(h))
                    .child(Image::new(e.thumb().to_string_lossy()).fit(ImageFit::Contain).placeholder(false).class("clip-image")),
            )
            .child(Text::new(format!("{}×{} · {} · {}", info.width, info.height, fmt, synshell_common::drives::format_size(info.bytes))).class("clip-dim")),
    )
}

fn mtime(p: &Path) -> i64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Миниатюра файла (картинка, видео), пока не готова — `None`.
fn thumb(p: &Path, mime: &str, size: u32) -> Option<PathBuf> {
    synshell_common::thumbs::get(p, mime, mtime(p), size)
}

/// Ярлык `.desktop` — запись приложения.
fn desktop_entry(p: &Path) -> Option<xdg::DesktopEntry> {
    if p.extension().is_none_or(|x| x != "desktop") {
        return None;
    }
    let id = p.file_stem()?.to_string_lossy().into_owned();
    xdg::parse_desktop_file(p, id, &[])
}

/// Квадратная плитка файла: миниатюра, значок приложения или значок типа.
fn tile(p: &Path, size: f32) -> Box<dyn Widget> {
    let wrap = |w: Box<dyn Widget>| -> Box<dyn Widget> {
        Box::new(DecoratedBox::new().class("clip-tile").clip(true).style("width", StyleValue::px(size)).style("height", StyleValue::px(size)).child(w))
    };
    if let Some(app) = desktop_entry(p) {
        let l = crate::launchers::Launchable::from_entry(&app);
        return wrap(Box::new(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(crate::launchers::icon_widget(&l.icon, &None, "clip-tile-app", size * 0.7))));
    }
    if let Some(m) = media_mime(p) {
        if let Some(t) = thumb(p, &m, 128) {
            let img: Box<dyn Widget> = Box::new(Image::new(t.to_string_lossy()).fit(ImageFit::Cover).placeholder(false).style("width", StyleValue::px(size)).style("height", StyleValue::px(size)));
            if m.starts_with("video/") {
                return wrap(Box::new(Stack::new().child(img).child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("clip-play small").child(icon(gl::PLAY).class("clip-play-icon"))).style("width", StyleValue::px(size)).style("height", StyleValue::px(size)))));
            }
            return wrap(img);
        }
    }
    let glyph_or_icon: Box<dyn Widget> = if p.is_dir() {
        Box::new(icon(gl::FOLDER).class("clip-tile-glyph"))
    } else {
        let m = if p.exists() { mime::detect(p) } else { mime::guess_by_name(&p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).unwrap_or_default() };
        match mime::icon_path(&m) {
            Some(ip) => Box::new(Image::new(ip.to_string_lossy()).fit(ImageFit::Contain).placeholder(false).style("width", StyleValue::px(size * 0.66)).style("height", StyleValue::px(size * 0.66))),
            None => Box::new(icon(if m.starts_with("image/") { gl::IMAGE } else { gl::FILE }).class("clip-tile-glyph")),
        }
    };
    wrap(Box::new(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(glyph_or_icon)))
}

fn file_meta(p: &Path) -> String {
    if !p.exists() {
        return t!("Файла больше нет").into();
    }
    if p.is_dir() {
        return t!("Папка").into();
    }
    let m = mime::detect(p);
    let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    format!("{} · {}", mime::description(&m), synshell_common::drives::format_size(size))
}

fn files_preview(paths: &[PathBuf]) -> Box<dyn Widget> {
    if paths.len() == 1 {
        let p = &paths[0];
        // Приложение — значок и имя.
        if let Some(app) = desktop_entry(p) {
            let l = crate::launchers::Launchable::from_entry(&app);
            return Box::new(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class("clip-app-plate").child(crate::launchers::icon_widget(&l.icon, &Some(gl::APPS.to_string()).filter(|_| l.icon.is_none()), "clip-app-icon", 40.0)))
                    .child(Column::new().gap(2.0).class("grow").child(Text::new(app.name.clone()).max_lines(1).class("clip-file-name")).child(Text::new(if app.comment.is_empty() { p.to_string_lossy().into_owned() } else { app.comment.clone() }).max_lines(2).class("clip-file-dir"))),
            );
        }
        // Картинка или видео — крупно.
        if let Some(m) = media_mime(p) {
            if let Some(t) = thumb(p, &m, 512) {
                let (w, h) = image::image_dimensions(&t).unwrap_or((16, 9));
                let hgt = fit_height(w, h, 220.0);
                let img: Box<dyn Widget> = Box::new(Image::new(t.to_string_lossy()).fit(ImageFit::Contain).placeholder(false).class("clip-image"));
                let pic: Box<dyn Widget> = if m.starts_with("video/") {
                    Box::new(Stack::new().child(img).child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).class("clip-image").child(DecoratedBox::new().class("clip-play").child(icon(gl::PLAY).class("clip-play-icon")))))
                } else {
                    img
                };
                return Box::new(
                    Column::new()
                        .gap(6.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(DecoratedBox::new().class("clip-image-box").clip(true).style("height", StyleValue::px(hgt)).child(pic))
                        .child(name_row(p)),
                );
            }
        }
        return Box::new(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(tile(p, 48.0)).child(name_row(p)));
    }
    let mut tiles = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for p in paths.iter().take(5) {
        tiles = tiles.child(tile(p, 56.0));
    }
    if paths.len() > 5 {
        tiles = tiles.child(DecoratedBox::new().class("clip-tile clip-tile-more").style("width", StyleValue::px(56.0)).style("height", StyleValue::px(56.0)).child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(format!("+{}", paths.len() - 5)).class("clip-more-count"))));
    }
    let names = paths.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ");
    Box::new(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(tiles).child(Text::new(names).max_lines(2).class("clip-file-dir")))
}

fn name_row(p: &Path) -> impl Widget {
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned());
    let dir = p.parent().map(|d| d.to_string_lossy().replace(&synshell_common::paths::home().to_string_lossy().into_owned(), "~")).unwrap_or_default();
    Column::new()
        .gap(2.0)
        .class("grow")
        .child(Text::new(name).max_lines(1).class(if p.exists() { "clip-file-name" } else { "clip-file-name clip-gone" }))
        .child(Text::new(format!("{} · {}", file_meta(p), dir)).max_lines(1).class("clip-file-dir"))
}
