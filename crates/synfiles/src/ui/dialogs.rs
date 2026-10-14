//! Модальные окна: подтверждения, «Открыть с помощью», свойства.

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;

use synshell_common::{mime, xdg};
use syngui::prelude::*;
use syngui::widgets::*;
use syngui::CursorIcon;

use super::{boxed, bx, icons, W};
use crate::model;
use crate::ops::{self, Op};
use crate::state::{self, Dialog};

fn close() {
    state::ctx().dialog.set(None);
}

fn card(title: &str, body: W, buttons: Vec<W>) -> W {
    let mut row = Row::new().gap(8.0).main_axis_alignment(MainAxisAlignment::End).child(DecoratedBox::new().class("grow"));
    for b in buttons {
        row = row.child(b);
    }
    // Телефон: карточка во всю ширину экрана с полями.
    let mut card = DecoratedBox::new().class("dialog");
    if state::is_phone() {
        let vw = syngui::viewport::viewport_size().get_untracked().width;
        card = card.style("width", (vw - 24.0).clamp(240.0, 480.0));
    }
    boxed(
        card.child(
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(Text::new(title).class("dialog-title"))
                .child(body)
                .child(row),
        ),
    )
}

fn names(paths: &[PathBuf]) -> String {
    if paths.len() == 1 {
        format!("«{}»", ops::name_of(&paths[0]))
    } else {
        model::format_count(paths.len() as u32)
    }
}

fn confirm_delete(paths: Vec<PathBuf>) -> W {
    let p2 = paths.clone();
    let from_trash = paths.iter().all(|p| crate::trash::contains(p));
    let permanent = from_trash || !state::ctx().cfg.get_untracked().files.confirm_trash;
    let body = Text::new(if permanent {
        t!("Удалить {v} навсегда? Это действие нельзя отменить.", v = names(&paths))
    } else {
        t!("Переместить {v} в корзину?", v = names(&paths))
    })
    .class("dialog-text");
    card(
        &if permanent { t!("Удалить навсегда") } else { t!("Удалить в корзину") },
        boxed(body),
        vec![
            boxed(Button::new(t!("Отмена")).autofocus(true).on_click(close)),
            boxed(Button::new(t!("Удалить")).class(if permanent { "danger" } else { "primary" }).on_click(move || {
                close();
                let op = if permanent { Op::Delete { srcs: p2.clone() } } else { Op::Trash { srcs: p2.clone() } };
                ops::start(op);
            })),
        ],
    )
}

fn confirm_empty_trash() -> W {
    card(
        &t!("Очистить корзину"),
        boxed(Text::new(t!("Все файлы в корзине будут удалены навсегда.")).class("dialog-text")),
        vec![
            boxed(Button::new(t!("Отмена")).autofocus(true).on_click(close)),
            boxed(Button::new(t!("Очистить")).class("danger").on_click(|| {
                close();
                ops::start(Op::EmptyTrash);
            })),
        ],
    )
}

