//! Боковая панель: быстрый доступ, закреплённые, корзина, устройства.

use syngui::prelude::*;
use syngui::widgets::*;
use syngui::DragData;

use super::{boxed, icons, W};
use crate::actions;
use crate::drives::{Device, Kind};
use crate::loc::Location;
use crate::places::{self, Place, Target};
use crate::state;
use crate::ui::view;

/// Безопасно извлечь в фоне: диск — отмонтировать разделы и выключить,
/// телефон — отключить.
fn eject(dev: Device) {
    std::thread::spawn(move || {
        let r = dev.eject();
        syngui::async_runtime::run_on_main_thread(move || match r {
            Ok(()) => {
                // Панель стояла внутри извлечённого — уйти домой, иначе она покажет пустоту.
                let p = state::pane();
                let cur = p.loc.get_untracked();
                if let (Some(d), Some(m)) = (cur.dir(), dev.mount_point()) {
                    if d.starts_with(m) {
                        state::navigate(p, Location::Dir(synshell_common::paths::home()), true);
                    }
                }
                state::toast(t!("«{title}» можно отключать", title = dev.title()));
            }
            Err(e) => state::toast_error(t!("Не удалось извлечь «{title}»: {e}", title = dev.title(), e = e)),
        });
    });
}

/// Монтирует устройство в фоне и открывает его в текущей панели.
fn mount_and_open(dev: Device) {
    std::thread::spawn(move || match dev.mount() {
        Ok(path) => syngui::async_runtime::run_on_main_thread(move || {
            state::navigate(state::pane(), Location::Dir(path), true);
        }),
        Err(e) => syngui::async_runtime::run_on_main_thread(move || {
            state::toast_error(t!("Не удалось открыть «{title}»: {e}", title = dev.title(), e = e));
        }),
    });
}

/// Устройство, которое ещё не смонтировано: щелчок монтирует и открывает.
/// Встроенный диск и несмонтированный телефон извлекать нечего — в меню
/// только «Открыть».
fn volume_row(dev: Device) -> W {
    let glyph = places::kind_icon(dev.kind());
    let sub = match &dev {
        Device::Disk(v) => t!("Не смонтирован · {v}", v = crate::drives::format_size(v.size)),
        Device::Gadget(_) => t!("{kind_name} · нажмите, чтобы открыть", kind_name = dev.kind_name()),
    };
    let col = Column::new()
        .gap(3.0)
        .child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(glyph).class("icon place-icon"))
                .child(Text::new(dev.title().to_string()).max_lines(1).class("place-title grow")),
        )
        .child(Text::new(sub).class("space-text"));
    let click = dev.clone();
    let row = GestureDetector::new()
        .on_click(move || mount_and_open(click.clone()))
        .on_secondary_click(move |at| {
            let mut items = vec![MenuItem::new("mount", t!("Открыть")).icon(icons::OPEN)];
            if matches!(dev.kind(), Kind::Usb) {
                items.push(MenuItem::separator());
                items.push(MenuItem::new("eject", t!("Безопасно извлечь")).icon(icons::EJECT));
            }
            let d = dev.clone();
            state::show_menu(items, at, move |id| match id {
                "mount" => mount_and_open(d.clone()),
                "eject" => eject(d.clone()),
                _ => {}
            });
        })
        .child(DecoratedBox::new().class("place").child(col));
    boxed(row)
}

fn place_row(pl: Place, current: &Location) -> W {
    let loc_src = match &pl.target {
        Target::Dir(loc) => loc.clone(),
        Target::Device(dev) => return volume_row(dev.clone()),
    };
    let active = &loc_src == current;
    let loc = loc_src.clone();
    let loc_mid = loc_src.clone();
    let loc_menu = loc_src.clone();
    let loc_drop = loc_src;
    let pinned = pl.pinned;
    let device = pl.device.clone();
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
        col = col.child(Text::new(t!("Свободно {v} из {v2}", v = crate::model::format_size(free), v2 = crate::model::format_size(total))).class("space-text"));
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
                MenuItem::new("open", t!("Открыть")).icon(icons::OPEN),
                MenuItem::new("open-tab", t!("Открыть в новой вкладке")).icon(icons::TAB),
            ];
            if loc_menu.dir().is_some() {
                items.push(MenuItem::new("terminal", t!("Открыть в терминале")).icon(icons::TERMINAL));
                items.push(MenuItem::separator());
                items.push(MenuItem::new("copy-path", t!("Копировать путь")).icon(icons::COPY_PATH));
            }
            if pinned {
                items.push(MenuItem::new("unpin", t!("Открепить")).icon(icons::PIN));
            }
            if matches!(loc_menu, Location::Trash) {
                items.push(MenuItem::separator());
                items.push(MenuItem::new("empty", t!("Очистить корзину")).icon(icons::DELETE_FOREVER).disabled(crate::trash::is_empty()));
            }
            if loc_menu.dir().is_some() {
                items.push(MenuItem::separator());
                items.push(MenuItem::new("props", t!("Свойства")).icon(icons::INFO));
            }
            if let Some(dev) = device.clone() {
                items.push(MenuItem::separator());
                items.push(MenuItem::new("eject", t!("Безопасно извлечь")).icon(icons::EJECT));
                let target = loc_menu.clone();
                state::show_menu(items, at, move |id| {
                    let p = state::pane();
                    match id {
                        "eject" => eject(dev.clone()),
                        _ => menu_action(id, &target, p),
                    }
                });
                return;
            }
            let target = loc_menu.clone();
            state::show_menu(items, at, move |id| {
                let p = state::pane();
                menu_action(id, &target, p);
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

/// Действие пункта меню места (кроме извлечения).
fn menu_action(id: &str, target: &Location, p: state::Pane) {
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
}

/// Места по разделам (боковая панель и выдвижная панель телефона).
pub fn places_list() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let ctx = state::ctx();
        let _ = ctx.places_rev.get();
        let _ = ctx.jobs_rev.get();
        let cfg = ctx.cfg.get();
        let current = state::tab_tracked().pane_tracked().loc.get();
        let mut col = Column::new().gap(1.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for (i, sec) in places::sections(&cfg.files.pinned).into_iter().enumerate() {
            if i > 0 {
                col = col.child(DecoratedBox::new().class("side-sep"));
            }
            col = col.child(Text::new(syngui::i18n::t(sec.title)).class("side-title"));
            for pl in sec.places {
                col = col.child(place_row(pl, &current));
            }
        }
        vec![boxed(col)]
    }))
}

/// Боковая панель окна; ширину и показ задаёт `app::body` (разделитель).
pub fn sidebar() -> impl Widget + 'static {
    DecoratedBox::new().class("sidebar").child(ScrollView::new().vertical().class("grow").child(places_list()))
}
