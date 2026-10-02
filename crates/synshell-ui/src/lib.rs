//! synshell-ui — общая оболочка synshell: панели и доки с апплетами, меню
//! запуска, всплывающие окна, уведомления (D-Bus), экранные подсказки,
//! блокировка, Alt+Tab, глобальное меню. Рисуется syngui на layer-shell
//! поверхностях (syngui-layer), с композитором synwm общается по IPC.
//!
//! Две оболочки собирают её по-своему: `syndesktop-shell` — обои и панели
//! по мониторам ([`manager`]), `synmobile-shell` — домашний экран, шторка,
//! жесты и те же панели/доки под телефонный форм-фактор
//! ([`ctx::ShellCtx::form_factor`]).

pub mod access;
pub mod actions;
pub mod android_boot;
pub mod anim;
pub mod app_colors;
pub mod applets;
pub mod appmenu;
pub mod clock;
pub mod incall;
pub mod commands;
pub mod ctx;
pub mod datetime;
pub mod dock;
pub mod edit;
pub mod gtkmenu;
pub mod ipc;
pub mod launcher;
pub mod launchers;
pub mod link;
pub mod location;
pub mod lock;
pub mod manager;
pub mod modem;
pub mod netmenu;
pub mod notifications;
pub mod osd;
pub mod panel;
pub mod popup;
pub mod recents;
pub mod rotation;
pub mod autobright;
pub mod camera;
pub mod shade;
pub mod start_menu;
pub mod switcher;
pub mod system;
pub mod theme;
pub mod tray;
pub mod ui;
pub mod xdg;

use std::sync::Arc;
use std::time::Duration;
use synshell_common::config::FormFactor;
use synshell_common::watch::FileWatcher;
use synshell_common::Config;
use syngui::prelude::*;

pub use ctx::ShellCtx;

/// Как собрать оболочку.
pub struct Shell {
    /// Имя для журнала и `--version`.
    pub name: &'static str,
    pub form_factor: FormFactor,
    /// Встроенные стили поверх общего `shell.mss` (например, `mobile.mss`).
    pub extra_mss: &'static str,
    /// Пользовательский файл стилей этой оболочки в `~/.config/synshell/`
    /// (поверх `theme.mss`).
    pub user_mss: Option<&'static str>,
    /// Создать поверхности оболочки (обои, панели, домашний экран…). Общие
    /// службы (часы, система, IPC, уведомления, всплывающие окна, OSD,
    /// Alt+Tab) уже запущены.
    pub install: Box<dyn FnOnce(ShellCtx)>,
}

/// Форм-фактор процесса: `SYNSHELL_FORM_FACTOR` (его выставляет композитор
/// детям), иначе `[platform] form_factor`, иначе рабочий стол.
pub fn detect_form_factor(cfg: &Config) -> FormFactor {
    cfg.process_form_factor()
}

/// Загрузить конфиг с политикой форм-фактора (как у композитора).
pub fn load_config(ff: FormFactor) -> (Config, Option<String>) {
    let (mut cfg, err) = Config::load();
    cfg.apply_form_factor(ff);
    (cfg, err)
}

