//! Журнал изменений (служба `synfsd`): история файла или папки в
//! «Свойствах» и раздел настроек с исключёнными программами. Запросы к
//! службе — в фоне; нет службы — подсказка, как её включить.

use std::path::{Path, PathBuf};

use synfsd::{Kind, Record};
use syngui::prelude::*;

use super::{boxed, icons, W};
use crate::model;
use crate::state;

/// Сколько записей истории показывать в «Свойствах».
const HISTORY_LIMIT: usize = 60;

fn unavailable(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
            t!("Журнал изменений не ведётся: служба synfsd не запущена. Включить: sudo systemctl enable --now synfsd").into()
        }
        _ => t!("Журнал изменений: {e}", e = e),
    }
}

/// Переименование временного файла поверх настоящего — так сохраняют
/// редакторы (`a.txt.1301621731`, `.a.txt.swp`, `a.txt~` → `a.txt`).
fn is_save(r: &Record) -> bool {
    let name = |p: &str| Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (Kind::Renamed, Some(old_path)) = (r.kind, &r.old_path) else { return false };
    let (new, old) = (name(&r.path), name(old_path));
    Path::new(&r.path).parent() == Path::new(old_path).parent()
        && !new.is_empty()
        && old != new
        && old.trim_start_matches('.').starts_with(&new)
}

fn kind_text(r: &Record) -> String {
    if is_save(r) {
        return t!("сохранила");
    }
    match r.kind {
        Kind::Created => t!("создала"),
        Kind::Modified => t!("изменила"),
        Kind::Deleted => t!("удалила"),
        Kind::Renamed => t!("переименовала"),
    }
}

fn kind_icon(k: Kind) -> &'static str {
    match k {
        Kind::Created => icons::ADD,
        Kind::Modified => icons::EDIT,
        Kind::Deleted => icons::DELETE,
        Kind::Renamed => icons::RENAME,
    }
}

/// Путь записи относительно `base` (сам `base` — его имя).
fn relative(base: &Path, p: &str) -> String {
    let p = Path::new(p);
    match p.strip_prefix(base) {
        Ok(rest) if rest.as_os_str().is_empty() => base.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        Ok(rest) => rest.display().to_string(),
        Err(_) => super::items::short_path(p),
    }
}

/// Правило исключения для записи: имя файла программы, иначе имя процесса.
fn rule_of(r: &Record) -> String {
    let name = Path::new(&r.exe).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name.is_empty() {
        r.comm.clone()
    } else {
        name
    }
}

fn exclude(program: String, after: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        let r = synfsd::set_excluded(&program, true);
        run_on_main_thread(move || match r {
            Ok(_) => {
                state::toast(t!("Изменения от «{p}» больше не записываются", p = program));
                after();
            }
            Err(e) => state::toast_error(e.to_string()),
        });
    });
}

fn history_row(base: &Path, r: &Record, rev: RwSignal<u64>) -> W {
    let mut what = format!("{} {}", r.program(), kind_text(r));
    if r.count > 1 {
        what.push_str(&format!(" ×{}", r.count));
    }
    let mut target = relative(base, &r.path);
    if let (Kind::Renamed, Some(old), false) = (r.kind, &r.old_path, is_save(r)) {
        target = t!("{new}, было «{old}»", new = target, old = relative(base, old));
    }
    let when = model::format_time(r.last / 1000);
    let rule = rule_of(r);
    let mut row = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            DecoratedBox::new()
                .class(format!("hist-badge {}", kind_class(r.kind)))
                .child(Icon::new(if is_save(r) { icons::EDIT } else { kind_icon(r.kind) }).class("icon")),
        )
        .child(
            Column::new()
                .gap(1.0)
                .class("grow")
                .child(Text::new(what).max_lines(1).class("hist-what"))
                .child(Text::new(target).elide(Elide::Middle).class("meta"))
                .child(Text::new(if r.exe.is_empty() { when } else { format!("{when} · {}", r.exe) }).elide(Elide::Middle).class("meta hist-exe")),
        );
    if !rule.is_empty() {
        row = row.child(super::icon_button(icons::BLOCK, &t!("Не записывать «{p}»", p = rule), "small", true, move || {
            exclude(rule.clone(), move || rev.update(|v| *v += 1))
        }));
    }
    boxed(DecoratedBox::new().class("hist-row").child(row))
}

fn kind_class(k: Kind) -> &'static str {
    match k {
        Kind::Created => "created",
        Kind::Modified => "modified",
        Kind::Deleted => "deleted",
        Kind::Renamed => "renamed",
    }
}

