//! Вид «Программ»: рабочий стол — разделы, список, подробности; телефон —
//! стек «список → пакет» с нижней навигацией. Поверх — очередь, окно
//! удаления с зависимостями, просмотр снимков, контекстное меню, пароль.

use std::time::Duration;

use synsystem::packages::{self as pk, CatalogApp, Op, Pkg, RemoveMode, Source, Update};
use syngui::async_runtime::run_on_main_thread;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::{EventHook, KeyReply, MenuItem, PopupMenu};
use syngui::overlay::WindowResizeRegion;
use syngui::{GestureDetector, ShowIf};

use crate::*;
use crate::Tab;

pub type W = Box<dyn Widget>;

/// Миниатюра снимка экрана в ленте.
const SHOT_W: f32 = 300.0;
const SHOT_H: f32 = 180.0;

const CHECK_ON: &str = "\u{E834}";
const CHECK_OFF: &str = "\u{E835}";

pub fn root(st: St) -> W {
    let narrow = syngui::viewport::viewport_below(720.0);
    let body = Reactive::new(move || -> Vec<W> {
        let phone_ui = narrow.get();
        // Телефон: рамку (закрыть, свернуть) рисует композитор; рабочий стол — свой заголовок.
        run_on_main_thread(move || syngui::signal::set_decorations(phone_ui));
        vec![if phone_ui { phone(st) } else { crate::desk::desktop(st) }]
    });
    // Слои поверх окна выравниваются снаружи Reactive: он отдаёт детям
    // свободные ограничения, и слой внутри него ужимается в угол.
    // Column пропускает касания мимо подсказки.
    let toast = Column::new()
        .main_axis_alignment(MainAxisAlignment::End)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Reactive::new(move || -> Vec<W> {
            let t = st.toast.get();
            if t.is_empty() {
                return vec![];
            }
            vec![Box::new(DecoratedBox::new().child(Text::new(t).max_lines(2).class("toast-text")).class("toast"))]
        }))
        .class("toast-place");
    // Подсказка гаснет через 4 с.
    create_effect(move || {
        let t = st.toast.get();
        if !t.is_empty() {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(4));
                run_on_main_thread(move || {
                    if st.toast.get_untracked() == t {
                        st.toast.set(String::new());
                    }
                });
            });
        }
    });
    let menu = Reactive::new(move || -> Vec<W> {
        let items = st.menu_items.get();
        vec![Box::new(PopupMenu::new().items(items).position(st.menu_pos).is_open(st.menu_open).min_width(250.0).on_select(move |id| {
            st.menu_open.set(false);
            crate::menu_selected(id);
        }))]
    });
    let hook = EventHook::new()
            .on_key_down(move |k, _| {
                if matches!(k, syngui::input::Key::Escape) && go_back(st) {
                    KeyReply::Handled
                } else {
                    KeyReply::Ignore
                }
            })
            .child(
                GestureDetector::new().on_back(move || go_back(st)).child(
                    Stack::new()
                        .fit(StackFit::Expand)
                        .child(DecoratedBox::new().child(body).class("root"))
                        .child(toast)
                        .child(shot_viewer(st))
                        .child(remove_dialog(st))
                        .child(auth_overlay(st))
                        .child(menu),
                ),
            );
    // Своя рамка окна: скругление и обводка как у окон композитора; развёрнутое — без них
    // (телефон: окно на весь экран, рамку рисует композитор).
    let frame = DecoratedBox::new().class("window-frame").clip(true).child(hook);
    Box::new(WindowResizeRegion::new().over_content(true).inset(5.0).child(frame))
}

/// «Назад»: снимок, окно удаления, пакет, категория каталога.
pub fn go_back(st: St) -> bool {
    if st.menu_open.get_untracked() {
        st.menu_open.set(false);
        true
    } else if st.shot.get_untracked().is_some() {
        st.shot.set(None);
        true
    } else if st.remove_ask.get_untracked().is_some() {
        st.remove_ask.set(None);
        true
    } else if st.selected.get_untracked().is_some() {
        st.selected.set(None);
        true
    } else if st.tab.get_untracked() == Tab::Explore && st.category.get_untracked().is_some() {
        st.category.set(None);
        true
    } else {
        false
    }
}

pub fn open_tab(st: St, t: Tab) {
    st.tab.set(t);
    if t == Tab::Updates && st.updates.get_untracked().is_none() {
        check_updates(st);
    }
    if t == Tab::Aur && st.aur_updates.get_untracked().is_none() {
        check_aur_updates(st);
    }
}

/// Слой поверх окна, видимый, пока `on` — `true`.
pub fn overlay(on: impl Fn() -> bool + Send + Sync + 'static, child: impl Widget + 'static) -> W {
    let shown = use_signal(0usize);
    create_effect(move || shown.set(usize::from(on())));
    Box::new(ShowIf::new(1, shown).child(child))
}

/// Окно «нужны права администратора» поверх всего: пароль уходит агенту polkit.
pub fn auth_overlay(st: St) -> W {
    let card = Reactive::new(move || -> Vec<W> {
        let Some(AuthView(req)) = st.auth.get() else {
            return vec![];
        };
        let pass = use_signal(String::new());
        let (r1, r2, r3) = (req.clone(), req.clone(), req.clone());
        let submit = move || {
            r1.respond(Some(pass.get_untracked()));
            st.auth.set(None);
        };
        let s2 = submit.clone();
        let cancel = move || {
            r2.respond(None);
            st.auth.set(None);
        };
        let mut card = Column::new()
            .gap(12.0)
            .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new("\u{E897}").class("auth-icon")).child(Text::new("Нужны права администратора").class("auth-title grow")))
            .child(Text::new(r3.message.clone()).class("desc"))
            .child(Text::new(format!("Пароль пользователя {}", r3.user)).class("muted"));
        if let Some(e) = &r3.error {
            card = card.child(Text::new(e.clone()).class("auth-error"));
        }
        card = card
            .child(TextField::new().obscure(r3.secret).autofocus(true).placeholder(if r3.prompt.is_empty() { "Пароль".to_string() } else { r3.prompt.trim_end_matches(':').to_string() }).on_change(move |t| pass.set(t.to_string())).on_submit(move |_| s2()))
            .child(
                Row::new()
                    .gap(8.0)
                    .main_axis_alignment(MainAxisAlignment::End)
                    .child(Button::new("Отмена").on_click(cancel))
                    .child(Button::new("Подтвердить").class("primary").on_click(submit)),
            );
        vec![Box::new(DecoratedBox::new().child(card).class("auth-card"))]
    });
    overlay(move || st.auth.get().is_some(), GestureDetector::new().on_click(|| {}).child(DecoratedBox::new().child(card).class("auth-scrim")))
}

