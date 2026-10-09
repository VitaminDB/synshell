//! Режим выбора файлов — окно портала `org.freedesktop.impl.portal.FileChooser` оболочки
//! (synsystem::portal_files): `synfiles --choose '<JSON запроса>'`.
//!
//! Запрос: `{"mode":"open"|"save"|"save_files","title","accept_label","multiple","directory",
//! "filters":[{"name","patterns":[[0,"*.jpg"],[1,"image/*"]]}],"current_filter","current_name",
//! "current_folder","files":[имена]}` (0 — маска имени, 1 — тип MIME, как у портала).
//! Ответ — одна строка JSON в stdout: `{"response":0,"uris":[…],"current_filter":N}`; отмена —
//! `{"response":1}`. Окно закрывается сразу после ответа.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;
use syngui::prelude::*;
use syngui::widgets::*;

use crate::model::Entry;
use crate::state::{self, Pane};
use crate::ui::{boxed, icons, W};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Open,
    Save,
    SaveFiles,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Filter {
    pub name: String,
    /// (0 — маска имени, 1 — тип MIME)
    pub patterns: Vec<(u32, String)>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Request {
    pub mode: Mode,
    pub title: String,
    pub accept_label: String,
    pub multiple: bool,
    pub directory: bool,
    pub filters: Vec<Filter>,
    pub current_filter: Option<usize>,
    pub current_name: String,
    pub current_folder: String,
    pub files: Vec<String>,
}

static REQ: OnceLock<Request> = OnceLock::new();

thread_local! {
    static SIG: std::cell::Cell<Option<(RwSignal<usize>, RwSignal<String>, RwSignal<bool>)>> = const { std::cell::Cell::new(None) };
}

/// Разобрать запрос из аргумента `--choose`.
pub fn init(json: &str) -> std::result::Result<(), String> {
    let r: Request = serde_json::from_str(json).map_err(|e| t!("запрос выбора файлов: {e}", e = e))?;
    let _ = REQ.set(r);
    Ok(())
}

pub fn active() -> Option<&'static Request> {
    REQ.get()
}

/// Сигналы окна выбора: текущий фильтр, имя файла (сохранение), «заменить существующий?».
fn sig() -> (RwSignal<usize>, RwSignal<String>, RwSignal<bool>) {
    SIG.with(|s| {
        if let Some(v) = s.get() {
            return v;
        }
        let r = active().cloned().unwrap_or_default();
        let v = (use_signal(r.current_filter.unwrap_or(0)), use_signal(r.current_name.clone()), use_signal(false));
        s.set(Some(v));
        v
    })
}

/// Заголовок окна.
pub fn title() -> String {
    let Some(r) = active() else { return String::new() };
    if !r.title.is_empty() {
        return r.title.clone();
    }
    match (r.mode, r.directory) {
        (Mode::Open, true) => t!("Выбор папки").into(),
        (Mode::Open, false) if r.multiple => t!("Выбор файлов").into(),
        (Mode::Open, false) => t!("Выбор файла").into(),
        _ => t!("Сохранение").into(),
    }
}

/// Папка, с которой открывается окно.
pub fn start_dir() -> Option<PathBuf> {
    let r = active()?;
    let d = PathBuf::from(&r.current_folder);
    if !r.current_folder.is_empty() && d.is_dir() {
        return Some(d);
    }
    Some(synshell_common::paths::user_dir("DOCUMENTS").filter(|p| p.is_dir()).unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))))
}

fn glob_match(pat: &str, name: &str) -> bool {
    // * и ? без учёта регистра
    fn m(p: &[char], n: &[char]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) => m(&p[1..], n) || (!n.is_empty() && m(p, &n[1..])),
            (Some('?'), Some(_)) => m(&p[1..], &n[1..]),
            (Some(a), Some(b)) => a == b && m(&p[1..], &n[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pat.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    m(&p, &n)
}

fn mime_match(pat: &str, mime: &str) -> bool {
    match pat.strip_suffix("/*") {
        Some(major) => mime.split('/').next() == Some(major),
        None => pat == mime,
    }
}

/// Показывать ли запись в окне выбора (папки — всегда; в выборе папки — только они).
pub fn visible(e: &Entry) -> bool {
    let Some(r) = active() else { return true };
    if e.is_dir {
        return true;
    }
    if r.directory {
        return false;
    }
    let Some(f) = SIG.with(|s| s.get()).map(|s| s.0.get_untracked()).and_then(|i| r.filters.get(i)) else { return true };
    f.patterns.iter().any(|(k, p)| if *k == 1 { mime_match(p, &e.mime) } else { glob_match(p, &e.name) })
}

fn uri(p: &Path) -> String {
    let mut s = String::from("file://");
    for b in p.to_string_lossy().as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => s.push(*b as char),
            _ => s.push_str(&format!("%{b:02X}")),
        }
    }
    s
}

