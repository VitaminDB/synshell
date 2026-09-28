//! Просмотрщик картинок: `syndesktop-files --viewer ФАЙЛ`.
//!
//! Сцена перенесена из просмотрщика вложений synthos (`image_stage`):
//! размытая копия картинки вместо пустых полей, масштаб к курсору,
//! перетаскивание и полосы прокрутки (`syngui::ImageViewport`), стрелки
//! листания, лента миниатюр и панель инструментов поверх. В отличие от
//! чата здесь — отдельное окно со своим заголовком, листание по всем
//! картинкам папки, поворот по EXIF, HEIC/AVIF/TIFF и прочее, чего не
//! декодирует syngui (через преобразование в PNG в кэше), удаление в
//! корзину, «Показать в папке» и «Открыть с помощью».
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────┐
//! │ ▣ photo.jpg   4032×3024 · 3,1 МБ · 3 из 12   ⓘ ⧉ 🗑 ─ □ ✕ │ ← заголовок
//! ├──────────────────────────────────────────────────────────┤
//! │ ░░░░░░░ та же картинка, размытая ░░░░░░░░░░░░░░ ┌──────┐ │
//! │  ‹  ┌──────────────────────────┐             › │ info │ │
//! │     │         картинка         │               └──────┘ │
//! │     └──────────────────────────┘                        │
//! │             ▢ ▣ ▢ ▢   миниатюры папки                   │
//! │      ( − 100% + │ ⛶ 1:1 ⤢ │ ⟲ ⟳ ⇋ ⇅ │ ⛶ )               │ ← панель
//! └──────────────────────────────────────────────────────────┘
//! ```

mod prepare;
mod ui;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use syndesktop_common::mime;
use syngui::prelude::*;
use syngui::widgets::{ImageViewCommand, ImageViewInfo};

/// Картинка в списке листания.
#[derive(Clone, PartialEq)]
pub struct Item {
    pub path: PathBuf,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub mtime: i64,
}

impl Item {
    fn from_path(path: &Path, mime: String) -> Option<Self> {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(path).ok()?;
        if !m.is_file() {
            return None;
        }
        Some(Self {
            path: path.to_path_buf(),
            name: path.file_name()?.to_string_lossy().to_string(),
            mime,
            size: m.len(),
            mtime: m.mtime(),
        })
    }
}

/// Поворот и отражение, заданные пользователем (поверх EXIF).
#[derive(Clone, Copy, PartialEq, Default)]
pub struct Orientation {
    /// По часовой стрелке, четверти оборота.
    pub turns: i32,
    pub flip_h: bool,
    pub flip_v: bool,
}

#[derive(Clone, Copy)]
pub struct Viewer {
    pub items: RwSignal<Arc<Vec<Item>>>,
    pub index: RwSignal<usize>,
    /// Команда области просмотра: номер растёт с каждым нажатием.
    pub cmd: RwSignal<(u64, ImageViewCommand)>,
    pub orient: RwSignal<HashMap<PathBuf, Orientation>>,
    pub info: RwSignal<ImageViewInfo>,
    /// Растёт, когда фон дописал преобразованную картинку, размытый фон
    /// или миниатюру ленты.
    pub rev: RwSignal<u64>,
    pub window: RwSignal<syngui::window::WindowState>,
    pub show_info: RwSignal<bool>,
    pub message: RwSignal<Option<String>>,
    pub menu: RwSignal<Vec<MenuItem>>,
    pub menu_open: RwSignal<bool>,
    pub menu_pos: RwSignal<Point>,
    pub theme: RwSignal<String>,
}

thread_local! {
    static VIEWER: RefCell<Option<Viewer>> = const { RefCell::new(None) };
}

pub fn v() -> Viewer {
    VIEWER.with(|c| c.borrow().expect("просмотрщик не создан"))
}

impl Viewer {
    pub fn current(&self) -> Option<Item> {
        let i = self.index.get();
        self.items.with(|v| v.get(i).cloned())
    }

    pub fn current_untracked(&self) -> Option<Item> {
        let i = self.index.get_untracked();
        self.items.with_untracked(|v| v.get(i).cloned())
    }

    pub fn send(&self, c: ImageViewCommand) {
        let seq = self.cmd.get_untracked().0 + 1;
        self.cmd.set((seq, c));
    }

    pub fn orientation(&self, p: &Path) -> Orientation {
        self.orient.with(|m| m.get(p).copied().unwrap_or_default())
    }

    pub fn orient_current(&self, f: impl FnOnce(&mut Orientation)) {
        let Some(it) = self.current_untracked() else { return };
        let mut m = self.orient.get_untracked();
        f(m.entry(it.path).or_default());
        self.orient.set(m);
    }

    pub fn step(&self, delta: isize) {
        let n = self.items.with_untracked(|v| v.len());
        if n < 2 {
            return;
        }
        let i = (self.index.get_untracked() as isize + delta).rem_euclid(n as isize) as usize;
        self.go_to(i);
    }

