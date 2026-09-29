//! Сведения о системе для страниц: темы значков и курсоров, раскладки XKB,
//! автозапуск XDG, «О системе», связь с композитором и внешние диалоги.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use synshell_common::ipc::{Client, OutputInfo, Request, Response, WindowInfo};
use synshell_common::paths;

// ─── Темы ───────────────────────────────────────────────────────────────────

fn icon_dirs() -> Vec<PathBuf> {
    let mut v = vec![paths::expand_tilde("~/.icons")];
    v.extend(paths::data_dirs().into_iter().map(|d| d.join("icons")));
    v
}

/// Темы значков: каталоги с `index.theme`, кроме чисто курсорных.
/// Возвращает `(каталог, отображаемое имя)`.
pub fn icon_themes() -> Vec<(String, String)> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for dir in icon_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let id = e.file_name().to_string_lossy().to_string();
            let index = p.join("index.theme");
            if !index.exists() || seen.contains(&id) {
                continue;
            }
            let text = std::fs::read_to_string(&index).unwrap_or_default();
            // Курсорные темы тоже несут index.theme, но без Directories=.
            if !text.lines().any(|l| l.starts_with("Directories=")) {
                continue;
            }
            if id == "default" || id == "locolor" {
                continue;
            }
            let name = desktop_value(&text, "Name").unwrap_or_else(|| id.clone());
            seen.insert(id.clone());
            out.push((id, name));
        }
    }
    out.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    out
}

/// Темы курсоров: каталоги с подкаталогом `cursors`.
pub fn cursor_themes() -> Vec<(String, String)> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for dir in icon_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let id = e.file_name().to_string_lossy().to_string();
            if !p.join("cursors").is_dir() || seen.contains(&id) {
                continue;
            }
            let name = std::fs::read_to_string(p.join("index.theme"))
                .ok()
                .and_then(|t| desktop_value(&t, "Name"))
                .unwrap_or_else(|| id.clone());
            seen.insert(id.clone());
            out.push((id, name));
        }
    }
    out.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    out
}

/// Семейства шрифтов из fontconfig (без повторов).
pub fn font_families() -> Vec<String> {
    let out = std::process::Command::new("fc-list").args([":", "family"]).output();
    let Ok(out) = out else { return Vec::new() };
    let mut set = BTreeSet::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(first) = line.split(',').next() {
            let f = first.trim();
            if !f.is_empty() {
                set.insert(f.to_string());
            }
        }
    }
    set.into_iter().collect()
}

/// Значение ключа из .desktop/index.theme (первое вхождение, без локали).
pub fn desktop_value(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    text.lines()
        .map(str::trim)
        .find(|l| l.starts_with(&prefix))
        .map(|l| l[prefix.len()..].trim().to_string())
}

/// Локализованное значение (`Name[ru]`), иначе обычное.
pub fn desktop_value_localized(text: &str, key: &str) -> Option<String> {
    let lang = std::env::var("LANG").unwrap_or_default();
    let short = lang.split(['_', '.']).next().unwrap_or("");
    if !short.is_empty() {
        if let Some(v) = desktop_value(text, &format!("{key}[{short}]")) {
            return Some(v);
        }
    }
    desktop_value(text, key)
}

// ─── XKB ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct XkbLayout {
    pub name: String,
    pub description: String,
    /// (имя варианта, описание)
    pub variants: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default)]
pub struct XkbData {
    pub layouts: Vec<XkbLayout>,
    /// Опции группы `grp:` (переключение раскладок): (имя, описание).
    pub switch_options: Vec<(String, String)>,
    /// Все прочие опции: (группа, имя, описание).
    pub other_options: Vec<(String, String, String)>,
}

/// Разобрать `/usr/share/X11/xkb/rules/evdev.xml` без XML-парсера:
/// нужны только `<name>` и `<description>` внутри `configItem`.
pub fn xkb_data() -> XkbData {
    let text = std::fs::read_to_string("/usr/share/X11/xkb/rules/evdev.xml").unwrap_or_default();
    parse_xkb(&text)
}

