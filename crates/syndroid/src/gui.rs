//! Окно управления Android — `syndroid` без аргументов. Одно окно для рабочего стола (навигация слева,
//! подробности приложения справа) и телефона (нижняя навигация, подробности — отдельным экраном), как
//! «Программы» (synpkg). Всё — через демон (`api::call`) в фоновых потоках.
//!
//! - «Обзор»: состояние, запуск/остановка/перезапуск/заморозка, «Показать Android», задания, ошибки;
//! - «Приложения»: приложения запущенного Android — открыть, остановить, очистить данные, удалить;
//!   установка APK;
//! - «Образы»: экземпляры Android (свои данные и приложения у каждого) и их наборы образов — загрузка
//!   (LineageOS / с GApps), проверка обновлений, импорт, выбор, удаление, сброс данных;
//! - «Настройки»: автозапуск, режим окон, сеть, плотность экрана, свойства Android, OTA-каналы;
//! - «Журнал»: logcat с фильтром.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use syngui::widgets::{EventHook, KeyReply};
use syngui::{GestureDetector, IntoWidget, ShowIf};
use synshell_common::Config as ShellConfig;

use crate::api::{self, Instance, Request, Response, Session, State, Status};
use crate::config::Config;
use crate::images;

type W = Box<dyn Widget>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    Apps,
    Images,
    Settings,
    Log,
}

impl Tab {
    const ALL: [Tab; 5] = [Tab::Overview, Tab::Apps, Tab::Images, Tab::Settings, Tab::Log];
    fn label(self) -> &'static str {
        match self {
            Tab::Overview => "Обзор",
            Tab::Apps => "Приложения",
            Tab::Images => "Образы",
            Tab::Settings => "Настройки",
            Tab::Log => "Журнал",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Tab::Overview => "\u{E859}",  // android
            Tab::Apps => "\u{E5C3}",      // apps
            Tab::Images => "\u{E53B}",    // layers
            Tab::Settings => "\u{E8B8}",  // settings
            Tab::Log => "\u{E873}",       // description
        }
    }
}

/// Приложение Android: пакет, имя и значок — из ярлыка syndroid (`xdg::android_apps_dir`).
#[derive(Clone, PartialEq)]
struct AppRow {
    package: String,
    name: String,
    icon: Option<PathBuf>,
}

/// Подтверждение опасного действия.
#[derive(Clone)]
struct Confirm {
    id: u64,
    title: String,
    text: String,
    button: String,
    action: Arc<dyn Fn() + Send + Sync>,
}

impl PartialEq for Confirm {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id
    }
}

#[derive(Clone, Copy)]
struct St {
    tab: RwSignal<Tab>,
    status: RwSignal<Option<Status>>,
    apps: RwSignal<Option<Vec<AppRow>>>,
    app_filter: RwSignal<String>,
    selected: RwSignal<Option<AppRow>>,
    instances: RwSignal<Option<Vec<Instance>>>,
    /// Ответ проверки обновлений (текст).
    updates: RwSignal<String>,
    /// 0 — LineageOS, 1 — LineageOS с GApps.
    variant: RwSignal<usize>,
    import_system: RwSignal<String>,
    import_vendor: RwSignal<String>,
    apk: RwSignal<String>,
    /// Черновик настроек (сохраняется кнопкой). Представление его не отслеживает — иначе каждый символ в
    /// поле пересоздавал бы поля (терялся фокус); пересборка — по `cfg_rev`.
    cfg: RwSignal<Option<Config>>,
    cfg_rev: RwSignal<u64>,
    log: RwSignal<String>,
    log_filter: RwSignal<String>,
    toast: RwSignal<String>,
    confirm: RwSignal<Option<Confirm>>,
    /// Перерисовать списки, зависящие от данных демона (приложения, образы).
    rev: RwSignal<u64>,
    /// Android загружен — меняется только при смене состояния (статус обновляется каждые 1,5 с, и
    /// представления с полями ввода от него не зависят).
    running: RwSignal<bool>,
}

static CONFIRM_ID: AtomicU64 = AtomicU64::new(1);

/// Окно управления; `images` — сразу на «Образах» (из уведомления о загрузке).
pub fn run(images: bool) {
    let (scfg, _) = ShellConfig::load();
    let mss = theme(&scfg);
    App::new()
        .title("Android")
        .app_id("syndroid")
        .size(1040, 720)
        .min_size(340, 480)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .run(move |_| {
            let st = St {
                tab: use_signal(if images { Tab::Images } else { Tab::Overview }),
                status: use_signal(None),
                apps: use_signal(None),
                app_filter: use_signal(String::new()),
                selected: use_signal(None),
                instances: use_signal(None),
                updates: use_signal(String::new()),
                variant: use_signal(0usize),
                import_system: use_signal(String::new()),
                import_vendor: use_signal(String::new()),
                apk: use_signal(String::new()),
                cfg: use_signal(None),
                cfg_rev: use_signal(0u64),
                log: use_signal(String::new()),
                log_filter: use_signal(String::new()),
                toast: use_signal(String::new()),
                confirm: use_signal(None),
                rev: use_signal(0u64),
                running: use_signal(false),
            };
            poll_status(st);
            load_instances(st);
            load_config(st);
            // Android загрузился (или сменился) — перечитать приложения
            let last: RwSignal<Option<(State, Option<String>)>> = use_signal(None);
            create_effect(move || {
                let s = st.status.get();
                let key = s.as_ref().map(|s| (s.state, s.instance.clone()));
                if key != last.get_untracked() {
                    last.set(key.clone());
                    // Замороженный (Android сам «уснул» без окон) — тоже рабочий: действия будят его
                    let running = matches!(key, Some((State::Running | State::Frozen, _)));
                    if st.running.get_untracked() != running {
                        st.running.set(running);
                    }
                    match key {
                        Some((State::Running | State::Frozen, _)) => load_apps(st),
                        _ => {
                            st.apps.set(None);
                            st.selected.set(None);
                        }
                    }
                    load_instances(st);
                }
            });
            root(st)
        });
}

