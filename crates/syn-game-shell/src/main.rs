//! syngame-shell — игровой режим synshell (gameshell): лаунчер игр на весь
//! экран в духе игровой консоли поверх общей `synshell-ui`.
//!
//! - Главный экран ([`home`]): вкладки (главная, игры, Steam, приложения,
//!   свои подборки), крупные обложки, афиша выбранной игры, строка
//!   состояния и подсказки кнопок.
//! - Библиотека ([`library`]): .desktop-игры, Steam ([`steam`]), свои ярлыки
//!   `[[gameshell.game]]`, избранное и последние запуски.
//! - Быстрое меню ([`overlay`]) поверх игры по Guide/Super: продолжить, на
//!   главную, закрыть игру, другие запущенные, громкость, выход из режима,
//!   питание.
//! - Управление: геймпад ([`gamepad`], evdev), клавиатура, касания и мышь.
//!
//! Окна в режиме — во весь экран без рамок (политика композитора synwm в
//! `[platform] form_factor = "game"`).

mod gamepad;
mod home;
mod library;
mod nav;
mod overlay;
mod steam;
mod tiles;

use synshell_common::config::FormFactor;
use synshell_ui::ShellCtx;

fn main() {
    synshell_ui::theme::set_config_mss(config_mss);
    let result = synshell_ui::run(synshell_ui::Shell {
        name: "syngame-shell",
        form_factor: FormFactor::Game,
        extra_mss: include_str!("../styles/game.mss"),
        user_mss: Some("game.mss"),
        install: Box::new(install),
    });
    if let Err(e) = result {
        log::error!("syngame-shell: {e:#}");
        std::process::exit(1);
    }
}

fn install(ctx: ShellCtx) {
    if ctx.cfg().gameshell.panels {
        synshell_ui::manager::install_with(ctx, false);
    }
    home::install(ctx);
    {
        use synshell_ui::sounds::{self, Ev};
        sounds::preload(&[Ev::Navigate, Ev::Bump, Ev::Select, Ev::Back, Ev::Tab, Ev::MenuOpen, Ev::MenuClose, Ev::Launch, Ev::Favorite, Ev::Click]);
        // Вход в игровой режим (оболочка запускается при переключении).
        syngui_layer::add_timer(std::time::Duration::from_millis(600), || {
            sounds::play(Ev::Startup);
            None
        });
    }
    synshell_ui::commands::set_extra(command);
    if ctx.cfg().gameshell.gamepad {
        gamepad::start(nav::gamepad);
    }
    // Экранная клавиатура и контроллер — там, где нет своих (телефон, планшет).
    if device_is_touch() {
        synshell_ui::daemons::keyboard();
        synshell_ui::daemons::gamepad();
        synshell_ui::input_mode::start(ctx);
    }
}

/// Стили из `[gameshell]`: акцент и цвет фона.
fn config_mss(cfg: &synshell_common::Config) -> String {
    let g = &cfg.gameshell;
    let mut out = String::new();
    let color = |s: &str| s.trim().starts_with('#') && matches!(s.trim().len(), 4 | 7 | 9);
    if color(&g.accent) {
        out.push_str(&format!(":root {{ --accent: {}; }}\n", g.accent.trim()));
    }
    if color(&g.background_color) {
        out.push_str(&format!(":root {{ --gs-bg: {}; }}\n", g.background_color.trim()));
    }
    out
}

/// Устройство — телефон или планшет (по мнению композитора).
fn device_is_touch() -> bool {
    use synshell_common::ipc::{Client, Request, Response};
    matches!(Client::connect().and_then(|mut c| c.request(&Request::FormFactor)), Ok(Response::FormFactor { device, .. }) if device == "phone")
}

/// Команды композитора (`ShellCommand`), жестов и клавиш.
fn command(name: &str, arg: &str) -> bool {
    let _ = arg;
    match name {
        // Guide, Super, жест «домой».
        "home" | "launcher" | "menu" | "apps" | "guide" => nav::guide(),
        "recents" | "window-switcher" => overlay::open(overlay::Section::Running),
        "power-menu" | "power" => overlay::open(overlay::Section::Power),
        "quick-settings" | "notifications" | "shade" => overlay::open(overlay::Section::Main),
        "back" if overlay::is_open() => overlay::close(),
        "keyboard" => synshell_ui::actions::spawn("synkeyboard toggle"),
        "gamepad" => synshell_ui::actions::spawn("syngamepad fold"),
        _ => return false,
    }
    true
}
