use std::result::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use syngui::prelude::*;
use syngui::widgets::visual::canvas::Canvas;
use syngui::widgets::visual::MarkdownView;
use syngui::widgets::{Stack, Tooltip};
use synassist::agent::{Agent, AgentEvent, DaemonBackend};
use synassist::chat::{Chat, Role, Store};
use synassist_voice::{Phase, VisHandle, VoiceConfig, VoiceEvent};

use syngui::StyleValue;
use syngui::widgets::containers::Positioned;
use syngui::widgets::Chip;
use syngui_layer::{DropData, DropEvent};
use syngui::input::Key;
use syngui_layer::{Anchor, KeyInfo, KeyboardInteractivity, Layer, SurfaceHooks, SurfaceId, SurfaceSpec};

use crate::ui::{icon, rx};
use crate::ctx::PopupKind;
use crate::ShellCtx;

const SPARKLE: &str = "\u{E65F}";
const MIC: &str = "\u{E029}";
const MIC_OFF: &str = "\u{E02B}";
const STOP: &str = "\u{E047}";
const VOICE_CHAT: &str = "\u{E62E}";
const KEYBOARD: &str = "\u{E312}";
const CALL_END: &str = "\u{E0B1}";
const VOLUME_UP: &str = "\u{E050}";
const COPY: &str = "\u{E14D}";
const OPEN: &str = "\u{E89E}";
const ADD: &str = "\u{E145}";
const SEND: &str = "\u{E163}";
const CLOSE: &str = "\u{E5CD}";
const ATTACH: &str = "\u{E226}";
const PLAY: &str = "\u{E037}";
const AUDIO: &str = "\u{E405}";
const PDF: &str = "\u{E415}";
/// Сторона превью картинки-вложения.
const THUMB: f32 = 72.0;

#[derive(Clone, Copy)]
struct Panel {
    query: RwSignal<String>,
    answer: RwSignal<String>,
    tool: RwSignal<Option<String>>,
    /// Ход инструмента: доля 0..1 (None — неизвестна) и подпись этапа.
    tool_progress: RwSignal<Option<(Option<f32>, String)>>,
    confirm: RwSignal<Option<String>>,
    busy: RwSignal<bool>,
    chat_id: RwSignal<Option<String>>,
    input_rev: RwSignal<u64>,
    voice_open: RwSignal<bool>,
    phase: RwSignal<Phase>,
    vis: RwSignal<Option<VisHandle>>,
    heard: RwSignal<String>,
    muted: RwSignal<bool>,
    dictating: RwSignal<bool>,
    speaking: RwSignal<bool>,
    note: RwSignal<Option<String>>,
    attachments: RwSignal<Vec<std::path::PathBuf>>,
    drag_over: RwSignal<bool>,
    /// Файлы, созданные инструментами за этот ответ (картинки, звук…).
    results: RwSignal<Vec<std::path::PathBuf>>,
}

thread_local! {
    static PANEL: std::cell::Cell<Option<Panel>> = const { std::cell::Cell::new(None) };
}

struct Shared {
    cancel: Arc<AtomicBool>,
    answer: Mutex<Option<Sender<bool>>>,
    turn: Mutex<Option<Sender<Result<String, String>>>>,
}

fn shared() -> &'static Shared {
    static S: std::sync::OnceLock<Shared> = std::sync::OnceLock::new();
    S.get_or_init(|| Shared { cancel: Arc::new(AtomicBool::new(false)), answer: Mutex::new(None), turn: Mutex::new(None) })
}

fn panel() -> Panel {
    PANEL.with(|p| match p.get() {
        Some(v) => v,
        None => {
            let v = Panel {
                query: use_signal(String::new()),
                answer: use_signal(String::new()),
                tool: use_signal(None),
                tool_progress: use_signal(None),
                confirm: use_signal(None),
                busy: use_signal(false),
                chat_id: use_signal(None),
                input_rev: use_signal(0),
                voice_open: use_signal(false),
                phase: use_signal(Phase::Off),
                vis: use_signal(None),
                heard: use_signal(String::new()),
                muted: use_signal(false),
                dictating: use_signal(false),
                speaking: use_signal(false),
                note: use_signal(None),
                attachments: use_signal(Vec::new()),
                drag_over: use_signal(false),
                results: use_signal(Vec::new()),
            };
            p.set(Some(v));
            v
        }
    })
}

thread_local! {
    static SURFACE: std::cell::Cell<Option<SurfaceId>> = const { std::cell::Cell::new(None) };
    static SHOWN: std::cell::Cell<Option<RwSignal<bool>>> = const { std::cell::Cell::new(None) };
}

fn popup_open() -> bool {
    SURFACE.with(|s| s.get()).is_some() || ShellCtx::get().popup.get_untracked().is_some_and(|p| matches!(p.kind, PopupKind::Assistant))
}

