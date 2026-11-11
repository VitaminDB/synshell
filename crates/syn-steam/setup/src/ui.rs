//! Окно первой установки: согласие с Соглашением подписчика Steam, ход загрузки, «нет сети» с повтором.
//! Своя рамка (заголовок со знаком, перетаскивание, кнопки окна из настроек рабочего стола); в узком окне
//! (телефон) — без заголовка, страница на весь экран.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::window::WindowState;
use synshell_common::Config;

use crate::install::{self, Step};
use crate::manifest::Manifest;
use crate::Opts;

pub const SSA_URL: &str = "https://store.steampowered.com/subscriber_agreement/";

type W = Box<dyn Widget>;

fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

fn icon(g: &str) -> syngui::widget::StyledWidget<Icon> {
    Icon::new(g).class("ic")
}

/// Кодовые точки Material Icons.
mod icons {
    pub const GAME: &str = "\u{E338}";
    pub const DOWNLOAD: &str = "\u{E2C4}";
    pub const OFFLINE: &str = "\u{E2C1}";
    pub const ERROR: &str = "\u{E000}";
    pub const REFRESH: &str = "\u{E5D5}";
}

/// Код выхода окна: 0 — поставлено, 1 — ошибка, 2 — отменено.
static EXIT: AtomicI32 = AtomicI32::new(2);
static CANCEL: AtomicBool = AtomicBool::new(false);

pub fn offline_text() -> String {
    t!("Нет связи с серверами Steam. Проверьте подключение к интернету и попробуйте снова.")
}

#[derive(Clone, PartialEq)]
enum Phase {
    /// Читаем манифест.
    Checking,
    /// Манифест есть — ждём согласия.
    Ready(Arc<Manifest>),
    Busy(String, Option<f32>),
    Offline(String),
    Error(String),
}

#[derive(Clone, Copy)]
struct St {
    phase: RwSignal<Phase>,
    /// Последний прочитанный манифест — версия и размер видны и во время загрузки.
    manifest: RwSignal<Option<Arc<Manifest>>>,
    window: RwSignal<WindowState>,
    opts: &'static Opts,
}

fn mb(n: u64) -> String {
    t!(
        "{n} МБ",
        n = synshell_common::decimal(n as f64 / 1048576.0, 1)
    )
}

fn set_error(st: St, e: anyhow::Error) {
    if install::is_cancelled(&e) {
        return;
    }
    if install::is_offline(&e) {
        st.phase.set(Phase::Offline(format!("{e:#}")));
    } else {
        st.phase.set(Phase::Error(format!("{e:#}")));
    }
}

/// Манифест в фоне: размер и версия для страницы согласия (и ранняя проверка сети).
fn check(st: St) {
    st.phase.set(Phase::Checking);
    let channel = st.opts.channel;
    std::thread::spawn(move || {
        let r = install::fetch_manifest(channel);
        run_on_main_thread(move || match r {
            Ok((m, _)) => {
                let m = Arc::new(m);
                st.manifest.set(Some(m.clone()));
                st.phase.set(Phase::Ready(m));
            }
            Err(e) => set_error(st, e),
        });
    });
}

fn start(st: St) {
    if matches!(st.phase.get_untracked(), Phase::Busy(..)) {
        return;
    }
    st.phase
        .set(Phase::Busy(t!("Подключение к серверу Steam…"), None));
    let opts = st.opts;
    std::thread::spawn(move || {
        let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let r = install::install(&opts.root, opts.channel, &CANCEL, &mut |s| {
            let (text, frac) = match s {
                Step::Download {
                    done,
                    total,
                    index,
                    count,
                } => {
                    // не чаще 10 раз в секунду
                    if done < total && last.elapsed() < std::time::Duration::from_millis(100) {
                        return;
                    }
                    (
                        t!(
                            "Скачивание {d} из {t} (файл {i} из {n})",
                            d = mb(done),
                            t = mb(total),
                            i = index + 1,
                            n = count
                        ),
                        Some(if total > 0 {
                            done as f32 / total as f32
                        } else {
                            0.0
                        }),
                    )
                }
                Step::Unpack { index, count } => (
                    t!("Распаковка {i} из {n}…", i = index + 1, n = count),
                    Some((index as f32 + 0.5) / count.max(1) as f32),
                ),
            };
            last = std::time::Instant::now();
            run_on_main_thread(move || st.phase.set(Phase::Busy(text, frac)));
        });
        run_on_main_thread(move || match r {
            Ok(_) => {
                EXIT.store(0, Ordering::SeqCst);
                syngui::signal::quit_app();
            }
            Err(e) => set_error(st, e),
        });
    });
}

fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
    EXIT.store(2, Ordering::SeqCst);
    syngui::signal::quit_app();
}

fn open_ssa() {
    let _ = std::process::Command::new("xdg-open").arg(SSA_URL).spawn();
}