/// Удаление упёрлось в зависимости: удалить каскадом, принудительно или отменить.
pub fn remove_dialog(st: St) -> W {
    let card = Reactive::new(move || -> Vec<W> {
        let Some(ask) = st.remove_ask.get() else {
            return vec![];
        };
        // Кому нужен каждый удаляемый.
        let mut by: Vec<(String, Vec<String>)> = Vec::new();
        for (p, who) in &ask.check.blockers {
            match by.iter_mut().find(|(n, _)| n == p) {
                Some((_, v)) => v.push(who.clone()),
                None => by.push((p.clone(), vec![who.clone()])),
            }
        }
        let mut list = Column::new().gap(4.0);
        for (p, who) in by {
            list = list.child(Text::new(format!("{p} нужен для: {}", who.join(", "))).class("dlg-line"));
        }
        let extra: Vec<String> = ask.check.cascade.iter().filter(|n| !ask.names.contains(n)).cloned().collect();
        let mut col = Column::new()
            .gap(12.0)
            .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new("\u{E002}").class("dlg-icon")).child(Text::new("Удаляемые пакеты нужны другим").class("auth-title grow")))
            .child(Text::new("pacman не удалит пакет, от которого зависят другие. Как поступить?").class("desc"))
            .child(DecoratedBox::new().child(ScrollView::new().vertical().child(list)).class("dlg-box"));
        let a1 = ask.clone();
        let cascade = Column::new()
            .gap(4.0)
            .child(Button::new(format!("Удалить вместе с зависящими ({})", packages_word(ask.check.cascade.len()))).class("danger").disabled(ask.check.cascade.is_empty()).on_click(move || run_remove(st, a1.clone(), RemoveMode::Cascade)))
            .child(Text::new(if extra.is_empty() { "Каскадное удаление недоступно".to_string() } else { format!("Уйдут также: {}", extra.join(", ")) }).max_lines(3).class("muted"));
        let a2 = ask.clone();
        let force = Column::new()
            .gap(4.0)
            .child(Button::new("Удалить принудительно").on_click(move || run_remove(st, a2.clone(), RemoveMode::Force)))
            .child(Text::new("Без проверки зависимостей (pacman -Rdd): зависящие программы останутся и могут перестать работать.").class("muted"));
        col = col.child(cascade).child(force).child(Row::new().main_axis_alignment(MainAxisAlignment::End).child(Button::new("Отмена").on_click(move || st.remove_ask.set(None))));
        vec![Box::new(DecoratedBox::new().child(col).class("auth-card dlg-card"))]
    });
    overlay(move || st.remove_ask.get().is_some(), GestureDetector::new().on_click(|| {}).child(DecoratedBox::new().child(card).class("auth-scrim")))
}

/// Снимок экрана на всё окно: стрелки — соседние, клик мимо — закрыть.
pub fn shot_viewer(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let (Some(i), Some(m)) = (st.shot.get(), st.media.get()) else {
            return vec![];
        };
        let n = m.shots.len();
        let Some(path) = m.shots.get(i).cloned() else {
            return vec![];
        };
        let nav = |glyph: &'static str, to: usize, on: bool| -> W {
            if !on {
                return Box::new(DecoratedBox::new().class("viewer-nav-off"));
            }
            Box::new(GestureDetector::new().on_click(move || st.shot.set(Some(to))).child(DecoratedBox::new().child(Icon::new(glyph).class("viewer-nav-icon")).class("viewer-nav")))
        };
        // Размер — от окна: в ряду со стрелками картинка иначе берёт свой собственный.
        let vp = syngui::viewport::viewport_size().get();
        let (iw, ih) = ((vp.width - 220.0).max(100.0), (vp.height - 110.0).max(100.0));
        let img = DecoratedBox::new()
            .child(Image::new(path.to_string_lossy()).fit(ImageFit::Contain).style("width", StyleValue::px(iw)).style("height", StyleValue::px(ih)))
            .style("width", StyleValue::px(iw))
            .style("height", StyleValue::px(ih));
        vec![Box::new(
            Column::new()
                .gap(10.0)
                .child(
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Text::new(format!("{} из {n}", i + 1)).class("viewer-count grow"))
                        .child(GestureDetector::new().on_click(move || st.shot.set(None)).child(DecoratedBox::new().child(Icon::new("\u{E5CD}").class("viewer-nav-icon")).class("viewer-nav"))),
                )
                .child(
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(nav("\u{E5CB}", i.saturating_sub(1), i > 0))
                        .child(GestureDetector::new().on_click(|| {}).child(img))
                        .child(nav("\u{E5CC}", i + 1, i + 1 < n))
                        .class("grow"),
                )
                .class("grow"),
        )]
    });
    overlay(move || st.shot.get().is_some(), GestureDetector::new().on_click(move || st.shot.set(None)).child(DecoratedBox::new().child(body).class("viewer")))
}

pub fn tab_content(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        vec![match st.tab.get() {
            Tab::Explore => explore_view(st),
            Tab::Aur => aur_view(st),
            Tab::Installed => installed_view(st),
            Tab::Updates => updates_view(st),
            Tab::Jobs => jobs_view(st),
        }]
    }))
}

/// Счётчик у раздела: задачи в работе и очередь, доступные обновления.
pub fn tab_badge(st: St, t: Tab) -> Option<String> {
    match t {
        Tab::Jobs => {
            let running = st.jobs.get().iter().filter(|j| j.done.is_none()).count();
            let q = st.queue.get().len();
            (running + q > 0).then(|| (running + q).to_string())
        }
        Tab::Updates => st.updates.get().filter(|u| !u.is_empty()).map(|u| u.len().to_string()),
        Tab::Aur => st.aur_updates.get().filter(|u| !u.is_empty()).map(|u| u.len().to_string()),
        _ => None,
    }
}

pub fn phone(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let sel = st.selected.get();
        let open = sel.is_some();
        vec![Box::new(
            AnimatedSwitcher::new(if open { 2u64 } else { 1 }, move || -> Box<dyn Widget> {
                match st.selected.get_untracked() {
                    Some(p) => Box::new(
                        Column::new()
                            .child(
                                Row::new()
                                    .gap(6.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .child(GestureDetector::new().on_click(move || st.selected.set(None)).child(DecoratedBox::new().child(Icon::new("\u{E5C4}").class("back-icon")).class("back")))
                                    .child(Text::new(title_of(&p)).max_lines(1).class("bar-title grow"))
                                    .class("bar"),
                            )
                            .child(DecoratedBox::new().child(details_view(st, p)).class("grow")),
                    ),
                    None => tab_content(st),
                }
            })
            .directional(true)
            .slide(48.0, 0.0)
            .duration_ms(240)
            .animate_size(false)
            .class("grow"),
        )]
    });
    let navbar = Reactive::new(move || -> Vec<W> {
        let cur = st.tab.get();
        let mut row = Row::new().main_axis_alignment(MainAxisAlignment::SpaceAround);
        for t in Tab::ALL {
            let mut item = Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().child(Icon::new(t.icon()).class("nb-icon")).class(if cur == t { "nb-pill nb-pill-on" } else { "nb-pill" }))
                .child(Text::new(t.label()).max_lines(1).class("nb-label"));
            if tab_badge(st, t).is_some() {
                item = item.child(DecoratedBox::new().class("nb-dot"));
            }
            row = row.child(GestureDetector::new().on_click(move || {
                st.selected.set(None);
                open_tab(st, t);
            }).child(DecoratedBox::new().child(item).class("nb-item")));
        }
        vec![Box::new(row.class("navbar"))]
    });
    Box::new(Column::new().child(DecoratedBox::new().child(body).class("grow")).child(queue_bar(st)).child(navbar))
}

