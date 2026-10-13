//! Рецепты — графы с именем, описанием и параметрами: встроенные (каталог
//! `recipes/` крейта, вшиты в программу) и пользовательские (`*.json` в
//! каталоге рецептов). Пользовательский с тем же именем заменяет встроенный.

use std::path::Path;

use crate::Graph;

const BUILTIN: &[&str] = &[
    include_str!("../recipes/talking-avatar.json"),
    include_str!("../recipes/music-video.json"),
    include_str!("../recipes/animate-photos.json"),
    include_str!("../recipes/dub-video.json"),
    include_str!("../recipes/cover-art.json"),
    include_str!("../recipes/story-video.json"),
];

pub fn builtin() -> Vec<Graph> {
    BUILTIN.iter().filter_map(|s| serde_json::from_str(s).map_err(|e| eprintln!("встроенный рецепт: {e}")).ok()).collect()
}

/// Пользовательские рецепты каталога (ошибочные файлы пропускаются).
pub fn user(dir: &Path) -> Vec<(Graph, Result<(), String>)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let name = p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Graph>(&t).map_err(|e| e.to_string())) {
            Ok(mut g) => {
                if g.name.is_empty() {
                    g.name = name;
                }
                let check = g.order().map(|_| ());
                out.push((g, check));
            }
            Err(e) => out.push((Graph { name, ..Default::default() }, Err(e))),
        }
    }
    out
}

/// Все рецепты: пользовательские поверх встроенных, по имени.
pub fn list(dir: &Path) -> Vec<Graph> {
    let mut all = builtin();
    for (g, ok) in user(dir) {
        if ok.is_err() {
            continue;
        }
        all.retain(|b| b.name != g.name);
        all.push(g);
    }
    all.sort_by(|a, b| a.name.cmp(&b.name));
    all
}

/// Имя рецепта → имя файла (латиница, цифры, дефисы).
pub fn slug(name: &str) -> String {
    let mut s: String = name.trim().to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    s.trim_matches('-').chars().take(60).collect()
}

/// Сохранить граф своим рецептом: проверка (имя, описание, порядок, операции,
/// ссылки только на параметры и узлы), запись `<slug>.json`. Возвращает путь
/// и признак «заменён существующий».
pub fn save(dir: &Path, g: &Graph) -> Result<(std::path::PathBuf, bool), String> {
    let name = slug(&g.name);
    if name.is_empty() {
        return Err("у рецепта нужно имя (name)".into());
    }
    if g.description.trim().is_empty() {
        return Err("у рецепта нужно описание (description) — по нему его выбирают".into());
    }
    if g.nodes.is_empty() {
        return Err("в рецепте нет шагов (nodes)".into());
    }
    g.order()?;
    for n in &g.nodes {
        if !crate::exec::OPS.contains(&n.op.as_str()) {
            return Err(format!("узел «{}»: неизвестная операция «{}»", n.id, n.op));
        }
    }
    let mut g = g.clone();
    g.normalize_output()?;
    g.name = name.clone();
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!("{name}.json"));
    let existed = path.exists();
    let json = serde_json::to_string_pretty(&g).map_err(|e| e.to_string())?;
    std::fs::write(&path, json + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((path, existed))
}

pub fn find(dir: &Path, name: &str) -> Option<Graph> {
    list(dir).into_iter().find(|g| g.name.eq_ignore_ascii_case(name))
}

/// Строка параметров рецепта для промпта: `photo (file, обязательно) — …`.
pub fn params_line(g: &Graph) -> String {
    g.params
        .iter()
        .map(|(k, p)| {
            let req = if p.default.is_none() { ", обязательно" } else { "" };
            let kind = if p.kind.is_empty() { "text" } else { &p.kind };
            format!("{k} ({kind}{req}){}", if p.description.is_empty() { String::new() } else { format!(" — {}", p.description) })
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_recipes_are_valid() {
        let all = builtin();
        assert_eq!(all.len(), BUILTIN.len(), "все встроенные рецепты разбираются");
        for g in &all {
            assert!(!g.name.is_empty() && !g.description.is_empty(), "{}", g.name);
            g.order().unwrap_or_else(|e| panic!("{}: {e}", g.name));
            for n in &g.nodes {
                assert!(crate::exec::OPS.contains(&n.op.as_str()), "{}: {}", g.name, n.op);
            }
        }
    }

    #[test]
    fn save_and_find() {
        let dir = tempfile::tempdir().unwrap();
        let g: Graph = serde_json::from_value(serde_json::json!({
            "name": "Мой Постер!", "description": "постер по теме",
            "params": {"topic": {"type": "text"}},
            "nodes": [{"id": "img", "op": "image", "prompt": "poster about {{topic}}"}]
        })).unwrap();
        let (path, existed) = save(dir.path(), &g).unwrap();
        assert!(!existed && path.ends_with("мой-постер.json"));
        assert_eq!(find(dir.path(), "мой-постер").unwrap().description, "постер по теме");
        let bad: Graph = serde_json::from_value(serde_json::json!({"name": "x", "description": "", "nodes": [{"id": "a", "op": "text"}]})).unwrap();
        assert!(save(dir.path(), &bad).is_err());
    }

    #[test]
    fn user_overrides_builtin() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cover-art.json"), r#"{"name":"cover-art","description":"моя обложка","nodes":[{"id":"t","op":"text","text":"x"}]}"#).unwrap();
        let g = find(dir.path(), "cover-art").unwrap();
        assert_eq!(g.description, "моя обложка");
    }
}
