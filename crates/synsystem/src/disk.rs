//! Накопители: смонтированные файловые системы (`/proc/self/mounts` + statvfs) и скорость
//! чтения/записи дисков (`/sys/block/*/stat`).

use crate::Sys;

/// Смонтированная файловая система.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mount {
    pub device: String,
    pub path: String,
    pub fstype: String,
    pub total: u64,
    pub used: u64,
    pub read_only: bool,
}

/// Файловые системы, на которых лежат данные (без виртуальных, привязок и служебных точек).
const REAL_FS: &[&str] = &["ext4", "ext3", "ext2", "btrfs", "xfs", "f2fs", "vfat", "exfat", "ntfs", "ntfs3", "fuseblk", "zfs", "bcachefs", "erofs", "squashfs"];

pub fn mounts(sys: &Sys) -> Vec<Mount> {
    let mut out: Vec<Mount> = Vec::new();
    let text = sys.read("/proc/self/mounts").or_else(|| sys.read("/proc/mounts")).unwrap_or_default();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let (Some(dev), Some(path), Some(fs), Some(opts)) = (f.first(), f.get(1), f.get(2), f.get(3)) else { continue };
        if !REAL_FS.contains(fs) {
            continue;
        }
        let path = unescape(path);
        // Образы только для чтения (squashfs снапов, erofs вендора), кроме съёмных.
        if (*fs == "squashfs" || *fs == "erofs") && !path.starts_with("/run/media") {
            continue;
        }
        // Один раздел в нескольких местах (подтома btrfs, bind) — первый раз.
        if out.iter().any(|m| m.device == *dev) {
            continue;
        }
        let Some((total, used)) = statvfs(&sys.path(&path)) else { continue };
        if total == 0 {
            continue;
        }
        out.push(Mount { device: dev.to_string(), path, fstype: fs.to_string(), total, used, read_only: opts.split(',').any(|o| o == "ro") });
    }
    out
}

/// `\040` в путях mounts — пробел.
fn unescape(s: &str) -> String {
    s.replace("\\040", " ").replace("\\011", "\t").replace("\\134", "\\")
}

fn statvfs(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: c — строка с нулём на конце, st — выходной буфер.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let bs = st.f_frsize as u64;
    let total = st.f_blocks as u64 * bs;
    let free = st.f_bfree as u64 * bs;
    Some((total, total.saturating_sub(free)))
}

/// Диск целиком: имя, модель, размер, байт/с чтения и записи.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Disk {
    pub name: String,
    pub model: Option<String>,
    pub size: u64,
    pub read_bps: f64,
    pub write_bps: f64,
    /// Доля времени, когда диск был занят, %.
    pub busy_percent: f32,
    pub rotational: bool,
}

/// Скорость дисков по разности `/sys/block/<диск>/stat`.
#[derive(Debug, Default)]
pub struct DiskSampler {
    sys: Sys,
    last: std::collections::HashMap<String, (u64, u64, u64)>,
    last_at: Option<std::time::Instant>,
}

impl DiskSampler {
    pub fn new(sys: Sys) -> Self {
        Self { sys, ..Default::default() }
    }

    pub fn sample(&mut self) -> Vec<Disk> {
        let now = std::time::Instant::now();
        let dt = self.last_at.map(|t| now.duration_since(t).as_secs_f64()).filter(|d| *d > 0.0);
        self.last_at = Some(now);
        let mut out = Vec::new();
        let mut seen = std::collections::HashMap::new();
        for name in self.sys.list("/sys/block") {
            // ОЗУ-диски, петли (образ корня на телефоне считается на своём разделе), zram — не накопители.
            if ["ram", "loop", "zram", "dm-", "md", "sr", "fd", "nbd"].iter().any(|p| name.starts_with(p)) {
                continue;
            }
            let b = format!("/sys/block/{name}");
            let size = self.sys.read_num::<u64>(format!("{b}/size")).unwrap_or(0) * 512;
            if size == 0 {
                continue;
            }
            let stat = self.sys.read(format!("{b}/stat")).unwrap_or_default();
            let f: Vec<u64> = stat.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            let (rd, wr, io_ms) = (f.get(2).copied().unwrap_or(0) * 512, f.get(6).copied().unwrap_or(0) * 512, f.get(9).copied().unwrap_or(0));
            let mut d = Disk {
                model: self.sys.read(format!("{b}/device/model")).filter(|m| !m.is_empty()),
                size,
                rotational: self.sys.read(format!("{b}/queue/rotational")).as_deref() == Some("1"),
                name: name.clone(),
                ..Default::default()
            };
            if let (Some(dt), Some(&(r0, w0, t0))) = (dt, self.last.get(&name)) {
                d.read_bps = rd.saturating_sub(r0) as f64 / dt;
                d.write_bps = wr.saturating_sub(w0) as f64 / dt;
                d.busy_percent = (io_ms.saturating_sub(t0) as f64 / (dt * 10.0)).clamp(0.0, 100.0) as f32;
            }
            seen.insert(name, (rd, wr, io_ms));
            out.push(d);
        }
        self.last = seen;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn disk_rates() {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("sys/block/sda");
        fs::create_dir_all(b.join("device")).unwrap();
        fs::create_dir_all(dir.path().join("sys/block/loop0")).unwrap();
        fs::write(b.join("size"), "2000").unwrap();
        fs::write(b.join("device/model"), "KM8L9001JM-B624").unwrap();
        fs::write(b.join("stat"), "1 0 100 0 1 0 200 0 0 10 0").unwrap();
        let mut s = DiskSampler::new(Sys::at(dir.path()));
        let d = s.sample();
        assert_eq!(d.len(), 1);
        assert_eq!((d[0].size, d[0].model.as_deref()), (1_024_000, Some("KM8L9001JM-B624")));
        fs::write(b.join("stat"), "1 0 300 0 1 0 200 0 0 10 0").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(s.sample()[0].read_bps > 0.0);
    }

    #[test]
    fn root_mount() {
        let m = mounts(&Sys::host());
        assert!(m.iter().all(|m| m.total > 0));
        assert_eq!(unescape("/run/media/My\\040Disk"), "/run/media/My Disk");
    }
}