/// Полоса очереди внизу списка: что отмечено и «Применить».
pub fn queue_bar(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let q = st.queue.get();
        if q.is_empty() {
            return vec![];
        }
        let inst = q.iter().filter(|x| x.act != QAct::Remove).count();
        let rm = q.len() - inst;
        let mut parts = Vec::new();
        if inst > 0 {
            parts.push(format!("установить {inst}"));
        }
        if rm > 0 {
            parts.push(format!("удалить {rm}"));
        }
        let names: Vec<String> = q.iter().map(|x| x.name.clone()).collect();
        vec![Box::new(
            DecoratedBox::new()
                .child(
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Icon::new("\u{E03C}").class("qbar-icon"))
                        .child(
                            GestureDetector::new().on_click(move || open_tab(st, Tab::Jobs)).child(
                                Column::new()
                                    .gap(1.0)
                                    .child(Text::new(format!("Очередь: {}", parts.join(", "))).max_lines(1).class("qbar-title"))
                                    .child(Text::new(names.join(", ")).max_lines(1).class("qbar-sub")),
                            ),
                        )
                        .child(DecoratedBox::new().class("grow"))
                        .child(Button::new("Очистить").class("small").on_click(move || st.queue.set(Vec::new())))
                        .child(Button::new("Применить").class("primary").on_click(move || apply_queue(st))),
                )
                .class("qbar"),
        )]
    }))
}

pub fn chip(label: impl Into<String>, class: &str) -> syngui::widget::StyledWidget<DecoratedBox> {
    DecoratedBox::new().child(Text::new(label.into()).class("chip-text")).class(format!("chip {class}"))
}

pub fn source_chip(p: &Pkg) -> syngui::widget::StyledWidget<DecoratedBox> {
    match &p.source {
        Source::Aur => chip("AUR", "chip-aur"),
        Source::Local => chip("не из репозитория", "chip-local"),
        Source::Repo(r) if r.is_empty() => chip("репозиторий", "chip-repo"),
        Source::Repo(r) => chip(r.clone(), "chip-repo"),
    }
}

/// Название программы из каталога, иначе имя пакета.
pub fn title_of(p: &Pkg) -> String {
    app_meta(&p.name).map(|m| m.title).unwrap_or_else(|| p.name.clone())
}

/// Значок: из темы по имени AppStream или пакета, иначе кэш каталога, иначе глиф.
pub fn app_icon(name: &str, class: &str) -> W {
    let meta = app_meta(name);
    let path = meta
        .as_ref()
        .and_then(|m| m.icon.as_deref())
        .and_then(synshell_common::xdg::lookup_icon)
        .or_else(|| synshell_common::xdg::lookup_icon(name))
        .or_else(|| meta.as_ref().and_then(|m| m.icon64.as_ref()).and_then(|j| media::icon_png(j)));
    match path {
        Some(p) => Box::new(Image::new(p.to_string_lossy()).fit(ImageFit::Contain).placeholder(false).class(class.to_string())),
        None => Box::new(DecoratedBox::new().child(Icon::new("\u{E1BD}").class("pkg-glyph")).class(format!("pkg-glyph-box {class}-glyph"))),
    }
}

/// Флажок очереди. Щелчок по строке открывает пакет; по флажку — ставит в
/// очередь: флажок ловит щелчок сам (внутренний детектор жестов).
pub fn check_icon(on: bool) -> W {
    Box::new(DecoratedBox::new().child(Icon::new(if on { CHECK_ON } else { CHECK_OFF }).class(if on { "check check-on" } else { "check" })).class("check-box"))
}

/// Пункты контекстного меню пакета. `update` — строка из «Обновлений».
pub fn pkg_menu(st: St, p: Pkg, at: syngui::core::Point, update: bool) {
    let mut items = vec![MenuItem::new("details", "Подробности").icon("\u{E88E}")];
    let q = queued(st, &p.name);
    let installed = p.installed.is_some();
    if q.is_some() {
        items.push(MenuItem::new("unqueue", "Убрать из очереди").icon("\u{E15C}"));
    } else if installed {
        items.push(MenuItem::new("q", "Удалить (в очередь)").icon("\u{E872}"));
    } else {
        items.push(MenuItem::new("q", "Установить (в очередь)").icon("\u{E03B}"));
    }
    if installed {
        items.push(MenuItem::new("remove-now", "Удалить сейчас…").icon("\u{E92B}"));
    } else {
        items.push(MenuItem::new("install-now", "Установить сейчас").icon("\u{E2C4}"));
    }
    let desktop = installed.then(|| synshell_common::xdg::app_by_id(&p.name)).flatten();
    if desktop.is_some() {
        items.push(MenuItem::new("open", "Открыть").icon("\u{E89E}"));
    }
    if installed {
        items.push(MenuItem::separator());
        if update {
            let skipped = st.skip.get_untracked().contains(&p.name);
            items.push(MenuItem::new("skip", if skipped { "Обновить в этот раз" } else { "Пропустить в этот раз" }).icon("\u{E044}"));
        }
        let ignored = st.cfg.get_untracked().packages.ignore.contains(&p.name);
        items.push(MenuItem::new("ignore", if ignored { "Снова обновлять" } else { "Не обновлять никогда" }).icon("\u{E14B}"));
    }
    items.push(MenuItem::separator());
    if p.source == Source::Aur {
        items.push(MenuItem::new("pkgbuild", "Показать PKGBUILD").icon("\u{E86F}"));
    }
    items.push(MenuItem::new("web", "Страница пакета").icon("\u{E894}"));
    items.push(MenuItem::new("copy", "Копировать имя").icon("\u{E14D}"));
    show_menu(st, items, at, move |id| {
        let name = p.name.clone();
        match id {
            "details" => select(st, p.clone()),
            "unqueue" => unqueue(st, &name),
            "q" => {
                toggle_queue(st, &p);
                st.toast.set(format!("{name} — в очереди, «Применить» внизу"));
            }
            "install-now" => {
                let op = if p.source == Source::Aur { Op::InstallAur(vec![name.clone()]) } else { Op::Install(vec![name.clone()]) };
                run_op(st, format!("Установка {name}"), op);
            }
            "remove-now" => remove_checked(st, vec![name.clone()], Vec::new(), format!("Удаление {name}"), false),
            "open" => {
                if let Some(e) = synshell_common::xdg::app_by_id(&name) {
                    let _ = std::process::Command::new("sh").arg("-c").arg(e.command()).spawn();
                }
            }
            "skip" => {
                let mut s = st.skip.get_untracked();
                if s.contains(&name) {
                    s.retain(|n| *n != name);
                } else {
                    s.push(name.clone());
                }
                st.skip.set(s);
            }
            "ignore" => {
                let ignored = st.cfg.get_untracked().packages.ignore.contains(&name);
                set_ignored(st, &name, !ignored);
            }
            "pkgbuild" => {
                select(st, p.clone());
                show_pkgbuild(st, name);
            }
            "web" => {
                let url = match &p.source {
                    Source::Aur | Source::Local => format!("https://aur.archlinux.org/packages/{name}"),
                    _ => format!("https://archlinux.org/packages/?q={name}"),
                };
                let _ = std::process::Command::new("xdg-open").arg(url).spawn();
            }
            "copy" => syngui::clipboard::copy(&name),
            _ => {}
        }
    });
}

