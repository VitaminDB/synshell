//! Приложения (окна) и процессы: поиск, сортировка, закрыть мягко (окно — просьбой закрыться,
//! процесс — SIGTERM) или убить (SIGKILL).

use syngui::core::Color;
use syngui::prelude::*;
use syngui::GestureDetector;
use synshell_common::ipc::WindowOp;

use super::{header, Cx};
use crate::sampler::{App, Snap};
use crate::state::{icons, Confirm, Page, Sort, St};
use crate::ui::parts::{self, boxed, W};

/// Строк процессов без поиска (длинный список тормозил бы перестройку раз в секунду).
const MAX_ROWS: usize = 150;

/// Действие над окном через композитор.
fn window_op(id: u64, op: WindowOp) {
    std::thread::spawn(move || {
        use synshell_common::ipc::{Client, Request};
        let _ = Client::connect().and_then(|mut c| c.request(&Request::WindowAction { id, op }));
    });
}

/// Сигнал процессу; ошибка — во всплывающую строку.
fn signal(st: St, pid: i32, name: &str, sig: i32) {
    // SAFETY: kill без побочных эффектов для нас самих (pid не наш — проверено ниже).
    let r = if pid <= 1 || pid == std::process::id() as i32 { -1 } else { unsafe { libc::kill(pid, sig) } };
    let msg = if r == 0 {
        if sig == libc::SIGKILL {
            t!("«{n}» убит", n = name)
        } else {
            t!("«{n}»: отправлен запрос завершиться", n = name)
        }
    } else {
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::EPERM) => t!("«{n}»: нет прав — процесс другого пользователя", n = name),
            _ => t!("«{n}»: не удалось", n = name),
        }
    };
    st.toast.set(msg);
    st.confirm.set(None);
}

/// Панель над списком: поиск, сортировка, фильтры, сообщение о последнем действии.
pub fn toolbar(st: St, narrow: bool) -> W {
    let search = TextField::new().placeholder(t!("Поиск по имени, команде, PID, пользователю")).prefix_icon(icons::SEARCH).on_change(move |s| st.query.set(s.to_string())).class("search");
    let chips = Reactive::new(move || -> Vec<W> {
        let sort = st.sort.get();
        let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for (s, label) in [(Sort::Cpu, t!("Процессор")), (Sort::Memory, t!("Память")), (Sort::Name, t!("Имя")), (Sort::Pid, t!("PID"))] {
            row = row.child(pill(label, sort == s, move || st.sort.set(s)));
        }
        row = row.child(DecoratedBox::new().class("pill-sep"));
        let mine = st.mine.get();
        row = row.child(pill(t!("Только мои"), mine, move || st.mine.set(!mine)));
        let k = st.kernel_threads.get();
        row = row.child(pill(t!("Потоки ядра"), k, move || st.kernel_threads.set(!k)));
        vec![boxed(row)]
    });
    let toast = Reactive::new(move || -> Vec<W> {
        let t = st.toast.get();
        if t.is_empty() {
            return vec![];
        }
        vec![boxed(
            GestureDetector::new().on_click(move || st.toast.set(String::new())).child(
                DecoratedBox::new()
                    .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icons::INFO).class("toast-icon")).child(Text::new(t).max_lines(2).class("grow")))
                    .class("toast"),
            ),
        )]
    });
    boxed(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(search)
            .child(ScrollView::new().horizontal().child(chips))
            .child(toast)
            .class(if narrow { "toolbar narrow" } else { "toolbar" }),
    )
}

fn pill(label: String, active: bool, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    GestureDetector::new().on_click(f).child(DecoratedBox::new().child(Text::new(label).class("pill-label")).class(if active { "pill active" } else { "pill" }))
}

fn button(label: String, icon: &str, class: &str, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    GestureDetector::new().on_click(f).child(
        DecoratedBox::new()
            .child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icon).class("btn-icon")).child(Text::new(label).class("btn-label")))
            .class(format!("btn {class}")),
    )
}

/// Кнопка-значок (узкое окно).
fn round(icon: &str, class: &str, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    GestureDetector::new().on_click(f).child(DecoratedBox::new().child(Icon::new(icon).class("btn-icon")).class(format!("btn round {class}")))
}

fn matches(q: &str, fields: &[&str]) -> bool {
    let q = q.trim().to_lowercase();
    q.is_empty() || fields.iter().any(|f| f.to_lowercase().contains(&q))
}

