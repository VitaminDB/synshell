//! synpkg — «Программы»: поиск, установка, удаление и обновление программ
//! из репозиториев pacman и AUR. Одна программа для рабочего стола (три
//! колонки: разделы, список, подробности) и телефона (стек «список →
//! пакет» с нижней навигацией). Бэкенд — `synsystem::packages`.
//!
//! `synpkg [запрос]` — открыть поиск с запросом.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use synshell_common::Config;
use synsystem::packages::{self as pk, Details, JobEvent, Op, Pkg, Source, Update};
use synsystem::polkit_agent::{self, AuthRequest, Prompter};
use syngui::async_runtime::run_on_main_thread;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::{EventHook, KeyReply};
use syngui::GestureDetector;

type W = Box<dyn Widget>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Search,
    Installed,
    Updates,
    Jobs,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Search, Tab::Installed, Tab::Updates, Tab::Jobs];
    fn label(self) -> &'static str {
        match self {
            Tab::Search => "Поиск",
            Tab::Installed => "Установленные",
            Tab::Updates => "Обновления",
            Tab::Jobs => "Задачи",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Tab::Search => "\u{E8B6}",
            Tab::Installed => "\u{E1DB}",
            Tab::Updates => "\u{E923}",
            Tab::Jobs => "\u{E8B5}",
        }
    }
}

#[derive(Clone, PartialEq)]
struct JobView {
    id: u64,
    title: String,
    stage: String,
    log: Vec<String>,
    done: Option<std::result::Result<(), String>>,
}

/// Открытое окно пароля polkit (агент — [`polkit_agent`]).
#[derive(Clone)]
struct AuthView(Arc<AuthRequest>);

impl PartialEq for AuthView {
    fn eq(&self, o: &Self) -> bool {
        self.0.id == o.0.id
    }
}

/// Окно пароля для pkexec: запросы агента приходят из его потока — в сигнал через главный поток.
struct AuthPrompter(RwSignal<Option<AuthView>>);

impl Prompter for AuthPrompter {
    fn ask(&self, req: AuthRequest) {
        let s = self.0;
        let v = AuthView(Arc::new(req));
        run_on_main_thread(move || s.set(Some(v)));
    }
    fn cancel(&self, id: u64) {
        let s = self.0;
        run_on_main_thread(move || {
            if s.get_untracked().is_some_and(|a| a.0.id == id) {
                s.set(None);
            }
        });
    }
}

#[derive(Clone, Copy)]
struct St {
    tab: RwSignal<Tab>,
    query: RwSignal<String>,
    results: RwSignal<Vec<Pkg>>,
    searching: RwSignal<bool>,
    installed: RwSignal<Vec<Pkg>>,
    filter: RwSignal<String>,
    updates: RwSignal<Option<Vec<Update>>>,
    selected: RwSignal<Option<Pkg>>,
    details: RwSignal<Option<Details>>,
    pkgbuild: RwSignal<Option<String>>,
    jobs: RwSignal<Vec<JobView>>,
    toast: RwSignal<String>,
    cfg: RwSignal<Config>,
    auth: RwSignal<Option<AuthView>>,
}

static SEARCH_GEN: AtomicU64 = AtomicU64::new(0);
static JOB_ID: AtomicU64 = AtomicU64::new(1);

fn main() {
    let query = std::env::args().skip(1).find(|a| !a.starts_with('-')).unwrap_or_default();
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    App::new()
        .title("Программы")
        .app_id("synpkg")
        .size(1100, 720)
        .min_size(340, 480)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .run(move |_| {
            let st = St {
                tab: use_signal(Tab::Search),
                query: use_signal(query.clone()),
                results: use_signal(Vec::new()),
                searching: use_signal(false),
                installed: use_signal(Vec::new()),
                filter: use_signal(String::new()),
                updates: use_signal(None),
                selected: use_signal(None),
                details: use_signal(None),
                pkgbuild: use_signal(None),
                jobs: use_signal(Vec::new()),
                toast: use_signal(String::new()),
                cfg: use_signal(cfg.clone()),
                auth: use_signal(None),
            };
            polkit_agent::set_prompter(AuthPrompter(st.auth));
            if !query.is_empty() {
                search(st, query.clone());
            }
            load_installed(st);
            root(st)
        });
}

fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let mut s = a.mss_variables();
    s.push_str(&a.theme_mss_variables());
    s.push_str(include_str!("../styles/synpkg.mss"));
    if !a.font.trim().is_empty() {
        s.push_str(&format!("Text, Button, TextField {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    s
}

// ─── Фоновые запросы ─────────────────────────────────────────────────────────

fn search(st: St, q: String) {
    let gen = SEARCH_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let aur = st.cfg.get_untracked().packages.aur;
    if q.trim().len() < 2 {
        st.results.set(Vec::new());
        st.searching.set(false);
        return;
    }
    st.searching.set(true);
    std::thread::spawn(move || {
        // Набор ещё идёт — подождать паузы.
        std::thread::sleep(Duration::from_millis(350));
        if SEARCH_GEN.load(Ordering::SeqCst) != gen {
            return;
        }
        let r = pk::search(&q, aur);
        run_on_main_thread(move || {
            if SEARCH_GEN.load(Ordering::SeqCst) == gen {
                st.results.set(r);
                st.searching.set(false);
            }
        });
    });
}

fn load_installed(st: St) {
    std::thread::spawn(move || {
        let v = pk::installed();
        run_on_main_thread(move || st.installed.set(v));
    });
}

fn check_updates(st: St) {
    st.updates.set(None);
    let aur = st.cfg.get_untracked().packages.aur;
    std::thread::spawn(move || {
        let v = pk::updates(aur);
        run_on_main_thread(move || st.updates.set(Some(v)));
    });
}

fn select(st: St, p: Pkg) {
    st.details.set(None);
    st.pkgbuild.set(None);
    st.selected.set(Some(p.clone()));
    std::thread::spawn(move || {
        let d = pk::details(&p);
        run_on_main_thread(move || {
            if st.selected.get_untracked().is_some_and(|s| s.name == p.name) {
                st.details.set(Some(d));
            }
        });
    });
}

fn show_pkgbuild(st: St, name: String) {
    std::thread::spawn(move || {
        let url = format!("https://aur.archlinux.org/cgit/aur.git/plain/PKGBUILD?h={name}");
        let text = std::process::Command::new("curl")
            .args(["-sfL", "--max-time", "20", &url])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_else(|| "Не удалось загрузить PKGBUILD".into());
        run_on_main_thread(move || st.pkgbuild.set(Some(text)));
    });
}

/// Запустить операцию: задание появляется в «Задачах», лог идёт вживую.
fn run_op(st: St, title: String, op: Op) {
    let id = JOB_ID.fetch_add(1, Ordering::SeqCst);
    let mut jobs = st.jobs.get_untracked();
    jobs.insert(0, JobView { id, title: title.clone(), stage: "Запуск…".into(), log: Vec::new(), done: None });
    st.jobs.set(jobs);
    st.toast.set(format!("{title} — в «Задачах»"));
    let build_user = st.cfg.get_untracked().packages.build_user.clone();
    let job = pk::start(op, build_user);
    std::thread::spawn(move || {
        let mut batch: Vec<JobEvent> = Vec::new();
        loop {
            match job.rx.recv_timeout(Duration::from_millis(150)) {
                Ok(e) => {
                    let end = matches!(e, JobEvent::Done(_));
                    batch.push(e);
                    if !end {
                        continue;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break,
            }
            if batch.is_empty() {
                continue;
            }
            let events = std::mem::take(&mut batch);
            let finished = events.iter().any(|e| matches!(e, JobEvent::Done(_)));
            run_on_main_thread(move || {
                let mut jobs = st.jobs.get_untracked();
                if let Some(j) = jobs.iter_mut().find(|j| j.id == id) {
                    for e in events {
                        match e {
                            JobEvent::Line(l) => j.log.push(l),
                            JobEvent::Stage(s) => j.stage = s,
                            JobEvent::Done(r) => {
                                st.toast.set(match &r {
                                    Ok(()) => format!("Готово: {}", j.title),
                                    Err(e) => format!("Ошибка: {} — {e}", j.title),
                                });
                                j.stage = if r.is_ok() { "Готово".into() } else { "Ошибка".into() };
                                j.done = Some(r);
                            }
                        }
                    }
                    let n = j.log.len();
                    if n > 400 {
                        j.log.drain(..n - 400);
                    }
                }
                st.jobs.set(jobs);
                if finished {
                    load_installed(st);
                    let q = st.query.get_untracked();
                    if !q.is_empty() {
                        search(st, q);
                    }
                    if let Some(p) = st.selected.get_untracked() {
                        let mut p = p;
                        p.installed = pk::installed_map().get(&p.name).cloned();
                        select(st, p);
                    }
                    if st.updates.get_untracked().is_some() {
                        check_updates(st);
                    }
                }
            });
            if finished {
                break;
            }
        }
    });
}

// ─── Интерфейс ───────────────────────────────────────────────────────────────

fn root(st: St) -> W {
    let narrow = syngui::viewport::viewport_below(720.0);
    let body = Reactive::new(move || -> Vec<W> { vec![if narrow.get() { phone(st) } else { desktop(st) }] });
    let toast = Reactive::new(move || -> Vec<W> {
        let t = st.toast.get();
        if t.is_empty() {
            return vec![];
        }
        vec![Box::new(Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().child(Text::new(t).max_lines(2).class("toast-text")).class("toast")).class("toast-place"))]
    });
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
    Box::new(
        EventHook::new()
            .on_key_down(move |k, _| {
                if matches!(k, syngui::input::Key::Escape) && st.selected.get_untracked().is_some() {
                    st.selected.set(None);
                    KeyReply::Handled
                } else {
                    KeyReply::Ignore
                }
            })
            .child(GestureDetector::new().on_back(move || {
                if st.selected.get_untracked().is_some() {
                    st.selected.set(None);
                    true
                } else {
                    false
                }
            }).child(Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().child(body).class("root")).child(toast).child(auth_overlay(st)))),
    )
}

/// Окно «нужны права администратора» поверх всего: пароль уходит агенту polkit.
fn auth_overlay(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
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
                    .child(Button::new("Отмена").class("btn").on_click(cancel))
                    .child(Button::new("Подтвердить").class("btn primary").on_click(submit)),
            );
        vec![Box::new(
            GestureDetector::new().on_click(|| {}).child(
                DecoratedBox::new()
                    .child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().child(card).class("auth-card")).class("grow"))
                    .class("auth-scrim"),
            ),
        )]
    }))
}