fn theme(cfg: &ShellConfig) -> String {
    let a = &cfg.appearance;
    let mut s = a.mss_variables();
    let p = a.palette();
    let dark = a.is_dark();
    s.push_str(&format!(
        ":root {{\n  --input-bg: {};\n  --accent-hover: {};\n  --danger-soft: {};\n}}\n",
        if dark { p.bg.mix(p.surface, 0.35) } else { p.surface }.hex(),
        p.accent.mix(p.fg, 0.15).hex(),
        p.danger.with_alpha(0.16).hex(),
    ));
    s.push_str(&a.theme_mss_variables());
    s.push_str(include_str!("../styles/syndroid.mss"));
    if !a.font.trim().is_empty() {
        s.push_str(&format!("Text, Button, TextField {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    s
}

// ─── Фоновые запросы ─────────────────────────────────────────────────────────

/// Выполнить в потоке, результат — в главном потоке.
fn bg<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static, done: impl FnOnce(anyhow::Result<T>) + Send + 'static) {
    std::thread::spawn(move || {
        let r = f();
        run_on_main_thread(move || done(r));
    });
}

/// Запрос к демону с подсказкой об успехе или ошибке; после — обновить состояние.
fn act(st: St, req: Request, ok: &'static str) {
    let removes = matches!(req, Request::RemoveImages { .. } | Request::RemoveInstance { .. });
    bg(move || {
        let r = api::call(&req);
        // Удалённые образы — убрать их приложения из меню
        if removes && r.is_ok() {
            crate::bridge::prune_entries();
        }
        r
    }, move |r| {
        match r {
            Ok(Response::Job { .. }) => {
                st.toast.set(ok.to_string());
                st.tab.set(st.tab.get_untracked());
            }
            Ok(_) => {
                if !ok.is_empty() {
                    st.toast.set(ok.to_string());
                }
            }
            Err(e) => st.toast.set(format!("{e:#}")),
        }
        load_instances(st);
        refresh_status(st);
    });
}

fn refresh_status(st: St) {
    bg(|| api::call(&Request::Status), move |r| {
        if let Ok(Response::Status(s)) = r {
            if st.status.get_untracked().as_ref() != Some(&s) {
                st.status.set(Some(s));
            }
        }
    });
}

/// Состояние демона — раз в полторы секунды (пока окно открыто). Заодно — каталог ярлыков экземпляра:
/// мост меняет его при установке/удалении приложений в Android, и список перечитывается.
fn poll_status(st: St) {
    let mut seen: Option<(String, std::time::SystemTime)> = None;
    std::thread::spawn(move || loop {
        let r = api::call(&Request::Status);
        if let Ok(Response::Status(s)) = &r {
            if let Some(inst) = &s.instance {
                let dir = synshell_common::xdg::android_apps_dir().join(inst);
                if let Ok(m) = std::fs::metadata(&dir).and_then(|m| m.modified()) {
                    let now = Some((inst.clone(), m));
                    if seen.is_some() && seen != now {
                        run_on_main_thread(move || {
                            if st.running.get_untracked() {
                                load_apps(st);
                            }
                        });
                    }
                    seen = now;
                }
            }
        }
        run_on_main_thread(move || match r {
            Ok(Response::Status(s)) => {
                // Задание закончилось — списки образов могли измениться
                let finished_now = st.status.get_untracked().is_some_and(|old| {
                    old.jobs.iter().any(|j| !j.finished && s.jobs.iter().any(|n| n.id == j.id && n.finished))
                });
                if st.status.get_untracked().as_ref() != Some(&s) {
                    st.status.set(Some(s));
                }
                if finished_now {
                    load_instances(st);
                    if st.status.get_untracked().is_some_and(|s| s.state == State::Running) {
                        load_apps(st);
                    }
                }
            }
            Ok(_) => {}
            Err(e) => {
                if st.status.get_untracked().is_some() {
                    st.toast.set(format!("{e:#}"));
                }
                st.status.set(None);
            }
        });
        std::thread::sleep(Duration::from_millis(1500));
    });
}

fn load_instances(st: St) {
    bg(|| api::call(&Request::Instances), move |r| {
        if let Ok(Response::Instances { instances }) = r {
            st.instances.set(Some(instances));
            st.rev.set(st.rev.get_untracked() + 1);
        }
    });
}

fn load_config(st: St) {
    bg(|| api::call(&Request::GetConfig), move |r| {
        if let Ok(Response::Config { config }) = r {
            st.cfg.set(Some(config));
            st.cfg_rev.set(st.cfg_rev.get_untracked() + 1);
        }
    });
}

/// Приложения Android — из ярлыков его экземпляра (мост держит их в согласии с Android: имена, значки,
/// пакеты). Запрос к самому Android не нужен — замороженный не просыпается.
fn load_apps(st: St) {
    let instance = st.status.get_untracked().and_then(|s| s.instance);
    bg(
        move || {
            let mut rows: Vec<AppRow> = synshell_common::xdg::load_apps()
                .into_iter()
                .filter(|e| e.android.as_ref().map(|a| &a.instance) == instance.as_ref())
                .filter_map(|e| {
                    let package = e.wm_class.strip_prefix("waydroid.")?.to_string();
                    let icon = Some(PathBuf::from(&e.icon)).filter(|p| p.is_absolute() && p.exists());
                    Some(AppRow { package, name: e.name, icon })
                })
                .collect();
            rows.sort_by_key(|r| r.name.to_lowercase());
            Ok(rows)
        },
        move |r: anyhow::Result<Vec<AppRow>>| match r {
            Ok(v) => {
                st.apps.set(Some(v));
                st.rev.set(st.rev.get_untracked() + 1);
            }
            Err(e) => st.toast.set(format!("{e:#}")),
        },
    );
}

fn load_log(st: St) {
    bg(|| api::call(&Request::Logcat { lines: 800 }), move |r| match r {
        Ok(Response::Log { text }) => st.log.set(text),
        Ok(_) => {}
        Err(e) => st.log.set(format!("{e:#}")),
    });
}

fn ask(st: St, title: &str, text: &str, button: &str, action: impl Fn() + Send + Sync + 'static) {
    st.confirm.set(Some(Confirm {
        id: CONFIRM_ID.fetch_add(1, Ordering::Relaxed),
        title: title.into(),
        text: text.into(),
        button: button.into(),
        action: Arc::new(action),
    }));
}

// ─── Общее ───────────────────────────────────────────────────────────────────

use crate::notify::human;

fn uptime(s: u64) -> String {
    if s >= 3600 {
        format!("{} ч {} мин", s / 3600, s % 3600 / 60)
    } else if s >= 60 {
        format!("{} мин {} с", s / 60, s % 60)
    } else {
        format!("{s} с")
    }
}

fn date(ts: i64) -> String {
    // Без зависимостей: дни от эпохи → гражданская дата (алгоритм Хиннанта)
    let days = ts.div_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{d:02}.{m:02}.{y}")
}

fn chip(label: impl Into<String>, class: &str) -> impl Widget {
    DecoratedBox::new().child(Text::new(label.into()).class("chip-text")).class(format!("chip {class}"))
}

fn state_chip(s: State) -> impl Widget {
    match s {
        State::Running => chip("работает", "chip-ok"),
        State::Starting => chip("загружается", "chip-busy"),
        State::Frozen => chip("заморожен", "chip-frozen"),
        State::Stopping => chip("останавливается", "chip-busy"),
        State::Stopped => chip("остановлен", "chip-off"),
    }
}

fn card<M>(child: impl IntoWidget<M>) -> syngui::widget::StyledWidget<DecoratedBox> {
    DecoratedBox::new().child(child).class("card")
}

fn kv(k: &str, v: impl Into<String>) -> impl Widget {
    Row::new().gap(10.0).child(Text::new(k.to_string()).class("k")).child(Text::new(v.into()).selectable(true).class("v grow")).class("kv")
}

fn app_icon(icon: &Option<PathBuf>, class: &str) -> W {
    match icon {
        Some(p) => Box::new(Image::new(p.to_string_lossy()).fit(ImageFit::Contain).placeholder(false).class(class.to_string())),
        None => Box::new(DecoratedBox::new().child(Icon::new("\u{E859}").class("pkg-glyph")).class("pkg-glyph-box")),
    }
}

fn empty(icon: &str, text: &str) -> syngui::widget::StyledWidget<DecoratedBox> {
    DecoratedBox::new()
        .child(Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icon.to_string()).class("empty-icon")).child(Text::new(text.to_string()).class("muted")))
        .class("detail-empty")
}

