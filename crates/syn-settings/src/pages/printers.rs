//! Принтеры: очереди CUPS — состояние, по умолчанию, пауза, пробная страница,
//! описание и расположение, настройки печати по умолчанию (бумага, двусторонняя,
//! цвет, качество и прочие настройки драйвера — `lpoptions` пользователя),
//! очередь заданий с отменой, поиск и добавление принтеров (USB, сеть, по адресу;
//! IPP Everywhere или драйвер из `lpinfo -m`), удаление.
//!
//! Всё — через `syngui::print::cups` (CUPS разрешает это пользователям из
//! `SystemGroup`, обычно wheel), без своего демона.

use std::cell::Cell;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use syngui::print::cups::{self, Device, Driver, Job, Printer, PrinterState};
use syngui::print::{choice_label, duplex_label, option_label, paper, Ppd};

use crate::state;
use crate::ui::*;

/// Пояснение внутри карточки (с отступами строки).
fn hint(text: impl AsRef<str>) -> W {
    boxed(Text::new(tl(text)).class("row-hint pad"))
}

type R<T> = Option<std::result::Result<T, String>>;

const ICON_TUNE: &str = "\u{e429}";

/// Добавление принтера: устройство и подобранные драйверы.
#[derive(Clone, PartialEq)]
struct Adding {
    device: Device,
    drivers: Option<Vec<Driver>>,
}

#[derive(Clone, Copy)]
struct Sigs {
    list: RwSignal<R<Vec<Printer>>>,
    jobs: RwSignal<Vec<Job>>,
    /// Принтер, чьи настройки раскрыты, и его описание.
    open: RwSignal<Option<String>>,
    ppd: RwSignal<Option<(String, std::result::Result<Ppd, String>, Vec<(String, String)>)>>,
    delete: RwSignal<Option<String>>,
    /// Поиск устройств: `None` — не искали, `Some(None)` — идёт.
    scan: RwSignal<Option<R<Vec<Device>>>>,
    adding: RwSignal<Option<Adding>>,
    manual: RwSignal<String>,
}

thread_local! {
    static SIGS: Cell<Option<Sigs>> = const { Cell::new(None) };
}

fn sigs() -> Sigs {
    SIGS.with(|c| match c.get() {
        Some(s) => s,
        None => {
            let s = Sigs {
                list: use_signal(None),
                jobs: use_signal(Vec::new()),
                open: use_signal(None),
                ppd: use_signal(None),
                delete: use_signal(None),
                scan: use_signal(None),
                adding: use_signal(None),
                manual: use_signal(String::new()),
            };
            c.set(Some(s));
            s
        }
    })
}

fn reload(s: Sigs) {
    std::thread::spawn(move || {
        let r = cups::printers();
        let jobs = cups::jobs(None).unwrap_or_default();
        run_on_main_thread(move || {
            s.list.set(Some(r));
            s.jobs.set(jobs);
        });
    });
}

fn load_ppd(s: Sigs, name: String) {
    std::thread::spawn(move || {
        let ppd = cups::printer_ppd(&name);
        let user = cups::user_defaults(&name);
        run_on_main_thread(move || {
            if s.open.get_untracked().as_deref() == Some(name.as_str()) {
                s.ppd.set(Some((name, ppd, user)));
            }
        });
    });
}

/// Действие в фоне с сообщением и обновлением списка.
fn act(label: impl AsRef<str>, f: impl FnOnce() -> std::result::Result<(), String> + Send + 'static) {
    let s = sigs();
    let label = tl(label);
    state::toast(format!("{label}…"));
    std::thread::spawn(move || {
        let r = f();
        run_on_main_thread(move || {
            state::toast(match r {
                Ok(()) => t!("{label}: готово", label = label),
                Err(e) => format!("{label}: {e}"),
            });
            reload(s);
        });
    });
}

