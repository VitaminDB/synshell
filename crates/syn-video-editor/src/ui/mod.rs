//! Окно как в hexspire: заголовок с меню и палитрой команд, панель
//! инструментов, док-панели, строка состояния. Любое действие окна —
//! команда из [`crate::cmd`] (через [`St::exec`]), как у агента.

pub mod graph;
mod panels;
mod thumbs;
pub mod timeline;

use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use serde_json::{json, Value};
use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::DragData;
use syngui::widgets::overlay::MenuItem;
use syngui::widgets::*;
use synshell_common::config::Rgba;
use synshell_common::Config;

use crate::agent::Shared;
use crate::cmd;
use crate::editor::Editor;

pub mod icons {
    pub const FOLDER_OPEN: &str = "\u{E2C8}";
    pub const SAVE: &str = "\u{E161}";
    pub const UNDO: &str = "\u{E166}";
    pub const REDO: &str = "\u{E15A}";
    pub const SEARCH: &str = "\u{E8B6}";
    pub const CUT: &str = "\u{E14E}";
    pub const TITLE: &str = "\u{E264}";
    pub const PLAY: &str = "\u{E037}";
    pub const PAUSE: &str = "\u{E034}";
    pub const AI: &str = "\u{E65F}";
    pub const MOVIE: &str = "\u{E02C}";
    pub const AUDIO: &str = "\u{E405}";
    pub const IMAGE: &str = "\u{E3F4}";
    pub const ADD: &str = "\u{E145}";
    pub const SEND: &str = "\u{E163}";
    pub const STOP: &str = "\u{E047}";
    pub const FULLSCREEN: &str = "\u{E5D0}";
    pub const FULLSCREEN_EXIT: &str = "\u{E5D1}";
    pub const ZOOM_IN: &str = "\u{E8FF}";
    pub const ZOOM_OUT: &str = "\u{E900}";
    pub const FAVORITE: &str = "\u{E87D}";
}

static EDITOR: OnceLock<Shared> = OnceLock::new();

pub fn shared() -> &'static Shared {
    EDITOR.get_or_init(|| Arc::new(Mutex::new(Editor::new())))
}

pub fn ed() -> MutexGuard<'static, Editor> {
    shared().lock().unwrap_or_else(|e| e.into_inner())
}

/// Команды, об успехе которых стоит сказать уведомлением (остальные видны сами).
const LOUD: &[&str] = &["project.save", "project.open", "project.new", "project.export", "frame.export", "media.import", "media.transcribe", "clip.cut_silence", "clip.cut_ranges", "template.apply", "edit.undo", "edit.redo"];

/// Какой диалог выбора файлов открыть.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    ImportFiles,
    ImportFolder,
    Open,
    SaveAs,
    Export,
}

/// Строка панели ИИ.
#[derive(Clone, Debug, PartialEq)]
pub enum AiLine {
    User(String),
    Text(String),
    Tool(String),
    Result(String),
    Error(String),
}

#[derive(Clone, Copy)]
pub struct St {
    /// Номер правки редактора: панели перечитывают документ по нему.
    pub rev: RwSignal<u64>,
    pub window: RwSignal<syngui::window::WindowState>,
    pub theme: RwSignal<String>,
    pub palette_open: RwSignal<bool>,
    pub dock: DockState,
    /// Пикселей на секунду таймлайна.
    pub zoom: RwSignal<f32>,
    pub toast: RwSignal<Option<String>>,
    pub ai_log: RwSignal<Vec<AiLine>>,
    pub ai_busy: RwSignal<bool>,
    pub view: RwSignal<String>,
    pub playing: RwSignal<bool>,
    /// Готовы новые ленты превью медиатеки.
    pub thumbs: RwSignal<u64>,
    /// Идёт экспорт: доля готовности.
    pub exporting: RwSignal<Option<f32>>,
    /// Ширина плиток медиатеки, пикс. (ползунок).
    pub tile_size: RwSignal<f32>,
    /// Превью броска на таймлайн (место, куда встанет клип).
    pub drop_hint: RwSignal<Option<timeline::Hint>>,
    /// Идёт обрезка края клипа: клип, левый ли край, сдвиг мыши (пикс.).
    pub trim: RwSignal<Option<(u64, bool, f32)>>,
}

impl St {
    pub fn new() -> Self {
        let st = Self::make();
        thumbs::bind(st.thumbs);
        st
    }

    fn make() -> Self {
        Self {
            rev: use_signal(0),
            window: use_signal(syngui::window::WindowState::default()),
            theme: use_signal(String::new()),
            palette_open: use_signal(false),
            dock: DockState::new(layout("edit")),
            zoom: use_signal(60.0),
            toast: use_signal(None),
            ai_log: use_signal(Vec::new()),
            ai_busy: use_signal(false),
            view: use_signal("edit".to_string()),
            playing: use_signal(false),
            thumbs: use_signal(0),
            exporting: use_signal(None),
            tile_size: use_signal(110.0),
            drop_hint: use_signal(None),
            trim: use_signal(None),
        }
    }

