//! Протокол демона: unix-сокет [`crate::paths::SOCKET`], по одному JSON-запросу в строке, ответ — одна
//! строка JSON. Длинные операции (загрузка образов) идут фоновым заданием: ответ сразу, ход — в `Status`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::images::ImageSet;
use crate::paths;

/// Графический сеанс пользователя, для которого запускается Android.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Session {
    pub uid: u32,
    pub gid: u32,
    pub user: String,
    pub xdg_runtime_dir: String,
    pub wayland_display: String,
    /// Каталог с сокетом `native` PulseAudio (PipeWire-pulse); пусто — `$XDG_RUNTIME_DIR/pulse`.
    pub pulse_runtime_path: String,
}

impl Session {
    /// Сеанс из окружения текущего процесса.
    pub fn from_env() -> Result<Self> {
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };
        let user = std::env::var("USER").unwrap_or_else(|_| uid.to_string());
        let xdg = std::env::var("XDG_RUNTIME_DIR").context("нет XDG_RUNTIME_DIR — запускать из сеанса")?;
        let wl = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
        let pulse = std::env::var("PULSE_RUNTIME_PATH").unwrap_or_else(|_| format!("{xdg}/pulse"));
        Ok(Self { uid, gid, user, xdg_runtime_dir: xdg, wayland_display: wl, pulse_runtime_path: pulse })
    }

    pub fn wayland_socket(&self) -> std::path::PathBuf {
        if self.wayland_display.starts_with('/') {
            self.wayland_display.clone().into()
        } else {
            std::path::Path::new(&self.xdg_runtime_dir).join(&self.wayland_display)
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Stopped,
    /// Контейнер запущен, Android загружается.
    Starting,
    /// `sys.boot_completed=1`.
    Running,
    Frozen,
    Stopping,
}

/// Фоновое задание (загрузка/импорт образов).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Job {
    pub id: u64,
    pub title: String,
    /// Текущий шаг.
    pub step: String,
    pub done: u64,
    pub total: u64,
    pub finished: bool,
    pub error: Option<String>,
    /// Задание можно отменить (`CancelJob`).
    #[serde(default)]
    pub cancellable: bool,
    /// Отменено пользователем (завершено без ошибки и без результата).
    #[serde(default)]
    pub cancelled: bool,
    /// Номер попытки (загрузка с автоповтором), с 1.
    #[serde(default)]
    pub attempt: u32,
    /// Ждёт повтора: unix-время следующей попытки (`RetryJobNow` — сразу).
    #[serde(default)]
    pub retry_at: Option<i64>,
    /// Ошибка последней попытки (при ожидании повтора).
    #[serde(default)]
    pub last_error: Option<String>,
    /// Итог успешного задания (имя скачанного набора образов).
    #[serde(default)]
    pub result: Option<String>,
    /// Что качается — для уведомлений («LineageOS + GApps»).
    #[serde(default)]
    pub subject: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Status {
    pub state: State,
    pub image: Option<String>,
    /// Экземпляр Android активного набора и его название.
    pub instance: Option<String>,
    pub instance_title: Option<String>,
    /// PID init Android (в пространстве имён хоста).
    pub init_pid: Option<i32>,
    pub session: Option<Session>,
    /// Секунды с запуска контейнера.
    pub uptime: Option<u64>,
    pub android_version: Option<String>,
    pub jobs: Vec<Job>,
    /// Память контейнера (cgroup `memory.current`), байт.
    #[serde(default)]
    pub memory: Option<u64>,
    /// Последняя ошибка запуска.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Status,
    /// Запустить Android для сеанса (по умолчанию — сеанс вызывающего).
    Start { session: Session },
    Stop,
    Restart,
    Freeze,
    Unfreeze,
    Images,
    /// Свежие сборки OTA-каналов (без загрузки).
    CheckUpdates,
    /// Скачать последний набор из OTA (фоновое задание); `system_type` — VANILLA или GAPPS (по умолчанию —
    /// из настроек).
    FetchImages {
        #[serde(default)]
        system_type: Option<String>,
        /// Сеанс, которому показывать уведомления о ходе загрузки (по умолчанию — сеанс запущенного Android).
        #[serde(default)]
        session: Option<Session>,
    },
    /// Отменить фоновое задание (загрузку образов).
    CancelJob { id: u64 },
    /// Задание ждёт повтора — повторить сейчас.
    RetryJobNow { id: u64 },
    /// Экземпляры Android: наборы образов, размер данных.
    Instances,
    /// Удалить данные экземпляра (Android этого экземпляра должен быть остановлен).
    ResetData { instance: String },
    /// Удалить экземпляр целиком: все его наборы образов и данные.
    RemoveInstance { instance: String },
    /// Установить APK (путь на хосте; фоновое задание).
    InstallApk { path: String },
    UninstallApp { package: String },
    ClearAppData { package: String },
    /// Последние строки журнала Android.
    Logcat { lines: u32 },
    /// Набор из локальных файлов (zip из OTA или .img).
    ImportImages { system: String, vendor: String, name: Option<String> },
    UseImages { name: String },
    RemoveImages { name: String },
    GetConfig,
    SetConfig { config: Config },
    /// Весь Android одним окном.
    ShowFullUi,
    /// Запускаемые приложения.
    Apps,
    LaunchApp { package: String },
    StopApp { package: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Response {
    Ok,
    Error { message: String },
    Status(Status),
    Images { sets: Vec<ImageSet>, active: Option<String> },
    Updates { system: crate::images::OtaEntry, vendor: crate::images::OtaEntry, installed: bool },
    Job { id: u64 },
    Config { config: Config },
    Apps { apps: Vec<crate::android::App> },
    Instances { instances: Vec<Instance> },
    Log { text: String },
}

/// Экземпляр Android (линейка образов) — для окна управления.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Instance {
    pub id: String,
    pub title: String,
    /// Наборы образов, свежие первыми.
    pub sets: Vec<ImageSet>,
    /// Размер /data, байт.
    pub data_size: u64,
    /// Этот экземпляр выбран (его запускает `syndroid start`).
    pub active: bool,
}

/// Запрос к демону.
pub fn call(req: &Request) -> Result<Response> {
    let mut s = UnixStream::connect(paths::SOCKET)
        .with_context(|| format!("{} — демон syndroidd не запущен?", paths::SOCKET))?;
    let mut line = serde_json::to_string(req)?;
    line.push('\n');
    s.write_all(line.as_bytes())?;
    let mut r = BufReader::new(s);
    let mut out = String::new();
    r.read_line(&mut out)?;
    if out.is_empty() {
        bail!("демон закрыл соединение");
    }
    let resp: Response = serde_json::from_str(&out)?;
    if let Response::Error { message } = resp {
        bail!("{message}");
    }
    Ok(resp)
}
