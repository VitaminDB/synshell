//! Окно как в hexspire: заголовок с меню и палитрой команд, панель
//! инструментов, док-панели, строка состояния. Любое действие окна —
//! команда из [`crate::cmd`] (через [`St::exec`]), как у агента.

pub mod graph;
mod graph_presets;
pub(super) mod inspector;
mod panels;
pub(super) mod templates;
mod thumbs;
mod wave;
pub mod timeline;
mod fontpick;
mod layouts_ui;
mod luts;
mod framegraph_ui;
pub mod socket;
mod coverage_ui;
pub mod ai_chat;
pub mod settings_page;
mod project_templates;
pub(crate) mod args_dialog;
mod curves;
mod color;
mod xform;

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
    pub const SHAPE: &str = "\u{E574}";
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
    pub const VOLUME_UP: &str = "\u{E050}";
    pub const VOLUME_OFF: &str = "\u{E04F}";
    pub const VISIBLE: &str = "\u{E8F4}";
    pub const HIDDEN: &str = "\u{E8F5}";
    pub const SNAP: &str = "\u{E157}";
    pub const ZOOM_OUT: &str = "\u{E900}";
    pub const FAVORITE: &str = "\u{E87D}";
    pub const DESCRIPTION: &str = "\u{E873}";
    pub const PSYCHOLOGY: &str = "\u{EA4A}";
    pub const BUILD: &str = "\u{E869}";
    pub const HUB: &str = "\u{E9F4}";
    pub const ACCOUNT_TREE: &str = "\u{E97A}";
    pub const TUNE: &str = "\u{E429}";
    pub const REFRESH: &str = "\u{E5D5}";
    pub const RESTORE: &str = "\u{E8B3}";
    pub const COPY: &str = "\u{E14D}";
    pub const EDIT: &str = "\u{E3C9}";
    pub const CHECK: &str = "\u{E5CA}";
    pub const DELETE: &str = "\u{E872}";
    pub const SETTINGS: &str = "\u{E8B8}";
    pub const CODE: &str = "\u{E86F}";
}

static EDITOR: OnceLock<Shared> = OnceLock::new();

pub fn shared() -> &'static Shared {
    EDITOR.get_or_init(|| Arc::new(Mutex::new(Editor::new())))
}

pub fn ed() -> MutexGuard<'static, Editor> {
    shared().lock().unwrap_or_else(|e| e.into_inner())
}

/// Откуда пустили воспроизведение (для «Стоп»).
static PLAY_FROM: std::sync::Mutex<f64> = std::sync::Mutex::new(0.0);

/// Команды, об успехе которых стоит сказать уведомлением (остальные видны сами).
const LOUD: &[&str] = &["project.save", "project.open", "project.new", "project.export", "frame.export", "media.import", "media.transcribe", "clip.cut_silence", "clip.cut_ranges", "template.apply", "edit.undo", "edit.redo", "clip.copy", "clip.paste"];

/// Какой диалог выбора файлов открыть.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    ImportFiles,
    ImportFolder,
    Open,
    SaveAs,
    Export,
}

/// Генерируемый ассет: узел рецепта, вид, элемент k из n, шаг s из t.
#[derive(Clone, Debug, PartialEq)]
pub struct GenTile {
    pub node: String,
    pub op: String,
    pub item: Option<(usize, usize)>,
    pub step: usize,
    pub total: usize,
}

/// Разобрать строку хода рецепта «refs (image) — 2 из 5 · 13/40».
pub fn parse_gen(label: &str) -> Option<GenTile> {
    let (head, step) = match label.rsplit_once(" · ") {
        Some((h, s)) => (h, Some(s)),
        None => (label, None),
    };
    let (nodeop, item) = match head.split_once(" — ") {
        Some((a, b)) => (a, b.split_once(" из ").and_then(|(k, n)| Some((k.trim().parse().ok()?, n.trim().parse().ok()?)))),
        None => (head, None),
    };
    let (node, op) = nodeop.trim().split_once(" (")?;
    let op = op.trim_end_matches(')').to_string();
    if !matches!(op.as_str(), "image" | "video" | "music" | "speak") {
        return None;
    }
    let (s, t) = step.and_then(|x| x.split_once('/')).and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?))).unwrap_or((0, 0));
    Some(GenTile { node: node.to_string(), op, item, step: s, total: t })
}

/// Строка панели ИИ.
#[derive(Clone, Debug, PartialEq)]
pub enum AiLine {
    User(String),
    Text(String),
    Tool(String),
    Result(String),
    Error(String),
    /// Итог хода: что изменилось; `rev` — номер правки сразу после хода
    /// (откат возможен, пока после хода ничего не правили); `undone` — откачен.
    Diff { lines: Vec<String>, rev: u64, undone: bool },
}

