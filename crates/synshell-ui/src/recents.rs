//! «Недавние» (телефон): лента карточек открытых окон поверх всего —
//! жест снизу с задержкой (`shell recents`). Тап по карточке — перейти к
//! окну, свайп карточки вверх — закрыть окно, «Закрыть все» — все окна.
//! Карточки въезжают и съезжаются (`Keyed` + `AnimatedPosition`), закрытая
//! улетает вверх (`Presence`).

use std::cell::Cell;
use synshell_common::ipc::{WindowInfo, WindowOp};
use syngui::prelude::*;
use syngui::widgets::{Motion, PanAxis, Presence, SwipeDirection};
use syngui::containers::Keyed;
use syngui::GestureDetector;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceId, SurfaceSpec};

use crate::ctx::ShellCtx;
use crate::launchers;
use crate::ui::{icon, mi, rx};

thread_local! {
    static SURFACE: Cell<Option<SurfaceId>> = const { Cell::new(None) };
    static OPEN: Cell<Option<RwSignal<bool>>> = const { Cell::new(None) };
    /// Окна, которые уже смахнули (уходят с анимацией, ждут закрытия).
    static GONE: Cell<Option<RwSignal<Vec<u64>>>> = const { Cell::new(None) };
}

pub fn is_open() -> bool {
    SURFACE.with(|s| s.get()).is_some()
}

pub fn toggle() {
    if is_open() {
        close();
    } else {
        open();
    }
}

pub fn close() {
    let ctx = ShellCtx::get();
    match OPEN.with(|o| o.get()) {
        Some(s) if crate::anim::group_ms(&ctx, "pages", 100) > 0 => s.set(false),
        _ => close_now(),
    }
}

fn close_now() {
    if let Some(id) = SURFACE.with(|s| s.take()) {
        syngui_layer::close_surface(id);
    }
    OPEN.with(|o| o.set(None));
}

pub(crate) fn visible_windows(ctx: &ShellCtx) -> Vec<WindowInfo> {
    ctx.windows.get().into_iter().filter(|w| !w.skip_taskbar && !w.app_id.is_empty() || !w.title.is_empty()).collect()
}

pub fn open() {
    if is_open() {
        return;
    }
    let ctx = ShellCtx::get();
    ctx.close_popup();
    crate::shade::close();
    // Экранная клавиатура поверх ленты/шторки не нужна.
    crate::actions::spawn("synkeyboard hide");
    let open = use_signal(true);
    OPEN.with(|o| o.set(Some(open)));
    GONE.with(|g| {
        if g.get().is_none() {
            g.set(Some(use_signal(Vec::new())));
        }
    });
    let spec = SurfaceSpec {
        namespace: "syndesktop-recents".into(),
        layer: Layer::Overlay,
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        size: (0, 0),
        margin: [0; 4],
        exclusive_zone: -1,
        keyboard: KeyboardInteractivity::OnDemand,
        output: crate::manager::focused_output(&ctx),
        auto_size: false,
        clear_color: [0.0; 4],
    };
    let id = syngui_layer::create_surface(spec, move || Box::new(view(ShellCtx::get(), open)));
    SURFACE.with(|s| s.set(Some(id)));
}

fn gone() -> RwSignal<Vec<u64>> {
    GONE.with(|g| g.get()).unwrap_or_else(|| use_signal(Vec::new()))
}