fn open_with(paths: Vec<PathBuf>, mime_type: String) -> W {
    let remember = use_signal(false);
    let filter = use_signal(String::new());
    let apps_all: Vec<xdg::DesktopEntry> = {
        // Сначала подходящие по типу, затем все остальные программы.
        let mut v: Vec<xdg::DesktopEntry> = mime::apps_for(&mime_type).into_iter().filter(|a| a.takes_files()).collect();
        let mut rest: Vec<xdg::DesktopEntry> =
            xdg::apps().iter().filter(|a| !a.no_display && a.takes_files() && !v.iter().any(|x| x.id == a.id)).cloned().collect();
        rest.sort_by(|a, b| model::natural_cmp(&a.name, &b.name));
        v.extend(rest);
        v
    };
    let m2 = mime_type.clone();
    let list = Reactive::new(move || -> Vec<W> {
        let q = filter.get().to_lowercase();
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for a in apps_all.iter().filter(|a| q.is_empty() || a.name.to_lowercase().contains(&q)).take(60) {
            let app = a.clone();
            let files = paths.clone();
            let mt = m2.clone();
            let icon: W = match xdg::lookup_icon(&a.icon) {
                Some(p) => boxed(Image::new(p.to_string_lossy().to_string()).fit(ImageFit::Contain).style("width", 24.0).style("height", 24.0)),
                None => boxed(Icon::new(icons::OPEN_WITH).class("icon")),
            };
            col = col.child(
                GestureDetector::new()
                    .on_click(move || {
                        if remember.get_untracked() {
                            if let Err(e) = mime::set_default_app(&mt, &app.id) {
                                state::toast_error(format!("mimeapps.list: {e}"));
                            }
                        }
                        close();
                        crate::actions::launch(&app, &files);
                    })
                    .child(
                        DecoratedBox::new().class("app-row").child(
                            Row::new()
                                .gap(10.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(icon)
                                .child(Text::new(a.name.clone()).max_lines(1).class("grow"))
                                .child(Text::new(if state::is_phone() { String::new() } else { a.comment.clone() }).max_lines(1).class("meta")),
                        ),
                    ),
            );
        }
        vec![boxed(col)]
    });
    let body = Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(t!("Тип: {v} ({mime_type})", v = mime::description(&mime_type), mime_type = mime_type)).class("meta"))
        .child(TextField::new().placeholder(t!("Найти программу")).prefix_icon(icons::SEARCH).autofocus(true).on_change(move |s| filter.set(s.to_string())))
        .child(bx("app-list", ScrollView::new().vertical().child(list)))
        .child(Checkbox::new().label(t!("Всегда открывать файлы этого типа выбранной программой")).on_change(move |v| remember.set(v)));
    card(&t!("Открыть с помощью"), boxed(body), vec![boxed(Button::new(t!("Отмена")).on_click(close))])
}

fn prop_row(k: &str, v: String) -> W {
    boxed(
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .child(DecoratedBox::new().class("prop-key").style("width", if state::is_phone() { 96.0 } else { 130.0 }).child(Text::new(k).class("meta")))
            .child(Text::new(v).selectable(true).class("prop-val grow")),
    )
}

fn owner_name(uid: u32) -> String {
    let pw = unsafe { libc::getpwuid(uid) };
    if pw.is_null() {
        return uid.to_string();
    }
    unsafe { std::ffi::CStr::from_ptr((*pw).pw_name) }.to_string_lossy().to_string()
}

