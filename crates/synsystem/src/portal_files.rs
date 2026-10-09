//! Бэкенд портала выбора файлов `org.freedesktop.impl.portal.FileChooser` (OpenFile, SaveFile,
//! SaveFiles) — на том же имени шины, что и бэкенд доступа ([`crate::portal_access`]).
//!
//! Окно выбора — Проводник в режиме выбора: `synfiles --choose '<JSON запроса>'` (раскладка
//! телефона или компьютера — по ширине окна). Он печатает ответ одной строкой JSON и закрывается;
//! программа закрыла запрос (`Request.Close`) — окно убивается, ответ «отменено».

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value as ZValue};
use synshell_tr::t;

const RESPONSE_OK: u32 = 0;
const RESPONSE_CANCELLED: u32 = 1;
const RESPONSE_OTHER: u32 = 2;

/// Программа окна выбора (`SYNSHELL_FILE_CHOOSER` — подменить).
fn chooser_cmd() -> String {
    std::env::var("SYNSHELL_FILE_CHOOSER").unwrap_or_else(|_| "synfiles".into())
}

pub struct FileChooser;

struct Request {
    pid: Arc<Mutex<Option<u32>>>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    fn close(&self) {
        if let Some(pid) = *self.pid.lock().unwrap() {
            // SAFETY: сигнал своему дочернему процессу
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }
    }
}

fn bool_opt(o: &HashMap<String, OwnedValue>, k: &str) -> bool {
    o.get(k).and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
}

fn str_opt(o: &HashMap<String, OwnedValue>, k: &str) -> String {
    o.get(k).and_then(|v| v.try_clone().ok()).and_then(|v| String::try_from(v).ok()).unwrap_or_default()
}

/// Путь в `ay` (байты с нулём на конце).
fn bytes_opt(o: &HashMap<String, OwnedValue>, k: &str) -> String {
    let Some(v) = o.get(k).and_then(|v| v.try_clone().ok()) else { return String::new() };
    let b: Vec<u8> = Vec::<u8>::try_from(v).unwrap_or_default();
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// Фильтр портала `(sa(us))` → JSON `{"name","patterns":[[kind,pat]]}`.
fn filter_json(v: &ZValue) -> Option<Value> {
    let ZValue::Structure(s) = v else { return None };
    let f = s.fields();
    let name = match f.first()? {
        ZValue::Str(s) => s.to_string(),
        _ => return None,
    };
    let ZValue::Array(arr) = f.get(1)? else { return None };
    let mut pats = Vec::new();
    for p in arr.iter() {
        if let ZValue::Structure(ps) = p {
            let pf = ps.fields();
            if let (Some(ZValue::U32(k)), Some(ZValue::Str(pat))) = (pf.first(), pf.get(1)) {
                pats.push(json!([k, pat.to_string()]));
            }
        }
    }
    Some(json!({"name": name, "patterns": pats}))
}

fn filters(o: &HashMap<String, OwnedValue>) -> (Vec<Value>, Option<usize>) {
    let mut list = Vec::new();
    if let Some(v) = o.get("filters") {
        if let ZValue::Array(arr) = &**v {
            list.extend(arr.iter().filter_map(filter_json));
        }
    }
    // текущий фильтр — по совпадению с одним из списка (или добавить в начало)
    let cur = o.get("current_filter").and_then(|v| filter_json(v)).map(|c| match list.iter().position(|f| *f == c) {
        Some(i) => i,
        None => {
            list.insert(0, c);
            0
        }
    });
    (list, cur)
}

fn files_opt(o: &HashMap<String, OwnedValue>) -> Vec<String> {
    let Some(v) = o.get("files") else { return Vec::new() };
    let ZValue::Array(arr) = &**v else { return Vec::new() };
    arr.iter()
        .filter_map(|e| match e {
            ZValue::Array(b) => {
                let bytes: Vec<u8> = b.iter().filter_map(|x| if let ZValue::U8(c) = x { Some(*c) } else { None }).collect();
                let end = bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len());
                let p = String::from_utf8_lossy(&bytes[..end]).into_owned();
                // портал даёт полные пути — нужны имена
                Some(std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(p))
            }
            _ => None,
        })
        .collect()
}

