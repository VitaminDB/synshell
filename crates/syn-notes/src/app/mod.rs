//! Приложение syn-notes: резидентный хозяин проектов заметок и его окно.
//!
//! Процесс стартует с сеансом (`syn-notes --background`) без окна и живёт
//! весь сеанс: автосохранение, напоминания и сервер сокета работают и при
//! спрятанном окне. Закрытие окна его прячет; на Wayland окно вместе с
//! деревом виджетов уничтожается и строится заново при показе — поэтому всё
//! состояние (контексты, сигналы, эффекты) поднимается здесь, до цикла
//! событий, а корень дерева только рисует.
//!
//! Второй запуск (`syn-notes`, `syn-notes open <страница>`) не открывает
//! проекты сам, а передаёт команду хозяину по сокету — см. [`server`].

use syngui::input::Key;
use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::widgets::feedback::NotificationHost;
use syngui::widgets::{EventHook, KeyReply, ToolButton};
use syngui::widgets::overlay::PortalAnchor;
use synshell_common::Config;

use crate::config::AppConfig;
use crate::context::AppCtx;
use crate::notes::{self, NotesCtx};

pub mod desk;
mod rail_view;
pub mod server;
mod shot;
pub mod theme;

const APP_ID: &str = "syn-notes";


struct Args {
    background: bool,
    command: Option<server::Command>,
    screenshot: Option<String>,
    rest: Vec<String>,
}

fn parse_args() -> Args {
    let mut a = Args { background: false, command: None, screenshot: None, rest: std::env::args().skip(1).collect() };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--background" => a.background = true,
            "--screenshot" => a.screenshot = it.next(),
            "open" => a.command = Some(server::Command::Open(it.next().unwrap_or_default())),
            "-h" | "--help" => {
                println!("syn-notes [--background] [open СТРАНИЦА|ПРОЕКТ.syn]");
                std::process::exit(0);
            }
            other if other.ends_with(".syn") => {
                a.command = Some(server::Command::Open(other.to_string()));
            }
            _ => {}
        }
    }
    a
}

pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
    let args = parse_args();
    if let Some(out) = args.screenshot.clone() {
        shot::run(shot::parse(out, &args.rest));
        return;
    }

    // Хозяин уже работает — передать ему команду и выйти.
    if args.screenshot.is_none() {
        let cmd = args.command.clone().unwrap_or(if args.background {
            server::Command::Ping
        } else {
            server::Command::Show
        });
        if server::send(&cmd).is_ok() {
            return;
        }
    }

    synshell_common::i18n::init(&crate::i18n::CATALOGS);
    let (cfg, _) = Config::load();
    let theme = use_signal(theme::mss(&cfg));
    install_state();
    // Основное хранилище всегда открыто: с ним работают ассистент, рабочий
    // стол и напоминания.
    if let Some(main) = server::main_storage() {
        let ctx = use_context::<NotesCtx>();
        if !ctx.projects.get_untracked().iter().any(|p| p.path == crate::notes::project::normalize_path(&main)) {
            let _ = ctx.open_project_quietly(&main);
        }
    }
    // Первый запуск (ни открытых, ни недавних проектов): сразу проект по
    // умолчанию — как у читалки, открыл и пишешь, без экрана «создать/открыть».
    {
        let ctx = use_context::<NotesCtx>();
        if ctx.projects.get_untracked().is_empty() && ctx.recent.get_untracked().is_empty() {
            if let Err(e) = ctx.ensure_project() {
                tracing::warn!("проект по умолчанию не создан: {}", e.message());
            }
        }
    }
    if let Some(cmd) = args.command.clone() {
        server::handle(cmd);
    }
    server::start();
    watch_shell_config(theme);

    let window = use_context::<WindowCtx>().state;
    App::new()
        .title(syngui::tr!("notes.title"))
        .app_id(APP_ID)
        .size(1280, 820)
        .min_size(360, 480)
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(theme)
        .with_window_state(window)
        .start_hidden(args.background)
        .hide_on_close(true)
        .run(|_| Box::new(root()));
}