fn properties(paths: Vec<PathBuf>) -> W {
    // Размер папок считается в фоне.
    let size = use_signal(None::<(u64, u64)>);
    {
        let paths = paths.clone();
        std::thread::spawn(move || {
            let r = ops::measure(&paths);
            size.set(Some(r));
        });
    }
    let one = paths.len() == 1;
    let first = paths[0].clone();
    let lmeta = std::fs::symlink_metadata(&first).ok();
    let entry = crate::model::Entry::from_path(&first);
    let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    if one {
        if let Some(e) = &entry {
            col = col.child(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().style("width", 48.0).style("height", 48.0).child(super::items::entry_image(e, 48, true)))
                    .child(Text::new(e.name.clone()).selectable(true).max_lines(2).class("prop-name grow")),
            );
            col = col.child(DecoratedBox::new().class("side-sep"));
            col = col.child(prop_row(&t!("Тип"), format!("{} ({})", e.description(), e.mime)));
            if e.is_link {
                let target = std::fs::read_link(&first).map(|t| t.display().to_string()).unwrap_or_default();
                col = col.child(prop_row(&t!("Ссылка на"), target));
            }
            if let Some(app) = (!e.is_dir).then(|| mime::default_app(&e.mime)).flatten() {
                col = col.child(prop_row(&t!("Открывается"), app.name));
            }
        }
    } else {
        col = col.child(Text::new(model::format_count(paths.len() as u32)).class("prop-name"));
    }
    col = col.child(prop_row(&t!("Расположение"), first.parent().map(|p| p.display().to_string()).unwrap_or_default()));
    col = col.child(boxed(Reactive::new(move || -> Vec<W> {
        let v = match size.get() {
            None => t!("Подсчёт…").to_string(),
            Some((b, f)) => t!("{v} ({b} байт), файлов и папок: {v2}", v = model::format_size(b), b = b, v2 = f.saturating_sub(1).max(if f == 1 { 1 } else { 0 })),
        };
        vec![prop_row(&t!("Размер"), v)]
    })));
    if one {
        if let Some(m) = &lmeta {
            col = col.child(prop_row(&t!("Изменён"), model::format_time(m.mtime())));
            col = col.child(prop_row(&t!("Открыт"), model::format_time(m.atime())));
            col = col.child(prop_row(&t!("Владелец"), format!("{} ({})", owner_name(m.uid()), m.uid())));
            col = col.child(prop_row(&t!("Права"), format!("{} ({:o})", model::format_mode(m.permissions().mode()), m.permissions().mode() & 0o7777)));
            if let Some((free, total)) = crate::places::space(&first) {
                col = col.child(prop_row(&t!("На диске"), t!("свободно {v} из {v2}", v = model::format_size(free), v2 = model::format_size(total))));
            }
            if m.is_file() {
                let exec = m.permissions().mode() & 0o111 != 0;
                let path = first.clone();
                col = col.child(
                    Checkbox::checked(exec).label(t!("Разрешить запуск как программы")).on_change(move |on| {
                        if let Ok(meta) = std::fs::metadata(&path) {
                            let mut mode = meta.permissions().mode();
                            if on {
                                mode |= (mode & 0o444) >> 2;
                            } else {
                                mode &= !0o111;
                            }
                            if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)) {
                                state::toast_error(t!("Не удалось изменить права: {e}", e = e));
                            }
                        }
                    }),
                );
            }
        }
        col = col.child(super::activity::history_section(first.clone()));
    }
    card(
        &if one { t!("Свойства") } else { t!("Свойства выделенного") },
        boxed(col),
        vec![boxed(Button::new(t!("Закрыть")).class("primary").autofocus(true).on_click(close))],
    )
}

fn field(label: &str, f: TextField) -> W {
    boxed(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().style("width", if state::is_phone() { 84.0 } else { 110.0 }).child(Text::new(label).class("meta")))
            .child(bx("grow", f)),
    )
}

