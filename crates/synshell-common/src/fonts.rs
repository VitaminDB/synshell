//! Шрифты системы: семейства из fontconfig (выбор шрифта в настройках и в виджетах стола).

use std::collections::BTreeSet;

/// Семейства шрифтов из fontconfig (`fc-list : family`), по алфавиту, без повторов.
pub fn families() -> Vec<String> {
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