fn pane<M>(st: St, title: &str, body: impl IntoWidget<M>) -> W {
    let _ = st;
    Box::new(ScrollView::new().vertical().child(Column::new().gap(14.0).child(Text::new(title.to_string()).class("h1")).child(body).class("pane")))
}

// ─── Каркас ──────────────────────────────────────────────────────────────────

fn root(st: St) -> W {
    let narrow = syngui::viewport::viewport_below(720.0);
    let body = Reactive::new(move || -> Vec<W> { vec![if narrow.get() { phone(st) } else { desktop(st) }] });
    let toast = Column::new()
        .main_axis_alignment(MainAxisAlignment::End)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Reactive::new(move || -> Vec<W> {
            let t = st.toast.get();
            if t.is_empty() {
                return vec![];
            }
            vec![Box::new(DecoratedBox::new().child(Text::new(t).max_lines(3).class("toast-text")).class("toast"))]
        }))
        .class("toast-place");
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
                        .child(confirm_overlay(st)),
                ),
            ),
    )
}

fn go_back(st: St) -> bool {
    if st.confirm.get_untracked().is_some() {
        st.confirm.set(None);
        true
    } else if st.selected.get_untracked().is_some() {
        st.selected.set(None);
        true
    } else {
        false
    }
}

fn confirm_overlay(st: St) -> W {
    let shown = use_signal(0usize);
    create_effect(move || shown.set(usize::from(st.confirm.get().is_some())));
    let card = Reactive::new(move || -> Vec<W> {
        let Some(c) = st.confirm.get() else { return vec![] };
        let action = c.action.clone();
        let col = Column::new()
            .gap(12.0)
            .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new("\u{E002}").class("confirm-icon")).child(Text::new(c.title.clone()).class("auth-title grow")))
            .child(Text::new(c.text.clone()).class("desc"))
            .child(
                Row::new()
                    .gap(8.0)
                    .main_axis_alignment(MainAxisAlignment::End)
                    .child(Button::new("Отмена").on_click(move || st.confirm.set(None)))
                    .child(Button::new(c.button.clone()).class("danger").on_click(move || {
                        st.confirm.set(None);
                        action();
                    })),
            );
        vec![Box::new(DecoratedBox::new().child(col).class("auth-card"))]
    });
    Box::new(ShowIf::new(1, shown).child(GestureDetector::new().on_click(|| {}).child(DecoratedBox::new().child(card).class("auth-scrim"))))
}

fn tab_content(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        vec![match st.tab.get() {
            Tab::Overview => overview(st),
            Tab::Apps => apps_view(st),
            Tab::Images => images_view(st),
            Tab::Settings => settings_view(st),
            Tab::Log => log_view(st),
        }]
    }))
}

fn select_tab(st: St, t: Tab) {
    st.selected.set(None);
    st.tab.set(t);
    match t {
        Tab::Log => load_log(st),
        Tab::Images => load_instances(st),
        Tab::Settings => load_config(st),
        _ => {}
    }
}

