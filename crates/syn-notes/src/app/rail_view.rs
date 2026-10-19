//! Панель проектов слева: плитка на каждый открытый `.syn`, «+» — создать,
//! открыть или вернуть недавний. Замена плиток рейла synthos.

use syngui::prelude::*;
use syngui::widgets::overlay::menu::MenuItem;
use syngui::widgets::{ContextMenu, GestureDetector, PopupMenu, ToolButton, Tooltip};

use crate::icons::*;
use crate::notes::{project, project_ui, NotesCtx};

pub fn view() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    let main = use_signal(super::server::main_storage().map(|p| project::normalize_path(&p)));
    provide_context(MainStorage(main));
    let tiles = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let active = ctx.project_path.get();
        let main = main.get();
        let mut list = ctx.projects.get();
        list.sort_by_key(|p| p.opened_at);
        list.into_iter()
            .map(|p| {
                let is_main = main.as_ref() == Some(&p.path);
                Box::new(tile(p.path.clone(), p.path == active, is_main)) as Box<dyn Widget>
            })
            .collect()
    });
    Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("notes-app-rail")
        .child(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(tiles))
        .child(add_button())
}

/// Основное хранилище (`[notes] storage`) — для отметки на плитке.
#[derive(Clone, Copy)]
struct MainStorage(RwSignal<Option<std::path::PathBuf>>);

/// Сделать проект основным хранилищем: ассистент, рабочий стол и
/// напоминания работают с ним.
fn set_main(path: &std::path::Path) {
    let value = toml_edit::Value::from(path.display().to_string());
    match synshell_common::config_edit::set_value(&["notes", "storage"], value) {
        Ok(_) => use_context::<MainStorage>().0.set(Some(path.to_path_buf())),
        Err(e) => tracing::warn!("основное хранилище не записано: {e}"),
    }
}

fn tile(path: std::path::PathBuf, selected: bool, is_main: bool) -> impl Widget {
    let title = project::project_title(&path);
    let letter: String = title.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    let class = if selected { "notes-app-tile selected" } else { "notes-app-tile" };
    let target = path.clone();
    let body = GestureDetector::new()
        .cursor(syngui::input::CursorIcon::Pointer)
        .on_click(move || {
            use_context::<NotesCtx>().switch_project(&target);
        })
        .child(
            DecoratedBox::new()
                .class(class)
                .child(
                    Stack::new()
                        .child(Center::new().child(Text::new(letter).class("notes-app-tile-letter")))
                        .children(if is_main {
                            vec![Box::new(Icon::new(MI_STAR).class("notes-app-tile-main")) as Box<dyn Widget>]
                        } else {
                            Vec::new()
                        }),
                ),
        );
    let hint = if is_main {
        format!("{title} · {}\n{}", syngui::tr!("notes.project.main"), crate::paths::pretty(&path))
    } else {
        format!("{title}\n{}", crate::paths::pretty(&path))
    };
    let menu_path = path.clone();
    ContextMenu::new()
        .items(vec![
            MenuItem::new("main", syngui::tr!("notes.project.make_main")).icon(MI_STAR).disabled(is_main),
            MenuItem::separator(),
            MenuItem::new("rename", syngui::tr!("notes.project.rename")).icon(MI_DRIVE_FILE_RENAME_OUTLINE),
            MenuItem::new("reveal", syngui::tr!("notes.project.reveal")).icon(MI_FOLDER),
            MenuItem::separator(),
            MenuItem::new("close", syngui::tr!("notes.project.close")).icon(MI_CLOSE),
        ])
        .on_select(move |id| match id {
            "main" => set_main(&menu_path),
            "rename" => project_ui::request_rename(&menu_path),
            "reveal" => crate::paths::reveal_in_files(&menu_path),
            "close" => {
                if let Err(e) = use_context::<NotesCtx>().close_project(&menu_path) {
                    project_ui::report_error(&e);
                }
            }
            _ => {}
        })
        .child(Tooltip::new(body, hint))
}

fn add_button() -> impl Widget {
    let open = use_signal(false);
    let pos = use_signal(Point::zero());
    let btn = ToolButton::new(MI_ADD)
        .tooltip(syngui::tr!("notes.project.menu"))
        .on_click_with_bounds(move |_, bounds| {
            pos.set(Point::new(bounds.origin.x + bounds.size.width + 8.0, bounds.origin.y));
            open.set(true);
        })
        .class("notes-app-add");
    let menu = PopupMenu::new()
        .items(project_ui::add_menu_items())
        .is_open(open)
        .position(pos)
        .on_select(|id| {
            project_ui::handle_menu(id);
        });
    Stack::new().clip(false).child(btn).child(menu)
}