/// Контексты, автосохранение, напоминания и память о проектах — один раз
/// на процесс.
pub(crate) fn install_state() {
    let cfg = AppConfig::load();
    let app = AppCtx::new(&cfg);
    provide_context(app.clone());
    provide_context(NotesCtx::new_or_restore(&cfg));
    provide_context(notes::library::Library::new());
    notes::kinds::install();
    provide_context(WindowCtx { state: use_signal(syngui::window::WindowState::default()) });
    notes::autosave::install_notes_autosave();
    notes::reminders::install_reminders();
    notes::right_panel::install_props_panel_memory();
    notes::projects::install_busy_retry();
    install_state_autosave(app);
    for (file, backup) in crate::fsutil::take_quarantined() {
        notify_quarantined(&file, &backup);
    }
    crate::fsutil::set_notifier(|file, backup| {
        syngui::async_runtime::run_on_main_thread(move || notify_quarantined(&file, &backup));
    });
}

fn notify_quarantined(file: &std::path::Path, backup: &std::path::Path) {
    use_context::<AppCtx>().notifications.error(syngui::tr!(
        "app.file.quarantined",
        file = crate::paths::pretty(file),
        backup = crate::paths::pretty(backup)
    ));
}

/// Открытые проекты, страницы, панели и разделители — в файл состояния.
fn install_state_autosave(app: AppCtx) {
    let notes = use_context::<NotesCtx>();
    create_effect(move || {
        let st = notes.persist();
        let cfg = AppConfig {
            notes_projects: Some(st.projects),
            notes_active_project: st.active_project,
            notes_recent: st.recent,
            notes_left_split_ratio: app.notes_left_split_ratio.get(),
            notes_right_split_ratio: app.notes_right_split_ratio.get(),
            notes_left: app.panels.notes.0.get(),
            notes_right: app.panels.notes.1.get(),
            // Поля миграции из synthos больше не нужны: проекты уже в списке.
            ..AppConfig::default()
        };
        if let Err(e) = cfg.save() {
            tracing::warn!("состояние заметок не записано: {e}");
        }
    });
}

/// Тема (и примерка из редактора тем) и язык меняются вместе с оформлением
/// оболочки. Таблица — своя (`theme::mss`, переменные synthos), не `AppStyle`.
fn watch_shell_config(theme: RwSignal<String>) {
    synshell_common::live::spawn(Vec::new(), move |cfg| {
        let mss = theme::mss(&cfg);
        syngui::async_runtime::run_on_main_thread(move || {
            synshell_common::i18n::apply(&cfg);
            if theme.get_untracked() != mss {
                theme.set(mss);
            }
        });
    });
}

/// Состояние окна (развёрнуто, фокус, полный экран) — для своего заголовка.
#[derive(Clone, Copy)]
pub struct WindowCtx {
    pub state: RwSignal<syngui::window::WindowState>,
}

type W = Box<dyn Widget>;

/// Широкое окно (заголовок с кнопками окна, панель проектов).
fn wide_signal() -> RwSignal<bool> {
    let narrow = viewport_below(720.0);
    let w = use_signal(!narrow.get_untracked());
    create_effect(move || {
        let v = !narrow.get();
        if w.get_untracked() != v {
            w.set(v);
        }
    });
    w
}

/// Свой заголовок окна (CSD): значок и имя, заголовок открытого, полный
/// экран, кнопки окна из настроек рабочего стола.
fn titlebar() -> W {
    let window = use_context::<WindowCtx>().state;
    let ctx = use_context::<NotesCtx>();
    let lib = use_context::<notes::library::Library>();
    let title = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let tree = ctx.tree.get();
        let t = match lib.screen.get() {
            notes::library::Screen::Open(id) => tree.title_of(&id).unwrap_or_default(),
            notes::library::Screen::Library => ctx.project_title.get(),
        };
        vec![Box::new(Text::new(t).max_lines(1).elide(Elide::End).class("tb-title"))]
    });
    let controls = Reactive::new(move || -> Vec<W> {
        let w = window.get();
        vec![Box::new(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    Box::new(
        DecoratedBox::new().class("titlebar").child(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Icon::new(crate::icons::MI_EDIT_NOTE).class("tb-brand-icon"))
                        .child(Text::new(syngui::tr!("notes.title")).class("tb-brand")),
                )
                .child(
                    DecoratedBox::new()
                        .class("grow")
                        .child(WindowDragRegion::new().child(DecoratedBox::new().class("drag-space").child(title))),
                )
                .child(fullscreen_button())
                .child(DecoratedBox::new().class("window-controls").child(controls)),
        ),
    )
}

fn fullscreen_button() -> W {
    let window = use_context::<WindowCtx>().state;
    Box::new(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let fs = window.get().fullscreen;
        vec![Box::new(
            ToolButton::new(if fs { crate::icons::MI_FULLSCREEN_EXIT } else { crate::icons::MI_FULLSCREEN })
                .tooltip(if fs { syngui::tr!("notes.fullscreen.exit") } else { syngui::tr!("notes.fullscreen.enter") })
                .on_click(syngui::signal::toggle_fullscreen)
                .class("tb-btn"),
        )]
    }))
}

