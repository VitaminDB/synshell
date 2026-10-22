//! Слева: темы (встроенные и свои) с образцами цветов и действия над темой.

use syngui::prelude::*;
use syngui::GestureDetector;
use synshell_common::t;

use super::{boxed, icon_button, W};
use crate::icons;
use crate::state::{with_draft, Confirm, St};

fn swatches(colors: &[String]) -> W {
    let mut row = Row::new().gap(0.0).class("te-swatches");
    for (i, c) in colors.iter().enumerate() {
        let c = synshell_common::config::Rgba::parse(c).unwrap_or(synshell_common::config::Rgba::rgb(0x80, 0x80, 0x80));
        let class = if i == 0 { "te-sw te-sw-first" } else if i + 1 == colors.len() { "te-sw te-sw-last" } else { "te-sw" };
        row = row.child(DecoratedBox::new().class(class).style("background-color", Color::from_srgb(c.r, c.g, c.b, c.a as f32 / 255.0)));
    }
    boxed(row)
}

fn theme_list(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let items = st.themes.get();
        let cur = st.current.get();
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        let section = |col: Column, title: String| col.child(Text::new(title).class("te-side-group"));
        let (mine, builtin): (Vec<_>, Vec<_>) = items.into_iter().partition(|i| !i.builtin);
        for (title, list) in [(t!("Свои"), mine), (t!("Встроенные"), builtin)] {
            if list.is_empty() {
                continue;
            }
            col = section(col, title);
            for it in list {
                let id = it.id.clone();
                let active = it.id == cur;
                let mut text = Column::new().gap(1.0).child(Text::new(it.name.clone()).max_lines(1).elide(Elide::End).class("te-theme-name"));
                if it.overrides_builtin {
                    text = text.child(Text::new(t!("изменённая встроенная")).class("te-theme-sub"));
                }
                let row = Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(swatches(&it.swatches))
                    .child(DecoratedBox::new().class("grow").child(text));
                col = col.child(
                    GestureDetector::new()
                        .on_click(move || st.open(&id))
                        .child(DecoratedBox::new().class(if active { "te-theme-item te-theme-item-active" } else { "te-theme-item" }).child(row)),
                );
            }
        }
        vec![boxed(col)]
    }))
}

/// Новая тема: из текущей, пустая (генератор), импорт.
fn new_menu(st: St) -> W {
    let open = use_signal(false);
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        col = col.child(Button::new(t!("Новая тема")).icon(icons::ADD).class("te-btn te-primary te-wide").on_click(move || open.set(!open.get_untracked())));
        if open.get() {
            let item = |glyph: &str, label: String, f: Box<dyn FnMut() + Send>| -> W {
                let mut f = f;
                boxed(Button::new(label).icon(glyph).class("te-btn te-flat te-menu-item").on_click(move || {
                    open.set(false);
                    f();
                }))
            };
            col = col
                .child(item(icons::COPY, t!("Из текущей"), Box::new(move || {
                    let base = with_draft(|d| d.id.clone());
                    let name = t!("Моя тема");
                    // Несохранённые правки текущей тоже попадут в копию.
                    if st.dirty.get_untracked() {
                        st.duplicate();
                    } else {
                        st.create(Some(&base), &name);
                    }
                })))
                .child(item(icons::MAGIC, t!("С нуля (генератор)"), Box::new(move || {
                    st.create(None, &t!("Моя тема"));
                    st.mode.set(crate::state::Mode::Quick);
                })))
                .child(item(icons::UPLOAD, t!("Импорт файла .syntheme"), Box::new(move || import(st))));
        }
        vec![boxed(DecoratedBox::new().class("te-new-menu").child(col))]
    }))
}

pub fn import(st: St) {
    std::thread::spawn(move || {
        let Some(path) = crate::files::choose(&t!("Импорт темы"), &t!("Темы synshell"), &["*.syntheme", "*.toml"], None, "DOWNLOAD") else { return };
        let r = std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| crate::draft::Draft::import_bundle(&t));
        run_on_main_thread(move || match r {
            Ok(d) => super::import_draft(st, d),
            Err(e) => st.toast.set(t!("Не удалось импортировать: {e}", e = e)),
        });
    });
}

pub fn export(st: St) {
    let (id, text) = with_draft(|d| (d.id.clone(), d.export_bundle()));
    std::thread::spawn(move || {
        let Some(path) = crate::files::choose(&t!("Экспорт темы"), &t!("Темы synshell"), &["*.syntheme"], Some(&format!("{id}.syntheme")), "DOWNLOAD") else { return };
        let r = std::fs::write(&path, text);
        run_on_main_thread(move || match r {
            Ok(()) => st.toast.set(t!("Тема сохранена в файл {path}", path = path)),
            Err(e) => st.toast.set(t!("Не записалось: {e}", e = e)),
        });
    });
}

fn actions(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let _ = st.rev.get();
        let _ = st.themes.get();
        let on_disk = with_draft(|d| d.save_dir().exists());
        let mut row = Row::new().gap(4.0).main_axis_alignment(MainAxisAlignment::Center);
        row = row
            .child(icon_button(icons::COPY, t!("Дублировать"), "", move || st.duplicate()))
            .child(icon_button(icons::DOWNLOAD, t!("Экспорт в файл .syntheme"), "", move || export(st)))
            .child(icon_button(icons::FOLDER, t!("Открыть папку тем"), "", || {
                let dir = synshell_common::theme::user_dir();
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
            }));
        if on_disk {
            row = row.child(icon_button(icons::DELETE, t!("Удалить с диска"), "te-danger-ic", move || st.confirm.set(Some(Confirm::Delete))));
        }
        vec![boxed(row)]
    }))
}

pub fn sidebar(st: St) -> W {
    boxed(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("te-sidebar")
            .child(new_menu(st))
            .child(DecoratedBox::new().class("grow").child(ScrollView::new().vertical().child(DecoratedBox::new().class("te-side-list").child(theme_list(st)))))
            .child(DecoratedBox::new().class("te-side-footer").child(actions(st))),
    )
}