/// Флаг «Стоп» текущего хода агента.
pub static AI_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
    /// Открыта страница «Настройки» (вместо монтажа).
    pub page: RwSignal<bool>,
    /// Номер правки настроек — страница и шапка ИИ перечитывают по нему.
    pub set_rev: RwSignal<u64>,
    /// Чаты агента (список), текущий чат, контекст (занято, окно).
    pub ai_chats: RwSignal<Vec<syn_assist::chat::ChatSummary>>,
    pub ai_chat: RwSignal<String>,
    pub ai_ctx: RwSignal<(usize, usize)>,
    /// Сведения о модели агента и список LLM службы.
    pub ai_info: RwSignal<Option<synaptixd::LlmInfo>>,
    pub ai_models: RwSignal<Vec<(String, String)>>,
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
    /// Позиция воспроизведения (зеркало `Editor::playhead`): курсор, просмотр,
    /// время — по нему, без перестройки таймлайна (протяжка, воспроизведение).
    pub ph: RwSignal<f64>,
    /// Ключ состава инспектора (см. `inspector::key`).
    pub insp_key: RwSignal<String>,
    /// Тянут край плашки перехода: клип, сдвиг мыши (пикс., + — шире).
    pub tr_edit: RwSignal<Option<(u64, f32)>>,
    /// Рамка выделения на таймлайне (x0, y0, x1, y1 — пикс. поля).
    pub band: RwSignal<Option<[f32; 4]>>,
    /// Привязка к краям клипов, позиции и началу (N).
    pub snap: RwSignal<bool>,
    /// Контекстное меню таймлайна: открыто, где (окно), по чему: клип (0 — пусто) и время.
    pub menu_open: RwSignal<bool>,
    pub menu_pos: RwSignal<Point>,
    pub menu_at: RwSignal<timeline::MenuAt>,
    /// Дорожка, которую переименовывают (поле в заголовке).
    pub rename_track: RwSignal<Option<u64>>,
    /// Окно выбора шрифта: `(clip_id, опц. ключ параметра шаблона)`, строка поиска.
    pub font_pick: RwSignal<Option<(u64, Option<String>)>>,
    pub font_query: RwSignal<String>,
    /// Панель «Кривые»: выбранный параметр и перетаскивание ключа или ручки.
    pub curve_param: RwSignal<Option<String>>,
    pub curve_drag: RwSignal<Option<curves::Drag>>,
    /// Панель «Цвет»: канал кривой (0 — яркость, 1–3 — R/G/B), сравнение «до/после», размер холста кривой.
    pub color_channel: RwSignal<usize>,
    pub color_compare: RwSignal<bool>,
    pub color_curve_size: RwSignal<syngui::prelude::Size>,
    /// Рамка трансформации на просмотре: перетаскивание.
    pub xf_drag: RwSignal<Option<xform::Drag>>,
    /// Поколение списка пресетов видов (перестраивает переключатель и окно управления).
    pub layout_rev: RwSignal<u64>,
    /// Модальное окно сохранения и управления пресетами видов.
    pub layout_modal: RwSignal<Option<layouts_ui::Mode>>,
    pub layout_name: RwSignal<String>,
    pub layout_desc: RwSignal<String>,
    /// Окно отчёта покрытия шаблонов Fusion (C2).
    pub coverage_report: RwSignal<Option<crate::fusion::report::CoverageReport>>,
    /// Окно «Ролик по шаблону» (G3).
    pub proj_draft: RwSignal<Option<project_templates::Draft>>,
    /// Готовые превью шаблонов проектов (счётчик для перерисовки).
    pub tpl_thumbs: RwSignal<u64>,
    /// Окно аргументов команды (ручной запуск пункта меню с аргументами).
    pub args_draft: RwSignal<Option<args_dialog::Draft>>,
    /// Выделенные файлы медиатеки (по порядку щелчков).
    pub media_sel: RwSignal<Vec<u64>>,
    /// Открыто окно «Свойства» файла медиатеки.
    pub media_props: RwSignal<Option<u64>>,
    /// Масштаб окна «Просмотр»: `None` — вписать, `Some(k)` — доля от 100% (0.1–8.0).
    pub viewer_zoom: RwSignal<Option<f32>>,
    /// Сдвиг (панорамирование) кадра в «Просмотре», пикс. панели от центра.
    pub viewer_pan: RwSignal<(f32, f32)>,
    /// Поле за кадром в «Просмотре»: 0 — тёмное (#1c1e26), 1 — серое, 2 — светлое, 3 — шахматка прозрачности.
    pub viewer_bg: RwSignal<u8>,
    /// Выбранный верхний раздел в панели «Шаблоны» (пусто — все, `"Переходы"` — переходы Fusion).
    pub tpl_cat: RwSignal<String>,
    /// Что сейчас генерирует агент (плитки в медиатеке до готового файла).
    pub gen_tiles: RwSignal<Vec<GenTile>>,
}

impl St {
    pub fn new() -> Self {
        let (view, spec) = crate::layout::load_current();
        let st = Self::make(view, spec);
        thumbs::bind(st.thumbs);
        install_ui_bridge(st);
        ai_chat::init(st);
        st
    }

    pub fn new_ephemeral(view: &str) -> Self {
        let st = Self::make(view.to_string(), crate::layout::builtin_spec(view));
        thumbs::bind(st.thumbs);
        st
    }

