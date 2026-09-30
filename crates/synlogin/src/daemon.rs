//! Демон входа (`synlogin daemon`, от root, юнит systemd): по кругу
//! 1. композитор synwm с экраном входа вместо оболочки (`SYNSHELL_SHELL=
//!    "synlogin greeter"`); экран входа проверяет пароль, пишет выбранного
//!    пользователя в [`REQUEST`] и завершает композитор;
//! 2. сеанс пользователя: устройства (DRM, KGSL, ввод) — ему во владение,
//!    пользовательский systemd (linger → `user@UID`, `/run/user/UID`, шина
//!    сеанса systemd), композитор от его имени; без logind — свой каталог
//!    `/run/synlogin/session/UID` и `dbus-run-session synwm`;
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

/// Падения композитора сразу после старта подряд. GPU-рендер (zink/turnip на
/// телефоне) может падать в драйвере — после двух таких падений экран входа и
/// сеансы запускаются на CPU (`SYNSHELL_RENDERER=cpu`) до перезапуска демона.
static EARLY_CRASHES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
const EARLY_CRASH: Duration = Duration::from_secs(20);

fn note_compositor_exit(st: &std::io::Result<std::process::ExitStatus>, started: std::time::Instant) {
    use std::sync::atomic::Ordering;
    let crashed = !matches!(st, Ok(s) if s.success()) && started.elapsed() < EARLY_CRASH;
    if !crashed {
        EARLY_CRASHES.store(0, Ordering::Relaxed);
        return;
    }
    let n = EARLY_CRASHES.fetch_add(1, Ordering::Relaxed) + 1;
    let renderer = std::env::var("SYNSHELL_RENDERER").unwrap_or_default();
    if n >= 2 && renderer != "cpu" && !synwm_args().iter().any(|a| a == "--cpu" || a == "--gpu") {
        log::warn!("композитор упал {n} раза подряд сразу после старта — дальше CPU-рендер (SYNSHELL_RENDERER=cpu)");
        std::env::set_var("SYNSHELL_RENDERER", "cpu");
    }
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
    let started = std::time::Instant::now();
    let st = Command::new("dbus-run-session")
        .arg(synwm_bin())
        .args(synwm_args())
        .arg("--no-autostart")
        .env("XDG_RUNTIME_DIR", rt)
        .env("SYNSHELL_SHELL", "synlogin greeter")
        .env("HOME", "/root")
        .status();
    log::info!("экран входа завершён: {st:?}");
    note_compositor_exit(&st, started);
}

/// Пользовательский systemd сеанса: linger через logind поднимает `user@UID`
/// (и `/run/user/UID`, который logind тогда не удаляет после выхода из ssh),
/// у сеанса появляются `systemctl --user`, шина сеанса systemd (`/run/user/UID/bus`)
/// и пользовательские юниты (PipeWire по сокетам и т. п.). Нет logind или user@
/// не поднялся — `None`, сеанс по-старому: свой каталог и `dbus-run-session`.
struct UserManager {
    runtime_dir: String,
    /// user@UID запустили мы (до входа не работал) — остановить после сеанса.
    started: bool,
}

fn start_user_manager(user: &users::User) -> Option<UserManager> {
    if std::env::var_os("SYNLOGIN_NO_SYSTEMD_USER").is_some() {
        return None;
    }
    let unit = format!("user@{}.service", user.uid);
    let active = |u: &str| Command::new("systemctl").args(["is-active", "--quiet", u]).status().is_ok_and(|s| s.success());
    let was_active = active(&unit);
    let ok = Command::new("loginctl").args(["enable-linger", &user.name]).status().is_ok_and(|s| s.success())
        && Command::new("systemctl").args(["start", &unit]).status().is_ok_and(|s| s.success());
    let runtime_dir = format!("/run/user/{}", user.uid);
    // Шина сеанса (dbus.socket пользователя) появляется вместе с user@.
    let bus = format!("{runtime_dir}/bus");
    for _ in 0..50 {
        if Path::new(&bus).exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !ok || !Path::new(&bus).exists() {
        log::warn!("systemd --user для {} не поднялся — сеанс без него", user.name);
        let _ = Command::new("loginctl").args(["disable-linger", &user.name]).status();
        return None;
    }
    log::info!("systemd --user: {unit}");
    Some(UserManager { runtime_dir, started: !was_active })
}

fn stop_user_manager(user: &users::User, m: &UserManager) {
    let _ = Command::new("loginctl").args(["disable-linger", &user.name]).status();
    if m.started {
        let _ = Command::new("systemctl").args(["stop", &format!("user@{}.service", user.uid)]).status();
    }
}

fn run_session(user: &users::User) {
    let _ = std::fs::create_dir_all(SESSIONS);
    let link = format!("{SESSIONS}/{}", user.uid);
    let manager = start_user_manager(user);
    let rt = match &manager {
        // /run/user/UID; /run/synlogin/session/UID — ссылка на него (там сокеты
        // сеанса ищут скрипты и юнит устройства).
        Some(m) => {
            if let Ok(meta) = std::fs::symlink_metadata(&link) {
                if meta.is_dir() {
                    let _ = std::fs::remove_dir_all(&link);
                } else {
                    let _ = std::fs::remove_file(&link);
                }
            }
            let _ = std::os::unix::fs::symlink(&m.runtime_dir, &link);
            m.runtime_dir.clone()
        }
        // Не /run/user/UID: без linger им управляет logind — после выхода из
        // последнего ssh-сеанса пользователя он удаляет каталог вместе с
        // сокетами композитора (сеанс synlogin для logind не существует).
        None => {
            if std::fs::symlink_metadata(&link).is_ok_and(|m| m.file_type().is_symlink()) {
                let _ = std::fs::remove_file(&link);
            }
            let _ = std::fs::create_dir_all(&link);
            chown_all(&[link.clone()], user.uid, user.gid);
            let _ = std::fs::set_permissions(&link, std::os::unix::fs::PermissionsExt::from_mode(0o700));
            link.clone()
        }
    };
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
    let mut cmd = match &manager {
        Some(_) => {
            let mut c = Command::new(synwm_bin());
            c.env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={rt}/bus"));
            c
        }
        None => {
            let mut c = Command::new("dbus-run-session");
            c.arg(synwm_bin());
            c
        }
    };
    cmd.args(synwm_args())
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
    let started = std::time::Instant::now();
    let st = cmd.status();
    log::info!("сеанс {} завершён: {st:?}", user.name);
    note_compositor_exit(&st, started);
    if let Some(m) = &manager {
        stop_user_manager(user, m);
    }
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
