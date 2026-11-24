//! Связь редактора с программой, в которую он встроен: уведомления, имя
//! программы (каталоги настроек и кэша, имена файлов по умолчанию), открытие
//! файлов, откладывание битого файла, подпись шаблона.
//!
//! Программа реализует [`EditorHost`] (все методы с умолчаниями) и ставит
//! его один раз при запуске: `syn_node_editor::host::set_host(MyHost)`.
//! Без этого действуют умолчания — для проверок и примеров.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::model::Template;

/// Вид уведомления.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    Info,
    Error,
}

/// Хост-колбэки программы. Методы зовутся на главном потоке интерфейса
/// (кроме [`quarantine`](Self::quarantine) и путей — те и из воркеров).
pub trait EditorHost: Send + Sync + 'static {
    /// Имя программы: каталоги `~/.config/<имя>`, `~/.cache/<имя>`, имена
    /// файлов по умолчанию (`<имя>-<время>.wav`).
    fn app_name(&self) -> &'static str {
        "syn-node-editor"
    }

    /// Уведомление пользователю (кнопки прогона: «запущено», «цикл в графе»…).
    fn notify(&self, kind: Notice, text: String) {
        match kind {
            Notice::Info => tracing::info!("{text}"),
            Notice::Error => tracing::warn!("{text}"),
        }
    }

    /// Файл открытых графов (`EditorWorkspace`).
    fn workspace_path(&self) -> PathBuf {
        home().join(".config").join(self.app_name()).join("workspace.json")
    }

    /// Открыть файл или папку системной программой.
    fn open_path(&self, path: &Path) -> std::io::Result<()> {
        open_with_system_default(path)
    }

    /// Отложить нечитаемый файл (`x.json` → `x.json.bad-<время>`), чтобы
    /// автосохранение не затёрло данные пользователя. Возвращает путь копии.
    fn quarantine(&self, path: &Path) -> std::io::Result<PathBuf> {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".bad-{secs}"));
        let backup = path.with_file_name(name);
        std::fs::rename(path, &backup)?;
        Ok(backup)
    }

    /// Подпись шаблона для заголовка открытого графа (перевод имени
    /// встроенного шаблона и т. п.).
    fn template_title(&self, t: &Template) -> String {
        t.name.clone()
    }
}

struct DefaultHost;
impl EditorHost for DefaultHost {}

static HOST: OnceLock<Box<dyn EditorHost>> = OnceLock::new();

/// Поставить хост программы. Повторный вызов игнорируется (хост один на
/// процесс); вызывать до первого обращения к редактору.
pub fn set_host(host: impl EditorHost) {
    if HOST.set(Box::new(host)).is_err() {
        tracing::warn!("syn-node-editor: хост уже задан, повторный set_host пропущен");
    }
}

/// Текущий хост (до `set_host` — умолчания).
pub fn host() -> &'static dyn EditorHost {
    HOST.get_or_init(|| Box::new(DefaultHost)).as_ref()
}

pub fn app_name() -> &'static str {
    host().app_name()
}

pub fn notify(kind: Notice, text: String) {
    host().notify(kind, text)
}

pub fn open_with_system(path: &Path) -> std::io::Result<()> {
    host().open_path(path)
}

pub fn quarantine(path: &Path) -> std::io::Result<PathBuf> {
    host().quarantine(path)
}

/// Unix-миллисекунды.
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn home() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// Путь, введённый руками: `~` и `~/…` раскрываются в домашний каталог.
/// Пробелы по краям срезаются.
pub fn expand_home(raw: &str) -> PathBuf {
    let raw = raw.trim();
    let h = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok().filter(|h| !h.is_empty());
    match (h, raw) {
        (Some(h), "~") => PathBuf::from(h),
        (Some(h), r) if r.starts_with("~/") => PathBuf::from(h).join(&r[2..]),
        _ => PathBuf::from(raw),
    }
}

/// `xdg-open` (macOS — `open`, Windows — `explorer`) без ожидания.
pub fn open_with_system_default(target: &Path) -> std::io::Result<()> {
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let mut child = std::process::Command::new(cmd).arg(target).spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
