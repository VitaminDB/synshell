//! Боковая панель: быстрый доступ, закреплённые, корзина, устройства.

use syngui::prelude::*;
use syngui::widgets::*;
use syngui::DragData;

use super::{boxed, bx, icons, W};
use crate::actions;
use crate::loc::Location;
use crate::places::{self, Place};
use crate::state;
use crate::ui::view;

fn place_row(pl: Place, current: &Location) -> W {
    let active = &pl.loc == current;
    let loc = pl.loc.clone();
    let loc_mid = pl.loc.clone();
    let loc_menu = pl.loc.clone();
    let loc_drop = pl.loc.clone();
    let pinned = pl.pinned;
    let mut col = Column::new().gap(3.0).child(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(pl.icon).class("icon place-icon"))
            .child(Text::new(pl.title.clone()).max_lines(1).class("place-title grow")),
    );
    if let Some((free, total)) = pl.space.filter(|s| s.1 > 0) {
        let used = 1.0 - free as f32 / total as f32;
        col = col.child(
            DecoratedBox::new()
                .class(if used > 0.9 { "space-bar full" } else { "space-bar" })
                .child(DecoratedBox::new().class("space-fill").style("width", (used * 150.0).clamp(2.0, 150.0))),
        );
        col = col.child(Text::new(format!("Свободно {} из {}", crate::model::format_size(free), crate::model::format_size(total))).class("space-text"));
    }
    let row = GestureDetector::new()
        .on_click(move || {
            let p = state::pane();
            state::navigate(p, loc.clone(), true);
        })
        .on_middle_click(move |_| {
            state::new_tab(loc_mid.clone(), false);
        })
        .on_secondary_click(move |at| {
            let mut items = vec![
                MenuItem::new("open", "Открыть").icon(icons::OPEN),
                MenuItem::new("open-tab", "Открыть в новой вкладке").icon(icons::TAB),
            ];
            if loc_menu.dir().is_some() {
                items.push(MenuItem::new("terminal", "Открыть в терминале").icon(icons::TERMINAL));
                items.push(MenuItem::separator());
                items.push(MenuItem::new("copy-path", "Копировать путь").icon(icons::COPY_PATH));
            }
            if pinned {
                items.push(MenuItem::new("unpin", "Открепить").icon(icons::PIN));
            }
            if matches!(loc_menu, Location::Trash) {
                items.push(MenuItem::separator());
                items.push(MenuItem::new("empty", "Очистить корзину").icon(icons::DELETE_FOREVER).disabled(crate::trash::is_empty()));
            }
            if loc_menu.dir().is_some() {
                items.push(MenuItem::separator());
                items.push(MenuItem::new("props", "Свойства").icon(icons::INFO));
            }
            let target = loc_menu.clone();
            state::show_menu(items, at, move |id| {
                let p = state::pane();
                match id {
                    "open" => state::navigate(p, target.clone(), true),
                    "open-tab" => {
                        state::new_tab(target.clone(), true);
                    }
                    "terminal" => {
                        if let Some(d) = target.dir() {
                            actions::open_terminal(d);
                        }
                    }
                    "copy-path" => {
                        if let Some(d) = target.dir() {
                            actions::copy_paths_text(&[d.to_path_buf()]);
                        }
                    }
                    "unpin" => {
                        if let Some(d) = target.dir() {
                            actions::toggle_pin(d);
                        }
                    }
                    "empty" => state::ctx().dialog.set(Some(state::Dialog::ConfirmEmptyTrash)),
                    "props" => {
                        if let Some(d) = target.dir() {
                            state::ctx().dialog.set(Some(state::Dialog::Properties { paths: vec![d.to_path_buf()] }));
                        }
                    }
                    _ => {}
                }
            });
        })
        .child(DecoratedBox::new().class(if active { "place active" } else { "place" }).child(col));
    // Перенос файлов на место: в папку — как на папку в списке, в корзину — удалить.
    boxed(
        DropArea::new()
            .accept_types(vec![view::DRAG_FILES.to_string(), DragData::TYPE_FILE.to_string()])
            .on_drop_positioned(move |info| {
                let m = state::ctx_modifiers();
                match &loc_drop {
                    Location::Trash => {
                        let srcs = view::drag_paths(&info.data);
                        if !srcs.is_empty() {
                            crate::ops::start(crate::ops::Op::Trash { srcs });
                        }
                    }
                    Location::Dir(d) => view::drop_into(d, &info.data, m),
                    Location::Search { .. } => {}
                }
            })
            .child(row),
    )
}

pub fn sidebar() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let ctx = state::ctx();
        if !ctx.sidebar.get() {
            return vec![];
        }
        let _ = ctx.places_rev.get();
        let _ = ctx.jobs_rev.get();
        let cfg = ctx.cfg.get();
        let current = state::tab_tracked().pane_tracked().loc.get();
        let mut col = Column::new().gap(1.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for (i, sec) in places::sections(&cfg.files.pinned).into_iter().enumerate() {
            if i > 0 {
                col = col.child(DecoratedBox::new().class("side-sep"));
            }
            col = col.child(Text::new(sec.title).class("side-title"));
            for pl in sec.places {
                col = col.child(place_row(pl, &current));
            }
        }
        vec![bx("sidebar", ScrollView::new().vertical().class("grow").child(col))]
    }))
}
