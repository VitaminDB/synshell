//! Тема оболочки, слоями снизу вверх:
//! 1. переменные палитры из `[appearance]` (с учётом темы);
//! 2. `--shadow`/`--scrim` и свои переменные темы;
//! 3. встроенный `styles/shell.mss`;
//! 4. `shell.mss` темы;
//! 5. фон рабочего стола (`[wallpaper]` или обои темы);
//! 6. пользовательский `~/.config/synshell/theme.mss`.

use synshell_common::{paths, Config};

const BASE: &str = include_str!("../styles/shell.mss");

/// Слой оболочки поверх общего: встроенные стили и пользовательский файл.
static LAYER: std::sync::OnceLock<(&'static str, Option<&'static str>)> = std::sync::OnceLock::new();

/// Стили конкретной оболочки (`mobile.mss`) и её пользовательский файл в
/// `~/.config/synshell/`.
pub fn set_layer(extra: &'static str, user_file: Option<&'static str>) {
    let _ = LAYER.set((extra, user_file));
}

fn layer() -> (&'static str, Option<&'static str>) {
    LAYER.get().copied().unwrap_or(("", None))
}

pub fn build(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let mut out = a.mss_variables();
    let pal = a.palette();
    out.push_str(&format!(
        ":root {{\n  --shadow: {};\n  --scrim: {};\n}}\n",
        if a.is_dark() { "#00000080" } else { "#00000038" },
        pal.bg.with_alpha(0.35).hex(),
    ));
    out.push_str(&a.theme_mss_variables());
    out.push_str(BASE);
    out.push_str(layer().0);
    // Общий шрифт — на всё, что рисует текст.
    if !a.font.trim().is_empty() {
        out.push_str(&format!("Text, Button, TextField {{\n  font-family: \"{}\";\n}}\n", a.font.trim()));
    }
    let theme = a.theme_mss(false);
    if !theme.is_empty() {
        out.push_str(&format!("\n/* тема {} */\n", a.theme));
        out.push_str(theme);
        out.push('\n');
    }
    out.push_str(&format!(".wallpaper-color {{ background: {}; }}\n", a.wallpaper_background(&cfg.wallpaper)));
    if let Ok(user) = std::fs::read_to_string(paths::user_theme_file()) {
        out.push_str("\n/* ~/.config/synshell/theme.mss */\n");
        out.push_str(&user);
    }
    if let Some(f) = layer().1 {
        if let Ok(user) = std::fs::read_to_string(paths::config_dir().join(f)) {
            out.push_str(&format!("\n/* ~/.config/synshell/{f} */\n"));
            out.push_str(&user);
        }
    }
    out
}

/// Файлы, при изменении которых тема пересобирается.
pub fn watched_files(cfg: &Config) -> Vec<std::path::PathBuf> {
    let mut v = vec![paths::config_file(), paths::user_theme_file()];
    if let Some(f) = layer().1 {
        v.push(paths::config_dir().join(f));
    }
    v.extend(cfg.appearance.theme_files());
    v
}