    /// Выполнить команду; ошибка — во всплывающее сообщение.
    pub fn exec(self, id: &str, args: Value) -> Option<cmd::Out> {
        let r = {
            let mut e = ed();
            let r = cmd::run(&mut e, id, &args);
            self.rev.set(e.revision);
            r
        };
        match r {
            Ok(o) => {
                // удачные правки видны сами (таймлайн, просмотр) — уведомление только об итогах
                if !o.text.is_empty() && LOUD.contains(&id) {
                    self.toast.set(Some(o.text.lines().next().unwrap_or_default().to_string()));
                }
                Some(o)
            }
            Err(e) => {
                self.toast.set(Some(e));
                None
            }
        }
    }

    /// Команда из меню, палитры или клавиш: без аргументов — сразу; с
    /// обязательными аргументами — просьба к агенту в панели ИИ.
    pub fn run_menu(self, id: &str) {
        match id {
            "view.palette" => return self.palette_open.set(true),
            "view.fullscreen" => return syngui::signal::toggle_fullscreen(),
            "view.ai" => return self.dock.open("ai", Some("inspector")),
            v if v.starts_with("view.layout.") => return self.set_view(&v["view.layout.".len()..]),
            // свои диалоги вместо аргументов
            "media.import" => return self.dialog(Pick::ImportFiles),
            "media.import_folder" => return self.dialog(Pick::ImportFolder),
            "project.open" => return self.dialog(Pick::Open),
            "project.save_as" => return self.dialog(Pick::SaveAs),
            "project.save" if ed().path.is_none() => return self.dialog(Pick::SaveAs),
            "project.export" => return self.dialog(Pick::Export),
            _ => {}
        }
        let Some(c) = cmd::find(id) else { return };
        if c.args.iter().any(|a| a.required) {
            self.dock.open("ai", Some("inspector"));
            self.toast.set(Some(format!("«{}»: скажите агенту, что именно (панель ИИ)", c.title)));
            return;
        }
        let args = if id == "project.save" && ed().path.is_none() { json!({"path": default_path()}) } else { json!({}) };
        self.exec(id, args);
    }

    /// Пуск и пауза: позиция идёт с частотой проекта, кадры декодирует просмотр.
    pub fn toggle_play(self) {
        if self.playing.get_untracked() {
            self.playing.set(false);
            return;
        }
        {
            let mut e = ed();
            if e.playhead >= e.project.duration() - 1e-3 {
                e.playhead = 0.0;
            }
        }
        self.playing.set(true);
        let fps = ed().project.settings.fps.max(1.0);
        std::thread::spawn(move || {
            // звук от позиции (до 5 мин вперёд) — тем же сведением, что экспорт; часы — по звуку
            let (project, from) = {
                let e = ed();
                (e.project.clone(), e.playhead)
            };
            let t1 = project.duration().min(from + 300.0);
            let player = match crate::render::export::mix_pcm(&project, from, t1) {
                Ok(Some(pcm)) => {
                    tracing::info!("звук: {:.1} с с {:.2}", pcm.len() as f64 / 2.0 / project.settings.sample_rate as f64, from);
                    syngui::audio::AudioPlayer::start_stereo(pcm.into(), project.settings.sample_rate).map_err(|e| tracing::warn!("звук: {e}")).ok()
                }
                Ok(None) => {
                    tracing::info!("звука на таймлайне нет");
                    None
                }
                Err(e) => {
                    tracing::warn!("сведение звука: {e}");
                    None
                }
            };
            let start = std::time::Instant::now();
            let clock = move |p: &Option<syngui::audio::AudioPlayer>| p.as_ref().and_then(|p| p.playback_clock_sec()).unwrap_or_else(|| start.elapsed().as_secs_f64());
            let mut n = 0u64;
            loop {
                n += 1;
                let next = start + std::time::Duration::from_secs_f64(n as f64 / fps);
                if let Some(d) = next.checked_duration_since(std::time::Instant::now()) {
                    std::thread::sleep(d);
                }
                let now = clock(&player);
                let (tx, rx) = std::sync::mpsc::channel();
                syngui::async_runtime::run_on_main_thread(move || {
                    if !self.playing.get_untracked() {
                        let _ = tx.send(false);
                        return;
                    }
                    let mut e = ed();
                    let end = e.project.duration();
                    let t = (from + now).min(end);
                    e.playhead = (t * fps).floor() / fps;
                    e.revision += 1;
                    let rev = e.revision;
                    drop(e);
                    self.rev.set(rev);
                    if t >= end {
                        self.playing.set(false);
                    }
                    let _ = tx.send(t < end);
                });
                if !rx.recv().unwrap_or(false) {
                    break;
                }
            }
            if let Some(p) = player {
                p.stop();
            }
        });
    }

