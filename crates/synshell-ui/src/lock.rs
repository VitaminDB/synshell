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
    let st = state();
    st.error.set(String::new());
    st.checking.set(false);
    NO_PASSWORD.store(detect_no_password(), std::sync::atomic::Ordering::Relaxed);
    ctx.close_popup();
    syngui_layer::lock_session(move |out: &OutputInfo| {
        let ctx = ShellCtx::get();
        if ctx.is_phone() {
            Box::new(phone_view(ctx, out.clone(), Stage::Cover))
        } else {
            Box::new(view(ctx, out.clone()))
        }
    });
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

// ─── Телефон ─────────────────────────────────────────────────────────────────
//
// Обложка: обои, крупные часы, дата, уведомления; свайп вверх (или тап)
// открывает ввод: цифровая панель для PIN (пароль PAM из цифр) или полная
// клавиатура syngui («ABC») — экранная клавиатура synkeyboard при
// блокировке сеанса не показывается.

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Cover,
    Pin,
    Text,
}

thread_local! {
    static PHONE: std::cell::OnceCell<(RwSignal<Stage>, RwSignal<String>)> = const { std::cell::OnceCell::new() };
}

fn phone_state() -> (RwSignal<Stage>, RwSignal<String>) {
    PHONE.with(|p| *p.get_or_init(|| (use_signal(Stage::Cover), use_signal(String::new()))))
}

/// Отладка без блокировки сеанса: телефонный экран блокировки обычной
/// поверхностью поверх всего (`shell lock-preview [pin|text]` — сразу
/// цифровая панель или ввод пароля; убрать — `restart-shell`).
pub fn preview(stage: &str) {
    let stage = match stage {
        "pin" => Stage::Pin,
        "text" => Stage::Text,
        _ => Stage::Cover,
    };
    use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceSpec};
    let outs = syngui_layer::outputs().get_untracked();
    let Some(out) = outs.first().cloned() else { return };
    NO_PASSWORD.store(detect_no_password(), std::sync::atomic::Ordering::Relaxed);
    let spec = SurfaceSpec {
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
    syngui_layer::create_surface(spec, move || Box::new(phone_view(ShellCtx::get(), out.clone(), stage)));
}

