//! syndesktop-shell — оболочка рабочего стола: обои, панели с апплетами,
//! меню запуска, уведомления, экранные подсказки. Рисуется syngui на
//! layer-shell поверхностях (крейт syngui-layer), с композитором syndesktop
//! общается по IPC, а под другими композиторами работает без него.

mod actions;
mod applets;
mod clock;
mod commands;
mod ctx;
mod ipc;
mod launcher;
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
use syndesktop_common::watch::FileWatcher;
use syndesktop_common::{paths, Config};
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
    let font = Some(config.appearance.font.trim().to_string()).filter(|f| !f.is_empty());

    let result = syngui_layer::run(syngui_layer::RunOptions { font_family: font }, &mss, move || {
        let ctx = ShellCtx::new(config);
        provide_context(ctx);
        clock::start(ctx);
        system::start(ctx);
        ipc::start(ctx);
        notifications::start(ctx);
        manager::install(ctx);
        popup::install(ctx);
        osd::install(ctx);
        notifications::install(ctx);
        watch_config();
        // Отладка: выполнить команды оболочки после старта
        // (`SYNDESKTOP_SHELL_EXEC="launcher;volume +5"`).
        if let Ok(cmds) = std::env::var("SYNDESKTOP_SHELL_EXEC") {
            syngui_layer::add_timer(Duration::from_millis(400), move || {
                for c in cmds.split(';').filter(|c| !c.trim().is_empty()) {
                    commands::handle(c.trim());
                }
                None
            });
        }
    });
    if let Err(e) = result {
        log::error!("оболочка завершилась с ошибкой: {e:#}");
        std::process::exit(1);
    }
}

/// Раз в секунду смотреть на config.toml и theme.mss.
fn watch_config() {
    let mut w = FileWatcher::new([paths::config_file(), paths::user_theme_file()]);
    syngui_layer::add_timer(Duration::from_secs(1), move || {
        if w.poll() {
            reload_config();
        }
        Some(Duration::from_secs(1))
    });
}

/// Перечитать конфиг и тему, пересобрать поверхности.
pub fn reload_config() {
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
    syngui_layer::set_stylesheet(theme::build(&cfg));
    ctx.dnd.set(cfg.notifications.do_not_disturb);
    ctx.config.set_always(Arc::new(cfg));
    ctx.close_popup();
    ctx.generation.set(ctx.generation.get_untracked() + 1);
}