/// Открыть или закрыть окно ассистента (команда `assistant`). На компьютере
/// оно живёт своей поверхностью и не закрывается щелчком мимо: на него
/// перетаскивают файлы и текст из других окон. Закрывают явно — Esc,
/// крестиком или той же командой.
pub fn toggle(ctx: ShellCtx) {
    if ctx.is_phone() {
        ctx.open_popup(PopupKind::Assistant, crate::commands::centered());
        return;
    }
    if SURFACE.with(|s| s.get()).is_some() {
        close();
        return;
    }
    let my_id: Arc<Mutex<Option<SurfaceId>>> = Arc::new(Mutex::new(None));
    let hook_id = my_id.clone();
    let hooks = SurfaceHooks {
        on_key: Some(Box::new(|k: &KeyInfo| {
            if k.pressed && k.key == Key::Escape {
                close();
                return true;
            }
            false
        })),
        on_closed: Some(Box::new(move || {
            if SURFACE.with(|s| s.get()) == *hook_id.lock().unwrap() {
                SURFACE.with(|s| s.set(None));
                SHOWN.with(|o| o.take());
            }
        })),
        ..Default::default()
    };
    let output = crate::manager::focused_output(&ctx);
    let spec = SurfaceSpec {
        namespace: "syndesktop-assistant".into(),
        layer: Layer::Overlay,
        anchor: Anchor::empty(),
        size: (0, 0),
        margin: [0; 4],
        exclusive_zone: -1,
        // Сразу — клавиатура окну, чтобы печатать без щелчка; дальше «по
        // требованию»: щелчок по другому окну отдаёт фокус ему.
        keyboard: KeyboardInteractivity::Exclusive,
        output,
        auto_size: true,
        clear_color: [0.0; 4],
    };
    let open = use_signal(true);
    let sid = my_id.clone();
    let id = syngui_layer::create_surface_with(spec.clone(), hooks, move || {
        let ctx = ShellCtx::get();
        let sid = sid.clone();
        let card = crate::anim::popup_presence(&ctx, open, None, false, move || {
            Box::new(DecoratedBox::new().child(view(ShellCtx::get())).class("popup-card assist-card").style("width", StyleValue::px(600.0)))
        })
        .on_exit_complete(move || {
            if let Some(id) = *sid.lock().unwrap() {
                syngui_layer::close_surface(id);
            }
        });
        Box::new(DecoratedBox::new().child(card).class("assist-surface"))
    });
    *my_id.lock().unwrap() = Some(id);
    SURFACE.with(|s| s.set(Some(id)));
    SHOWN.with(|o| o.set(Some(open)));
    let p = panel();
    syngui_layer::set_surface_drop_handler(id, move |ev| match ev {
        DropEvent::Enter(_) | DropEvent::Motion(_) => {
            if !p.drag_over.get_untracked() {
                p.drag_over.set(true);
            }
        }
        DropEvent::Leave => p.drag_over.set(false),
        DropEvent::Drop(_, data) => {
            p.drag_over.set(false);
            dropped(p, data);
        }
    });
    syngui_layer::add_timer(Duration::from_millis(400), move || {
        if SURFACE.with(|s| s.get()) == Some(id) {
            syngui_layer::reconfigure_surface(id, SurfaceSpec { keyboard: KeyboardInteractivity::OnDemand, ..spec.clone() });
        }
        None
    });
}

/// Брошенное на окно: файлы и картинки — вложениями, короткий текст — в поле
/// ввода, длинный — вложением-файлом.
fn dropped(p: Panel, data: DropData) {
    let save = |ext: &str, bytes: &[u8]| -> Option<std::path::PathBuf> {
        let dir = synshell_common::paths::data_home().join("synshell/assistant-drops");
        std::fs::create_dir_all(&dir).ok()?;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let path = dir.join(format!("{stamp}.{ext}"));
        std::fs::write(&path, bytes).ok()?;
        Some(path)
    };
    let files = match data {
        DropData::Files(f) => f,
        DropData::Data { mime, bytes } => {
            let ext = mime.strip_prefix("image/").map(|e| e.split(['+', ';']).next().unwrap_or("png").replace("jpeg", "jpg")).unwrap_or_else(|| "bin".into());
            save(&ext, &bytes).into_iter().collect()
        }
        DropData::Text(t) if t.chars().count() > 2000 => save("txt", t.as_bytes()).into_iter().collect(),
        DropData::Text(t) => {
            let mut q = p.query.get_untracked();
            if !q.is_empty() && !q.ends_with(char::is_whitespace) {
                q.push(' ');
            }
            q.push_str(t.trim());
            p.query.set(q);
            p.input_rev.set(p.input_rev.get_untracked() + 1);
            return;
        }
    };
    let mut v = p.attachments.get_untracked();
    for f in files {
        if !v.contains(&f) {
            v.push(f);
        }
    }
    p.attachments.set(v);
}

fn is_video(path: &std::path::Path) -> bool {
    synassist::attach::kind(path) == synassist::attach::Kind::Video
}

/// Постер видео (миниатюра freedesktop) или `None`, пока он готовится —
/// по готовности окно перерисуется (`thumbs_rev`).
fn video_poster(path: &std::path::Path) -> Option<std::path::PathBuf> {
    thread_local! {
        static REV: std::cell::Cell<Option<RwSignal<u64>>> = const { std::cell::Cell::new(None) };
    }
    let rev = REV.with(|r| match r.get() {
        Some(v) => v,
        None => {
            let v = use_signal(0u64);
            r.set(Some(v));
            synshell_common::thumbs::set_notify(move || run_on_main_thread(move || v.set(v.get_untracked() + 1)));
            v
        }
    });
    rev.get();
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
    synshell_common::thumbs::get(path, "video/mp4", mtime, 256)
}

fn is_image(path: &std::path::Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "avif" | "heic")
}

thread_local! {
    /// Проигрыватель звука результата: файл, плеер, длительность.
    static AUDIO_PLAYER: std::cell::RefCell<Option<(std::path::PathBuf, syngui::audio::AudioPlayer)>> = const { std::cell::RefCell::new(None) };
    /// Ролик, который сейчас играет в панели.
    static VIDEO_PLAYER: std::cell::RefCell<Option<(std::path::PathBuf, Arc<Mutex<syngui::video::VideoPlayer>>)>> = const { std::cell::RefCell::new(None) };
}

