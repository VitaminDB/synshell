//! IPC композитора: JSON-строки по Unix-сокету (`paths::socket_path()`).
//!
//! Клиент пишет одну строку-[`Request`], получает одну строку-[`Response`].
//! После `Request::EventStream` соединение переходит в режим событий:
//! сначала приходит полный снимок состояния (`Event::Snapshot`), затем
//! строки-[`Event`] по мере изменений. Так работают панель задач, пейджер
//! столов и индикатор раскладки в оболочке.

use crate::action::{Action, LayoutKind};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "request", rename_all = "kebab-case")]
pub enum Request {
    Version,
    Windows,
    Workspaces,
    Outputs,
    KeyboardLayouts,
    /// Выполнить действие (как по сочетанию клавиш).
    Action { action: Action },
    /// Действие над конкретным окном (панель задач).
    WindowAction { id: u64, op: WindowOp },
    /// Перейти в режим событий.
    EventStream,
    /// Сообщить геометрию значка окна на панели — сюда сворачивается окно.
    SetMinimizeRect { id: u64, output: String, rect: [i32; 4] },
}

/// Операции над окном из панели задач.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WindowOp {
    /// Показать и дать фокус (развернуть из свёрнутого, перейти на его стол).
    Activate,
    /// Как клик по кнопке на панели задач: активно — свернуть, иначе активировать.
    ToggleMinimize,
    Minimize,
    Close,
    Kill,
    ToggleMaximize,
    ToggleFullscreen,
    ToggleFloating,
    ToggleSticky,
    ToggleAlwaysOnTop,
    /// Начать перетаскивание окна мышью (кнопка уже нажата — например, на
    /// заголовке окна на панели); развёрнутое окно при этом восстанавливается.
    StartMove,
    /// Переместить окно на стол с индексом (с 0).
    MoveToWorkspace(u32),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "response", rename_all = "kebab-case")]
pub enum Response {
    Ok,
    Error { message: String },
    Version { version: String },
    Windows { windows: Vec<WindowInfo> },
    Workspaces { workspaces: Vec<WorkspaceInfo> },
    Outputs { outputs: Vec<OutputInfo> },
    KeyboardLayouts { layouts: KeyboardLayouts },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    /// Полное состояние — первым после подписки.
    Snapshot {
        windows: Vec<WindowInfo>,
        workspaces: Vec<WorkspaceInfo>,
        outputs: Vec<OutputInfo>,
        keyboard: KeyboardLayouts,
    },
    /// Окно появилось или изменилось.
    WindowChanged { window: WindowInfo },
    WindowClosed { id: u64 },
    /// Окно в фокусе (None — фокуса нет, например рабочий стол).
    WindowFocused { id: Option<u64> },
    WorkspacesChanged { workspaces: Vec<WorkspaceInfo> },
    OutputsChanged { outputs: Vec<OutputInfo> },
    KeyboardLayoutChanged { keyboard: KeyboardLayouts },
    /// Команда оболочке (`Action::Shell`).
    ShellCommand { command: String },
    /// Композитор перечитал конфиг.
    ConfigReloaded { error: Option<String> },
    /// Сеанс завершается — оболочке пора выйти.
    Exiting,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub app_id: String,
    pub pid: Option<i32>,
    /// Индекс стола (с 0).
    pub workspace: u32,
    pub output: Option<String>,
    pub focused: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub floating: bool,
    pub sticky: bool,
    pub always_on_top: bool,
    pub urgent: bool,
    pub skip_taskbar: bool,
    /// Геометрия в глобальных логических координатах [x, y, w, h].
    pub geometry: [i32; 4],
    /// Меню приложения (`com.canonical.dbusmenu`), о котором окно сообщило
    /// по `org_kde_kwin_appmenu`: служба D-Bus и путь объекта.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appmenu: Option<(String, String)>,
    /// Идентификатор окна X11 (XWayland) — по нему меню регистрируют через
    /// `com.canonical.AppMenu.Registrar`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x11_id: Option<u32>,
    /// Меню GTK-программы (`gtk_shell1.set_dbus_properties`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gtk_menu: Option<GtkMenu>,
}

