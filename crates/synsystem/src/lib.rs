//! Системные бэкенды synshell — общие для оболочек, «Параметров» и
//! экрана ресурсов: читатели sysfs/procfs без UI и сигналов. Опрос и
//! доставку в интерфейс делает вызывающий (фоновый поток →
//! `run_on_main_thread`).
//!
//! Все читатели работают от корня [`Sys`]: на живой системе это `/`, в
//! тестах — каталог с фикстурами (снятыми с телефона и десктопа).

pub mod backlight;
pub mod battery;
pub mod cpu;
pub mod gpu;
pub mod memory;
pub mod network;
pub mod procs;
pub mod thermal;
pub mod util;
pub mod volume;

use std::path::{Path, PathBuf};

/// Корень файловой системы, от которого читаются `/sys` и `/proc`.
#[derive(Debug, Clone)]
pub struct Sys {
    root: PathBuf,
}

impl Default for Sys {
    fn default() -> Self {
        Self::host()
    }
}

impl Sys {
    /// Живая система.
    pub fn host() -> Self {
        Self { root: PathBuf::from("/") }
    }

    /// Фикстуры: `root/sys/...`, `root/proc/...`.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Абсолютный путь внутри корня (`"/sys/class"` → `root/sys/class`).
    pub fn path(&self, p: impl AsRef<Path>) -> PathBuf {
        let p = p.as_ref();
        self.root.join(p.strip_prefix("/").unwrap_or(p))
    }

    pub fn read(&self, p: impl AsRef<Path>) -> Option<String> {
        std::fs::read_to_string(self.path(p)).ok().map(|s| s.trim().to_string())
    }

    pub fn read_num<T: std::str::FromStr>(&self, p: impl AsRef<Path>) -> Option<T> {
        self.read(p)?.parse().ok()
    }

    /// Имена записей каталога, по порядку.
    pub fn list(&self, p: impl AsRef<Path>) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.path(p))
            .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        v.sort_by(|a, b| util::natural_cmp(a, b));
        v
    }
}