pub fn printers() -> W {
    let s = sigs();
    reload(s);
    let list = Reactive::new(move || -> Vec<W> {
        let mut rows: Vec<W> = Vec::new();
        let jobs = s.jobs.get();
        match s.list.get() {
            None => rows.push(hint(&t!("Читаю список принтеров…"))),
            Some(Err(e)) => {
                rows.push(hint(&t!("Служба печати CUPS недоступна: {e}", e = e)));
                rows.push(hint(&t!("Установите пакет cups и включите службу: systemctl enable --now cups")));
            }
            Some(Ok(l)) if l.is_empty() => rows.push(hint(&t!("Принтеров пока нет. Найдите принтер ниже или добавьте по адресу."))),
            Some(Ok(l)) => {
                for p in l {
                    let n = jobs.iter().filter(|j| j.printer == p.name).count();
                    rows.push(printer_row(s, &p, n));
                    if s.open.get().as_deref() == Some(p.name.as_str()) {
                        rows.extend(printer_settings(s, &p));
                    }
                }
            }
        }
        vec![group(&t!("Принтеры"), rows)]
    });
    let queue = Reactive::new(move || -> Vec<W> {
        let jobs = s.jobs.get();
        if jobs.is_empty() {
            return vec![];
        }
        let names: Vec<(String, String)> = match s.list.get() {
            Some(Ok(l)) => l.iter().map(|p| (p.name.clone(), p.title().to_string())).collect(),
            _ => Vec::new(),
        };
        let mut rows: Vec<W> = jobs.iter().map(|j| job_row(j, &names)).collect();
        let ids: Vec<String> = jobs.iter().map(|j| j.id.clone()).collect();
        rows.push(row_inline(
            t!("Отменить все задания"),
            "",
            button(&t!("Отменить все"), move || {
                let ids = ids.clone();
                act(n_!("Отмена заданий"), move || {
                    for id in &ids {
                        cups::cancel_job(id)?;
                    }
                    Ok(())
                });
            }),
        ));
        vec![group(&t!("Очередь печати"), rows)]
    });
    let add = Reactive::new(move || -> Vec<W> {
        if let Some(a) = s.adding.get() {
            return vec![add_form(s, a)];
        }
        vec![discover(s)]
    });
    page(
        t!("Принтеры"),
        t!("Принтеры CUPS: состояние, настройки печати по умолчанию, очередь заданий, поиск и добавление."),
        vec![
            boxed(Row::new().gap(8.0).child(DecoratedBox::new().class("grow")).child(button(&t!("Обновить"), move || reload(s)))),
            boxed(list),
            boxed(queue),
            boxed(add),
        ],
    )
}

fn state_text(p: &Printer) -> String {
    match p.state {
        PrinterState::Idle => t!("готов"),
        PrinterState::Printing => t!("печатает"),
        PrinterState::Disabled => t!("приостановлен"),
    }
}

fn printer_row(s: Sigs, p: &Printer, jobs: usize) -> W {
    let name = p.name.clone();
    if s.delete.get().as_deref() == Some(name.as_str()) {
        let n = name.clone();
        return row_inline(
            &t!("Удалить «{name}»?", name = p.title()),
            &t!("Очередь и её задания будут удалены"),
            Row::new().gap(6.0).child(button(&t!("Отмена"), move || s.delete.set(None))).child(primary_button(&t!("Удалить"), move || {
                s.delete.set(None);
                s.open.set(None);
                let n = n.clone();
                act(n_!("Удаление принтера"), move || cups::remove_printer(&n));
            })),
        );
    }
    let mut hint = vec![state_text(p)];
    if p.is_default {
        hint.insert(0, t!("по умолчанию"));
    }
    if !p.location.is_empty() {
        hint.push(p.location.clone());
    }
    if jobs > 0 {
        hint.push(t!("заданий: {n}", n = jobs));
    }
    let msg = p.message_text();
    if !msg.is_empty() {
        hint.push(msg);
    }
    let mut controls = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if !p.is_default {
        let n = name.clone();
        controls = controls.child(button(&t!("По умолчанию"), move || {
            let n = n.clone();
            act(n_!("Принтер по умолчанию"), move || cups::set_default(&n));
        }));
    }
    let paused = p.state == PrinterState::Disabled;
    let n = name.clone();
    controls = controls.child(button(if paused { t!("Возобновить") } else { t!("Приостановить") }, move || {
        let n = n.clone();
        act(if paused { n_!("Возобновление принтера") } else { n_!("Приостановка принтера") }, move || cups::set_enabled(&n, paused));
    }));
    let n = name.clone();
    controls = controls.child(icon_button(ICON_TUNE, move || {
        if s.open.get_untracked().as_deref() == Some(n.as_str()) {
            s.open.set(None);
            s.ppd.set(None);
        } else {
            s.open.set(Some(n.clone()));
            s.ppd.set(None);
            load_ppd(s, n.clone());
        }
    }));
    let n = name.clone();
    controls = controls.child(icon_button(icons::DELETE, move || s.delete.set(Some(n.clone()))));
    if narrow() {
        return row_wide(p.title(), hint.join(" · "), controls);
    }
    row_inline(p.title(), hint.join(" · "), controls)
}