fn desktop(st: St) -> W {
    let nav = Reactive::new(move || -> Vec<W> {
        let cur = st.tab.get();
        let mut col = Column::new().gap(4.0).child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new("\u{E859}").class("brand-icon"))
                .child(Text::new("Android").class("brand")),
        );
        for t in Tab::ALL {
            col = col.child(GestureDetector::new().on_click(move || select_tab(st, t)).child(
                DecoratedBox::new()
                    .child(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(t.icon()).class("nav-icon")).child(Text::new(t.label()).class("nav-label")))
                    .class(if cur == t { "nav-item active" } else { "nav-item" }),
            ));
        }
        vec![Box::new(col)]
    });
    let detail = Reactive::new(move || -> Vec<W> {
        if st.tab.get() != Tab::Apps {
            return vec![];
        }
        vec![Box::new(
            DecoratedBox::new()
                .child(match st.selected.get() {
                    Some(a) => app_details(st, a),
                    None => Box::new(empty("\u{E859}", "Выберите приложение")),
                })
                .class("detail-pane"),
        )]
    });
    Box::new(
        Row::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(DecoratedBox::new().child(nav).class("sidebar"))
            .child(DecoratedBox::new().child(tab_content(st)).class("grow list-pane"))
            .child(detail),
    )
}

fn phone(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let open = st.selected.get().is_some();
        vec![Box::new(
            AnimatedSwitcher::new(if open { 2u64 } else { 1 }, move || -> Box<dyn Widget> {
                match st.selected.get_untracked() {
                    Some(a) => Box::new(
                        Column::new()
                            .child(
                                Row::new()
                                    .gap(6.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .child(GestureDetector::new().on_click(move || st.selected.set(None)).child(DecoratedBox::new().child(Icon::new("\u{E5C4}").class("back-icon")).class("back")))
                                    .child(Text::new(a.name.clone()).max_lines(1).class("bar-title grow"))
                                    .class("bar"),
                            )
                            .child(DecoratedBox::new().child(app_details(st, a)).class("grow")),
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
            let item = Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().child(Icon::new(t.icon()).class("nb-icon")).class(if cur == t { "nb-pill nb-pill-on" } else { "nb-pill" }))
                .child(Text::new(t.label()).max_lines(1).class("nb-label"));
            row = row.child(GestureDetector::new().on_click(move || select_tab(st, t)).child(DecoratedBox::new().child(item).class("nb-item")));
        }
        vec![Box::new(row.class("navbar"))]
    });
    Box::new(Column::new().child(DecoratedBox::new().child(body).class("grow")).child(navbar))
}

// ─── Обзор ───────────────────────────────────────────────────────────────────

fn overview(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let Some(s) = st.status.get() else {
            return vec![Box::new(card(
                Column::new()
                    .gap(8.0)
                    .child(Text::new("Нет связи со службой syndroid").class("h2"))
                    .child(Text::new("Служба syndroid.service не запущена: sudo systemctl enable --now syndroid").class("muted")),
            ))];
        };
        let mut out: Vec<W> = Vec::new();
        // Карточка Android
        let title = s.instance_title.clone().unwrap_or_else(|| "Android не установлен".into());
        let head = Row::new()
            .gap(14.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().child(Icon::new("\u{E859}").class("hero-icon")).class("hero-icon-box"))
            .child(Column::new().gap(4.0).child(Text::new(title).class("h1")).child(Row::new().gap(6.0).child(state_chip(s.state))).class("grow"));
        let mut info = Column::new().gap(0.0);
        if let Some(i) = &s.image {
            info = info.child(kv("Образы", i.clone()));
        }
        if let Some(v) = &s.android_version {
            info = info.child(kv("Android", v.clone()));
        }
        if let Some(u) = s.uptime {
            info = info.child(kv("Работает", uptime(u)));
        }
        if let Some(m) = s.memory {
            info = info.child(kv("Память", human(m)));
        }
        let mut buttons = Flex::new().wrap().gap(8.0);
        let has_images = s.image.is_some();
        match s.state {
            State::Stopped => {
                buttons = buttons.child(Button::new("Запустить").class("primary").disabled(!has_images).on_click(move || match Session::from_env() {
                    Ok(session) => act(st, Request::Start { session }, "Android запускается"),
                    Err(e) => st.toast.set(format!("{e:#}")),
                }));
            }
            State::Running => {
                buttons = buttons
                    .child(Button::new("Показать Android").class("primary").on_click(move || act(st, Request::ShowFullUi, "")))
                    .child(Button::new("Перезапустить").on_click(move || act(st, Request::Restart, "Android перезапускается")))
                    .child(Button::new("Заморозить").on_click(move || act(st, Request::Freeze, "Android заморожен")))
                    .child(Button::new("Остановить").class("danger").on_click(move || act(st, Request::Stop, "Android остановлен")));
            }
            State::Frozen => {
                buttons = buttons
                    .child(Button::new("Разморозить").class("primary").on_click(move || act(st, Request::Unfreeze, "")))
                    .child(Button::new("Остановить").class("danger").on_click(move || act(st, Request::Stop, "Android остановлен")));
            }
            State::Starting | State::Stopping => {
                buttons = buttons.child(Button::new("Остановить").class("danger").on_click(move || act(st, Request::Stop, "Android остановлен")));
            }
        }
        let mut col = Column::new().gap(14.0).child(head);
        if s.state == State::Starting {
            col = col.child(ProgressBar::new().indeterminate());
        }
        col = col.child(info).child(buttons);
        out.push(Box::new(card(col).class("card hero")));
        if !has_images {
            out.push(Box::new(card(
                Column::new()
                    .gap(10.0)
                    .child(Text::new("Образов Android ещё нет").class("h2"))
                    .child(Text::new("Скачайте LineageOS — около 1 ГБ — в разделе «Образы».").class("muted"))
                    .child(Button::new("К образам").class("primary").on_click(move || select_tab(st, Tab::Images))),
            )));
        }
        if let Some(e) = &s.error {
            out.push(Box::new(DecoratedBox::new().child(Text::new(e.clone()).class("warn-text")).class("warn")));
        }
        for j in s.jobs.iter().filter(|j| !j.finished || j.error.is_some()) {
            out.push(job_card(st, j));
        }
        vec![Box::new(Column::new().gap(14.0).children(out))]
    });
    pane(st, "Обзор", body)
}

fn job_card(st: St, j: &api::Job) -> W {
    let mut col = Column::new().gap(8.0).child(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(if j.error.is_some() { "\u{E000}" } else { "\u{E2C4}" }).class(if j.error.is_some() { "job-icon job-err" } else { "job-icon job-running" }))
            .child(Text::new(j.title.clone()).class("h2 grow")),
    );
    if let Some(e) = &j.error {
        col = col.child(Text::new(e.clone()).class("job-error"));
    } else if j.retry_at.is_some() {
        // Ждёт повтора: последняя ошибка и время следующей попытки
        if let Some(e) = &j.last_error {
            col = col.child(Text::new(e.clone()).class("job-error"));
        }
        col = col.child(Text::new(crate::notify::job_line(j)).class("muted"));
    } else {
        col = col.child(Text::new(crate::notify::job_line(j)).class("muted"));
        col = col.child(if j.total > 0 { ProgressBar::with_value(j.done as f32 / j.total as f32) } else { ProgressBar::new().indeterminate() });
    }
    if j.cancellable && !j.finished {
        let id = j.id;
        let mut row = Flex::new().wrap().gap(8.0);
        if j.retry_at.is_some() {
            row = row.child(Button::new("Повторить сейчас").class("primary").on_click(move || act(st, Request::RetryJobNow { id }, "")));
        }
        row = row.child(Button::new("Отменить").on_click(move || act(st, Request::CancelJob { id }, "Загрузка отменена")));
        col = col.child(row);
    }
    Box::new(DecoratedBox::new().child(col).class("job"))
}

