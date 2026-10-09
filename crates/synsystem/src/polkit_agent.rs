//! Агент аутентификации polkit внутри приложения (как `pkttyagent --process`).
//!
//! polkitd ищет агента по субъекту проверки: сначала агент, зарегистрированный
//! ровно для этого процесса, затем — для его logind-сеанса. Сеанс synlogin на
//! телефоне идёт без logind, сеансового агента нет — и `pkexec` из приложения
//! падал с «Error creating textual authentication agent». Поэтому приложение
//! регистрирует агентом само себя (субъект `unix-process` = свой pid), и
//! `pkexec`, запущенный им, спрашивает пароль через [`Prompter`] приложения.
//! На рабочем столе это тоже работает: агент процесса важнее сеансового.
//!
//! Пароль проверяет `polkit-agent-helper-1` (PAM): через сокет
//! `/run/polkit/agent-helper.socket` (polkit ≥ 126), иначе — setuid-помощник.
//! Помощнику за сокетом нужен pidfd пира (ядро ≥ 6.5); на старом ядре
//! (Android GKI 5.10) он молча закрывает соединение — тогда до конца жизни
//! процесса берём setuid-помощник.
//! Протокол: имя пользователя (только сокет) и cookie построчно, затем строки
//! `PAM_PROMPT_ECHO_OFF …` / `PAM_ERROR_MSG …` / `PAM_TEXT_INFO …` →
//! ответ строкой; итог — `SUCCESS` или `FAILURE`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};

use zbus::zvariant::{OwnedValue, Value};
use synshell_tr::t;

const AGENT_PATH: &str = "/org/synshell/PolkitAgent";
const HELPER_SOCKET: &str = "/run/polkit/agent-helper.socket";
const HELPER_BIN: &str = "/usr/lib/polkit-1/polkit-agent-helper-1";
/// Попыток ввода пароля на один запрос polkit.
const ATTEMPTS: u32 = 3;

/// Запрос пароля для приложения. Ответить — [`AuthRequest::respond`]
/// (`None` — отмена); без ответа запрос висит, пока polkit его не отменит.
pub struct AuthRequest {
    /// Для [`Prompter::cancel`].
    pub id: u64,
    /// Что просят сделать (сообщение действия polkit, уже локализованное).
    pub message: String,
    pub action_id: String,
    /// Чей пароль спрашивается (логин).
    pub user: String,
    /// Подсказка PAM («Password:»).
    pub prompt: String,
    /// Пароль скрыт при вводе (почти всегда).
    pub secret: bool,
    /// Ошибка предыдущей попытки («Неверный пароль») или сообщение PAM.
    pub error: Option<String>,
    reply: mpsc::Sender<Option<String>>,
}

impl AuthRequest {
    pub fn respond(&self, password: Option<String>) {
        let _ = self.reply.send(password);
    }
}

/// Интерфейс приложения: показать окно ввода пароля и убрать его.
pub trait Prompter: Send + Sync {
    /// Показать окно (вызывается не из потока интерфейса).
    fn ask(&self, req: AuthRequest);
    /// Убрать окно запроса `id`: ответ получен или polkit отменил запрос.
    fn cancel(&self, id: u64);
}

static PROMPTER: OnceLock<Arc<dyn Prompter>> = OnceLock::new();
static AGENT: OnceLock<Result<zbus::blocking::Connection, String>> = OnceLock::new();
static SESSION_AGENT: OnceLock<Result<zbus::blocking::Connection, String>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// Помощник за сокетом закрыл соединение, ничего не ответив (нет pidfd в ядре).
static SOCKET_BROKEN: AtomicBool = AtomicBool::new(false);
/// Запросы в работе: cookie → (id окна, канал ответа) — для CancelAuthentication.
static PENDING: Mutex<Vec<(String, u64, mpsc::Sender<Option<String>>)>> = Mutex::new(Vec::new());

/// Задать окно ввода пароля приложения (один раз, при старте).
pub fn set_prompter(p: impl Prompter + 'static) {
    let _ = PROMPTER.set(Arc::new(p));
}

/// Зарегистрировать процесс агентом polkit (один раз; повторные вызовы —
/// результат первого). Без [`set_prompter`] ничего не делает: тогда пароль
/// спросит сеансовый агент рабочего стола, если он есть.
pub fn ensure() -> Result<(), String> {
    if PROMPTER.get().is_none() {
        return Ok(());
    }
    AGENT.get_or_init(register).as_ref().map(|_| ()).map_err(Clone::clone)
}

