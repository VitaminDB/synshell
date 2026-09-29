//! Встроенный экран блокировки: ext-session-lock (через syngui-layer) и
//! проверка пароля PAM. Сервис PAM — `syndesktop-lock`, если файл
//! `/etc/pam.d/syndesktop-lock` установлен, иначе `login` (как у swaylock).

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use syngui::prelude::*;
use syngui_layer::OutputInfo;

use crate::ctx::ShellCtx;
use crate::ui::{icon, mi, InputArea};

// ─── PAM ─────────────────────────────────────────────────────────────────────

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

#[repr(C)]
struct PamConv {
    conv: extern "C" fn(c_int, *mut *const PamMessage, *mut *mut PamResponse, *mut c_void) -> c_int,
    appdata_ptr: *mut c_void,
}

#[link(name = "pam")]
extern "C" {
    fn pam_start(service: *const c_char, user: *const c_char, conv: *const PamConv, pamh: *mut *mut c_void) -> c_int;
    fn pam_authenticate(pamh: *mut c_void, flags: c_int) -> c_int;
    fn pam_acct_mgmt(pamh: *mut c_void, flags: c_int) -> c_int;
    fn pam_setcred(pamh: *mut c_void, flags: c_int) -> c_int;
    fn pam_end(pamh: *mut c_void, status: c_int) -> c_int;
    fn pam_strerror(pamh: *mut c_void, errnum: c_int) -> *const c_char;
}

const PAM_SUCCESS: c_int = 0;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_REFRESH_CRED: c_int = 0x0010;
const PAM_BUF_ERR: c_int = 5;

/// Ответ на все запросы PAM — пароль (он в `appdata_ptr`).
extern "C" fn conversation(
    num: c_int,
    msgs: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata: *mut c_void,
) -> c_int {
    if num <= 0 || msgs.is_null() || resp.is_null() {
        return PAM_BUF_ERR;
    }
    // SAFETY: PAM передаёт массив из `num` сообщений; ответы выделяются
    // calloc — их освобождает сам PAM.
    unsafe {
        let out = libc::calloc(num as usize, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
        if out.is_null() {
            return PAM_BUF_ERR;
        }
        let password = appdata as *const c_char;
        for i in 0..num as isize {
            let m = *msgs.offset(i);
            if m.is_null() {
                continue;
            }
            let style = (*m).msg_style;
            if style == PAM_PROMPT_ECHO_OFF || style == PAM_PROMPT_ECHO_ON {
                (*out.offset(i)).resp = libc::strdup(password);
            }
        }
        *resp = out;
    }
    PAM_SUCCESS
}

fn pam_service() -> &'static str {
    if std::path::Path::new("/etc/pam.d/syndesktop-lock").exists() {
        "syndesktop-lock"
    } else {
        "login"
    }
}

/// Проверить пароль пользователя. Блокирует — вызывать из фонового потока.
pub fn authenticate(user: &str, password: &str) -> std::result::Result<(), String> {
    let service = CString::new(pam_service()).unwrap();
    let user = CString::new(user).map_err(|_| "имя пользователя".to_string())?;
    let mut pwd = CString::new(password).map_err(|_| "недопустимый символ".to_string())?.into_bytes_with_nul();
    let conv = PamConv { conv: conversation, appdata_ptr: pwd.as_mut_ptr() as *mut c_void };
    let mut h: *mut c_void = std::ptr::null_mut();
    // SAFETY: строки живут до pam_end, дескриптор проверен.
    let result = unsafe {
        let r = pam_start(service.as_ptr(), user.as_ptr(), &conv, &mut h);
        if r != PAM_SUCCESS || h.is_null() {
            Err(format!("pam_start: {r}"))
        } else {
            let mut r = pam_authenticate(h, 0);
            if r == PAM_SUCCESS {
                r = pam_acct_mgmt(h, 0);
            }
            if r == PAM_SUCCESS {
                // Продлить билеты Kerberos и т.п. — ошибки не критичны.
                let _ = pam_setcred(h, PAM_REFRESH_CRED);
            }
            let msg = if r == PAM_SUCCESS {
                Ok(())
            } else {
                let e = pam_strerror(h, r);
                Err(if e.is_null() { format!("ошибка PAM {r}") } else { CStr::from_ptr(e).to_string_lossy().into_owned() })
            };
            pam_end(h, r);
            msg
        }
    };
    // Стереть пароль из памяти.
    pwd.iter_mut().for_each(|b| *b = 0);
    result
}