    fn make(view: String, spec: DockSpec) -> Self {
        Self {
            rev: use_signal(0),
            window: use_signal(syngui::window::WindowState::default()),
            theme: use_signal(String::new()),
            palette_open: use_signal(false),
            dock: DockState::new(spec),
            zoom: use_signal(60.0),
            toast: use_signal(None),
            ai_log: use_signal(Vec::new()),
            ai_busy: use_signal(false),
            page: use_signal(false),
            set_rev: use_signal(0),
            ai_chats: use_signal(Vec::new()),
            ai_chat: use_signal(String::new()),
            ai_ctx: use_signal((0, 0)),
            ai_info: use_signal(None),
            ai_models: use_signal(Vec::new()),
            view: use_signal(view),
            playing: use_signal(false),
            thumbs: use_signal(0),
            exporting: use_signal(None),
            tile_size: use_signal(110.0),
            drop_hint: use_signal(None),
            trim: use_signal(None),
            ph: use_signal(0.0),
            insp_key: use_signal(String::new()),
            tr_edit: use_signal(None),
            band: use_signal(None),
            snap: use_signal(true),
            menu_open: use_signal(false),
            menu_pos: use_signal(Point::zero()),
            menu_at: use_signal(timeline::MenuAt::Field(0, 0.0)),
            rename_track: use_signal(None),
            font_pick: use_signal(None),
            font_query: use_signal(String::new()),
            curve_param: use_signal(None),
            curve_drag: use_signal(None),
            color_channel: use_signal(0),
            color_compare: use_signal(false),
            color_curve_size: use_signal(syngui::prelude::Size::new(480.0, 220.0)),
            xf_drag: use_signal(None),
            layout_rev: use_signal(0),
            layout_modal: use_signal(None),
            layout_name: use_signal(String::new()),
            layout_desc: use_signal(String::new()),
            coverage_report: use_signal(None),
            proj_draft: use_signal(None),
            tpl_thumbs: use_signal(0),
            args_draft: use_signal(None),
            media_sel: use_signal(Vec::new()),
            media_props: use_signal(None),
            viewer_zoom: use_signal(None),
            viewer_pan: use_signal((0.0, 0.0)),
            viewer_bg: use_signal(0),
            tpl_cat: use_signal(String::new()),
            gen_tiles: use_signal(Vec::new()),
        }
    }

