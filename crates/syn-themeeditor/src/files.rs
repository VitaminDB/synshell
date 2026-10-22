//! Окно выбора файла Проводника (`synfiles --choose`, как портал FileChooser).

/// Открыть (`save == false`) или сохранить файл: путь или `None` при отмене.
/// Ждёт ответа пользователя — вызывать из фонового потока.
pub fn choose(title: &str, filter_name: &str, patterns: &[&str], save: Option<&str>, folder_kind: &str) -> Option<String> {
    let filters: Vec<serde_json::Value> = if patterns.is_empty() {
        Vec::new()
    } else {
        vec![serde_json::json!({ "name": filter_name, "patterns": patterns.iter().map(|p| serde_json::json!([0, p])).collect::<Vec<_>>() })]
    };
    let folder = synshell_common::paths::user_dir_or_default(folder_kind);
    let mut req = serde_json::json!({
        "mode": if save.is_some() { "save" } else { "open" },
        "title": title,
        "filters": filters,
        "current_folder": folder.to_string_lossy(),
    });
    if let Some(name) = save {
        req["current_name"] = serde_json::json!(name);
    }
    let exe = synshell_common::paths::self_exe("synfiles").with_file_name("synfiles");
    let exe = if exe.exists() { exe } else { std::path::PathBuf::from("synfiles") };
    let out = std::process::Command::new(exe).arg("--choose").arg(req.to_string()).output().ok()?;
    let line = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = line.lines().rev().find_map(|l| serde_json::from_str(l).ok())?;
    if v.get("response")?.as_u64()? != 0 {
        return None;
    }
    let uri = v.get("uris")?.as_array()?.first()?.as_str()?.to_string();
    Some(uri_to_path(&uri))
}

fn uri_to_path(uri: &str) -> String {
    let Some(p) = uri.strip_prefix("file://") else { return uri.to_string() };
    let b = p.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&p[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
