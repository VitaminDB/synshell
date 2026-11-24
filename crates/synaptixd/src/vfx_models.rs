//! Модели видеофильтров (увеличение, RIFE) — бандлы `.syn` в папках `vfx`
//! библиотек службы и `~/.local/share/synaptix/vfx`. Общий поиск для программ
//! (видеоплеер, видеоредактор).

use std::path::PathBuf;

/// Папки `vfx`: из библиотек службы (если она запущена) и пользовательская.
pub fn dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(mut c) = crate::Client::connect() {
        if let Ok(libs) = c.libraries() {
            out.extend(libs.into_iter().map(|l| PathBuf::from(l.path).join("vfx")));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        out.push(PathBuf::from(home).join(".local/share/synaptix/vfx"));
    }
    out.dedup();
    out
}

/// Бандлы видеофильтров: (id, путь, arch), без повторов id.
pub fn bundles() -> Vec<(String, PathBuf, String)> {
    let mut out: Vec<(String, PathBuf, String)> = Vec::new();
    for d in dirs() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "syn") {
                continue;
            }
            let Ok(b) = synaptix_bundle::Bundle::open(&p) else { continue };
            let (id, arch) = (b.id().to_string(), b.meta().arch.clone());
            if !out.iter().any(|m| m.0 == id) {
                out.push((id, p, arch));
            }
        }
    }
    out
}