    /// Шаг позиции на `frames` кадров.
    pub fn step(self, frames: f64) {
        let t = {
            let e = ed();
            (e.playhead + frames / e.project.settings.fps.max(1.0)).max(0.0)
        };
        self.exec("playhead.set", json!({"at": t}));
        self.toast.set(None);
    }

    /// Выбор файлов через портал — в своём потоке, окно не замирает.
    pub fn dialog(self, kind: Pick) {
        static OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if OPEN.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let start = ed().path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf())).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Videos")));
        let name = format!("{}.syn", ed().project.name.replace('/', "_"));
        tracing::info!("диалог {kind:?}");
        std::thread::spawn(move || {
            let mut d = rfd::FileDialog::new();
            if let Some(dir) = start.filter(|d| d.is_dir()) {
                d = d.set_directory(dir);
            }
            let picked: Vec<std::path::PathBuf> = match kind {
                Pick::ImportFiles => {
                    let mut exts: Vec<&str> = Vec::new();
                    exts.extend(cmd::media::VIDEO);
                    exts.extend(cmd::media::AUDIO);
                    exts.extend(cmd::media::IMAGE);
                    d.set_title("Импорт медиа").add_filter("Медиафайлы", &exts).add_filter("Все файлы", &["*"]).pick_files().unwrap_or_default()
                }
                Pick::ImportFolder => d.set_title("Импорт папки с медиа").pick_folder().into_iter().collect(),
                Pick::Open => d.set_title("Открыть проект").add_filter("Проект видеоредактора", &["syn"]).pick_file().into_iter().collect(),
                Pick::SaveAs => d.set_title("Сохранить проект").set_file_name(name).add_filter("Проект видеоредактора", &["syn"]).save_file().into_iter().collect(),
                Pick::Export => d.set_title("Экспорт видео").set_file_name(name.replace(".syn", ".mp4")).add_filter("Видео", &["mp4", "mov", "mkv", "webm"]).save_file().into_iter().collect(),
            };
            tracing::info!("диалог {kind:?}: выбрано {}", picked.len());
            OPEN.store(false, std::sync::atomic::Ordering::SeqCst);
            if picked.is_empty() {
                return;
            }
            syngui::async_runtime::run_on_main_thread(move || {
                let paths: Vec<String> = picked.iter().map(|p| p.display().to_string()).collect();
                match kind {
                    Pick::ImportFiles | Pick::ImportFolder => self.import(paths),
                    Pick::Open => {
                        self.exec("project.open", json!({"path": paths[0]}));
                    }
                    Pick::SaveAs => {
                        self.exec("project.save", json!({"path": paths[0]}));
                    }
                    Pick::Export => self.export(paths[0].clone()),
                }
            });
        });
    }

    /// Экспорт в фоне: ход — в строке состояния, итог — уведомлением.
    pub fn export(self, path: String) {
        if self.exporting.get_untracked().is_some() {
            return;
        }
        self.exporting.set(Some(0.0));
        let project = ed().project.clone();
        std::thread::spawn(move || {
            let r = crate::render::run_child(&project, &["export".into(), path.clone(), "18".into()], |i, n| {
                let f = i as f32 / n.max(1) as f32;
                syngui::async_runtime::run_on_main_thread(move || self.exporting.set(Some(f)));
            });
            syngui::async_runtime::run_on_main_thread(move || {
                self.exporting.set(None);
                match r {
                    Ok(s) => self.toast.set(Some(format!("Экспорт готов: {s}"))),
                    Err(e) => self.toast.set(Some(format!("Ошибка экспорта: {e}"))),
                }
            });
        });
    }

    /// Импорт файлов и папок (диалог, перетаскивание).
    pub fn import(self, paths: Vec<String>) {
        if paths.is_empty() {
            return;
        }
        self.exec("media.import", json!({"paths": paths}));
        self.dock.activate("media");
    }

    pub fn set_view(self, key: &str) {
        self.view.set(key.to_string());
        self.dock.set_spec(layout(key));
    }

    /// Ход агента в фоне; события — в панель ИИ, правки — в окно.
    pub fn ask_ai(self, text: String) {
        if self.ai_busy.get_untracked() || text.trim().is_empty() {
            return;
        }
        self.ai_busy.set(true);
        self.ai_log.update(|l| l.push(AiLine::User(text.clone())));
        std::thread::spawn(move || {
            use syn_assist::agent::AgentEvent;
            static AGENT: OnceLock<syn_assist::agent::Agent> = OnceLock::new();
            static CHAT: OnceLock<Mutex<syn_assist::chat::Chat>> = OnceLock::new();
            let agent = AGENT.get_or_init(|| crate::agent::agent(shared()));
            let mut chat = CHAT.get_or_init(|| Mutex::new(syn_assist::chat::Chat::new())).lock().unwrap_or_else(|e| e.into_inner());
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let push = move |line: AiLine| {
                syngui::async_runtime::run_on_main_thread(move || {
                    self.ai_log.update(|l| match (&line, l.last_mut()) {
                        (AiLine::Text(t), Some(AiLine::Text(prev))) => prev.push_str(t),
                        _ => l.push(line),
                    });
                    self.rev.set(ed().revision);
                });
            };
            let r = crate::agent::run_turn(agent, shared(), &mut chat, &text, &crate::agent::AutoConfirm(false), &cancel, &mut |ev| match ev {
                AgentEvent::Text(t) => push(AiLine::Text(t)),
                AgentEvent::ToolStart { summary, .. } => push(AiLine::Tool(summary)),
                AgentEvent::Message(m) if m.role == syn_assist::chat::Role::Tool => push(AiLine::Result(m.text.lines().next().unwrap_or_default().to_string())),
                _ => {}
            });
            if let Err(e) = r {
                push(AiLine::Error(e));
            }
            syngui::async_runtime::run_on_main_thread(move || {
                self.ai_busy.set(false);
                self.rev.set(ed().revision);
            });
        });
    }
}