    pub fn go_to(&self, i: usize) {
        if i < self.items.with_untracked(|v| v.len()) && i != self.index.get_untracked() {
            self.index.set(i);
            // Новая картинка — вписанной.
            self.send(ImageViewCommand::Fit);
        }
    }

    /// Сообщение вверху сцены на 3 с.
    pub fn flash(&self, text: impl Into<String>) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::SeqCst) + 1;
        self.message.set(Some(text.into()));
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(3));
            // Не стирать сообщение, появившееся позже.
            if SEQ.load(Ordering::SeqCst) == seq {
                run_on_main_thread(|| v().message.set(None));
            }
        });
    }
}

/// Картинка ли это (для проводника: открывать ли просмотрщиком).
pub fn handles(mime_type: &str) -> bool {
    mime_type.starts_with("image/") && !matches!(mime_type, "image/x-xcf" | "image/vnd.adobe.photoshop" | "image/vnd.djvu" | "image/x-icns")
}

/// Картинки папки файла (по имени, естественный порядок); сам файл — даже
/// скрытый или неизвестного типа.
fn folder_items(file: &Path) -> (Vec<Item>, usize) {
    let own = Item::from_path(file, mime::detect_file(file));
    let mut v: Vec<Item> = Vec::new();
    if let Some(dir) = file.parent() {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p == file {
                    continue;
                }
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                let Some(m) = mime::guess_by_name(&name) else { continue };
                if handles(&m) {
                    if let Some(it) = Item::from_path(&p, m) {
                        v.push(it);
                    }
                }
            }
        }
    }
    if let Some(o) = own {
        v.push(o);
    }
    v.sort_by(|a, b| crate::model::natural_cmp(&a.name, &b.name));
    let i = v.iter().position(|it| it.path == file).unwrap_or(0);
    (v, i)
}

fn init(file: &Path, cfg: &syndesktop_common::Config) -> Viewer {
    let (items, index) = folder_items(file);
    let v = Viewer {
        items: use_signal(Arc::new(items)),
        index: use_signal(index),
        cmd: use_signal((0, ImageViewCommand::Fit)),
        orient: use_signal(HashMap::new()),
        info: use_signal(ImageViewInfo::default()),
        rev: use_signal(0),
        window: use_signal(syngui::window::WindowState::default()),
        show_info: use_signal(false),
        message: use_signal(None),
        menu: use_signal(Vec::new()),
        menu_open: use_signal(false),
        menu_pos: use_signal(Point::zero()),
        theme: use_signal(theme_mss(cfg)),
    };
    VIEWER.with(|c| *c.borrow_mut() = Some(v));
    connect_background();
    prepare::prune_cache();
    v
}

pub const STYLES: &str = include_str!("../../styles/viewer.mss");

pub fn theme_mss(cfg: &syndesktop_common::Config) -> String {
    let mut s = crate::state::theme_mss(cfg);
    s.push('\n');
    s.push_str(STYLES);
    s
}

/// Готовность фоновой работы (преобразование, размытие, миниатюры) —
/// одной перестройкой раз в 100 мс.
fn connect_background() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static PENDING: AtomicBool = AtomicBool::new(false);
    fn bump() {
        if PENDING.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            PENDING.store(false, Ordering::SeqCst);
            run_on_main_thread(|| v().rev.update(|r| *r += 1));
        });
    }
    crate::thumbs::set_notify(bump);
    prepare::set_notify(bump);
}

/// Снимок без окна (проверка): тот же корень, что и в окне.
pub fn screenshot(out: &str, size: (u32, u32), scale: f64, file: PathBuf, cfg: syndesktop_common::Config, script: &[String]) -> anyhow::Result<()> {
    crate::shot::screenshot_with(out, size, scale, move || init(&file, &cfg).theme, ui::root, prepare::busy, script)
}

pub fn run(file: PathBuf, cfg: syndesktop_common::Config) {
    let file = std::fs::canonicalize(&file).unwrap_or(file);
    if !file.is_file() {
        eprintln!("нет такого файла: {}", file.display());
        std::process::exit(1);
    }
    let v = init(&file, &cfg);
    let initial = v.theme.get_untracked();
    let title = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    App::new()
        .title(format!("{title} — Просмотр"))
        .app_id("syndesktop-viewer")
        .size(1280, 860)
        .min_size(480, 360)
        .frameless()
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&initial)
        .with_dynamic_theme(v.theme)
        .with_window_state(v.window)
        .run(move |_| ui::root());
}

/// Запустить отвязанно от окна.
pub fn spawn(program: &Path, args: &[&std::ffi::OsStr]) {
    use std::os::unix::process::CommandExt;
    let mut c = std::process::Command::new(program);
    c.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let _ = c.spawn();
}

/// Открыть картинки просмотрщиком (новым процессом): из проводника.
pub fn open(file: &Path) {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("syndesktop-files"));
    spawn(&exe, &["--viewer".as_ref(), file.as_os_str()]);
}