/// Раскрытые настройки принтера.
fn printer_settings(s: Sigs, p: &Printer) -> Vec<W> {
    let name = p.name.clone();
    let mut rows: Vec<W> = Vec::new();
    // Описание и расположение.
    let desc = use_signal(p.description.clone());
    let loc = use_signal(p.location.clone());
    let n = name.clone();
    rows.push(row(
        t!("Название и расположение"),
        if p.uri.is_empty() { String::new() } else { p.uri.clone() },
        Row::new()
            .gap(6.0)
            .child(TextField::new().text(p.description.clone()).placeholder(t!("Название")).on_change(move |t| desc.set(t.to_string())).class("grow"))
            .child(TextField::new().text(p.location.clone()).placeholder(t!("Где стоит")).on_change(move |t| loc.set(t.to_string())).class("grow"))
            .child(button(&t!("Сохранить"), move || {
                let (n, d, l) = (n.clone(), desc.get_untracked(), loc.get_untracked());
                act(n_!("Описание принтера"), move || cups::set_info(&n, &d, &l));
            })),
    ));
    let n = name.clone();
    rows.push(row_inline(
        t!("Пробная страница"),
        t!("Напечатать страницу CUPS для проверки принтера"),
        button(&t!("Напечатать"), move || {
            let n = n.clone();
            act(n_!("Пробная страница"), move || cups::print_test_page(&n).map(|_| ()));
        }),
    ));
    // Настройки печати по умолчанию.
    match s.ppd.get() {
        Some((pn, Ok(ppd), user)) if pn == name => {
            rows.push(hint(t!("Настройки печати по умолчанию: с ними открывается окно печати в программах.")));
            for o in &ppd.options {
                if o.key == "PageRegion" || o.choices.len() < 2 {
                    continue;
                }
                let current = user.iter().find(|(k, _)| *k == o.key).map(|(_, v)| v.clone()).filter(|v| o.choice(v).is_some()).unwrap_or_else(|| o.default.clone());
                let items: Vec<(String, String)> = o
                    .choices
                    .iter()
                    .map(|c| {
                        let label = match o.key.as_str() {
                            "PageSize" => paper::paper(&c.key, Some(&c.label)).map(|p| p.full_label()).unwrap_or_else(|| c.label.clone()),
                            "Duplex" => duplex_label(&c.key, &c.label),
                            _ => choice_label(&o.key, &c.key, &c.label),
                        };
                        (c.key.clone(), label)
                    })
                    .collect();
                let (n, key) = (name.clone(), o.key.clone());
                let mut dd = Dropdown::new().width(260.0).max_height(320.0);
                for (v, l) in items {
                    dd = dd.item(DropdownItem::new(v, l));
                }
                let dd = dd.selected(current).on_change(move |v: &str| {
                    let (n, key, v) = (n.clone(), key.clone(), v.to_string());
                    act(n_!("Настройки принтера"), move || cups::set_user_defaults(&n, &[(key, v)]));
                });
                let label = option_label(&o.key, &o.label);
                rows.push(row(label, "", dd));
            }
        }
        Some((pn, Err(e), _)) if pn == name => rows.push(hint(t!("Описание драйвера недоступно: {e}", e = e))),
        _ => rows.push(hint(t!("Загружаю настройки принтера…"))),
    }
    rows
}