fn batch_rename(paths: Vec<PathBuf>) -> W {
    use crate::rename::{self, Case, Rule};
    let rule = use_signal(Rule::default());
    let items: Vec<(String, bool)> = paths.iter().map(|p| (ops::name_of(p), p.is_dir())).collect();
    let edit = move |f: &dyn Fn(&mut Rule)| {
        let mut r = rule.get_untracked();
        f(&mut r);
        rule.set(r);
    };
    let (paths2, items2) = (paths.clone(), items.clone());
    let preview = Reactive::new(move || -> Vec<W> {
        let r = rule.get();
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        match rename::new_names(&items2, &r) {
            Err(e) => col = col.child(Text::new(e).class("rename-bad")),
            Ok(names) => {
                let bad = rename::problems(&paths2, &names);
                for ((old, _), (new, why)) in items2.iter().zip(names.iter().zip(&bad)).take(200) {
                    let row = Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(DecoratedBox::new().class("grow").style("width", 0.0).child(Text::new(old.clone()).max_lines(1).class("meta")))
                        .child(Icon::new(icons::FORWARD).class("rename-arrow"))
                        .child(DecoratedBox::new().class("grow").style("width", 0.0).child(
                            Text::new(match why {
                                Some(w) => format!("{new} — {w}"),
                                None => new.clone(),
                            })
                            .max_lines(1)
                            .class(if why.is_some() { "rename-bad" } else if new == old { "meta" } else { "prop-val" }),
                        ));
                    col = col.child(row);
                }
            }
        }
        vec![boxed(col)]
    });
    let phone = state::is_phone();
    let body = Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(field(
            &t!("Шаблон"),
            TextField::new().placeholder("{name}").autofocus(true).on_change(move |s: &str| edit(&|r| r.template = s.to_string())),
        ))
        .child(Text::new(t!("{name} — прежнее имя, {n} — номер, {ext} — расширение (без него — остаётся прежнее)")).class("meta"))
        .child(field(&t!("Найти"), TextField::new().on_change(move |s: &str| edit(&|r| r.find = s.to_string()))))
        .child(field(&t!("Заменить на"), TextField::new().on_change(move |s: &str| edit(&|r| r.replace = s.to_string()))))
        .child(boxed(
            Row::new()
                .gap(16.0)
                .child(Checkbox::new().label(t!("Регулярное выражение")).on_change(move |v| edit(&|r| r.regex = v)))
                .child(Checkbox::new().label(t!("Без учёта регистра")).on_change(move |v| edit(&|r| r.ignore_case = v))),
        ))
        .child(field(
            &t!("Нумерация с"),
            TextField::new().text("1").on_change(move |s: &str| {
                let n = s.trim().parse().unwrap_or(1);
                edit(&|r| r.start = n)
            }),
        ))
        .child(boxed(
            SegmentedButton::new(if phone {
                vec!["Aa".to_string(), "aa".into(), "AA".into(), "Aa Bb".into()]
            } else {
                vec![t!("Как есть").to_string(), t!("строчные").into(), t!("ПРОПИСНЫЕ").into(), t!("Каждое Слово").into()]
            })
            .selected(0)
            .on_change(move |i| {
                let c = [Case::Keep, Case::Lower, Case::Upper, Case::Title][i.min(3)];
                edit(&|r| r.case = c)
            }),
        ))
        .child(DecoratedBox::new().class("side-sep"))
        .child(bx("rename-list", ScrollView::new().vertical().child(preview)));
    let title = t!("Переименовать {v}", v = names(&paths));
    let ok = move || {
        let r = rule.get_untracked();
        let res = rename::new_names(&items, &r).and_then(|names| ops::rename_many(&paths, &names));
        match res {
            Ok(_) => {
                close();
                for d in paths.iter().filter_map(|p| p.parent()).collect::<std::collections::BTreeSet<_>>() {
                    state::reload_dir(d);
                }
            }
            Err(e) => state::toast_error(e),
        }
    };
    card(
        &title,
        boxed(body),
        vec![boxed(Button::new(t!("Отмена")).on_click(close)), boxed(Button::new(t!("Переименовать")).class("primary").on_click(ok))],
    )
}

/// Слой модальных окон (затемнение + карточка по центру). `Reactive`
/// отдаёт детям свободные ограничения, поэтому растягивающий слой и
/// центрирующая колонка — снаружи, а затемнению размер задан по окну.
pub fn dialogs() -> W {
    let scrim = Reactive::new(move || -> Vec<W> {
        if state::ctx().dialog.with(|d| d.is_none()) {
            return vec![];
        }
        let vp = syngui::viewport::viewport_size().get();
        vec![boxed(
            GestureDetector::new()
                .cursor(CursorIcon::Default)
                .on_click(close)
                .child(DecoratedBox::new().class("scrim").style("width", vp.width).style("height", vp.height)),
        )]
    });
    let card = Reactive::new(move || -> Vec<W> {
        let Some(d) = state::ctx().dialog.get() else { return vec![] };
        let content = match d {
            Dialog::ConfirmDelete { paths } => confirm_delete(paths),
            Dialog::ConfirmEmptyTrash => confirm_empty_trash(),
            Dialog::OpenWith { paths, mime } => open_with(paths, mime),
            Dialog::Properties { paths } if !paths.is_empty() => properties(paths),
            Dialog::BatchRename { paths } if !paths.is_empty() => batch_rename(paths),
            _ => return vec![],
        };
        vec![boxed(GestureDetector::new().cursor(CursorIcon::Default).child(content))]
    });
    boxed(
        Stack::new().fit(StackFit::Expand).child(scrim).child(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(card),
        ),
    )
}