fn media_rev() -> RwSignal<u64> {
    thread_local! {
        static REV: std::cell::Cell<Option<RwSignal<u64>>> = const { std::cell::Cell::new(None) };
    }
    REV.with(|r| match r.get() {
        Some(v) => v,
        None => {
            let v = use_signal(0u64);
            r.set(Some(v));
            v
        }
    })
}

fn bump_media() {
    let r = media_rev();
    r.set(r.get_untracked() + 1);
}

/// Остановить звук и ролик (новый ответ, закрытие окна).
fn stop_media() {
    if let Some((_, p)) = AUDIO_PLAYER.with(|a| a.borrow_mut().take()) {
        p.stop();
    }
    VIDEO_PLAYER.with(|v| v.borrow_mut().take());
}

fn fmt_time(s: f64) -> String {
    let s = s.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Звук → перемежённое стерео 48 кГц (ffmpeg).
fn decode_stereo(path: &std::path::Path) -> Option<Arc<[f32]>> {
    let out = std::process::Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(path).args(["-vn", "-ac", "2", "-ar", "48000", "-f", "f32le", "-"]).output().ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    Some(out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect::<Vec<_>>().into())
}

/// Строка проигрывателя звука: ▶/⏸, ползунок, время.
fn audio_row(path: std::path::PathBuf) -> Box<dyn Widget> {
    let rev = media_rev();
    Box::new(rx(move || {
        rev.get();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let state = AUDIO_PLAYER.with(|a| a.borrow().as_ref().filter(|(p, _)| *p == path).map(|(_, pl)| (pl.position_seconds(), pl.duration_seconds(), pl.is_paused() || pl.is_done())));
        let (pos, dur, paused) = state.unwrap_or((0.0, 0.0, true));
        let p1 = path.clone();
        let toggle = Button::new("").icon(if paused { PLAY } else { "\u{E034}" }).class("btn assist-mic send").on_click(move || {
            let mine = AUDIO_PLAYER.with(|a| a.borrow().as_ref().is_some_and(|(p, _)| *p == p1));
            if mine {
                AUDIO_PLAYER.with(|a| {
                    if let Some((_, pl)) = a.borrow().as_ref() {
                        if pl.is_done() {
                            let _ = pl.seek_seconds(0.0);
                            pl.resume();
                        } else if pl.is_paused() {
                            pl.resume();
                        } else {
                            pl.pause();
                        }
                    }
                });
            } else {
                stop_media();
                if let Some(pcm) = decode_stereo(&p1) {
                    if let Ok(pl) = syngui::audio::AudioPlayer::start_stereo(pcm, 48_000) {
                        AUDIO_PLAYER.with(|a| *a.borrow_mut() = Some((p1.clone(), pl)));
                        // Пока играет — обновлять ползунок и время.
                        syngui_layer::add_timer(Duration::from_millis(250), move || {
                            bump_media();
                            let playing = AUDIO_PLAYER.with(|a| a.borrow().as_ref().is_some_and(|(_, pl)| !pl.is_done()));
                            playing.then_some(Duration::from_millis(250))
                        });
                    }
                }
            }
            bump_media();
        });
        let p2 = path.clone();
        let slider = Slider::new().range(0.0, dur.max(0.01) as f32).value(pos as f32).width(260.0).on_change(move |v| {
            AUDIO_PLAYER.with(|a| {
                if let Some((p, pl)) = a.borrow().as_ref() {
                    if *p == p2 {
                        let _ = pl.seek_seconds(v as f64);
                    }
                }
            });
        });
        Box::new(
            DecoratedBox::new()
                .class("assist-audio")
                .child(
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(toggle)
                        .child(Column::new().gap(2.0).child(Text::new(name).max_lines(1).class("assist-audio-name")).child(slider))
                        .child(Text::new(if dur > 0.0 { format!("{} / {}", fmt_time(pos), fmt_time(dur)) } else { String::new() }).class("assist-hint")),
                ),
        ) as Box<dyn Widget>
    }))
}

/// Ролик: постер с ▶, по щелчку — компактный проигрыватель в панели.
fn video_tile(path: std::path::PathBuf) -> Box<dyn Widget> {
    let rev = media_rev();
    Box::new(rx(move || {
        rev.get();
        let playing = VIDEO_PLAYER.with(|v| v.borrow().as_ref().filter(|(p, _)| *p == path).map(|(_, pl)| pl.clone()));
        if let Some(pl) = playing {
            return Box::new(DecoratedBox::new().class("assist-video").child(syngui::widgets::visual::video_view::VideoPlayerView::file(pl).compact(true).build())) as Box<dyn Widget>;
        }
        let poster: Box<dyn Widget> = match video_poster(&path) {
            Some(p) => Box::new(Image::new(p.to_string_lossy()).fit(ImageFit::Cover).class("assist-result-img")),
            None => Box::new(DecoratedBox::new().class("assist-result-img assist-thumb-empty")),
        };
        let play = Column::new()
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("assist-result-img")
            .child(DecoratedBox::new().class("assist-play big").child(icon(PLAY).class("assist-play-ic")));
        let p = path.clone();
        Box::new(
            crate::ui::InputArea::new(DecoratedBox::new().child(Stack::new().child(poster).child(play)).class("assist-result").clip(true)).pointer().on_click(move |b, _, _| {
                if b != syngui::input::MouseButton::Left {
                    return;
                }
                stop_media();
                // Аппаратный декодер, где он есть (иначе программный).
                match syngui::video::VideoPlayer::open_with_hwaccel(&p.to_string_lossy(), syngui::video::HwAccel::platform_default()) {
                    Ok(pl) => VIDEO_PLAYER.with(|v| *v.borrow_mut() = Some((p.clone(), Arc::new(Mutex::new(pl))))),
                    Err(_) => {
                        let _ = std::process::Command::new("xdg-open").arg(&p).spawn();
                    }
                }
                bump_media();
            }),
        ) as Box<dyn Widget>
    }))
}

/// Вложения под строкой ввода — чипы с крестиком.
fn attachments_row(p: Panel) -> Box<dyn Widget> {
    Box::new(rx(move || {
        let files = p.attachments.get();
        if files.is_empty() {
            return Box::new(Row::new()) as Box<dyn Widget>;
        }
        let remove = move |i: usize| {
            let mut v = p.attachments.get_untracked();
            if i < v.len() {
                v.remove(i);
            }
            p.attachments.set(v);
        };
        let mut row = Flex::new().direction(FlexDirection::Row).wrap().gap(8.0);
        // Картинки — превью, остальное — чипами.
        for (i, f) in files.iter().enumerate().filter(|(_, f)| is_image(f)) {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let thumb = Stack::new()
                .child(Image::new(f.to_string_lossy()).fit(ImageFit::Cover).class("assist-thumb"))
                .child(Positioned::new(Button::new("").icon(CLOSE).class("btn assist-thumb-x").on_click(move || remove(i))).at(THUMB - 26.0, 4.0));
            row = row.child(tip(DecoratedBox::new().child(thumb).class("assist-thumb-box").clip(true), name));
        }
        // Видео — постер с ▶.
        for (i, f) in files.iter().enumerate().filter(|(_, f)| is_video(f)) {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let poster: Box<dyn Widget> = match video_poster(f) {
                Some(p) => Box::new(Image::new(p.to_string_lossy()).fit(ImageFit::Cover).class("assist-thumb")),
                None => Box::new(DecoratedBox::new().class("assist-thumb assist-thumb-empty")),
            };
            let play = Column::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("assist-thumb")
                .child(DecoratedBox::new().class("assist-play").child(icon(PLAY).class("assist-play-ic")));
            let thumb = Stack::new()
                .child(poster)
                .child(play)
                .child(Positioned::new(Button::new("").icon(CLOSE).class("btn assist-thumb-x").on_click(move || remove(i))).at(THUMB - 26.0, 4.0));
            row = row.child(tip(DecoratedBox::new().child(thumb).class("assist-thumb-box").clip(true), name));
        }
        for (i, f) in files.iter().enumerate().filter(|(_, f)| !is_image(f) && !is_video(f)) {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| f.to_string_lossy().into_owned());
            let glyph = match synassist::attach::kind(f) {
                synassist::attach::Kind::Audio => AUDIO,
                synassist::attach::Kind::Pdf => PDF,
                _ => ATTACH,
            };
            row = row.child(Chip::new(name).icon(glyph).deletable().on_delete(move || remove(i)).class("assist-att"));
        }
        Box::new(DecoratedBox::new().child(row).class("assist-atts"))
    }))
}

