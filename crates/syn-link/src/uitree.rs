//! Элементы интерфейса окна для агента: найти кнопку или поле по тексту и
//! нажать — без поиска по снимку. Источник — осмотр syngui
//! (`$XDG_RUNTIME_DIR/syngui-inspect/<pid>.sock`, см. syngui `a11y::inspect`);
//! у других программ — AT-SPI (`crate::atspi`).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use synshell_tr::t;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Node {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub depth: u32,
    #[serde(default)]
    pub role: String,
    /// [x, y, w, h] в логических пикселях окна (от левого верхнего угла поверхности).
    #[serde(default)]
    pub bounds: [f64; 4],
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub shortcut: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Tree {
    /// Откуда: syngui, atspi.
    pub source: String,
    pub nodes: Vec<Node>,
}

impl Node {
    pub fn text(&self) -> String {
        [&self.label, &self.value, &self.placeholder, &self.description].iter().filter_map(|s| s.as_deref()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
    }

    pub fn center(&self) -> (f64, f64) {
        (self.bounds[0] + self.bounds[2] / 2.0, self.bounds[1] + self.bounds[3] / 2.0)
    }

    pub fn line(&self) -> String {
        let b = self.bounds;
        let mut s = format!("{}#{} {} «{}» [{:.0} {:.0} {:.0}×{:.0}]", "  ".repeat(self.depth.min(12) as usize), self.id, self.role, self.text(), b[0], b[1], b[2], b[3]);
        if let Some(sc) = self.shortcut.as_deref().filter(|s| !s.is_empty()) {
            s.push_str(&format!(" ⌨{sc}"));
        }
        if let Some(st) = self.state.as_deref().filter(|s| !s.is_empty()) {
            s.push_str(&format!(" ({st})"));
        }
        s
    }
}

pub fn socket_path(pid: i32) -> std::path::PathBuf {
    synshell_common::paths::runtime_dir().join("syngui-inspect").join(format!("{pid}.sock"))
}

/// Запрос к осмотру syngui процесса `pid` на этой машине: строка JSON.
pub fn syngui_request(pid: i32, req: &str) -> Result<Value> {
    let path = socket_path(pid);
    let s = UnixStream::connect(&path).with_context(|| t!("у процесса {pid} нет осмотра syngui ({p})", pid = pid, p = path.display()))?;
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut w = s.try_clone()?;
    w.write_all(format!("{req}\n").as_bytes())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line)?;
    let v: Value = serde_json::from_str(&line).context(t!("ответ осмотра"))?;
    if v.get("ok").and_then(Value::as_bool) != Some(true) {
        bail!("{}", v.get("error").and_then(Value::as_str).unwrap_or("?"));
    }
    Ok(v)
}

/// Окно X11 для помощника Wine: id окна, его [x, y, w, h] глазами X-сервера, масштаб Xwayland.
#[derive(Debug, Clone, Copy)]
pub struct X11Ctx {
    pub id: u32,
    pub geo: [i32; 4],
    pub scale: f64,
}

/// Дерево окна процесса на этой машине: программа Windows под Wine — помощник
/// syn-wine-inspect.exe, иначе syngui, иначе AT-SPI.
pub fn local_tree(pid: i32, x11: Option<X11Ctx>) -> Result<Tree> {
    if let Some(x) = x11.filter(|_| is_wine(pid)) {
        return wine_tree(pid, x);
    }
    match syngui_request(pid, "tree") {
        Ok(v) => {
            // Окно процесса — самое большое из известных (диалоги — отдельные окна, позже).
            let wins = v.get("windows").and_then(Value::as_array).cloned().unwrap_or_default();
            let best = wins
                .iter()
                .max_by_key(|w| w.get("nodes").and_then(Value::as_array).map(|n| n.len()).unwrap_or(0))
                .cloned()
                .unwrap_or(Value::Null);
            let nodes: Vec<Node> = serde_json::from_value(best.get("nodes").cloned().unwrap_or(Value::Array(vec![]))).unwrap_or_default();
            Ok(Tree { source: "syngui".into(), nodes })
        }
        Err(e) if socket_path(pid).exists() => Err(e),
        Err(e) => crate::atspi::tree(pid).map_err(|e2| anyhow::anyhow!("{e:#}; AT-SPI: {e2:#}")),
    }
}

/// Подходит ли узел: часть текста (без учёта регистра), `#id`, роль.
pub fn matches(n: &Node, q: &str, role: Option<&str>) -> bool {
    if let Some(r) = role.filter(|r| !r.is_empty()) {
        if !n.role.eq_ignore_ascii_case(r) {
            return false;
        }
    }
    let q = q.trim();
    if let Some(id) = q.strip_prefix('#') {
        return id.parse::<u64>().ok() == Some(n.id);
    }
    q.is_empty() || n.text().to_lowercase().contains(&q.to_lowercase())
}