// ─── Экран ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct LockState {
    checking: RwSignal<bool>,
    error: RwSignal<String>,
    /// Счётчик попыток — пересоздаёт поле ввода (очистка после ошибки).
    attempt: RwSignal<u32>,
}

thread_local! {
    static STATE: std::cell::OnceCell<LockState> = const { std::cell::OnceCell::new() };
}

fn state() -> LockState {
    STATE.with(|s| {
        *s.get_or_init(|| LockState { checking: use_signal(false), error: use_signal(String::new()), attempt: use_signal(0) })
    })
}

fn user_name() -> String {
    std::env::var("USER").unwrap_or_default()
}

/// Полное имя из GECOS (`/etc/passwd`), иначе логин.
fn display_name() -> String {
    let user = user_name();
    std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|s| {
            s.lines().find(|l| l.split(':').next() == Some(user.as_str())).and_then(|l| {
                let g = l.split(':').nth(4)?.split(',').next()?.trim().to_string();
                (!g.is_empty()).then_some(g)
            })
        })
        .unwrap_or(user)
}

/// Заблокировать сеанс встроенным экраном.
pub fn lock_now(ctx: ShellCtx) {
    if syngui_layer::session_locked().get_untracked() {
        return;
    }
    let st = state();
    st.error.set(String::new());
    st.checking.set(false);
    ctx.close_popup();
    syngui_layer::lock_session(move |out: &OutputInfo| Box::new(view(ShellCtx::get(), out.clone())));
}

fn submit(password: String) {
    let st = state();
    if st.checking.get_untracked() || password.is_empty() {
        return;
    }
    st.checking.set(true);
    st.error.set(String::new());
    std::thread::spawn(move || {
        let r = authenticate(&user_name(), &password);
        drop(password);
        syngui::async_runtime::run_on_main_thread(move || {
            let st = state();
            st.checking.set(false);
            match r {
                Ok(()) => {
                    log::info!("экран блокировки: пароль верный, снимаем блокировку");
                    syngui_layer::unlock_session();
                }
                Err(e) => {
                    log::info!("экран блокировки: отказ ({e})");
                    st.error.set("Неверный пароль".into());
                    st.attempt.set(st.attempt.get_untracked() + 1);
                }
            }
        });
    });
}

fn view(ctx: ShellCtx, _out: OutputInfo) -> impl Widget {
    let st = state();
    let name = display_name();
    let initial: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    let clock = move || {
        let now = ctx.now.get();
        Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new(crate::clock::format(now - now % 60, "%H:%M")).class("lock-time"))
            .child(Text::new(crate::clock::format(now, "%A, %e %B")).class("lock-date"))
    };
    let field = move || {
        let _ = st.attempt.get();
        let checking = st.checking.get();
        TextField::new()
            .obscure(true)
            .autofocus(true)
            .disabled(checking)
            .placeholder(if checking { "Проверка…" } else { "Пароль" })
            .on_submit(|t| submit(t.to_string()))
            .class("lock-field")
    };
    let status = move || {
        let e = st.error.get();
        let kb = ctx.keyboard.get();
        let layout = kb.short.get(kb.current as usize).cloned().unwrap_or_default().to_uppercase();
        Row::new()
            .gap(10.0)
            .child(Text::new(e).class("lock-error"))
            .child(Text::new(layout).class("lock-layout"))
    };
    let card = Column::new()
        .gap(14.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(crate::ui::vcenter(Text::new(initial).class("lock-avatar-text"))).class("lock-avatar"))
        .child(Text::new(name).class("lock-user"))
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(icon(mi::LOCK).class("lock-icon"))
                .child(field),
        )
        .child(status);
    let power = Row::new()
        .gap(8.0)
        .child(
            InputArea::new(DecoratedBox::new().child(icon(mi::SLEEP)).class("footer-btn"))
                .pointer()
                .on_click(|_, _, _| crate::actions::spawn("systemctl suspend")),
        )
        .child(
            InputArea::new(DecoratedBox::new().child(icon(mi::POWER)).class("footer-btn"))
                .pointer()
                .on_click(|_, _, _| crate::actions::spawn("systemctl poweroff")),
        );
    Stack::new()
        .fit(StackFit::Expand)
        .child(DecoratedBox::new().class("wallpaper-color"))
        .child(DecoratedBox::new().class("lock-scrim"))
        .child(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(clock)
                .child(card)
                .child(power)
                .class("lock-root"),
        )
}
