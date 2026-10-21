//! История буфера обмена: апплет `clipboard` на панели, команда
//! `shell clipboard` (Super+V), настройки — `[clipboard]`.
//!
//! За буфером следит [`watch`] (wlr-data-control, свой поток), записи
//! хранит [`store`], окно — [`view`]. Источник записи — программа, окно
//! которой было в фокусе, когда содержимое сменилось (или synlink — пришло
//! с другого устройства).

mod store;
mod view;
pub mod watch;

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;

pub use store::{Entry, Kind};
pub use view::view;

use crate::ctx::ShellCtx;
use watch::{Captured, Content, TEXT_MIMES};

/// Следующий id записи (картинки пишутся в рабочем потоке — id нужен там).
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct State {
    entries: RwSignal<Vec<Entry>>,
    /// Готовы новые миниатюры файлов (видео, картинки) — перерисовать.
    thumbs_rev: RwSignal<u64>,
    handle: Option<watch::Handle>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&State) -> R) -> Option<R> {
    STATE.with(|s| s.borrow().as_ref().map(f))
}

pub fn entries() -> RwSignal<Vec<Entry>> {
    with(|s| s.entries).unwrap_or_else(|| use_signal(Vec::new()))
}

pub(crate) fn thumbs_rev() -> Option<RwSignal<u64>> {
    with(|s| s.thumbs_rev)
}

/// Запустить (один раз на процесс оболочки; главный поток).
pub fn start(ctx: ShellCtx) {
    if with(|_| ()).is_some() {
        return;
    }
    apply_config(&ctx);
    let (list, next) = store::load();
    NEXT_ID.store(next, Ordering::SeqCst);
    let entries = use_signal(list);
    let thumbs_rev = use_signal(0u64);
    synshell_common::thumbs::add_notify(move || run_on_main_thread(move || thumbs_rev.set(thumbs_rev.get_untracked() + 1)));
    let handle = watch::start(|c| {
        let Some(prepared) = prepare(c) else { return };
        run_on_main_thread(move || add(ShellCtx::get(), prepared));
    });
    let handle = match handle {
        Ok(h) => Some(h),
        Err(e) => {
            log::warn!("история буфера обмена не запущена: {e:#}");
            None
        }
    };
    STATE.with(|s| *s.borrow_mut() = Some(State { entries, thumbs_rev, handle }));
}

/// Перечитан конфиг.
pub fn apply_config(ctx: &ShellCtx) {
    let c = ctx.cfg().clipboard.clone();
    watch::IMAGES.store(c.images, Ordering::Relaxed);
    watch::MAX_IMAGE.store((c.max_image_mb.max(1) as u64) << 20, Ordering::Relaxed);
    if let Some(entries) = with(|s| s.entries) {
        let mut v = entries.get_untracked();
        let before = v.len();
        store::trim(&mut v, c.max_items as usize);
        if v.len() != before {
            entries.set(v);
            save();
        }
    }
}

/// Запись, подготовленная в рабочем потоке (картинка уже на диске).
struct Prepared {
    entry: Entry,
    remote: bool,
}

