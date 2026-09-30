//! Уведомления: свои пересылаем на спаренные устройства, чужие
//! показываем у себя (через обычный `org.freedesktop.Notifications` —
//! попадают в центр уведомлений оболочки).
//!
//! Свои перехватываются мониторингом шины сеанса (`BecomeMonitor`):
//! сервер уведомлений — сама оболочка, отдельного сигнала «появилось
//! уведомление» у протокола нет. Пересланные помечены подсказкой
//! `x-synlink-device` и обратно не уходят.

use std::collections::HashMap;
use std::sync::OnceLock;

use synshell_common::link::{PairPrompt, RemoteNotification};
use tokio::sync::mpsc;
use zbus::zvariant::{OwnedValue, Value};

use crate::daemon::D;
use crate::proto::{Ctl, Note};

static DAEMON: OnceLock<D> = OnceLock::new();
/// Уведомления-запросы спаривания: устройство → id уведомления.
static PROMPTS: std::sync::Mutex<Option<HashMap<String, u32>>> = std::sync::Mutex::new(None);

const HINT: &str = "x-synlink-device";

pub fn start(d: D, mut incoming: mpsc::UnboundedReceiver<(String, Note)>) {
    let _ = DAEMON.set(d.clone());
    {
        let d2 = d.clone();
        let rt = tokio::runtime::Handle::current();
        std::thread::Builder::new()
            .name("synlink-notify-mon".into())
            .spawn(move || loop {
                if let Err(e) = monitor(&d2, &rt) {
                    tracing::warn!("мониторинг уведомлений: {e:#}");
                }
                std::thread::sleep(std::time::Duration::from_secs(10));
            })
            .ok();
    }
    // Пришедшие с устройств — показать.
    tokio::spawn(async move {
        while let Some((id, n)) = incoming.recv().await {
            let name = d.trusted(&id).map(|t| t.name).unwrap_or_else(|| id.clone());
            d.push_note(RemoteNotification {
                device: id.clone(),
                app: n.app.clone(),
                summary: n.summary.clone(),
                body: n.body.clone(),
                icon: n.icon.clone(),
                urgency: n.urgency,
                time: n.time,
            });
            if !d.notifications.load(std::sync::atomic::Ordering::Relaxed) {
                continue;
            }
            let _ = tokio::task::spawn_blocking(move || show(&id, &name, &n)).await;
        }
    });
}

fn monitor(d: &D, _rt: &tokio::runtime::Handle) -> anyhow::Result<()> {
    let conn = zbus::blocking::Connection::session()?;
    let rules = vec!["type='method_call',interface='org.freedesktop.Notifications',member='Notify'"];
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus.Monitoring"),
        "BecomeMonitor",
        &(rules, 0u32),
    )?;
    tracing::info!("слежу за уведомлениями сеанса");
    for msg in zbus::blocking::MessageIterator::from(&conn) {
        let msg = msg?;
        let h = msg.header();
        if h.member().map(|m| m.as_str()) != Some("Notify") {
            continue;
        }
        type Args = (String, u32, String, String, String, Vec<String>, HashMap<String, OwnedValue>, i32);
        let Ok((app, _replaces, icon, summary, body, _actions, hints, _timeout)) = msg.body().deserialize::<Args>() else {
            continue;
        };
        if hints.contains_key(HINT) || app == "synlink" || !d.notifications.load(std::sync::atomic::Ordering::Relaxed) {
            continue;
        }
        // Служебные мгновенные (громкость, яркость) не пересылаем.
        if hints.get("transient").and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
            || hints.contains_key("x-canonical-private-synchronous")
        {
            continue;
        }
        let urgency = hints.get("urgency").and_then(|v| u8::try_from(v).ok()).unwrap_or(1);
        let note = Note { app, summary, body, icon, urgency, time: crate::identity::now() };
        d.broadcast_ctl(Ctl::Notification(note));
    }
    Ok(())
}

