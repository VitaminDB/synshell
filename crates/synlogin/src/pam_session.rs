//! Сеанс logind через PAM (`synlogin session-worker`) — на десктопе, где в
//! ядре есть VT и работает logind. Без logind-сеанса libseat композитора не
//! открывает seat («Failed to open session: Function not implemented»), а
//! устройства, VT и права polkit сеанса выдаёт logind.
//!
//! Как у greetd: отдельный процесс-«работник» (root) открывает сеанс PAM
//! (pam_systemd регистрирует его в logind: seat0, VT, класс greeter/user),
//! запускает программу от имени пользователя, ждёт её и закрывает сеанс.
//! Демон в сеанс не попадает — pam_systemd переносит в scope сеанса только
//! работника и его потомков.
//!
//! Телефоны (Android-ядро без VT, юниты с `LIBSEAT_BACKEND=noop`) идут прежним
//! путём без logind-сеанса — см. [`available`].

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

#[repr(C)]
struct PamConv {
    conv: extern "C" fn(c_int, *mut *const c_void, *mut *mut c_void, *mut c_void) -> c_int,
    appdata_ptr: *mut c_void,
}

#[link(name = "pam")]
extern "C" {
    fn pam_start(service: *const c_char, user: *const c_char, conv: *const PamConv, pamh: *mut *mut c_void) -> c_int;
    fn pam_set_item(pamh: *mut c_void, item: c_int, value: *const c_void) -> c_int;
    fn pam_putenv(pamh: *mut c_void, name_value: *const c_char) -> c_int;
    fn pam_getenvlist(pamh: *mut c_void) -> *mut *mut c_char;
    fn pam_setcred(pamh: *mut c_void, flags: c_int) -> c_int;
    fn pam_open_session(pamh: *mut c_void, flags: c_int) -> c_int;
    fn pam_close_session(pamh: *mut c_void, flags: c_int) -> c_int;
    fn pam_end(pamh: *mut c_void, status: c_int) -> c_int;
    fn pam_strerror(pamh: *mut c_void, errnum: c_int) -> *const c_char;
}

const PAM_SUCCESS: c_int = 0;
const PAM_CONV_ERR: c_int = 19;
const PAM_TTY: c_int = 3;
const PAM_ESTABLISH_CRED: c_int = 0x0002;
const PAM_DELETE_CRED: c_int = 0x0004;

const VT_ACTIVATE: libc::c_ulong = 0x5606;
const VT_WAITACTIVE: libc::c_ulong = 0x5607;

/// Сеансу ничего спрашивать не нужно: пароль проверил экран входа.
extern "C" fn no_conversation(_: c_int, _: *mut *const c_void, _: *mut *mut c_void, _: *mut c_void) -> c_int {
    PAM_CONV_ERR
}

/// Сеанс через logind: есть VT (`/dev/tty0`), logind ведёт seat0 и libseat не
/// переназначен явно (на телефонах юнит задаёт `LIBSEAT_BACKEND=noop`).
/// `SYNLOGIN_NO_LOGIND=1` — прежний путь и на десктопе.
pub fn available() -> bool {
    std::env::var_os("SYNLOGIN_NO_LOGIND").is_none()
        && std::env::var("LIBSEAT_BACKEND").map_or(true, |v| v.is_empty())
        && Path::new("/dev/tty0").exists()
        && Path::new("/run/systemd/seats/seat0").exists()
}

/// VT экрана входа и сеансов: `SYNLOGIN_VT`, иначе 1 (юнит гасит getty@tty1).
pub fn vt() -> u32 {
    std::env::var("SYNLOGIN_VT").ok().and_then(|v| v.parse().ok()).filter(|&v| v > 0).unwrap_or(1)
}

/// Команда работника: `synlogin session-worker КЛАСС ПОЛЬЗОВАТЕЛЬ -- программа…`.
pub fn worker_command(class: &str, user: &str, program: &str) -> Command {
    let mut c = Command::new(std::env::current_exe().unwrap_or_else(|_| "synlogin".into()));
    c.args(["session-worker", class, user, "--", program]);
    // SAFETY: prctl — async-signal-safe. Демон умер (systemctl restart) —
    // работник завершает сеанс, а не остаётся вторым композитором.
    unsafe {
        c.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    c
}

fn pam_err(pamh: *mut c_void, rc: c_int) -> String {
    // SAFETY: pam_strerror возвращает статическую строку.
    unsafe { CStr::from_ptr(pam_strerror(pamh, rc)).to_string_lossy().into_owned() }
}

/// Переключиться на VT сеанса и дождаться его (как greetd): logind делает
/// активным сеанс текущего VT.
fn activate_vt(vt: u32) {
    let path = CString::new(format!("/dev/tty{vt}")).unwrap();
    // SAFETY: open/ioctl/close с проверенным дескриптором.
    unsafe {
        let fd = libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NOCTTY);
        if fd < 0 {
            log::warn!("VT {vt}: {}", std::io::Error::last_os_error());
            return;
        }
        if libc::ioctl(fd, VT_ACTIVATE, vt as libc::c_int) == 0 {
            libc::ioctl(fd, VT_WAITACTIVE, vt as libc::c_int);
        }
        libc::close(fd);
    }
}

static CHILD: AtomicI32 = AtomicI32::new(0);

extern "C" fn forward_signal(sig: c_int) {
    let pid = CHILD.load(Ordering::Relaxed);
    if pid > 0 {
        // SAFETY: kill — async-signal-safe.
        unsafe {
            libc::kill(pid, sig);
        }
    }
}

