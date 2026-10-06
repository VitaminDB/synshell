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
    /// Снять все выводы разом (без указателя) для программы снимков:
    /// кадры — сырой RGBA в `$XDG_RUNTIME_DIR`, ответ — [`Response::Capture`].
    Capture,
    /// Состояние режима окон телефона — [`Response::Mobile`].
    Mobile,
    /// Удалённый ввод (synlink): события проходят тот же путь, что и
    /// настоящие устройства (жесты, сочетания). Координаты — доли 0..1
    /// вывода `output` (по умолчанию первого).
    Input { output: Option<String>, events: Vec<InputEvent> },
    /// Перевести соединение в поток кадров вывода (synlink): ответ —
    /// [`Response::Frame`] с полным кадром, дальше по [`Request::FrameNext`]
    /// приходят только изменившиеся прямоугольники. Кадр лежит в файле
    /// `FrameInfo::path` (RGBA построчно), композитор пишет в него только
    /// между запросом и ответом — читать можно до следующего `FrameNext`.
    ///
    /// `video` — изменения большой площади кодировать аппаратным видеокодером
    /// ([`FrameVideo`]), если он есть у композитора (GPU + V4L2); иначе и
    /// при ошибке кодера — без потерь, как без `video`.
    FrameStream {
        output: Option<String>,
        cursor: bool,
        #[serde(default)]
        video: Option<VideoRequest>,
    },
    /// Следующий кадр потока: ответ приходит, когда на выводе что-то
    /// изменилось (или сразу, если изменения накопились). `key` — следующий
    /// видеокадр сделать ключевым (у зрителя новый декодер или ошибка).
    FrameNext {
        #[serde(default)]
        key: bool,
    },
    /// Дисплей X11 для программы с именами `keys` (имя exe, `steam:<AppId>`…):
    /// с её разрешением из `[[x11_app]]`, иначе общий. Отдельный Xwayland
    /// запускается при первом запросе — ответ [`Response::X11Display`].
    X11Display { keys: Vec<String> },
}

/// Видео в потоке кадров.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VideoRequest {
    /// `h264` или `hevc`.
    pub codec: String,
    /// Бит/с.
    pub bitrate: u32,
    /// Мелкие изменения (курсор, часы, набор текста) — без потерь, видео —
    /// только крупные; иначе видео для всех изменений.
    #[serde(default)]
    pub mixed: bool,
}

/// Видеопакет кадра: в файле потока с `offset`, `len` байт.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FrameVideo {
    pub codec: String,
    /// Ключевой кадр (с SPS/PPS).
    pub key: bool,
    pub offset: u64,
    pub len: u64,
    /// Где кадр изменился: зритель берёт из декодированного кадра только
    /// эти прямоугольники, остальное у него — без потерь. Пусто — пакет
    /// только для декодера (ключевой кадр при полном кадре без потерь).
    pub rects: Vec<[i32; 4]>,
}

/// Событие удалённого ввода. `x`, `y` — доли ширины и высоты вывода.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum InputEvent {
    /// Указатель в точку вывода.
    Motion { x: f64, y: f64 },
    /// Относительное движение, логические px.
    MotionRelative { dx: f64, dy: f64 },
    /// Кнопка мыши: код evdev (`BTN_LEFT` = 0x110).
    Button { button: u32, pressed: bool },
    /// Прокрутка, логические px; `discrete` — колесо (шаги по 15 px).
    Axis { dx: f64, dy: f64, #[serde(default)] discrete: bool },
    /// Клавиша: код evdev (`KEY_A` = 30).
    Key { code: u32, pressed: bool },
    /// Нажать сочетание, как `key Alt+Left` в действиях.
    Combo { combo: String },
    /// Набрать текст по текущей раскладке (символы, которых в ней нет,
    /// пропускаются).
    Text { text: String },
    TouchDown { id: u32, x: f64, y: f64 },
    TouchMotion { id: u32, x: f64, y: f64 },
    TouchUp { id: u32 },
    /// Конец пачки касаний (после down/motion/up).
    TouchFrame,
    TouchCancel,
}

