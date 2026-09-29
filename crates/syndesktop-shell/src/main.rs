//! syndesktop-shell — оболочка рабочего стола: обои, панели с апплетами,
//! меню запуска, уведомления, экранные подсказки. Рисуется syngui на
//! layer-shell поверхностях (крейт syngui-layer), с композитором synshell
//! общается по IPC, а под другими композиторами работает без него.

mod actions;
mod anim;
mod app_colors;
mod appmenu;
mod applets;
mod clock;
mod commands;
mod ctx;
mod dock;
mod edit;
mod gtkmenu;
mod ipc;
mod launcher;
mod launchers;
mod lock;
mod manager;
mod notifications;
mod osd;
mod panel;
mod popup;
mod switcher;
mod system;
mod theme;
mod tray;
mod ui;
mod xdg;

use std::sync::Arc;
use std::time::Duration;
use synshell_common::watch::FileWatcher;
use synshell_common::Config;
use syngui::prelude::*;

use ctx::ShellCtx;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn,usvg=error"))
        .format_timestamp_millis()
        .init();

    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("syndesktop-shell {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if let Err(e) = Config::ensure_file_exists() {
        log::warn!("не удалось создать конфиг: {e}");
    }
    let (config, err) = Config::load();
    if let Some(e) = err {
        log::error!("ошибка в конфиге, взяты значения по умолчанию: {e}");
    }
    xdg::set_icon_theme(&config.appearance.icon_theme);
    xdg::warm_up();
    let mss = theme::build(&config);
    app_colors::sync(&config);
    let font = Some(config.appearance.font.trim().to_string()).filter(|f| !f.is_empty());

    let result = syngui_layer::run(syngui_layer::RunOptions { font_family: font }, &mss, move || {
        let ctx = ShellCtx::new(config);
        provide_context(ctx);
        clock::start(ctx);
        system::start(ctx);
        ipc::start(ctx);
        appmenu::install(ctx);
        notifications::start(ctx);
        manager::install(ctx);
        popup::install(ctx);
        osd::install(ctx);
        notifications::install(ctx);
        switcher::install(ctx);
        applets::taskbar::start_minimize_rects();
        watch_config();
        // Отладка: выполнить команды оболочки после старта
        // (`SYNSHELL_SHELL_EXEC="launcher;volume +5"`).
        if let Ok(cmds) = std::env::var("SYNSHELL_SHELL_EXEC") {
            // `sleep МС` между командами — задержка.
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
    });
    if let Err(e) = result {
        log::error!("оболочка завершилась с ошибкой: {e:#}");
        std::process::exit(1);
    }
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

thread_local! {
    /// mtime config.toml, прочитанного последним `reload_config`.
    static LOADED_MTIME: std::cell::Cell<Option<std::time::SystemTime>> = const { std::cell::Cell::new(None) };
    /// Последнюю правку config.toml сделала сама оболочка.
    static SELF_WRITTEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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
    let (cfg, err) = Config::load();
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
    let structural = {
        let mut probe = cfg.clone();
        probe.appearance = old.appearance.clone();
        probe.wallpaper = old.wallpaper.clone();
        probe.animations = old.animations.clone();
        probe != *old
    };
    let theme_ms = cfg.animations.theme_ms();
    if theme_ms > 0 && !structural {
        syngui_layer::set_stylesheet_with_transition(theme::build(&cfg), theme_ms);
    } else {
        syngui_layer::set_stylesheet(theme::build(&cfg));
    }
    app_colors::sync(&cfg);
    ctx.dnd.set(cfg.notifications.do_not_disturb);
    ctx.config.set_always(Arc::new(cfg));
    if structural {
        if !ctx.popup.get_untracked().is_some_and(|p| p.kind.survives_reload()) {
            ctx.close_popup();
        }
        ctx.generation.set(ctx.generation.get_untracked() + 1);
    }
}
