//! Шаблоны проектов в галерее (G3): раздел «Проекты» — плитки с кадром
//! шаблона (места заполнены цветными образцами), окно «Ролик по шаблону»:
//! места по порядку — свои файлы (выбрать, из медиатеки), тексты, музыка →
//! `template.project.use`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};
use syngui::prelude::*;
use syngui::widgets::overlay::MenuItem;
use syngui::widgets::*;

use super::{ed, St};
use crate::cmd::slots;

/// Строка окна: место шаблона и чем его заполнить.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotRow {
    pub label: String,
    /// media | text | music
    pub kind: &'static str,
    pub dur: f64,
    /// Путь файла (media/music) или текст.
    pub value: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub name: String,
    pub title: String,
    pub info: String,
    pub rows: Vec<SlotRow>,
}

// ---------- превью плиток ----------

fn cache_dir() -> PathBuf {
    crate::store::data_dir().join("cache").join("project-templates")
}

/// Цветные образцы для превью (градиенты разных оттенков).
fn demo_images() -> Vec<PathBuf> {
    let dir = cache_dir().join("demo");
    let _ = std::fs::create_dir_all(&dir);
    let hues = [(250, 120, 60), (60, 140, 230), (90, 190, 120), (200, 80, 170), (240, 190, 70), (80, 200, 210)];
    hues.iter()
        .enumerate()
        .map(|(i, &(r, g, b))| {
            let p = dir.join(format!("demo{i}.png"));
            if !p.exists() {
                let (w, h) = (640u32, 360u32);
                let img = image::RgbaImage::from_fn(w, h, |x, y| {
                    let t = x as f32 / w as f32 * 0.6 + y as f32 / h as f32 * 0.4;
                    let k = 0.35 + 0.65 * (1.0 - t);
                    let c = |v: u8| ((v as f32 * k) as u8).saturating_add((30.0 * t) as u8);
                    image::Rgba([c(r), c(g), c(b), 255])
                });
                let _ = img.save(&p);
            }
            p
        })
        .collect()
}

#[derive(Default)]
struct Thumbs {
    done: HashMap<String, PathBuf>,
    pending: Vec<String>,
}

fn thumbs() -> &'static Mutex<Thumbs> {
    static T: OnceLock<Mutex<Thumbs>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(Thumbs::default()))
}

fn thumb_key(t: &slots::ProjectTemplate) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.name.hash(&mut h);
    t.code.hash(&mut h);
    if let Some(p) = &t.path {
        p.hash(&mut h);
        std::fs::metadata(p).ok().and_then(|m| m.modified().ok()).hash(&mut h);
    }
    format!("{}-{:016x}", t.name, h.finish())
}

/// Кадр шаблона (с образцами в местах) или None — тогда он ставится в очередь.
fn thumb(st: St, t: &slots::ProjectTemplate) -> Option<PathBuf> {
    let key = thumb_key(t);
    let file = cache_dir().join(format!("{key}.png"));
    let mut g = thumbs().lock().unwrap();
    if let Some(p) = g.done.get(&key) {
        return Some(p.clone());
    }
    if file.exists() {
        g.done.insert(key, file.clone());
        return Some(file);
    }
    if g.pending.contains(&key) {
        return None;
    }
    g.pending.push(key.clone());
    let name = t.name.clone();
    std::thread::spawn(move || {
        let r = (|| -> std::result::Result<(), String> {
            let t = slots::find(&name)?;
            let mut p = slots::build(&t)?;
            let demo: Vec<String> = demo_images().iter().map(|p| p.display().to_string()).collect();
            let n = slots::slots(&p).iter().filter(|x| x.2 == "media").count();
            let files: Vec<String> = (0..n).map(|i| demo[i % demo.len()].clone()).collect();
            let _ = slots::fill_all(&mut p, &files, None, None, true)?;
            // кадр, где виден текст: начало первого текстового места + немного
            let at = slots::slots(&p).iter().find(|x| x.2 == "text").map(|x| x.0.start + (x.0.duration * 0.6).min(1.2)).unwrap_or(p.duration() * 0.3);
            let _ = std::fs::create_dir_all(cache_dir());
            let w = if p.settings.width >= p.settings.height { 384 } else { 216 };
            crate::render::run_child(&p, &["frame".into(), at.to_string(), file.display().to_string(), w.to_string()], |_, _| {})?;
            Ok(())
        })();
        if let Err(e) = &r {
            tracing::warn!("превью шаблона {name}: {e}");
        }
        let mut g = thumbs().lock().unwrap();
        g.pending.retain(|k| *k != key);
        if r.is_ok() {
            g.done.insert(key, file);
        }
        drop(g);
        syngui::async_runtime::run_on_main_thread(move || st.tpl_thumbs.update(|n| *n += 1));
    });
    None
}