fn default_path() -> String {
    let dir = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default().join("Videos");
    let name = ed().project.name.replace('/', "_");
    dir.join(format!("{name}.syn")).display().to_string()
}

/// Раскладки видов (позже — сценарии Rhai `library/layouts`, как в hexspire).
pub const VIEWS: &[(&str, &str)] = &[("edit", "Монтаж"), ("graph", "Граф"), ("ai", "ИИ-монтаж")];

pub fn layout(key: &str) -> DockSpec {
    let left = DockSpec::tabs(["media", "effects"]);
    let right = DockSpec::tabs(["inspector", "ai"]);
    match key {
        "graph" => DockSpec::column(DockSpec::row(DockSpec::tabs(["media", "effects"]), DockSpec::row(DockSpec::tabs(["graph"]).named("graph"), DockSpec::tabs(["viewer", "inspector", "ai"]), 0.7), 0.2), DockSpec::tabs(["timeline"]), 0.7),
        "ai" => DockSpec::row(
            DockSpec::column(DockSpec::row(left, DockSpec::tabs(["viewer"]).named("viewer"), 0.32), DockSpec::tabs(["timeline"]), 0.58),
            DockSpec::tabs(["ai", "inspector"]),
            0.68,
        ),
        _ => DockSpec::column(DockSpec::row(DockSpec::row(left, DockSpec::tabs(["viewer", "graph"]).named("viewer"), 0.26), right, 0.74), DockSpec::tabs(["timeline"]), 0.58),
    }
}

pub fn panel_title(id: &str) -> String {
    match id {
        "media" => "Медиатека",
        "effects" => "Эффекты",
        "viewer" => "Просмотр",
        "timeline" => "Таймлайн",
        "inspector" => "Инспектор",
        "ai" => "ИИ",
        "graph" => "Граф",
        other => other,
    }
    .to_string()
}

// ── тема: стили hexspire на палитре synshell ─────────────────────────────

