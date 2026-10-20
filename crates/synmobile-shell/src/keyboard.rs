//! Экранная клавиатура — отдельный демон `synkeyboard` (крейт synkeyboard),
//! его запускает и перезапускает общая оболочка (`synshell_ui::daemons`);
//! показывается сама по text-input окна или командой `shell keyboard`.

pub fn start() {
    synshell_ui::daemons::keyboard();
}

pub fn toggle() {
    synshell_ui::actions::spawn("synkeyboard toggle");
}