    /// Выполнить команду; ошибка — во всплывающее сообщение.
    pub fn exec(self, id: &str, args: Value) -> Option<cmd::Out> {
        let r = {
            let mut e = ed();
            let r = cmd::run(&mut e, id, &args);
            self.rev.set(e.revision);
            self.ph.set(e.playhead);
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
    /// обязательными аргументами — окно аргументов.
    pub fn run_menu(self, id: &str) {
        match id {
            "view.palette" => return self.palette_open.set(true),
            "view.fullscreen" => return self.set_fullscreen(!self.window.get_untracked().fullscreen),
            "view.ai" => return self.open_panel("ai"),
            "view.layout.save" => return layouts_ui::open_save(self),
            "view.layout.manage" => return layouts_ui::open_manage(self),
            "view.layout.reset" => return self.reset_layout(),
            "view.zoom_fit" => return self.viewer_fit(),
            "view.zoom_100" => {
                self.viewer_zoom.set(Some(1.0));
                self.viewer_pan.set((0.0, 0.0));
                return;
            }
            "view.zoom_in" => return self.viewer_zoom_step(1.25),
            "view.zoom_out" => return self.viewer_zoom_step(1.0 / 1.25),
            "view.viewer_bg" => {
                self.viewer_bg.update(|b| *b = (*b + 1) % 4);
                return;
            }
            "template.coverage" => return coverage_ui::open(self),
            // шаблоны проектов (G): галерея «Проекты», сохранение текущего проекта шаблоном
            "template.project.use" | "template.project.list" => {
                self.tpl_cat.set("Проекты".into());
                self.open_panel("templates");
                return;
            }
            "template.project.save" => {
                let name = ed().project.name.clone();
                if let Some(o) = self.exec("template.project.save", json!({"name": name})) {
                    self.toast.set(Some(o.text));
                    self.tpl_thumbs.update(|n| *n += 1);
                }
                return;
            }
            v if v.starts_with("view.layout.") => return self.set_view(&v["view.layout.".len()..]),
            v if v.starts_with("view.panel.") => return self.open_panel(&v["view.panel.".len()..]),
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
        // руками: окно аргументов по схеме команды (то же умеет и агент); без
        // обязательных, но бессмысленные пустыми — тоже через окно
        const FORM: &[&str] = &["clip.set", "lut.apply", "slot.fill"];
        if c.args.iter().any(|a| a.required) || FORM.contains(&c.id) {
            return args_dialog::open(self, c.id);
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
        *PLAY_FROM.lock().unwrap() = ed().playhead;
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
                    let ph = e.playhead;
                    drop(e);
                    // только позиция: таймлайн не перестраивается на каждом кадре
                    self.ph.set(ph);
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

    /// Стоп: пауза и возврат туда, откуда пустили воспроизведение.
    pub fn stop(self) {
        let was = self.playing.get_untracked();
        self.playing.set(false);
        if was {
            let t = *PLAY_FROM.lock().unwrap();
            self.seek_live(t);
        }
    }

    /// Позиция без команды и без перестройки таймлайна (протяжка курсора).
    pub fn seek_live(self, t: f64) {
        let t = {
            let mut e = ed();
            let t = e.project.snap(t.clamp(0.0, e.project.duration().max(0.0)));
            e.playhead = t;
            t
        };
        self.ph.set(t);
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
        let (view_key, title, spec) = match crate::layout::find(key) {
            Some(p) => (p.key, p.name, p.spec),
            None => (key.to_string(), key.to_string(), crate::layout::builtin_spec(key)),
        };
        self.view.set(view_key);
        self.dock.set_spec(spec);
        self.toast.set(Some(format!("Вид: {title}")));
    }

    /// Во весь экран или в окно; выбор запоминается (на телефоне по умолчанию — во весь экран).
    pub fn set_fullscreen(self, on: bool) {
        // запоминает track_fullscreen — по фактическому состоянию окна
        syngui::signal::set_fullscreen(on);
    }

    pub fn reset_layout(self) {
        let cur = self.view.get_untracked();
        let key = if crate::layout::BUILTINS.iter().any(|b| b.0 == cur) { cur } else { crate::layout::default_view().to_string() };
        let spec = crate::layout::builtin_spec(&key);
        self.view.set(key.clone());
        self.dock.set_spec(spec.clone());
        crate::layout::save_current(&spec, &key);
        self.toast.set(Some("Раскладка панелей сброшена".into()));
    }

    pub fn viewer_fit(self) {
        self.viewer_zoom.set(None);
        self.viewer_pan.set((0.0, 0.0));
    }

    pub fn viewer_zoom_step(self, factor: f32) {
        let cur = self.viewer_zoom.get_untracked().unwrap_or_else(panels::viewer_fit_k);
        let next = (cur * factor).clamp(0.1, 8.0);
        self.viewer_zoom.set(Some(next));
    }

    pub fn open_panel(self, id: &str) {
        let near = match id {
            "media" | "templates" | "effects" | "luts" | "layers" => Some("media"),
            "viewer" | "graph" => Some("viewer"),
            "inspector" | "ai" => Some("inspector"),
            "timeline" | "curves" | "color" => Some("timeline"),
            _ => None,
        };
        self.dock.open(id, near);
    }

    /// Ход агента в фоне; события — в панель ИИ, правки — в окно.
    pub fn ask_ai(self, text: String) {
        if self.ai_busy.get_untracked() || text.trim().is_empty() {
            return;
        }
        self.ai_busy.set(true);
        AI_CANCEL.store(false, std::sync::atomic::Ordering::SeqCst);
        self.ai_log.update(|l| l.push(AiLine::User(text.clone())));
        let rev_before = ed().revision;
        std::thread::spawn(move || {
            use syn_assist::agent::AgentEvent;
            // агент заново на каждый ход — правки «Настроек» действуют сразу
            let set = crate::settings::load();
            let agent = crate::agent::agent_with(shared(), &set);
            let mut chat = ai_chat::take();
            crate::agent::prepare_chat(&mut chat, &set);
            if chat.title.trim().is_empty() {
                chat.title = syn_assist::chat::title_from(&text);
            }
            // «Стоп» из панели — флаг хода (агент проверяет его между шагами)
            let cancel = &AI_CANCEL;
            let push = move |line: AiLine| {
                syngui::async_runtime::run_on_main_thread(move || {
                    self.ai_log.update(|l| match (&line, l.last_mut()) {
                        (AiLine::Text(t), Some(AiLine::Text(prev))) => prev.push_str(t),
                        // ход того же узла («cover (image) · 11/40») — обновить строку, а не плодить
                        (AiLine::Result(t), Some(AiLine::Result(prev))) if t.starts_with("… ") && prev.starts_with("… ") && t.split(" · ").next() == prev.split(" · ").next() => *prev = t.clone(),
                        _ => l.push(line),
                    });
                    self.rev.set(ed().revision);
                });
            };
            let r = crate::agent::run_turn(&agent, shared(), &mut chat, &text, &crate::agent::AutoConfirm(false), cancel, &mut |ev| match ev {
                AgentEvent::Text(t) => push(AiLine::Text(t)),
                AgentEvent::ToolStart { summary, .. } => {
                    // рецепт агента — сразу вкладкой в «Графе»: видно, что и с какими параметрами идёт
                    if let Some(rest) = summary.strip_prefix("Прогнать рецепт в проект") {
                        let args: serde_json::Value = rest.find('{').and_then(|i| serde_json::from_str(&rest[i..]).ok()).unwrap_or_default();
                        if let Some(name) = args.get("name").and_then(|v| v.as_str()).map(str::to_string) {
                            let params = args.get("params").and_then(|v| v.as_object()).cloned().unwrap_or_default();
                            syngui::async_runtime::run_on_main_thread(move || templates::open_recipe_run(self, &name, &params));
                        }
                    }
                    push(AiLine::Tool(summary))
                }
                // ход долгого инструмента (шаги рецепта) — строками в чат, а не тишина
                AgentEvent::ToolProgress { label, .. } if !label.trim().is_empty() => {
                    // плитка генерируемого ассета в медиатеке: прошлые элементы того же
                    // узла и другие узлы уже готовы (прогон последовательный)
                    if let Some(t) = parse_gen(&label) {
                        syngui::async_runtime::run_on_main_thread(move || {
                            self.gen_tiles.update(|v| {
                                v.retain(|x| x.node == t.node && x.item == t.item);
                                match v.iter_mut().find(|x| x.node == t.node && x.item == t.item) {
                                    Some(x) => *x = t,
                                    None => v.push(t),
                                }
                            });
                        });
                    }
                    push(AiLine::Result(format!("… {label}")))
                }
                AgentEvent::Message(m) if m.role == syn_assist::chat::Role::Tool => {
                    // инструмент закончил — готовые файлы уже в медиатеке, плитки хода убрать
                    syngui::async_runtime::run_on_main_thread(move || self.gen_tiles.set(Vec::new()));
                    push(AiLine::Result(m.text.lines().next().unwrap_or_default().to_string()))
                }
                _ => {}
            });
            syngui::async_runtime::run_on_main_thread(move || self.gen_tiles.set(Vec::new()));
            let stopped = AI_CANCEL.load(std::sync::atomic::Ordering::SeqCst);
            match r {
                Err(e) => push(AiLine::Error(if stopped { "Остановлено".into() } else { e })),
                Ok(()) if stopped => push(AiLine::Error("Остановлено".into())),
                Ok(()) => {}
            }
            // до/после: ход агента — один шаг отмены «ИИ: …»
            let diff = {
                let e = ed();
                match e.last_step() {
                    Some((label, before)) if e.revision != rev_before && label.starts_with("ИИ:") => Some((crate::doc::diff_summary(before, &e.project), e.revision)),
                    _ => None,
                }
            };
            if let Some((lines, rev)) = diff.filter(|(l, _)| !l.is_empty()) {
                push(AiLine::Diff { lines, rev, undone: false });
            }
            chat.updated = syn_assist::chat::now();
            let shown = chat.clone();
            ai_chat::put(chat);
            syngui::async_runtime::run_on_main_thread(move || {
                ai_chat::after_turn(self, &shown);
                self.ai_busy.set(false);
                self.rev.set(ed().revision);
            });
        });
    }
}

static UI_ST: OnceLock<St> = OnceLock::new();

/// Состояние окна для крючков, вызываемых не из виджета (меню нод и т. п.).
pub(crate) fn ui_st() -> Option<St> {
    UI_ST.get().copied()
}

/// Мост действий окна для агента и сокета (`ui.list`, `ui.run`): всё из меню и
/// палитры, что не команда документа.
fn install_ui_bridge(st: St) {
    let _ = UI_ST.set(st);
    cmd::ui_bridge::install(cmd::ui_bridge::Bridge {
        list: || {
            palette()
                .into_iter()
                .filter(|c| cmd::find(&c.id).is_none())
                .map(|c| (c.id, c.category, c.title, c.shortcut))
                .collect()
        },
        run: |id| {
            let id = id.to_string();
            let Some(st) = UI_ST.get().copied() else { return Err("окна нет".into()) };
            let title = palette().into_iter().find(|c| c.id == id).map(|c| c.title).unwrap_or_else(|| id.clone());
            syngui::async_runtime::run_on_main_thread(move || st.run_menu(&id));
            Ok(format!("выполнено в окне: {title}"))
        },
        state: || {
            let Some(st) = UI_ST.get().copied() else { return "нет окна".into() };
            let w = st.window.get_untracked();
            let zoom = st.viewer_zoom.get_untracked().map(|z| format!("{:.0}%", z * 100.0)).unwrap_or_else(|| "вписать".into());
            format!(
                "вид «{}», просмотр {zoom}{}{}",
                st.view.get_untracked(),
                if w.fullscreen { ", во весь экран" } else { "" },
                if w.maximized { ", развёрнуто" } else { "" }
            )
        },
    });
}

fn default_path() -> String {
    let dir = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default().join("Videos");
    let name = ed().project.name.replace('/', "_");
    dir.join(format!("{name}.syn")).display().to_string()
}

pub use crate::layout::{builtin_spec as layout, panel_title};

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
            ("clip-shape", Rgba::rgb(0x14, 0xb8, 0xa6).mix(shell, 0.42).hex()),
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
                .child(DecoratedBox::new().class("grow").child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
                    // страница «Настройки» вместо монтажа (вкладки в заголовке)
                    if st.page.get() {
                        vec![Box::new(DecoratedBox::new().class("grow").class("dock-area-pad").child(settings_page::view(st)))]
                    } else {
                        vec![Box::new(DecoratedBox::new().class("grow").class("dock-area-pad").child(dock(st)))]
                    }
                })))
                .child(statusbar(st)),
        ),
    );
    let hotkeys = EventHook::new()
        .on_key_down(move |key, mods| {
            if key == Key::F11 {
                st.set_fullscreen(!st.window.get_untracked().fullscreen);
                return KeyReply::Handled;
            }
            if key == Key::Escape && st.window.get_untracked().fullscreen {
                st.set_fullscreen(false);
                return KeyReply::Handled;
            }
            // на странице «Настройки» клавиши монтажа (пробел, S, N, Delete…) не действуют
            if st.page.get_untracked() {
                return KeyReply::Ignore;
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
                        st.seek_live(0.0);
                        return KeyReply::Handled;
                    }
                    Key::End => {
                        let d = ed().project.duration();
                        st.seek_live(d);
                        return KeyReply::Handled;
                    }
                    // разрез в позиции: выделенные клипы под ней, иначе все
                    Key::S => {
                        st.exec("clip.split", json!({}));
                        return KeyReply::Handled;
                    }
                    Key::Delete | Key::Backspace => {
                        if !ed().selection.is_empty() {
                            st.exec("clip.remove", json!({}));
                        }
                        return KeyReply::Handled;
                    }
                    // к предыдущей / следующей склейке
                    Key::Up | Key::Down => {
                        let (ph, t) = {
                            let e = ed();
                            (e.playhead, timeline::edit_point(&e.project, e.playhead, key == Key::Down))
                        };
                        if let Some(t) = t.filter(|t| (t - ph).abs() > 1e-6) {
                            st.seek_live(t);
                        }
                        return KeyReply::Handled;
                    }
                    Key::N => {
                        let on = !st.snap.get_untracked();
                        st.snap.set(on);
                        st.toast.set(Some(if on { "Привязка включена (N)".into() } else { "Привязка выключена (N)".into() }));
                        return KeyReply::Handled;
                    }
                    Key::Escape => {
                        if !ed().selection.is_empty() {
                            st.exec("selection.set", json!({"ids": []}));
                            return KeyReply::Handled;
                        }
                    }
                    _ => {}
                }
            }
            if mods.shift && !mods.ctrl && !mods.alt {
                match key {
                    // удалить со сдвигом: дыра закрывается
                    Key::Delete | Key::Backspace => {
                        if !ed().selection.is_empty() {
                            st.exec("clip.remove", json!({"ripple": true}));
                        }
                        return KeyReply::Handled;
                    }
                    Key::Left | Key::Right => {
                        let ph = ed().playhead;
                        st.seek_live((ph + if key == Key::Left { -1.0 } else { 1.0 }).max(0.0));
                        return KeyReply::Handled;
                    }
                    _ => {}
                }
            }
            // Alt+стрелки — сдвиг выделенных клипов на кадр, с Shift — на секунду
            if mods.alt && !mods.ctrl && matches!(key, Key::Left | Key::Right) {
                let fps = ed().project.settings.fps.max(1.0);
                let step = if mods.shift { 1.0 } else { 1.0 / fps };
                if !ed().selection.is_empty() {
                    st.exec("clip.shift", json!({"by": if key == Key::Left { -step } else { step }}));
                }
                return KeyReply::Handled;
            }
            if mods.ctrl && !mods.alt && !mods.shift && key == Key::A {
                let ids: Vec<u64> = ed().project.tracks.iter().flat_map(|t| t.clips.iter().map(|c| c.id)).collect();
                st.exec("selection.set", json!({"ids": ids}));
                return KeyReply::Handled;
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
        .child(timeline::menu(st))
        .child(fontpick::view(st))
        .child(layouts_ui::view(st))
        .child(coverage_ui::view(st))
        .child(project_templates::dialog(st))
        .child(args_dialog::view(st))
        .child(panels::media_props_view(st))
}