fn drop_zone() -> Box<dyn Widget> {
    Box::new(
        DecoratedBox::new().class("assist-drop").child(
            Column::new()
                .gap(6.0)
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(icon(ATTACH).class("assist-drop-ic"))
                .child(Text::new(t!("Отпустите, чтобы прикрепить")).class("assist-drop-title"))
                .child(Text::new(t!("Файлы, картинки, видео — вложениями, текст — в поле ввода")).class("assist-hint")),
        ),
    )
}

/// Закрыть окно ассистента (с анимацией ухода).
pub fn close() {
    let ctx = ShellCtx::get();
    if ctx.popup.get_untracked().is_some_and(|p| matches!(p.kind, PopupKind::Assistant)) {
        ctx.close_popup();
    }
    let Some(id) = SURFACE.with(|s| s.take()) else { return };
    match SHOWN.with(|o| o.take()) {
        Some(open) if crate::anim::on(&ctx) => open.set(false),
        _ => syngui_layer::close_surface(id),
    }
}

fn reply(p: Panel, yes: bool) {
    if let Some(tx) = shared().answer.lock().unwrap().take() {
        let _ = tx.send(yes);
    }
    p.confirm.set(None);
}

fn stop(p: Panel) {
    shared().cancel.store(true, Ordering::Relaxed);
    reply(p, false);
}