fn tab_content(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        vec![match st.tab.get() {
            Tab::Search => search_view(st),
            Tab::Installed => installed_view(st),
            Tab::Updates => updates_view(st),
            Tab::Jobs => jobs_view(st),
        }]
    }))
}

fn desktop(st: St) -> W {
    let nav = Reactive::new(move || -> Vec<W> {
        let cur = st.tab.get();
        let jobs = st.jobs.get().iter().filter(|j| j.done.is_none()).count();
        let mut col = Column::new().gap(4.0).child(Text::new("Программы").class("brand"));
        for t in Tab::ALL {
            let label = if t == Tab::Jobs && jobs > 0 { format!("{} · {jobs}", t.label()) } else { t.label().to_string() };
            col = col.child(GestureDetector::new().on_click(move || {
                st.tab.set(t);
                if t == Tab::Updates && st.updates.get_untracked().is_none() {
                    check_updates(st);
                }
            }).child(
                DecoratedBox::new()
                    .child(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(t.icon()).class("nav-icon")).child(Text::new(label).class("nav-label")))
                    .class(if cur == t { "nav-item active" } else { "nav-item" }),
            ));
        }
        vec![Box::new(col)]
    });
    let detail = Reactive::new(move || -> Vec<W> {
        vec![match st.selected.get() {
            Some(p) => details_view(st, p),
            None => Box::new(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new("\u{E8F4}").class("empty-icon")).child(Text::new("Выберите пакет").class("muted")).class("grow")),
        }]
    });
    Box::new(
        Row::new()
            .child(DecoratedBox::new().child(nav).class("sidebar"))
            .child(DecoratedBox::new().child(tab_content(st)).class("grow list-pane"))
            .child(DecoratedBox::new().child(detail).class("detail-pane")),
    )
}

