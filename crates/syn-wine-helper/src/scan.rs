//! Проверка программы без запуска: таблицы импорта exe и его DLL (рекурсивно) против того, что
//! есть у Wine, в общем каталоге и рядом с программой.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use syngui::t;

use crate::pe::Arch;

#[derive(Debug, Clone)]
pub struct Missing {
    pub dll: String,
    /// Кому нужна (имя файла модуля).
    pub needed_by: String,
}

pub struct Report {
    pub arch: Arch,
    pub missing: Vec<Missing>,
}

pub fn scan(exe: &Path) -> Result<Report> {
    let data = std::fs::read(exe).with_context(|| exe.display().to_string())?;
    let info = crate::pe::parse(&data).with_context(|| t!("{f}: не PE-файл", f = exe.display()))?;
    let Some(arch) = info.arch() else {
        bail!("{}", t!("{f}: машина {m:04x} — не x86 и не x64", f = exe.display(), m = info.machine));
    };
    let app_dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    let wine = crate::store::wine_dirs();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut missing = Vec::new();
    let mut queue: Vec<(String, Vec<String>)> = vec![(file_name(exe), info.imports)];
    while let Some((owner, imports)) = queue.pop() {
        for dll in imports {
            if dll.starts_with("api-ms-win-") || dll.starts_with("ext-ms-") || !seen.insert(dll.clone()) {
                continue;
            }
            let local: Option<PathBuf> = crate::store::find_in(&app_dir, &dll).or_else(|| crate::store::find_dll(arch, &dll));
            if let Some(p) = local {
                // своя DLL программы или уже поставленная — её импорты тоже нужны
                if let Some(i) = std::fs::read(&p).ok().and_then(|d| crate::pe::parse(&d)) {
                    queue.push((dll.clone(), i.imports));
                }
                continue;
            }
            if crate::store::wine_has(&wine, arch, &dll) {
                continue;
            }
            missing.push(Missing { dll, needed_by: owner.clone() });
        }
    }
    Ok(Report { arch, missing })
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}
