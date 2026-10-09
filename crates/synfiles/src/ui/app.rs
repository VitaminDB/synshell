//! Корень окна: заголовок с вкладками, навигация, командная панель,
//! боковая панель и панели файлов, строка состояния, слои поверх.

use syngui::prelude::*;
use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::widgets::*;

use super::{boxed, bx, icons, W};
use crate::loc::Location;
use crate::model;
use crate::state::{self, Tab, ToastKind};

fn tab_chip(i: usize, t: Tab, current: bool) -> W {
    let loc = t.pane_tracked().loc.get();
    let title = loc.title();
    let glyph = loc.icon();
    let close = GestureDetector::new()
        .on_click(move || state::close_tab(i))
        .child(DecoratedBox::new().class("tab-close").child(Icon::new(icons::CLOSE).class("icon")));
    boxed(
        GestureDetector::new()
            .on_click(move || state::ctx().cur.set(i))
            .on_middle_click(move |_| state::close_tab(i))
            .child(
                DecoratedBox::new().class(if current { "tab current" } else { "tab" }).child(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Icon::new(glyph).class("icon tab-icon"))
                        .child(Text::new(title).max_lines(1).class("tab-title grow"))
                        .child(close),
                ),
            ),
    )
}

/// Заголовок окна: вкладки (как в Проводнике Windows 11) и кнопки окна.
fn titlebar() -> W {
    let tabs = Reactive::new(move || -> Vec<W> {
        let ctx = state::ctx();
        let tabs = ctx.tabs.get();
        let cur = ctx.cur.get();
        let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::End);
        for (i, t) in tabs.iter().enumerate() {
            row = row.child(tab_chip(i, *t, i == cur));
        }
        row = row.child(
            GestureDetector::new()
                .on_click(|| {
                    let loc = state::pane().loc.get_untracked();
                    state::new_tab(loc, true);
                })
                .child(DecoratedBox::new().class("new-tab").child(Icon::new(icons::ADD).class("icon"))),
        );
        vec![boxed(row)]
    });
    let controls = Reactive::new(move || -> Vec<W> {
        let w = state::ctx().window.get();
        vec![boxed(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    bx(
        "titlebar",
        Row::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(DecoratedBox::new().class("tabs").child(tabs))
            .child(DecoratedBox::new().class("grow drag-wrap").child(WindowDragRegion::new().child(DecoratedBox::new().class("drag-space"))))
            .child(DecoratedBox::new().class("window-controls").child(controls)),
    )
}

fn panes() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let t = state::tab_tracked();
        if t.split.get() {
            vec![boxed(
                SplitView::new(
                    DecoratedBox::new().class("pane-wrap").child(super::view::pane_view(t, 0)),
                    DecoratedBox::new().class("pane-wrap").child(super::view::pane_view(t, 1)),
                )
                .initial_ratio(0.5)
                .min_size(220.0)
                .class("split grow"),
            )]
        } else {
            vec![super::view::pane_view(t, 0)]
        }
    }))
}