/// «Свойства»: кто и когда менял файл (у папки — и всё внутри).
pub fn history_section(path: PathBuf) -> W {
    let data = use_signal(None::<std::result::Result<Vec<Record>, String>>);
    let rev = use_signal(0u64);
    let is_dir = path.is_dir();
    {
        let path = path.clone();
        create_effect(move || {
            let _ = rev.get();
            let path = path.clone();
            std::thread::spawn(move || {
                let r = synfsd::history(&path, is_dir, HISTORY_LIMIT).map_err(|e| unavailable(&e));
                data.set(Some(r));
            });
        });
    }
    // Пути в записях — от папки (у файла — от той, где он лежит).
    let base = if is_dir { path.clone() } else { path.parent().map(Path::to_path_buf).unwrap_or_else(|| path.clone()) };
    let list = Reactive::new(move || -> Vec<W> {
        let base = base.clone();
        let body: W = match data.get() {
            None => boxed(Text::new(t!("Загрузка…")).class("meta")),
            Some(Err(e)) => boxed(Text::new(e).max_lines(4).selectable(true).class("meta")),
            Some(Ok(v)) if v.is_empty() => boxed(
                Text::new(if is_dir { t!("Пока ничего не менялось — записи появятся после изменений внутри.") } else { t!("Пока не менялся — записи появятся после изменений.") })
                    .max_lines(3)
                    .class("meta"),
            ),
            Some(Ok(v)) => {
                let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
                for r in &v {
                    col = col.child(history_row(&base, r, rev));
                }
                let h = (v.len() as f32 * 58.0).min(260.0);
                boxed(DecoratedBox::new().style("height", h).child(ScrollView::new().vertical().child(col)))
            }
        };
        vec![body]
    });
    boxed(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(DecoratedBox::new().class("side-sep"))
            .child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(icons::HISTORY).class("icon hist-title-icon"))
                    .child(Text::new(t!("История изменений")).class("hist-title grow"))
                    .child(super::icon_button(icons::REFRESH, &t!("Обновить"), "small", true, move || rev.update(|v| *v += 1))),
            )
            .child(list),
    )
}

/// Состояние раздела настроек.
#[derive(Clone, PartialEq)]
enum Excl {
    Loading,
    Ready { programs: Vec<String>, can_edit: bool },
    Failed(String),
}

fn load(st: RwSignal<Excl>) {
    std::thread::spawn(move || {
        let v = match synfsd::exclusions() {
            Ok((programs, can_edit)) => Excl::Ready { programs, can_edit },
            Err(e) => Excl::Failed(unavailable(&e)),
        };
        st.set(v);
    });
}

fn change(st: RwSignal<Excl>, program: String, on: bool) {
    std::thread::spawn(move || {
        let r = synfsd::set_excluded(&program, on);
        run_on_main_thread(move || match r {
            Ok(programs) => {
                let can_edit = true;
                st.set(Excl::Ready { programs, can_edit });
            }
            Err(e) => state::toast_error(e.to_string()),
        });
    });
}

fn program_chip(st: RwSignal<Excl>, p: String, can_edit: bool) -> W {
    let mut row = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Icon::new(icons::BLOCK).class("icon excl-icon"))
        .child(Text::new(p.clone()).max_lines(1).class("excl-name grow"));
    if can_edit {
        row = row.child(super::icon_button(icons::CLOSE, &t!("Снова записывать"), "small", true, move || change(st, p.clone(), false)));
    }
    boxed(DecoratedBox::new().class("excl-row").child(row))
}

/// Настройки → «Журнал изменений»: исключённые программы.
pub fn settings_rows() -> Vec<W> {
    let st = use_signal(Excl::Loading);
    load(st);
    let input = use_signal(String::new());
    let body = Reactive::new(move || -> Vec<W> {
        match st.get() {
            Excl::Loading => vec![boxed(Text::new(t!("Загрузка…")).class("set-hint"))],
            Excl::Failed(e) => vec![boxed(Text::new(e).max_lines(4).selectable(true).class("set-hint"))],
            Excl::Ready { programs, can_edit } => {
                let mut v: Vec<W> = Vec::new();
                v.push(boxed(Text::new(t!("Не записывать изменения программ")).class("set-row-title")));
                if programs.is_empty() {
                    v.push(boxed(Text::new(t!("Исключений нет — записывается всё.")).class("set-hint")));
                }
                for p in programs {
                    v.push(program_chip(st, p, can_edit));
                }
                if can_edit {
                    let add = move || {
                        let p = input.get_untracked().trim().to_string();
                        if !p.is_empty() {
                            input.set(String::new());
                            change(st, p, true);
                        }
                    };
                    v.push(boxed(
                        Row::new()
                            .gap(8.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(
                                DecoratedBox::new().class("grow").child(
                                    TextField::new()
                                        .placeholder(t!("journalctl или /usr/bin/…"))
                                        .on_change(move |s| input.set(s.to_string()))
                                        .on_submit(move |_| add()),
                                ),
                            )
                            .child(Button::new(t!("Добавить")).class("primary").on_click(add)),
                    ));
                } else {
                    v.push(boxed(Text::new(t!("Менять список могут администраторы (группа wheel).")).max_lines(2).class("set-hint")));
                }
                // Одним блоком: несколько детей `Reactive` легли бы друг на друга.
                let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
                for w in v {
                    col = col.child(w);
                }
                vec![boxed(col)]
            }
        }
    });
    vec![
        boxed(Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(body)),
        boxed(
            Text::new(t!(
                "Служба synfsd записывает, какая программа создала, изменила, удалила или переименовала файл. Имя — как у процесса (journalctl) или полный путь программы. История — в «Свойствах» файла или папки."
            ))
            .max_lines(6)
            .class("set-hint"),
        ),
    ]
}