fn device_icon(id: &str) -> &'static str {
    match DAEMON.get().and_then(|d| d.trusted(id)).map(|t| t.kind) {
        Some(k) if k.is_touch() => "phone",
        _ => "computer",
    }
}

fn show(id: &str, device: &str, n: &Note) {
    let res = (|| -> anyhow::Result<()> {
        let conn = zbus::blocking::Connection::session()?;
        let mut hints: HashMap<&str, Value> = HashMap::new();
        hints.insert(HINT, Value::from(id));
        hints.insert("urgency", Value::U8(n.urgency));
        let app = if n.app.is_empty() { device.to_string() } else { format!("{} · {device}", n.app) };
        let icon = if n.icon.is_empty() || n.icon.starts_with('/') { device_icon(id).to_string() } else { n.icon.clone() };
        conn.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(app.as_str(), 0u32, icon.as_str(), n.summary.as_str(), n.body.as_str(), Vec::<&str>::new(), hints, -1i32),
        )?;
        Ok(())
    })();
    if let Err(e) = res {
        tracing::debug!("показ уведомления: {e:#}");
    }
}

fn post(summary: &str, body: &str, icon: &str, actions: &[&str], conn: Option<&zbus::blocking::Connection>) -> anyhow::Result<u32> {
    let own;
    let conn = match conn {
        Some(c) => c,
        None => {
            own = zbus::blocking::Connection::session()?;
            &own
        }
    };
    let mut hints: HashMap<&str, Value> = HashMap::new();
    hints.insert(HINT, Value::from("self"));
    let reply = conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &("synlink", 0u32, icon, summary, body, actions.to_vec(), hints, if actions.is_empty() { -1i32 } else { 0 }),
    )?;
    Ok(reply.body().deserialize::<u32>()?)
}

/// Запрос спаривания, если оболочка своего диалога не показывает (нет
/// подписчиков): уведомление с кнопками «Принять» / «Отклонить».
pub fn pair_prompt(p: &PairPrompt) {
    let Some(d) = DAEMON.get().cloned() else { return };
    if d.events.receiver_count() > 0 {
        return;
    }
    let p = p.clone();
    std::thread::spawn(move || {
        let res = (|| -> anyhow::Result<()> {
            let conn = zbus::blocking::Connection::session()?;
            let body = match &p.code {
                Some(c) => format!("{} «{}» по {} хочет соединиться. Код: {c}", p.kind.title(), p.name, p.transport.title()),
                None => format!("{} «{}» подключён по {}. Разрешить доступ?", p.kind.title(), p.name, p.transport.title()),
            };
            let nid = post("Спаривание устройств", &body, "phone", &["accept", "Принять", "reject", "Отклонить"], Some(&conn))?;
            PROMPTS.lock().unwrap().get_or_insert_with(HashMap::new).insert(p.id.clone(), nid);
            let rule = zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .interface("org.freedesktop.Notifications")?
                .build();
            let it = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, None)?;
            for msg in it {
                let msg = msg?;
                let member = msg.header().member().map(|m| m.to_string());
                match member.as_deref() {
                    Some("ActionInvoked") => {
                        if let Ok((id, key)) = msg.body().deserialize::<(u32, String)>() {
                            if id == nid {
                                let _ = d.pair_reply(&p.id, key == "accept");
                                break;
                            }
                        }
                    }
                    Some("NotificationClosed") => {
                        if let Ok((id, _)) = msg.body().deserialize::<(u32, u32)>() {
                            if id == nid {
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(())
        })();
        if let Err(e) = res {
            tracing::debug!("уведомление спаривания: {e:#}");
        }
    });
}

/// Запрос спаривания решён (здесь, в оболочке или той стороной) —
/// убрать его уведомление.
pub fn close_prompt(id: &str) {
    let Some(nid) = PROMPTS.lock().unwrap().as_mut().and_then(|m| m.remove(id)) else { return };
    std::thread::spawn(move || {
        if let Ok(conn) = zbus::blocking::Connection::session() {
            let _ = conn.call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "CloseNotification",
                &(nid,),
            );
        }
    });
}