pub fn theme(cfg: &Config) -> String {
    use synshell_common::style::AppStyle;
    let a = &cfg.appearance;
    let p = a.palette();
    let dark = a.is_dark();
    let black = Rgba::rgb(0, 0, 0);
    let white = Rgba::rgb(255, 255, 255);
    let shell = if dark { p.bg.mix(black, 0.25) } else { p.bg };
    let panel = if dark { p.bg.mix(p.surface, 0.35) } else { p.surface };
    let panel2 = if dark { p.surface } else { p.surface_alt };
    let field = if dark { p.surface.mix(white, 0.04) } else { p.surface_alt.mix(p.bg, 0.3) };
    let hover = if dark { white.with_alpha(0.06) } else { black.with_alpha(0.05) };
    let active = if dark { white.with_alpha(0.09) } else { black.with_alpha(0.08) };
    let sub = p.muted.mix(p.bg, 0.35);
    AppStyle::new(cfg, "syn-video-editor")
        .vars(&[
            ("bg-window", "#00000000".into()),
            ("bg-shell", shell.hex()),
            ("bg-panel", panel.hex()),
            ("bg-panel-2", panel2.hex()),
            ("bg-field", field.hex()),
            ("bg-hex", shell.mix(panel, 0.4).hex()),
            ("text", p.fg.hex()),
            ("text-muted", p.muted.hex()),
            ("text-subtle", sub.hex()),
            ("primary", p.accent.hex()),
            ("primary-hover", p.accent.mix(white, 0.15).hex()),
            ("primary-soft", p.accent.with_alpha(0.16).hex()),
            ("on-primary", p.accent_fg.hex()),
            ("success", p.success.hex()),
            ("success-soft", p.success.with_alpha(0.14).hex()),
            ("warning", p.warning.hex()),
            ("warning-soft", p.warning.with_alpha(0.14).hex()),
            ("error", p.danger.hex()),
            ("modified", p.warning.hex()),
            ("border", p.border.hex()),
            ("border-soft", p.border.mix(shell, 0.4).hex()),
            ("surface-hover", hover.hex()),
            ("surface-active", active.hex()),
            ("hover", hover.hex()),
            ("radius-shell", "14px".into()),
            ("radius-card", "12px".into()),
            ("radius-field", "9px".into()),
            ("radius-pill", "999px".into()),
            ("clip-video", p.accent.mix(shell, 0.45).hex()),
            ("clip-audio", p.success.mix(shell, 0.5).hex()),
            ("clip-title", Rgba::rgb(0x9b, 0x6b, 0xf2).mix(shell, 0.4).hex()),
            ("clip-color", p.warning.mix(shell, 0.5).hex()),
            // редактор узлов (syn-node-editor) — в цветах темы
            ("ne-canvas-bg", shell.mix(panel, 0.35).hex()),
            ("ne-grid", p.muted.with_alpha(0.22).hex()),
            ("ne-panel-bg", panel2.hex()),
            ("ne-panel-hover", panel2.mix(p.fg, 0.06).hex()),
            ("ne-text", p.fg.mix(p.muted, 0.3).hex()),
            ("ne-text-strong", p.fg.hex()),
            ("ne-text-muted", p.muted.hex()),
            ("ne-node-bg", panel2.hex()),
            ("ne-node-header", panel.hex()),
            ("ne-hover", hover.hex()),
            ("bg-popup", panel2.hex()),
            ("border-strong", p.border.mix(p.fg, 0.15).hex()),
            ("radius-panel", "12px".into()),
            ("duration-fast", "120ms".into()),
            ("duration-med", "200ms".into()),
            ("ease-standard", "ease-out".into()),
        ])
        .base(&format!("{}\n{}", concat!(
            include_str!("../../styles/base/reset.mss"),
            "\n",
            include_str!("../../styles/base/layout.mss"),
            "\n",
            include_str!("../../styles/base/controls.mss"),
            "\n",
            include_str!("../../styles/components/shell.mss"),
            "\n",
            include_str!("../../styles/components/toolbar.mss"),
            "\n",
            include_str!("../../styles/components/side_panel.mss"),
            "\n",
            include_str!("../../styles/components/dock.mss"),
            "\n",
            include_str!("../../styles/components/statusbar.mss"),
            "\n",
            include_str!("../../styles/app.mss"),
        ), syn_node_editor::STYLES))
        .build()
}

// ── шелл окна ───────────────────────────────────────────────────────────

pub fn root(st: St) -> impl Widget {
    let shell = DecoratedBox::new().class("window-backdrop").child(
        DecoratedBox::new().clip(true).class("shell").child(
            Column::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(titlebar(st))
                .child(toolbar(st))
                .child(DecoratedBox::new().class("grow").class("dock-area-pad").child(dock(st)))
                .child(statusbar(st)),
        ),
    );
    let hotkeys = EventHook::new()
        .on_key_down(move |key, mods| {
            if key == Key::F11 {
                syngui::signal::toggle_fullscreen();
                return KeyReply::Handled;
            }
            if key == Key::Escape && st.window.get_untracked().fullscreen {
                syngui::signal::set_fullscreen(false);
                return KeyReply::Handled;
            }
            if !mods.ctrl && !mods.alt && !mods.shift {
                match key {
                    Key::Space => {
                        st.toggle_play();
                        return KeyReply::Handled;
                    }
                    Key::Left => {
                        st.step(-1.0);
                        return KeyReply::Handled;
                    }
                    Key::Right => {
                        st.step(1.0);
                        return KeyReply::Handled;
                    }
                    Key::Home => {
                        st.exec("playhead.set", json!({"at": 0}));
                        return KeyReply::Handled;
                    }
                    _ => {}
                }
            }
            if !mods.ctrl && !mods.alt {
                return KeyReply::Ignore;
            }
            match by_chord(key, mods) {
                Some(id) => {
                    st.run_menu(id);
                    KeyReply::Handled
                }
                None => KeyReply::Ignore,
            }
        })
        .child(WindowResizeRegion::new().inset(10.0).enabled(!st.window.get_untracked().fullscreen).child(shell));
    // на всё окно: слой подсказок (внизу справа) и палитра считают от его краёв
    Stack::new()
        .fit(StackFit::Expand)
        .clip(false)
        .child(hotkeys)
        .child(CommandPalette::new(st.palette_open).placeholder("Команда… (можно в другой раскладке)").commands(palette).on_run(move |id| st.run_menu(id)).build())
        .child(toast(st))
}

/// Пункты меню вида и окна (не команды документа).
const VIEW_ITEMS: &[(&str, &str, &str)] = &[("view.palette", "Палитра команд", "Ctrl+Shift+P"), ("view.ai", "Панель ИИ", "Ctrl+Shift+A"), ("view.fullscreen", "Во весь экран", "F11")];

/// Действия окна без команды документа: (id, раздел меню, заголовок, клавиши).
const UI_ITEMS: &[(&str, &str, &str, &str)] = &[("media.import_folder", "Медиа", "Импорт папки…", "Ctrl+Shift+I"), ("project.save_as", "Проект", "Сохранить как…", "Ctrl+Shift+S")];