// ─── Приложения ──────────────────────────────────────────────────────────────

fn apps_view(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let running = st.running.get();
        if !running {
            return vec![Box::new(
                Column::new().gap(10.0).child(empty("\u{E859}", "Android не запущен")).child(
                    Row::new().main_axis_alignment(MainAxisAlignment::Center).child(Button::new("Запустить").class("primary").on_click(move || match Session::from_env() {
                        Ok(session) => act(st, Request::Start { session }, "Android запускается"),
                        Err(e) => st.toast.set(format!("{e:#}")),
                    })),
                ),
            )];
        }
        let install = Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(TextField::with_text(st.apk.get_untracked()).placeholder("Путь к файлу .apk").prefix_icon("\u{E2C4}").on_change(move |t| st.apk.set(t.to_string())).class("grow"))
            .child(Button::new("Установить").class("primary").on_click(move || {
                let p = st.apk.get_untracked().trim().to_string();
                let p = synshell_common::paths::expand_tilde(&p);
                if !p.is_file() {
                    st.toast.set("Нет такого файла".into());
                    return;
                }
                let p = std::fs::canonicalize(&p).unwrap_or(p);
                act(st, Request::InstallApk { path: p.display().to_string() }, "Установка APK началась");
            }));
        let filter = TextField::with_text(st.app_filter.get_untracked()).placeholder("Найти приложение").prefix_icon("\u{E8B6}").on_change(move |t| st.app_filter.set(t.to_string())).class("search");
        let list = Reactive::new(move || -> Vec<W> {
            let q = st.app_filter.get().to_lowercase();
            let sel = st.selected.get().map(|a| a.package);
            let rows: Vec<W> = match st.apps.get() {
                None => vec![Box::new(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(CircularProgress::new().indeterminate().size(22.0)))],
                Some(v) => v
                    .into_iter()
                    .filter(|a| q.is_empty() || a.name.to_lowercase().contains(&q) || a.package.contains(&q))
                    .map(|a| -> W {
                        let on = sel.as_deref() == Some(a.package.as_str());
                        let a2 = a.clone();
                        Box::new(GestureDetector::new().on_click(move || st.selected.set(Some(a2.clone()))).child(
                            DecoratedBox::new()
                                .child(
                                    Row::new()
                                        .gap(12.0)
                                        .cross_axis_alignment(CrossAxisAlignment::Center)
                                        .child(app_icon(&a.icon, "pkg-icon"))
                                        .child(Column::new().gap(2.0).child(Text::new(a.name.clone()).max_lines(1).class("pkg-name")).child(Text::new(a.package.clone()).max_lines(1).class("pkg-desc")).class("grow")),
                                )
                                .class(if on { "pkg-row pkg-row-on" } else { "pkg-row" }),
                        ))
                    })
                    .collect(),
            };
            // Reactive не раскладывает детей — один столбец
            vec![Box::new(Column::new().gap(2.0).children(rows))]
        });
        let jobs = Reactive::new(move || -> Vec<W> {
            let v: Vec<W> = st.status.get().map(|s| s.jobs.iter().filter(|j| !j.finished || j.error.is_some()).map(|j| job_card(st, j)).collect()).unwrap_or_default();
            vec![Box::new(Column::new().gap(12.0).children(v))]
        });
        vec![Box::new(Column::new().gap(12.0).child(card(Column::new().gap(8.0).child(Text::new("Установить APK").class("h2")).child(install))).child(jobs).child(filter).child(Column::new().gap(2.0).child(list)))]
    });
    pane(st, "Приложения", body)
}