fn ask(p: Panel, text: String, voice: bool) {
    let text = text.trim().to_string();
    if (text.is_empty() && p.attachments.get_untracked().is_empty()) || p.busy.get_untracked() {
        if voice {
            if let Some(tx) = shared().turn.lock().unwrap().take() {
                let _ = tx.send(Err(String::new()));
            }
        }
        return;
    }
    let files = if voice { Vec::new() } else { p.attachments.get_untracked() };
    if !voice {
        p.attachments.set(Vec::new());
    }
    p.busy.set(true);
    stop_media();
    p.results.set(Vec::new());
    p.answer.set(String::new());
    p.tool.set(None);
    p.query.set(String::new());
    p.input_rev.set(p.input_rev.get_untracked() + 1);
    shared().cancel.store(false, Ordering::Relaxed);
    let store = Store::open();
    let mut chat = p.chat_id.get_untracked().and_then(|id| store.load(&id)).unwrap_or_else(Chat::new);
    let from = chat.messages.len();
    let speak_after = !voice && ShellCtx::get().cfg().assistant.speak_answers;
    std::thread::Builder::new()
        .name("assistant".into())
        .spawn(move || {
            let cfg = synshell_common::Config::load().0.assistant;
            let mut agent = Agent::configured(Arc::new(DaemonBackend), &cfg.disabled_tools, cfg.auto_approve, cfg.autocompact);
                agent = agent.approve(&cfg.approved_tools);
                if cfg.memory {
                    agent = agent.with_memory(synassist::memory::resolve(&cfg.memory_dir));
                }
                if cfg.skills {
                    agent = agent.with_skills(synassist::skills::resolve(&cfg.skills_dir));
                }
                agent = agent.with_knowledge(cfg.knowledge.iter().map(|k| synassist::builtin::expand(k)).collect());
                agent = agent.with_recipes(synassist::pipeline::resolve(&cfg.recipes_dir));
            if voice {
                agent.system = Some(synassist::prompt::build_voice(&agent.tools));
            }
            let confirm = move |_: &str, summary: &str| -> bool {
                let (tx, rx) = channel();
                *shared().answer.lock().unwrap() = Some(tx);
                let s = summary.to_string();
                run_on_main_thread(move || p.confirm.set(Some(s)));
                rx.recv().unwrap_or(false)
            };
            let cancel = shared().cancel.clone();
            let r = agent.run_turn_files(&mut chat, &text, files, &confirm, &cancel, &mut |ev| {
                run_on_main_thread(move || match ev {
                    AgentEvent::Text(t) => {
                        let mut a = p.answer.get_untracked();
                        a.push_str(&t);
                        p.answer.set(a);
                    }
                    AgentEvent::ToolStart { summary, .. } => {
                        p.tool.set(Some(summary));
                        p.tool_progress.set(None);
                    }
                    AgentEvent::ToolProgress { step, total, label } => {
                        let frac = (total > 0).then(|| step as f32 / total as f32);
                        p.tool_progress.set(Some((frac, label)));
                    }
                    AgentEvent::Message(m) if m.role == Role::Assistant && !m.tool_calls.is_empty() => p.answer.set(String::new()),
                    AgentEvent::Message(m) if m.role == Role::Tool && !m.files.is_empty() => {
                        let mut v = p.results.get_untracked();
                        v.extend(m.files.iter().filter(|f| f.exists()).cloned());
                        p.results.set(v);
                    }
                    _ => {}
                });
            });
            let _ = store.save(&chat);
            let id = chat.id.clone();
            let answer: String = chat
                .messages
                .iter()
                .skip(from)
                .filter(|m| m.role == Role::Assistant && !m.text.trim().is_empty())
                .map(|m| m.text.trim().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            let result = match &r {
                Ok(()) => Ok(answer.clone()),
                Err(e) => Err(e.clone()),
            };
            if let Some(tx) = shared().turn.lock().unwrap().take() {
                let _ = tx.send(result);
            }
            run_on_main_thread(move || {
                if let Err(e) = r {
                    p.answer.set(format!("**Ошибка:** {e}"));
                }
                p.chat_id.set(Some(id));
                p.tool.set(None);
                p.busy.set(false);
                p.confirm.set(None);
                if speak_after && !answer.is_empty() {
                    speak(p, synassist_voice::strip_markdown(&answer));
                }
            });
        })
        .ok();
}

fn listener(p: Panel) -> synassist_voice::Listener {
    Arc::new(move |ev: VoiceEvent| {
        run_on_main_thread(move || {
            if !popup_open() && synassist_voice::conversation_running() {
                synassist_voice::stop_conversation();
            }
            match ev {
                VoiceEvent::Phase(ph) => p.phase.set(ph),
                VoiceEvent::Vis(v) => p.vis.set(v),
                VoiceEvent::Heard(t) => p.heard.set(t),
                VoiceEvent::Speaking(on) => p.speaking.set(on),
                VoiceEvent::Error(e) => p.note.set(Some(e)),
                VoiceEvent::Stopped => {
                    if p.note.get_untracked().is_none() {
                        p.voice_open.set(false);
                    }
                }
            }
        });
    })
}

fn turn(p: Panel) -> synassist_voice::Turn {
    Arc::new(move |text: String| -> Result<String, String> {
        let (tx, rx) = channel::<Result<String, String>>();
        *shared().turn.lock().unwrap() = Some(tx);
        run_on_main_thread(move || ask(p, text, true));
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(r) => return r,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return Err(String::new()),
            }
        }
    })
}

fn start_voice(p: Panel) {
    p.voice_open.set(true);
    p.note.set(None);
    p.heard.set(String::new());
    p.muted.set(false);
    synassist_voice::set_muted(false);
    synassist_voice::start_conversation(VoiceConfig::load(), turn(p), listener(p));
}

fn stop_voice(p: Panel) {
    synassist_voice::stop_conversation();
    if let Some(tx) = shared().turn.lock().unwrap().take() {
        let _ = tx.send(Err(String::new()));
    }
    p.voice_open.set(false);
    p.note.set(None);
}

fn speak(p: Panel, text: String) {
    p.speaking.set(true);
    synassist_voice::speak(
        VoiceConfig::load(),
        text,
        Arc::new(move |_: Result<(), String>| run_on_main_thread(move || p.speaking.set(false))),
    );
}