fn status_bar() -> W {
    let info = Reactive::new(move || -> Vec<W> {
        let p = state::tab_tracked().pane_tracked();
        let entries = p.entries.get();
        let sel = p.sel.get();
        let loc = p.loc.get();
        let mut left = t!("Элементов: {n}", n = entries.len());
        if !sel.selected.is_empty() {
            let chosen: Vec<&model::Entry> = sel.selected.iter().filter_map(|&i| entries.get(i)).collect();
            let files: Vec<&&model::Entry> = chosen.iter().filter(|e| !e.is_dir).collect();
            left.push_str(&t!("   Выбрано: {n}", n = chosen.len()));
            if !files.is_empty() {
                let bytes: u64 = files.iter().map(|e| e.size).sum();
                left.push_str(&format!(" ({})", model::format_size(bytes)));
            }
        }
        let right = match &loc {
            Location::Dir(d) => crate::places::space(d)
                .map(|(free, total)| t!("Свободно {v} из {v2}", v = model::format_size(free), v2 = model::format_size(total)))
                .unwrap_or_default(),
            _ => String::new(),
        };
        vec![boxed(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(left).max_lines(1).class("status-text"))
                .child(DecoratedBox::new().class("grow"))
                .child(Text::new(right).max_lines(1).class("status-text dim")),
        )]
    });
    let views = Reactive::new(move || -> Vec<W> {
        let p = state::tab_tracked().pane_tracked();
        let view = p.view.get();
        let seg = |v: state::ViewMode, glyph: &str, tip: &str| -> W {
            super::icon_button(glyph, tip, if view == v { "toggled small" } else { "small" }, true, move || crate::actions::set_view(p, v))
        };
        vec![boxed(
            Row::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(seg(state::ViewMode::Details, icons::VIEW_DETAILS, &t!("Таблица (Ctrl+4)")))
                .child(seg(state::ViewMode::Icons, icons::VIEW_ICONS, &t!("Значки (Ctrl+1)"))),
        )]
    });
    bx(
        "status-bar",
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class("grow").child(info))
            .child(zoom_bar())
            .child(views),
    )
}

/// Масштаб значков: «−», ползунок, «+», размер в пикселях. Ползунок
/// пересобирается при смене вкладки и при масштабе клавишами, но не от
/// собственного перетаскивания (см. `Ctx::zoom_rev`).
fn zoom_bar() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let p = state::tab_tracked().pane_tracked();
        let _ = state::ctx().zoom_rev.get();
        let px = p.icon_size.get_untracked();
        let label = Reactive::new(move || -> Vec<W> {
            vec![boxed(Text::new(format!("{} px", p.icon_size.get())).max_lines(1).class("status-text dim zoom-value"))]
        });
        vec![boxed(
            Row::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(super::icon_button(icons::ZOOM_OUT, &t!("Мельче (Ctrl+−)"), "small", true, move || crate::actions::zoom(p, -1)))
                .child(
                    Slider::new()
                        .range(32.0, 256.0)
                        .step(8.0)
                        .value(px as f32)
                        .width(140.0)
                        .on_change(move |v| crate::actions::set_icon_size(p, v.round() as u32))
                        .class("zoom-slider"),
                )
                .child(super::icon_button(icons::ZOOM_IN, &t!("Крупнее (Ctrl++)"), "small", true, move || crate::actions::zoom(p, 1)))
                .child(DecoratedBox::new().class("zoom-label").child(label)),
        )]
    }))
}

pub(super) fn toast_layer() -> W {
    let toast = Reactive::new(move || -> Vec<W> {
        let Some(t) = state::ctx().toast.get() else { return vec![] };
        let mut row = Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(if t.kind == ToastKind::Error { icons::ERROR } else { icons::INFO }).class("icon"))
            .child(Text::new(t.text).max_lines(2).class("toast-text"));
        if t.undo {
            row = row.child(
                GestureDetector::new()
                    .on_click(|| {
                        state::ctx().toast.set(None);
                        crate::actions::undo();
                    })
                    .child(DecoratedBox::new().class("toast-action").child(Text::new(t!("Отменить")))),
            );
        }
        let cls = if t.kind == ToastKind::Error { "toast error" } else { "toast" };
        vec![boxed(DecoratedBox::new().class(cls).child(row))]
    });
    boxed(
        Column::new()
            .main_axis_alignment(MainAxisAlignment::End)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("toast-wrap")
            .child(toast),
    )
}

pub(super) fn menu_layer() -> W {
    let ctx = state::ctx();
    boxed(Reactive::new(move || -> Vec<W> {
        let items = ctx.menu.get();
        vec![boxed(
            PopupMenu::new()
                .items(items)
                .position(ctx.menu_pos)
                .is_open(ctx.menu_open)
                .min_width(240.0)
                .on_select(|id| {
                    let id = id.to_string();
                    state::ctx().menu_open.set(false);
                    state::menu_selected(&id);
                }),
        )]
    }))
}

