//! Экран машины: кадр гостя во всю область, мышь и клавиатура уходят гостю;
//! сверху — плавающая панель (сочетания клавиш, буфер обмена, снимок, полный экран).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::{LiveInput, LiveView};
use syngui::GestureDetector;
use syngui::CursorIcon;

use super::parts::{self, boxed, btn, icon_btn, W};
use crate::keymap;
use crate::state::{self, icons, Power, St};

static CTRL: AtomicBool = AtomicBool::new(false);
static ALT: AtomicBool = AtomicBool::new(false);
static RESIZE_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn page(st: St, full: bool) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let Some(id) = st.sel.get() else { return vec![] };
        let power = st.live.get().get(&id).map(|l| l.power).unwrap_or_default();
        let busy = st.busy.get().get(&id).cloned();
        let connected = st.connected.get();
        let Some(vm) = st.vm(&id) else { return vec![] };
        if !power.on() {
            return vec![off(st, &vm, busy)];
        }
        let Some(sess) = state::session(&id).filter(|_| connected) else {
            return vec![waiting(t!("Подключение к экрану…"))];
        };
        let s = sess.clone();
        let view = Reactive::new(move || -> Vec<W> {
            st.frame_tick.get();
            let s2 = s.clone();
            let id2 = id.clone();
            vec![boxed(
                LiveView::new(s.frame.clone())
                    .cursor(CursorIcon::None)
                    .on_input(move |e| input(st, &id2, &s2, e))
                    .on_resize(move |sz| on_resize(st, sz))
                    .class("screen"),
            )]
        });
        let mut stack = Stack::new().fit(StackFit::Expand).child(view);
        if power == Power::Paused {
            stack = stack.child(paused_veil(st, &vm.id));
        }
        stack = stack.child(toolbar(st, &vm.id, full));
        vec![boxed(DecoratedBox::new().child(stack).class(if full { "screen-wrap full" } else { "screen-wrap" }))]
    }))
}

/// Ввод над кадром → гостю.
fn input(st: St, id: &str, s: &crate::vnc::Session, e: LiveInput) {
    match e {
        LiveInput::Move { x, y } => s.pointer(x, y),
        LiveInput::Down { button, x, y } => {
            if let Some(i) = button_index(button) {
                if i == 0 {
                    state::offer_clipboard(id);
                }
                s.button(i, true, x, y);
            }
            if st.keys_menu.get_untracked() {
                st.keys_menu.set(false);
            }
        }
        LiveInput::Up { button, x, y } => {
            if let Some(i) = button_index(button) {
                s.button(i, false, x, y);
            }
        }
        LiveInput::Wheel { dx, dy, .. } => s.wheel(dx, dy),
        LiveInput::TouchStart { x, y, .. } => {
            s.pointer(x, y);
            s.button(0, true, x, y);
        }
        LiveInput::TouchMove { x, y, .. } => s.pointer(x, y),
        LiveInput::TouchEnd { x, y, .. } => s.button(0, false, x, y),
        LiveInput::Key { key, pressed } => {
            match key {
                Key::Ctrl => CTRL.store(pressed, Ordering::Relaxed),
                Key::Alt => ALT.store(pressed, Ordering::Relaxed),
                _ => {}
            }
            // Ctrl+Alt+F — полный экран, Ctrl+Alt+V — вставить текст хоста набором.
            if pressed && CTRL.load(Ordering::Relaxed) && ALT.load(Ordering::Relaxed) {
                match key {
                    Key::F => {
                        // Модификаторы гостю — отпущенными, иначе залипнут.
                        s.key(0x1d, 0, false);
                        s.key(0x38, 0, false);
                        toggle_full(st);
                        return;
                    }
                    Key::V => {
                        s.key(0x1d, 0, false);
                        s.key(0x38, 0, false);
                        type_clipboard(id);
                        return;
                    }
                    _ => {}
                }
            }
            let code = syngui::input::current_scancode().or_else(|| keymap::evdev_of(key)).and_then(keymap::qnum);
            if let Some(q) = code {
                s.key(q, keymap::keysym_of(key), pressed);
            } else {
                let ks = keymap::keysym_of(key);
                if ks != 0 {
                    s.key(0, ks, pressed);
                }
            }
        }
        LiveInput::Char(_) => {}
    }
}

fn button_index(b: MouseButton) -> Option<u8> {
    match b {
        MouseButton::Left => Some(0),
        MouseButton::Middle => Some(1),
        MouseButton::Right => Some(2),
        _ => None,
    }
}

/// Область экрана изменилась: через полсекунды покоя — разрешение гостя под неё.
fn on_resize(st: St, sz: Size) {
    let k = syngui::scale::pixel_ratio();
    let w = (sz.width * k).round() as u32;
    let h = (sz.height * k).round() as u32;
    if st.view_size.get_untracked() == (w, h) {
        return;
    }
    st.view_size.set((w, h));
    let seq = RESIZE_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(450));
        if RESIZE_SEQ.load(Ordering::Relaxed) == seq {
            run_on_main_thread(move || state::fit_guest(st));
        }
    });
}

pub fn toggle_full(st: St) {
    let on = !st.fullscreen.get_untracked();
    st.fullscreen.set(on);
    syngui::signal::set_fullscreen(on);
}

fn type_clipboard(id: &str) {
    if let (Some(s), Some(text)) = (state::session(id), syngui::clipboard::paste()) {
        s.type_text(&text);
    }
}

fn combo(id: &str, keys: &[u32]) {
    if let Some(s) = state::session(id) {
        s.combo(keys);
    }
}

