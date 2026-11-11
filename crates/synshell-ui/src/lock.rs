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
                Err(if e.is_null() { t!("ошибка PAM {r}", r = r) } else { CStr::from_ptr(e).to_string_lossy().into_owned() })
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
    // За блокировкой запомненный пароль sudo действовать не должен.
    syn_system::polkit_agent::forget();
    let st = state();
    st.error.set(String::new());
    st.checking.set(false);
    NO_PASSWORD.store(detect_no_password(), std::sync::atomic::Ordering::Relaxed);
    ctx.close_popup();
    close_preview();
    syngui_layer::lock_session(move |out: &OutputInfo| {
        let ctx = ShellCtx::get();
        if touch_lock(ctx) {
            Box::new(phone_view(ctx, out.clone()))
        } else {
            Box::new(view(ctx, out.clone()))
        }
    });
}

/// Телефонный экран блокировки (своя клавиатура и PIN-панель): режим «телефон» или игровой режим на телефоне.
/// Экранная клавиатура synkeyboard поверх блокировки не показывается, а у настольного вида своей нет — в игровом
/// режиме на телефоне поле пароля было не заполнить ничем (только перезагрузка).
fn touch_lock(ctx: ShellCtx) -> bool {
    ctx.is_phone() || device_is_phone(&ctx)
}

/// Игровой режим на телефоне (композитор считает устройство телефоном).
pub fn device_is_phone(ctx: &ShellCtx) -> bool {
    use synshell_common::config::FormFactor;
    static DEVICE_PHONE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    ctx.form_factor == FormFactor::Game
        && *DEVICE_PHONE.get_or_init(|| {
            use synshell_common::ipc::{Client, Request, Response};
            matches!(Client::connect().and_then(|mut c| c.request(&Request::FormFactor)), Ok(Response::FormFactor { device, .. }) if device == "phone")
        })
}

fn submit(password: String) {
    let st = state();
    if st.checking.get_untracked() {
        return;
    }
    if password.is_empty() {
        // Пользователь без пароля — снять блокировку без проверки.
        if no_password() {
            syngui_layer::unlock_session();
        }
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
                    st.error.set(t!("Неверный пароль").into());
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
            .placeholder(if checking { t!("Проверка…") } else { t!("Пароль") })
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
    // Идущий разговор — над часами; входящий — карточкой поверх всего (ответ без разблокировки).
    let top = Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Center).child(crate::calls::lock_banner()).child(clock);
    Stack::new()
        .fit(StackFit::Expand)
        .child(DecoratedBox::new().class("wallpaper-color"))
        .child(DecoratedBox::new().class("lock-scrim"))
        .child(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(top)
                .child(card)
                .child(power)
                .class("lock-root"),
        )
        .child(crate::calls::lock_overlay())
}

// ─── Телефон ─────────────────────────────────────────────────────────────────
//
// Общий экран входа и блокировки ([`crate::login_screen`]): обложка с часами и уведомлениями, свайп
// вверх — цифровая панель PIN или пароль с клавиатурой synkeyboard (встроенной: демон при блокировке
// сеанса не показывается). У блокировки и у предпросмотра — свои экраны и своё состояние.

use crate::login_screen::{self, Stage};

/// Экран блокировки телефона на выводе `out`.
fn phone_view(ctx: ShellCtx, out: OutputInfo) -> impl Widget {
    let st = state();
    let spec = login_screen::Spec {
        name: display_name(),
        now: ctx.now,
        notes: Some(ctx.history),
        checking: st.checking,
        error: st.error,
        attempt: st.attempt,
        submit: std::sync::Arc::new(submit),
        swipe: std::sync::Arc::new(|| {
            if swipe_unlocks() {
                log::info!("экран блокировки: снят свайпом ([lock] method = \"swipe\")");
                syngui_layer::unlock_session();
                return true;
            }
            false
        }),
        extra: None,
        footer: None,
        banner: Some(std::sync::Arc::new(|| Box::new(crate::calls::lock_banner()) as Box<dyn Widget>)),
        background: std::sync::Arc::new(move || Box::new(crate::manager::wallpaper_view(out.name.clone(), use_signal(0u64), None)) as Box<dyn Widget>),
        initial: Stage::Cover,
        anim_ms: crate::anim::group_ms(&ctx, "home", 280),
    };
    // Входящий звонок — поверх всего: ответить или отклонить можно, не снимая блокировку.
    Stack::new()
        .fit(StackFit::Expand)
        .child(login_screen::view(spec))
        .child(crate::charge_anim::layer())
        .child(crate::calls::lock_overlay())
}