fn page(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let phase = st.phase.get();
        let mut col = Column::new()
            .gap(16.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch);
        let (hero, hero_class, title) = match &phase {
            Phase::Offline(_) => (icons::OFFLINE, "hero warn", t!("Нет подключения")),
            Phase::Error(_) => (
                icons::ERROR,
                "hero err",
                t!("Не получилось установить Steam"),
            ),
            _ => (icons::GAME, "hero", t!("Установка Steam")),
        };
        col = col.child(
            Row::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(
                    DecoratedBox::new()
                        .class(hero_class)
                        .child(icon(hero).class("ic hero-ic")),
                )
                .child(
                    DecoratedBox::new()
                        .class("grow")
                        .child(Text::new(title).max_lines(2).class("title")),
                ),
        );
        match &phase {
            Phase::Offline(details) => {
                col = col.child(Text::new(offline_text()).max_lines(6));
                col = col.child(
                    Text::new(details.clone())
                        .max_lines(4)
                        .class("muted small mono"),
                );
            }
            Phase::Error(e) => {
                col = col.child(Text::new(e.clone()).max_lines(10).class("small"));
            }
            _ => {
                col = col.child(Text::new(t!("Клиент Steam для ARM64 скачивается один раз с серверов Valve и ставится в {r}. Дальше Steam обновляется сам.", r = st.opts.root.display())).max_lines(6));
                let info = match &phase {
                    Phase::Ready(m) => t!(
                        "Версия {v}, скачать {s}.",
                        v = &m.version,
                        s = mb(m.download_size())
                    ),
                    Phase::Checking => t!("Проверка версии…"),
                    _ => st.manifest.get().map(|m| t!("Версия {v}, скачать {s}.", v = &m.version, s = mb(m.download_size()))).unwrap_or_default(),
                };
                if !info.is_empty() {
                    col = col.child(Text::new(info).max_lines(2).class("muted"));
                }
                col = col.child(
                    DecoratedBox::new().class("card").child(
                        Column::new()
                            .gap(8.0)
                            .cross_axis_alignment(CrossAxisAlignment::Start)
                            .child(Text::new(t!("Устанавливая Steam, вы принимаете Соглашение подписчика Steam.")).max_lines(4).class("small"))
                            .child(Button::new(t!("Открыть соглашение")).class("flat link").on_click(open_ssa)),
                    ),
                );
            }
        }
        vec![boxed(col)]
    });
    let footer = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let phase = st.phase.get();
        let mut col = Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch);
        if let Phase::Busy(text, frac) = &phase {
            let bar = match frac {
                Some(f) => ProgressBar::new().value(f.clamp(0.0, 1.0)),
                None => ProgressBar::new().indeterminate(),
            };
            col = col
                .child(bar.class("bar"))
                .child(Text::new(text.clone()).max_lines(2).class("muted small"));
        }
        let mut row = Row::new()
            .gap(10.0)
            .main_axis_alignment(MainAxisAlignment::End)
            .cross_axis_alignment(CrossAxisAlignment::Center);
        row = row.child(Button::new(t!("Отмена")).on_click(cancel));
        row = match &phase {
            Phase::Offline(_) => row.child(
                Button::new(t!("Повторить"))
                    .icon(icons::REFRESH)
                    .class("primary")
                    .on_click(move || check(st)),
            ),
            Phase::Error(_) => row.child(
                Button::new(t!("Повторить"))
                    .icon(icons::REFRESH)
                    .class("primary")
                    .on_click(move || start(st)),
            ),
            Phase::Ready(_) => row.child(
                Button::new(t!("Установить"))
                    .icon(icons::DOWNLOAD)
                    .class("primary")
                    .on_click(move || start(st)),
            ),
            Phase::Busy(..) => row,
            Phase::Checking => row.child(
                Button::new(t!("Установить"))
                    .icon(icons::DOWNLOAD)
                    .class("primary")
                    .disabled(true),
            ),
        };
        col = col.child(row);
        vec![boxed(col)]
    });
    boxed(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                DecoratedBox::new().class("grow").child(
                    ScrollView::new()
                        .vertical()
                        .child(DecoratedBox::new().class("page").child(body)),
                ),
            )
            .child(DecoratedBox::new().class("footer").child(footer)),
    )
}