/// Правый нижний угол: задания.
pub(super) fn jobs_layer() -> W {
    boxed(
        Column::new()
            .main_axis_alignment(MainAxisAlignment::End)
            .cross_axis_alignment(CrossAxisAlignment::End)
            .class("jobs-wrap")
            .child(super::jobs::jobs_panel()),
    )
}

/// Узкое окно: ширина меньше этой — телефонная раскладка.
const NARROW_BELOW: f32 = 720.0;

pub fn root() -> W {
    let narrow = syngui::viewport::viewport_below(NARROW_BELOW);
    boxed(Reactive::new(move || -> Vec<W> {
        let phone = narrow.get();
        state::set_phone(phone);
        // Телефон: своего заголовка с кнопками окна нет — рамку (закрыть,
        // свернуть, двигать) рисует композитор. Окно может быть ещё не
        // создано — переключаем из цикла событий.
        run_on_main_thread(move || syngui::signal::set_decorations(phone));
        let ctx = state::ctx();
        ctx.drawer.set(false);
        ctx.sheet.set(false);
        ctx.phone_search.set(false);
        ctx.settings.set(false);
        vec![if phone { super::phone::root() } else { desktop_root() }]
    }))
}

/// Боковая панель и панели файлов. Между ними — разделитель: ширина
/// боковой панели меняется перетаскиванием (`Ctx::sidebar_width`), при
/// изменении размера окна меняются только панели файлов.
fn body() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let ctx = state::ctx();
        let content = DecoratedBox::new().class("content grow").child(panes());
        if !ctx.sidebar.get() {
            return vec![boxed(content)];
        }
        vec![boxed(
            SplitView::new(super::sidebar::sidebar(), content)
                .first_size_signal(ctx.sidebar_width)
                .first_size_range(state::SIDEBAR_MIN, state::SIDEBAR_MAX)
                .min_size(300.0)
                .class("side-split"),
        )]
    }))
}

fn desktop_root() -> W {
    // Разделителю нужна конечная ширина и высота: их даёт растянутая коробка
    // (`Reactive` прямо в строке или столбце мерится по содержимому).
    let body = bx("grow body", body());
    let main = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("window")
        .child(titlebar())
        .child(super::toolbar::nav_bar())
        .child(super::toolbar::command_bar())
        .child(body)
        .child(status_bar())
        .child(crate::chooser::bar());
    let hook = EventHook::new()
        .on_key_down(|k, m| {
            state::set_modifiers(m);
            if k == Key::Escape && state::ctx().settings.get_untracked() {
                state::ctx().settings.set(false);
                return KeyReply::Handled;
            }
            if state::ctx().dialog.get_untracked().is_some() {
                if k == Key::Escape {
                    state::ctx().dialog.set(None);
                    return KeyReply::Handled;
                }
                return KeyReply::Ignore;
            }
            if crate::actions::key(k, m) {
                KeyReply::Handled
            } else {
                KeyReply::Ignore
            }
        })
        .on_key_up(|_, m| {
            state::set_modifiers(m);
            KeyReply::Ignore
        })
        .on_char(|c| crate::actions::char_key(c, state::ctx_modifiers()))
        .child(
            Stack::new()
                .fit(StackFit::Expand)
                .child(main)
                .child(jobs_layer())
                .child(toast_layer())
                .child(super::settings::layer(false))
                .child(super::dialogs::dialogs())
                .child(menu_layer()),
        );
    // Своя рамка окна (CSD): скругление и обводка как у окон композитора,
    // тень под окном рисует он сам. Окно прозрачное — углы за скруглением пустые.
    let frame = DecoratedBox::new().class("window-frame").clip(true).child(hook);
    boxed(WindowResizeRegion::new().over_content(true).inset(5.0).child(frame))
}