thread_local! {
    /// Поверхность предпросмотра (`shell lock-preview`).
    static PREVIEW: std::cell::Cell<Option<syngui_layer::SurfaceId>> = const { std::cell::Cell::new(None) };
    static PREVIEW_WATCH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Сколько живёт предпросмотр без действий — потом закрывается сам.
const PREVIEW_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Открыт ли предпросмотр экрана блокировки (жест «назад» закрывает его).
pub fn preview_open() -> bool {
    PREVIEW.with(|p| p.get()).is_some()
}

/// Закрыть предпросмотр экрана блокировки, если он открыт.
pub fn close_preview() {
    if let Some(id) = PREVIEW.with(|p| p.take()) {
        syngui_layer::close_surface(id);
    }
}

/// Отладка без блокировки сеанса: телефонный экран блокировки отдельной поверхностью поверх всего
/// (`shell lock-preview [pin|text]` — сразу цифровая панель или ввод пароля). Состояние — своё, не
/// настоящей блокировки. Закрывается жестом «назад», свайпом вверх по обложке, верным паролем, сам через
/// [`PREVIEW_TIMEOUT`], повторной командой, а также при блокировке и разблокировке сеанса.
pub fn preview(stage: &str) {
    let stage = match stage {
        "pin" => Stage::Pin,
        "text" => Stage::Text,
        _ => Stage::Cover,
    };
    close_preview();
    use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceSpec};
    let outs = syngui_layer::outputs().get_untracked();
    let Some(out) = outs.first().cloned() else { return };
    // Блокировка или разблокировка сеанса снимает предпросмотр (иначе после разблокировки он
    // оставался поверх всего с «Проведите вверх», и свайп его уже не снимал).
    if !PREVIEW_WATCH.with(|w| w.replace(true)) {
        let locked = syngui_layer::session_locked();
        let last = std::cell::Cell::new(locked.get_untracked());
        create_effect(move || {
            let now = locked.get();
            if last.replace(now) != now {
                close_preview();
            }
        });
    }
    let surface = SurfaceSpec {
        namespace: "syndesktop-lock-preview".into(),
        layer: Layer::Overlay,
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        size: (0, 0),
        margin: [0; 4],
        exclusive_zone: -1,
        keyboard: KeyboardInteractivity::OnDemand,
        output: Some(out.name.clone()),
        auto_size: false,
        clear_color: [0.0, 0.0, 0.0, 1.0],
    };
    let id = syngui_layer::create_surface(surface, move || {
        let ctx = ShellCtx::get();
        let (checking, error, attempt) = (use_signal(false), use_signal(String::new()), use_signal(0u32));
        let spec = login_screen::Spec {
            name: display_name(),
            now: ctx.now,
            notes: Some(ctx.history),
            checking,
            error,
            attempt,
            // Пароль проверяется по-настоящему, но верный только закрывает предпросмотр.
            submit: std::sync::Arc::new(move |password: String| {
                if checking.get_untracked() {
                    return;
                }
                checking.set(true);
                error.set(String::new());
                std::thread::spawn(move || {
                    let r = authenticate(&user_name(), &password);
                    syngui::async_runtime::run_on_main_thread(move || {
                        checking.set(false);
                        match r {
                            Ok(()) => close_preview(),
                            Err(_) => {
                                error.set(t!("Неверный пароль").into());
                                attempt.set(attempt.get_untracked() + 1);
                            }
                        }
                    });
                });
            }),
            swipe: std::sync::Arc::new(|| {
                close_preview();
                true
            }),
            extra: None,
            footer: None,
            banner: Some(std::sync::Arc::new(|| Box::new(crate::calls::lock_banner()) as Box<dyn Widget>)),
            background: std::sync::Arc::new({
                let out = out.name.clone();
                move || Box::new(crate::manager::wallpaper_view(out.clone(), use_signal(0u64), None)) as Box<dyn Widget>
            }),
            initial: stage,
            anim_ms: crate::anim::group_ms(&ctx, "home", 280),
        };
        Box::new(Stack::new().fit(StackFit::Expand).child(login_screen::view(spec)).child(crate::calls::lock_overlay()))
    });
    PREVIEW.with(|p| p.set(Some(id)));
    syngui_layer::add_timer(PREVIEW_TIMEOUT, move || {
        if PREVIEW.with(|p| p.get()) == Some(id) {
            close_preview();
        }
        None
    });
}

/// Разблокировка свайпом: так настроено или у пользователя нет пароля
/// (пустой или заблокированный в /etc/shadow — проверить нечем).
fn swipe_unlocks() -> bool {
    if ShellCtx::get().cfg().lock.method == "swipe" {
        return true;
    }
    no_password()
}

/// Пароля нет — посчитано при блокировке ([`lock_now`]).
static NO_PASSWORD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn no_password() -> bool {
    NO_PASSWORD.load(std::sync::atomic::Ordering::Relaxed)
}

/// Пароля у пользователя нет: пустой или заблокированный. `/etc/shadow`
/// читается только root — иначе `passwd -S` о себе (NP — нет пароля, L — заблокирован).
fn detect_no_password() -> bool {
    let user = user_name();
    if let Ok(s) = std::fs::read_to_string("/etc/shadow") {
        return s
            .lines()
            .find(|l| l.split(':').next() == Some(user.as_str()))
            .map(|l| l.split(':').nth(1).unwrap_or("").to_string())
            .is_some_and(|h| h.is_empty() || h.starts_with('!') || h.starts_with('*'));
    }
    std::process::Command::new("passwd")
        .args(["-S", &user])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| matches!(String::from_utf8_lossy(&o.stdout).split_whitespace().nth(1), Some("NP" | "L" | "LK")))
}
