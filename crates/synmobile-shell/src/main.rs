//! synmobile-shell — оболочка synshell для телефона поверх общей
//! `synshell-ui`: те же панели и доки (добавляются удержанием на рабочем
//! столе, встроенных строки состояния и навигации нет), меню, уведомления,
//! блокировка; свои — домашний экран со страницами и команды жестов
//! композитора (`home`, `shade`, `recents`, `back`).

mod home;
mod keyboard;

use synshell_common::config::FormFactor;
use synshell_ui::ShellCtx;

fn main() {
    let result = synshell_ui::run(synshell_ui::Shell {
        name: "synmobile-shell",
        form_factor: FormFactor::Phone,
        extra_mss: include_str!("../styles/mobile.mss"),
        user_mss: Some("mobile.mss"),
        install: Box::new(install),
    });
    if let Err(e) = result {
        log::error!("synmobile-shell: {e:#}");
        std::process::exit(1);
    }
}

fn install(ctx: ShellCtx) {
    // Панели и доки телефона; обои рисует домашний экран.
    synshell_ui::manager::install_with(ctx, false);
    home::install(ctx);
    synshell_ui::commands::set_extra(command);
    keyboard::start();
}

/// Команды жестов и кнопок (приходят от композитора `ShellCommand`).
fn command(name: &str, arg: &str) -> bool {
    let ctx = ShellCtx::get();
    match name {
        // Домой: закрыть оверлеи, свернуть окна, показать сетку приложений.
        "home" => {
            ctx.close_popup();
            synshell_ui::applets::show_desktop_now(&ctx);
            home::show_apps();
        }
        "apps" => synshell_ui::commands::handle("launcher"),
        "keyboard" => keyboard::toggle(),
        "mode" => {
            let label = match arg.trim() {
                "pages" => "Страницы",
                "tiles" => "Плитки",
                "free" => "Свободный стол",
                other => other,
            };
            synshell_ui::osd::show(ctx, synshell_ui::ui::mi::WINDOW, None, format!("Режим окон: {label}"));
        }
        _ => return false,
    }
    true
}