fn toolbar(st: St, id: &str, full: bool) -> W {
    let (a, b, c, d) = (id.to_string(), id.to_string(), id.to_string(), id.to_string());
    let auto = st.vm(id).map(|v| v.auto_resize).unwrap_or(true);
    let size = st.guest_size.get_untracked();
    let bar = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(icon_btn(icons::KEYBOARD, t!("Ctrl+Alt+Del"), move || combo(&a, keymap::CTRL_ALT_DEL)))
        .child(icon_btn(icons::MORE, t!("Другие сочетания"), move || st.keys_menu.set(!st.keys_menu.get_untracked())))
        .child(DecoratedBox::new().class("vsep"))
        .child(icon_btn(icons::PASTE, t!("Напечатать текст из буфера (Ctrl+Alt+V)"), move || type_clipboard(&b)))
        .child(icon_btn(icons::CAMERA, t!("Снимок экрана"), move || state::screenshot(st, &c)))
        .child(icon_btn(
            icons::FIT,
            if auto { t!("Разрешение под окно: включено") } else { t!("Разрешение под окно: выключено") },
            move || {
                state::edit(st, |v| v.auto_resize = !v.auto_resize);
                state::fit_guest(st);
                let on = st.vm(&d).map(|v| v.auto_resize).unwrap_or(false);
                state::toast(st, if on { t!("Разрешение гостя подстраивается под окно") } else { t!("Разрешение гостя больше не меняется") });
            },
        ))
        .child(Text::new(if size.0 > 0 { format!("{}×{}", size.0, size.1) } else { String::new() }).class("res-label"))
        .child(DecoratedBox::new().class("vsep"))
        .child(icon_btn(if full { icons::FULLSCREEN_EXIT } else { icons::FULLSCREEN }, t!("Во весь экран (Ctrl+Alt+F)"), move || toggle_full(st)));
    let bar = DecoratedBox::new().child(bar).class(if auto { "float-bar auto" } else { "float-bar" });
    let id2 = id.to_string();
    let menu = Reactive::new(move || -> Vec<W> {
        if !st.keys_menu.get() {
            return vec![];
        }
        let item = |label: &str, keys: &'static [u32]| -> W {
            let id = id2.clone();
            boxed(
                GestureDetector::new()
                    .cursor(CursorIcon::Pointer)
                    .on_click(move || {
                        combo(&id, keys);
                        st.keys_menu.set(false);
                    })
                    .child(DecoratedBox::new().child(Text::new(label.to_string()).class("menu-label")).class("menu-item")),
            )
        };
        vec![boxed(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .gap(2.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(item("Ctrl+Alt+Backspace", keymap::CTRL_ALT_BACKSPACE))
                        .child(item("Ctrl+Alt+F1", keymap::CTRL_ALT_F1))
                        .child(item("Ctrl+Alt+F2", keymap::CTRL_ALT_F2))
                        .child(item("Ctrl+Alt+F7", keymap::CTRL_ALT_F7))
                        .child(item("Alt+Tab", keymap::ALT_TAB))
                        .child(item("Alt+F4", keymap::ALT_F4))
                        .child(item("Super", keymap::SUPER))
                        .child(item("Print Screen", keymap::PRINT)),
                )
                .class("menu"),
        )]
    });
    boxed(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(bar)
            .child(menu)
            .class("float-wrap"),
    )
}

fn paused_veil(st: St, id: &str) -> W {
    let id = id.to_string();
    boxed(
        DecoratedBox::new()
            .child(
                Column::new()
                    .gap(14.0)
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(icons::PAUSE).class("veil-icon"))
                    .child(Text::new(t!("Машина на паузе")).class("veil-title"))
                    .child(btn(icons::PLAY, t!("Продолжить"), "primary", move || state::toggle_pause(st, &id))),
            )
            .class("veil"),
    )
}

fn waiting(text: String) -> W {
    boxed(
        Column::new()
            .gap(14.0)
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(CircularProgress::new().indeterminate().size(36.0).stroke_width(3.5))
            .child(Text::new(text).class("muted"))
            .class("screen-wrap"),
    )
}

/// Машина выключена: последний кадр под вуалью и большая кнопка.
fn off(st: St, vm: &crate::model::Vm, busy: Option<String>) -> W {
    let id = vm.id.clone();
    let mut stack = Stack::new().fit(StackFit::Expand);
    if let Some(f) = state::thumb(vm) {
        stack = stack.child(DecoratedBox::new().child(LiveView::new(f).class("screen")).style("opacity", StyleValue::Number(0.35)));
    }
    let action: W = match busy {
        Some(b) => boxed(
            Column::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(CircularProgress::new().indeterminate().size(40.0).stroke_width(3.5))
                .child(Text::new(b).class("veil-title")),
        ),
        None => boxed(
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(
                    GestureDetector::new()
                        .cursor(CursorIcon::Pointer)
                        .on_click(move || state::start(st, &id))
                        .child(DecoratedBox::new().child(Icon::new(icons::PLAY).class("big-play-icon")).class("big-play")),
                )
                .child(Text::new(t!("Машина выключена")).class("veil-title"))
                .child(Text::new(if vm.cdrom.is_empty() { t!("Нажмите, чтобы запустить") } else { t!("В приводе: {f}", f = parts::file_name(&vm.cdrom)) }).max_lines(2).class("veil-sub")),
        ),
    };
    stack = stack.child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(action));
    boxed(DecoratedBox::new().child(stack).class("screen-wrap"))
}