fn phone(st: St) -> W {
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
                                    .child(Text::new(p.name.clone()).max_lines(1).class("bar-title grow"))
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
        let jobs = st.jobs.get().iter().filter(|j| j.done.is_none()).count();
        let mut row = Row::new().main_axis_alignment(MainAxisAlignment::SpaceAround);
        for t in Tab::ALL {
            let mut item = Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().child(Icon::new(t.icon()).class("nb-icon")).class(if cur == t { "nb-pill nb-pill-on" } else { "nb-pill" }))
                .child(Text::new(t.label()).max_lines(1).class("nb-label"));
            if t == Tab::Jobs && jobs > 0 {
                item = item.child(DecoratedBox::new().class("nb-dot"));
            }
            row = row.child(GestureDetector::new().on_click(move || {
                st.selected.set(None);
                st.tab.set(t);
                if t == Tab::Updates && st.updates.get_untracked().is_none() {
                    check_updates(st);
                }
            }).child(DecoratedBox::new().child(item).class("nb-item")));
        }
        vec![Box::new(row.class("navbar"))]
    });
    Box::new(Column::new().child(DecoratedBox::new().child(body).class("grow")).child(navbar))
}

fn chip(label: String, class: &str) -> impl Widget {
    DecoratedBox::new().child(Text::new(label).class("chip-text")).class(format!("chip {class}"))
}

fn source_chip(p: &Pkg) -> impl Widget {
    match &p.source {
        Source::Aur => chip("AUR".into(), "chip-aur"),
        Source::Local => chip("локальный".into(), "chip-local"),
        Source::Repo(r) if r.is_empty() => chip("репозиторий".into(), "chip-repo"),
        Source::Repo(r) => chip(r.clone(), "chip-repo"),
    }
}

/// Значок программы, если у пакета есть .desktop с таким именем.
fn pkg_icon(name: &str) -> W {
    match synshell_common::xdg::lookup_icon(name) {
        Some(p) => Box::new(Image::new(p.to_string_lossy()).fit(ImageFit::Contain).placeholder(false).class("pkg-icon")),
        None => Box::new(DecoratedBox::new().child(Icon::new("\u{E1BD}").class("pkg-glyph")).class("pkg-glyph-box")),
    }
}

fn pkg_row(st: St, p: Pkg) -> W {
    let sel = st.selected.get_untracked().is_some_and(|s| s.name == p.name && s.source == p.source);
    let p2 = p.clone();
    let mut meta = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(p.version.clone()).class("pkg-ver")).child(source_chip(&p));
    if p.installed.is_some() {
        meta = meta.child(chip("установлен".into(), "chip-ok"));
    }
    if p.out_of_date {
        meta = meta.child(chip("устарел".into(), "chip-warn"));
    }
    if let Some(v) = p.votes {
        meta = meta.child(Text::new(format!("★ {v}")).class("pkg-ver"));
    }
    Box::new(GestureDetector::new().on_click(move || select(st, p2.clone())).child(
        DecoratedBox::new()
            .child(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(pkg_icon(&p.name))
                    .child(
                        Column::new()
                            .gap(3.0)
                            .child(Text::new(p.name.clone()).max_lines(1).class("pkg-name"))
                            .child(Text::new(p.description.clone()).max_lines(2).class("pkg-desc"))
                            .child(meta)
                            .class("grow"),
                    ),
            )
            .class(if sel { "pkg-row pkg-row-on" } else { "pkg-row" }),
    ))
}