pub fn list(cx: &Cx, s: &Snap) -> Vec<W> {
    let st = cx.st;
    let q = st.query.get();
    let threads: u32 = s.procs.iter().map(|p| p.threads).sum();
    let mut out = vec![header(cx, Page::Processes, Some(t!("{n} процессов · {t} потоков", n = s.procs.len(), t = threads)), false)];

    // Приложения — окна композитора.
    if let Some(apps) = &s.apps {
        let apps: Vec<&App> = apps.iter().filter(|a| matches(&q, &[&a.name, &a.title, &a.app_id])).collect();
        if !apps.is_empty() {
            let mut col = Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(parts::card_head(icons::LIST, t!("Приложения"), Some(apps.len().to_string()), Page::Processes.color()));
            for a in apps {
                col = col.child(app_row(cx, a));
            }
            out.push(boxed(parts::card(col)));
        }
    }

    // Процессы.
    let me = unsafe { libc::getuid() };
    let kernel = st.kernel_threads.get();
    let mine = st.mine.get();
    let mut list: Vec<&synsystem::procs::Process> = s
        .procs
        .iter()
        .filter(|p| kernel || !p.is_kernel())
        .filter(|p| !mine || p.uid == me)
        .filter(|p| matches(&q, &[&p.name, &p.cmdline, &p.user, &p.pid.to_string()]))
        .collect();
    match st.sort.get() {
        Sort::Cpu => list.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent).then(b.rss_kb.cmp(&a.rss_kb))),
        Sort::Memory => list.sort_by(|a, b| b.rss_kb.cmp(&a.rss_kb)),
        Sort::Name => list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        Sort::Pid => list.sort_by_key(|p| p.pid),
    }
    let total = list.len();
    let sel = st.selected.get();
    // Выбранный процесс не пропадает из урезанного списка.
    if let Some(pos) = sel.and_then(|pid| list.iter().position(|p| p.pid == pid)) {
        if pos >= MAX_ROWS {
            let p = list.remove(pos);
            list.insert(0, p);
        }
    }
    let mut col = Column::new()
        .gap(2.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(parts::card_head(icons::CPU, t!("Процессы"), Some(if total > MAX_ROWS { t!("{n} из {t}", n = MAX_ROWS, t = total) } else { total.to_string() }), Page::Processes.color()));
    if !cx.narrow {
        col = col.child(
            Row::new()
                .gap(10.0)
                .child(Text::new(t!("Имя")).class("th grow"))
                .child(Text::new(t!("Пользователь")).class("th col-user"))
                .child(Text::new(t!("PID")).class("th col-pid"))
                .child(Text::new(t!("ЦП")).class("th col-num"))
                .child(Text::new(t!("Память")).class("th col-mem"))
                .class("thead"),
        );
    }
    if list.is_empty() {
        col = col.child(Text::new(t!("Ничего не найдено")).class("muted empty"));
    }
    for p in list.into_iter().take(MAX_ROWS) {
        col = col.child(proc_row(cx, p, sel == Some(p.pid)));
    }
    out.push(boxed(parts::card(col)));
    out
}

fn app_row(cx: &Cx, a: &App) -> W {
    let st = cx.st;
    let id = a.window;
    let color = avatar_color(&a.app_id);
    let letter = a.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    let avatar = DecoratedBox::new()
        .child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(letter).class("avatar-letter")))
        .class("avatar")
        .style("background-color", color);
    let info = Column::new()
        .gap(1.0)
        .child(Text::new(a.name.clone()).max_lines(1).class("row-title"))
        .child(Text::new(format!("{} · {} · {}", parts::kb(a.rss_kb), t!("ЦП {c}", c = parts::pct(a.cpu)), if a.pids.len() > 1 { t!("процессов {n}", n = a.pids.len()) } else { a.pid.map(|p| format!("PID {p}")).unwrap_or_default() })).max_lines(1).class("muted small"))
        .class("grow");
    let name = a.name.clone();
    let confirming = st.confirm.get() == Some(Confirm::Window(id));
    let mut row = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(avatar).child(info);
    if confirming {
        row = row
            .child(button(t!("Убить"), icons::STOP, "danger", move || {
                window_op(id, WindowOp::Kill);
                st.toast.set(t!("«{n}» убит", n = name));
                st.confirm.set(None);
            }))
            .child(button(t!("Отмена"), icons::CLOSE, "", move || st.confirm.set(None)));
    } else {
        let name2 = a.name.clone();
        let close = move || {
            window_op(id, WindowOp::Close);
            st.toast.set(t!("«{n}»: окну отправлена просьба закрыться", n = name2));
        };
        let kill = move || st.confirm.set(Some(Confirm::Window(id)));
        if cx.narrow {
            row = row.child(round(icons::CLOSE, "", close)).child(round(icons::STOP, "danger-soft", kill));
        } else {
            row = row.child(button(t!("Закрыть"), icons::CLOSE, "", close)).child(button(t!("Убить"), icons::STOP, "danger-soft", kill));
        }
    }
    boxed(DecoratedBox::new().child(row).class("row-item"))
}