fn tag<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let a = s.find(&open)? + open.len();
    let b = s[a..].find(&close)? + a;
    Some(&s[a..b])
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&").replace("&quot;", "\"").replace("&apos;", "'")
}

pub fn parse_xkb(text: &str) -> XkbData {
    let mut data = XkbData::default();
    if let (Some(a), Some(b)) = (text.find("<layoutList>"), text.find("</layoutList>")) {
        for chunk in text[a..b].split("<layout>").skip(1) {
            let (head, variants_part) = match chunk.find("<variantList>") {
                Some(i) => (&chunk[..i], &chunk[i..]),
                None => (chunk, ""),
            };
            let Some(name) = tag(head, "name") else { continue };
            let desc = tag(head, "description").unwrap_or(name);
            let mut l = XkbLayout { name: name.to_string(), description: unescape(desc), variants: vec![] };
            for v in variants_part.split("<variant>").skip(1) {
                if let Some(vn) = tag(v, "name") {
                    l.variants.push((vn.to_string(), unescape(tag(v, "description").unwrap_or(vn))));
                }
            }
            data.layouts.push(l);
        }
    }
    data.layouts.sort_by(|a, b| a.description.cmp(&b.description));
    if let Some(a) = text.find("<optionList>") {
        for group in text[a..].split("<group").skip(1) {
            let gname = tag(group, "name").unwrap_or("").to_string();
            let gdesc = unescape(tag(group, "description").unwrap_or(""));
            for opt in group.split("<option").skip(1) {
                let Some(n) = tag(opt, "name") else { continue };
                let d = unescape(tag(opt, "description").unwrap_or(n));
                if gname == "grp" {
                    data.switch_options.push((n.to_string(), d));
                } else {
                    data.other_options.push((gdesc.clone(), n.to_string(), d));
                }
            }
        }
    }
    data
}

// ─── Автозапуск XDG ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AutostartEntry {
    /// Имя файла (`org.kde.kdeconnect.daemon.desktop`).
    pub file: String,
    pub name: String,
    pub exec: String,
    pub enabled: bool,
    /// Есть ли пользовательская копия в `~/.config/autostart`.
    pub user: bool,
    /// Запись ограничена другими окружениями (`OnlyShowIn=KDE;`).
    pub only_in: String,
}

pub fn user_autostart_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| paths::expand_tilde("~/.config"))
        .join("autostart")
}

fn system_autostart_dirs() -> Vec<PathBuf> {
    let dirs = std::env::var("XDG_CONFIG_DIRS").unwrap_or_else(|_| "/etc/xdg".into());
    dirs.split(':').filter(|s| !s.is_empty()).map(|d| PathBuf::from(d).join("autostart")).collect()
}

pub fn autostart_entries() -> Vec<AutostartEntry> {
    let mut map: std::collections::BTreeMap<String, AutostartEntry> = Default::default();
    let mut read = |dir: &Path, user: bool| {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let file = e.file_name().to_string_lossy().to_string();
            if !file.ends_with(".desktop") {
                continue;
            }
            let text = std::fs::read_to_string(e.path()).unwrap_or_default();
            let hidden = desktop_value(&text, "Hidden").as_deref() == Some("true");
            let disabled_gnome =
                desktop_value(&text, "X-GNOME-Autostart-enabled").as_deref() == Some("false");
            map.insert(
                file.clone(),
                AutostartEntry {
                    name: desktop_value_localized(&text, "Name").unwrap_or_else(|| file.clone()),
                    exec: desktop_value(&text, "Exec").unwrap_or_default(),
                    enabled: !hidden && !disabled_gnome,
                    user,
                    only_in: desktop_value(&text, "OnlyShowIn").unwrap_or_default(),
                    file,
                },
            );
        }
    };
    for d in system_autostart_dirs().iter().rev() {
        read(d, false);
    }
    read(&user_autostart_dir(), true);
    map.into_values().collect()
}