fn search_box(value: String, placeholder: &str, on_change: impl Fn(String) + Send + Sync + 'static) -> impl Widget {
    TextField::with_text(value).placeholder(placeholder).prefix_icon("\u{E8B6}").on_change(move |t| on_change(t.to_string())).class("search")
}

fn search_view(st: St) -> W {
    let field = search_box(st.query.get_untracked(), "Найти программу", move |t| {
        st.query.set(t.clone());
        search(st, t);
    });
    let list = Reactive::new(move || -> Vec<W> {
        let busy = st.searching.get();
        let res = st.results.get();
        let q = st.query.get_untracked();
        let mut col = Column::new().gap(4.0);
        if busy {
            col = col.child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(CircularProgress::new().indeterminate().size(20.0)).child(Text::new("Поиск…").class("muted")));
        } else if res.is_empty() {
            let msg = if q.trim().len() < 2 { "Введите название или слово из описания: «браузер», «gimp», «office»" } else { "Ничего не найдено" };
            col = col.child(Text::new(msg).class("muted empty"));
        }
        for p in res {
            col = col.child(pkg_row(st, p));
        }
        vec![Box::new(col)]
    });
    Box::new(Column::new().gap(10.0).child(field).child(ScrollView::new().vertical().child(list).class("grow")).class("pane"))
}

fn installed_view(st: St) -> W {
    let field = search_box(st.filter.get_untracked(), "Фильтр установленных", move |t| st.filter.set(t));
    let list = Reactive::new(move || -> Vec<W> {
        let f = st.filter.get().to_lowercase();
        let all = st.installed.get();
        let v: Vec<Pkg> = all.into_iter().filter(|p| f.is_empty() || p.name.contains(&f) || p.description.to_lowercase().contains(&f)).collect();
        let n = v.len();
        // Длинный список — первые 300 (фильтр сужает).
        let mut col = Column::new().gap(4.0).child(Text::new(format!("Пакетов: {n}")).class("muted"));
        for p in v.into_iter().take(300) {
            col = col.child(pkg_row(st, p));
        }
        vec![Box::new(col)]
    });
    Box::new(Column::new().gap(10.0).child(field).child(ScrollView::new().vertical().child(list).class("grow")).class("pane"))
}

fn updates_view(st: St) -> W {
    let list = Reactive::new(move || -> Vec<W> {
        let Some(ups) = st.updates.get() else {
            return vec![Box::new(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(CircularProgress::new().indeterminate().size(20.0)).child(Text::new("Проверка обновлений…").class("muted")))];
        };
        let aur = ups.iter().any(|u| u.aur);
        let mut col = Column::new().gap(4.0);
        let head = Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new(if ups.is_empty() { "Система обновлена".to_string() } else { format!("Доступно обновлений: {}", ups.len()) }).class("h2 grow"))
            .child(Button::new("Проверить").class("btn").on_click(move || check_updates(st)));
        col = col.child(head);
        if !ups.is_empty() {
            col = col.child(Button::new("Обновить всё").class("btn primary").on_click(move || run_op(st, "Обновление системы".into(), Op::Upgrade { aur })));
        }
        for u in ups {
            col = col.child(
                DecoratedBox::new()
                    .child(
                        Row::new()
                            .gap(10.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(pkg_icon(&u.name))
                            .child(Column::new().gap(2.0).child(Text::new(u.name.clone()).class("pkg-name")).child(Text::new(format!("{} → {}", u.old, u.new)).class("pkg-desc")).class("grow"))
                            .child(chip(if u.aur { "AUR".into() } else { "репозиторий".into() }, if u.aur { "chip-aur" } else { "chip-repo" })),
                    )
                    .class("pkg-row"),
            );
        }
        vec![Box::new(col)]
    });
    Box::new(Column::new().gap(10.0).child(ScrollView::new().vertical().child(list).class("grow")).class("pane"))
}