/// Пункты меню вида и окна (не команды документа).
const VIEW_ITEMS: &[(&str, &str, &str)] = &[("view.palette", "Палитра команд", "Ctrl+Shift+P"), ("view.ai", "Панель ИИ", "Ctrl+Shift+A"), ("view.fullscreen", "Во весь экран", "F11")];

/// Действия окна без команды документа: (id, раздел меню, заголовок, клавиши).
const UI_ITEMS: &[(&str, &str, &str, &str)] = &[
    ("media.import_folder", "Медиа", "Импорт папки…", "Ctrl+Shift+I"),
    ("project.save_as", "Проект", "Сохранить как…", "Ctrl+Shift+S"),
    ("view.panel.templates", "Шаблоны", "Галерея шаблонов", ""),
];

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
    let mut v: Vec<PaletteCommand> = cmd::all().into_iter().filter(|c| !c.id.starts_with("ui.")).map(|c| PaletteCommand::new(c.id, c.title).category(c.category).shortcut(c.shortcut).keywords(c.keywords).description(c.help)).collect();
    for (id, t, k) in VIEW_ITEMS {
        v.push(PaletteCommand::new(*id, *t).category("Вид").shortcut(*k));
    }
    for (id, cat, t, k) in UI_ITEMS {
        v.push(PaletteCommand::new(*id, *t).category(*cat).shortcut(*k));
    }
    for p in crate::layout::list() {
        v.push(PaletteCommand::new(format!("view.layout.{}", p.key), format!("Вид: {}", p.name)).category("Вид").keywords("layout preset раскладка пресет вид"));
    }
    v.push(PaletteCommand::new("view.layout.save", "Сохранить вид шаблоном…").category("Вид").keywords("layout preset save сохранить раскладку пресет вид шаблон сценарий rhai"));
    v.push(PaletteCommand::new("view.layout.manage", "Управление пресетами видов…").category("Вид").keywords("layout preset manage управление пресеты раскладка"));
    v.push(PaletteCommand::new("view.layout.reset", "Сбросить раскладку панелей").category("Вид").keywords("layout reset сбросить раскладку панели"));
    v.push(PaletteCommand::new("view.zoom_fit", "Просмотр: Вписать кадр").category("Вид").keywords("zoom fit вписать просмотр масштаб"));
    v.push(PaletteCommand::new("view.zoom_100", "Просмотр: Масштаб 100%").category("Вид").keywords("zoom 100 просмотр масштаб"));
    v.push(PaletteCommand::new("view.zoom_in", "Просмотр: Увеличить").category("Вид").keywords("zoom in увеличить приблизить просмотр"));
    v.push(PaletteCommand::new("view.zoom_out", "Просмотр: Уменьшить").category("Вид").keywords("zoom out уменьшить отдалить просмотр"));
    v.push(PaletteCommand::new("view.viewer_bg", "Просмотр: Переключить фон поля / шахматку").category("Вид").keywords("background checkerboard фон поле шахматка границы просмотр"));
    for (id, title) in crate::layout::PANELS {
        v.push(PaletteCommand::new(format!("view.panel.{id}"), format!("Панель: {title}")).category("Вид").keywords("panel окно панель открыть"));
    }
    v
}

