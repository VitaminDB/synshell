//! Панель просмотра справа (F11): выделенный файл — картинка, видео и
//! звук (проигрыватель создаётся по щелчку ▶ и останавливается при смене
//! файла), текст, код и Markdown с подсветкой, остальное — миниатюрой
//! (PDF, документы); ниже — сведения. Ничего не выделено — сама папка.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use syngui::prelude::*;
use syngui::video::{AudioFilePlayer, VideoPlayer};
use syngui::widgets::*;

use super::{boxed, bx, icons, W};
use crate::model::{self, Entry};
use crate::state;

/// Больше — текст не показываем целиком (только начало).
const TEXT_LIMIT: usize = 256 << 10;
/// Файлы больше — не проверяем, текст ли это.
const TEXT_PROBE_MAX: u64 = 8 << 20;

enum Media {
    Video(Arc<Mutex<VideoPlayer>>),
    Audio(Arc<Mutex<AudioFilePlayer>>),
}

thread_local! {
    /// Играющее сейчас (путь, проигрыватель): другой файл — прежнее останавливается.
    static MEDIA: RefCell<Option<(PathBuf, Media)>> = const { RefCell::new(None) };
}

fn media_for(path: &Path) -> Option<()> {
    MEDIA.with(|m| m.borrow().as_ref().filter(|(p, _)| p == path).map(|_| ()))
}

fn stop_other(path: Option<&Path>) {
    MEDIA.with(|m| {
        let mut m = m.borrow_mut();
        if m.as_ref().is_some_and(|(p, _)| Some(p.as_path()) != path) {
            *m = None;
        }
    });
}

/// Картинки, которые syngui рисует сам; остальные — через миниатюру.
fn direct_image(mime: &str) -> bool {
    matches!(mime, "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/bmp" | "image/svg+xml" | "image/x-icon" | "image/vnd.microsoft.icon")
}

/// Текст ли: по типу или (неизвестный тип, небольшой файл) — нет нулевых байтов в начале.
fn is_text(e: &Entry) -> bool {
    if e.is_dir || e.size > TEXT_PROBE_MAX {
        return false;
    }
    let m = e.mime.as_str();
    if m.starts_with("text/") || m.ends_with("+xml") || m.ends_with("json") || m.ends_with("yaml") || m.ends_with("toml") {
        return true;
    }
    if m.starts_with("image/") || m.starts_with("video/") || m.starts_with("audio/") || m == "application/pdf" {
        return false;
    }
    let mut head = [0u8; 4096];
    let n = std::fs::File::open(&e.path).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
    n > 0 && !head[..n].contains(&0)
}

fn read_text(p: &Path) -> String {
    let mut buf = Vec::new();
    if let Ok(f) = std::fs::File::open(p) {
        let _ = std::io::Read::read_to_end(&mut std::io::Read::take(f, TEXT_LIMIT as u64), &mut buf);
    }
    String::from_utf8_lossy(&buf).to_string()
}

