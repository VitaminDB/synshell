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

/// Дерево окна процесса на этой машине: syngui, иначе AT-SPI.
pub fn local_tree(pid: i32) -> Result<Tree> {
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
