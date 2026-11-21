//! Уведомления системы → Курьер: слушаем шину сеанса D-Bus в режиме монитора
//! (вызовы `org.freedesktop.Notifications.Notify` любых программ) и
//! пересылаем по режиму из настроек. «Нет за компьютером» — по композитору
//! synwm: экран заблокирован или долго не было ввода.

use std::collections::HashMap;
use std::sync::Arc;

use crate::hub::Hub;

/// Пользователь отошёл: экран заблокирован или `minutes` без ввода.
pub fn away(minutes: u32) -> bool {
    use synshell_common::ipc::{Client, Request, Response};
    // Служба systemd не знает WAYLAND_DISPLAY — сокет композитора ищем сами.
    let path = std::env::var_os("WAYLAND_DISPLAY").map(|_| synshell_common::paths::socket_path()).filter(|p| p.exists()).or_else(compositor_socket);
    let Some(path) = path else { return true };
    match Client::connect_to(&path).and_then(|mut c| c.request(&Request::Idle)) {
        Ok(Response::Idle { idle }) => idle.locked || idle.seconds >= minutes as u64 * 60,
        // Нет композитора (сеанс не графический) — считаем, что пользователя нет.
        _ => true,
    }
}

/// Сокет synwm в каталоге выполнения (`synwm.wayland-N.sock`).
fn compositor_socket() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from)?;
    let mut v: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("synwm.") && n.ends_with(".sock")))
        .collect();
    v.sort();
    v.into_iter().next()
}

/// Тело `Notify`: (программа, replaces_id, значок, заголовок, текст, действия, подсказки, таймаут).
type NotifyBody = (String, u32, String, String, String, Vec<String>, HashMap<String, zbus::zvariant::OwnedValue>, i32);

pub fn start(hub: Arc<Hub>) {
    let rt = tokio::runtime::Handle::current();
    let _ = std::thread::Builder::new().name("dbus-notify".into()).spawn(move || loop {
        if let Err(e) = listen(&hub, &rt) {
            tracing::warn!("уведомления D-Bus: {e:#}");
        }
        std::thread::sleep(std::time::Duration::from_secs(30));
    });
}

fn listen(hub: &Arc<Hub>, rt: &tokio::runtime::Handle) -> anyhow::Result<()> {
    let conn = zbus::blocking::Connection::session()?;
    let rule = "type='method_call',interface='org.freedesktop.Notifications',member='Notify'";
    conn.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus.Monitoring"), "BecomeMonitor", &(vec![rule], 0u32))?;
    tracing::info!("уведомления системы: слушаю D-Bus");
    for msg in zbus::blocking::MessageIterator::from(&conn) {
        let msg = msg?;
        let h = msg.header();
        if h.member().map(|m| m.as_str()) != Some("Notify") {
            continue;
        }
        let Ok((app, _, _, summary, body, _, _, _)) = msg.body().deserialize::<NotifyBody>() else { continue };
        // Свои же пересылки и пустые — мимо.
        if app.eq_ignore_ascii_case("syncourier") || summary.trim().is_empty() && body.trim().is_empty() {
            continue;
        }
        let hub = hub.clone();
        rt.spawn(async move {
            hub.notify(&summary, &strip_markup(&body), "", &app).await;
        });
    }
    Ok(())
}

/// Разметка тела уведомления (`<b>`, `&amp;`) → текст.
pub fn strip_markup(s: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in s.chars() {
        match c {
            '<' => tag = true,
            '>' if tag => tag = false,
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'")
}

#[cfg(test)]
mod tests {
    #[test]
    fn markup() {
        assert_eq!(super::strip_markup("<b>Привет</b> &amp; пока"), "Привет & пока");
    }
}