pub fn pkg_row(st: St, p: Pkg) -> W {
    pkg_row_titled(st, p, None)
}

/// Строка пакета: флажок очереди, значок, название (из каталога), описание, метки.
/// Подсветка (выбран, в очереди) — пересборкой строки по сигналам.
pub fn pkg_row_titled(st: St, p: Pkg, title: Option<String>) -> W {
    let title = title.or_else(|| app_meta(&p.name).map(|m| m.title)).filter(|t| *t != p.name);
    let (p2, p3) = (p.clone(), p.clone());
    let row = Reactive::new(move || -> Vec<W> {
        let sel = st.selected.get().is_some_and(|s| s.name == p.name && s.source == p.source);
        let q = st.queue.get().iter().find(|x| x.name == p.name).map(|x| x.act);
        let class = match (sel, q) {
            (_, Some(QAct::Remove)) => "pkg-row pkg-row-rm",
            (_, Some(_)) => "pkg-row pkg-row-q",
            (true, None) => "pkg-row pkg-row-on",
            _ => "pkg-row",
        };
        let mut meta = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if title.is_some() {
            meta = meta.child(Text::new(p.name.clone()).max_lines(1).class("pkg-ver"));
        }
        meta = meta.child(Text::new(p.version.clone()).max_lines(1).class("pkg-ver")).child(source_chip(&p));
        if p.installed.is_some() {
            meta = meta.child(chip("установлен", "chip-ok"));
        }
        match q {
            Some(QAct::Remove) => meta = meta.child(chip("к удалению", "chip-rm")),
            Some(_) => meta = meta.child(chip("к установке", "chip-q")),
            None => {}
        }
        if p.out_of_date {
            meta = meta.child(chip("устарел", "chip-warn"));
        }
        if let Some(v) = p.votes {
            meta = meta.child(Text::new(format!("★ {v}")).class("pkg-ver"));
        }
        let content = Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child({
                let pq = p.clone();
                GestureDetector::new().on_click(move || toggle_queue(st, &pq)).child(check_icon(q.is_some()))
            })
            .child(app_icon(&p.name, "pkg-icon"))
            .child(
                Column::new()
                    .gap(3.0)
                    .child(Text::new(title.clone().unwrap_or_else(|| p.name.clone())).max_lines(1).class("pkg-name"))
                    .child(Text::new(p.description.clone()).max_lines(2).class("pkg-desc"))
                    .child(meta)
                    .class("grow"),
            );
        vec![Box::new(DecoratedBox::new().child(content).class(class))]
    });
    Box::new(
        GestureDetector::new()
            .on_click(move || select(st, p2.clone()))
            .on_secondary_click(move |at| pkg_menu(st, p3.clone(), at, false))
            .child(row),
    )
}

/// Рабочий стол: окно шире телефонного (поиск — в заголовке окна, подробности — страницей).
pub fn is_desk() -> bool {
    syngui::viewport::viewport_size().get_untracked().width >= 720.0
}

/// Поле поиска вида — только на телефоне: на рабочем столе оно в заголовке окна.
pub fn with_field(field: impl Widget + 'static, col: Column) -> Column {
    if is_desk() {
        col
    } else {
        col.child(field)
    }
}

pub fn search_box(value: String, placeholder: &str, on_change: impl Fn(String) + Send + Sync + 'static) -> impl Widget {
    TextField::with_text(value).placeholder(placeholder).prefix_icon("\u{E8B6}").on_change(move |t| on_change(t.to_string())).class("search")
}

pub fn busy_row(text: &str) -> W {
    Box::new(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(CircularProgress::new().indeterminate().size(20.0)).child(Text::new(text.to_string()).class("muted")).class("empty"))
}

pub fn explore_view(st: St) -> W {
    let field = search_box(st.query.get_untracked(), "Найти программу в репозиториях и AUR", move |t| {
        st.query.set(t.clone());
        search(st, t);
    });
    let list = Reactive::new(move || -> Vec<W> {
        let _ = st.icons_rev.get();
        let busy = st.searching.get();
        let res = st.results.get();
        let q = st.query.get();
        // Пустой запрос — витрина и категории.
        if q.trim().len() < 2 {
            return vec![catalog_view(st)];
        }
        let mut col = Column::new().gap(4.0);
        if busy {
            col = col.child(busy_row("Поиск…"));
        } else if res.is_empty() {
            col = col.child(Text::new("Ничего не найдено").class("muted empty"));
        } else {
            col = col.child(select_all_row(st, &res, "Найдено"));
        }
        for p in res {
            col = col.child(pkg_row(st, p));
        }
        vec![Box::new(col)]
    });
    Box::new(with_field(field, Column::new().gap(12.0)).child(ScrollView::new().vertical().child(list).class("grow")).class("pane"))
}

/// «Найдено: 12 · Выбрать все»: отметить показанные пакеты.
pub fn select_all_row(st: St, v: &[Pkg], label: &str) -> W {
    let all: Vec<QItem> = v.iter().take(400).map(|p| QItem { name: p.name.clone(), act: default_act(p) }).collect();
    let names: Vec<String> = all.iter().map(|q| q.name.clone()).collect();
    let n = v.len();
    let label = label.to_string();
    Box::new(Reactive::new(move || -> Vec<W> {
        let q = st.queue.get();
        let marked = names.iter().filter(|n| q.iter().any(|x| &x.name == *n)).count();
        let all = all.clone();
        let names2 = names.clone();
        let mut row = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(format!("{label}: {n}")).class("muted grow"));
        if marked > 0 {
            row = row.child(Text::new(format!("отмечено {marked}")).class("muted")).child(Button::new("Снять").class("small").on_click(move || {
                let mut q = st.queue.get_untracked();
                q.retain(|x| !names2.contains(&x.name));
                st.queue.set(q);
            }));
        }
        // Удалить разом сотни пакетов случайно — нельзя: для удаления «Выбрать все» только у короткого списка.
        let removal = all.iter().any(|q| q.act == QAct::Remove);
        if marked < names.len() && (!removal || n <= 100) {
            row = row.child(Button::new("Выбрать все").class("small").on_click(move || set_queued(st, all.clone())));
        }
        vec![Box::new(row.class("list-head"))]
    }))
}

/// «1 программа», «3 программы», «5 программ».
pub fn programs(n: usize) -> String {
    let w = match (n % 10, n % 100) {
        (1, r) if r != 11 => "программа",
        (2..=4, r) if !(12..=14).contains(&r) => "программы",
        _ => "программ",
    };
    format!("{n} {w}")
}

/// Колонок плиток шириной ~`tile` в списке.
pub fn grid_cols(tile: f32) -> usize {
    let w = syngui::viewport::viewport_size().get().width;
    let list_w = if w < 720.0 { w - 32.0 } else { w - 230.0 - 480.0 - 40.0 };
    ((list_w + 12.0) / (tile + 12.0)).floor().max(if w < 720.0 { 1.0 } else { 2.0 }) as usize
}