fn reply(json: String) -> ! {
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{json}");
    let _ = o.flush();
    std::process::exit(0)
}

pub fn cancel() -> ! {
    reply("{\"response\":1}".into())
}

fn finish(paths: &[PathBuf]) -> ! {
    let uris: Vec<String> = paths.iter().map(|p| uri(p)).collect();
    let f = sig().0.get_untracked();
    reply(serde_json::json!({"response": 0, "uris": uris, "current_filter": f}).to_string())
}

fn current_dir(p: Pane) -> Option<PathBuf> {
    p.loc.get_untracked().dir().map(Path::to_path_buf)
}

/// Касание файла: открыть — выбрать его (один) или отметить (несколько); сохранить — взять его имя.
/// true — обработано здесь.
pub fn activate(p: Pane, e: &Entry) -> bool {
    let Some(r) = active() else { return false };
    if e.is_dir {
        return false;
    }
    match r.mode {
        Mode::Open if !r.multiple => finish(&[e.path.clone()]),
        Mode::Open => {
            // несколько файлов: касание отмечает или снимает отметку
            let Some(i) = p.entries.with_untracked(|v| v.iter().position(|x| x.path == e.path)) else { return true };
            let mut sel = p.sel.get_untracked();
            if !sel.selected.remove(&i) {
                sel.selected.insert(i);
            }
            sel.cursor = Some(i);
            p.sel.set(sel);
            true
        }
        _ => {
            sig().1.set(e.name.clone());
            sig().2.set(false);
            true
        }
    }
}

/// Главная кнопка.
fn accept(p: Pane) {
    let Some(r) = active() else { return };
    let (_, name, confirm) = sig();
    let Some(dir) = current_dir(p) else {
        state::toast_error(t!("Выберите обычную папку"));
        return;
    };
    match r.mode {
        Mode::Open if r.directory => {
            // выделенная папка или текущая
            let sel: Vec<PathBuf> = state::selected_entries(p).into_iter().filter(|e| e.is_dir).map(|e| e.path).collect();
            if sel.is_empty() {
                finish(&[dir])
            } else if r.multiple {
                finish(&sel)
            } else {
                finish(&sel[..1])
            }
        }
        Mode::Open => {
            let sel = state::selected_entries(p);
            let files: Vec<PathBuf> = sel.iter().filter(|e| !e.is_dir).map(|e| e.path.clone()).collect();
            if files.is_empty() {
                // выделена только папка — войти в неё
                if let Some(d) = sel.iter().find(|e| e.is_dir) {
                    state::navigate(p, crate::loc::Location::Dir(d.path.clone()), true);
                } else {
                    state::toast(t!("Выберите файл"));
                }
                return;
            }
            if r.multiple {
                finish(&files)
            } else {
                finish(&files[..1])
            }
        }
        Mode::Save => {
            let n = name.get_untracked().trim().to_string();
            if n.is_empty() || n.contains('/') {
                state::toast_error(t!("Введите имя файла"));
                return;
            }
            let path = dir.join(&n);
            if path.is_dir() {
                state::navigate(p, crate::loc::Location::Dir(path), true);
                return;
            }
            if path.exists() && !confirm.get_untracked() {
                confirm.set(true);
                state::toast(t!("«{n}» уже есть — нажмите ещё раз, чтобы заменить", n = n));
                return;
            }
            finish(&[path])
        }
        Mode::SaveFiles => {
            let paths: Vec<PathBuf> = r.files.iter().map(|f| dir.join(f)).collect();
            if paths.is_empty() {
                finish(&[dir])
            } else {
                finish(&paths)
            }
        }
    }
}

fn accept_label(r: &Request) -> String {
    if !r.accept_label.is_empty() {
        return r.accept_label.replace('_', "");
    }
    match (r.mode, r.directory) {
        (Mode::Open, true) => t!("Выбрать").into(),
        (Mode::Open, false) => t!("Открыть").into(),
        _ => t!("Сохранить").into(),
    }
}