impl FileChooser {
    async fn run(
        &self,
        server: &zbus::ObjectServer,
        handle: OwnedObjectPath,
        app_id: String,
        mode: &str,
        title: String,
        o: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let (flist, fcur) = filters(&o);
        let mut folder = bytes_opt(&o, "current_folder");
        let file = bytes_opt(&o, "current_file");
        let mut name = str_opt(&o, "current_name");
        if !file.is_empty() {
            let p = std::path::Path::new(&file);
            if folder.is_empty() {
                folder = p.parent().map(|d| d.display().to_string()).unwrap_or_default();
            }
            if name.is_empty() {
                name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            }
        }
        let req = json!({
            "mode": mode,
            "title": title,
            "accept_label": str_opt(&o, "accept_label"),
            "multiple": bool_opt(&o, "multiple"),
            "directory": bool_opt(&o, "directory"),
            "filters": flist,
            "current_filter": fcur,
            "current_name": name,
            "current_folder": folder,
            "files": files_opt(&o),
        });
        let pid = Arc::new(Mutex::new(None));
        let registered = server.at(handle.as_ref(), Request { pid: pid.clone() }).await.unwrap_or(false);
        let req_s = req.to_string();
        let out = blocking::unblock(move || -> Result<String, String> {
            let child = Command::new(chooser_cmd())
                .arg("--choose")
                .arg(&req_s)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("{}: {e}", chooser_cmd()))?;
            *pid.lock().unwrap() = Some(child.id());
            let o = child.wait_with_output().map_err(|e| e.to_string())?;
            *pid.lock().unwrap() = None;
            Ok(String::from_utf8_lossy(&o.stdout).into_owned())
        })
        .await;
        if registered {
            let _ = server.remove::<Request, _>(handle.as_ref()).await;
        }
        let text = match out {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("портал: окно выбора файлов: {e}");
                return (RESPONSE_OTHER, HashMap::new());
            }
        };
        // ответ — последняя строка JSON
        let Some(v) = text.lines().rev().find_map(|l| serde_json::from_str::<Value>(l.trim()).ok()) else {
            return (RESPONSE_CANCELLED, HashMap::new());
        };
        if v["response"].as_u64() != Some(0) {
            tracing::info!(app = %app_id, mode, "портал: выбор файлов отменён");
            return (RESPONSE_CANCELLED, HashMap::new());
        }
        let uris: Vec<String> = v["uris"].as_array().map(|a| a.iter().filter_map(|u| u.as_str().map(String::from)).collect()).unwrap_or_default();
        tracing::info!(app = %app_id, mode, n = uris.len(), "портал: выбор файлов");
        let mut res = HashMap::new();
        if let Ok(u) = OwnedValue::try_from(ZValue::from(uris)) {
            res.insert("uris".to_string(), u);
        }
        // выбранный фильтр — обратно, как его прислали
        if let (Some(i), Some(f)) = (v["current_filter"].as_u64(), o.get("filters")) {
            if let ZValue::Array(arr) = &**f {
                if let Some(Ok(ov)) = arr.iter().nth(i as usize).map(|x| x.try_to_owned()) {
                    res.insert("current_filter".to_string(), ov);
                }
            }
        }
        (RESPONSE_OK, res)
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooser {
    #[allow(clippy::too_many_arguments)]
    async fn open_file(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        self.run(server, handle, app_id, "open", title, options).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn save_file(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        self.run(server, handle, app_id, "save", title, options).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn save_files(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        self.run(server, handle, app_id, "save_files", title, options).await
    }
}

// --- клиент: выбрать файл или папку через портал (любая программа synshell) ---------------------------------

/// Фильтр окна выбора: подпись и маски имени (`*.png`) или типы (`image/*` — с «/»).
pub struct PickFilter<'a> {
    pub name: &'a str,
    pub patterns: &'a [&'a str],
}

/// Блокирующий: окно выбора системного портала (`org.freedesktop.portal.FileChooser.OpenFile`), ответ —
/// сигнал `Response` запроса. `directory` — папка. `Ok(None)` — отменили; ошибка — портала нет.
pub fn pick(title: &str, directory: bool, start: &std::path::Path, filters: &[PickFilter]) -> Result<Option<std::path::PathBuf>, String> {
    use zbus::zvariant::Value as V;
    let conn = zbus::blocking::Connection::session().map_err(|e| e.to_string())?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let token = format!("synshell{}_{nanos}", std::process::id());
    let sender = conn.unique_name().map(|n| n.trim_start_matches(':').replace('.', "_")).unwrap_or_default();
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    // подписка — до вызова: ответ может прийти раньше, чем вернётся путь запроса
    let req = zbus::blocking::Proxy::new(&conn, "org.freedesktop.portal.Desktop", path.as_str(), "org.freedesktop.portal.Request")
        .map_err(|e| e.to_string())?;
    let mut responses = req.receive_signal("Response").map_err(|e| e.to_string())?;
    let mut folder = start.to_string_lossy().as_bytes().to_vec();
    folder.push(0);
    let mut opts: HashMap<&str, V> = HashMap::new();
    opts.insert("handle_token", V::from(token.as_str()));
    opts.insert("directory", V::from(directory));
    if directory {
        opts.insert("accept_label", V::from(t!("Выбрать")));
    }
    opts.insert("current_folder", V::from(folder));
    if !filters.is_empty() {
        let list: Vec<(String, Vec<(u32, String)>)> = filters
            .iter()
            .map(|f| (f.name.to_string(), f.patterns.iter().map(|p| (u32::from(p.contains('/')), p.to_string())).collect()))
            .collect();
        opts.insert("filters", V::from(list));
    }
    let fc = zbus::blocking::Proxy::new(&conn, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.FileChooser")
        .map_err(|e| e.to_string())?;
    let _: OwnedObjectPath = fc.call("OpenFile", &("", title, opts)).map_err(|e| t!("портал выбора файлов: {e}", e = e))?;
    let Some(msg) = responses.next() else { return Ok(None) };
    let (code, results): (u32, HashMap<String, OwnedValue>) = msg.body().deserialize().map_err(|e| e.to_string())?;
    if code != 0 {
        return Ok(None);
    }
    let uris: Vec<String> = results.get("uris").and_then(|v| v.try_clone().ok()).and_then(|v| Vec::<String>::try_from(v).ok()).unwrap_or_default();
    Ok(uris.first().and_then(|u| uri_path(u)))
}

/// `file:///…%D0%98…` → путь.
pub fn uri_path(u: &str) -> Option<std::path::PathBuf> {
    let b = u.strip_prefix("file://")?.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    Some(std::path::PathBuf::from(String::from_utf8_lossy(&out).into_owned()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn uri_decodes() {
        assert_eq!(super::uri_path("file:///home/u/%D0%92%D0%B8%D0%B4%D0%B5%D0%BE/a%20b").unwrap(), std::path::PathBuf::from("/home/u/Видео/a b"));
    }
}