/// Включить/выключить запись автозапуска: пользовательская копия с
/// `Hidden=true` перекрывает системную (так делает и Plasma).
pub fn set_autostart_enabled(file: &str, enabled: bool) -> std::io::Result<()> {
    let user_dir = user_autostart_dir();
    let user_path = user_dir.join(file);
    let text = if user_path.exists() {
        std::fs::read_to_string(&user_path)?
    } else {
        let src = system_autostart_dirs()
            .into_iter()
            .map(|d| d.join(file))
            .find(|p| p.exists())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, file.to_string()))?;
        std::fs::read_to_string(src)?
    };
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| !l.starts_with("Hidden=") && !l.starts_with("X-GNOME-Autostart-enabled="))
        .map(String::from)
        .collect();
    if !enabled {
        // Ключ — в секцию [Desktop Entry], сразу после заголовка.
        let pos = lines.iter().position(|l| l.trim() == "[Desktop Entry]").map(|i| i + 1).unwrap_or(0);
        lines.insert(pos, "Hidden=true".into());
    }
    std::fs::create_dir_all(&user_dir)?;
    std::fs::write(user_path, lines.join("\n") + "\n")
}

/// Удалить пользовательскую запись автозапуска.
pub fn remove_user_autostart(file: &str) -> std::io::Result<()> {
    std::fs::remove_file(user_autostart_dir().join(file))
}

// ─── О системе ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct About {
    pub os: String,
    pub kernel: String,
    pub hostname: String,
    pub cpu: String,
    pub cores: usize,
    pub memory: String,
    pub gpus: Vec<String>,
    pub session: String,
}

pub fn about() -> About {
    let osr = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let os = osr
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim_matches('"').to_string())
        .unwrap_or_else(|| "Linux".into());
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_string();
    let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default().trim().to_string();
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    // x86 — model name; ARM — Hardware или SoC из device tree.
    let cpu = synsystem::cpu::model(&synsystem::Sys::host()).unwrap_or_default();
    let cores = cpuinfo.lines().filter(|l| l.starts_with("processor")).count();
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let memory = meminfo
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<f64>().ok())
        .map(|kb| format!("{:.1} ГиБ", kb / 1024.0 / 1024.0))
        .unwrap_or_default();
    About {
        os,
        kernel,
        hostname,
        cpu,
        cores,
        memory,
        gpus: {
            let mut g = gpus();
            // Qualcomm Adreno (KGSL) не виден в lspci и drm.
            if g.is_empty() {
                g.extend(synsystem::gpu::read(&synsystem::Sys::host()).and_then(|x| x.name));
            }
            g
        },
        session: std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
    }
}

fn gpus() -> Vec<String> {
    if let Ok(out) = std::process::Command::new("lspci").arg("-mm").output() {
        let text = String::from_utf8_lossy(&out.stdout);
        let v: Vec<String> = text
            .lines()
            .filter(|l| l.contains("\"VGA") || l.contains("\"3D") || l.contains("\"Display"))
            .map(|l| {
                // Формат: slot "class" "vendor" "device" ...
                let parts: Vec<&str> = l.split('"').collect();
                let vendor = parts.get(3).copied().unwrap_or("");
                let device = parts.get(5).copied().unwrap_or("");
                format!("{vendor} {device}").trim().to_string()
            })
            .collect();
        if !v.is_empty() {
            return v;
        }
    }
    // Без lspci — драйверы DRM-устройств.
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/sys/class/drm") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("card") && !n.contains('-') {
                if let Ok(u) = std::fs::read_to_string(e.path().join("device/uevent")) {
                    if let Some(d) = u.lines().find_map(|l| l.strip_prefix("DRIVER=")) {
                        v.push(d.to_string());
                    }
                }
            }
        }
    }
    v
}

// ─── Композитор ─────────────────────────────────────────────────────────────