/// Нижняя панель окна выбора: фильтр типов, имя файла (сохранение), «Отмена» и главная кнопка.
pub fn bar() -> W {
    let Some(r) = active() else { return boxed(Column::new()) };
    let fresh = SIG.with(|s| s.get()).is_none();
    let (filter, name, confirm) = sig();
    if fresh && !r.filters.is_empty() {
        // список уже мог загрузиться без фильтра
        run_on_main_thread(state::refilter_all);
    }
    let filters = Reactive::new(move || -> Vec<W> {
        let cur = filter.get();
        if r.filters.len() < 2 {
            return vec![];
        }
        let mut row = Flex::new().wrap().gap(6.0);
        for (i, f) in r.filters.iter().enumerate() {
            row = row.child(
                GestureDetector::new()
                    .on_click(move || {
                        filter.set(i);
                        state::refilter_all();
                    })
                    .child(DecoratedBox::new().class(if i == cur { "ch-chip ch-chip-on" } else { "ch-chip" }).child(Text::new(f.name.clone()).max_lines(1).class("ch-chip-text"))),
            );
        }
        vec![boxed(row)]
    });
    let info: W = match r.mode {
        Mode::Save => boxed(
            TextField::new()
                .text(name.get_untracked())
                .placeholder(t!("Имя файла"))
                .autofocus(true)
                .on_change(move |t: &str| {
                    name.set(t.to_string());
                    confirm.set(false);
                })
                .on_submit(move |_| accept(state::pane()))
                .class("ch-name"),
        ),
        Mode::SaveFiles => boxed(Text::new(t!("Файлов: {n} — выберите папку", n = r.files.len())).max_lines(2).class("ch-info")),
        Mode::Open => boxed(Reactive::new(move || -> Vec<W> {
            let p = state::tab_tracked().pane_tracked();
            let sel = state::selected_entries_tracked(p);
            let text = if r.directory {
                match sel.iter().find(|e| e.is_dir) {
                    Some(e) => t!("Папка: {name}", name = e.name),
                    None => t!("Папка: {v}", v = p.loc.get().title()),
                }
            } else {
                let files: Vec<&Entry> = sel.iter().filter(|e| !e.is_dir).collect();
                match files.len() {
                    0 if r.multiple => t!("Отметьте файлы").into(),
                    0 => t!("Коснитесь файла").into(),
                    1 => files[0].name.clone(),
                    n => t!("Выбрано файлов: {n}", n = n),
                }
            };
            vec![boxed(Text::new(text).max_lines(1).class("ch-info"))]
        })),
    };
    let label = accept_label(r);
    boxed(
        DecoratedBox::new().class("chooser-bar").child(
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(filters)
                .child(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(DecoratedBox::new().class("grow").child(info))
                        .child(GestureDetector::new().on_click(|| cancel()).child(DecoratedBox::new().class("ch-btn").child(Text::new(t!("Отмена")).class("ch-btn-text"))))
                        .child(
                            GestureDetector::new().on_click(|| accept(state::pane())).child(
                                DecoratedBox::new().class("ch-btn ch-btn-main").child(
                                    Row::new()
                                        .gap(6.0)
                                        .cross_axis_alignment(CrossAxisAlignment::Center)
                                        .child(Icon::new(if r.mode == Mode::Open && !r.directory { icons::OPEN } else { icons::CHECK }).class("icon ch-btn-icon"))
                                        .child(Text::new(label.clone()).max_lines(1).class("ch-btn-text")),
                                ),
                            ),
                        ),
                ),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("*.JPG", "photo.jpg"));
        assert!(glob_match("img_??.png", "IMG_01.png"));
        assert!(!glob_match("*.png", "a.jpg"));
        assert!(mime_match("image/*", "image/jpeg"));
        assert!(!mime_match("image/*", "video/mp4"));
    }

    #[test]
    fn uri_escapes() {
        assert_eq!(uri(Path::new("/home/u/Изображения/a b.jpg")), "file:///home/u/%D0%98%D0%B7%D0%BE%D0%B1%D1%80%D0%B0%D0%B6%D0%B5%D0%BD%D0%B8%D1%8F/a%20b.jpg");
    }

    #[test]
    fn request_parses() {
        let r: Request = serde_json::from_str(r#"{"mode":"save","current_name":"x.txt","filters":[{"name":"Картинки","patterns":[[1,"image/*"],[0,"*.png"]]}]}"#).unwrap();
        assert_eq!(r.mode, Mode::Save);
        assert_eq!(r.filters[0].patterns[1], (0, "*.png".to_string()));
    }
}
