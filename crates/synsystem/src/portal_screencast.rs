//! Бэкенды портала `org.freedesktop.impl.portal.ScreenCast` (демонстрация экрана: браузеры,
//! OBS, Telegram, gpu-screen-recorder) и `org.freedesktop.impl.portal.Screenshot` — на том же
//! имени шины, что и доступ ([`crate::portal_access`]) и выбор файлов.
//!
//! Экран спрашивается у пользователя диалогом оболочки ([`ScreenPicker`]): какая программа,
//! какой монитор. Сам захват — помощник `syn-screencast` (кадры synwm по wlr-screencopy прямо
//! в буферы PipeWire), по процессу на поток; закрыл сеанс — процессы завершаются.

use std::collections::HashMap;
use std::io::BufRead;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value as ZValue};

const RESPONSE_OK: u32 = 0;
const RESPONSE_CANCELLED: u32 = 1;
const RESPONSE_OTHER: u32 = 2;
/// Источники: только мониторы (окна — позже).
const SOURCE_MONITOR: u32 = 1;
/// Курсор: скрыт (1) или в кадре (2).
const CURSOR_HIDDEN: u32 = 1;
const CURSOR_EMBEDDED: u32 = 2;
const ASK_TIMEOUT: Duration = Duration::from_secs(120);

/// Монитор компьютера.
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    pub name: String,
    pub description: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: i32,
}

/// Что спрашивается.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenAsk {
    /// Показ экрана (ScreenCast).
    Cast,
    /// Снимок экрана (Screenshot, интерактивный).
    Shot,
}

/// Вопрос «какой экран показать». Ответ — [`ScreenRequest::respond`] с именами выводов
/// (пусто — отказ).
pub struct ScreenRequest {
    pub id: u64,
    pub app_id: String,
    pub kind: ScreenAsk,
    pub outputs: Vec<Output>,
    pub multiple: bool,
    reply: mpsc::Sender<Vec<String>>,
}

impl ScreenRequest {
    pub fn respond(&self, outputs: Vec<String>) {
        let _ = self.reply.send(outputs);
    }
}

/// Диалог оболочки.
pub trait ScreenPicker: Send + Sync {
    fn ask(&self, req: ScreenRequest);
    fn cancel(&self, id: u64);
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Программа захвата (`SYNSHELL_SCREENCAST` — подменить).
fn helper() -> String {
    if let Ok(p) = std::env::var("SYNSHELL_SCREENCAST") {
        return p;
    }
    // Рядом со своим бинарником (сборка из исходников), иначе из PATH.
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("syn-screencast")))
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "syn-screencast".into())
}

/// Мониторы (через помощника — он же их и снимает).
pub fn outputs() -> Vec<Output> {
    let Ok(o) = Command::new(helper()).arg("--list").stderr(Stdio::null()).output() else { return vec![] };
    let v: Value = serde_json::from_slice(&o.stdout).unwrap_or(Value::Null);
    v.as_array()
        .into_iter()
        .flatten()
        .map(|o| {
            let s = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            let i = |k: &str| o.get(k).and_then(Value::as_i64).unwrap_or(0) as i32;
            Output { name: s("name"), description: s("description"), x: i("x"), y: i("y"), width: i("width"), height: i("height"), scale: i("scale").max(1) }
        })
        .collect()
}

fn u32_opt(o: &HashMap<String, OwnedValue>, k: &str) -> Option<u32> {
    o.get(k).and_then(|v| u32::try_from(v).ok())
}

fn bool_opt(o: &HashMap<String, OwnedValue>, k: &str) -> bool {
    o.get(k).and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
}

fn ov<'a>(v: impl Into<ZValue<'a>>) -> OwnedValue {
    OwnedValue::try_from(v.into()).expect("значение без дескрипторов")
}