// ---------- раздел галереи ----------

pub fn view(st: St, query: &str, tile: f32) -> Box<dyn Widget> {
    let _ = st.tpl_thumbs.get();
    let q = query.trim().to_lowercase();
    let list: Vec<slots::ProjectTemplate> = slots::all().into_iter().filter(|t| q.is_empty() || format!("{} {} {} {}", t.name, t.title, t.description, t.category).to_lowercase().contains(&q)).collect();
    let mut col = Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("panel-list")
        .child(Text::new(format!("Шаблонов проектов: {} · 2×клик — ролик по шаблону со своими файлами · свои — «Шаблоны → Сохранить проект как шаблон»", list.len())).max_lines(3).class("panel-hint"));
    let mut grid = Grid::new(16).min_column_width(tile.max(180.0)).gap(8.0);
    for t in &list {
        grid = grid.child(tile_widget(st, t));
    }
    col = col.child(grid);
    Box::new(col)
}

fn tile_widget(st: St, t: &slots::ProjectTemplate) -> Box<dyn Widget> {
    let info = slots::build(t).map(|p| slots::summary(&p)).unwrap_or_else(|e| e);
    let thumb_w: Box<dyn Widget> = match thumb(st, t) {
        Some(p) => Box::new(Image::new(p.display().to_string()).fit(ImageFit::Contain)),
        None => Box::new(Center::new().child(Text::new("готовлю превью…").class("graph-chain"))),
    };
    let body = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(AspectRatio::new(16.0 / 9.0).child(DecoratedBox::new().class("tile-thumb graph-thumb").child(thumb_w)))
        .child(Text::new(t.title.clone()).max_lines(1).elide(Elide::End).class("row-title"))
        .child(Text::new(t.description.clone()).max_lines(2).elide(Elide::End).class("row-sub"))
        .child(Text::new(info.clone()).max_lines(2).elide(Elide::End).class("row-sub"));
    let (n1, n2) = (t.name.clone(), t.name.clone());
    let tip = format!("{}\n\n{}\n{}", t.title, t.description, info);
    let mut menu = vec![MenuItem::new("use", "Ролик по шаблону…"), MenuItem::new("open", "Открыть пустым (заполнять на таймлайне)")];
    if t.path.is_some() {
        menu.push(MenuItem::new("folder", "Показать файл шаблона"));
    }
    let path = t.path.clone();
    Box::new(
        ContextMenu::new()
            .items(menu)
            .on_select(move |id: &str| match id {
                "use" => open(st, &n2),
                "open" => {
                    st.exec("template.project.use", json!({"name": n2}));
                }
                "folder" => {
                    if let Some(d) = path.as_ref().and_then(|p| p.parent()) {
                        let _ = std::process::Command::new("xdg-open").arg(d).spawn();
                    }
                }
                _ => {}
            })
            .child(Tooltip::new(GestureDetector::new().on_double_click(move || open(st, &n1)).child(DecoratedBox::new().class("tile").child(body)), tip).delay_ms(300).max_width(320.0)),
    )
}

// ---------- окно «Ролик по шаблону» ----------

pub fn open(st: St, name: &str) {
    let r = slots::find(name).and_then(|t| slots::build(&t).map(|p| (t, p)));
    match r {
        Ok((t, p)) => {
            let rows = slots::slots(&p)
                .iter()
                .map(|(c, _, k)| SlotRow {
                    label: c.slot.as_ref().unwrap().label.clone(),
                    kind: k,
                    dur: c.duration,
                    value: match &c.source {
                        crate::doc::Source::Title { text, .. } => text.clone(),
                        _ => String::new(),
                    },
                })
                .collect();
            st.proj_draft.set(Some(Draft { name: t.name.clone(), title: t.title.clone(), info: format!("{} · {}", t.description, slots::summary(&p)), rows }));
        }
        Err(e) => st.toast.set(Some(e)),
    }
}

fn close(st: St) {
    st.proj_draft.set(None);
}