fn by_chord(key: Key, mods: Modifiers) -> Option<&'static str> {
    let name = format!("{key:?}");
    let name = name.strip_prefix("Key").unwrap_or(&name).to_string();
    let mut chord = String::new();
    if mods.ctrl {
        chord.push_str("Ctrl+");
    }
    if mods.shift {
        chord.push_str("Shift+");
    }
    if mods.alt {
        chord.push_str("Alt+");
    }
    chord.push_str(&name.to_uppercase());
    let eq = |s: &str| s.eq_ignore_ascii_case(&chord);
    if let Some(v) = VIEW_ITEMS.iter().find(|v| eq(v.2)) {
        return Some(v.0);
    }
    if let Some(v) = UI_ITEMS.iter().find(|v| eq(v.3)) {
        return Some(v.0);
    }
    cmd::all().into_iter().find(|c| eq(c.shortcut)).map(|c| c.id)
}

fn palette() -> Vec<PaletteCommand> {
    let mut v: Vec<PaletteCommand> = cmd::all().into_iter().map(|c| PaletteCommand::new(c.id, c.title).category(c.category).shortcut(c.shortcut).keywords(c.keywords).description(c.help)).collect();
    for (id, t, k) in VIEW_ITEMS {
        v.push(PaletteCommand::new(*id, *t).category("Вид").shortcut(*k));
    }
    for (id, cat, t, k) in UI_ITEMS {
        v.push(PaletteCommand::new(*id, *t).category(*cat).shortcut(*k));
    }
    for (k, t) in VIEWS {
        v.push(PaletteCommand::new(format!("view.layout.{k}"), format!("Вид: {t}")).category("Вид").keywords("layout раскладка вид"));
    }
    v
}

