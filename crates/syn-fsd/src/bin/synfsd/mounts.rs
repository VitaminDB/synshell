//! Один раздел может быть смонтирован в нескольких местах
//! (`/run/media/storage` и `/home/master/Storage`): ядро отдаёт путь через
//! любую из точек, а спрашивают — через свою. Запрос истории идёт по всем
//! таким псевдонимам пути.

use std::path::{Path, PathBuf};

struct Mount {
    /// `major:minor` — у одного раздела (подтома btrfs) одинаков.
    dev: String,
    /// Какая папка раздела смонтирована.
    root: PathBuf,
    target: PathBuf,
}

fn mounts() -> Vec<Mount> {
    let text = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            (f.len() > 4).then(|| Mount {
                dev: f[2].to_string(),
                root: PathBuf::from(crate::fan::unescape(f[3])),
                target: PathBuf::from(crate::fan::unescape(f[4])),
            })
        })
        .collect()
}

/// Все пути, под которыми виден `path` (сам он — первым).
pub fn aliases(path: &str) -> Vec<String> {
    let p = Path::new(path);
    let all = mounts();
    let mut out = vec![path.to_string()];
    // Точка монтирования, в которой лежит путь, — самая длинная подходящая.
    let Some(a) = all.iter().filter(|m| p.starts_with(&m.target)).max_by_key(|m| m.target.as_os_str().len()) else {
        return out;
    };
    let Ok(rel) = p.strip_prefix(&a.target) else { return out };
    let inside = a.root.join(rel);
    for b in all.iter().filter(|m| m.dev == a.dev) {
        if let Ok(r) = inside.strip_prefix(&b.root) {
            let alias = b.target.join(r);
            let s = alias.to_string_lossy().trim_end_matches('/').to_string();
            let s = if s.is_empty() { "/".to_string() } else { s };
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// Путь записи, найденной через псевдоним `alias`, — в виде, как спросили
/// (`asked`): у псевдонимов общий хвост, различаются начала (точки
/// монтирования) — начало и меняем.
pub fn translate(record: &str, alias: &str, asked: &str) -> String {
    if alias == asked {
        return record.to_string();
    }
    let a: Vec<&str> = alias.split('/').collect();
    let q: Vec<&str> = asked.split('/').collect();
    let common = a.iter().rev().zip(q.iter().rev()).take_while(|(x, y)| x == y).count();
    let from = a[..a.len() - common].join("/");
    let to = q[..q.len() - common].join("/");
    match record.strip_prefix(&from) {
        Some(rest) if !from.is_empty() && (rest.is_empty() || rest.starts_with('/')) => format!("{to}{rest}"),
        _ => record.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_mount_prefix() {
        let alias = "/run/media/storage/N/a.txt";
        let asked = "/home/m/Storage/N/a.txt";
        assert_eq!(translate("/run/media/storage/N/a.txt.123", alias, asked), "/home/m/Storage/N/a.txt.123");
        assert_eq!(translate("/other/x", alias, asked), "/other/x");
    }
}
