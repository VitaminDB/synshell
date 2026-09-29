//! Места боковой панели: домашние папки, закреплённые, корзина, диски.

use std::path::{Path, PathBuf};

use synshell_common::paths;

use crate::loc::Location;

#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub title: String,
    pub icon: &'static str,
    pub loc: Location,
    /// Закреплена пользователем (можно открепить).
    pub pinned: bool,
    /// Для дисков: (свободно, всего) байт.
    pub space: Option<(u64, u64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: &'static str,
    pub places: Vec<Place>,
}

fn place(title: impl Into<String>, icon: &'static str, loc: Location) -> Place {
    Place { title: title.into(), icon, loc, pinned: false, space: None }
}

/// Папки пользователя: (ключ XDG, подпись, глиф Material).
pub const USER_DIRS: [(&str, &str, &str); 6] = [
    ("DESKTOP", "Рабочий стол", crate::ui::icons::DESKTOP),
    ("DOWNLOAD", "Загрузки", crate::ui::icons::DOWNLOAD),
    ("DOCUMENTS", "Документы", crate::ui::icons::DOCUMENT),
    ("PICTURES", "Изображения", crate::ui::icons::IMAGE),
    ("MUSIC", "Музыка", crate::ui::icons::MUSIC),
    ("VIDEOS", "Видео", crate::ui::icons::VIDEO),
];

/// Глиф и подпись для известной папки (для вкладок и заголовков).
pub fn known(path: &Path) -> Option<(&'static str, String)> {
    if path == paths::home() {
        return Some((crate::ui::icons::HOME, "Домашняя папка".into()));
    }
    if path == Path::new("/") {
        return Some((crate::ui::icons::DRIVE, "Корень системы".into()));
    }
    for (k, title, icon) in USER_DIRS {
        if paths::user_dir(k).as_deref() == Some(path) {
            return Some((icon, title.into()));
        }
    }
    None
}

pub fn sections(pinned: &[String]) -> Vec<Section> {
    let mut quick = vec![place("Домашняя папка", crate::ui::icons::HOME, Location::Dir(paths::home()))];
    for (k, title, icon) in USER_DIRS {
        if let Some(d) = paths::user_dir(k).filter(|d| d.is_dir()) {
            quick.push(place(title, icon, Location::Dir(d)));
        }
    }
    for p in pinned {
        let path = paths::expand_tilde(p);
        let title = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.clone());
        let mut pl = place(title, crate::ui::icons::FOLDER_PINNED, Location::Dir(path));
        pl.pinned = true;
        quick.push(pl);
    }
    quick.push(place("Корзина", crate::ui::icons::TRASH, Location::Trash));

    let mut drives = vec![{
        let mut p = place("Корень системы", crate::ui::icons::DRIVE, Location::Dir(PathBuf::from("/")));
        p.space = space(Path::new("/"));
        p
    }];
    for m in mounts() {
        let mut p = place(m.title, m.icon, Location::Dir(m.path.clone()));
        p.space = space(&m.path);
        drives.push(p);
    }
    vec![Section { title: "Быстрый доступ", places: quick }, Section { title: "Устройства", places: drives }]
}

struct Mount {
    path: PathBuf,
    title: String,
    icon: &'static str,
}

/// Съёмные и дополнительные разделы (`/run/media`, `/media`, `/mnt`), сеть gvfs.
fn mounts() -> Vec<Mount> {
    let mut out = Vec::new();
    let Ok(t) = std::fs::read_to_string("/proc/self/mounts") else { return out };
    let uid = unsafe { libc::getuid() };
    let gvfs = PathBuf::from(format!("/run/user/{uid}/gvfs"));
    for line in t.lines() {
        let mut it = line.split_whitespace();
        let (Some(dev), Some(mp), Some(fs)) = (it.next(), it.next(), it.next()) else { continue };
        let mp = PathBuf::from(mp.replace("\\040", " "));
        let user_visible = mp.starts_with("/run/media") || mp.starts_with("/media") || mp.starts_with("/mnt");
        let network = matches!(fs, "nfs" | "nfs4" | "cifs" | "smb3" | "sshfs" | "fuse.sshfs" | "fuse.rclone");
        if !(dev.starts_with("/dev/") && user_visible) && !(network && mp != Path::new("/")) {
            continue;
        }
        if out.iter().any(|m: &Mount| m.path == mp) {
            continue;
        }
        let title = mp.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| mp.display().to_string());
        let icon = if network {
            crate::ui::icons::NETWORK
        } else if mp.starts_with("/run/media") || mp.starts_with("/media") {
            crate::ui::icons::USB
        } else {
            crate::ui::icons::DRIVE
        };
        out.push(Mount { path: mp, title, icon });
    }
    // Сетевые ресурсы, подключённые через gvfs (Nautilus, gio mount).
    if let Ok(rd) = std::fs::read_dir(&gvfs) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            out.push(Mount { path: e.path(), title: gvfs_title(&name), icon: crate::ui::icons::NETWORK });
        }
    }
    out
}

/// `sftp:host=example.org,user=me` → `me@example.org (SFTP)`.
fn gvfs_title(name: &str) -> String {
    let Some((scheme, rest)) = name.split_once(':') else { return name.to_string() };
    let mut host = "";
    let mut user = "";
    let mut share = "";
    for kv in rest.split(',') {
        match kv.split_once('=') {
            Some(("host" | "server", v)) => host = v,
            Some(("user", v)) => user = v,
            Some(("share", v)) => share = v,
            _ => {}
        }
    }
    let who = if user.is_empty() { host.to_string() } else { format!("{user}@{host}") };
    let what = if share.is_empty() { who } else { format!("{share} на {who}") };
    format!("{what} ({})", scheme.to_uppercase())
}

/// Свободно и всего на разделе.
pub fn space(p: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let bs = s.f_frsize as u64;
    Some((s.f_bavail as u64 * bs, s.f_blocks as u64 * bs))
}

#[cfg(test)]
mod tests {
    #[test]
    fn gvfs_names() {
        assert_eq!(super::gvfs_title("sftp:host=example.org,user=me"), "me@example.org (SFTP)");
        assert_eq!(super::gvfs_title("smb-share:server=nas,share=media"), "media на nas (SMB-SHARE)");
    }
}
