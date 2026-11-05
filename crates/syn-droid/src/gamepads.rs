//! Геймпады в Android: экранный контроллер syngamepad (uinput, «Microsoft X-Box 360 pad») и настоящие
//! USB/Bluetooth-геймпады хоста. У контейнера свой /dev (tmpfs), а устройство появляется и уходит на ходу,
//! поэтому демон раз в секунду находит устройства ввода хоста с кнопкой BTN_SOUTH и создаёт/убирает
//! узлы `eventN` в `/dev/input` контейнера — EventHub Android замечает их через inotify сам (раскладка
//! Xbox 360 — Vendor_045e_Product_028e.kl в Android есть).

use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Группа input Android (AID_INPUT): её читает system_server.
const AID_INPUT: u32 = 1004;
/// BTN_SOUTH (BTN_A): у любого геймпада есть.
const BTN_SOUTH: usize = 0x130;

/// Устройство ввода хоста — геймпад: бит BTN_SOUTH в `capabilities/key` (шестнадцатеричные слова, старшее первым).
fn is_gamepad(event: &Path) -> bool {
    let Ok(s) = fs::read_to_string(event.join("device/capabilities/key")) else { return false };
    let words: Vec<&str> = s.split_whitespace().collect();
    let bits = usize::BITS as usize;
    let (word, bit) = (BTN_SOUTH / bits, BTN_SOUTH % bits);
    let Some(w) = words.len().checked_sub(word + 1).and_then(|i| words.get(i)) else { return false };
    usize::from_str_radix(w, 16).is_ok_and(|v| v & (1 << bit) != 0)
}

/// Геймпады хоста: (eventN, major, minor).
fn host_gamepads() -> Vec<(String, u32, u32)> {
    let mut out = Vec::new();
    for e in fs::read_dir("/sys/class/input").into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with("event") || !is_gamepad(&e.path()) {
            continue;
        }
        let Ok(dev) = fs::read_to_string(e.path().join("dev")) else { continue };
        let Some((ma, mi)) = dev.trim().split_once(':') else { continue };
        if let (Ok(ma), Ok(mi)) = (ma.parse(), mi.parse()) {
            out.push((name, ma, mi));
        }
    }
    out
}

fn mknod(path: &Path, major: u32, minor: u32) -> std::io::Result<()> {
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(std::io::Error::other)?;
    // SAFETY: путь — CString, dev собран libc::makedev.
    let r = unsafe { libc::mknod(c.as_ptr(), libc::S_IFCHR | 0o660, libc::makedev(major, minor)) };
    if r != 0 {
        return Err(std::io::Error::last_os_error());
    }
    std::os::unix::fs::chown(path, Some(0), Some(AID_INPUT))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o660))
}

/// Следить, пока `alive()`: контейнер с init `init_pid` работает.
pub fn watch(init_pid: i32, alive: impl Fn() -> bool + Send + 'static) {
    std::thread::Builder::new()
        .name("gamepads".into())
        .spawn(move || {
            let dir = PathBuf::from(format!("/proc/{init_pid}/root/dev/input"));
            let mut ours: HashSet<String> = HashSet::new();
            while alive() {
                // /dev/input создаёт hwcomposer Android при загрузке — до того ждать.
                if dir.is_dir() {
                    let now = host_gamepads();
                    for (name, ma, mi) in &now {
                        let p = dir.join(name);
                        if ours.contains(name) && p.exists() {
                            continue;
                        }
                        let _ = fs::remove_file(&p);
                        match mknod(&p, *ma, *mi) {
                            Ok(()) => {
                                tracing::info!("геймпад {name} ({ma}:{mi}) → Android");
                                ours.insert(name.clone());
                            }
                            Err(e) => tracing::warn!("геймпад {name}: {e}"),
                        }
                    }
                    let gone: Vec<String> = ours.iter().filter(|n| !now.iter().any(|(m, ..)| m == *n)).cloned().collect();
                    for name in gone {
                        let _ = fs::remove_file(dir.join(&name));
                        tracing::info!("геймпад {name} убран из Android");
                        ours.remove(&name);
                    }
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .expect("поток геймпадов");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn btn_south_bit() {
        let d = std::env::temp_dir().join(format!("syndroid-gp-{}", std::process::id()));
        fs::create_dir_all(d.join("device/capabilities")).unwrap();
        // xpad: «7cdb000000000000 0 0 0 0» — слово 4 от младшего, бит 48 (0x130 = 304 = 4·64 + 48)
        fs::write(d.join("device/capabilities/key"), "7cdb000000000000 0 0 0 0\n").unwrap();
        assert!(is_gamepad(&d));
        fs::write(d.join("device/capabilities/key"), "1000000000007 ff9f207ac14057ff febeffdfffefffff fffffffffffffffe\n").unwrap();
        assert!(!is_gamepad(&d), "клавиатура");
        let _ = fs::remove_dir_all(&d);
    }
}