/// Кодовый блок Markdown: ограда длиннее любой серии ``` в тексте.
pub fn fenced(lang: &str, text: &str) -> String {
    let mut run = 0;
    let mut best = 0;
    for c in text.chars() {
        if c == '`' {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat(best.max(2) + 1);
    format!("{fence}{lang}\n{}\n{fence}", text.trim_end_matches('\n'))
}

fn ext_of(e: &Entry) -> String {
    e.path.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn text_view(e: &Entry) -> W {
    let text = read_text(&e.path);
    let ext = ext_of(e);
    match ext.as_str() {
        "md" | "markdown" => boxed(MarkdownView::new(text).with_syntax_highlight(true).code_font_family("monospace").selectable(true).class("pv-text")),
        "" | "txt" | "log" | "text" => boxed(Text::new(text).selectable(true).class("pv-plain")),
        _ => boxed(MarkdownView::new(fenced(&ext, &text)).with_syntax_highlight(true).code_font_family("monospace").selectable(true).class("pv-text")),
    }
}

/// Картинка по центру рамки высотой `max_h` (по ширине — сколько есть).
fn fit_image(path: &Path, max_h: f32) -> W {
    framed(Image::new(path.to_string_lossy().to_string()).fit(ImageFit::Contain).class("pv-image"), max_h)
}

fn framed(child: impl Widget + 'static, h: f32) -> W {
    boxed(
        DecoratedBox::new().class("pv-media").style("height", h).child(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(child),
        ),
    )
}

fn big_icon(e: &Entry) -> W {
    boxed(
        Row::new()
            .main_axis_alignment(MainAxisAlignment::Center)
            .child(DecoratedBox::new().style("width", 128.0).style("height", 128.0).child(super::items::entry_image(e, 128, false))),
    )
}

fn thumb_or_icon(e: &Entry, max_h: f32) -> W {
    match crate::thumbs::get(&e.path, &e.mime, e.mtime, 512) {
        Some(t) => fit_image(&t, max_h),
        None => big_icon(e),
    }
}

fn clock(s: f64) -> String {
    let s = s.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Пока играет — раз в полсекунды двигать `tick` (время и кнопка).
fn ticker(path: PathBuf, tick: RwSignal<u64>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let (tx, rx) = std::sync::mpsc::channel();
        let p = path.clone();
        syngui::async_runtime::run_on_main_thread(move || {
            let alive = media_for(&p).is_some();
            if alive {
                tick.update(|t| *t += 1);
            }
            let _ = tx.send(alive);
        });
        if !rx.recv().unwrap_or(false) {
            return;
        }
    });
}

/// Видео: до щелчка — миниатюра с ▶, затем — само видео с паузой и временем.
fn video_view(e: &Entry, max_h: f32) -> W {
    let path = e.path.clone();
    let entry = e.clone();
    let tick = use_signal(0u64);
    boxed(Reactive::new(move || -> Vec<W> {
        let _ = tick.get();
        let player = MEDIA.with(|m| match m.borrow().as_ref() {
            Some((p, Media::Video(pl))) if *p == path => Some(pl.clone()),
            _ => None,
        });
        let Some(pl) = player else {
            let path = path.clone();
            let start = move || {
                match VideoPlayer::open_with_hwaccel(&path.to_string_lossy(), syngui::video::HwAccel::platform_default()) {
                    Ok(p) => {
                        MEDIA.with(|m| *m.borrow_mut() = Some((path.clone(), Media::Video(Arc::new(Mutex::new(p))))));
                        ticker(path.clone(), tick);
                        tick.update(|t| *t += 1);
                    }
                    Err(err) => state::toast_error(format!("{}: {err}", crate::ops::name_of(&path))),
                }
            };
            return vec![play_overlay(thumb_or_icon(&entry, max_h), max_h, start)];
        };
        let (paused, pos, dur) = pl.lock().map(|p| (p.is_paused() || p.is_ended(), p.position_sec(), p.duration_sec())).unwrap_or((true, 0.0, 0.0));
        let video = framed(VideoView::new(pl.clone()).fit(ImageFit::Contain), max_h);
        vec![boxed(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(video).child(transport(paused, pos, dur, move || {
            if let Ok(mut p) = pl.lock() {
                if p.is_ended() {
                    let _ = p.seek(0.0);
                    p.play();
                } else if p.is_paused() {
                    p.play();
                } else {
                    p.pause();
                }
            }
            tick.update(|t| *t += 1);
        })))]
    }))
}

/// Звук: обложка (или значок), теги, ▶/⏸ и время.
fn audio_view(e: &Entry, max_h: f32) -> W {
    let path = e.path.clone();
    let entry = e.clone();
    let tick = use_signal(0u64);
    let meta = use_signal(None::<syngui::video::AudioTrackMeta>);
    {
        let p = path.to_string_lossy().to_string();
        std::thread::spawn(move || {
            if let Ok(m) = syngui::video::probe_audio(&p) {
                syngui::async_runtime::run_on_main_thread(move || meta.set_always(Some(m)));
            }
        });
    }
    boxed(Reactive::new(move || -> Vec<W> {
        let _ = tick.get();
        let m = meta.get();
        let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        let cover = m.as_ref().and_then(|m| m.cover.clone());
        col = col.child(match cover {
            Some(bytes) => framed(Image::from_bytes(format!("cover:{}", path.display()), bytes).fit(ImageFit::Contain).class("pv-image"), max_h.min(260.0)),
            None => big_icon(&entry),
        });
        if let Some(m) = &m {
            let line = [m.artist.clone(), m.album.clone()].into_iter().flatten().collect::<Vec<_>>().join(" — ");
            if let Some(t) = &m.title {
                col = col.child(Text::new(t.clone()).max_lines(2).class("pv-title"));
            }
            if !line.is_empty() {
                col = col.child(Text::new(line).max_lines(2).class("meta"));
            }
        }
        let player = MEDIA.with(|mm| match mm.borrow().as_ref() {
            Some((p, Media::Audio(pl))) if *p == path => Some(pl.clone()),
            _ => None,
        });
        let dur = m.as_ref().map(|m| m.duration_sec).unwrap_or(0.0);
        let (paused, pos) = player.as_ref().and_then(|pl| pl.lock().ok().map(|p| (p.is_paused() || p.is_ended(), p.position_sec()))).unwrap_or((true, 0.0));
        let path = path.clone();
        col = col.child(transport(paused, pos, dur, move || {
            match &player {
                Some(pl) => {
                    if let Ok(mut p) = pl.lock() {
                        if p.is_ended() {
                            let _ = p.seek(0.0);
                            p.play();
                        } else if p.is_paused() {
                            p.play();
                        } else {
                            p.pause();
                        }
                    }
                }
                None => match AudioFilePlayer::open(&path.to_string_lossy()) {
                    Ok(p) => {
                        MEDIA.with(|mm| *mm.borrow_mut() = Some((path.clone(), Media::Audio(Arc::new(Mutex::new(p))))));
                        ticker(path.clone(), tick);
                    }
                    Err(err) => state::toast_error(format!("{}: {err}", crate::ops::name_of(&path))),
                },
            }
            tick.update(|t| *t += 1);
        }));
        vec![boxed(col)]
    }))
}

fn play_overlay(under: W, h: f32, start: impl Fn() + Send + Sync + 'static) -> W {
    boxed(
        GestureDetector::new().on_click(start).child(
            Stack::new().child(DecoratedBox::new().style("height", h).child(under)).child(
                DecoratedBox::new().style("height", h).child(
                    Row::new()
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(DecoratedBox::new().class("pv-play").child(
                            Column::new()
                                .main_axis_alignment(MainAxisAlignment::Center)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(Icon::new(icons::PLAY).class("pv-play-icon")),
                        )),
                ),
            ),
        ),
    )
}

fn transport(paused: bool, pos: f64, dur: f64, toggle: impl Fn() + Send + Sync + 'static) -> W {
    boxed(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(GestureDetector::new().on_click(toggle).child(DecoratedBox::new().class("pv-btn").child(Icon::new(if paused { icons::PLAY } else { icons::PAUSE }).class("icon"))))
            .child(Text::new(if dur > 0.0 { format!("{} / {}", clock(pos), clock(dur)) } else { clock(pos) }).class("meta")),
    )
}

fn prop(k: &str, v: String) -> W {
    boxed(
        Column::new()
            .gap(1.0)
            .child(Text::new(k).class("pv-key"))
            .child(Text::new(v).selectable(true).max_lines(3).class("pv-val")),
    )
}

/// Сведения о файле под просмотром.
fn props(e: &Entry) -> Vec<W> {
    let mut v = vec![prop(&t!("Тип"), e.description())];
    if e.is_dir {
        if let Some(n) = e.children {
            v.push(prop(&t!("Содержит"), model::format_count(n)));
        }
        if let Some(s) = crate::dirsize::get(&e.path) {
            v.push(prop(&t!("Размер"), model::format_size(s.bytes)));
        }
    } else {
        v.push(prop(&t!("Размер"), model::format_size(e.size)));
    }
    if e.mime.starts_with("image/") && e.mime != "image/svg+xml" {
        if let Ok((w, h)) = image::image_dimensions(&e.path) {
            v.push(prop(&t!("Разрешение"), format!("{w} × {h}")));
        }
    }
    v.push(prop(&t!("Изменён"), model::format_time(e.mtime)));
    let tags = crate::tags::get(&e.path);
    if !tags.tags.is_empty() {
        v.push(prop(&t!("Метки"), tags.tags.iter().map(|t| crate::tags::color_title(t)).collect::<Vec<_>>().join(", ")));
    }
    if tags.rating > 0 {
        v.push(prop(&t!("Оценка"), "★".repeat(tags.rating.div_ceil(2) as usize)));
    }
    if let Some(m) = crate::gitstatus::get(&e.path) {
        v.push(prop("git", m.title()));
    }
    v.push(prop(&t!("Расположение"), e.path.parent().map(|p| super::items::short_path(p)).unwrap_or_default()));
    v
}

fn entry_preview(e: &Entry, max_h: f32) -> W {
    if e.is_dir {
        return big_icon(e);
    }
    let m = e.mime.as_str();
    if direct_image(m) {
        fit_image(&e.path, max_h)
    } else if m.starts_with("video/") {
        video_view(e, max_h)
    } else if m.starts_with("audio/") {
        audio_view(e, max_h)
    } else if m.starts_with("image/") || m == "application/pdf" {
        thumb_or_icon(e, max_h)
    } else if is_text(e) {
        bx("pv-text-box", text_view(e))
    } else {
        thumb_or_icon(e, max_h)
    }
}

/// Сама панель: следит за выделением активной панели файлов.
pub fn panel() -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let ctx = state::ctx();
        let _ = ctx.thumbs_rev.get();
        let _ = ctx.marks_rev.get();
        let p = state::tab_tracked().pane_tracked();
        let sel = state::selected_entries_tracked(p);
        let loc = p.loc.get();
        let max_h = 300.0;
        let mut col = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        match sel.as_slice() {
            [e] => {
                stop_other(Some(&e.path));
                col = col.child(entry_preview(e, max_h));
                col = col.child(Text::new(e.name.clone()).selectable(true).max_lines(3).class("pv-title"));
                for w in props(e) {
                    col = col.child(w);
                }
            }
            [] => {
                stop_other(None);
                if let Some(e) = loc.dir().and_then(Entry::from_path) {
                    col = col.child(big_icon(&e));
                    col = col.child(Text::new(loc.title()).max_lines(3).class("pv-title"));
                    for w in props(&e) {
                        col = col.child(w);
                    }
                } else {
                    col = col.child(Text::new(loc.title()).class("pv-title"));
                }
            }
            many => {
                stop_other(None);
                let files = many.iter().filter(|e| !e.is_dir).count();
                let bytes: u64 = many.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
                col = col.child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(Icon::new(icons::SELECT_ALL).class("pv-many-icon")));
                col = col.child(Text::new(t!("Выбрано: {v}", v = model::format_count(many.len() as u32))).class("pv-title"));
                col = col.child(prop(&t!("Файлов"), files.to_string()));
                col = col.child(prop(&t!("Папок"), (many.len() - files).to_string()));
                col = col.child(prop(&t!("Размер файлов"), model::format_size(bytes)));
            }
        }
        vec![boxed(col)]
    });
    boxed(DecoratedBox::new().class("preview").child(ScrollView::new().vertical().class("grow").child(bx("pv-body", body))))
}

#[cfg(test)]
mod tests {
    #[test]
    fn fence_outgrows_backticks() {
        assert_eq!(super::fenced("rs", "fn x() {}\n"), "```rs\nfn x() {}\n```");
        assert!(super::fenced("md", "a ```` b").starts_with("`````md\n"));
    }
}
