//! Уведомления композитора (org.freedesktop.Notifications).
//!
//! Сервер уведомлений — сама оболочка: когда композитор сообщает о её смене
//! (оболочка режима упала, новая ещё стартует), имя на шине может быть пока
//! без владельца. Поэтому отправка — в фоне, с повтором, пока оболочка не
//! поднимется (не дольше `WAIT`).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use zbus::zvariant::Value;

const WAIT: Duration = Duration::from_secs(30);

/// Показать уведомление (в фоне, не блокирует композитор).
pub fn post(summary: String, body: String, icon: &'static str) {
    tracing::info!(summary, body, "уведомление");
    let spawned = std::thread::Builder::new().name("synwm-notify".into()).spawn(move || {
        let deadline = Instant::now() + WAIT;
        loop {
            match send(&summary, &body, icon) {
                Ok(()) => return,
                Err(e) if Instant::now() >= deadline => {
                    tracing::warn!(?e, summary, "уведомление не доставлено");
                    return;
                }
                Err(_) => std::thread::sleep(Duration::from_millis(500)),
            }
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(?e, "уведомление: поток не запустился");
    }
}

fn send(summary: &str, body: &str, icon: &str) -> zbus::Result<()> {
    let conn = zbus::blocking::Connection::session()?;
    let hints: HashMap<&str, Value> = HashMap::from([("urgency", Value::U8(1))]);
    conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &("synshell", 0u32, icon, summary, body, Vec::<&str>::new(), hints, 8000i32),
    )?;
    Ok(())
}
