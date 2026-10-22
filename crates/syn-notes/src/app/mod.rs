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

use syngui::prelude::*;
use syngui::widgets::feedback::NotificationHost;
use syngui::widgets::overlay::PortalAnchor;
use synshell_common::Config;

use crate::config::AppConfig;
use crate::context::AppCtx;
use crate::notes::{self, NotesCtx};

pub mod desk;
mod rail_view;
pub mod server;
pub mod theme;

const APP_ID: &str = "syn-notes";


struct Args {
    background: bool,
    command: Option<server::Command>,
    screenshot: Option<String>,
}

fn parse_args() -> Args {
    let mut a = Args { background: false, command: None, screenshot: None };
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
    if let Some(cmd) = args.command.clone() {
        server::handle(cmd);
    }
    server::start();
    watch_shell_config(theme);

    App::new()
        .title(syngui::tr!("notes.title"))
        .app_id(APP_ID)
        .size(1280, 820)
        .min_size(720, 480)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(theme)
        .start_hidden(args.background)
        .hide_on_close(true)
        .run(|_| Box::new(root()));
}

/// Контексты, автосохранение, напоминания и память о проектах — один раз
/// на процесс.
fn install_state() {
    let cfg = AppConfig::load();
    let app = AppCtx::new(&cfg);
    provide_context(app.clone());
    provide_context(NotesCtx::new_or_restore(&cfg));
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

/// Корень окна: панель проектов, заметки, диалоги и тосты.
fn root() -> impl Widget {
    let app = use_context::<AppCtx>();
    let notes_view = DecoratedBox::new().class("grow").child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        syngui::i18n::subscribe();
        let ctx = use_context::<NotesCtx>();
        if ctx.has_project() {
            vec![Box::new(notes::view())]
        } else {
            vec![Box::new(notes::project_ui::start_screen())]
        }
    }));
    let shell = notes::project_ui::hotkey_scope(
        DecoratedBox::new().class("window-backdrop").child(
            DecoratedBox::new().clip(true).class("shell").child(
                Row::new()
                    .gap(0.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(rail_view::view())
                    .child(notes_view),
            ),
        ),
    );
    let always = use_signal(true);
    let toasts = Portal::new()
        .is_open(always)
        .modal(false)
        .backdrop(false)
        .anchor(PortalAnchor::BottomEnd { margin_bottom: 16.0, margin_right: 16.0 })
        .child(NotificationHost::new(app.notifications.clone()).grow_up(true).class("synthos-notification-host"));
    Stack::new()
        .clip(false)
        .child(shell)
        .child(notes::project_ui::rename_dialog())
        .child(toasts)
}
