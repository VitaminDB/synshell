//! syndesktop-shell — оболочка рабочего стола: обои и панели по мониторам
//! поверх общей оболочки `synshell-ui` (меню, уведомления, подсказки,
//! блокировка). Рисуется syngui на layer-shell поверхностях, с композитором
//! synwm общается по IPC, под другими композиторами работает без него.

fn main() {
    let result = synshell_ui::run(synshell_ui::Shell {
        name: "syndesktop-shell",
        form_factor: synshell_common::config::FormFactor::Desktop,
        extra_mss: "",
        user_mss: None,
        install: Box::new(synshell_ui::manager::install),
    });
    if let Err(e) = result {
        log::error!("оболочка завершилась с ошибкой: {e:#}");
        std::process::exit(1);
    }
}