fn titlebar(st: St) -> W {
    let controls = Reactive::new(move || -> Vec<W> {
        let w = st.window.get();
        vec![boxed(
            SystemWindowControls::new(ControlsSide::Right)
                .maximized(w.maximized)
                .active(w.focused),
        )]
    });
    boxed(
        DecoratedBox::new().class("titlebar").child(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(
                    DecoratedBox::new().class("tb-left").child(
                        Row::new()
                            .gap(10.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(
                                DecoratedBox::new()
                                    .class("brand")
                                    .child(icon(icons::GAME).class("ic brand-ic")),
                            )
                            .child(Text::new("Steam").max_lines(1).class("brand-title")),
                    ),
                )
                .child(
                    DecoratedBox::new().class("grow drag-wrap").child(
                        WindowDragRegion::new().child(DecoratedBox::new().class("drag-space")),
                    ),
                )
                .child(DecoratedBox::new().class("window-controls").child(controls)),
        ),
    )
}

/// Телефон: окно на весь экран, без рамки и изменения размера (заголовок с кнопками окна есть).
fn is_phone() -> bool {
    std::env::var("SYNSHELL_FORM_FACTOR").is_ok_and(|v| v == "phone")
}

fn root(st: St) -> W {
    let w = !is_phone();
    boxed(Reactive::new(move || -> Vec<W> {
        let mut main = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch);
        if synshell_common::window_controls(false) {
            main = main.child(titlebar(st));
        }
        main = main.child(DecoratedBox::new().class("grow root").child(page(st)));
        let frame = DecoratedBox::new()
            .class(if w {
                "window-frame"
            } else {
                "window-frame flat"
            })
            .clip(true)
            .child(main);
        vec![boxed(
            WindowResizeRegion::new()
                .over_content(true)
                .inset(5.0)
                .enabled(w)
                .child(frame),
        )]
    }))
}

fn theme(cfg: &Config) -> String {
    use synshell_common::config::Rgba;
    use synshell_common::style::{px, AppStyle};
    let a = &cfg.appearance;
    let p = a.palette();
    let titlebar = if a.is_dark() {
        p.bg.mix(Rgba::rgb(0, 0, 0), 0.22)
    } else {
        p.surface_alt.mix(p.bg, 0.5)
    };
    // знак — в цветах Steam
    AppStyle::new(cfg, "syn-steam-setup")
        .font_selector("Text, Button")
        .vars(&[
            ("titlebar", titlebar.hex()),
            ("window-radius", px(cfg.decorations.corner_radius.max(0.0))),
        ])
        .base(include_str!("../styles/syn-steam-setup.mss"))
        .build()
}

/// Окно установки; код выхода.
pub fn run(opts: Opts) -> i32 {
    let opts: &'static Opts = Box::leak(Box::new(opts));
    let window = use_signal(WindowState::default());
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    App::new()
        .title(t!("Установка Steam"))
        .app_id("syn-steam-setup")
        .size(480, 560)
        .min_size(320, 420)
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(synshell_common::live::styles(mss, theme))
        .with_window_state(window)
        .run(move |_| {
            let st = St {
                phase: use_signal(Phase::Checking),
                manifest: use_signal(None),
                window,
                opts,
            };
            check(st);
            root(st)
        });
    EXIT.load(Ordering::SeqCst)
}

/// `--screenshot СНИМОК.png [--size ШxВ] [--scale N] [--page ask|busy|offline|error] [--phone]`
pub fn screenshot(args: Vec<String>) {
    let mut it = args.into_iter();
    let Some(out) = it.next() else { return };
    let mut size = (480u32, 560u32);
    let mut scale = 2.0f64;
    let mut page = String::from("ask");
    while let Some(a) = it.next() {
        match a.as_str() {
            "--size" => {
                if let Some((w, h)) = it.next().as_deref().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = it.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            "--page" => page = it.next().unwrap_or(page),
            "--phone" => std::env::set_var("SYNSHELL_FORM_FACTOR", "phone"),
            _ => {}
        }
    }
    let opts: &'static Opts = Box::leak(Box::new(Opts {
        root: "/home/user/.local/share/Steam".into(),
        channel: crate::manifest::Channel::Stable,
    }));
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    let r = syngui::shot::render_png(
        move || {
            let demo = Manifest {
                platform: "linuxarm64".into(),
                version: "1792618285".into(),
                components: vec![crate::manifest::Component {
                    name: "bins_linuxarm64_linuxarm64".into(),
                    file: "x".into(),
                    size: 287_000_000,
                    sha256: "0".repeat(64),
                }],
            };
            let phase = match page.as_str() {
                "busy" => Phase::Busy("Скачивание 120,4 из 273,7 МБ (файл 5 из 17)".into(), Some(0.44)),
                "offline" => Phase::Offline("https://client-update.fastly.steamstatic.com/steam_client_linuxarm64: dns failed".into()),
                "error" => Phase::Error("bins_linuxarm64_linuxarm64.zip.dc81: другой файл — 1000 байт".into()),
                _ => Phase::Ready(Arc::new(demo.clone())),
            };
            let st = St {
                manifest: use_signal(match &phase {
                    Phase::Busy(..) => Some(Arc::new(demo.clone())),
                    _ => None,
                }),
                phase: use_signal(phase),
                window: use_signal(WindowState::default()),
                opts,
            };
            root(st)
        },
        &mss,
        size,
        scale,
        &out,
    );
    match r {
        Ok((w, h)) => println!("{out}: {w}x{h}"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
