//! Общий каталог DLL — один на пользователя, для всех префиксов Wine:
//! `$SYN_WINE_DLLS` или `$XDG_DATA_HOME/syn-wine/dlls/<arch>-windows/`. Наш Wine (патч
//! syn-wine-helper.patch) ищет там последним и показывает модуль программе как system32\<имя>.

use std::path::{Path, PathBuf};

use crate::db::Package;
use crate::pe::Arch;

/// `$XDG_DATA_HOME/syn-wine` — свои пакеты и (по умолчанию) DLL.
pub fn base_dir() -> PathBuf {
    synshell_common::paths::data_home().join("syn-wine")
}

pub fn dll_root() -> PathBuf {
    match std::env::var_os("SYN_WINE_DLLS") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => base_dir().join("dlls"),
    }
}

pub fn arch_dir(arch: Arch) -> PathBuf {
    dll_root().join(arch.dir())
}

pub fn cache_dir() -> PathBuf {
    synshell_common::paths::cache_home().join("syn-wine-helper")
}

/// Файл DLL в общем каталоге (имя — без учёта регистра, как в Windows).
pub fn find_dll(arch: Arch, name: &str) -> Option<PathBuf> {
    find_in(&arch_dir(arch), name)
}

pub fn find_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.is_file() {
        return Some(exact);
    }
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case(name)) && p.is_file())
}

pub fn installed(pkg: &Package, arch: Arch) -> bool {
    pkg.dlls.iter().all(|d| find_dll(arch, d).is_some())
}

/// Каталоги встроенных DLL Wine: рядом с `wine` из PATH, стандартные и из `WINEDLLPATH`.
pub fn wine_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    let mut add = |p: PathBuf| {
        if p.is_dir() && !v.contains(&p) {
            v.push(p);
        }
    };
    if let Some(wine) = std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join("wine")).find(|p| p.is_file())) {
        if let Ok(real) = std::fs::canonicalize(wine) {
            if let Some(prefix) = real.parent().and_then(|b| b.parent()) {
                add(prefix.join("lib/wine"));
                add(prefix.join("lib64/wine"));
                add(prefix.join("lib32/wine"));
            }
        }
    }
    for d in ["/usr/lib/wine", "/usr/lib64/wine", "/usr/lib32/wine", "/usr/local/lib/wine"] {
        add(PathBuf::from(d));
    }
    if let Some(p) = std::env::var_os("WINEDLLPATH") {
        for d in std::env::split_paths(&p) {
            add(d);
        }
    }
    v
}

/// Есть ли у самого Wine такая DLL для архитектуры (x64 на ARM64 — ARM64EC из aarch64-windows).
pub fn wine_has(dirs: &[PathBuf], arch: Arch, name: &str) -> bool {
    let subs: &[&str] = match arch {
        Arch::X86 => &["i386-windows"],
        Arch::X64 => &["x86_64-windows", "aarch64-windows"],
    };
    let lower = name.to_ascii_lowercase();
    dirs.iter().any(|d| subs.iter().any(|s| d.join(s).join(&lower).is_file()))
}