/// Файлы — в пустые места по порядку (`kind`: media или music).
fn put_files(st: St, kind: &'static str, files: Vec<String>, from: Option<usize>) {
    st.proj_draft.update(|d| {
        let Some(d) = d else { return };
        let mut it = files.into_iter();
        match from {
            Some(i) => {
                if let (Some(r), Some(f)) = (d.rows.get_mut(i), it.next()) {
                    r.value = f;
                }
            }
            None => {
                for r in d.rows.iter_mut().filter(|r| r.kind == kind && r.value.is_empty()) {
                    match it.next() {
                        Some(f) => r.value = f,
                        None => break,
                    }
                }
            }
        }
    });
}

fn pick(st: St, kind: &'static str, multi: bool, row: Option<usize>) {
    std::thread::spawn(move || {
        let mut exts: Vec<&str> = Vec::new();
        if kind == "music" {
            exts.extend(crate::cmd::media::AUDIO);
        } else {
            exts.extend(crate::cmd::media::VIDEO);
            exts.extend(crate::cmd::media::IMAGE);
        }
        let mut d = rfd::FileDialog::new().set_title(if kind == "music" { "Музыка для шаблона" } else { "Видео и фото для шаблона" }).add_filter("Медиафайлы", &exts);
        if let Some(h) = std::env::var_os("HOME") {
            d = d.set_directory(PathBuf::from(h));
        }
        let picked: Vec<PathBuf> = if multi { d.pick_files().unwrap_or_default() } else { d.pick_file().into_iter().collect() };
        if picked.is_empty() {
            return;
        }
        let files: Vec<String> = picked.iter().map(|p| p.display().to_string()).collect();
        syngui::async_runtime::run_on_main_thread(move || put_files(st, kind, files, row));
    });
}

fn from_library(st: St) {
    let sel = st.media_sel.get_untracked();
    let e = ed();
    let mut ids: Vec<u64> = if sel.is_empty() { e.project.media.iter().map(|m| m.id).collect() } else { sel };
    ids.retain(|id| e.project.media(*id).is_some_and(|m| m.kind != crate::doc::MediaKind::Audio));
    let files: Vec<String> = ids.iter().filter_map(|id| e.project.media(*id)).map(|m| m.path.display().to_string()).collect();
    let music: Vec<String> = if sel_has_audio(&e, &st) { e.project.media.iter().filter(|m| m.kind == crate::doc::MediaKind::Audio && st.media_sel.get_untracked().contains(&m.id)).map(|m| m.path.display().to_string()).collect() } else { Vec::new() };
    drop(e);
    if files.is_empty() && music.is_empty() {
        st.toast.set(Some("В медиатеке нет видео и фото".into()));
        return;
    }
    put_files(st, "media", files, None);
    put_files(st, "music", music, None);
}

fn sel_has_audio(e: &crate::editor::Editor, st: &St) -> bool {
    st.media_sel.get_untracked().iter().any(|id| e.project.media(*id).is_some_and(|m| m.kind == crate::doc::MediaKind::Audio))
}

fn create(st: St) {
    let Some(d) = st.proj_draft.get_untracked() else { return };
    let files: Vec<Value> = d.rows.iter().filter(|r| r.kind == "media").map(|r| Value::from(r.value.clone())).collect();
    let music = d.rows.iter().find(|r| r.kind == "music" && !r.value.is_empty()).map(|r| r.value.clone());
    let texts: serde_json::Map<String, Value> = d.rows.iter().filter(|r| r.kind == "text").map(|r| (r.label.clone(), Value::from(r.value.clone()))).collect();
    let mut args = json!({"name": d.name, "files": files, "texts": texts});
    if let Some(m) = music {
        args["music"] = Value::from(m);
    }
    close(st);
    st.exec("template.project.use", args);
}

pub fn dialog(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(d) = st.proj_draft.get() else { return Vec::new() };
        vec![Box::new(
            Stack::new()
                .fit(StackFit::Expand)
                .child(GestureDetector::new().on_click(move || close(st)).child(DecoratedBox::new().class("fp-scrim")))
                .child(Center::new().child(
                    EventHook::new()
                        .on_key_down(move |k, _| {
                            if k == Key::Escape {
                                close(st);
                                return KeyReply::Handled;
                            }
                            KeyReply::Ignore
                        })
                        .child(card(st, &d)),
                )),
        )]
    })
}

fn file_name(p: &str) -> String {
    std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string())
}