fn app_details(st: St, a: AppRow) -> W {
    let (p1, p2, p3, p4) = (a.package.clone(), a.package.clone(), a.package.clone(), a.package.clone());
    let name = a.name.clone();
    let name2 = a.name.clone();
    let actions = Flex::new()
        .wrap()
        .gap(8.0)
        .child(Button::new("Открыть").class("primary").on_click(move || act(st, Request::LaunchApp { package: p1.clone() }, "")))
        .child(Button::new("Остановить").on_click(move || act(st, Request::StopApp { package: p2.clone() }, "Приложение остановлено")))
        .child(Button::new("Очистить данные").on_click(move || {
            let p = p3.clone();
            ask(st, "Очистить данные?", &format!("Все данные «{name}» (вход, настройки, файлы приложения) будут удалены."), "Очистить", move || act(st, Request::ClearAppData { package: p.clone() }, "Данные очищены"))
        }))
        .child(Button::new("Удалить").class("danger").on_click(move || {
            let p = p4.clone();
            ask(st, "Удалить приложение?", &format!("«{name2}» будет удалено из Android вместе с его данными."), "Удалить", move || {
                st.selected.set(None);
                act(st, Request::UninstallApp { package: p.clone() }, "Приложение удалено");
                let st2 = st;
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(2));
                    run_on_main_thread(move || load_apps(st2));
                });
            })
        }));
    let head = Row::new()
        .gap(14.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(app_icon(&a.icon, "big-app-icon")).class("big-icon"))
        .child(Column::new().gap(4.0).child(Text::new(a.name.clone()).class("h1")).child(Text::new(a.package.clone()).selectable(true).class("pkg-ver")).class("grow"));
    let instance = st.status.get_untracked().and_then(|s| s.instance_title).unwrap_or_default();
    Box::new(ScrollView::new().vertical().child(
        Column::new()
            .gap(14.0)
            .child(head)
            .child(actions)
            .child(card(Column::new().gap(0.0).child(kv("Пакет", a.package.clone())).child(kv("Android", instance))))
            .class("detail"),
    ))
}

// ─── Образы и экземпляры ─────────────────────────────────────────────────────

fn images_view(st: St) -> W {
    let fetch = Reactive::new(move || -> Vec<W> {
        let v = st.variant.get();
        let upd = st.updates.get();
        let mut col = Column::new()
            .gap(10.0)
            .child(Text::new("Скачать Android").class("h2"))
            .child(Text::new("Свежая сборка LineageOS из OTA-канала Waydroid (около 1 ГБ). Вариант с GApps — с сервисами Google; это отдельный Android со своими приложениями и данными.").class("muted"))
            .child(SegmentedButton::new(vec!["LineageOS", "LineageOS + GApps"]).selected(v).on_change(move |i| {
                st.variant.set(i);
                st.updates.set(String::new());
            }))
            .child(
                Flex::new()
                    .wrap()
                    .gap(8.0)
                    .child(Button::new("Скачать").class("primary").on_click(move || {
                        let t = if st.variant.get_untracked() == 1 { "GAPPS" } else { "VANILLA" };
                        act(st, Request::FetchImages { system_type: Some(t.into()), session: Session::from_env().ok() }, "Загрузка началась — ход в уведомлении, здесь и в «Обзоре»; окно можно закрыть");
                    }))
                    .child(Button::new("Проверить обновления").on_click(move || {
                        let t = if st.variant.get_untracked() == 1 { "GAPPS" } else { "VANILLA" };
                        st.updates.set("Проверка…".into());
                        bg(
                            move || {
                                let Response::Config { config: mut c } = api::call(&Request::GetConfig)? else { anyhow::bail!("ответ") };
                                c.system_type = t.into();
                                let (s, ven) = images::ota_latest(&c)?;
                                let name = images::set_name(&s.filename);
                                let have = images::exists(&name);
                                Ok(format!(
                                    "Последняя сборка: {} от {} ({} + {}){}",
                                    images::instance_title(&images::instance_of(&name)),
                                    date(s.datetime),
                                    human(s.size),
                                    human(ven.size),
                                    if have { " — уже установлена" } else { " — можно скачать" }
                                ))
                            },
                            move |r| st.updates.set(r.unwrap_or_else(|e| format!("{e:#}"))),
                        );
                    })),
            );
        if !upd.is_empty() {
            col = col.child(Text::new(upd).class("desc"));
        }
        vec![Box::new(card(col))]
    });
    let import = card(
        Column::new()
            .gap(8.0)
            .child(Text::new("Импорт из файлов").class("h2"))
            .child(Text::new("Архивы OTA (.zip) или образы (.img) system и vendor.").class("muted"))
            .child(TextField::with_text(st.import_system.get_untracked()).placeholder("system: путь к .zip или .img").on_change(move |t| st.import_system.set(t.to_string())))
            .child(TextField::with_text(st.import_vendor.get_untracked()).placeholder("vendor: путь к .zip или .img").on_change(move |t| st.import_vendor.set(t.to_string())))
            .child(Row::new().child(Button::new("Импортировать").on_click(move || {
                let abs = |s: String| std::fs::canonicalize(synshell_common::paths::expand_tilde(s.trim())).map(|p| p.display().to_string());
                match (abs(st.import_system.get_untracked()), abs(st.import_vendor.get_untracked())) {
                    (Ok(system), Ok(vendor)) => act(st, Request::ImportImages { system, vendor, name: None }, "Импорт начался"),
                    _ => st.toast.set("Нет такого файла".into()),
                }
            }))),
    );
    let list = Reactive::new(move || -> Vec<W> {
        let _ = st.rev.get();
        let status = st.status.get();
        let jobs: Vec<W> = status.as_ref().map(|s| s.jobs.iter().filter(|j| !j.finished || j.error.is_some()).map(|j| job_card(st, j)).collect()).unwrap_or_default();
        let mut out: Vec<W> = jobs;
        out.push(Box::new(Text::new("Установленные").class("h2")));
        match st.instances.get() {
            None => out.push(Box::new(CircularProgress::new().indeterminate().size(22.0))),
            Some(v) if v.is_empty() => out.push(Box::new(Text::new("Образов Android нет.").class("muted"))),
            Some(v) => {
                let active_set = status.as_ref().and_then(|s| s.image.clone());
                for i in v {
                    out.push(instance_card(st, i, active_set.clone(), status.as_ref().map(|s| s.state).unwrap_or(State::Stopped)));
                }
            }
        }
        vec![Box::new(Column::new().gap(12.0).children(out))]
    });
    pane(st, "Образы", Column::new().gap(14.0).child(fetch).child(list).child(import))
}