/// `synlogin session-worker КЛАСС ПОЛЬЗОВАТЕЛЬ -- программа аргументы…` (root).
/// Окружение программы — своё (его задал демон), недостающее — из PAM
/// (XDG_SESSION_ID, XDG_SEAT, XDG_VTNR…). Код выхода — код программы.
pub fn worker(args: &[String]) -> ! {
    let (Some(class), Some(name)) = (args.first(), args.get(1)) else {
        eprintln!("synlogin session-worker КЛАСС ПОЛЬЗОВАТЕЛЬ -- программа…");
        std::process::exit(2);
    };
    let prog: Vec<&String> = args.iter().skip_while(|a| *a != "--").skip(1).collect();
    let Some(user) = crate::users::find(name) else {
        log::error!("session-worker: нет пользователя {name}");
        std::process::exit(1);
    };
    if prog.is_empty() {
        std::process::exit(2);
    }
    let vt = vt();
    activate_vt(vt);

    let service = if Path::new("/etc/pam.d/synlogin").exists() { "synlogin" } else { "login" };
    let c_service = CString::new(service).unwrap();
    let c_user = CString::new(name.as_str()).unwrap();
    let conv = PamConv { conv: no_conversation, appdata_ptr: std::ptr::null_mut() };
    let mut pamh: *mut c_void = std::ptr::null_mut();
    // SAFETY: строки и conv живут до pam_end.
    let rc = unsafe { pam_start(c_service.as_ptr(), c_user.as_ptr(), &conv, &mut pamh) };
    if rc != PAM_SUCCESS {
        log::error!("pam_start({service}): {rc}");
        std::process::exit(1);
    }
    let tty = CString::new(format!("tty{vt}")).unwrap();
    let env = [
        "XDG_SESSION_TYPE=wayland".to_string(),
        format!("XDG_SESSION_CLASS={class}"),
        "XDG_SESSION_DESKTOP=synshell".into(),
        "XDG_SEAT=seat0".into(),
        format!("XDG_VTNR={vt}"),
    ];
    // SAFETY: pamh открыт; строки копируются PAM.
    unsafe {
        pam_set_item(pamh, PAM_TTY, tty.as_ptr() as *const c_void);
        for e in &env {
            let c = CString::new(e.as_str()).unwrap();
            pam_putenv(pamh, c.as_ptr());
        }
        pam_setcred(pamh, PAM_ESTABLISH_CRED);
    }
    // SAFETY: pamh открыт.
    let rc = unsafe { pam_open_session(pamh, 0) };
    if rc != PAM_SUCCESS {
        log::error!("pam_open_session({service}, {name}): {}", pam_err(pamh, rc));
        // SAFETY: pamh открыт.
        unsafe { pam_end(pamh, rc) };
        std::process::exit(1);
    }
    let mut pam_env: Vec<(String, String)> = Vec::new();
    // SAFETY: pam_getenvlist отдаёт NULL-терминированный массив строк malloc —
    // освобождаем сами.
    unsafe {
        let list = pam_getenvlist(pamh);
        if !list.is_null() {
            let mut p = list;
            while !(*p).is_null() {
                let s = CStr::from_ptr(*p).to_string_lossy().into_owned();
                if let Some((k, v)) = s.split_once('=') {
                    pam_env.push((k.into(), v.into()));
                }
                libc::free(*p as *mut c_void);
                p = p.add(1);
            }
            libc::free(list as *mut c_void);
        }
    }
    // pam_systemd в стеке login — optional: отказ logind (VT занят и т. п.)
    // PAM не считает ошибкой, причина — в журнале от pam_systemd.
    match pam_env.iter().find(|(k, _)| k == "XDG_SESSION_ID") {
        Some((_, id)) => log::info!("logind-сеанс {id}: {name} ({class}, tty{vt})"),
        None => log::warn!("logind не создал сеанс {name} на tty{vt} — см. pam_systemd в журнале"),
    }

    // Шина сеанса user@UID поднимается вместе с сеансом — дождаться сокета.
    if let Some(bus) = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok().and_then(|a| a.strip_prefix("unix:path=").map(String::from)) {
        for _ in 0..50 {
            if Path::new(&bus).exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    let mut cmd = Command::new(prog[0]);
    cmd.args(&prog[1..]);
    for (k, v) in &pam_env {
        if std::env::var_os(k).is_none() {
            cmd.env(k, v);
        }
    }
    let (uid, gid) = (user.uid, user.gid);
    let c_name = c_user.clone();
    // SAFETY: между fork и exec — только async-signal-safe вызовы libc.
    unsafe {
        cmd.pre_exec(move || {
            if uid != 0 && (libc::initgroups(c_name.as_ptr(), gid) != 0 || libc::setgid(gid) != 0 || libc::setuid(uid) != 0) {
                return Err(std::io::Error::last_os_error());
            }
            libc::setsid();
            Ok(())
        });
    }
    let code = match cmd.spawn() {
        Ok(mut child) => {
            CHILD.store(child.id() as i32, Ordering::Relaxed);
            // SAFETY: обработчик только пересылает сигнал программе.
            unsafe {
                for sig in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT] {
                    libc::signal(sig, forward_signal as *const () as libc::sighandler_t);
                }
            }
            match child.wait() {
                Ok(st) => st.code().unwrap_or(128 + std::os::unix::process::ExitStatusExt::signal(&st).unwrap_or(0)),
                Err(_) => 1,
            }
        }
        Err(e) => {
            log::error!("{}: {e}", prog[0]);
            127
        }
    };
    // SAFETY: pamh открыт.
    unsafe {
        let rc = pam_close_session(pamh, 0);
        if rc != PAM_SUCCESS {
            log::warn!("pam_close_session: {}", pam_err(pamh, rc));
        }
        pam_setcred(pamh, PAM_DELETE_CRED);
        pam_end(pamh, PAM_SUCCESS);
    }
    std::process::exit(code);
}