fn jobs_view(st: St) -> W {
    let list = Reactive::new(move || -> Vec<W> {
        let jobs = st.jobs.get();
        let mut col = Column::new().gap(10.0);
        if jobs.is_empty() {
            col = col.child(Text::new("Установки и обновления появятся здесь").class("muted empty"));
        }
        for j in jobs {
            let (icon, class) = match &j.done {
                None => ("\u{E863}", "job-running"),
                Some(Ok(())) => ("\u{E86C}", "job-ok"),
                Some(Err(_)) => ("\u{E000}", "job-err"),
            };
            let mut log = Column::new().gap(0.0);
            for l in j.log.iter().rev().take(60).collect::<Vec<_>>().into_iter().rev() {
                log = log.child(Text::new(l.clone()).selectable(true).class("log-line"));
            }
            let mut card = Column::new()
                .gap(8.0)
                .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icon).class(format!("job-icon {class}"))).child(Column::new().gap(0.0).child(Text::new(j.title.clone()).class("pkg-name")).child(Text::new(j.stage.clone()).class("pkg-desc")).class("grow")));
            if j.done.is_none() {
                card = card.child(ProgressBar::new().indeterminate());
            }
            if let Some(Err(e)) = &j.done {
                card = card.child(Text::new(e.clone()).selectable(true).class("job-error"));
            }
            card = card.child(DecoratedBox::new().child(ScrollView::new().vertical().follow_end(true).child(log)).class("log"));
            col = col.child(DecoratedBox::new().child(card).class("job"));
        }
        vec![Box::new(col)]
    });
    Box::new(Column::new().child(ScrollView::new().vertical().child(list).class("grow")).class("pane"))
}

fn details_view(st: St, p: Pkg) -> W {
    let name = p.name.clone();
    let installed = p.installed.is_some();
    let aur = p.source == Source::Aur;
    let mut actions = Row::new().gap(8.0);
    if installed {
        let n = name.clone();
        actions = actions.child(Button::new("Удалить").class("btn danger").on_click(move || run_op(st, format!("Удаление {n}"), Op::Remove(vec![n.clone()]))));
        if let Some(e) = synshell_common::xdg::app_by_id(&name) {
            let cmd = e.command();
            actions = actions.child(Button::new("Открыть").class("btn").on_click(move || {
                let _ = std::process::Command::new("sh").arg("-c").arg(&cmd).spawn();
            }));
        }
    } else {
        let n = name.clone();
        let op = if aur { Op::InstallAur(vec![n.clone()]) } else { Op::Install(vec![n.clone()]) };
        actions = actions.child(Button::new("Установить").class("btn primary").on_click(move || run_op(st, format!("Установка {n}"), op.clone())));
    }
    let head = Row::new()
        .gap(14.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(pkg_icon(&name)).class("big-icon"))
        .child(Column::new().gap(4.0).child(Text::new(name.clone()).class("h1")).child(Row::new().gap(6.0).child(Text::new(p.version.clone()).class("pkg-ver")).child(source_chip(&p))).class("grow"));
    let mut col = Column::new().gap(14.0).child(head).child(Text::new(p.description.clone()).class("desc")).child(actions);
    if aur {
        let n = name.clone();
        col = col.child(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .gap(6.0)
                        .child(Text::new("Пакет из AUR собирают пользователи, а не Arch Linux. Проверьте PKGBUILD перед установкой.").class("warn-text"))
                        .child(Button::new("Показать PKGBUILD").class("btn small").on_click(move || show_pkgbuild(st, n.clone()))),
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
                out = out.child(DecoratedBox::new().child(table).class("card"));
                if !d.depends.is_empty() {
                    let mut deps = Flex::new().gap(6.0);
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

#[allow(dead_code)]
fn _px(v: f32) -> StyleValue {
    StyleValue::px(v)
}