fn view_menu() -> Vec<MenuItem> {
    let mut view: Vec<MenuItem> = VIEW_ITEMS.iter().map(|(id, t, k)| MenuItem::new(*id, *t).shortcut(*k)).collect();
    view.push(MenuItem::separator());
    view.extend(crate::layout::list().into_iter().map(|p| MenuItem::new(format!("view.layout.{}", p.key), format!("Вид: {}", p.name))));
    view.push(MenuItem::separator());
    view.push(MenuItem::new("view.layout.save", "Сохранить вид шаблоном…"));
    view.push(MenuItem::new("view.layout.manage", "Управление пресетами видов…"));
    view.push(MenuItem::new("view.layout.reset", "Сбросить раскладку панелей"));
    view.push(MenuItem::separator());
    view.extend(crate::layout::PANELS.iter().map(|(id, t)| MenuItem::new(format!("view.panel.{id}"), format!("Панель: {t}"))));
    view
}

fn menus() -> Vec<(&'static str, Vec<MenuItem>)> {
    let mut out: Vec<(&'static str, Vec<MenuItem>)> = Vec::new();
    // ui.* — мост действий окна для агента; в меню они уже есть сами по себе
    for c in cmd::all().into_iter().filter(|c| !c.id.starts_with("ui.")) {
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
    out
}

fn titlebar(st: St) -> Box<dyn Widget> {
    let mut menu = MenuBar::new().on_select(move |id| st.run_menu(id));
    for (title, items) in menus() {
        menu = menu.menu(title, move || items.clone());
    }
    menu = menu.menu("Вид", view_menu);
    let controls = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let w = st.window.get();
        vec![Box::new(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    if crate::layout::is_phone() {
        // телефон: значок, меню одной кнопкой (палитра команд — всё, что в меню), кнопки окна
        return Box::new(WindowDragRegion::new().child(
            DecoratedBox::new().class("titlebar").child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class("logo-mark").child(Icon::new(icons::MOVIE).class("logo-icon")))
                    .child(Text::new("Видеоредактор").max_lines(1).class("logo-text"))
                    .child(DecoratedBox::new().class("grow"))
                    .child(ToolButton::new(icons::SEARCH).tooltip("Команды, медиа, эффекты").on_click(move || st.palette_open.set(true)))
                    .child(DecoratedBox::new().class("window-controls").child(controls)),
            ),
        ));
    }
    Box::new(WindowDragRegion::new().child(
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
    ))
}

/// Вкладки страниц окна (как в Hexspire): монтаж и настройки.
fn page_tabs(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let settings = st.page.get();
        let tab = move |on: bool, page: bool, icon: &'static str, title: String| {
            GestureDetector::new().on_click(move || {
                st.page.set(page);
                if page {
                    settings_page::load_models(st);
                }
            }).child(
                DecoratedBox::new().class(if on { "page-tab on" } else { "page-tab" }).child(
                    Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icon).class("page-tab-icon")).child(Text::new(title).max_lines(1).class("page-tab-text")),
                ),
            )
        };
        vec![Box::new(
            Row::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(tab(!settings, false, icons::MOVIE, tr!("page.edit")))
                .child(tab(settings, true, icons::SETTINGS, tr!("page.settings"))),
        )]
    })
}

