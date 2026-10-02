//! Бэкенд портала доступа `org.freedesktop.impl.portal.Access`: диалог
//! «разрешить программе …?» для xdg-desktop-portal.
//!
//! Фронтенд портала (xdg-desktop-portal) сам ведёт хранилище разрешений и
//! выдаёт доступ: портал Camera спрашивает бэкенд `AccessDialog` только для
//! программы без сохранённого решения, ответ запоминает в таблице
//! `devices`/`camera` (её же читает WirePlumber, открывая программе узлы
//! камер PipeWire). Оболочка регистрирует имя
//! `org.freedesktop.impl.portal.desktop.synshell` на шине сеанса (файл
//! `synshell.portal`, выбор бэкенда — `synshell-portals.conf`) и спрашивает
//! пользователя через [`Prompter`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.synshell";
const PATH: &str = "/org/freedesktop/portal/desktop";
/// Ответ портала: 0 — разрешено, 1 — отменено (отказ).
const RESPONSE_OK: u32 = 0;
const RESPONSE_CANCELLED: u32 = 1;
/// Сколько ждать ответа пользователя.
const ASK_TIMEOUT: Duration = Duration::from_secs(120);

/// Вопрос пользователю. Ответить — [`AccessRequest::respond`]; без ответа за
/// две минуты или при закрытии запроса программой — отказ.
pub struct AccessRequest {
    pub id: u64,
    /// Id программы (.desktop без `.desktop`); пусто — программа не из
    /// песочницы, портал её не различает.
    pub app_id: String,
    pub title: String,
    pub subtitle: String,
    pub body: String,
    pub grant_label: Option<String>,
    pub deny_label: Option<String>,
    /// Значок, который предлагает портал (имя темы, например `camera-web-symbolic`).
    pub icon: Option<String>,
    reply: mpsc::Sender<bool>,
}

impl AccessRequest {
    pub fn respond(&self, allow: bool) {
        let _ = self.reply.send(allow);
    }
}

/// Интерфейс приложения: показать вопрос и убрать его (ответ получен, время
/// вышло или программа закрыла запрос).
pub trait Prompter: Send + Sync {
    fn ask(&self, req: AccessRequest);
    fn cancel(&self, id: u64);
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct Access {
    prompter: Arc<dyn Prompter>,
}

/// Объект запроса по пути `handle`: портал зовёт `Close`, если программа
/// передумала.
struct Request {
    close: Mutex<Option<mpsc::Sender<bool>>>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    fn close(&self) {
        if let Some(tx) = self.close.lock().unwrap().take() {
            let _ = tx.send(false);
        }
    }
}

fn opt_str(options: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    options.get(key).and_then(|v| String::try_from(v.try_clone().ok()?).ok()).filter(|s| !s.is_empty())
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Access")]
impl Access {
    #[allow(clippy::too_many_arguments)]
    async fn access_dialog(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        title: String,
        subtitle: String,
        body: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let (tx, rx) = mpsc::channel();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let req_obj = Request { close: Mutex::new(Some(tx.clone())) };
        let registered = server.at(handle.as_ref(), req_obj).await.unwrap_or(false);
        self.prompter.ask(AccessRequest {
            id,
            app_id: app_id.clone(),
            title,
            subtitle,
            body,
            grant_label: opt_str(&options, "grant_label"),
            deny_label: opt_str(&options, "deny_label"),
            icon: opt_str(&options, "icon"),
            reply: tx,
        });
        let answer = blocking::unblock(move || rx.recv_timeout(ASK_TIMEOUT).ok()).await;
        // ответ из окна убирает вопрос сам; время вышло или Close — убрать здесь
        self.prompter.cancel(id);
        if registered {
            let _ = server.remove::<Request, _>(handle.as_ref()).await;
        }
        let allow = answer.unwrap_or(false);
        tracing::info!(app = %app_id, allow, "портал: запрос доступа");
        (if allow { RESPONSE_OK } else { RESPONSE_CANCELLED }, HashMap::new())
    }
}

/// Занять имя бэкенда на шине сеанса и обслуживать `AccessDialog`.
/// Соединение живёт, пока жив возвращённый [`Connection`].
pub fn start(prompter: impl Prompter + 'static) -> Result<Connection, String> {
    zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name(BUS_NAME))
        .and_then(|b| b.serve_at(PATH, Access { prompter: Arc::new(prompter) }))
        .and_then(|b| b.build())
        .map_err(|e| e.to_string())
}

// --- решения порталов (хранилище разрешений xdg-desktop-portal) -------------------------------------------

const STORE: &str = "org.freedesktop.impl.portal.PermissionStore";
const STORE_PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";

macro_rules! store_call {
    ($method:expr, $body:expr) => {
        Connection::session().and_then(|c| c.call_method(Some(STORE), STORE_PATH, Some(STORE), $method, $body))
    };
}

/// Решения портала Camera: (id программы, разрешено). Пустой id — программы вне песочницы (одно решение на
/// всех). Нет таблицы — пустой список.
pub fn camera_permissions() -> Result<Vec<(String, bool)>, String> {
    match store_call!("Lookup", &("devices", "camera")) {
        Ok(m) => {
            let (perms, _data): (HashMap<String, Vec<String>>, OwnedValue) = m.body().deserialize().map_err(|e| e.to_string())?;
            let mut v: Vec<(String, bool)> = perms.into_iter().map(|(app, p)| (app, p.iter().any(|x| x == "yes"))).collect();
            v.sort();
            Ok(v)
        }
        // «No entry for camera» — ещё никто не спрашивал
        Err(zbus::Error::MethodError(_, _, _)) => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}

/// Разрешить или запретить программе камеру (WirePlumber применяет сразу — следит за хранилищем).
pub fn set_camera_permission(app_id: &str, allow: bool) -> Result<(), String> {
    let perm: Vec<&str> = vec![if allow { "yes" } else { "no" }];
    store_call!("SetPermission", &("devices", true, "camera", app_id, perm)).map(|_| ()).map_err(|e| e.to_string())
}

/// Забыть решение: при следующем запросе портал спросит снова.
pub fn forget_camera_permission(app_id: &str) -> Result<(), String> {
    store_call!("DeletePermission", &("devices", "camera", app_id)).map(|_| ()).map_err(|e| e.to_string())
}