/// Спросить пользователя; `None` — отказ, время вышло или запрос закрыт.
async fn ask(picker: &Arc<dyn ScreenPicker>, server: &zbus::ObjectServer, handle: &OwnedObjectPath, app_id: &str, kind: ScreenAsk, multiple: bool) -> Option<Vec<String>> {
    let outs = blocking::unblock(outputs).await;
    if outs.is_empty() {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let registered = server.at(handle.as_ref(), Request { close: Mutex::new(Some(tx.clone())) }).await.unwrap_or(false);
    picker.ask(ScreenRequest { id, app_id: app_id.to_string(), kind, outputs: outs, multiple, reply: tx });
    let answer = blocking::unblock(move || rx.recv_timeout(ASK_TIMEOUT).ok()).await;
    picker.cancel(id);
    if registered {
        let _ = server.remove::<Request, _>(handle.as_ref()).await;
    }
    answer.filter(|a| !a.is_empty())
}

/// Объект запроса: `Close` — программа передумала.
struct Request {
    close: Mutex<Option<mpsc::Sender<Vec<String>>>>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    fn close(&self) {
        if let Some(tx) = self.close.lock().unwrap().take() {
            let _ = tx.send(Vec::new());
        }
    }
}

// ─── Сеансы ─────────────────────────────────────────────────────────────

#[derive(Default)]
struct SessionData {
    multiple: bool,
    cursor: u32,
    children: Vec<Child>,
}

fn sessions() -> &'static Mutex<HashMap<String, SessionData>> {
    static S: OnceLock<Mutex<HashMap<String, SessionData>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

fn end_session(path: &str) {
    if let Some(mut s) = sessions().lock().unwrap().remove(path) {
        for c in &mut s.children {
            // Закрытый stdin — помощник завершается сам; на всякий случай — SIGTERM.
            drop(c.stdin.take());
            unsafe { libc::kill(c.id() as i32, libc::SIGTERM) };
            let _ = c.wait();
        }
        tracing::info!(session = path, "портал: показ экрана закончен");
    }
}

struct Session {
    path: String,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl Session {
    async fn close(&self, #[zbus(object_server)] server: &zbus::ObjectServer) {
        end_session(&self.path);
        let _ = server.remove::<Session, _>(self.path.as_str()).await;
    }

    #[zbus(signal)]
    async fn closed(emitter: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

// ─── ScreenCast ─────────────────────────────────────────────────────────

pub struct ScreenCast {
    picker: Arc<dyn ScreenPicker>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCast {
    async fn create_session(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        _handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        _app_id: String,
        _options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let path = session_handle.to_string();
        sessions().lock().unwrap().insert(path.clone(), SessionData { cursor: CURSOR_EMBEDDED, ..Default::default() });
        if !server.at(session_handle.as_ref(), Session { path: path.clone() }).await.unwrap_or(false) {
            sessions().lock().unwrap().remove(&path);
            return (RESPONSE_OTHER, HashMap::new());
        }
        let mut r = HashMap::new();
        r.insert("session_id".to_string(), ov(path.as_str()));
        (RESPONSE_OK, r)
    }

    async fn select_sources(
        &self,
        _handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        _app_id: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let types = u32_opt(&options, "types").unwrap_or(SOURCE_MONITOR);
        if types & SOURCE_MONITOR == 0 {
            // Только окна — пока не умеем.
            return (RESPONSE_OTHER, HashMap::new());
        }
        let mut s = sessions().lock().unwrap();
        let Some(d) = s.get_mut(session_handle.as_str()) else { return (RESPONSE_OTHER, HashMap::new()) };
        d.multiple = bool_opt(&options, "multiple");
        d.cursor = match u32_opt(&options, "cursor_mode") {
            Some(CURSOR_HIDDEN) => CURSOR_HIDDEN,
            _ => CURSOR_EMBEDDED,
        };
        (RESPONSE_OK, HashMap::new())
    }

    async fn start(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(connection)] conn: &zbus::Connection,
        handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        _options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let path = session_handle.to_string();
        let (multiple, cursor) = match sessions().lock().unwrap().get(&path) {
            Some(d) => (d.multiple, d.cursor),
            None => return (RESPONSE_OTHER, HashMap::new()),
        };
        let Some(chosen) = ask(&self.picker, server, &handle, &app_id, ScreenAsk::Cast, multiple).await else {
            tracing::info!(app = %app_id, "портал: показ экрана — отказ");
            return (RESPONSE_CANCELLED, HashMap::new());
        };
        let outs = blocking::unblock(outputs).await;
        let mut streams: Vec<(u32, HashMap<String, OwnedValue>)> = Vec::new();
        for name in chosen {
            let Some(o) = outs.iter().find(|o| o.name == name).cloned() else { continue };
            let cur = if cursor == CURSOR_HIDDEN { "0" } else { "1" };
            let spawned = Command::new(helper()).args(["--stream", &o.name, "--cursor", cur]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn();
            let Ok(mut child) = spawned else {
                tracing::warn!("портал: не запустился syn-screencast");
                continue;
            };
            let out = child.stdout.take().unwrap();
            let line = blocking::unblock(move || {
                let mut l = String::new();
                std::io::BufReader::new(out).read_line(&mut l).ok().map(|_| l)
            })
            .await
            .unwrap_or_default();
            let node = serde_json::from_str::<Value>(&line).ok().and_then(|v| v.get("node")?.as_u64()).map(|n| n as u32);
            let Some(node) = node else {
                let _ = child.kill();
                let _ = child.wait();
                tracing::warn!("портал: syn-screencast не дал узел PipeWire");
                continue;
            };
            // Помощник умер сам (вывод отключён, ошибка) — закрыть сеанс у программы.
            let pid = child.id();
            let watch_path = path.clone();
            let conn = conn.clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_millis(500));
                    let alive = {
                        let mut s = sessions().lock().unwrap();
                        match s.get_mut(&watch_path) {
                            None => return,
                            Some(d) => d.children.iter_mut().any(|c| c.id() == pid && c.try_wait().ok().flatten().is_none()),
                        }
                    };
                    if !alive {
                        end_session(&watch_path);
                        let _ = zbus::block_on(async {
                            let iface = conn.object_server().interface::<_, Session>(watch_path.as_str()).await?;
                            Session::closed(iface.signal_emitter()).await?;
                            conn.object_server().remove::<Session, _>(watch_path.as_str()).await.map(|_| ())
                        });
                        return;
                    }
                }
            });
            if let Some(d) = sessions().lock().unwrap().get_mut(&path) {
                d.children.push(child);
            }
            let mut props = HashMap::new();
            props.insert("position".to_string(), ov((o.x, o.y)));
            props.insert("size".to_string(), ov((o.width / o.scale.max(1), o.height / o.scale.max(1))));
            props.insert("source_type".to_string(), ov(SOURCE_MONITOR));
            streams.push((node, props));
            tracing::info!(app = %app_id, output = %o.name, node, "портал: показ экрана");
        }
        if streams.is_empty() {
            end_session(&path);
            return (RESPONSE_OTHER, HashMap::new());
        }
        let mut r = HashMap::new();
        r.insert("streams".to_string(), ov(streams));
        (RESPONSE_OK, r)
    }

    #[zbus(property)]
    fn available_source_types(&self) -> u32 {
        SOURCE_MONITOR
    }

    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 {
        CURSOR_HIDDEN | CURSOR_EMBEDDED
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }
}

// ─── Screenshot ─────────────────────────────────────────────────────────

pub struct Screenshot {
    picker: Arc<dyn ScreenPicker>,
}

/// Вывод под фокусом (`synwm msg focused-output`), иначе первый.
fn focused_output() -> String {
    Command::new("synwm")
        .args(["msg", "focused-output"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty() && !s.contains(' '))
        .unwrap_or_default()
}

fn shot(output: &str) -> Option<String> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join("synshell-screenshots");
    let _ = std::fs::create_dir_all(&dir);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let file = dir.join(format!("screenshot-{stamp}.png"));
    let st = Command::new(helper()).arg("--screenshot").arg(&file).args(["--output", output]).stdout(Stdio::null()).status().ok()?;
    st.success().then(|| format!("file://{}", file.display()))
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl Screenshot {
    async fn screenshot(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        // Неинтерактивный снимок портал уже разрешил сам (хранилище разрешений).
        let output = if bool_opt(&options, "interactive") {
            match ask(&self.picker, server, &handle, &app_id, ScreenAsk::Shot, false).await {
                Some(v) => v.into_iter().next().unwrap_or_default(),
                None => return (RESPONSE_CANCELLED, HashMap::new()),
            }
        } else {
            blocking::unblock(focused_output).await
        };
        match blocking::unblock(move || shot(&output)).await {
            Some(uri) => {
                tracing::info!(app = %app_id, "портал: снимок экрана");
                let mut r = HashMap::new();
                r.insert("uri".to_string(), ov(uri.as_str()));
                (RESPONSE_OK, r)
            }
            None => (RESPONSE_OTHER, HashMap::new()),
        }
    }

    /// Выбор цвета пока не поддержан.
    async fn pick_color(&self, _handle: OwnedObjectPath, _app_id: String, _parent_window: String, _options: HashMap<String, OwnedValue>) -> (u32, HashMap<String, OwnedValue>) {
        (RESPONSE_OTHER, HashMap::new())
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }
}

pub fn interfaces(picker: impl ScreenPicker + 'static) -> (ScreenCast, Screenshot) {
    let p: Arc<dyn ScreenPicker> = Arc::new(picker);
    (ScreenCast { picker: p.clone() }, Screenshot { picker: p })
}