fn toggle_dictation(p: Panel) {
    if synassist_voice::dictating() {
        synassist_voice::toggle_dictation(VoiceConfig::load(), listener(p), Arc::new(|_| {}));
        return;
    }
    p.dictating.set(true);
    let started = synassist_voice::toggle_dictation(
        VoiceConfig::load(),
        listener(p),
        Arc::new(move |r: Result<String, String>| {
            run_on_main_thread(move || {
                p.dictating.set(false);
                match r {
                    Ok(t) if !t.is_empty() => {
                        let prev = p.query.get_untracked();
                        p.query.set(if prev.trim().is_empty() { t } else { format!("{} {t}", prev.trim_end()) });
                        p.input_rev.set(p.input_rev.get_untracked() + 1);
                    }
                    Ok(_) => {}
                    Err(e) => p.note.set(Some(e)),
                }
            });
        }),
    );
    if !started {
        p.dictating.set(false);
    }
}

fn open_window(p: Panel) {
    let id = p.chat_id.get_untracked();
    let cmd = match id {
        Some(id) => format!("synassistant --chat {id}"),
        None => "synassistant".into(),
    };
    crate::actions::spawn(&cmd);
    close();
}

fn tip(w: impl Widget + 'static, text: impl Into<String>) -> Box<dyn Widget> {
    Box::new(Tooltip::new(w, text).delay_ms(500))
}

fn aura(handle: Option<VisHandle>, phase: Phase) -> Box<dyn Widget> {
    let prev: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(vec![0.0; 24]));
    Box::new(
        Canvas::new(move |ctx, t| {
            let accent = ctx.mss_accent().unwrap_or_else(|| Color::from_hex("#3B82F6"));
            let (cx, cy) = (ctx.width() * 0.5, ctx.height() * 0.5);
            let inner = 30.0;
            let outer = ctx.width().min(ctx.height()) * 0.5 - 4.0;
            let target = match (&handle, phase) {
                (Some(h), Phase::Listening) => h.snapshot_bars(24),
                _ => vec![0.0; 24],
            };
            let mut bars = prev.lock().unwrap_or_else(|e| e.into_inner());
            for (i, v) in target.iter().enumerate() {
                bars[i] += (v - bars[i]) * 0.3;
            }
            match phase {
                Phase::Listening => {
                    let n = bars.len() as f32;
                    for (i, v) in bars.iter().enumerate() {
                        let a = (i as f32 / n) * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                        let r0 = inner + 6.0;
                        let r1 = r0 + (outer - r0 - 2.0) * v.clamp(0.0, 1.0);
                        ctx.set_stroke_width(3.0);
                        ctx.set_color(accent.with_alpha(0.5 + v.clamp(0.0, 1.0) * 0.5));
                        ctx.draw_line(cx + a.cos() * r0, cy + a.sin() * r0, cx + a.cos() * r1, cy + a.sin() * r1);
                    }
                }
                Phase::Speaking => {
                    for i in 0..3 {
                        let p = ((t / 1.5) + i as f32 / 3.0).fract();
                        ctx.set_stroke_width(2.0 + (1.0 - p) * 3.0);
                        ctx.set_color(accent.with_alpha((1.0 - p) * 0.6));
                        ctx.stroke_circle(cx, cy, inner + (outer - inner) * p);
                    }
                }
                Phase::Off => {}
                _ => {
                    for i in 0..2 {
                        let start = t * 2.0 + i as f32 * 2.6;
                        ctx.set_stroke_width(3.0);
                        ctx.set_color(accent.with_alpha(0.75));
                        ctx.draw_arc(cx, cy, inner + 10.0 + i as f32 * 10.0, start, start + 1.4);
                    }
                }
            }
            ctx.set_color(accent.with_alpha(0.25));
            ctx.fill_circle(cx, cy, inner + 4.0);
            ctx.set_color(accent);
            ctx.fill_circle(cx, cy, inner - 2.0);
        })
        .size(140.0, 140.0)
        .animated(true)
        .class("assist-aura"),
    )
}