fn job_row(j: &Job, names: &[(String, String)]) -> W {
    let printer = names.iter().find(|(n, _)| *n == j.printer).map(|(_, t)| t.clone()).unwrap_or_else(|| j.printer.clone());
    let size = if j.size > 0 { format!(" · {} {}", (j.size + 1023) / 1024, t!("КБ")) } else { String::new() };
    let id = j.id.clone();
    let title = if j.title.is_empty() { j.id.clone() } else { j.title.clone() };
    row_inline(
        &title,
        format!("{printer} · {} · {}{size}", j.id, j.user),
        button(&t!("Отменить"), move || {
            let id = id.clone();
            act(n_!("Отмена задания"), move || cups::cancel_job(&id));
        }),
    )
}

fn scan(s: Sigs) {
    s.scan.set(Some(None));
    std::thread::spawn(move || {
        let r = cups::devices(6);
        run_on_main_thread(move || s.scan.set(Some(Some(r))));
    });
}

/// Поиск устройств и добавление по адресу.
fn discover(s: Sigs) -> W {
    let mut rows: Vec<W> = Vec::new();
    let known: Vec<String> = match s.list.get() {
        Some(Ok(l)) => l.iter().map(|p| p.uri.clone()).collect(),
        _ => Vec::new(),
    };
    match s.scan.get() {
        None => rows.push(row_inline(t!("Найти принтеры"), t!("USB и сеть: Wi-Fi, Ethernet (DNS-SD, IPP)"), primary_button(&t!("Найти"), move || scan(s)))),
        Some(None) => rows.push(hint(&t!("Ищу принтеры… (до 10 секунд)"))),
        Some(Some(Err(e))) => rows.push(hint(&t!("Поиск не удался: {e}", e = e))),
        Some(Some(Ok(devs))) => {
            let fresh: Vec<Device> = devs.into_iter().filter(|d| !known.contains(&d.uri) && d.class != "file").collect();
            if fresh.is_empty() {
                rows.push(hint(&t!("Новых принтеров не найдено. Включите принтер и подключите его к этой сети или по USB.")));
            }
            for d in fresh {
                let kind = match d.class.as_str() {
                    "direct" => t!("USB"),
                    _ if d.driverless() => t!("сеть, без драйвера"),
                    _ => t!("сеть"),
                };
                let dev = d.clone();
                rows.push(row_inline(d.title(), format!("{kind} · {}", d.uri), button(&t!("Добавить"), move || start_add(s, dev.clone()))));
            }
            rows.push(row_inline(t!("Искать снова"), "", button(&t!("Найти"), move || scan(s))));
        }
    }
    rows.push(row(
        t!("По адресу"),
        t!("IP-адрес или имя принтера в сети (IPP), либо полный адрес: ipp://, ipps://, socket://, lpd://"),
        Row::new()
            .gap(6.0)
            .child(TextField::new().text(s.manual.get_untracked()).placeholder("192.168.1.50").on_change(move |t| s.manual.set(t.trim().to_string())).class("grow"))
            .child(button(&t!("Добавить"), move || {
                let a = s.manual.get_untracked();
                if a.is_empty() {
                    return;
                }
                let uri = if a.contains("://") { a.clone() } else { format!("ipp://{a}/ipp/print") };
                start_add(s, Device { uri, class: "network".into(), info: a, ..Default::default() });
            })),
    ));
    group(&t!("Добавить принтер"), rows)
}