/// Карточка витрины: крупный значок, название, сводка, кнопка очереди.
pub fn app_card(st: St, a: CatalogApp) -> W {
    let p = a.pkg.clone();
    let (p2, p3, p4) = (p.clone(), p.clone(), p.clone());
    let name = p.name.clone();
    let button = Reactive::new(move || -> Vec<W> {
        let q = st.queue.get().iter().find(|x| x.name == name).map(|x| x.act);
        let p = p4.clone();
        let b = match (q, p.installed.is_some()) {
            (Some(_), _) => Button::new("✓ В очереди").class("small queued").on_click(move || toggle_queue(st, &p)),
            (None, true) => Button::new("Удалить").class("small").on_click(move || toggle_queue(st, &p)),
            (None, false) => Button::new("Установить").class("small primary").on_click(move || toggle_queue(st, &p)),
        };
        vec![Box::new(b)]
    });
    Box::new(
        GestureDetector::new().on_click(move || select(st, p2.clone())).on_secondary_click(move |at| pkg_menu(st, p3.clone(), at, false)).child(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .gap(8.0)
                        .child(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(app_icon(&p.name, "card-icon")).child(Column::new().gap(2.0).child(Text::new(a.title.clone()).max_lines(1).class("card-title")).child(Text::new(category_label(&a)).max_lines(1).class("pkg-ver")).class("grow")))
                        .child(Text::new(p.description.clone()).max_lines(2).class("pkg-desc card-desc"))
                        // Высота явно: кнопка в Reactive при обмере сетки ещё пуста — карточка выходила ниже кнопки.
                        .child(Row::new().cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("grow")).child(button).style("height", StyleValue::px(30.0))),
                )
                .class("app-card"),
        ),
    )
}

pub fn category_label(a: &CatalogApp) -> String {
    CATEGORIES.iter().find(|c| c.contains(a)).map(|c| c.label.to_string()).unwrap_or_default()
}

/// Каталог: витрина и плитки категорий или программы открытой категории.
pub fn catalog_view(st: St) -> W {
    let Some(apps) = st.catalog.get() else {
        return busy_row("Загрузка каталога…");
    };
    if apps.is_empty() {
        let mut col = Column::new()
            .gap(10.0)
            .child(Text::new("Введите название или слово из описания: «браузер», «gimp», «office»").class("muted"))
            .child(Text::new("Чтобы смотреть программы по категориям и со снимками экрана, нужен каталог AppStream — пакет archlinux-appstream-data.").class("desc"));
        let has_pkg = st.installed.get().iter().any(|p| p.name == "archlinux-appstream-data");
        if !has_pkg {
            col = col.child(Row::new().child(Button::new("Установить каталог").class("primary").on_click(move || run_op(st, "Установка каталога программ".into(), Op::Install(vec!["archlinux-appstream-data".into()])))));
        }
        return Box::new(col.class("empty"));
    }
    let cat = st.category.get().and_then(Category::by_key);
    if is_desk() {
        return match cat {
            Some(c) => crate::desk::category_page(st, c, apps),
            None => crate::desk::home(st, apps),
        };
    }
    let Some(cat) = cat else {
        let mut col = Column::new().gap(14.0);
        // Витрина: известные программы, которых ещё нет.
        let featured: Vec<CatalogApp> = FEATURED.iter().filter_map(|n| apps.iter().find(|a| a.pkg.name == *n)).filter(|a| a.pkg.installed.is_none()).take(12).cloned().collect();
        if !featured.is_empty() {
            let mut grid = Grid::new(grid_cols(250.0)).gap(12.0);
            for a in featured {
                grid = grid.child(app_card(st, a));
            }
            col = col.child(Text::new("Популярные программы").class("h2")).child(grid);
        }
        let mut grid = Grid::new(grid_cols(220.0)).gap(10.0);
        for c in CATEGORIES {
            let n = apps.iter().filter(|a| c.contains(a)).count();
            if n == 0 {
                continue;
            }
            let key = c.key;
            grid = grid.child(GestureDetector::new().on_click(move || st.category.set(Some(key))).child(
                DecoratedBox::new()
                    .child(
                        Row::new()
                            .gap(12.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(DecoratedBox::new().child(Icon::new(c.icon).class("cat-icon")).class("cat-icon-box"))
                            .child(Column::new().gap(2.0).child(Text::new(c.label).max_lines(1).class("cat-label")).child(Text::new(programs(n)).class("pkg-ver")).class("grow")),
                    )
                    .class("cat-tile"),
            ));
        }
        return Box::new(col.child(Text::new("Категории").class("h2")).child(grid).child(Text::new(format!("В каталоге {}. Поиск находит и пакеты вне каталога, и AUR.", programs(apps.len()))).class("muted")));
    };
    let mut v: Vec<CatalogApp> = apps.into_iter().filter(|a| cat.contains(a)).collect();
    // Установленные — ниже: каталог для поиска нового.
    v.sort_by_key(|a| a.pkg.installed.is_some());
    let pkgs: Vec<Pkg> = v.iter().map(|a| a.pkg.clone()).collect();
    let mut col = Column::new()
        .gap(4.0)
        .child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(GestureDetector::new().on_click(move || st.category.set(None)).child(DecoratedBox::new().child(Icon::new("\u{E5C4}").class("back-icon")).class("back")))
                .child(Icon::new(cat.icon).class("cat-head-icon"))
                .child(Text::new(cat.label).max_lines(1).class("h2 grow")),
        )
        .child(select_all_row(st, &pkgs, "Программ"));
    for a in v {
        col = col.child(pkg_row_titled(st, a.pkg, Some(a.title)));
    }
    Box::new(col)
}

pub fn aur_view(st: St) -> W {
    let field = search_box(st.aur_query.get_untracked(), "Искать в AUR", move |t| {
        st.aur_query.set(t.clone());
        search_aur(st, t);
    });
    let warn = DecoratedBox::new()
        .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new("\u{E002}").class("warn-icon")).child(Text::new("Пакеты AUR собирают пользователи, а не Arch Linux, — сборка идёт на этом компьютере. Проверяйте PKGBUILD (правый щелчок → «Показать PKGBUILD»).").class("warn-text grow")))
        .class("warn");
    let list = Reactive::new(move || -> Vec<W> {
        let _ = st.icons_rev.get();
        let q = st.aur_query.get();
        let mut col = Column::new().gap(4.0);
        if q.trim().len() >= 2 {
            let res = st.aur_results.get();
            if st.aur_searching.get() {
                col = col.child(busy_row("Поиск в AUR…"));
            } else if res.is_empty() {
                col = col.child(Text::new("В AUR ничего не найдено").class("muted empty"));
            } else {
                col = col.child(select_all_row(st, &res, "Найдено в AUR"));
            }
            for p in res {
                col = col.child(pkg_row(st, p));
            }
            return vec![Box::new(col)];
        }
        // Без запроса: обновления AUR и установленное не из репозиториев.
        col = col.child(aur_updates_block(st));
        let foreign: Vec<Pkg> = st.installed.get().into_iter().filter(|p| p.source == Source::Local).collect();
        col = col.child(Text::new("Установлено не из репозиториев").class("h2 section-gap"));
        if foreign.is_empty() {
            col = col.child(Text::new("Пакетов из AUR пока нет").class("muted"));
        } else {
            col = col.child(select_all_row(st, &foreign, "Пакетов"));
        }
        for p in foreign {
            col = col.child(pkg_row(st, p));
        }
        vec![Box::new(col)]
    });
    Box::new(with_field(field, Column::new().gap(12.0).child(warn)).child(ScrollView::new().vertical().child(list).class("grow")).class("pane"))
}