fn card(st: St, d: &Draft) -> impl Widget {
    let n_media = d.rows.iter().filter(|r| r.kind == "media").count();
    let filled = d.rows.iter().filter(|r| r.kind == "media" && !r.value.is_empty()).count();
    let head = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(format!("Ролик по шаблону «{}»", d.title)).class("fp-title"))
        .child(DecoratedBox::new().class("grow"))
        .child(ToolButton::new("\u{E5CD}").tooltip("Закрыть (Esc)").on_click(move || close(st)));
    let tools = Flex::new()
        .wrap()
        .gap(6.0)
        .child(Button::new("Выбрать файлы…").class("head-btn").on_click(move || pick(st, "media", true, None)))
        .child(Button::new("Из медиатеки").class("head-btn").on_click(move || from_library(st)))
        .child(Button::new("Очистить").class("head-btn").on_click(move || {
            st.proj_draft.update(|d| {
                if let Some(d) = d {
                    for r in d.rows.iter_mut().filter(|r| r.kind != "text") {
                        r.value.clear();
                    }
                }
            })
        }))
        .child(Text::new(format!("видео/фото: {filled} из {n_media}")).class("row-sub"));

    let mut list = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for (i, r) in d.rows.iter().enumerate() {
        let kind = r.kind;
        let label = Text::new(format!("{} · {:.1} с", r.label, r.dur)).max_lines(1).elide(Elide::End).class("lp-name");
        let row: Box<dyn Widget> = if kind == "text" {
            Box::new(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().style("width", 200.0f32).child(label))
                    .child(DecoratedBox::new().class("grow").child(TextField::new().text(r.value.clone()).placeholder("текст").on_change(move |s: &str| {
                        let s = s.to_string();
                        st.proj_draft.update(|d| {
                            if let Some(r) = d.as_mut().and_then(|d| d.rows.get_mut(i)) {
                                r.value = s;
                            }
                        });
                    }))),
            )
        } else {
            let what = if r.value.is_empty() { "пусто".to_string() } else { file_name(&r.value) };
            Box::new(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().style("width", 200.0f32).child(label))
                    .child(DecoratedBox::new().class("grow").child(Text::new(what).max_lines(1).elide(Elide::Middle).class(if r.value.is_empty() { "row-sub" } else { "lp-desc" })))
                    .child(ToolButton::new("\u{E2C7}").tooltip(if kind == "music" { "Выбрать музыку" } else { "Выбрать файл" }).on_click(move || pick(st, kind, false, Some(i))))
                    .child(ToolButton::new("\u{E5CD}").tooltip("Убрать").on_click(move || {
                        st.proj_draft.update(|d| {
                            if let Some(r) = d.as_mut().and_then(|d| d.rows.get_mut(i)) {
                                r.value.clear();
                            }
                        })
                    })),
            )
        };
        list = list.child(DecoratedBox::new().class("lp-item").child(row));
    }
    let foot = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new("Пустые места останутся плашками — заполнить потом: 2×клик по плашке или инспектор").max_lines(2).class("row-sub"))
        .child(DecoratedBox::new().class("grow"))
        .child(Button::new("Отмена").class("head-btn").on_click(move || close(st)))
        .child(Button::new("Создать ролик").class("primary-btn").on_click(move || create(st)));

    DecoratedBox::new().class("fp-card").style("width", 720.0f32).style("max-height", 720.0f32).child(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(head)
            .child(Text::new(d.info.clone()).max_lines(3).class("row-sub"))
            .child(tools)
            .child(DecoratedBox::new().class("grow").child(ScrollView::new().vertical().child(list)))
            .child(foot),
    )
}

/// Выбрать файл для места на таймлайне (инспектор, 2×щелчок по плашке).
pub fn pick_for_slot(st: St, clip: u64, audio: bool) {
    std::thread::spawn(move || {
        let mut exts: Vec<&str> = Vec::new();
        if audio {
            exts.extend(crate::cmd::media::AUDIO);
        } else {
            exts.extend(crate::cmd::media::VIDEO);
            exts.extend(crate::cmd::media::IMAGE);
        }
        let mut d = rfd::FileDialog::new().set_title("Файл для места шаблона").add_filter("Медиафайлы", &exts);
        if let Some(h) = std::env::var_os("HOME") {
            d = d.set_directory(PathBuf::from(h));
        }
        let Some(f) = d.pick_file() else { return };
        let f = f.display().to_string();
        syngui::async_runtime::run_on_main_thread(move || {
            st.exec("slot.fill", json!({"id": clip, "file": f}));
        });
    });
}