fn menus() -> Vec<(&'static str, Vec<MenuItem>)> {
    let mut out: Vec<(&'static str, Vec<MenuItem>)> = Vec::new();
    for c in cmd::all() {
        let item = MenuItem::new(c.id, c.title).shortcut(c.shortcut);
        match out.iter_mut().find(|(t, _)| *t == c.category) {
            Some((_, items)) => items.push(item),
            None => out.push((c.category, vec![item])),
        }
    }
    for (id, cat, t, k) in UI_ITEMS {
        if let Some((_, items)) = out.iter_mut().find(|(c, _)| c == cat) {
            items.push(MenuItem::new(*id, *t).shortcut(*k));
        }
    }
    let mut view: Vec<MenuItem> = VIEW_ITEMS.iter().map(|(id, t, k)| MenuItem::new(*id, *t).shortcut(*k)).collect();
    view.push(MenuItem::separator());
    view.extend(VIEWS.iter().map(|(k, t)| MenuItem::new(format!("view.layout.{k}"), format!("Вид: {t}"))));
    out.push(("Вид", view));
    out
}

fn titlebar(st: St) -> impl Widget {
    let mut menu = MenuBar::new().on_select(move |id| st.run_menu(id));
    for (title, items) in menus() {
        menu = menu.menu(title, move || items.clone());
    }
    let controls = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let w = st.window.get();
        vec![Box::new(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    WindowDragRegion::new().child(
        DecoratedBox::new().class("titlebar").child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(DecoratedBox::new().class("logo-mark").child(Icon::new(icons::MOVIE).class("logo-icon")))
                        .child(Text::new("Видеоредактор").class("logo-text")),
                )
                .child(build_chip())
                .child(menu.build())
                .child(DecoratedBox::new().class("grow"))
                .child(
                    GestureDetector::new().on_click(move || st.palette_open.set(true)).child(
                        DecoratedBox::new().class("palette-pill").child(
                            Row::new()
                                .gap(8.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(Icon::new(icons::SEARCH).class("palette-pill-icon"))
                                .child(DecoratedBox::new().class("grow").child(Text::new("Команды, медиа, эффекты…").class("palette-pill-text")))
                                .child(DecoratedBox::new().class("kbd").child(Text::new("Ctrl+Shift+P").class("kbd-text"))),
                        ),
                    ),
                )
                .child(DecoratedBox::new().class("grow"))
                .child(link_chip(icons::FAVORITE, "donate", "Donate", "Поддержать разработку", || { let _ = syngui::open_url(DONATE_URL); }))
                .child(link_chip("", "github", "GitHub", "Исходники synshell на GitHub", || { let _ = syngui::open_url(GITHUB_URL); }))
                .child(DecoratedBox::new().class("window-controls").child(controls)),
        ),
    )
}

fn sep() -> impl Widget {
    DecoratedBox::new().class("toolbar-sep")
}

fn toolbar(st: St) -> impl Widget {
    let cmd_btn = move |icon: &'static str, tip: &'static str, id: &'static str| ToolButton::new(icon).tooltip(tip).on_click(move || st.run_menu(id));
    DecoratedBox::new().class("toolbar").child(
        Row::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(cmd_btn(icons::FOLDER_OPEN, "Открыть… (Ctrl+O)", "project.open"))
            .child(cmd_btn(icons::SAVE, "Сохранить (Ctrl+S)", "project.save"))
            .child(sep())
            .child(move || {
                let _ = st.rev.get();
                let (u, r) = {
                    let e = ed();
                    (e.undo_label().map(str::to_string), e.redo_label().map(str::to_string))
                };
                Row::new()
                    .gap(4.0)
                    .child(ToolButton::new(icons::UNDO).tooltip(u.as_ref().map(|l| format!("Отменить: {l} (Ctrl+Z)")).unwrap_or_else(|| "Отменить (Ctrl+Z)".into())).disabled(u.is_none()).on_click(move || st.run_menu("edit.undo")))
                    .child(ToolButton::new(icons::REDO).tooltip(r.as_ref().map(|l| format!("Повторить: {l} (Ctrl+Shift+Z)")).unwrap_or_else(|| "Повторить (Ctrl+Shift+Z)".into())).disabled(r.is_none()).on_click(move || st.run_menu("edit.redo")))
            })
            .child(sep())
            .child(move || {
                let on = st.playing.get();
                ToolButton::new(if on { icons::PAUSE } else { icons::PLAY }).tooltip(if on { "Пауза (Пробел)" } else { "Воспроизвести (Пробел)" }).on_click(move || st.toggle_play())
            })
            .child(sep())
            .child(ToolButton::new(icons::CUT).tooltip("Разрезать в позиции (S)").on_click(move || {
                st.exec("clip.split", json!({}));
            }))
            .child(ToolButton::new(icons::TITLE).tooltip("Титр в позиции (Ctrl+T)").on_click(move || {
                st.exec("title.add", json!({"text": "Заголовок"}));
            }))
            .child(sep())
            .child(ToolButton::new(icons::ZOOM_OUT).tooltip("Мельче").on_click(move || st.zoom.update(|z| *z = (*z / 1.5).max(4.0))))
            .child(ToolButton::new(icons::ZOOM_IN).tooltip("Крупнее").on_click(move || st.zoom.update(|z| *z = (*z * 1.5).min(800.0))))
            .child(DecoratedBox::new().class("grow"))
            .child(
                GestureDetector::new().on_click(move || st.dock.open("ai", Some("inspector"))).child(
                    DecoratedBox::new().class("tool-pill").child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icons::AI).class("tool-pill-icon")).child(Text::new("ИИ-монтаж").class("tool-pill-text"))),
                ),
            )
            .child(move || {
                let cur = st.view.get();
                Dropdown::new().items(VIEWS.iter().map(|(k, t)| DropdownItem::new(*k, *t)).collect()).selected(cur).on_change(move |k: &str| st.set_view(k)).class("view-select")
            })
            .child(move || {
                let fs = st.window.get().fullscreen;
                ToolButton::new(if fs { icons::FULLSCREEN_EXIT } else { icons::FULLSCREEN }).tooltip(if fs { "Выйти из полноэкранного режима (F11)" } else { "Во весь экран (F11)" }).on_click(syngui::signal::toggle_fullscreen)
            }),
    )
}

fn dock(st: St) -> impl Widget {
    // файлы из проводника — в медиатеку, проект .syn — открыть
    DropArea::new()
        .accept_types(vec![DragData::TYPE_FILE.to_string()])
        .on_drop(move |d: DragData| {
            let paths: Vec<String> = d.payload.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
            match paths.iter().find(|p| p.ends_with(".syn")) {
                Some(p) => {
                    st.exec("project.open", json!({"path": p}));
                }
                None => st.import(paths),
            }
        })
        .child(DockArea::new(st.dock).inset(8.0, 0.0, 8.0, 0.0).content(move |panel| panels::content(st, panel)).title(|panel| panel_title(panel)).build())
}

fn statusbar(st: St) -> impl Widget {
    DecoratedBox::new().class("statusbar").child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let (left, right) = {
            let e = ed();
            let s = &e.project.settings;
            let file = e.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "не сохранён".into());
            (format!("{}{} — {}", e.project.name, if e.dirty { " ●" } else { "" }, file), format!("{}{} / {}  ·  {}×{}  ·  {} fps", &format!("{}{}", st.exporting.get().map(|f| format!("Экспорт {:.0} %  ·  ", f * 100.0)).unwrap_or_default(), if st.ai_busy.get() { "ИИ работает…  ·  " } else { "" }), crate::doc::tc(e.playhead), crate::doc::tc(e.project.duration()), s.width, s.height, s.fps))
        };
        vec![Box::new(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(left).max_lines(1).elide(Elide::End).class("status-text")).child(DecoratedBox::new().class("grow")).child(Text::new(right).class("status-text")))]
    }))
}