/// Где на D-Bus меню GTK-программы: `org.gtk.Menus` по пути `menubar`,
/// действия `app.*` — `org.gtk.Actions` по `app_path`, `win.*` — по
/// `window_path`; всё у владельца `bus`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub struct GtkMenu {
    pub bus: String,
    pub menubar: String,
    pub app_path: String,
    pub window_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct WorkspaceInfo {
    /// Индекс (с 0).
    pub index: u32,
    pub name: String,
    pub active: bool,
    pub windows: u32,
    pub urgent: bool,
    pub layout: LayoutKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct OutputInfo {
    pub name: String,
    pub description: String,
    pub make: String,
    pub model: String,
    /// Логическая геометрия [x, y, w, h].
    pub geometry: [i32; 4],
    pub scale: f64,
    pub refresh_mhz: u32,
    pub physical_mm: [i32; 2],
    pub modes: Vec<ModeInfo>,
    pub current_mode: Option<usize>,
    pub transform: String,
    pub primary: bool,
    pub focused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ModeInfo {
    pub width: i32,
    pub height: i32,
    pub refresh_mhz: i32,
    pub preferred: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct KeyboardLayouts {
    /// Полные имена раскладок («English (US)», «Russian»).
    pub names: Vec<String>,
    /// Короткие коды из конфига («us», «ru»).
    pub short: Vec<String>,
    pub current: u32,
}

// ─── Клиент ─────────────────────────────────────────────────────────────────

/// Синхронный клиент IPC.
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    pub fn connect() -> std::io::Result<Self> {
        Self::connect_to(&crate::paths::socket_path())
    }

    pub fn connect_to(path: &Path) -> std::io::Result<Self> {
        let stream = UnixStream::connect(path)?;
        let writer = stream.try_clone()?;
        Ok(Self { reader: BufReader::new(stream), writer })
    }

    pub fn request(&mut self, req: &Request) -> std::io::Result<Response> {
        let mut line = serde_json::to_string(req).map_err(std::io::Error::other)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes())?;
        let mut resp = String::new();
        if self.reader.read_line(&mut resp)? == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "композитор закрыл соединение",
            ));
        }
        serde_json::from_str(&resp).map_err(std::io::Error::other)
    }

    /// Подписаться на события. Возвращает итератор; блокирует на чтении.
    pub fn event_stream(mut self) -> std::io::Result<EventStream> {
        match self.request(&Request::EventStream)? {
            Response::Ok => Ok(EventStream { reader: self.reader }),
            Response::Error { message } => Err(std::io::Error::other(message)),
            other => Err(std::io::Error::other(format!("неожиданный ответ: {other:?}"))),
        }
    }
}

pub struct EventStream {
    reader: BufReader<UnixStream>,
}

impl Iterator for EventStream {
    type Item = std::io::Result<Event>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut line = String::new();
        loop {
            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(0) => return None,
                Ok(_) if line.trim().is_empty() => continue,
                Ok(_) => {
                    return Some(serde_json::from_str(&line).map_err(std::io::Error::other));
                }
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

/// Отправить одно действие и вернуть ответ (для кнопок панели и CLI).
pub fn send_action(action: Action) -> std::io::Result<Response> {
    Client::connect()?.request(&Request::Action { action })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_shapes() {
        let r = Request::Action { action: "workspace 2".parse().unwrap() };
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(s, r#"{"request":"action","action":"workspace 2"}"#);
        let back: Request = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
        let w = Request::WindowAction { id: 3, op: WindowOp::MoveToWorkspace(1) };
        let s = serde_json::to_string(&w).unwrap();
        assert_eq!(serde_json::from_str::<Request>(&s).unwrap(), w);
        let e = Event::ShellCommand { command: "launcher".into() };
        let s = serde_json::to_string(&e).unwrap();
        assert_eq!(s, r#"{"event":"shell-command","command":"launcher"}"#);
    }
}
