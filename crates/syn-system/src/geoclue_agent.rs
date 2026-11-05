//! Агент GeoClue: решает, каким программам отдавать местоположение.
//!
//! GeoClue (`org.freedesktop.GeoClue2` на системной шине) не выдаёт
//! местоположение клиенту, пока агент его пользователя не ответит на
//! `AuthorizeApp(desktop_id, точность)`; без агента клиенты ждут и
//! отваливаются по тайм-ауту. Оболочка регистрирует агентом себя
//! (`AddAgent("synshell")` — id должен быть в `[agent] whitelist` конфига
//! GeoClue) и решает по [`Policy`]: разрешено/запрещено навсегда или
//! спросить пользователя через [`Prompter`].
//!
//! GeoClue запускается по D-Bus и после перезапуска агентов не помнит —
//! регистрация повторяется, когда имя `org.freedesktop.GeoClue2` снова
//! появляется на шине.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;

use zbus::blocking::Connection;

/// Id агента для `[agent] whitelist` GeoClue.
pub const AGENT_ID: &str = "synshell";
const AGENT_PATH: &str = "/org/freedesktop/GeoClue2/Agent";
const GEOCLUE: &str = "org.freedesktop.GeoClue2";
/// Уровни точности GeoClue: 0 — нет, 8 — точное (спутники).
pub const ACCURACY_NONE: u32 = 0;
pub const ACCURACY_EXACT: u32 = 8;
/// Сколько ждать ответа пользователя (GeoClue сам ждёт клиента дольше).
const ASK_TIMEOUT: Duration = Duration::from_secs(60);

/// Решение по программе без вопроса.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    Ask,
}

/// Правила приложения-агента: решение по id программы (.desktop без
/// `.desktop`) и общий выключатель.
pub trait Policy: Send + Sync {
    fn decide(&self, desktop_id: &str) -> Decision;
    fn enabled(&self) -> bool;
}

/// Вопрос пользователю. Ответить — [`LocationRequest::respond`]; без ответа
/// за минуту — отказ.
pub struct LocationRequest {
    pub id: u64,
    pub desktop_id: String,
    /// Запрошенная точность GeoClue (8 — точное).
    pub accuracy: u32,
    reply: mpsc::Sender<bool>,
}

impl LocationRequest {
    pub fn respond(&self, allow: bool) {
        let _ = self.reply.send(allow);
    }
}

/// Интерфейс приложения: показать вопрос и убрать его (ответ получен или
/// истекло время).
pub trait Prompter: Send + Sync {
    fn ask(&self, req: LocationRequest);
    fn cancel(&self, id: u64);
}

struct Agent {
    policy: Arc<dyn Policy>,
    prompter: Arc<dyn Prompter>,
    max: Arc<AtomicU32>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[zbus::interface(name = "org.freedesktop.GeoClue2.Agent")]
impl Agent {
    async fn authorize_app(&self, desktop_id: String, req_accuracy_level: u32) -> (bool, u32) {
        if !self.policy.enabled() {
            return (false, ACCURACY_NONE);
        }
        let allow = match self.policy.decide(&desktop_id) {
            Decision::Allow => true,
            Decision::Deny => false,
            Decision::Ask => {
                let (tx, rx) = mpsc::channel();
                let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                self.prompter.ask(LocationRequest { id, desktop_id: desktop_id.clone(), accuracy: req_accuracy_level, reply: tx });
                let answer = blocking::unblock(move || rx.recv_timeout(ASK_TIMEOUT).ok()).await;
                if answer.is_none() {
                    self.prompter.cancel(id);
                }
                answer.unwrap_or(false)
            }
        };
        tracing::info!(app = %desktop_id, allow, "GeoClue: запрос местоположения");
        if allow {
            (true, req_accuracy_level.min(self.max.load(Ordering::Relaxed)))
        } else {
            (false, ACCURACY_NONE)
        }
    }

    #[zbus(property)]
    fn max_accuracy_level(&self) -> u32 {
        self.max.load(Ordering::Relaxed)
    }
}

/// Работающий агент.
pub struct Handle {
    conn: Connection,
    max: Arc<AtomicU32>,
}

static HANDLE: OnceLock<Mutex<Option<Arc<Handle>>>> = OnceLock::new();

impl Handle {
    /// Общий выключатель изменился: GeoClue узнаёт по свойству
    /// `MaxAccuracyLevel` и перестаёт (или снова начинает) отдавать данные.
    pub fn set_enabled(&self, on: bool) {
        let level = if on { ACCURACY_EXACT } else { ACCURACY_NONE };
        if self.max.swap(level, Ordering::Relaxed) == level {
            return;
        }
        if let Ok(iface) = self.conn.object_server().interface::<_, Agent>(AGENT_PATH) {
            let _ = zbus::block_on(iface.get().max_accuracy_level_changed(iface.signal_emitter()));
        }
    }
}

fn add_agent(conn: &Connection) -> zbus::Result<()> {
    conn.call_method(Some(GEOCLUE), "/org/freedesktop/GeoClue2/Manager", Some("org.freedesktop.GeoClue2.Manager"), "AddAgent", &(AGENT_ID,))
        .map(|_| ())
}

/// Зарегистрировать агента (один раз на процесс) и следить за перезапуском
/// GeoClue. Блокирующая — звать из фонового потока; GeoClue нет на шине —
/// ошибка.
pub fn start(policy: impl Policy + 'static, prompter: impl Prompter + 'static) -> Result<Arc<Handle>, String> {
    let slot = HANDLE.get_or_init(|| Mutex::new(None));
    if let Some(h) = slot.lock().unwrap().clone() {
        return Ok(h);
    }
    let conn = Connection::system().map_err(|e| e.to_string())?;
    let max = Arc::new(AtomicU32::new(if policy.enabled() { ACCURACY_EXACT } else { ACCURACY_NONE }));
    let agent = Agent { policy: Arc::new(policy), prompter: Arc::new(prompter), max: max.clone() };
    conn.object_server().at(AGENT_PATH, agent).map_err(|e| e.to_string())?;
    add_agent(&conn).map_err(|e| format!("GeoClue AddAgent: {e}"))?;
    let h = Arc::new(Handle { conn: conn.clone(), max });
    *slot.lock().unwrap() = Some(h.clone());
    std::thread::Builder::new()
        .name("geoclue-agent".into())
        .spawn(move || {
            let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conn) else { return };
            let Ok(changes) = dbus.receive_name_owner_changed_with_args(&[(0, GEOCLUE)]) else { return };
            for ch in changes {
                let Ok(args) = ch.args() else { continue };
                if args.new_owner().is_some() {
                    // GeoClue перезапустился: агентов он не помнит
                    if let Err(e) = add_agent(&conn) {
                        tracing::warn!("GeoClue AddAgent: {e}");
                    }
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(h)
}