/// Уведомления syngui (карточки внизу справа, сами исчезают). Сигнал `toast`
/// — мост: любое новое сообщение уходит сюда.
fn notifications() -> &'static NotificationCtx {
    static N: OnceLock<NotificationCtx> = OnceLock::new();
    N.get_or_init(|| NotificationCtx::with_default_duration(3500))
}

fn toast(st: St) -> impl Widget {
    create_effect(move || {
        if let Some(t) = st.toast.get() {
            if std::env::var_os("SYN_VIDEO_EDITOR_TOAST").is_none() {
                syngui::async_runtime::run_on_main_thread(move || st.toast.set(None));
            }
            let low = t.to_lowercase();
            if low.contains("ошибк") || low.contains("нет ") || low.contains("не ") && low.contains(':') {
                notifications().warning(t);
            } else {
                notifications().info(t);
            }
        }
    });
    let open = use_signal(true);
    Portal::new()
        .is_open(open)
        .modal(false)
        .backdrop(false)
        .anchor(PortalAnchor::BottomEnd { margin_bottom: 40.0, margin_right: 24.0 })
        .child(NotificationHost::new(notifications().clone()).grow_up(true).class("notifications"))
}

// ── чипы заголовка ──────────────────────────────────────────────────────

/// Куда ведут чипы — те же ссылки, что в Synthos и README.
pub const DONATE_URL: &str = "https://paypal.me/vitamindbnfkz";
pub const GITHUB_URL: &str = "https://github.com/VitaminDB/synshell";
const GITHUB_MARK_SVG: &[u8] = include_bytes!("../../data/icons/github-mark.svg");

/// Номер сборки (число коммитов = pkgrel пакета): видно, на какой версии получен результат.
fn build_chip() -> impl Widget {
    let tip = format!("Видеоредактор {} · сборка {} · {}", env!("CARGO_PKG_VERSION"), env!("SVE_BUILD"), env!("SVE_COMMIT"));
    Tooltip::new(
        DecoratedBox::new().class("build-chip").child(
            Row::new()
                .gap(5.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("build-chip-dot"))
                .child(Text::new(concat!("#", env!("SVE_BUILD"))).class("build-chip-text")),
        ),
        tip,
    )
}

/// Чип-ссылка как в Synthos: значок и подпись, подложка при наведении. Пустой
/// `glyph` — логотип GitHub.
fn link_chip(glyph: &'static str, variant: &'static str, label: &'static str, tooltip: &'static str, on_click: impl FnMut() + Send + 'static) -> impl Widget {
    let hovered = use_signal(false);
    let chip = move || {
        let state = if hovered.get() { " titlebar-chip--hover" } else { "" };
        let icon: Box<dyn Widget> = if glyph.is_empty() {
            Box::new(Image::from_bytes("sve-github-mark", GITHUB_MARK_SVG.to_vec()).fit(ImageFit::Contain).placeholder(false).class("titlebar-chip-logo"))
        } else {
            Box::new(Icon::new(glyph).class("titlebar-chip-icon"))
        };
        DecoratedBox::new().class(format!("titlebar-chip titlebar-chip-{variant}{state}")).child(Center::new().child(Row::new().gap(5.0).cross_axis_alignment(CrossAxisAlignment::Center).children(vec![icon, Box::new(Text::new(label).max_lines(1).class("titlebar-chip-text"))])))
    };
    Tooltip::new(GestureDetector::new().on_hover_change(move |inside| hovered.set(inside)).on_click(on_click).child(Padding::symmetric(0.0, 6.0).child(DecoratedBox::new().child(chip))), tooltip)
}

/// Автосохранение: через 2 с после правки (проект с файлом — в свой файл,
/// без файла — в `autosave.syn`); последний проект запоминается для запуска.
pub fn autosave(st: St) {
    std::thread::spawn(move || {
        let mut saved_rev = u64::MAX;
        let mut saved_path: Option<std::path::PathBuf> = None;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let job = {
                let e = ed();
                if e.revision == saved_rev && e.path == saved_path {
                    None
                } else {
                    Some((e.project.clone(), e.path.clone(), e.revision, e.dirty))
                }
            };
            let Some((project, path, rev, dirty)) = job else { continue };
            if path != saved_path {
                crate::store::remember(path.as_deref());
            }
            if dirty {
                let target = path.clone().unwrap_or_else(crate::store::autosave_path);
                if let Some(d) = target.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                match crate::store::save(&project, &target) {
                    Ok(()) => {
                        if path.is_some() {
                            let mut e = ed();
                            // правок за время сохранения не было — проект чистый
                            if e.revision == rev {
                                e.dirty = false;
                                e.revision += 1;
                                let r = e.revision;
                                drop(e);
                                syngui::async_runtime::run_on_main_thread(move || st.rev.set(r));
                            }
                        }
                    }
                    Err(err) => tracing::warn!("автосохранение: {err}"),
                }
            }
            saved_rev = rev;
            saved_path = path;
        }
    });
}
