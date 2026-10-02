//! Выбор папки через системный портал `org.freedesktop.portal.FileChooser` (в synshell — окно
//! Проводника): `OpenFile` с `directory`, ответ — сигнал `Response` объекта запроса.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use zbus::zvariant::{OwnedValue, Value};

/// Блокирующий: открыть окно выбора папки, `None` — отменили или портала нет (ошибка — текстом).
pub fn choose_folder(title: &str, start: &Path) -> Result<Option<PathBuf>, String> {
    let conn = zbus::blocking::Connection::session().map_err(|e| e.to_string())?;
    let token = format!("syncamera{}", std::process::id() as u64 * 1000 + rand_part());
    let sender = conn.unique_name().map(|n| n.trim_start_matches(':').replace('.', "_")).unwrap_or_default();
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    // подписка — до вызова: ответ может прийти раньше, чем вернётся путь запроса
    let req = zbus::blocking::Proxy::new(&conn, "org.freedesktop.portal.Desktop", path.as_str(), "org.freedesktop.portal.Request")
        .map_err(|e| e.to_string())?;
    let mut responses = req.receive_signal("Response").map_err(|e| e.to_string())?;
    let mut folder = start.to_string_lossy().as_bytes().to_vec();
    folder.push(0);
    let mut opts: HashMap<&str, Value> = HashMap::new();
    opts.insert("handle_token", Value::from(token.as_str()));
    opts.insert("directory", Value::from(true));
    opts.insert("accept_label", Value::from("Выбрать"));
    opts.insert("current_folder", Value::from(folder));
    let fc = zbus::blocking::Proxy::new(&conn, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.FileChooser")
        .map_err(|e| e.to_string())?;
    let _: zbus::zvariant::OwnedObjectPath = fc.call("OpenFile", &("", title, opts)).map_err(|e| format!("портал выбора файлов: {e}"))?;
    let Some(msg) = responses.next() else { return Ok(None) };
    let (code, results): (u32, HashMap<String, OwnedValue>) = msg.body().deserialize().map_err(|e| e.to_string())?;
    if code != 0 {
        return Ok(None);
    }
    let uris: Vec<String> = results.get("uris").and_then(|v| v.try_clone().ok()).and_then(|v| Vec::<String>::try_from(v).ok()).unwrap_or_default();
    Ok(uris.first().and_then(|u| uri_path(u)))
}

fn rand_part() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() as u64 % 1000).unwrap_or(0)
}

/// `file:///…%D0%98…` → путь.
fn uri_path(u: &str) -> Option<PathBuf> {
    let s = u.strip_prefix("file://")?;
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).ok()?, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    Some(PathBuf::from(String::from_utf8_lossy(&out).into_owned()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn uri_decodes() {
        assert_eq!(
            super::uri_path("file:///home/u/%D0%92%D0%B8%D0%B4%D0%B5%D0%BE/a%20b").unwrap(),
            std::path::PathBuf::from("/home/u/Видео/a b")
        );
    }
}