fn proc_row(cx: &Cx, p: &synsystem::procs::Process, selected: bool) -> W {
    let st = cx.st;
    let pid = p.pid;
    let cpu_class = if p.cpu_percent >= 25.0 { "num col-num hot" } else { "num col-num" };
    let main: W = if cx.narrow {
        boxed(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Column::new().gap(1.0).child(Text::new(p.name.clone()).max_lines(1).class("row-title")).child(Text::new(format!("{} · {}", p.pid, p.user)).max_lines(1).class("muted small")).class("grow"))
                .child(Text::new(parts::pct(p.cpu_percent)).class(cpu_class))
                .child(Text::new(parts::kb(p.rss_kb)).class("num col-mem")),
        )
    } else {
        boxed(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(
                    Column::new()
                        .gap(0.0)
                        .child(Text::new(p.name.clone()).max_lines(1).class("row-title"))
                        .child(Text::new(if p.cmdline.is_empty() { t!("[поток ядра]") } else { p.cmdline.clone() }).max_lines(1).elide(Elide::Middle).class("muted small"))
                        .class("grow"),
                )
                .child(Text::new(p.user.clone()).max_lines(1).class("muted col-user"))
                .child(Text::new(p.pid.to_string()).class("muted col-pid"))
                .child(Text::new(parts::pct(p.cpu_percent)).class(cpu_class))
                .child(Text::new(parts::kb(p.rss_kb)).class("num col-mem")),
        )
    };
    let mut col = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(GestureDetector::new().on_click(move || st.selected.set(if selected { None } else { Some(pid) })).child(main));
    if selected {
        let state = match p.state {
            'R' => t!("выполняется"),
            'S' => t!("ждёт"),
            'D' => t!("ждёт ввода-вывода"),
            'Z' => t!("зомби"),
            'T' | 't' => t!("остановлен"),
            'I' => t!("простаивает"),
            c => c.to_string(),
        };
        col = col.child(parts::kv_list(vec![
            (t!("Команда"), if p.cmdline.is_empty() { p.name.clone() } else { p.cmdline.clone() }),
            (t!("PID / родитель"), format!("{} / {}", p.pid, p.ppid)),
            (t!("Пользователь"), p.user.clone()),
            (t!("Состояние"), state),
            (t!("Потоков"), p.threads.to_string()),
            (t!("Работает"), parts::duration(p.age_secs)),
        ]));
        let name = p.name.clone();
        let mut actions = Row::new().gap(8.0).main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center);
        if p.is_kernel() {
            actions = actions.child(Text::new(t!("Поток ядра — завершить нельзя")).class("muted small"));
        } else if st.confirm.get() == Some(Confirm::Process(pid)) {
            let n = name.clone();
            actions = actions
                .child(Text::new(t!("Убить без сохранения данных?")).max_lines(2).class("muted small grow"))
                .child(button(t!("Убить"), icons::STOP, "danger", move || signal(st, pid, &n, libc::SIGKILL)))
                .child(button(t!("Отмена"), icons::CLOSE, "", move || st.confirm.set(None)));
        } else {
            let n = name.clone();
            actions = actions
                .child(button(t!("Завершить"), icons::CANCEL, "", move || signal(st, pid, &n, libc::SIGTERM)))
                .child(button(t!("Убить"), icons::STOP, "danger-soft", move || st.confirm.set(Some(Confirm::Process(pid)))));
        }
        col = col.child(actions);
    }
    boxed(DecoratedBox::new().child(col).class(if selected { "row-item selected" } else { "row-item" }))
}

fn avatar_color(key: &str) -> Color {
    const C: [&str; 8] = ["#4f9dff", "#2ec27e", "#b77cff", "#ff9f43", "#22c3e6", "#f06292", "#a3d65c", "#ff6b5b"];
    let h = key.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    Color::from_hex(C[h as usize % C.len()])
}