fn voice_stage(p: Panel) -> Box<dyn Widget> {
    let stage = rx(move || {
        let handle = p.vis.get();
        let phase = p.phase.get();
        let muted = p.muted.get();
        let (glyph, label) = match phase {
            Phase::Speaking => (STOP, t!("Остановить озвучку")),
            Phase::Listening if muted => (MIC_OFF, t!("Включить микрофон")),
            Phase::Listening => (MIC, t!("Выключить микрофон")),
            Phase::Thinking => (STOP, t!("Прервать ответ")),
            _ => (MIC, t!("Подождите…")),
        };
        let btn = Button::new("").icon(glyph).class("btn assist-center").on_click(move || match phase {
            Phase::Speaking => synassist_voice::stop_speaking(),
            Phase::Listening => {
                synassist_voice::set_muted(!muted);
                p.muted.set(!muted);
            }
            Phase::Thinking => stop(p),
            _ => {}
        });
        Box::new(
            DecoratedBox::new().class("assist-aura-wrap").child(
                Stack::new()
                    .clip(false)
                    .child(aura(handle, phase))
                    .child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).class("assist-aura-wrap").child(tip(btn, label))),
            ),
        )
    });
    let status = rx(move || {
        let phase = p.phase.get();
        let muted = p.muted.get();
        let text = if let Some(n) = p.note.get() {
            n
        } else {
            match phase {
                Phase::Off => t!("Голосовой режим выключен").to_string(),
                Phase::Starting => t!("Включаю микрофон…").to_string(),
                Phase::Listening if muted => t!("Микрофон выключен").to_string(),
                Phase::Listening => t!("Слушаю…").to_string(),
                Phase::Transcribing => t!("Распознаю…").to_string(),
                Phase::Thinking => t!("Думаю…").to_string(),
                Phase::Speaking => t!("Говорю…").to_string(),
            }
        };
        Box::new(Text::new(text).max_lines(2).class("assist-voice-status"))
    });
    let transcript = rx(move || {
        let heard = p.heard.get();
        let answer = p.answer.get();
        let tool = p.tool.get();
        let confirm = p.confirm.get();
        let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        if !heard.is_empty() {
            col = col.child(Row::new().main_axis_alignment(MainAxisAlignment::End).child(DecoratedBox::new().class("assist-heard").child(Text::new(heard).class("assist-heard-text"))));
        }
        if let Some(s) = confirm {
            col = col.child(confirm_card(p, s));
        } else if let Some(t) = tool.filter(|_| p.busy.get() && answer.is_empty()) {
            col = col.child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon("\u{E869}").class("assist-tool-icon")).child(Text::new(t).max_lines(1).class("assist-tool grow")));
        }
        if !answer.is_empty() {
            col = col.child(ScrollView::new().vertical().follow_end(true).class("assist-voice-scroll").child(MarkdownView::new(answer).selectable(true).class("assist-answer")));
        }
        Box::new(col)
    });
    let actions = rx(move || {
        let muted = p.muted.get();
        let speaking = p.speaking.get();
        let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).main_axis_alignment(MainAxisAlignment::Center);
        row = row.child(tip(
            Button::new("").icon(if muted { MIC_OFF } else { MIC }).class(if muted { "btn assist-act off" } else { "btn assist-act" }).on_click(move || {
                synassist_voice::set_muted(!muted);
                p.muted.set(!muted);
            }),
            if muted { t!("Включить микрофон") } else { t!("Выключить микрофон") },
        ));
        if speaking {
            row = row.child(tip(Button::new("").icon(STOP).class("btn assist-act").on_click(|| synassist_voice::stop_speaking()), t!("Замолчать")));
        }
        row = row.child(tip(Button::new("").icon(KEYBOARD).class("btn assist-act").on_click(move || stop_voice(p)), t!("Вернуться к тексту")));
        row = row.child(tip(Button::new("").icon(CALL_END).class("btn assist-act end").on_click(move || stop_voice(p)), t!("Завершить")));
        Box::new(row)
    });
    Box::new(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).class("assist-voice").child(stage).child(status).child(DecoratedBox::new().class("assist-voice-body").child(transcript)).child(actions))
}

/// Индикатор детектора речи (как проверка микрофона в synspeak): полоса уровня
/// с отметкой порога тишины, цифры и что детектор слышит.
fn confirm_card(p: Panel, summary: String) -> Column {
    Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("assist-confirm")
        .child(Text::new(t!("Разрешить действие?")).class("assist-confirm-title"))
        .child(Text::new(summary).max_lines(4).class("assist-confirm-sum"))
        .child(
            Row::new()
                .gap(8.0)
                .main_axis_alignment(MainAxisAlignment::End)
                .child(Button::new(t!("Отклонить")).class("btn").on_click(move || reply(p, false)))
                .child(Button::new(t!("Разрешить")).class("btn btn-primary").on_click(move || reply(p, true))),
        )
}

fn header(p: Panel) -> Box<dyn Widget> {
    Box::new(rx(move || {
        p.input_rev.get();
        let q = p.query;
        let initial = q.get_untracked();
        let mic = rx(move || {
            let on = p.dictating.get();
            let phase = p.phase.get();
            let label = if on && phase == Phase::Transcribing { t!("Распознаю…") } else if on { t!("Говорите… нажмите ещё раз, чтобы закончить") } else { t!("Надиктовать") };
            tip(Button::new("").icon(if on { STOP } else { MIC }).class(if on { "btn assist-mic rec on" } else { "btn assist-mic rec" }).on_click(move || toggle_dictation(p)), label)
        });
        let send = rx(move || {
            if p.busy.get() {
                tip(Button::new("").icon(STOP).class("btn assist-mic stop").on_click(move || stop(p)), t!("Остановить"))
            } else {
                tip(Button::new("").icon(SEND).class("btn assist-mic send").on_click(move || ask(p, p.query.get_untracked(), false)), t!("Отправить"))
            }
        });
        Box::new(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("assist-brand").child(icon(SPARKLE).class("assist-brand-ic")))
                .child(
                    TextField::new()
                        .text(initial)
                        .placeholder(t!("Спросите ассистента…"))
                        .autofocus(true)
                        .focus_token(p.input_rev.get_untracked())
                        .on_change(move |t| q.set(t.to_string()))
                        .on_submit(move |t| ask(p, t.to_string(), false))
                        .class("search-field grow"),
                )
                .child(mic)
                .child(send)
                .child(tip(Button::new("").icon(VOICE_CHAT).class("btn assist-mic voice").on_click(move || start_voice(p)), t!("Голосовой разговор")))
                .child(tip(Button::new("").icon(CLOSE).class("btn assist-mic close").on_click(close), t!("Закрыть (Esc)"))),
        )
    }))
}