/// Запрос к композитору с коротким таймаутом (без композитора — `None`).
pub fn ipc(req: Request) -> Option<Response> {
    let path = paths::socket_path();
    if !path.exists() {
        return None;
    }
    // Клиент блокирующий; зависший композитор не должен вешать окно.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = Client::connect_to(&path).ok().and_then(|mut c| c.request(&req).ok());
        let _ = tx.send(r);
    });
    rx.recv_timeout(Duration::from_millis(700)).ok().flatten()
}

pub fn outputs() -> Option<Vec<OutputInfo>> {
    match ipc(Request::Outputs)? {
        Response::Outputs { outputs } => Some(outputs),
        _ => None,
    }
}

pub fn windows() -> Option<Vec<WindowInfo>> {
    match ipc(Request::Windows)? {
        Response::Windows { windows } => Some(windows),
        _ => None,
    }
}

// ─── Внешние программы ──────────────────────────────────────────────────────

fn which(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
        .unwrap_or(false)
}

/// Диалог выбора файла/каталога через kdialog или zenity (блокирующий —
/// звать из отдельного потока). `None` — отменили или диалога нет.
pub fn pick_path(directory: bool, start: &str) -> Option<String> {
    let start = if start.is_empty() { paths::expand_tilde("~").display().to_string() } else { start.to_string() };
    let out = if which("kdialog") {
        let mut c = std::process::Command::new("kdialog");
        if directory {
            c.args(["--getexistingdirectory", &start]);
        } else {
            c.args([
                "--getopenfilename",
                &start,
                "Изображения (*.png *.jpg *.jpeg *.webp *.bmp *.gif)",
            ]);
        }
        c.output().ok()?
    } else if which("zenity") {
        let mut c = std::process::Command::new("zenity");
        c.arg("--file-selection");
        if directory {
            c.arg("--directory");
        }
        c.arg(format!("--filename={start}/"));
        c.output().ok()?
    } else {
        return None;
    };
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

pub fn has_file_dialog() -> bool {
    which("kdialog") || which("zenity")
}

/// Открыть файл в редакторе пользователя.
pub fn open_in_editor(path: &Path) {
    let p = path.display().to_string();
    let _ = std::process::Command::new("xdg-open")
        .arg(&p)
        .spawn()
        .or_else(|_| std::process::Command::new("kate").arg(&p).spawn());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xkb_parse() {
        let xml = r#"<xkbConfigRegistry><layoutList>
          <layout><configItem><name>us</name><description>English (US)</description></configItem>
            <variantList><variant><configItem><name>dvorak</name><description>English (Dvorak)</description></configItem></variant></variantList>
          </layout>
          <layout><configItem><name>ru</name><description>Russian</description></configItem></layout>
        </layoutList>
        <optionList>
          <group allowMultipleSelection="true"><configItem><name>grp</name><description>Switching</description></configItem>
            <option><configItem><name>grp:alt_shift_toggle</name><description>Alt+Shift</description></configItem></option>
          </group>
          <group><configItem><name>caps</name><description>Caps Lock</description></configItem>
            <option><configItem><name>caps:escape</name><description>Caps is Esc &amp; more</description></configItem></option>
          </group>
        </optionList></xkbConfigRegistry>"#;
        let d = parse_xkb(xml);
        assert_eq!(d.layouts.len(), 2);
        assert_eq!(d.layouts[0].name, "us");
        assert_eq!(d.layouts[0].variants[0].0, "dvorak");
        assert_eq!(d.switch_options, vec![("grp:alt_shift_toggle".into(), "Alt+Shift".into())]);
        assert_eq!(d.other_options[0].2, "Caps is Esc & more");
    }

    #[test]
    fn real_xkb_if_present() {
        let d = xkb_data();
        if !d.layouts.is_empty() {
            assert!(d.layouts.iter().any(|l| l.name == "ru"));
            assert!(!d.switch_options.is_empty());
        }
    }
}
