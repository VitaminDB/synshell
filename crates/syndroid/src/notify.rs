//! Уведомление о ходе фонового задания (загрузка образов): `syndroidd __notify <id>` от имени владельца
//! сеанса. Опрашивает демон и держит одно уведомление с полосой хода (подсказка `value`), кнопками «Отмена» и
//! «Повторить сейчас»; по окончании — отдельное уведомление «скачано» или об ошибке. Живёт, пока живо
//! задание, — окно `syndroid` для этого не нужно.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use zbus::blocking::Connection;
use zbus::zvariant::Value;

use crate::api::{self, Job, Request, Response};

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

/// Размер для людей: «512 МБ», «1.3 ГБ».
pub fn human(b: u64) -> String {
    let mb = b as f64 / 1048576.0;
    if mb >= 1024.0 {
        format!("{:.1} ГБ", mb / 1024.0)
    } else {
        format!("{mb:.0} МБ")
    }
}

/// Местное время «ЧЧ:ММ».
pub fn clock(ts: i64) -> String {
    let t = ts as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

/// Строка о ходе задания: «загрузка system: 512 МБ из 1.3 ГБ» или «Повтор в 11:20 (попытка 2)».
pub fn job_line(j: &Job) -> String {
    if let Some(at) = j.retry_at {
        return format!("Повтор в {} (попытка {})", clock(at), j.attempt + 1);
    }
    if j.total > 0 {
        format!("{}: {} из {}", j.step, human(j.done), human(j.total))
    } else if j.done > 0 {
        format!("{}: {}", j.step, human(j.done))
    } else {
        j.step.clone()
    }
}

/// Первая строка ошибки без хвоста с адресом — для уведомления.
fn short_error(e: &str) -> String {
    let e = e.lines().next().unwrap_or(e);
    match e.rfind(" (http") {
        Some(i) => e[..i].to_string(),
        None => e.to_string(),
    }
}

struct Notifier {
    conn: Connection,
    /// Текущее уведомление о ходе (0 — нет).
    id: Arc<AtomicU32>,
}

impl Notifier {
    fn notify(&self, replaces: u32, summary: &str, body: &str, actions: &[&str], hints: HashMap<&str, Value>) -> Result<u32> {
        let reply = self.conn.call_method(
            Some(DEST),
            PATH,
            Some(DEST),
            "Notify",
            &("Android", replaces, "syndroid", summary, body, actions, hints, -1i32),
        )?;
        Ok(reply.body().deserialize()?)
    }

    fn close(&self) {
        let id = self.id.swap(0, Ordering::Relaxed);
        if id != 0 {
            let _ = self.conn.call_method(Some(DEST), PATH, Some(DEST), "CloseNotification", &(id,));
        }
    }
}

fn hints(progress: Option<i32>, urgency: u8) -> HashMap<&'static str, Value<'static>> {
    let mut h: HashMap<&str, Value> = HashMap::new();
    h.insert("desktop-entry", "syndroid".into());
    h.insert("category", "transfer".into());
    h.insert("urgency", urgency.into());
    if let Some(v) = progress {
        h.insert("value", v.into());
    }
    // Ход загрузки (urgency 0) не исчезает по клику — как текущие уведомления Android
    h.insert("resident", (urgency == 0).into());
    h
}

/// Нажатия на уведомления: «Отмена», «Повторить сейчас», клик — окно syndroid на «Образах».
fn listen_actions(conn: Connection, job: u64, ours: Arc<AtomicU32>) {
    std::thread::spawn(move || {
        let rule = "type='signal',interface='org.freedesktop.Notifications',member='ActionInvoked'";
        let Ok(rule) = zbus::MatchRule::try_from(rule) else { return };
        let Ok(it) = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, None) else { return };
        for msg in it.flatten() {
            let Ok((id, action)) = msg.body().deserialize::<(u32, String)>() else { continue };
            if id == 0 || id != ours.load(Ordering::Relaxed) {
                continue;
            }
            let r = match action.as_str() {
                "cancel" => api::call(&Request::CancelJob { id: job }).map(drop),
                "retry" => api::call(&Request::RetryJobNow { id: job }).map(drop),
                _ => std::process::Command::new("syndroid").arg("--images").spawn().map(drop).map_err(Into::into),
            };
            if let Err(e) = r {
                tracing::warn!("уведомление: {action}: {e:#}");
            }
        }
    });
}

fn find_job(id: u64) -> Option<Job> {
    match api::call(&Request::Status) {
        Ok(Response::Status(s)) => s.jobs.into_iter().find(|j| j.id == id),
        _ => None,
    }
}

pub fn job_main(args: &[String]) -> ! {
    let Some(job) = args.first().and_then(|a| a.parse::<u64>().ok()) else {
        eprintln!("использование: syndroidd __notify <номер задания>");
        std::process::exit(2);
    };
    // D-Bus сеанса может ещё не быть (загрузка продолжена при старте системы, до входа)
    let conn = loop {
        match Connection::session() {
            Ok(c) => break c,
            Err(_) if find_job(job).is_some_and(|j| !j.finished) => std::thread::sleep(Duration::from_secs(5)),
            Err(e) => {
                tracing::debug!("D-Bus сеанса: {e}");
                std::process::exit(0);
            }
        }
    };
    let n = Notifier { conn: conn.clone(), id: Arc::new(AtomicU32::new(0)) };
    listen_actions(conn, job, n.id.clone());
    let mut last = String::new();
    loop {
        let Some(j) = find_job(job) else {
            // Демон перезапущен: задание продолжит новый уведомитель
            n.close();
            std::process::exit(0);
        };
        let summary = format!("Загрузка Android · {}", if j.subject.is_empty() { "LineageOS" } else { &j.subject });
        if j.finished {
            n.close();
            let r = if j.cancelled {
                Ok(0)
            } else if let Some(e) = &j.error {
                n.notify(0, "Android не скачан", &short_error(e), &["default", "Открыть"], hints(None, 1))
            } else {
                let what = j.result.as_deref().map(|r| format!("{} · {r}", crate::images::instance_title(&crate::images::instance_of(r))));
                n.notify(0, "Android скачан", what.as_deref().unwrap_or(&j.subject), &["default", "Открыть"], hints(None, 1))
            };
            if let Err(e) = r {
                tracing::warn!("уведомление: {e:#}");
            }
            std::process::exit(0);
        }
        let (body, progress, actions): (String, Option<i32>, Vec<&str>) = if j.retry_at.is_some() {
            let err = j.last_error.as_deref().map(short_error).unwrap_or_default();
            (format!("{}\n{err}", job_line(&j)), None, vec!["default", "Открыть", "retry", "Повторить сейчас", "cancel", "Отмена"])
        } else {
            let pct = (j.total > 0).then(|| (j.done.saturating_mul(100) / j.total).min(100) as i32).unwrap_or(-1);
            let line = if j.total > 0 { format!("{} · {pct}%", job_line(&j)) } else { job_line(&j) };
            (line, Some(pct), vec!["default", "Открыть", "cancel", "Отмена"])
        };
        let key = format!("{summary}|{body}|{progress:?}|{}", actions.len());
        if key != last {
            match n.notify(n.id.load(Ordering::Relaxed), &summary, &body, &actions, hints(progress, 0)) {
                Ok(id) => {
                    n.id.store(id, Ordering::Relaxed);
                    last = key;
                }
                Err(e) => tracing::debug!("уведомление: {e:#}"),
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}
