//! Своё окно (CSD): кнопки окна из настроек рабочего стола — в шапке библиотеки и «Сейчас играет» (за
//! название окно перетаскивается), изменение размера за край, полноэкранный режим: кнопка в шапке, F11,
//! Esc — выход. Выбор запоминается (`~/.config/synshell/audio-player.toml`); на телефоне — по умолчанию
//! во весь экран; в окне на телефоне — кнопки окна есть, рамки и изменения размера нет.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::widgets::*;

use crate::ui::{icon_button, icons, St};

type W = Box<dyn Widget>;

/// Телефон: окно на весь экран, без своего заголовка.
pub fn is_phone() -> bool {
    std::env::var("SYNSHELL_FORM_FACTOR").is_ok_and(|v| v == "phone")
}

fn prefs_file() -> PathBuf {
    synshell_common::paths::config_dir().join("audio-player.toml")
}

/// Выбор пользователя из файла настроек (`fullscreen = true|false`).
fn saved_fullscreen() -> Option<bool> {
    let s = std::fs::read_to_string(prefs_file()).ok()?;
    s.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == "fullscreen").then(|| v.trim() == "true")
    })
}

fn save_fullscreen(on: bool) {
    let f = prefs_file();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Err(e) = std::fs::write(&f, format!("fullscreen = {on}\n")) {
        tracing::warn!("{}: {e}", f.display());
    }
}

/// Полноэкранный режим при запуске: выбор пользователя, иначе — на телефоне.
pub fn start_fullscreen() -> bool {
    saved_fullscreen().unwrap_or_else(is_phone)
}

/// Во весь экран или в окно; выбор запоминается.
pub fn set_fullscreen(st: St, on: bool) {
    if st.fs.get_untracked() != on {
        st.fs.set(on);
    }
    syngui::signal::set_fullscreen(on);
    save(on);
}

fn save(on: bool) {
    if saved_fullscreen() != Some(on) {
        save_fullscreen(on);
    }
}

pub fn toggle_fullscreen(st: St) {
    set_fullscreen(st, !st.fs.get_untracked());
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
        if st.fs.get_untracked() != v {
            st.fs.set(v);
        }
        save(v);
    });
}

/// Есть ли кнопки окна: везде, где окнами управляет synwm (и в телефонном форм-факторе);
/// нет только на Android/iPhone.
pub fn has_controls() -> bool {
    !cfg!(any(target_os = "android", target_os = "ios"))
}

/// Кнопки окна (свернуть, развернуть, закрыть); во весь экран их нет.
pub fn controls(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        if !has_controls() || st.fs.get() {
            return vec![];
        }
        let w = st.window.get();
        vec![Box::new(DecoratedBox::new().class("window-controls").child(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused)))]
    }))
}

/// Растягивающаяся часть шапки, за которую окно перетаскивается.
pub fn drag(child: impl Widget + 'static) -> W {
    if !has_controls() {
        return Box::new(DecoratedBox::new().class("grow drag").child(child));
    }
    Box::new(DecoratedBox::new().class("grow drag").child(WindowDragRegion::new().child(DecoratedBox::new().class("drag-space").child(child))))
}

/// Кнопка «во весь экран» в шапке.
pub fn fullscreen_button(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let (icon, tip) = if st.fs.get() {
            (icons::FULLSCREEN_EXIT, t!("Выйти из полноэкранного режима (F11)"))
        } else {
            (icons::FULLSCREEN, t!("Во весь экран (F11)"))
        };
        vec![icon_button(icon, &tip, move || toggle_fullscreen(st))]
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
                Key::Escape if st.fs.get_untracked() => {
                    set_fullscreen(st, false);
                    KeyReply::Handled
                }
                _ => KeyReply::Ignore,
            }
        })
        .child(inner);
    let frame = DecoratedBox::new().class(if phone { "window-frame flat" } else { "window-frame" }).clip(true).child(keys);
    Box::new(WindowResizeRegion::new().over_content(true).inset(5.0).enabled(!phone).child(frame))
}