/// Зарегистрировать сеансовым агентом (субъект `unix-session`): пароль спрашивают
/// все программы сеанса, которым нужен pkexec и которые не регистрируют свой агент —
/// например, GParted, запущенный из меню. Нужен [`set_prompter`]; вызывать один раз
/// из оболочки сеанса (повторные вызовы — результат первого). Сеанс берётся из
/// `XDG_SESSION_ID` (logind выставляет его при входе).
pub fn ensure_session() -> Result<(), String> {
    if PROMPTER.get().is_none() {
        return Err(t!("polkit-агент сеанса: нет окна ввода пароля").into());
    }
    SESSION_AGENT.get_or_init(register_session).as_ref().map(|_| ()).map_err(Clone::clone)
}

fn register() -> Result<zbus::blocking::Connection, String> {
    let pid = std::process::id();
    let start = process_start_time(pid).ok_or(t!("polkit-агент: нет start-time процесса"))?;
    let mut details: HashMap<&str, Value> = HashMap::new();
    details.insert("pid", Value::U32(pid));
    details.insert("start-time", Value::U64(start));
    let conn = register_subject(("unix-process", details))?;
    tracing::info!(pid, "polkit-агент процесса зарегистрирован");
    Ok(conn)
}

fn register_session() -> Result<zbus::blocking::Connection, String> {
    let session = std::env::var("XDG_SESSION_ID").map_err(|_| "polkit-агент сеанса: нет XDG_SESSION_ID")?;
    let mut details: HashMap<&str, Value> = HashMap::new();
    details.insert("session-id", Value::new(session.as_str()));
    let conn = register_subject(("unix-session", details))?;
    tracing::info!(session = %session, "polkit-агент сеанса зарегистрирован");
    Ok(conn)
}

/// Общая часть: своя системная шина с объектом агента и вызов `RegisterAuthenticationAgent`.
fn register_subject(subject: (&str, HashMap<&str, Value>)) -> Result<zbus::blocking::Connection, String> {
    let conn = zbus::blocking::connection::Builder::system()
        .and_then(|b| b.serve_at(AGENT_PATH, Agent))
        .and_then(|b| b.build())
        .map_err(|e| t!("polkit-агент: системная шина: {e}", e = e))?;
    let locale = std::env::var("LC_ALL").or_else(|_| std::env::var("LANG")).unwrap_or_else(|_| "C".into());
    conn.call_method(
        Some("org.freedesktop.PolicyKit1"),
        "/org/freedesktop/PolicyKit1/Authority",
        Some("org.freedesktop.PolicyKit1.Authority"),
        "RegisterAuthenticationAgent",
        &(subject, locale.as_str(), AGENT_PATH),
    )
    .map_err(|e| t!("polkit-агент: регистрация: {e}", e = e))?;
    Ok(conn)
}

/// Время старта процесса в тиках (поле 22 `/proc/PID/stat`) — как считает polkit.
fn process_start_time(pid: u32) -> Option<u64> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    s.rsplit_once(')')?.1.split_whitespace().nth(19)?.parse().ok()
}

fn user_name(uid: u32) -> Option<String> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() > 2 && f[2].parse() == Ok(uid)).then(|| f[0].to_string())
    })
}

struct Agent;

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    async fn begin_authentication(
        &self,
        action_id: String,
        message: String,
        _icon_name: String,
        _details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, OwnedValue>)>,
    ) -> zbus::fdo::Result<()> {
        // Чей пароль: свой, если polkit его принимает (auth_self или мы в wheel), иначе root, иначе первый.
        let uids: Vec<u32> = identities
            .iter()
            .filter(|(kind, _)| kind == "unix-user")
            .filter_map(|(_, d)| d.get("uid").and_then(|v| u32::try_from(v).ok()))
            .collect();
        // SAFETY: getuid без побочных эффектов.
        let me = unsafe { libc::getuid() };
        let uid = [me, 0].into_iter().find(|u| uids.contains(u)).or(uids.first().copied());
        let Some(user) = uid.and_then(user_name) else {
            return Err(zbus::fdo::Error::Failed(t!("нет подходящего пользователя для аутентификации").into()));
        };
        let res = blocking::unblock(move || authenticate(&action_id, &message, &user, &cookie)).await;
        res.map_err(zbus::fdo::Error::Failed)
    }

    async fn cancel_authentication(&self, cookie: String) {
        if let Ok(p) = PENDING.lock() {
            for (c, _, tx) in p.iter() {
                if *c == cookie {
                    let _ = tx.send(None);
                }
            }
        }
    }
}

