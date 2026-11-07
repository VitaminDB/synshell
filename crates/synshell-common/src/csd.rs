//! Своя рамка окна (CSD) программ synshell на syngui: заголовок с местом для
//! перетаскивания, кнопки окна из настроек рабочего стола, рамка со скруглением
//! и изменение размера за края.
//!
//! Окно создаётся `App::frameless()` + `transparent(true)` с прозрачным фоном,
//! состояние окна — `with_window_state(сигнал)`. На телефоне
//! (`SYNSHELL_FORM_FACTOR=phone`, Android, iPhone) окна во весь экран: кнопок,
//! рамки и изменения размера нет; во весь экран — тоже нет.
//!
//! Классы MSS ([`MSS`], подключается [`crate::style::AppStyle::csd`]) — с
//! приставкой `csd-`; программа перекрывает их в своём MSS (цвет заголовка и т. п.).
//!
//! ```ignore
//! App::new().frameless().transparent(true).background(Color::from_srgb(0, 0, 0, 0.0)).with_window_state(win)
//! // корень: csd::frame(Column[csd::titlebar(win, …), содержимое])
//! ```

use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::widgets::Reactive;
use syngui::window::WindowState;

type W = Box<dyn Widget>;

/// Высота заголовка [`titlebar`] с нижней чертой, px (для раскладок, считающих высоту сами).
pub const TITLEBAR_H: f32 = 41.0;

/// Телефон: окна во весь экран, без кнопок окна и рамки.
pub fn is_phone() -> bool {
    cfg!(any(target_os = "android", target_os = "ios")) || std::env::var("SYNSHELL_FORM_FACTOR").is_ok_and(|v| v.trim().eq_ignore_ascii_case("phone"))
}

/// Показывать свой заголовок и кнопки окна: не телефон и не во весь экран
/// (читает сигнал — подписывает текущий Reactive).
pub fn decorated(win: RwSignal<WindowState>) -> bool {
    !is_phone() && !win.get().fullscreen
}

/// Сигнал «окно во весь экран» — меняется только с этим режимом (не с фокусом
/// и не с разворотом), чтобы раскладка не перестраивалась на каждый щелчок по окну.
pub fn fullscreen(win: RwSignal<WindowState>) -> RwSignal<bool> {
    let full = use_signal(win.get_untracked().fullscreen);
    create_effect(move || {
        let v = win.get().fullscreen;
        if full.get_untracked() != v {
            full.set(v);
        }
    });
    full
}

/// Кнопки окна (свернуть, развернуть, закрыть — набор и сторона из настроек
/// рабочего стола); на телефоне и во весь экран — ничего.
pub fn controls(win: RwSignal<WindowState>) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let w = win.get();
        if is_phone() || w.fullscreen {
            return vec![];
        }
        vec![Box::new(DecoratedBox::new().class("csd-controls").child(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused)))]
    }))
}

/// Место, за которое окно перетаскивают (и разворачивают двойным щелчком);
/// растягивается на свободную ширину строки.
pub fn drag<M>(child: impl syngui::IntoWidget<M>) -> W {
    Box::new(DecoratedBox::new().class("grow csd-drag").child(WindowDragRegion::new().child(DecoratedBox::new().class("csd-drag-space").child(child))))
}

/// Заголовок окна: значок и название (за них перетаскивают) и кнопки окна;
/// на телефоне и во весь экран его нет.
pub fn titlebar(win: RwSignal<WindowState>, title: impl Fn() -> String + Send + Sync + 'static, icon: Option<&'static str>) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        if !decorated(win) {
            return vec![];
        }
        let mut brand = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if let Some(glyph) = icon {
            brand = brand.child(Icon::new(glyph).class("csd-icon"));
        }
        brand = brand.child(Text::new(title()).max_lines(1).elide(Elide::End).class("csd-title"));
        vec![Box::new(DecoratedBox::new().class("csd-titlebar").child(Row::new().cross_axis_alignment(CrossAxisAlignment::Center).height(TITLEBAR_H - 1.0).child(drag(brand)).child(controls(win))))]
    }))
}

/// Рамка окна: скругление и обводка, изменение размера за края; на телефоне —
/// без рамки, развёрнутое и во весь экран — без скругления (`:window-maximized`).
pub fn frame<M>(child: impl syngui::IntoWidget<M>) -> W {
    let phone = is_phone();
    let frame = DecoratedBox::new().class(if phone { "csd-frame flat" } else { "csd-frame" }).clip(true).child(child);
    Box::new(WindowResizeRegion::new().over_content(true).inset(5.0).enabled(!phone).child(frame))
}

/// Стили рамки и заголовка (переменные палитры из `[appearance]`).
pub const MSS: &str = r#"
/* ── своя рамка окна (synshell_common::csd) ── */
.csd-frame { background-color: var(--bg); border-radius: var(--window-radius); border: 1px solid var(--border); }
.csd-frame.flat { border-radius: 0px; border-width: 0px; }
.csd-frame:window-maximized, .csd-frame:window-fullscreen { border-radius: 0px; border-width: 0px; }
.csd-titlebar { background-color: var(--surface); border-bottom: 1px solid var(--border); }
.csd-drag { height: 40px; }
.csd-drag-space { flex-grow: 1; height: 40px; min-width: 40px; padding: 0px 14px; align-items: center; }
.csd-icon { icon-size: 18px; icon-color: var(--accent); font-size: 18px; color: var(--accent); }
.csd-title { font-size: 14px; font-weight: 600; color: var(--fg); }
.csd-controls { padding: 0px 6px; align-items: center; justify-content: center; }
SystemWindowControls { color: var(--fg); background-color: var(--hover); }
"#;