fn sep() -> impl Widget {
    DecoratedBox::new().class("toolbar-sep")
}

fn toolbar(st: St) -> impl Widget {
    let phone = crate::layout::is_phone();
    let cmd_btn = move |icon: &'static str, tip: &'static str, id: &'static str| ToolButton::new(icon).tooltip(tip).on_click(move || st.run_menu(id));
    let row = Row::new()
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
            .child(ToolButton::new(icons::SHAPE).tooltip("Фигура в позиции").on_click(move || {
                st.exec("shape.add", json!({"kind": "rect"}));
            }))
            .child(sep())
            .child(ToolButton::new(icons::ZOOM_OUT).tooltip("Мельче").on_click(move || st.zoom.update(|z| *z = (*z / 1.5).max(4.0))))
            .child(ToolButton::new(icons::ZOOM_IN).tooltip("Крупнее").on_click(move || st.zoom.update(|z| *z = (*z * 1.5).min(800.0))))
            .child(sep())
            // страницы окна, как в Hexspire: монтаж / настройки
            .child(page_tabs(st))
            .child(DecoratedBox::new().class("grow"))
            .child(
                GestureDetector::new().on_click(move || st.open_panel("ai")).child({
                    let mut pill = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icons::AI).class("tool-pill-icon"));
                    if !phone {
                        pill = pill.child(Text::new("ИИ-монтаж").class("tool-pill-text"));
                    }
                    DecoratedBox::new().class("tool-pill").child(pill)
                }),
            )
            .child(move || {
                let _ = st.layout_rev.get();
                let cur = st.view.get();
                let presets = crate::layout::list();
                let known = presets.iter().any(|p| p.key == cur);
                let mut items: Vec<DropdownItem> = presets.into_iter().map(|p| DropdownItem::new(p.key, p.name)).collect();
                items.push(DropdownItem::new("__save", "Сохранить вид…"));
                items.push(DropdownItem::new("__manage", "Пресеты видов…"));
                items.push(DropdownItem::new("__reset", "Сбросить раскладку"));
                Dropdown::new()
                    .items(items)
                    .selected(if known { cur } else { String::new() })
                    .placeholder("Вид")
                    .on_change(move |k: &str| match k {
                        "__save" => {
                            st.layout_rev.update(|v| *v = v.wrapping_add(1));
                            layouts_ui::open_save(st);
                        }
                        "__manage" => {
                            st.layout_rev.update(|v| *v = v.wrapping_add(1));
                            layouts_ui::open_manage(st);
                        }
                        "__reset" => {
                            st.layout_rev.update(|v| *v = v.wrapping_add(1));
                            st.reset_layout();
                        }
                        k => st.set_view(k),
                    })
                    .class("view-select")
            })
            .child(move || {
                let fs = st.window.get().fullscreen;
                ToolButton::new(if fs { icons::FULLSCREEN_EXIT } else { icons::FULLSCREEN }).tooltip(if fs { "Выйти из полноэкранного режима (F11)" } else { "Во весь экран (F11)" }).on_click(move || st.set_fullscreen(!st.window.get_untracked().fullscreen))
            });
    // телефон: кнопки не помещаются — лента прокручивается пальцем
    let body: Box<dyn Widget> = if phone { Box::new(ScrollView::new().horizontal().child(row)) } else { Box::new(row) };
    DecoratedBox::new().class("toolbar").child(body)
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
        let _ = st.ph.get();
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
/// Запоминать полноэкранный режим, как бы его ни переключили (кнопка, F11,
/// композитор): после того как окно приняло режим запуска — до этого
/// исходное «не во весь экран» затёрло бы выбор.
pub fn track_fullscreen(st: St) {
    let want = crate::layout::start_fullscreen();
    let settled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(!want));
    let last = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(want));
    syngui::signal::create_detached_effect(move || {
        let fs = st.window.get().fullscreen;
        if !settled.load(std::sync::atomic::Ordering::Relaxed) {
            if fs == want {
                settled.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            return;
        }
        if last.swap(fs, std::sync::atomic::Ordering::Relaxed) != fs {
            crate::layout::save_fullscreen(fs);
        }
    });
}

