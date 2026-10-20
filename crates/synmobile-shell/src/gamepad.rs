//! Экранный контроллер — отдельный демон `syngamepad` (крейт syngamepad), его запускает и
//! перезапускает общая оболочка (`synshell_ui::daemons`); показывается по режиму ввода
//! приложения в фокусе (`synshell_ui::input_mode`).

pub fn start() {
    synshell_ui::daemons::gamepad();
}