fn phone_view(ctx: ShellCtx, out: OutputInfo, initial: Stage) -> impl Widget {
    use syngui::widgets::{Motion, PanAxis, SwipeDirection};
    use syngui::GestureDetector;
    let (stage, pin) = phone_state();
    stage.set(initial);
    pin.set(String::new());
    let st = state();
    let dur = crate::anim::group_ms(&ctx, "home", 280);
    // Раскладка — снаружи реактивных узлов: Reactive/AnimatedSwitcher отдают
    // детям свободные ограничения (натуральный размер, прижаты влево-вверх),
    // и `grow`/выравнивание внутри них не работают. Поэтому слои на весь
    // экран — здесь: верх (часы, уведомления) и низ (подсказка / цифры /
    // клавиатура) раздельно, оба по центру.
    let part = move |top: bool| {
        crate::ui::rx(move || {
            let s = stage.get();
            let key = match s {
                Stage::Cover => 1u64,
                Stage::Pin => 2,
                Stage::Text => 3,
            };
            Box::new(
                AnimatedSwitcher::new(key, move || -> Box<dyn Widget> {
                    match (s, top) {
                        (Stage::Cover, true) => Box::new(cover_top(ctx)),
                        (Stage::Cover, false) => Box::new(cover_bottom()),
                        (_, true) => Box::new(Column::new()),
                        (Stage::Pin, false) => Box::new(pin_pad(ctx)),
                        (Stage::Text, false) => Box::new(text_entry()),
                    }
                })
                .directional(true)
                .slide(0.0, 60.0)
                .duration_ms(dur)
                .exit_duration_ms(dur * 2 / 3)
                .animate_size(false),
            )
        })
    };
    // Ошибка пароля — снова пустой ввод.
    create_effect(move || {
        let _ = st.attempt.get();
        pin.set(String::new());
    });
    let gestures = GestureDetector::new()
        .pan_axis(PanAxis::Vertical)
        .on_swipe(move |dir, _| match dir {
            SwipeDirection::Up if stage.get_untracked() == Stage::Cover && swipe_unlocks() => {
                log::info!("экран блокировки: снят свайпом ([lock] method = \"swipe\")");
                syngui_layer::unlock_session();
            }
            SwipeDirection::Up if stage.get_untracked() == Stage::Cover => stage.set(Stage::Pin),
            SwipeDirection::Down => stage.set(Stage::Cover),
            _ => {}
        })
        .on_click(move || {
            if stage.get_untracked() == Stage::Cover && !swipe_unlocks() {
                stage.set(Stage::Pin);
            }
        })
        .child(
            // Два слоя, а не одна колонка SpaceBetween: у цифровой панели
            // верх пустой, и единственный блок встал бы в начало.
            Stack::new()
                .fit(StackFit::Expand)
                .child(Column::new().cross_axis_alignment(CrossAxisAlignment::Center).child(part(true)).class("lock-phone"))
                .child(
                    Column::new()
                        .main_axis_alignment(MainAxisAlignment::End)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(part(false))
                        .class("lock-phone"),
                ),
        );
    let _ = Motion::fade();
    Stack::new()
        .fit(StackFit::Expand)
        .child(crate::manager::wallpaper_view(out.name.clone(), use_signal(0u64), None))
        .child(DecoratedBox::new().class("lock-scrim"))
        .child(gestures)
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

fn cover_top(ctx: ShellCtx) -> impl Widget {
    let clock = crate::ui::rx(move || {
        let now = ctx.now.get();
        Box::new(
            Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(crate::clock::format(now - now % 60, "%H:%M")).class("lock-phone-time"))
                .child(Text::new(crate::clock::format(now, "%A, %e %B")).class("lock-date")),
        )
    });
    let notes = crate::ui::rx(move || {
        let list = ctx.history.get();
        // Ширина экрана без полей .lock-phone: в свободных ограничениях
        // карточки иначе ужались бы до своего текста.
        let w = (syngui::viewport::viewport_size().get().width - 48.0).max(0.0);
        let mut col = Column::new().gap(8.0);
        for n in list.iter().rev().take(4) {
            col = col.child(
                DecoratedBox::new()
                    .child(
                        Column::new()
                            .gap(2.0)
                            .child(Text::new(n.summary.clone()).max_lines(1).class("lock-note-title"))
                            .child(Text::new(n.body.clone()).max_lines(2).class("lock-note-body")),
                    )
                    .class("lock-note")
                    .style("width", w),
            );
        }
        Box::new(col)
    });
    Column::new()
        .gap(28.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(clock)
        .child(notes)
        .class("lock-cover")
}

fn cover_bottom() -> impl Widget {
    Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(icon(mi::LOCK).class("lock-phone-lock"))
        .child(Text::new(t!("Проведите вверх, чтобы разблокировать")).class("lock-hint"))
}

fn pin_pad(ctx: ShellCtx) -> impl Widget {
    use syngui::GestureDetector;
    let (stage, pin) = phone_state();
    let st = state();
    let dots = crate::ui::rx(move || {
        let n = pin.get().chars().count();
        let checking = st.checking.get();
        let mut row = Row::new().gap(14.0).main_axis_alignment(MainAxisAlignment::Center);
        for i in 0..n.max(4) {
            let class = if i < n { "lock-dot lock-dot-on" } else { "lock-dot" };
            row = row.child(DecoratedBox::new().class(if checking { "lock-dot lock-dot-checking" } else { class }));
        }
        Box::new(row)
    });
    let error = crate::ui::rx(move || Box::new(Text::new(st.error.get()).class("lock-error")));
    let key = move |label: &'static str, sub: &'static str| {
        GestureDetector::new()
            .on_press(move |_| {
                if state().checking.get_untracked() {
                    return;
                }
                let mut p = pin.get_untracked();
                if p.len() < 32 {
                    p.push_str(label);
                    pin.set(p);
                }
            })
            .child(
                DecoratedBox::new()
                    .child(
                        Column::new()
                            .gap(0.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(Text::new(label).class("lock-key-digit"))
                            // Пустая подпись («1») — неразрывный пробел: цифра на одной высоте с остальными.
                            .child(Text::new(if sub.is_empty() { "\u{a0}" } else { sub }).class("lock-key-sub")),
                    )
                    .class("lock-key"),
            )
    };
    let action = move |glyph: &'static str, f: Box<dyn Fn() + Send + Sync>| {
        GestureDetector::new().on_press(move |_| f()).child(
            DecoratedBox::new()
                .child(crate::ui::vcenter(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(icon(glyph).class("lock-key-icon"))))
                .class("lock-key lock-key-action"),
        )
    };
    let mut grid = Grid::new(3).gap(14.0);
    for (d, sub) in [("1", ""), ("2", "ABC"), ("3", "DEF"), ("4", "GHI"), ("5", "JKL"), ("6", "MNO"), ("7", "PQRS"), ("8", "TUV"), ("9", "WXYZ")] {
        grid = grid.child(key(d, sub));
    }
    grid = grid
        .child(action("\u{E14A}", Box::new(move || {
            let mut p = pin.get_untracked();
            p.pop();
            pin.set(p);
        })))
        .child(key("0", "+"))
        .child(action("\u{E5CA}", Box::new(move || submit(pin.get_untracked()))));
    let _ = ctx;
    Column::new()
        .gap(22.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .main_axis_alignment(MainAxisAlignment::End)
        .child(Text::new(display_name()).class("lock-user"))
        .child(Text::new(t!("Введите PIN-код или пароль")).class("lock-hint"))
        .child(dots)
        .child(error)
        .child(grid.class("lock-pad"))
        .child(
            Row::new()
                .gap(12.0)
                .child(
                    GestureDetector::new()
                        .on_click(move || stage.set(Stage::Text))
                        .child(DecoratedBox::new().child(Text::new("ABC").class("lock-pill-text")).class("lock-pill")),
                )
                .child(
                    GestureDetector::new()
                        .on_click(move || stage.set(Stage::Cover))
                        .child(DecoratedBox::new().child(Text::new(t!("Отмена")).class("lock-pill-text")).class("lock-pill")),
                ),
        )
        .class("lock-pin")
}

fn text_entry() -> impl Widget {
    use syngui::widgets::input::on_screen_keyboard::{on_screen_keyboard, KeyboardLayout, KeyboardState};
    use syngui::GestureDetector;
    let (stage, pin) = phone_state();
    let st = state();
    let kb_state = KeyboardState::new("");
    let dots = crate::ui::rx(move || {
        let n = pin.get().chars().count();
        Box::new(Text::new(if n == 0 { t!("Пароль").to_string() } else { "•".repeat(n) }).class("lock-text-dots"))
    });
    let error = crate::ui::rx(move || Box::new(Text::new(st.error.get()).class("lock-error")));
    let kb = on_screen_keyboard(kb_state, KeyboardLayout::text_en_ru(&t!("Войти")))
        .gap(5.0)
        .stretch(44.0)
        .on_change(move |t| pin.set(t.to_string()))
        .on_submit(|t| submit(t.to_string()))
        .build();
    Column::new()
        .gap(16.0)
        .main_axis_alignment(MainAxisAlignment::End)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(display_name()).class("lock-user"))
        .child(DecoratedBox::new().child(dots).class("lock-text-box"))
        .child(error)
        .child(DecoratedBox::new().child(kb).class("lock-osk").style("width", (syngui::viewport::viewport_size().get_untracked().width - 48.0).clamp(280.0, 720.0)))
        .child(
            GestureDetector::new()
                .on_click(move || stage.set(Stage::Pin))
                .child(DecoratedBox::new().child(Text::new("123").class("lock-pill-text")).class("lock-pill")),
        )
        .class("lock-pin")
}