fn instance_card(st: St, i: Instance, active_set: Option<String>, state: State) -> W {
    let running_here = i.active && state != State::Stopped;
    let mut head = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Icon::new("\u{E859}").class("inst-icon"))
        .child(Column::new().gap(2.0).child(Text::new(i.title.clone()).class("h2")).child(Text::new(format!("данные: {}", human(i.data_size))).class("pkg-ver")).class("grow"));
    if i.active {
        head = head.child(chip(if running_here { "запущен" } else { "выбран" }, "chip-ok"));
    }
    let mut col = Column::new().gap(10.0).child(head);
    if i.sets.is_empty() {
        col = col.child(Text::new("Образов нет — остались только данные.").class("muted"));
    }
    for s in &i.sets {
        let used = active_set.as_deref() == Some(s.name.as_str());
        let (n1, n2) = (s.name.clone(), s.name.clone());
        let mut row = Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Column::new().gap(1.0).child(Text::new(s.name.clone()).max_lines(1).class("set-name")).child(Text::new(format!("{} · установлен {}", human(s.size), date(s.installed))).class("pkg-ver")).class("grow"));
        if used {
            row = row.child(chip("используется", "chip-repo"));
        } else {
            row = row.child(Button::new("Использовать").class("small").on_click(move || {
                act(st, Request::UseImages { name: n1.clone() }, "Выбрано — подействует при следующем запуске Android");
            }));
            row = row.child(Button::new("Удалить").class("small danger").on_click(move || {
                let n = n2.clone();
                ask(st, "Удалить образы?", &format!("Набор {n} будет удалён с диска. Данные Android останутся."), "Удалить", move || act(st, Request::RemoveImages { name: n.clone() }, "Образы удалены"));
            }));
        }
        col = col.child(DecoratedBox::new().child(row).class("set-row"));
    }
    let (id1, id2, id3) = (i.id.clone(), i.id.clone(), i.id.clone());
    let (t2, t3) = (i.title.clone(), i.title.clone());
    let mut actions = Flex::new().wrap().gap(8.0);
    if !i.active {
        if let Some(latest) = i.sets.first().map(|s| s.name.clone()) {
            actions = actions.child(Button::new("Выбрать этот Android").on_click(move || {
                let _ = &id1;
                act(st, Request::UseImages { name: latest.clone() }, "Выбрано — подействует при следующем запуске Android");
            }));
        }
    }
    actions = actions
        .child(Button::new("Сбросить данные").class("small").disabled(running_here).on_click(move || {
            let id = id2.clone();
            ask(st, "Сбросить данные?", &format!("Все приложения и данные «{t2}» будут удалены; образы останутся. Android запустится как новый."), "Сбросить", move || act(st, Request::ResetData { instance: id.clone() }, "Данные сброшены"));
        }))
        .child(Button::new("Удалить Android").class("small danger").disabled(running_here).on_click(move || {
            let id = id3.clone();
            ask(st, "Удалить этот Android?", &format!("«{t3}» будет удалён целиком: образы, приложения и данные."), "Удалить", move || act(st, Request::RemoveInstance { instance: id.clone() }, "Android удалён"));
        }));
    col = col.child(actions);
    Box::new(card(col))
}

// ─── Настройки ───────────────────────────────────────────────────────────────

