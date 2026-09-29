//! Таблица стилей: переменные палитры из `[appearance]` (как у syndesktop-shell)
//! + встроенная мобильная тема + `~/.config/synshell/mobile.mss`.

use synshell_common::Config;

const BUILTIN: &str = include_str!("../styles/mobile.mss");

pub fn build(config: &Config) -> String {
    let a = &config.appearance;
    let mut out = a.mss_variables();
    out.push_str(&a.theme_mss_variables());
    out.push_str(BUILTIN);
    if !a.font.trim().is_empty() {
        out.push_str(&format!("Text, Button {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    let theme = a.theme_mss(false);
    if !theme.is_empty() {
        out.push_str(theme);
        out.push('\n');
    }
    if let Ok(user) = std::fs::read_to_string(synshell_common::paths::config_dir().join("mobile.mss")) {
        out.push_str(&user);
    }
    out
}