/// Строка помощника в C-экранировании (g_strescape) → текст.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

type Helper = (Box<dyn BufRead + Send>, Box<dyn Write + Send>, bool);

/// Помощник PAM: (чтение, запись, через сокет ли). Сокет polkit ≥ 126 или setuid-бинарник.
fn helper(user: &str, cookie: &str) -> Result<Helper, String> {
    if !SOCKET_BROKEN.load(Ordering::Relaxed) {
        if let Ok(s) = UnixStream::connect(HELPER_SOCKET) {
            let mut w = s.try_clone().map_err(|e| e.to_string())?;
            writeln!(w, "{user}\n{cookie}").map_err(|e| t!("помощник polkit: {e}", e = e))?;
            return Ok((Box::new(BufReader::new(s)), Box::new(w), true));
        }
    }
    let mut child = Command::new(HELPER_BIN)
        .arg(user)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{HELPER_BIN}: {e}"))?;
    let mut w = child.stdin.take().ok_or(t!("нет stdin помощника"))?;
    let r = child.stdout.take().ok_or(t!("нет stdout помощника"))?;
    std::thread::spawn(move || child.wait());
    writeln!(w, "{cookie}").map_err(|e| t!("помощник polkit: {e}", e = e))?;
    Ok((Box::new(BufReader::new(r)), Box::new(w), false))
}

fn authenticate(action_id: &str, message: &str, user: &str, cookie: &str) -> Result<(), String> {
    let prompter = PROMPTER.get().cloned().ok_or(t!("нет окна ввода пароля"))?;
    let (tx, rx) = mpsc::channel::<Option<String>>();
    let mut error: Option<String> = None;
    let mut cancelled = false;
    let mut attempt = 0;
    while attempt < ATTEMPTS {
        let (mut r, mut w, via_socket) = helper(user, cookie)?;
        let mut info: Option<String> = None;
        let mut ok = false;
        let mut silent = true;
        let mut line = String::new();
        loop {
            line.clear();
            if r.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            silent = false;
            let l = unescape(line.trim_end_matches('\n'));
            let (secret, prompt) = if let Some(p) = l.strip_prefix("PAM_PROMPT_ECHO_OFF ") {
                (true, p)
            } else if let Some(p) = l.strip_prefix("PAM_PROMPT_ECHO_ON ") {
                (false, p)
            } else {
                if let Some(m) = l.strip_prefix("PAM_ERROR_MSG ").or_else(|| l.strip_prefix("PAM_TEXT_INFO ")) {
                    info = Some(m.trim().to_string());
                } else if l.starts_with("SUCCESS") {
                    ok = true;
                    break;
                } else if l.starts_with("FAILURE") {
                    break;
                }
                continue;
            };
            while rx.try_recv().is_ok() {} // запоздалые ответы прошлых окон
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut p) = PENDING.lock() {
                p.push((cookie.to_string(), id, tx.clone()));
            }
            prompter.ask(AuthRequest {
                id,
                message: message.to_string(),
                action_id: action_id.to_string(),
                user: user.to_string(),
                prompt: prompt.trim().to_string(),
                secret,
                error: info.take().or_else(|| error.take()),
                reply: tx.clone(),
            });
            let answer = rx.recv().ok().flatten();
            if let Ok(mut p) = PENDING.lock() {
                p.retain(|(_, i, _)| *i != id);
            }
            prompter.cancel(id);
            let Some(pw) = answer else {
                cancelled = true;
                break;
            };
            if writeln!(w, "{pw}").is_err() {
                break;
            }
        }
        if ok {
            return Ok(());
        }
        if cancelled {
            return Err(t!("отменено").into());
        }
        if via_socket && silent {
            tracing::warn!("помощник polkit за сокетом не ответил (нет pidfd?) — setuid-помощник");
            SOCKET_BROKEN.store(true, Ordering::Relaxed);
            continue;
        }
        attempt += 1;
        error = Some(t!("Неверный пароль").into());
    }
    Err(t!("аутентификация не удалась").into())
}