/// Кадр потока вывода (synlink).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FrameInfo {
    pub output: String,
    /// Размер кадра в пикселях.
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    /// Номер кадра с начала потока (0 — первый, полный).
    pub seq: u64,
    /// Файл кадра: RGBA построчно (`width * 4` байт на строку).
    pub path: String,
    /// Изменившиеся прямоугольники [x, y, w, h] в пикселях кадра;
    /// у полного кадра — весь кадр.
    pub rects: Vec<[i32; 4]>,
    /// Указатель в пикселях кадра, если он на этом выводе.
    pub pointer: Option<[f64; 2]>,
    /// Видеопакет кадра (поток с `video`). Прямоугольники без потерь
    /// (`rects`) накладываются после него: так поверх видео досылаются
    /// чистые пиксели, когда экран замер.
    #[serde(default)]
    pub video: Option<FrameVideo>,
    /// Видео просили, но кодера нет или он сломался — поток без потерь.
    #[serde(default)]
    pub video_error: Option<String>,
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
    Capture { capture: CaptureInfo },
    Mobile { mobile: MobileInfo },
    Frame { frame: FrameInfo },
    /// `display` — значение DISPLAY (`:1`), `resolution` — разрешение этого Xwayland.
    X11Display { display: String, resolution: String },
}

/// Режим окон телефона для оболочки: какой режим, какая страница
/// приложений показана, порядок страниц, положение виртуального стола.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MobileInfo {
    pub mode: crate::action::MobileMode,
    /// Окно (корневое) показанной страницы; `None` — домашний экран.
    pub page: Option<u64>,
    /// Страницы по порядку — id корневых окон.
    pub pages: Vec<u64>,
    /// Сдвиг виртуального стола (режим `free`), логические px.
    pub camera: [i32; 2],
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
    /// Команда оболочке от свайпа от края экрана (`[gestures] edge_*`
    /// с действием `shell …`): `edge` — `top`, `bottom`, `left`, `right`.
    /// Оболочка тратит жест на показ спрятанной автоскрытием панели (дока)
    /// у этого края, если такая есть, иначе выполняет команду.
    EdgeGesture { edge: String, command: String },
    /// Композитор перечитал конфиг.
    ConfigReloaded { error: Option<String> },
    /// Сеанс завершается — оболочке пора выйти.
    Exiting,
    /// Режим окон телефона, страница или стол изменились.
    MobileChanged { mobile: MobileInfo },
    /// Экраны погашены (`on: false`) или включены: оболочка отпускает датчики,
    /// пока экран не горит.
    ScreenPower { on: bool },
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
    /// Касания окну идут как мышь (правило `touch_as_mouse`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub touch_as_mouse: bool,
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
    /// Высота экранной клавиатуры на выводе (логические пиксели), 0 — её нет.
    /// Док с автоскрытием прячется, пока она открыта: иначе композитор ставит
    /// его над клавиатурой, и окна его не перекрывают.
    #[serde(default)]
    pub keyboard: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ModeInfo {
    pub width: i32,
    pub height: i32,
    pub refresh_mhz: i32,
    pub preferred: bool,
}

/// Застывший экран для программы снимков: кадры всех выводов, окна и
/// указатель в один момент.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CaptureInfo {
    pub outputs: Vec<CapturedOutput>,
    /// Видимые окна сверху вниз по стопке.
    pub windows: Vec<CapturedWindow>,
    /// Указатель в глобальных логических координатах.
    pub pointer: [f64; 2],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CapturedOutput {
    pub name: String,
    /// Логическая геометрия [x, y, w, h].
    pub geometry: [i32; 4],
    pub scale: f64,
    /// Размер кадра в пикселях.
    pub width: u32,
    pub height: u32,
    /// Файл с кадром: RGBA построчно, без заголовка; читатель удаляет его.
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CapturedWindow {
    pub title: String,
    pub app_id: String,
    /// Окно вместе с заголовком рамки, глобально [x, y, w, h].
    pub rect: [i32; 4],
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