/// Запустить оболочку; возвращается при завершении цикла.
pub fn run(shell: Shell) -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn,usvg=error"))
        .format_timestamp_millis()
        .init();
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("{} {}", shell.name, env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if let Err(e) = Config::ensure_file_exists() {
        log::warn!("не удалось создать конфиг: {e}");
    }
    theme::set_layer(shell.extra_mss, shell.user_mss);
    FORM_FACTOR.with(|f| f.set(shell.form_factor));
    let (config, err) = load_config(shell.form_factor);
    if let Some(e) = err {
        log::error!("ошибка в конфиге, взяты значения по умолчанию: {e}");
    }
    xdg::set_icon_theme(&config.appearance.icon_theme);
    syngui_layer::set_ui_zoom(config.appearance.ui_scale);
    xdg::warm_up();
    // Виброотклик: удержание пальцем (меню, перенос значков), переключатели.
    synshell_common::haptics::set_config(&config.haptics);
    syngui::input::set_haptic_handler(|h| {
        use synshell_common::haptics::{play, Feedback};
        play(match h {
            syngui::input::Haptic::LongPress => Feedback::LongPress,
            _ => Feedback::Tick,
        })
    });
    let mss = theme::build(&config);
    app_colors::sync(&config);
    let font = Some(config.appearance.font.trim().to_string()).filter(|f| !f.is_empty());
    let form_factor = shell.form_factor;
    let install = shell.install;

    syngui_layer::run(syngui_layer::RunOptions { font_family: font }, &mss, move || {
        let ctx = ShellCtx::new(config, form_factor);
        provide_context(ctx);
        clock::start(ctx);
        system::start(ctx);
        datetime::start(ctx);
        rotation::start(ctx);
        autobright::start(ctx);
        ipc::start(ctx);
        appmenu::install(ctx);
        notifications::start(ctx);
        link::start(ctx);
        modem::start(ctx);
        incall::start(ctx);
        location::start(ctx);
        access::start(ctx);
        camera::start(ctx);
        install(ctx);
        popup::install(ctx);
        osd::install(ctx);
        notifications::install(ctx);
        switcher::install(ctx);
        applets::taskbar::start_minimize_rects();
        watch_config();
        watch_apps();
        // Отладка: выполнить команды оболочки после старта
        // (`SYNSHELL_SHELL_EXEC="launcher;sleep 500;volume +5"`).
        if let Ok(cmds) = std::env::var("SYNSHELL_SHELL_EXEC") {
            let mut at = 400u64;
            for c in cmds.split(';').map(str::trim).filter(|c| !c.is_empty()) {
                if let Some(ms) = c.strip_prefix("sleep ").and_then(|v| v.trim().parse::<u64>().ok()) {
                    at += ms;
                    continue;
                }
                let c = c.to_string();
                syngui_layer::add_timer(Duration::from_millis(at), move || {
                    commands::handle(&c);
                    None
                });
            }
        }
    })
}

thread_local! {
    static FORM_FACTOR: std::cell::Cell<FormFactor> = const { std::cell::Cell::new(FormFactor::Desktop) };
    /// mtime config.toml, прочитанного последним `reload_config`.
    static LOADED_MTIME: std::cell::Cell<Option<std::time::SystemTime>> = const { std::cell::Cell::new(None) };
    /// Последнюю правку config.toml сделала сама оболочка.
    static SELF_WRITTEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Обработчики перечитывания конфига у конкретной оболочки.
    static RELOAD_HOOKS: std::cell::RefCell<Vec<Box<dyn Fn(ShellCtx, bool)>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Вызывать `f(ctx, structural)` после каждого перечитывания конфига.
pub fn on_reload(f: impl Fn(ShellCtx, bool) + 'static) {
    RELOAD_HOOKS.with(|h| h.borrow_mut().push(Box::new(f)));
}

/// Раз в секунду смотреть на config.toml, theme.mss и файлы темы.
fn watch_config() {
    let mut w = FileWatcher::new(theme::watched_files(&ShellCtx::get().config.get_untracked()));
    syngui_layer::add_timer(Duration::from_secs(1), move || {
        if w.poll() {
            // Файл только что записала сама оболочка (режим редактирования)
            // и уже перечитала — второй раз пересобирать панели незачем.
            if SELF_WRITTEN.with(|f| f.replace(false)) && LOADED_MTIME.with(|m| m.get()).is_some_and(|t| Some(t) == config_mtime()) {
                return Some(Duration::from_secs(1));
            }
            reload_config();
            // Тема могла смениться — следить за её файлами.
            w = FileWatcher::new(theme::watched_files(&ShellCtx::get().config.get_untracked()));
        }
        Some(Duration::from_secs(1))
    });
}

/// Раз в 2 с смотреть на каталоги `applications`: поставленная или
/// удалённая программа сразу появляется в меню и на домашнем экране.
/// Перечитывание — в фоне (темы значков разбираются десятки мс), затем
/// `apps_rev` пересобирает списки.
fn watch_apps() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let mut stamp = xdg::apps_stamp();
    let done = Arc::new(AtomicBool::new(false));
    let mut busy = false;
    syngui_layer::add_timer(Duration::from_secs(2), move || {
        if busy {
            if done.swap(false, Ordering::AcqRel) {
                busy = false;
                log::info!("приложения перечитаны: {}", xdg::apps().len());
                let ctx = ShellCtx::get();
                ctx.apps_rev.set(ctx.apps_rev.get_untracked() + 1);
            }
            return Some(Duration::from_millis(200));
        }
        let now = xdg::apps_stamp();
        if now != stamp {
            stamp = now;
            busy = true;
            let done = done.clone();
            std::thread::spawn(move || {
                xdg::reload_apps();
                done.store(true, Ordering::Release);
            });
            return Some(Duration::from_millis(200));
        }
        Some(Duration::from_secs(2))
    });
}