/// Состояние окна у каждого проекта своё (`projstate`): при запуске —
/// состояние текущего проекта, правки окна сохраняются под его ключом, при
/// смене проекта графы и раскладка переключаются; новый проект — чистый.
pub fn project_state(st: St) {
    use crate::projstate as ps;
    fn apply(st: St, ui: &ps::Ui) {
        if let Some((view, spec)) = crate::layout::decode_current(&ui.layout) {
            st.view.set(view);
            st.dock.set_spec(spec);
        }
        if let Some(z) = ui.zoom {
            st.zoom.set(z as f32);
        }
        st.viewer_zoom.set(ui.viewer_zoom.map(|z| z as f32));
        if let Some((x, y)) = ui.viewer_pan {
            st.viewer_pan.set((x as f32, y as f32));
        }
        if let Some(t) = ui.tile_size {
            st.tile_size.set(t as f32);
        }
    }
    if let Some(ui) = ps::load_ui(&ps::current()) {
        apply(st, &ui);
    }
    use_effect(move || {
        let view = st.view.get();
        let Some(spec) = st.dock.spec_tracked() else { return };
        let (x, y) = st.viewer_pan.get();
        let ui = ps::Ui {
            layout: crate::layout::encode_current(&spec, &view),
            zoom: Some(st.zoom.get() as f64),
            viewer_zoom: st.viewer_zoom.get().map(|z| z as f64),
            viewer_pan: Some((x as f64, y as f64)),
            tile_size: Some(st.tile_size.get() as f64),
        };
        ps::save_ui(&ps::current(), &ui);
    });
    let last = use_signal(ps::current());
    use_effect(move || {
        let _ = st.rev.get();
        let k = ps::key(ed().path.as_deref());
        let old = last.get_untracked();
        if k == old {
            return;
        }
        last.set(k.clone());
        // первое «Сохранить как» несохранённого проекта — то же окно под новым ключом
        if old == "untitled" && ps::adopt(&old, &k) {
            ps::set_current(&k);
            return;
        }
        ps::set_current(&k);
        if k == "untitled" {
            // новый проект — чистое рабочее место
            let _ = std::fs::remove_file(ps::graphs_file(&k));
            let _ = std::fs::remove_file(ps::ui_file(&k));
        }
        if let Some(ws) = graph::workspace() {
            ws.replace_state(syn_node_editor::persist::load_from(&ps::graphs_file(&k)));
        }
        if let Some(ui) = ps::load_ui(&k) {
            apply(st, &ui);
        }
    });
}

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
