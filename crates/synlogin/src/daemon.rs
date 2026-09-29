//! Демон входа (`synlogin daemon`, от root, юнит systemd): по кругу
//! 1. композитор synwm с экраном входа вместо оболочки (`SYNSHELL_SHELL=
//!    "synlogin greeter"`); экран входа проверяет пароль, пишет выбранного
//!    пользователя в [`REQUEST`] и завершает композитор;
//! 2. сеанс пользователя: устройства (DRM, KGSL, ввод) — ему во владение,
//!    `/run/synlogin/session/UID`, композитор от его имени (`dbus-run-session synwm`);
//! 3. «Выйти» в оболочке завершает сеанс — снова экран входа.
//!
//! Как `greetd`, но без VT и logind-сеанса: на телефоне с Android-ядром их
//! нет (LIBSEAT_BACKEND=noop), поэтому доступ к устройствам выдаётся
//! владельцем файлов и возвращается root после сеанса.

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::users;

/// Запрос экрана входа: имя пользователя (или `!poweroff`, `!reboot`).
pub const REQUEST: &str = "/run/synlogin/request";
/// Каталоги XDG_RUNTIME_DIR сеансов: `/run/synlogin/session/UID`.
const SESSIONS: &str = "/run/synlogin/session";

fn synwm_args() -> Vec<String> {
    // Аргументы композитора — из командной строки демона после `--`
    // (например, `--cpu`); `--tty` всегда.
    let mut v: Vec<String> = std::env::args().skip_while(|a| a != "--").skip(1).collect();
    if !v.iter().any(|a| a == "--tty") {
        v.insert(0, "--tty".into());
    }
    v
}

fn synwm_bin() -> String {
    std::env::var("SYNLOGIN_SYNWM").unwrap_or_else(|_| "synwm".into())
}

/// Узлы устройств, которые нужны сеансу (без logind — выдаём владельцем).
fn device_nodes() -> Vec<String> {
    let mut v = Vec::new();
    for dir in ["/dev/dri", "/dev/input", "/dev/dma_heap", "/dev/snd"] {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                v.push(e.path().to_string_lossy().into_owned());
            }
        }
    }
    for f in ["/dev/kgsl-3d0", "/dev/uinput", "/dev/rfkill"] {
        if Path::new(f).exists() {
            v.push(f.into());
        }
    }
    // Подсветка и фонарик — запись яркости из оболочки.
    for dir in ["/sys/class/backlight", "/sys/class/leds"] {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let b = e.path().join("brightness");
                if b.exists() {
                    v.push(b.to_string_lossy().into_owned());
                }
            }
        }
    }
    v
}

fn chown_all(paths: &[String], uid: u32, gid: u32) {
    for p in paths {
        let c = std::ffi::CString::new(p.as_str()).unwrap();
        // SAFETY: путь — нуль-терминированная строка.
        unsafe {
            libc::chown(c.as_ptr(), uid, gid);
        }
    }
}

fn run_greeter() {
    let _ = std::fs::create_dir_all("/run/synlogin");
    let _ = std::fs::remove_file(REQUEST);
    let rt = "/run/synlogin/greeter";
    let _ = std::fs::create_dir_all(rt);
    let _ = std::fs::set_permissions(rt, std::os::unix::fs::PermissionsExt::from_mode(0o700));
    log::info!("экран входа");
    let st = Command::new("dbus-run-session")
        .arg(synwm_bin())
        .args(synwm_args())
        .arg("--no-autostart")
        .env("XDG_RUNTIME_DIR", rt)
        .env("SYNSHELL_SHELL", "synlogin greeter")
        .env("HOME", "/root")
        .status();
    log::info!("экран входа завершён: {st:?}");
}

fn run_session(user: &users::User) {
    // Не /run/user/UID: им управляет logind — после выхода из последнего
    // ssh-сеанса пользователя он удаляет каталог вместе с сокетами
    // композитора (сеанс synlogin для logind не существует).
    let _ = std::fs::create_dir_all(SESSIONS);
    let rt = format!("{SESSIONS}/{}", user.uid);
    let _ = std::fs::create_dir_all(&rt);
    chown_all(&[rt.clone()], user.uid, user.gid);
    let _ = std::fs::set_permissions(&rt, std::os::unix::fs::PermissionsExt::from_mode(0o700));
    let devices = device_nodes();
    if user.uid != 0 {
        chown_all(&devices, user.uid, user.gid);
    }
    log::info!("сеанс {} (uid {})", user.name, user.uid);
    let (uid, gid) = (user.uid, user.gid);
    let name = std::ffi::CString::new(user.name.as_str()).unwrap();
    // Перезагрузка/выключение из сеанса: композитор пишет сюда и выходит.
    let request = format!("{rt}/synlogin-request");
    let _ = std::fs::remove_file(&request);
    let mut cmd = Command::new("dbus-run-session");
    cmd.arg(synwm_bin())
        .args(synwm_args())
        .env_remove("SYNSHELL_SHELL")
        .env("HOME", &user.home)
        .env("USER", &user.name)
        .env("LOGNAME", &user.name)
        .env("SHELL", &user.shell)
        .env("XDG_RUNTIME_DIR", &rt)
        .env("SYNLOGIN_REQUEST", &request)
        .current_dir(&user.home);
    // SAFETY: между fork и exec — только async-signal-safe вызовы libc.
    unsafe {
        cmd.pre_exec(move || {
            if libc::initgroups(name.as_ptr(), gid) != 0 || libc::setgid(gid) != 0 || libc::setuid(uid) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::setsid();
            Ok(())
        });
    }
    let st = cmd.status();
    log::info!("сеанс {} завершён: {st:?}", user.name);
    if user.uid != 0 {
        // Программы, пережившие композитор, не должны держать устройства.
        let _ = Command::new("pkill").args(["-KILL", "-u", &user.uid.to_string()]).status();
        chown_all(&devices, 0, 0);
    }
    let req = std::fs::read_to_string(&request).unwrap_or_default();
    let _ = std::fs::remove_file(&request);
    power_request(req.trim());
}

fn power_request(req: &str) {
    match req {
        "!poweroff" => {
            let _ = Command::new("systemctl").arg("poweroff").status();
        }
        "!reboot" => {
            let _ = Command::new("systemctl").arg("reboot").status();
        }
        _ => {}
    }
}

pub fn run() -> ! {
    // SAFETY: geteuid без побочных эффектов.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("synlogin daemon: нужны права root");
        std::process::exit(1);
    }
    loop {
        run_greeter();
        let req = std::fs::read_to_string(REQUEST).unwrap_or_default();
        let req = req.trim();
        match req {
            "" => {
                // Экран входа упал или композитор не поднялся — не крутиться.
                std::thread::sleep(Duration::from_secs(2));
            }
            "!poweroff" | "!reboot" => power_request(req),
            name => match users::find(name) {
                Some(u) => run_session(&u),
                None => log::warn!("нет пользователя {name}"),
            },
        }
    }
}