/// «Обновления AUR»: флажки, «Обновить выбранные» — пересборка из AUR.
pub fn aur_updates_block(st: St) -> W {
    let mut col = Column::new().gap(4.0);
    let checking = st.aur_checking.get();
    let ups = st.aur_updates.get();
    let mut head = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new("Обновления AUR").class("h2 grow"));
    if checking {
        head = head.child(CircularProgress::new().indeterminate().size(18.0));
    }
    head = head.child(Button::new("Проверить").class("small").disabled(checking).on_click(move || check_aur_updates(st)));
    col = col.child(head);
    match ups {
        None => col = col.child(Text::new(if checking { "Проверка версий в AUR…" } else { "Не проверялись" }).class("muted")),
        Some(v) if v.is_empty() => col = col.child(Text::new("Все пакеты AUR свежие").class("muted")),
        Some(v) => {
            let ignore = st.cfg.get().packages.ignore.clone();
            let skip = st.skip.get();
            let chosen: Vec<String> = v.iter().filter(|u| !skip.contains(&u.name) && !ignore.contains(&u.name)).map(|u| u.name.clone()).collect();
            let n = chosen.len();
            let total = v.len();
            col = col.child(
                Row::new().child(
                    Button::new(format!("Обновить выбранные ({n} из {total})"))
                        .class("primary")
                        .disabled(n == 0)
                        .on_click(move || run_op(st, format!("Обновление AUR: {}", packages_word(chosen.len())), Op::InstallAur(chosen.clone()))),
                ),
            );
            for u in v {
                col = col.child(update_row(st, u, &skip, &ignore));
            }
        }
    }
    Box::new(col)
}

pub fn installed_view(st: St) -> W {
    let field = search_box(st.filter.get_untracked(), "Фильтр установленных", move |t| st.filter.set(t));
    let chips = Reactive::new(move || -> Vec<W> {
        let cur = st.inst_filter.get();
        let mut row = Row::new().gap(8.0);
        for (f, label) in [(InstFilter::All, "Все пакеты"), (InstFilter::Apps, "Программы"), (InstFilter::Foreign, "Не из репозиториев")] {
            row = row.child(GestureDetector::new().on_click(move || st.inst_filter.set(f)).child(DecoratedBox::new().child(Text::new(label).class("fchip-text")).class(if cur == f { "fchip fchip-on" } else { "fchip" })));
        }
        vec![Box::new(row)]
    });
    let list = Reactive::new(move || -> Vec<W> {
        let _ = st.icons_rev.get();
        let f = st.filter.get().to_lowercase();
        let kind = st.inst_filter.get();
        let all = st.installed.get();
        if all.is_empty() {
            return vec![busy_row("Чтение установленных…")];
        }
        let v: Vec<Pkg> = all
            .into_iter()
            .filter(|p| f.is_empty() || p.name.contains(&f) || p.description.to_lowercase().contains(&f))
            .filter(|p| match kind {
                InstFilter::All => true,
                InstFilter::Apps => app_meta(&p.name).is_some() || synshell_common::xdg::app_by_id(&p.name).is_some(),
                InstFilter::Foreign => p.source == Source::Local,
            })
            .collect();
        let n = v.len();
        // Длинный список — первые 400 (фильтр сужает).
        let mut col = Column::new().gap(4.0).child(select_all_row(st, &v, "Пакетов"));
        for p in v.into_iter().take(400) {
            col = col.child(pkg_row(st, p));
        }
        if n > 400 {
            col = col.child(Text::new(format!("Показаны первые 400 из {n} — уточните фильтр")).class("muted empty"));
        }
        vec![Box::new(col)]
    });
    Box::new(with_field(field, Column::new().gap(10.0)).child(chips).child(ScrollView::new().vertical().child(list).class("grow")).class("pane"))
}

/// Строка обновления: флажок «обновлять», значок, версии, метки; правый щелчок — меню.
pub fn update_row(st: St, u: Update, skip: &[String], ignore: &[String]) -> W {
    let ignored = ignore.contains(&u.name);
    let on = !ignored && !skip.contains(&u.name);
    let name = u.name.clone();
    let check = GestureDetector::new()
        .on_click(move || {
            if ignored {
                set_ignored(st, &name, false);
                return;
            }
            let mut s = st.skip.get_untracked();
            if s.contains(&name) {
                s.retain(|n| *n != name);
            } else {
                s.push(name.clone());
            }
            st.skip.set(s);
        })
        .child(check_icon(on));
    let mut meta = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(chip(if u.aur { "AUR" } else { "репозиторий" }, if u.aur { "chip-aur" } else { "chip-repo" }));
    if ignored {
        meta = meta.child(chip("не обновляется", "chip-warn"));
    } else if !on {
        meta = meta.child(chip("пропуск", "chip-local"));
    }
    let pkg = Pkg { name: u.name.clone(), version: u.new.clone(), description: String::new(), source: if u.aur { Source::Aur } else { Source::Repo(String::new()) }, installed: Some(u.old.clone()), votes: None, popularity: None, out_of_date: false };
    let (p2, p3) = (pkg.clone(), pkg.clone());
    Box::new(
        GestureDetector::new().on_click(move || select(st, p2.clone())).on_secondary_click(move |at| pkg_menu(st, p3.clone(), at, true)).child(
            DecoratedBox::new()
                .child(
                    Row::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(check)
                        .child(app_icon(&u.name, "pkg-icon"))
                        .child(Column::new().gap(3.0).child(Text::new(title_of(&pkg)).max_lines(1).class("pkg-name")).child(Text::new(format!("{} → {}", u.old, u.new)).class("pkg-desc")).class("grow"))
                        .child(meta),
                )
                .class(if on { "pkg-row" } else { "pkg-row pkg-row-off" }),
        ),
    )
}