fn view(ctx: ShellCtx, open: RwSignal<bool>) -> impl Widget {
    let dur = crate::anim::group_ms(&ctx, "pages", 260);
    Presence::signal(open, move || {
        let ctx = ShellCtx::get();
        let cards = rx(move || {
            let gone = gone().get();
            let list: Vec<WindowInfo> = visible_windows(&ctx).into_iter().filter(|w| !gone.contains(&w.id)).collect();
            if list.is_empty() {
                return Box::new(
                    Column::new()
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Text::new("Нет открытых приложений").class("recents-empty"))
                        .class("recents-empty-box"),
                ) as Box<dyn Widget>;
            }
            let mut row = Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center);
            for w in list {
                row = row.child(Keyed::new(w.id, 0, move || Box::new(AnimatedPosition::new(card(w.clone())))));
            }
            // Высота ленты — по карточкам: растянутая лента выталкивала
            // «Закрыть все» за нижний край.
            // ScrollView в колонке забирает всю высоту — держим его в блоке
            // высотой с карточку.
            Box::new(DecoratedBox::new().child(ScrollView::new().horizontal().child(row.class("recents-row"))).class("recents-scroll"))
        });
        let clear = GestureDetector::new()
            .on_click(|| {
                let ctx = ShellCtx::get();
                for w in visible_windows(&ctx) {
                    crate::actions::window_op(w.id, WindowOp::Close);
                }
                close();
            })
            .child(
                DecoratedBox::new()
                    .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(mi::CLEAR_ALL).class("recents-clear-icon")).child(Text::new("Закрыть все").class("recents-clear-text")))
                    .class("recents-clear"),
            );
        Box::new(
            GestureDetector::new().on_click(close).child(
                Column::new()
                    .gap(18.0)
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new("Недавние").class("recents-title"))
                    .child(cards)
                    .child(clear)
                    .class("recents")
                    // Presence меряет по содержимому — затемнение на весь экран.
                    .style("height", syngui::viewport::viewport_size().get_untracked().height),
            ),
        )
    })
    .enter(Motion::fade().scale(1.06))
    .exit(Motion::fade().scale(1.04))
    .duration_ms(dur)
    .initial(dur > 0)
    .on_exit_complete(close_now)
}

fn card(w: WindowInfo) -> impl Widget {
    let entry = crate::xdg::app_for_window(&w.app_id);
    let name = entry.as_ref().map(|e| e.name.clone()).unwrap_or_else(|| w.app_id.clone());
    let icon_path = entry.as_ref().and_then(|e| crate::xdg::lookup_icon(&e.icon)).or_else(|| crate::xdg::window_icon(&w.app_id));
    let id = w.id;
    let leaving = use_signal(true);
    let dur = crate::anim::group_ms(&ShellCtx::get(), "pages", 240);
    let title = w.title.clone();
    let body = move || -> Box<dyn Widget> {
        Box::new(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(
                            Row::new()
                                .gap(8.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(launchers::icon_widget(&icon_path, &None, "recents-app-icon", 28.0))
                                .child(Text::new(name.clone()).max_lines(1).class("recents-app-name")),
                        )
                        .child(
                            DecoratedBox::new()
                                .child(
                                    Column::new()
                                        .main_axis_alignment(MainAxisAlignment::Center)
                                        .cross_axis_alignment(CrossAxisAlignment::Center)
                                        .gap(10.0)
                                        .child(launchers::icon_widget(&icon_path, &None, "recents-big-icon", 72.0))
                                        .child(Text::new(title.clone()).max_lines(3).class("recents-window-title")),
                                )
                                .class("recents-preview"),
                        ),
                )
                .class("recents-card"),
        )
    };
    let presence = Presence::signal(leaving, body)
        .exit(Motion::fade().slide(0.0, -260.0))
        .duration_ms(dur)
        .initial(false)
        .on_exit_complete(move || {
            crate::actions::window_op(id, WindowOp::Close);
            let g = gone();
            let mut v = g.get_untracked();
            v.push(id);
            g.set(v);
        });
    GestureDetector::new()
        .pan_axis(PanAxis::Vertical)
        .on_click(move || {
            crate::actions::window_op(id, WindowOp::Activate);
            close();
        })
        .on_swipe(move |dir, _| {
            if dir == SwipeDirection::Up {
                let g = gone();
                if dur == 0 {
                    crate::actions::window_op(id, WindowOp::Close);
                    let mut v = g.get_untracked();
                    v.push(id);
                    g.set(v);
                } else {
                    leaving.set(false);
                }
            }
        })
        .child(presence)
}
