//! Своё окно (CSD): кнопки окна из настроек рабочего стола — в шапке библиотеки, плеера и монтажа
//! (за название окно перетаскивается), изменение размера за край, полноэкранный режим: кнопка в шапке
//! и в плеере, F11, Esc — выход. Выбор запоминается (`Settings::fullscreen`); на телефоне — по
//! умолчанию во весь экран, без кнопок окна и рамки.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::widgets::*;

use crate::settings::Settings;
use crate::ui::{icon_button, icons, St};

type W = Box<dyn Widget>;

/// Телефон: окно на весь экран, без своего заголовка.
pub fn is_phone() -> bool {
    std::env::var("SYNSHELL_FORM_FACTOR").is_ok_and(|v| v == "phone")
}

/// Полноэкранный режим при запуске: выбор пользователя, иначе — на телефоне.
pub fn start_fullscreen(s: &Settings) -> bool {
    s.fullscreen.unwrap_or_else(is_phone)
}

/// Окно уходит из полноэкранного режима не по выбору пользователя (монтаж) — не запоминать.
static NO_SAVE: AtomicBool = AtomicBool::new(false);

/// Во весь экран или в окно; `remember` — запомнить как выбор пользователя.
pub fn set_fullscreen(st: St, on: bool, remember: bool) {
    if st.fullscreen.get_untracked() != on {
        st.fullscreen.set(on);
        if !remember {
            NO_SAVE.store(true, Ordering::Relaxed);
        }
    }
    syngui::signal::set_fullscreen(on);
    if remember {
        save(st, on);
    }
}

fn save(st: St, on: bool) {
    if st.settings.with_untracked(|s| s.fullscreen != Some(on)) {
        crate::player::update(st, |s| s.fullscreen = Some(on));
    }
}

pub fn toggle_fullscreen(st: St) {
    set_fullscreen(st, !st.fullscreen.get_untracked(), true);
}

/// Окно сменило режим (F11 — её ловит сам syngui, кнопка, композитор): признак следует за окном,
/// новый режим запоминается. Первое значение сигнала — ещё не от окна, его пропускаем.
pub fn follow_window(st: St) {
    let last: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
    create_effect(move || {
        let v = st.window.get().fullscreen;
        let prev = last.replace(Some(v));
        if prev.is_none() || prev == Some(v) {
            return;
        }
        if st.fullscreen.get_untracked() != v {
            st.fullscreen.set(v);
        }
        if !NO_SAVE.swap(false, Ordering::Relaxed) {
            save(st, v);
        }
    });
}

/// Заголовок с кнопками окна: не на телефоне.
pub fn has_controls() -> bool {
    !is_phone()
}

/// Место боковой панели (субтитры, фильтры, настройки…): в окне — ниже строки с кнопками
/// окна, чтобы панель их не закрывала; во весь экран и на телефоне — во всю высоту.
pub fn panel_place(st: St) -> &'static str {
    if has_controls() && !st.fullscreen.get() {
        "vp-panel-place vp-panel-windowed"
    } else {
        "vp-panel-place"
    }
}

/// Кнопки окна (свернуть, развернуть, закрыть); во весь экран их нет.
pub fn controls(st: St, class: &'static str) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        if !has_controls() || st.fullscreen.get() {
            return vec![];
        }
        let w = st.window.get();
        vec![Box::new(DecoratedBox::new().class(class).child(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused)))]
    }))
}

/// Растягивающаяся часть шапки, за которую окно перетаскивается.
pub fn drag(child: impl Widget + 'static) -> W {
    if !has_controls() {
        return Box::new(DecoratedBox::new().class("grow drag").child(child));
    }
    Box::new(DecoratedBox::new().class("grow drag").child(WindowDragRegion::new().child(DecoratedBox::new().class("drag-space").child(child))))
}

/// Кнопка «во весь экран» в шапке библиотеки (`player` — белая, в шапке плеера).
pub fn fullscreen_button(st: St, player: bool) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let on = st.fullscreen.get();
        let (icon, tip) = if on {
            (icons::FULLSCREEN_EXIT, t!("Выйти из полноэкранного режима (F11)"))
        } else {
            (icons::FULLSCREEN, t!("Во весь экран (F11)"))
        };
        vec![if player {
            Box::new(ToolButton::new(icon).tooltip(tip).on_click(move || toggle_fullscreen(st)).class("vp-btn"))
        } else {
            icon_button(icon, &tip, move || toggle_fullscreen(st))
        }]
    }))
}

/// Рамка окна, изменение размера за край, Esc — выход из полноэкранного режима.
pub fn frame(st: St, inner: impl Widget + 'static) -> W {
    let phone = is_phone();
    let keys = EventHook::new()
        .on_key_down(move |k, m| {
            if m.ctrl || m.alt || m.meta {
                return KeyReply::Ignore;
            }
            // F11 переключает сам syngui (до приложения); выбор запоминает `follow_window`
            match k {
                Key::Escape if st.fullscreen.get_untracked() => {
                    set_fullscreen(st, false, true);
                    KeyReply::Handled
                }
                _ => KeyReply::Ignore,
            }
        })
        .child(inner);
    let frame = DecoratedBox::new().class(if phone { "window-frame flat" } else { "window-frame" }).clip(true).child(keys);
    Box::new(WindowResizeRegion::new().over_content(true).inset(5.0).enabled(!phone).child(frame))
}
