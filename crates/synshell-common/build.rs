//! Встроенные темы: каждый каталог `themes/<id>/` с `theme.toml` вшивается
//! в бинарь целиком — `theme.toml`, `*.mss` и `apps/*.mss` (слои для программ).
//! Порядок в галерее — `themes/order.txt`, остальные темы — после, по имени.

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("themes");
    println!("cargo:rerun-if-changed={}", root.display());
    let order: Vec<String> = std::fs::read_to_string(root.join("order.txt"))
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect();
    let mut ids: Vec<String> = std::fs::read_dir(&root)
        .expect("каталог themes/")
        .flatten()
        .filter(|e| e.path().join("theme.toml").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort_by_key(|id| (order.iter().position(|o| o == id).unwrap_or(usize::MAX), id.clone()));

    let mut out = String::from("&[\n");
    for id in &ids {
        let dir = root.join(id);
        println!("cargo:rerun-if-changed={}", dir.display());
        let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
        let mut scan = |sub: &str| {
            let d = if sub.is_empty() { dir.clone() } else { dir.join(sub) };
            println!("cargo:rerun-if-changed={}", d.display());
            let Ok(rd) = std::fs::read_dir(&d) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "mss") {
                    let name = p.file_name().unwrap().to_string_lossy().into_owned();
                    files.push((if sub.is_empty() { name } else { format!("{sub}/{name}") }, p));
                }
            }
        };
        scan("");
        scan("apps");
        files.sort();
        let _ = write!(out, "    ({id:?}, include_str!({:?}), &[", dir.join("theme.toml").display().to_string());
        for (name, p) in files {
            let _ = write!(out, "({name:?}, include_str!({:?})), ", p.display().to_string());
        }
        out.push_str("]),\n");
    }
    out.push(']');
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("builtin_themes.rs");
    std::fs::write(dest, out).unwrap();
}