pub fn view(_ctx: ShellCtx) -> impl Widget {
    let p = panel();
    let body = rx(move || {
        if p.voice_open.get() {
            return voice_stage(p);
        }
        if p.drag_over.get() {
            return drop_zone();
        }
        let answer = p.answer.get();
        let busy = p.busy.get();
        let tool = p.tool.get();
        let confirm = p.confirm.get();
        let speaking = p.speaking.get();
        let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        if let Some(s) = confirm {
            col = col.child(confirm_card(p, s));
        } else if let Some(t) = tool.filter(|_| busy && answer.is_empty()) {
            col = col.child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon("\u{E869}").class("assist-tool-icon")).child(Text::new(t).max_lines(1).class("assist-tool grow")));
            if let Some((frac, label)) = p.tool_progress.get() {
                col = col.child(progress_row(frac, &label));
            }
        } else if busy && answer.is_empty() {
            col = col.child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(CircularProgress::new().indeterminate().size(16.0).stroke_width(2.0).class("assist-spin")).child(Text::new(t!("Думаю…")).class("assist-hint")));
        }
        if !answer.is_empty() {
            col = col.child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .class("assist-msg")
                    .child(DecoratedBox::new().class("assist-avatar").child(icon(SPARKLE).class("assist-avatar-ic")))
                    .child(ScrollView::new().vertical().follow_end(true).class("assist-scroll grow").child(MarkdownView::new(answer.clone()).with_copy_code(true).selectable(true).class("assist-answer"))),
            );
        }
        let results = p.results.get();
        if !results.is_empty() {
            let mut row = Flex::new().direction(FlexDirection::Row).wrap().gap(8.0);
            for f in results {
                let open = f.clone();
                if is_video(&f) {
                    row = row.child(video_tile(f.clone()));
                    continue;
                }
                if synassist::attach::kind(&f) == synassist::attach::Kind::Audio {
                    row = row.child(audio_row(f.clone()));
                    continue;
                }
                let tile: Box<dyn Widget> = if is_image(&f) {
                    Box::new(DecoratedBox::new().child(Image::new(f.to_string_lossy()).fit(ImageFit::Cover).class("assist-result-img")).class("assist-result").clip(true))
                } else if is_video(&f) {
                    match video_poster(&f) {
                        Some(poster) => Box::new(DecoratedBox::new().child(Image::new(poster.to_string_lossy()).fit(ImageFit::Cover).class("assist-result-img")).class("assist-result").clip(true)),
                        None => Box::new(Chip::new(f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).icon(PLAY).class("assist-att")),
                    }
                } else {
                    let glyph = if synassist::attach::kind(&f) == synassist::attach::Kind::Audio { AUDIO } else { ATTACH };
                    Box::new(Chip::new(f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).icon(glyph).class("assist-att"))
                };
                let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                row = row.child(tip(
                    crate::ui::InputArea::new(tile).pointer().on_click(move |b, _, _| {
                        if b == syngui::input::MouseButton::Left {
                            let _ = std::process::Command::new("xdg-open").arg(&open).spawn();
                        }
                    }),
                    name,
                ));
            }
            col = col.child(DecoratedBox::new().child(row).class("assist-results"));
        }
        if !busy && p.chat_id.get().is_some() {
            let a1 = answer.clone();
            let a2 = answer.clone();
            let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
            if !answer.is_empty() {
                row = row.child(tip(Button::new("").icon(COPY).class("btn assist-act small").on_click(move || syngui::clipboard::copy(&a1)), t!("Копировать")));
                if speaking {
                    row = row.child(tip(Button::new("").icon(STOP).class("btn assist-act small").on_click(|| synassist_voice::stop_speaking()), t!("Остановить озвучку")));
                } else {
                    row = row.child(tip(Button::new("").icon(VOLUME_UP).class("btn assist-act small").on_click(move || speak(p, synassist_voice::strip_markdown(&a2))), t!("Прочитать вслух")));
                }
            }
            row = row
                .child(DecoratedBox::new().class("grow"))
                .child(Button::new(t!("Новый вопрос")).icon(ADD).class("btn assist-ghost").on_click(move || {
                    p.chat_id.set(None);
                    p.answer.set(String::new());
                    p.attachments.set(Vec::new());
                    p.results.set(Vec::new());
                    p.heard.set(String::new());
                    p.input_rev.set(p.input_rev.get_untracked() + 1);
                }))
                .child(Button::new(t!("Открыть чат")).icon(OPEN).class("btn btn-primary").on_click(move || open_window(p)));
            if !answer.is_empty() {
                col = col.child(DecoratedBox::new().class("assist-sep"));
            }
            col = col.child(row);
        }
        if col_is_empty(busy, &p) {
            return Box::new(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(t!("Управление системой, файлы, интернет и медиа — на нейросетях этого устройства.")).max_lines(2).class("assist-hint grow"))
                    .child(Text::new(t!("Esc — закрыть")).class("assist-hint dim")),
            ) as Box<dyn Widget>;
        }
        Box::new(col)
    });
    Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(DecoratedBox::new().child(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(header(p)).child(attachments_row(p))).class("search-box assist-box"))
        .child(AnimatedSize::new(body).axis(AnimationAxis::Height).spring(420.0, 40.0).class("popup-morph"))
}

/// Ход долгого инструмента: этап, полоса и проценты (или бегущая полоса).
fn progress_row(frac: Option<f32>, label: &str) -> Box<dyn Widget> {
    let bar = match frac {
        Some(f) => ProgressBar::new().value(f.clamp(0.0, 1.0)),
        None => ProgressBar::new().indeterminate(),
    }
    .class("assist-progress grow");
    let pct = frac.map(|f| format!("{:.0}%", f * 100.0)).unwrap_or_default();
    Box::new(
        Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("assist-progress-box")
            .child(Row::new().gap(8.0).child(Text::new(label.to_string()).max_lines(1).class("assist-hint grow")).child(Text::new(pct).class("assist-hint")))
            .child(bar),
    )
}

fn col_is_empty(busy: bool, p: &Panel) -> bool {
    !busy && p.answer.get_untracked().is_empty() && p.confirm.get_untracked().is_none() && p.chat_id.get_untracked().is_none()
}
