//! synfsd — журнал изменений файлов: какая программа и когда создала,
//! изменила, удалила или переименовала файл. Демон (root) слушает fanotify
//! на всех дисковых файловых системах, склеивает события и пишет сводку в
//! SQLite (`/var/lib/synfsd/activity.db`). Программы спрашивают его через
//! сокет [`SOCKET`]: запрос и ответ — по строке JSON.
//!
//! Этот модуль — протокол и клиент (без зависимостей демона).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Сокет демона.
pub const SOCKET: &str = "/run/synfsd.sock";

/// Что случилось с файлом.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Created,
    Modified,
    Deleted,
    Renamed,
}

impl Kind {
    pub fn id(self) -> i64 {
        match self {
            Kind::Created => 0,
            Kind::Modified => 1,
            Kind::Deleted => 2,
            Kind::Renamed => 3,
        }
    }

    pub fn from_id(v: i64) -> Kind {
        match v {
            0 => Kind::Created,
            2 => Kind::Deleted,
            3 => Kind::Renamed,
            _ => Kind::Modified,
        }
    }
}

/// Запись журнала: одинаковые изменения одной программы за короткое время
/// склеены в одну (`count` — сколько раз, `first`..`last` — когда).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Record {
    pub path: String,
    /// Переименование: прежний путь.
    pub old_path: Option<String>,
    pub kind: Kind,
    pub is_dir: bool,
    /// Исполняемый файл программы (`/usr/bin/firefox`); пусто — процесс
    /// успел завершиться.
    pub exe: String,
    /// Имя процесса (`comm`).
    pub comm: String,
    pub pid: i32,
    /// Пользователь процесса.
    pub uid: u32,
    /// Миллисекунды Unix.
    pub first: i64,
    pub last: i64,
    pub count: u64,
}

impl Record {
    /// Имя программы для показа: файл исполняемого или `comm`.
    pub fn program(&self) -> String {
        if !self.exe.is_empty() {
            if let Some(n) = Path::new(&self.exe).file_name() {
                return n.to_string_lossy().into_owned();
            }
        }
        if !self.comm.is_empty() {
            return self.comm.clone();
        }
        format!("pid {}", self.pid)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Изменения файла (или всего внутри папки, если `children`), новые сверху.
    History { path: String, children: bool, limit: usize },
    Status,
    /// Исключения: программы, чьи изменения не записываются.
    Exclusions,
    /// Не записывать программу: имя процесса (`journalctl`), имя файла или
    /// полный путь исполняемого (`/usr/bin/journalctl`). Менять список
    /// могут root и группа `wheel`.
    ExcludeProgram { program: String },
    /// Снова записывать программу.
    IncludeProgram { program: String },
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Status {
    /// Работает с (мс Unix).
    pub since: i64,
    pub events: u64,
    /// Сколько раз очередь ядра переполнялась (события потеряны).
    pub overflows: u64,
    /// Наблюдаемые файловые системы (точки монтирования).
    pub filesystems: Vec<String>,
    pub records: u64,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Response {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub items: Vec<Record>,
    #[serde(default)]
    pub status: Option<Status>,
    /// Исключённые программы (ответ на запросы об исключениях).
    #[serde(default)]
    pub programs: Vec<String>,
    /// Может ли спросивший менять исключения.
    #[serde(default)]
    pub can_edit: bool,
}

/// Отправить запрос демону.
pub fn request(req: &Request) -> std::io::Result<Response> {
    let mut s = UnixStream::connect(SOCKET)?;
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    s.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut line = serde_json::to_string(req)?;
    line.push('\n');
    s.write_all(line.as_bytes())?;
    let mut out = String::new();
    BufReader::new(s).read_line(&mut out)?;
    let r: Response = serde_json::from_str(&out)?;
    if !r.ok {
        return Err(std::io::Error::other(r.error.unwrap_or_default()));
    }
    Ok(r)
}

/// Исключённые программы и может ли текущий пользователь их менять.
pub fn exclusions() -> std::io::Result<(Vec<String>, bool)> {
    request(&Request::Exclusions).map(|r| (r.programs, r.can_edit))
}

/// Добавить (`on`) или убрать исключение программы; ответ — новый список.
pub fn set_excluded(program: &str, on: bool) -> std::io::Result<Vec<String>> {
    let program = program.trim().to_string();
    let req = if on { Request::ExcludeProgram { program } } else { Request::IncludeProgram { program } };
    request(&req).map(|r| r.programs)
}

/// Подходит ли запись под правило исключения: совпадает имя процесса,
/// имя исполняемого файла или его полный путь.
pub fn program_matches(rule: &str, exe: &str, comm: &str) -> bool {
    let rule = rule.trim();
    if rule.is_empty() {
        return false;
    }
    if rule.contains('/') {
        return rule == exe;
    }
    // `comm` ядро обрезает до 15 байт.
    let name = Path::new(exe).file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    rule == name || rule == comm || (comm.len() == 15 && rule.starts_with(comm))
}

/// История изменений пути (с содержимым папки, если `children`).
pub fn history(path: &Path, children: bool, limit: usize) -> std::io::Result<Vec<Record>> {
    request(&Request::History { path: path.to_string_lossy().into_owned(), children, limit }).map(|r| r.items)
}