/// Перечитать конфиг сразу после собственной записи (режим редактирования).
pub fn reload_after_write() {
    SELF_WRITTEN.with(|f| f.set(true));
    reload_config();
}

fn config_mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(synshell_common::paths::config_file()).and_then(|m| m.modified()).ok()
}

/// Перечитать конфиг и тему, пересобрать поверхности.
pub fn reload_config() {
    LOADED_MTIME.with(|m| m.set(config_mtime()));
    let ctx = ShellCtx::get();
    let (cfg, err) = load_config(FORM_FACTOR.with(|f| f.get()));
    if let Some(e) = err {
        // Ошибку показываем уведомлением и оставляем прежний конфиг.
        log::error!("конфиг: {e}");
        osd::show(ctx, ui::mi::INFO, None, "Ошибка в config.toml".into());
        return;
    }
    log::info!("конфиг перечитан");
    xdg::set_icon_theme(&cfg.appearance.icon_theme);
    // Сменилось только оформление (тема, обои, анимации) — поверхности
    // остаются, цвета перетекают, картинка обоев растворяется. Всё прочее
    // (панели, апплеты, разделы) пересобирает оболочку заново.
    let old = ctx.config.get_untracked();
    // Сменились только настройки поведения, которые читаются на лету (автояркость: кнопка «А» в шторке,
    // сохранение поправки), — ни пересборки, ни темы: иначе оболочка перемаргивает на каждое нажатие.
    let behavior_only = {
        let mut probe = cfg.clone();
        probe.brightness = old.brightness.clone();
        probe == *old
    };
    if behavior_only {
        ctx.config.set_always(Arc::new(cfg));
        RELOAD_HOOKS.with(|h| {
            for f in h.borrow().iter() {
                f(ctx, false);
            }
        });
        return;
    }
    let structural = {
        let mut probe = cfg.clone();
        probe.appearance = old.appearance.clone();
        probe.wallpaper = old.wallpaper.clone();
        probe.animations = old.animations.clone();
        probe.haptics = old.haptics.clone();
        // Масштаб интерфейса меняет размеры панелей — пересобрать.
        probe != *old || cfg.appearance.ui_scale != old.appearance.ui_scale
    };
    syngui_layer::set_ui_zoom(cfg.appearance.ui_scale);
    let theme_ms = cfg.animations.theme_ms();
    if theme_ms > 0 && !structural {
        syngui_layer::set_stylesheet_with_transition(theme::build(&cfg), theme_ms);
    } else {
        syngui_layer::set_stylesheet(theme::build(&cfg));
    }
    app_colors::sync(&cfg);
    ctx.dnd.set(cfg.notifications.do_not_disturb);
    synshell_common::haptics::set_config(&cfg.haptics);
    ctx.config.set_always(Arc::new(cfg));
    if structural {
        if !ctx.popup.get_untracked().is_some_and(|p| p.kind.survives_reload()) {
            ctx.close_popup();
        }
        ctx.generation.set(ctx.generation.get_untracked() + 1);
    }
    RELOAD_HOOKS.with(|h| {
        for f in h.borrow().iter() {
            f(ctx, structural);
        }
    });
}