fn prepare(c: Captured) -> Option<Prepared> {
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let now = now();
    let mut e = Entry { id, time: now, kind: Kind::Text, text: String::new(), html: false, image: None, source: None, pinned: false, count: 1, hash: 0 };
    match c.content {
        Content::Text { text, html } => {
            e.hash = store::hash_bytes(Kind::Text, text.as_bytes());
            e.text = text;
            if let Some(h) = html {
                store::write_html(id, &h);
                e.html = true;
            }
        }
        Content::Image { mime, data } => {
            e.kind = Kind::Image;
            e.hash = store::hash_bytes(Kind::Image, &data);
            e.image = Some(store::write_image(id, &mime, &data)?);
        }
        Content::Files { uris } => {
            let paths: Vec<String> = uris
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .filter_map(synshell_common::xdg::path_from_uri)
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            if paths.is_empty() {
                // Не файлы (ссылки) — как текст.
                e.text = uris.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect::<Vec<_>>().join("\n");
                e.hash = store::hash_bytes(Kind::Text, e.text.as_bytes());
            } else {
                e.kind = Kind::Files;
                e.text = paths.join("\n");
                e.hash = store::hash_bytes(Kind::Files, e.text.as_bytes());
            }
        }
    }
    Some(Prepared { entry: e, remote: c.remote })
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// id .desktop программы окна в фокусе.
fn focused_app(ctx: &ShellCtx) -> Option<String> {
    let id = ctx.focused.get_untracked()?;
    let w = ctx.windows.with_untracked(|ws| ws.iter().find(|w| w.id == id).cloned())?;
    if w.app_id.is_empty() {
        return None;
    }
    Some(crate::launchers::desktop_id_of(&w.app_id))
}

fn add(ctx: ShellCtx, p: Prepared) {
    let cfg = ctx.cfg().clipboard.clone();
    let Some(entries) = with(|s| s.entries) else { return };
    let mut e = p.entry;
    let discard = |e: &Entry| {
        let _ = std::fs::remove_file(e.thumb());
        let _ = std::fs::remove_file(e.image_file());
        let _ = std::fs::remove_file(e.html_file());
    };
    if !cfg.history {
        discard(&e);
        return;
    }
    e.source = if p.remote { Some("synlink".into()) } else { focused_app(&ctx) };
    if let Some(src) = &e.source {
        let app_id = ctx.focused.get_untracked().and_then(|id| ctx.windows.with_untracked(|ws| ws.iter().find(|w| w.id == id).map(|w| w.app_id.clone())));
        if cfg.ignore_apps.iter().any(|a| a == src || Some(a) == app_id.as_ref()) {
            discard(&e);
            return;
        }
    }
    let mut v = entries.get_untracked();
    // Та же, что последняя, — это не новое копирование (оболочку перезапустили
    // и буфер пришёл заново, источник переобъявил выбор).
    if v.first().is_some_and(|x| x.hash == e.hash && x.kind == e.kind) {
        discard(&e);
        return;
    }
    if let Some(i) = v.iter().position(|x| x.hash == e.hash && x.kind == e.kind) {
        // То же самое ещё раз — наверх, без второй записи.
        discard(&e);
        let mut old = v.remove(i);
        old.time = e.time;
        old.count = old.count.saturating_add(1);
        if e.source.is_some() {
            old.source = e.source;
        }
        v.insert(0, old);
    } else {
        v.insert(0, e);
    }
    store::trim(&mut v, cfg.max_items as usize);
    entries.set(v);
    save();
}

fn save() {
    if let Some(entries) = with(|s| s.entries) {
        entries.with_untracked(|v| store::save(v, NEXT_ID.load(Ordering::SeqCst)));
    }
}

/// Снова положить запись в буфер (и поднять её наверх).
pub fn activate(id: u64) {
    let Some(entries) = with(|s| s.entries) else { return };
    let Some(e) = entries.with_untracked(|v| v.iter().find(|e| e.id == id).cloned()) else { return };
    let offer = offer_of(&e);
    if offer.is_empty() {
        return;
    }
    match with(|s| s.handle.clone()).flatten() {
        Some(h) => h.set(offer),
        None => {
            // Без data-control — хотя бы текст.
            syngui::clipboard::copy(&e.text);
        }
    }
    let mut v = entries.get_untracked();
    if let Some(i) = v.iter().position(|x| x.id == id) {
        let mut x = v.remove(i);
        x.time = now();
        v.insert(0, x);
    }
    entries.set(v);
    save();
}

/// Форматы записи для источника.
fn offer_of(e: &Entry) -> watch::Offer {
    let mut o: watch::Offer = Vec::new();
    match e.kind {
        Kind::Text => {
            let t = Arc::new(e.text.clone().into_bytes());
            for m in TEXT_MIMES {
                o.push((m.to_string(), t.clone()));
            }
            if e.html {
                if let Ok(h) = std::fs::read(e.html_file()) {
                    o.push(("text/html".into(), Arc::new(h)));
                }
            }
        }
        Kind::Image => {
            if let (Some(info), Ok(data)) = (&e.image, std::fs::read(e.image_file())) {
                o.push((info.mime.clone(), Arc::new(data)));
                // Не PNG и не уменьшалась — её копия в PNG тоже годится.
                if info.mime != "image/png" && info.width.max(info.height) <= store::THUMB {
                    if let Ok(png) = std::fs::read(e.thumb()) {
                        o.push(("image/png".into(), Arc::new(png)));
                    }
                }
            }
        }
        Kind::Files => {
            let paths = e.paths();
            let uris: Vec<String> = paths.iter().map(|p| synshell_common::xdg::file_uri(p)).collect();
            let list = Arc::new(uris.join("\r\n").into_bytes());
            o.push(("text/uri-list".into(), list));
            o.push(("x-special/gnome-copied-files".into(), Arc::new(format!("copy\n{}", uris.join("\n")).into_bytes())));
            let t = Arc::new(e.text.clone().into_bytes());
            for m in TEXT_MIMES {
                o.push((m.to_string(), t.clone()));
            }
        }
    }
    o
}

pub fn remove(id: u64) {
    let Some(entries) = with(|s| s.entries) else { return };
    let mut v = entries.get_untracked();
    store::remove(&mut v, id);
    entries.set(v);
    save();
}

pub fn toggle_pin(id: u64) {
    let Some(entries) = with(|s| s.entries) else { return };
    let mut v = entries.get_untracked();
    if let Some(e) = v.iter_mut().find(|e| e.id == id) {
        e.pinned = !e.pinned;
    }
    entries.set(v);
    save();
}

pub fn clear() {
    let Some(entries) = with(|s| s.entries) else { return };
    let mut v = entries.get_untracked();
    store::clear(&mut v);
    entries.set(v);
    save();
}

// ─── Что за текст ────────────────────────────────────────────────────────────

/// Вид текстовой записи — для окна и фильтров.
#[derive(Debug, Clone, PartialEq)]
pub enum TextKind {
    Link(String),
    /// Цвет: RGBA.
    Color([u8; 4]),
    Markdown,
    Code,
    Plain,
}

pub fn text_kind(t: &str) -> TextKind {
    let s = t.trim();
    if !s.contains(char::is_whitespace) {
        let low = s.to_ascii_lowercase();
        if low.starts_with("http://") || low.starts_with("https://") || low.starts_with("www.") || low.starts_with("ftp://") || low.starts_with("mailto:") {
            return TextKind::Link(s.to_string());
        }
    }
    if let Some(c) = parse_color(s) {
        return TextKind::Color(c);
    }
    if looks_markdown(t) {
        return TextKind::Markdown;
    }
    if looks_code(t) {
        return TextKind::Code;
    }
    TextKind::Plain
}

fn parse_color(s: &str) -> Option<[u8; 4]> {
    let h = s.strip_prefix('#')?;
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let n = |i: usize, len: usize| u8::from_str_radix(&h[i..i + len], 16).ok();
    let d = |x: u8| x * 17;
    match h.len() {
        3 => Some([d(n(0, 1)?), d(n(1, 1)?), d(n(2, 1)?), 255]),
        4 => Some([d(n(0, 1)?), d(n(1, 1)?), d(n(2, 1)?), d(n(3, 1)?)]),
        6 => Some([n(0, 2)?, n(2, 2)?, n(4, 2)?, 255]),
        8 => Some([n(0, 2)?, n(2, 2)?, n(4, 2)?, n(6, 2)?]),
        _ => None,
    }
}

fn looks_markdown(t: &str) -> bool {
    let mut score = 0;
    let mut lines = 0;
    for l in t.lines().take(200) {
        let l = l.trim_start();
        lines += 1;
        if l.starts_with("```") || l.starts_with("# ") || l.starts_with("## ") || l.starts_with("### ") {
            score += 3;
        } else if l.starts_with("- [ ] ") || l.starts_with("- [x] ") || l.starts_with("|---") || l.starts_with("| --") || l.starts_with("> ") {
            score += 2;
        } else if l.starts_with("- ") || l.starts_with("* ") || l.starts_with("+ ") || l.split_once(". ").is_some_and(|(n, _)| !n.is_empty() && n.len() <= 3 && n.chars().all(|c| c.is_ascii_digit())) {
            score += 1;
        }
        if l.contains("**") || l.contains("](") || (l.matches('`').count() >= 2 && !l.starts_with("```")) {
            score += 1;
        }
    }
    score >= 3 || (lines == 1 && score >= 2)
}

fn looks_code(t: &str) -> bool {
    let lines: Vec<&str> = t.lines().filter(|l| !l.trim().is_empty()).take(200).collect();
    if lines.len() < 2 {
        return false;
    }
    let codey = lines
        .iter()
        .filter(|l| {
            let s = l.trim_end();
            s.ends_with(';') || s.ends_with('{') || s.ends_with('}') || s.ends_with(')') || s.ends_with(':') && l.starts_with("    ") || s.starts_with("    ") || s.starts_with('\t')
        })
        .count();
    codey * 2 >= lines.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(text_kind("https://example.com/a?b"), TextKind::Link("https://example.com/a?b".into()));
        assert_eq!(text_kind("#ff8800"), TextKind::Color([255, 136, 0, 255]));
        assert_eq!(text_kind("#f80"), TextKind::Color([255, 136, 0, 255]));
        assert_eq!(text_kind("# Заголовок\n\n- один\n- два"), TextKind::Markdown);
        assert_eq!(text_kind("fn main() {\n    println!(\"hi\");\n}"), TextKind::Code);
        assert_eq!(text_kind("Просто строка текста"), TextKind::Plain);
        assert_eq!(text_kind("/home/user/Проекты"), TextKind::Plain);
    }
}