fn settings_view(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let _ = st.cfg_rev.get();
        let Some(c) = st.cfg.get_untracked() else {
            return vec![Box::new(CircularProgress::new().indeterminate().size(22.0))];
        };
        // Правка черновика без пересборки представления
        let set = move |f: Box<dyn Fn(&mut Config) + Send + Sync>| {
            st.cfg.update(|o| {
                if let Some(c) = o {
                    f(c);
                }
            });
        };
        // Правка, меняющая состав полей (добавить/удалить свойство) — с пересборкой
        let reshape = move |f: Box<dyn Fn(&mut Config) + Send + Sync>| {
            set(f);
            st.cfg_rev.set(st.cfg_rev.get_untracked() + 1);
        };
        let toggle = |label: &str, note: &str, on: bool, f: fn(&mut Config, bool)| -> W {
            Box::new(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Column::new().gap(2.0).child(Text::new(label.to_string()).class("opt-label")).child(Text::new(note.to_string()).class("pkg-desc")).class("grow"))
                    .child(Toggle::with_state(on).on_change(move |v| set(Box::new(move |c| f(c, v)))))
                    .class("opt"),
            )
        };
        let general = card(
            Column::new()
                .gap(4.0)
                .child(toggle("Запускать при входе", "Android стартует вместе с сеансом (около 1 ГБ памяти).", c.autostart, |c, v| c.autostart = v))
                .child(toggle(
                    "Каждое приложение — своим окном",
                    "Окна freeform для рабочего стола. На телефоне лучше выключить: приложение на весь экран. Действует после перезапуска Android.",
                    c.multi_windows,
                    |c, v| c.multi_windows = v,
                ))
                .child(toggle("Сеть", "Доступ Android в интернет через телефон или компьютер.", c.network, |c, v| c.network = v))
                .child(toggle(
                    "Общие папки",
                    "Загрузки, Изображения, Музыка, Видео и Документы — те же, что в Android (Download, Pictures…). Действует после перезапуска Android.",
                    c.shared_folders,
                    |c, v| c.shared_folders = v,
                ))
                .child(
                    Row::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Column::new().gap(2.0).child(Text::new("Плотность экрана (dpi)").class("opt-label")).child(Text::new("0 — как решит Android; больше — крупнее интерфейс.").class("pkg-desc")).class("grow"))
                        .child(TextField::with_text(c.dpi.to_string()).on_change(move |t| {
                            let v = t.trim().parse().unwrap_or(0);
                            set(Box::new(move |c| c.dpi = v));
                        }).class("num"))
                        .class("opt"),
                ),
        );
        // Свойства Android: строки «имя = значение»
        let mut props = Column::new().gap(6.0);
        for (i, (k, v)) in c.properties.iter().enumerate() {
            let (k0, k1) = (k.clone(), k.clone());
            let _ = i;
            props = props.child(
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(k.clone()).max_lines(1).class("prop-key grow"))
                    .child(TextField::with_text(v.clone()).on_change(move |t| {
                        let (k, t) = (k0.clone(), t.to_string());
                        set(Box::new(move |c| {
                            c.properties.insert(k.clone(), t.clone());
                        }));
                    }).class("prop-val"))
                    .child(GestureDetector::new().on_click(move || {
                        let k = k1.clone();
                        reshape(Box::new(move |c| {
                            c.properties.remove(&k);
                        }));
                    }).child(DecoratedBox::new().child(Icon::new("\u{E5CD}").class("prop-del-icon")).class("prop-del"))),
            );
        }
        let new_key = use_signal(String::new());
        props = props.child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(TextField::new().placeholder("новое свойство, например persist.sys.timezone").on_change(move |t| new_key.set(t.to_string())).class("grow"))
                .child(Button::new("Добавить").class("small").on_click(move || {
                    let k = new_key.get_untracked().trim().to_string();
                    if !k.is_empty() && !k.contains(['=', ' ']) {
                        reshape(Box::new(move |c| {
                            c.properties.entry(k.clone()).or_default();
                        }));
                    }
                })),
        );
        let props = card(
            Column::new()
                .gap(8.0)
                .child(Text::new("Свойства Android").class("h2"))
                .child(Text::new("Добавляются к свойствам образа (vendor/waydroid.prop) при запуске Android; перекрывают и настройки устройства.").class("muted"))
                .child(props),
        );
        let ota = card(
            Column::new()
                .gap(8.0)
                .child(Text::new("OTA-каналы").class("h2"))
                .child(TextField::with_text(c.system_channel.clone()).placeholder("system").on_change(move |t| {
                    let t = t.to_string();
                    set(Box::new(move |c| c.system_channel = t.clone()));
                }))
                .child(TextField::with_text(c.vendor_channel.clone()).placeholder("vendor").on_change(move |t| {
                    let t = t.to_string();
                    set(Box::new(move |c| c.vendor_channel = t.clone()));
                })),
        );
        let num = |label: &str, note: &str, v: u32, f: fn(&mut Config, u32)| -> W {
            Box::new(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Column::new().gap(2.0).child(Text::new(label.to_string()).class("opt-label")).child(Text::new(note.to_string()).class("pkg-desc")).class("grow"))
                    .child(TextField::with_text(v.to_string()).on_change(move |t| {
                        if let Ok(v) = t.trim().parse::<u32>() {
                            set(Box::new(move |c| f(c, v)));
                        }
                    }).class("num"))
                    .class("opt"),
            )
        };
        let (rmin, rmax) = (*crate::config::RETRY_MINUTES.start(), *crate::config::RETRY_MINUTES.end());
        let (tmin, tmax) = (*crate::config::TIMEOUT_SECONDS.start(), *crate::config::TIMEOUT_SECONDS.end());
        let download = card(
            Column::new()
                .gap(4.0)
                .child(Text::new("Загрузка образов").class("h2"))
                .child(toggle(
                    "Повторять при сбое",
                    "Нет сети или сервер недоступен — загрузка ждёт и пробует снова сама, в фоне (окно можно закрыть); начатое докачивается.",
                    c.retry,
                    |c, v| c.retry = v,
                ))
                .child(num("Пауза между попытками, мин", &format!("От {rmin} до {rmax}."), c.retry_minutes, |c, v| c.retry_minutes = v))
                .child(num("Тайм-аут соединения, с", &format!("Сколько ждать ответа сервера ({tmin}–{tmax})."), c.connect_timeout, |c, v| c.connect_timeout = v))
                .child(num(
                    "Тайм-аут простоя, с",
                    &format!("Загрузка без единого байта дольше этого считается оборванной ({tmin}–{tmax})."),
                    c.stall_timeout,
                    |c, v| c.stall_timeout = v,
                )),
        );
        let save = Row::new()
            .gap(8.0)
            .child(Button::new("Сохранить").class("primary").on_click(move || {
                if let Some(c) = st.cfg.get_untracked() {
                    let c = c.clamped();
                    st.cfg.set(Some(c.clone()));
                    st.cfg_rev.set(st.cfg_rev.get_untracked() + 1);
                    act(st, Request::SetConfig { config: c }, "Сохранено — часть настроек действует со следующего запуска Android");
                }
            }))
            .child(Button::new("Отменить изменения").on_click(move || load_config(st)));
        vec![Box::new(Column::new().gap(14.0).child(general).child(download).child(props).child(ota).child(save))]
    });
    pane(st, "Настройки", body)
}

// ─── Журнал ──────────────────────────────────────────────────────────────────

fn log_view(st: St) -> W {
    let top = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(TextField::with_text(st.log_filter.get_untracked()).placeholder("Фильтр").prefix_icon("\u{E8B6}").on_change(move |t| st.log_filter.set(t.to_string())).class("grow search"))
        .child(Button::new("Обновить").on_click(move || load_log(st)));
    let lines = Reactive::new(move || -> Vec<W> {
        let q = st.log_filter.get().to_lowercase();
        let text = st.log.get();
        if text.is_empty() {
            return vec![Box::new(Text::new("Журнал пуст или Android не запущен").class("muted"))];
        }
        let v: Vec<&str> = text.lines().filter(|l| q.is_empty() || l.to_lowercase().contains(&q)).collect();
        let mut col = Column::new().gap(0.0);
        // Свежие — сверху
        for l in v.iter().rev().take(400) {
            let class = if l.contains(" E/") || l.contains(" F/") { "log-line log-err" } else if l.contains(" W/") { "log-line log-warn" } else { "log-line" };
            col = col.child(Text::new(l.to_string()).selectable(true).class(class));
        }
        vec![Box::new(col)]
    });
    pane(st, "Журнал Android", Column::new().gap(10.0).child(top).child(DecoratedBox::new().child(lines).class("log")))
}