pub fn updates_view(st: St) -> W {
    let list = Reactive::new(move || -> Vec<W> {
        let _ = st.icons_rev.get();
        let Some(ups) = st.updates.get() else {
            return vec![busy_row("Проверка обновлений…")];
        };
        let cfg = st.cfg.get();
        let ignore = cfg.packages.ignore.clone();
        let skip = st.skip.get();
        let mut col = Column::new().gap(4.0);
        let head = Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new(if ups.is_empty() { "Система обновлена".to_string() } else { format!("Доступно обновлений: {}", ups.len()) }).class("h2 grow"))
            .child(Button::new("Проверить").on_click(move || check_updates(st)));
        col = col.child(head);
        if !ups.is_empty() {
            let left: Vec<String> = ups.iter().filter(|u| skip.contains(&u.name) || ignore.contains(&u.name)).map(|u| u.name.clone()).collect();
            let n = ups.len() - left.len();
            let mut ign = left.clone();
            for i in &ignore {
                if !ign.contains(i) {
                    ign.push(i.clone());
                }
            }
            let aur = cfg.packages.aur;
            let mut actions = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(
                Button::new(if left.is_empty() { "Обновить всё".to_string() } else { format!("Обновить выбранные ({n} из {})", ups.len()) })
                    .class("primary")
                    .disabled(n == 0)
                    .on_click(move || run_op(st, "Обновление системы".into(), Op::Upgrade { aur, ignore: ign.clone() })),
            );
            if !left.is_empty() {
                actions = actions.child(Text::new(format!("Пропускаются: {}", left.join(", "))).max_lines(1).class("muted grow"));
            }
            col = col.child(actions);
            if !left.is_empty() {
                col = col.child(Text::new("Пропуск пакетов — частичное обновление: если от пропущенного зависят обновляемые, pacman откажется. Правый щелчок — «Не обновлять никогда», «Удалить».").class("muted"));
            }
        }
        for u in ups {
            col = col.child(update_row(st, u, &skip, &ignore));
        }
        vec![Box::new(col)]
    });
    Box::new(Column::new().gap(10.0).child(ScrollView::new().vertical().child(list).class("grow")).class("pane"))
}

/// Очередь в «Задачах»: пункты с крестиком и «Применить».
pub fn queue_section(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let q = st.queue.get();
        if q.is_empty() {
            return vec![];
        }
        let mut col = Column::new()
            .gap(4.0)
            .child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(format!("Очередь — {}", packages_word(q.len()))).class("h2 grow"))
                    .child(Button::new("Очистить").class("small").on_click(move || st.queue.set(Vec::new())))
                    .child(Button::new("Применить").class("primary").on_click(move || apply_queue(st))),
            );
        for it in q {
            let name = it.name.clone();
            let (label, class) = match it.act {
                QAct::Install => ("установить", "chip-ok"),
                QAct::InstallAur => ("собрать из AUR", "chip-aur"),
                QAct::Remove => ("удалить", "chip-rm"),
            };
            col = col.child(
                DecoratedBox::new()
                    .child(
                        Row::new()
                            .gap(10.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(app_icon(&it.name, "pkg-icon-sm"))
                            .child(Text::new(it.name.clone()).max_lines(1).class("pkg-name grow"))
                            .child(chip(label, class))
                            .child(GestureDetector::new().on_click(move || unqueue(st, &name)).child(DecoratedBox::new().child(Icon::new("\u{E5CD}").class("x-icon")).class("x-btn"))),
                    )
                    .class("q-row"),
            );
        }
        vec![Box::new(DecoratedBox::new().child(col).class("job"))]
    }))
}

/// Задачи: очередь, лог открытого задания — на всю высоту, остальные — строками ниже.
pub fn jobs_view(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let jobs = st.jobs.get();
        if jobs.is_empty() {
            return vec![Box::new(Text::new("Отметьте программы флажками или нажмите «Установить» / «Удалить» — они встанут в очередь, «Применить» выполнит её здесь.").class("muted empty"))];
        }
        let open = st.job_open.get().filter(|id| jobs.iter().any(|j| j.id == *id)).unwrap_or(jobs[0].id);
        let mut others = Column::new().gap(6.0);
        let mut has_others = false;
        let mut main: Option<W> = None;
        for j in jobs {
            let cancelled = matches!(&j.done, Some(Err(e)) if e == pk::CANCELLED);
            let (icon, class) = match &j.done {
                None => ("\u{E863}", "job-running"),
                Some(Ok(())) => ("\u{E86C}", "job-ok"),
                Some(Err(_)) if cancelled => ("\u{E5C9}", "job-cancelled"),
                Some(Err(_)) => ("\u{E000}", "job-err"),
            };
            let mut head = Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(icon).class(format!("job-icon {class}")))
                .child(Column::new().gap(0.0).child(Text::new(j.title.clone()).max_lines(1).class("pkg-name")).child(Text::new(j.stage.clone()).max_lines(1).class("pkg-desc")).class("grow"));
            if j.done.is_none() {
                head = head.child(if j.cancelling {
                    Button::new("Отмена…").class("small").disabled(true)
                } else {
                    let (id, c) = (j.id, j.cancel.clone());
                    Button::new("Отменить").class("small").on_click(move || cancel_job(st, id, &c))
                });
            }
            if j.id != open {
                let id = j.id;
                has_others = true;
                others = others.child(GestureDetector::new().on_click(move || st.job_open.set(Some(id))).child(DecoratedBox::new().child(head.child(Icon::new("\u{E5CF}").class("job-expand"))).class("job job-row")));
                continue;
            }
            let mut card = Column::new().gap(8.0).child(head);
            if j.done.is_none() {
                card = card.child(ProgressBar::new().indeterminate());
            }
            if let Some(Err(e)) = &j.done {
                if !cancelled {
                    card = card.child(Text::new(e.clone()).selectable(true).class("job-error"));
                }
            }
            let text = if j.log.is_empty() { "…".to_string() } else { j.log.join("\n") };
            card = card.child(DecoratedBox::new().child(ScrollView::new().vertical().follow_end(true).child(Text::new(text).selectable(true).class("log-line")).class("grow")).class("log"));
            main = Some(Box::new(DecoratedBox::new().child(card.class("grow")).class("job job-open")));
        }
        let mut col = Column::new().gap(10.0);
        if let Some(m) = main {
            col = col.child(m);
        }
        if has_others {
            col = col.child(Text::new("Другие задачи").class("muted")).child(ScrollView::new().vertical().child(others).class("job-others"));
        }
        vec![Box::new(col.class("grow"))]
    });
    Box::new(Column::new().gap(10.0).child(queue_section(st)).child(body).class("pane"))
}

/// Кнопки действий в подробностях: очередь, «сейчас», «Открыть».
pub fn detail_actions(st: St, p: Pkg) -> W {
    let name = p.name.clone();
    Box::new(Reactive::new(move || -> Vec<W> {
        let q = st.queue.get().iter().find(|x| x.name == name).map(|x| x.act);
        let installed = p.installed.is_some();
        let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
        let (p1, p2) = (p.clone(), p.clone());
        match q {
            Some(act) => {
                row = row.child(Button::new(if act == QAct::Remove { "✓ В очереди на удаление" } else { "✓ В очереди на установку" }).class("queued").on_click(move || toggle_queue(st, &p1)));
                row = row.child(Button::new("Применить").class("primary").on_click(move || apply_queue(st)));
            }
            None if installed => {
                row = row.child(Button::new("Удалить").class("danger").on_click(move || {
                    toggle_queue(st, &p1);
                    st.toast.set(format!("{} — в очереди на удаление", p1.name));
                }));
            }
            None => {
                row = row.child(Button::new("Установить").class("primary").on_click(move || {
                    toggle_queue(st, &p1);
                    st.toast.set(format!("{} — в очереди на установку", p1.name));
                }));
            }
        }
        if q.is_none() {
            let n = p2.name.clone();
            row = row.child(if installed {
                Button::new("Удалить сейчас").class("small").on_click(move || remove_checked(st, vec![n.clone()], Vec::new(), format!("Удаление {n}"), false))
            } else {
                let op = if p2.source == Source::Aur { Op::InstallAur(vec![n.clone()]) } else { Op::Install(vec![n.clone()]) };
                Button::new("Сейчас").class("small").on_click(move || run_op(st, format!("Установка {n}"), op.clone()))
            });
        }
        if installed {
            if let Some(e) = synshell_common::xdg::app_by_id(&p.name) {
                let cmd = e.command();
                row = row.child(Button::new("Открыть").on_click(move || {
                    let _ = std::process::Command::new("sh").arg("-c").arg(&cmd).spawn();
                }));
            }
        }
        vec![Box::new(row)]
    }))
}