/// Клавиши окна: F11 — полный экран; Esc / «назад» — выход из полного
/// экрана, затем назад по библиотеке.
fn keys(inner: impl Widget + 'static) -> W {
    let window = use_context::<WindowCtx>().state;
    let back = move || -> bool {
        if window.get_untracked().fullscreen {
            syngui::signal::set_fullscreen(false);
            return true;
        }
        let ctx = use_context::<NotesCtx>();
        use_context::<notes::library::Library>().back(ctx)
    };
    Box::new(
        EventHook::new()
            .on_back(back)
            .on_key_down(move |k, _m| {
                if k == Key::F11 {
                    syngui::signal::toggle_fullscreen();
                    return KeyReply::Handled;
                }
                KeyReply::Ignore
            })
            .child(inner),
    )
}

/// Корень окна: заголовок, панель проектов, библиотека или открытая
/// страница, диалоги и тосты. Дерево перестраивается только при смене
/// ширины окна или полноэкранного режима.
pub(crate) fn root() -> impl Widget {
    let window = use_context::<WindowCtx>().state;
    let wide = wide_signal();
    let full = use_signal(false);
    create_effect(move || {
        let v = window.get().fullscreen;
        if full.get_untracked() != v {
            full.set(v);
        }
    });
    Reactive::new(move || -> Vec<W> {
        let w = wide.get();
        let fs = full.get();
        vec![frame(wide, w && !fs)]
    })
}

fn frame(wide: RwSignal<bool>, chrome: bool) -> W {
    let app = use_context::<AppCtx>();
    let ctx = use_context::<NotesCtx>();
    let notes_view = DecoratedBox::new().class("grow").child(Stack::new().fit(StackFit::Expand).child(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let ctx = use_context::<NotesCtx>();
        if ctx.has_project() {
            vec![Box::new(notes::library::view(wide))]
        } else {
            vec![Box::new(notes::project_ui::start_screen())]
        }
    })));
    // Проекты переключаются в боковой панели библиотеки; без проекта —
    // прежний ряд плиток (создать / открыть).
    let mut row = Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow");
    if chrome && !ctx.has_project() {
        row = row.child(rail_view::view());
    }
    let mut col = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch);
    if chrome {
        col = col.child(titlebar());
    }
    let shell = notes::project_ui::hotkey_scope(
        DecoratedBox::new()
            .class("window-backdrop")
            .child(DecoratedBox::new().clip(true).class("shell").child(col.child(row.child(notes_view)))),
    );
    let always = use_signal(true);
    let toasts = Portal::new()
        .is_open(always)
        .modal(false)
        .backdrop(false)
        .anchor(PortalAnchor::BottomEnd { margin_bottom: 16.0, margin_right: 16.0 })
        .child(NotificationHost::new(app.notifications.clone()).grow_up(true).class("synthos-notification-host"));
    let stack = Stack::new()
        .clip(false)
        .fit(StackFit::Expand)
        .child(shell)
        .child(notes::project_ui::rename_dialog())
        .child(notes::library::rename_dialog(ctx))
        .child(toasts);
    Box::new(
        WindowResizeRegion::new().over_content(true).inset(5.0).enabled(chrome).child(
            DecoratedBox::new()
                .class(if chrome { "window-frame" } else { "window-frame flat" })
                .clip(true)
                .child(keys(stack)),
        ),
    )
}