/// Лучший узел: точное совпадение подписи важнее части, видимый (ненулевой) — обязателен.
pub fn best<'a>(t: &'a Tree, q: &str, role: Option<&str>, index: usize) -> Option<&'a Node> {
    let ql = q.trim().to_lowercase();
    let mut v: Vec<&Node> = t.nodes.iter().filter(|n| n.bounds[2] > 0.0 && n.bounds[3] > 0.0 && matches(n, q, role)).collect();
    v.sort_by_key(|n| {
        let exact = n.label.as_deref().map(|l| l.trim().to_lowercase() == ql).unwrap_or(false);
        let clickable = matches!(n.role.as_str(), "button" | "checkbox" | "radiobutton" | "link" | "textfield" | "combobox" | "push button" | "menu item" | "check box" | "toggle button" | "text" | "entry");
        (!exact, !clickable)
    });
    v.into_iter().nth(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_nodes() {
        let t = Tree {
            source: "syngui".into(),
            nodes: vec![
                Node { id: 1, role: "statictext".into(), label: Some("Создать папку".into()), bounds: [0.0, 0.0, 10.0, 10.0], ..Default::default() },
                Node { id: 2, role: "button".into(), label: Some("Создать".into()), bounds: [40.0, 100.0, 60.0, 20.0], ..Default::default() },
                Node { id: 3, role: "button".into(), label: Some("Скрытая".into()), bounds: [0.0, 0.0, 0.0, 0.0], ..Default::default() },
            ],
        };
        assert_eq!(best(&t, "создать", None, 0).unwrap().id, 2);
        assert_eq!(best(&t, "создать", None, 1).unwrap().id, 1);
        assert_eq!(best(&t, "#1", None, 0).unwrap().id, 1);
        assert!(best(&t, "Скрытая", None, 0).is_none());
        assert_eq!(best(&t, "", Some("button"), 0).unwrap().id, 2);
        assert_eq!(t.nodes[1].center(), (70.0, 110.0));
    }
}

fn is_wine(pid: i32) -> bool {
    std::fs::read_link(format!("/proc/{pid}/exe")).map(|p| p.to_string_lossy().contains("wine")).unwrap_or(false)
}

/// Помощник Windows: установленный, переопределённый или собранный рядом (разработка).
fn wine_helper() -> Option<std::path::PathBuf> {
    if let Some(p) = std::env::var_os("SYNLINK_WINE_INSPECT") {
        return Some(p.into());
    }
    let sys = std::path::PathBuf::from("/usr/lib/synshell/syn-wine-inspect.exe");
    if sys.exists() {
        return Some(sys);
    }
    let exe = std::env::current_exe().ok()?;
    let dev = exe.parent()?.parent()?.join("x86_64-pc-windows-gnu").join(exe.parent()?.file_name()?).join("syn-wine-inspect.exe");
    dev.exists().then_some(dev)
}

/// Элементы окна программы под Wine: тот же Wine и префикс, что у неё.
fn wine_tree(pid: i32, x: X11Ctx) -> Result<Tree> {
    let helper = wine_helper().context(t!("нет syn-wine-inspect.exe (пакет synshell-link)"))?;
    let exe = std::fs::read_link(format!("/proc/{pid}/exe"))?;
    let wine = exe.parent().map(|d| d.join("wine")).filter(|p| p.exists()).unwrap_or_else(|| "wine".into());
    let environ = std::fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
    let mut cmd = std::process::Command::new(&wine);
    cmd.arg(&helper).arg(x.id.to_string()).env("WINEDEBUG", "-all").env_remove("_");
    for kv in environ.split(|b| *b == 0) {
        let kv = String::from_utf8_lossy(kv);
        if let Some((k, v)) = kv.split_once('=') {
            if matches!(k, "WINEPREFIX" | "WINEARCH" | "WINESERVER" | "WINEDLLPATH" | "WINELOADER") {
                cmd.env(k, v);
            }
        }
    }
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().with_context(|| wine.display().to_string())?;
    let mut out = child.stdout.take().context("stdout")?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut out, &mut s);
        s
    });
    let t0 = std::time::Instant::now();
    while child.try_wait()?.is_none() {
        if t0.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            bail!("{}", t!("помощник Wine не ответил за 30 с"));
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let text = reader.join().unwrap_or_default();
    let v: Value = serde_json::from_str(text.lines().rev().find(|l| l.starts_with('{')).unwrap_or("{}")).context(t!("ответ помощника Wine"))?;
    if v.get("ok").and_then(Value::as_bool) != Some(true) {
        bail!("{}", v.get("error").and_then(Value::as_str).unwrap_or("?"));
    }
    let sc = if x.scale > 0.0 { x.scale } else { 1.0 };
    let mut nodes = Vec::new();
    for n in v.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default() {
        let s: Vec<f64> = n.get("screen").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
        if s.len() != 4 || s[2] <= 0.0 || s[3] <= 0.0 {
            continue;
        }
        let mut node: Node = serde_json::from_value(n.clone()).unwrap_or_default();
        // Экран Wine = экран X: в логические пиксели окна.
        node.bounds = [(s[0] - x.geo[0] as f64) / sc, (s[1] - x.geo[1] as f64) / sc, s[2] / sc, s[3] / sc];
        if node.label.is_none() {
            node.label = n.get("class").and_then(Value::as_str).filter(|c| !c.is_empty()).map(|c| format!("[{c}]"));
        }
        nodes.push(node);
    }
    Ok(Tree { source: "wine".into(), nodes })
}
