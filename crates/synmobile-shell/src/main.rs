//! synmobile-shell — оболочка synshell для телефона: строка состояния сверху,
//! домашний экран (сетка приложений) и панель навигации снизу. Рисуется
//! syngui на layer-shell поверхностях (syngui-layer), с композитором synwm
//! общается по IPC (`synshell_common::ipc`).
//!
//! Скелет: те же сервисы, что у syndesktop-shell (конфиг, темы, .desktop),
//! но своя раскладка под палец. Экранная клавиатура — отдельный демон
//! `synkeyboard`, оболочка его запускает и переключает кнопкой. Шторка
//! уведомлений и экран блокировки — следующие шаги (см. docs/MOBILE.md).

mod home;
mod ipc;
mod keyboard;
mod navbar;
mod statusbar;
mod theme;

use synshell_common::Config;
use syngui::prelude::*;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn,usvg=error"))
        .format_timestamp_millis()
        .init();
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("synmobile-shell {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if let Err(e) = Config::ensure_file_exists() {
        log::warn!("не удалось создать конфиг: {e}");
    }
    let (config, err) = Config::load();
    if let Some(e) = err {
        log::error!("ошибка в конфиге, взяты значения по умолчанию: {e}");
    }
    synshell_common::xdg::set_icon_theme(&config.appearance.icon_theme);
    synshell_common::xdg::warm_up();
    let mss = theme::build(&config);
    let font = Some(config.appearance.font.trim().to_string()).filter(|f| !f.is_empty());

    let result = syngui_layer::run(syngui_layer::RunOptions { font_family: font }, &mss, move || {
        let ctx = Ctx::new(config);
        provide_context(ctx);
        statusbar::install(ctx);
        navbar::install(ctx);
        home::install(ctx);
        ipc::start(ctx);
        keyboard::start();
    });
    if let Err(e) = result {
        log::error!("synmobile-shell: {e:#}");
        std::process::exit(1);
    }
}

/// Общее состояние оболочки (копируется — внутри сигналы).
#[derive(Clone, Copy)]
pub struct Ctx {
    pub config: RwSignal<Config>,
    /// Домашний экран показан поверх окон.
    pub home_visible: RwSignal<bool>,
    /// Часы: минута, чтобы перерисовывать строку состояния раз в минуту.
    pub clock: RwSignal<String>,
    /// Батарея: проценты и «заряжается».
    pub battery: RwSignal<Option<(u32, bool)>>,
}

impl Ctx {
    fn new(config: Config) -> Self {
        Self {
            config: use_signal(config),
            home_visible: use_signal(true),
            clock: use_signal(String::new()),
            battery: use_signal(None),
        }
    }
    pub fn get() -> Self {
        use_context::<Ctx>()
    }
}

/// Действие композитору по IPC; ошибки только в лог — оболочка работает и
/// под другими композиторами.
pub fn action(a: synshell_common::Action) {
    if let Err(e) = synshell_common::ipc::send_action(a) {
        log::warn!("IPC: {e}");
    }
}

/// Запустить команду (`sh -c`), не дожидаясь.
pub fn spawn(cmd: &str) {
    let cmd = cmd.to_string();
    std::thread::spawn(move || {
        let _ = std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|mut c| std::thread::spawn(move || { let _ = c.wait(); }));
    });
}
