//! Места боковой панели: домашние папки, закреплённые, корзина, диски.

use std::path::{Path, PathBuf};

use synshell_common::paths;

use crate::loc::Location;
use crate::drives::{Device, Kind};

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Dir(Location),
    /// Устройство, которое ещё не смонтировано (флешка, раздел встроенного
    /// диска, телефон): щелчок монтирует его.
    Device(Device),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub title: String,
    pub icon: &'static str,
    pub target: Target,
    /// Закреплена пользователем (можно открепить).
    pub pinned: bool,
    /// Для дисков: (свободно, всего) байт.
    pub space: Option<(u64, u64)>,
    /// Съёмное устройство, которому принадлежит место: для «Безопасно извлечь».
    pub device: Option<Device>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: &'static str,
    pub places: Vec<Place>,
}

fn place(title: impl Into<String>, icon: &'static str, loc: Location) -> Place {
    Place { title: title.into(), icon, target: Target::Dir(loc), pinned: false, space: None, device: None }
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
    if let Some((root, title, icon)) = gadget_root(path) {
        if root == path {
            return Some((icon, title));
        }
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
    let devices = crate::drives::devices();
    let mut gadgets = Vec::new();
    for m in mounts() {
        let device = devices.iter().find(|d| d.removable() && d.mounted_at(&m.path)).cloned();
        // Телефон в gvfs — с его именем и значком, а не «mtp:host=…».
        let (title, icon) = match &device {
            Some(d @ Device::Gadget(_)) => (d.title().to_string(), kind_icon(d.kind())),
            _ => (m.title, m.icon),
        };
        if matches!(device, Some(Device::Gadget(_))) {
            gadgets.push((m.path.clone(), title.clone(), icon));
        }
        let mut p = place(title, icon, Location::Dir(m.path.clone()));
        p.space = space(&m.path);
        p.device = device;
        drives.push(p);
    }
    // Что ещё никто не смонтировал (флешки, второй диск, NTFS с Windows,
    // телефоны): щелчок монтирует.
    for d in devices.into_iter().filter(|d| d.mount_point().is_none()) {
        drives.push(Place {
            title: d.title().to_string(),
            icon: kind_icon(d.kind()),
            target: Target::Device(d.clone()),
            pinned: false,
            space: None,
            device: Some(d),
        });
    }
    *GADGETS.lock().unwrap_or_else(|e| e.into_inner()) = gadgets;
    GADGETS_ASKED.store(false, std::sync::atomic::Ordering::Relaxed);
    vec![Section { title: "Быстрый доступ", places: quick }, Section { title: "Устройства", places: drives }]
}

/// Смонтированные телефоны и камеры (папка gvfs, имя, значок) — запоминаются
/// при построении боковой панели: заголовки и крошки спрашивают часто, D-Bus
/// для них не годится.
static GADGETS: std::sync::Mutex<Vec<(PathBuf, String, &'static str)>> = std::sync::Mutex::new(Vec::new());

/// Телефон или камера, внутри которых `path`: (папка gvfs, имя, значок).
/// Путь в gvfs, которого нет в запомненных (окно открыто прямо на телефоне,
/// панель ещё не строилась), — спросить gvfs, но один раз до следующей
/// перестройки панели.
pub fn gadget_root(path: &Path) -> Option<(PathBuf, String, &'static str)> {
    use std::sync::atomic::Ordering;
    let find = |g: &[(PathBuf, String, &'static str)]| g.iter().find(|(r, _, _)| path.starts_with(r)).cloned();
    let mut g = GADGETS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(hit) = find(&g) {
        return Some(hit);
    }
    let gvfs = PathBuf::from(format!("/run/user/{}/gvfs", unsafe { libc::getuid() }));
    if !path.starts_with(&gvfs) || GADGETS_ASKED.swap(true, Ordering::Relaxed) {
        return None;
    }
    *g = crate::drives::gadgets().into_iter().filter_map(|d| Some((d.mount?, d.title, kind_icon(d.kind)))).collect();
    find(&g)
}

static GADGETS_ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn kind_icon(k: Kind) -> &'static str {
    match k {
        Kind::Disk => crate::ui::icons::DRIVE,
        Kind::Usb => crate::ui::icons::USB,
        Kind::Phone => crate::ui::icons::PHONE,
        Kind::Camera => crate::ui::icons::CAMERA,
    }
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
        // Файлы связанных устройств (synlink) — отдельно, ниже: с именем и домашним каталогом.
        if fs == "fuse.synlink" {
            continue;
        }
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
    // Связанные устройства (synlink): телефон ↔ компьютер, открываются в домашнем
    // каталоге устройства. Демон не ответил за 150 мс — без них.
    if let Some(st) = synshell_common::link::status_quick(std::time::Duration::from_millis(150)) {
        use synshell_common::link::DeviceKind;
        for p in st.peers.iter().filter(|p| p.connected) {
            let Some(path) = p.files_path() else { continue };
            let icon = match p.kind {
                DeviceKind::Phone | DeviceKind::Tablet => crate::ui::icons::PHONE,
                DeviceKind::Laptop => crate::ui::icons::LAPTOP,
                DeviceKind::Desktop => crate::ui::icons::COMPUTER,
            };
            out.push(Mount { path: PathBuf::from(path), title: p.name.clone(), icon });
        }
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

/// Следить за монтированием (флешки, сетевые ресурсы, файлы связанных
/// устройств synlink): `/proc/self/mounts` будит `poll` с `POLLPRI` при
/// каждом изменении — тогда перестроить боковую панель.
pub fn watch_mounts() {
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("files-mounts".into())
        .spawn(|| {
            use std::os::fd::AsRawFd;
            let Ok(f) = std::fs::File::open("/proc/self/mounts") else { return };
            loop {
                let mut p = libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLPRI | libc::POLLERR, revents: 0 };
                let r = unsafe { libc::poll(&mut p, 1, -1) };
                if r < 0 {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    continue;
                }
                // Файл надо перечитать, иначе poll сработает снова сразу.
                use std::io::{Read, Seek};
                let mut g = &f;
                let _ = g.seek(std::io::SeekFrom::Start(0));
                let mut sink = String::new();
                let _ = g.read_to_string(&mut sink);
                syngui::async_runtime::run_on_main_thread(|| {
                    if let Some(ctx) = crate::state::try_ctx() {
                        ctx.places_rev.update(|r| *r += 1);
                    }
                });
            }
        })
        .ok();
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