pub fn details_view(st: St, p: Pkg) -> W {
    let name = p.name.clone();
    let aur = p.source == Source::Aur;
    let app = st.catalog.get_untracked().and_then(|c| c.into_iter().find(|a| a.pkg.name == name));
    let title = app.as_ref().map(|a| a.title.clone()).unwrap_or_else(|| name.clone());
    let summary = if p.description.is_empty() { app.as_ref().map(|a| a.pkg.description.clone()).unwrap_or_default() } else { p.description.clone() };
    let n2 = name.clone();
    let big_icon = Reactive::new(move || -> Vec<W> {
        match st.media.get().filter(|m| m.name == n2).and_then(|m| m.icon) {
            Some(path) => vec![Box::new(Image::new(path.to_string_lossy()).fit(ImageFit::Contain).placeholder(false).class("hero-icon"))],
            None => vec![app_icon(&n2, "hero-icon")],
        }
    });
    let mut chips = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(p.version.clone()).class("pkg-ver")).child(source_chip(&p));
    if p.installed.is_some() {
        chips = chips.child(chip("установлен", "chip-ok"));
    }
    if title != name {
        chips = chips.child(Text::new(name.clone()).class("pkg-ver"));
    }
    let head = Row::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(big_icon).class("hero-icon-box"))
        .child(Column::new().gap(6.0).child(Text::new(title).max_lines(2).class("h1")).child(chips).class("grow"));
    let mut col = Column::new().gap(16.0).child(head).child(Text::new(summary).class("desc-lead")).child(detail_actions(st, p.clone()));
    col = col.child(screenshots(st, name.clone(), SHOT_W, SHOT_H));
    // Полное описание: AppStream, иначе Flathub.
    let desc_local = app.as_ref().map(|a| a.description.clone()).unwrap_or_default();
    let n3 = name.clone();
    col = col.child(Reactive::new(move || -> Vec<W> {
        let paras = if desc_local.is_empty() { st.media.get().filter(|m| m.name == n3).map(|m| m.description).unwrap_or_default() } else { desc_local.clone() };
        if paras.is_empty() {
            return vec![];
        }
        let mut c = Column::new().gap(8.0).child(Text::new("Описание").class("h2"));
        for p in paras.into_iter().take(14) {
            c = c.child(Text::new(p).class("desc"));
        }
        vec![Box::new(c)]
    }));
    if aur {
        let n = name.clone();
        col = col.child(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .gap(6.0)
                        .child(Text::new("Пакет из AUR собирают пользователи, а не Arch Linux. Проверьте PKGBUILD перед установкой.").class("warn-text"))
                        .child(Row::new().child(Button::new("Показать PKGBUILD").class("small").on_click(move || show_pkgbuild(st, n.clone())))),
                )
                .class("warn"),
        );
    }
    let info = Reactive::new(move || -> Vec<W> {
        let mut out = Column::new().gap(12.0);
        if let Some(pb) = st.pkgbuild.get() {
            out = out.child(DecoratedBox::new().child(ScrollView::new().both().child(Text::new(pb).selectable(true).class("code"))).class("code-box"));
        }
        match st.details.get() {
            None => out = out.child(CircularProgress::new().indeterminate().size(22.0)),
            Some(d) => {
                let mut table = Column::new().gap(0.0);
                for (k, v) in &d.fields {
                    table = table.child(Row::new().gap(10.0).child(Text::new(k.clone()).class("k")).child(Text::new(v.clone()).selectable(true).class("v grow")).class("kv"));
                }
                if let Some(u) = &d.url {
                    let u2 = u.clone();
                    table = table.child(Row::new().gap(10.0).child(Text::new("Сайт").class("k")).child(GestureDetector::new().on_click(move || {
                        let _ = std::process::Command::new("xdg-open").arg(&u2).spawn();
                    }).child(Text::new(u.clone()).class("v link"))).class("kv"));
                }
                out = out.child(Text::new("Сведения").class("h2")).child(DecoratedBox::new().child(table).class("card"));
                if !d.depends.is_empty() {
                    let mut deps = Flex::new().wrap().gap(6.0);
                    for dep in &d.depends {
                        deps = deps.child(chip(dep.clone(), "chip-dep"));
                    }
                    out = out.child(Text::new("Зависимости").class("h2")).child(deps);
                }
                if !d.optional.is_empty() {
                    let mut col = Column::new().gap(2.0);
                    for o in &d.optional {
                        col = col.child(Text::new(o.clone()).class("pkg-desc"));
                    }
                    out = out.child(Text::new("Дополнительно").class("h2")).child(col);
                }
                if !d.files.is_empty() {
                    let mut col = Column::new().gap(0.0);
                    for f in d.files.iter().take(200) {
                        col = col.child(Text::new(f.clone()).class("log-line"));
                    }
                    out = out.child(Text::new(format!("Файлы ({})", d.files.len())).class("h2")).child(DecoratedBox::new().child(col).class("code-box"));
                }
            }
        }
        vec![Box::new(out)]
    });
    Box::new(ScrollView::new().vertical().child(col.child(info).class("detail")))
}

/// Лента снимков экрана; щелчок — на всё окно.
pub fn screenshots(st: St, name: String, sw: f32, sh: f32) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let Some(m) = st.media.get().filter(|m| m.name == name) else {
            return vec![Box::new(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(CircularProgress::new().indeterminate().size(16.0)).child(Text::new("Поиск снимков экрана…").class("muted")))];
        };
        if m.shots.is_empty() {
            return if m.loading { vec![Box::new(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(CircularProgress::new().indeterminate().size(16.0)).child(Text::new("Загрузка снимков…").class("muted")))] } else { vec![] };
        }
        let mut row = Row::new().gap(10.0);
        for (i, path) in m.shots.iter().enumerate() {
            row = row.child(GestureDetector::new().on_click(move || st.shot.set(Some(i))).child(
                DecoratedBox::new()
                    .child(Image::new(path.to_string_lossy()).fit(ImageFit::Cover).class("shot-img").style("width", StyleValue::px(sw)).style("height", StyleValue::px(sh)))
                    .class("shot")
                    .style("width", StyleValue::px(sw))
                    .style("height", StyleValue::px(sh)),
            ));
        }
        if m.loading {
            row = row.child(DecoratedBox::new().child(CircularProgress::new().indeterminate().size(20.0)).class("shot shot-wait"));
        }
        let src = if m.source.is_empty() { String::new() } else { format!("Снимки: {}", m.source) };
        vec![Box::new(Column::new().gap(6.0).child(ScrollView::new().horizontal().child(row).class("shots")).child(Text::new(src).class("pkg-ver")))]
    }))
}