fn start_add(s: Sigs, device: Device) {
    s.adding.set(Some(Adding { device: device.clone(), drivers: None }));
    std::thread::spawn(move || {
        let all = cups::drivers().unwrap_or_default();
        let mut list: Vec<Driver> = Vec::new();
        if device.driverless() || device.class == "network" {
            list.push(Driver { model: "everywhere".into(), name: t!("Без драйвера (IPP Everywhere)") });
        }
        list.extend(cups::rank_drivers(&all, device.title()));
        // Ничего не подошло — общие драйверы.
        if list.len() <= 1 {
            list.extend(all.iter().filter(|d| d.name.to_lowercase().contains("generic")).cloned());
        }
        run_on_main_thread(move || {
            if let Some(mut a) = s.adding.get_untracked() {
                if a.device.uri == device.uri {
                    a.drivers = Some(list);
                    s.adding.set(Some(a));
                }
            }
        });
    });
}

fn add_form(s: Sigs, a: Adding) -> W {
    let taken: Vec<String> = match s.list.get_untracked() {
        Some(Ok(l)) => l.iter().map(|p| p.name.clone()).collect(),
        _ => Vec::new(),
    };
    let title = a.device.title().to_string();
    let queue = use_signal(cups::queue_name(&title, &taken));
    let desc = use_signal(title.clone());
    let loc = use_signal(a.device.location.clone());
    let driver = use_signal(a.drivers.as_ref().and_then(|d| d.first()).map(|d| d.model.clone()).unwrap_or_default());
    let make_default = use_signal(matches!(s.list.get_untracked(), Some(Ok(ref l)) if l.is_empty()));
    let mut rows: Vec<W> = vec![
        row_inline(t!("Устройство"), a.device.uri.clone(), Text::new("")),
        row(t!("Название"), t!("Как принтер показывается в окне печати"), TextField::new().text(title.clone()).on_change(move |t| desc.set(t.to_string())).class("grow")),
        row(t!("Имя очереди"), t!("Латиница, цифры, «_» и «-»"), TextField::new().text(queue.get_untracked()).on_change(move |t| queue.set(t.trim().to_string())).class("grow")),
        row(t!("Расположение"), t!("Необязательно: «Кабинет», «2 этаж»"), TextField::new().text(a.device.location.clone()).on_change(move |t| loc.set(t.to_string())).class("grow")),
    ];
    match &a.drivers {
        None => rows.push(hint(&t!("Подбираю драйвер…"))),
        Some(list) if list.is_empty() => rows.push(hint(&t!("Подходящих драйверов нет. Установите драйвер производителя (пакет из AUR) и попробуйте снова."))),
        Some(list) => {
            let mut dd = Dropdown::new().width(360.0).max_height(360.0);
            for d in list {
                dd = dd.item(DropdownItem::new(d.model.clone(), d.name.clone()));
            }
            rows.push(row(t!("Драйвер"), "", dd.selected(driver.get_untracked()).on_change(move |v: &str| driver.set(v.to_string()))));
        }
    }
    rows.push(row_inline(t!("Сделать принтером по умолчанию"), "", Toggle::with_state(make_default.get_untracked()).on_change(move |v| make_default.set(v))));
    let uri = a.device.uri.clone();
    rows.push(boxed(
        Row::new()
            .gap(8.0)
            .class("setting-row")
            .child(DecoratedBox::new().class("grow"))
            .child(button(&t!("Отмена"), move || s.adding.set(None)))
            .child(primary_button(&t!("Добавить"), move || {
                let (q, d, l, m, def, uri) = (queue.get_untracked(), desc.get_untracked(), loc.get_untracked(), driver.get_untracked(), make_default.get_untracked(), uri.clone());
                if q.is_empty() || m.is_empty() {
                    state::toast(t!("Укажите имя очереди и драйвер"));
                    return;
                }
                s.adding.set(None);
                s.scan.set(None);
                s.manual.set(String::new());
                act(n_!("Добавление принтера"), move || {
                    cups::add_printer(&q, &uri, &m, &d, &l)?;
                    if def {
                        cups::set_default(&q)?;
                    }
                    Ok(())
                });
            })),
    ));
    group(&t!("Новый принтер"), rows)
}
