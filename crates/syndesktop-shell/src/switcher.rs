//! Переключатель окон (`shell window-switcher next|prev|commit`): карточки
//! окон текущего стола, выбор стрелками/повтором, Enter или commit —
//! активировать.

use syndesktop_common::config::Edge;
use syndesktop_common::ipc::{WindowInfo, WindowOp};
use syngui::input::{Key, MouseButton};
use syngui::prelude::*;

use crate::ctx::{Popup, PopupAnchor, PopupKind, ShellCtx};
use crate::ui::InputArea;

thread_local! {
    static SELECTED: std::cell::OnceCell<RwSignal<usize>> = const { std::cell::OnceCell::new() };
}

fn selected() -> RwSignal<usize> {
    SELECTED.with(|s| *s.get_or_init(|| use_signal(0usize)))
}

fn windows(ctx: &ShellCtx) -> Vec<WindowInfo> {
    let ws = ctx.workspaces.get_untracked().iter().find(|w| w.active).map(|w| w.index);
    let mut v: Vec<WindowInfo> = ctx
        .windows
        .get_untracked()
        .into_iter()
        .filter(|w| !w.skip_taskbar && (ws.is_none() || Some(w.workspace) == ws || w.sticky))
        .collect();
    // Окно в фокусе — первым.
    v.sort_by_key(|w| !w.focused);
    v
}

pub fn command(ctx: ShellCtx, arg: &str) {
    let open = ctx.popup.get_untracked().is_some_and(|p| p.kind == PopupKind::WindowSwitcher);
    let n = windows(&ctx).len();
    let sel = selected();
    match arg {
        "commit" => commit(ctx),
        "prev" | "next" if n > 0 => {
            let step = if arg == "next" { 1 } else { n - 1 };
            if open {
                sel.set((sel.get_untracked() + step) % n);
            } else {
                sel.set(step % n);
                ctx.popup.set(Some(Popup {
                    kind: PopupKind::WindowSwitcher,
                    anchor: PopupAnchor { output: None, rect: None, edge: Edge::Bottom },
                }));
            }
        }
        _ => {}
    }
}

fn commit(ctx: ShellCtx) {
    let list = windows(&ctx);
    if let Some(w) = list.get(selected().get_untracked()) {
        crate::actions::window_op(w.id, WindowOp::Activate);
    }
    ctx.close_popup();
}

pub fn view(ctx: ShellCtx) -> impl Widget {
    let sel = selected();
    crate::popup::set_key_handler(move |k| {
        if !k.pressed {
            // Отпустили Alt — выбрать (как Alt+Tab).
            if matches!(k.key, Key::Alt | Key::Meta) {
                commit(ShellCtx::get());
                return true;
            }
            return false;
        }
        let n = windows(&ShellCtx::get()).len().max(1);
        match k.key {
            Key::Right | Key::Tab | Key::Down => {
                sel.set((sel.get_untracked() + 1) % n);
                true
            }
            Key::Left | Key::Up => {
                sel.set((sel.get_untracked() + n - 1) % n);
                true
            }
            Key::Enter | Key::Space => {
                commit(ShellCtx::get());
                true
            }
            _ => false,
        }
    });
    Column::new().gap(10.0).child(move || {
        let cur = sel.get();
        let list = windows(&ctx);
        let mut flex = Flex::new().direction(FlexDirection::Row).wrap().gap(8.0);
        for (i, w) in list.into_iter().enumerate() {
            let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
            if let Some(p) = crate::xdg::window_icon(&w.app_id) {
                col = col.child(Image::new(p.to_string_lossy()).fit(ImageFit::Contain).placeholder(false).class("switch-icon"));
            }
            let title = if w.title.is_empty() { w.app_id.clone() } else { w.title.clone() };
            col = col.child(Text::new(title).max_lines(2).class("switch-title"));
            let id = w.id;
            flex = flex.child(
                InputArea::new(DecoratedBox::new().child(col).class(if i == cur { "switch-card switch-card-selected" } else { "switch-card" }))
                    .pointer()
                    .on_hover(move |h| {
                        if h {
                            sel.set(i);
                        }
                    })
                    .on_click(move |b, _, _| {
                        if b == MouseButton::Left {
                            crate::actions::window_op(id, WindowOp::Activate);
                            ShellCtx::get().close_popup();
                        }
                    }),
            );
        }
        flex
    })
}
