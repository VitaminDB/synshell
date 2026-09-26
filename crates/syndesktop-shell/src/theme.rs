//! Тема оболочки: переменные палитры из `[appearance]` + встроенный MSS +
//! пользовательский `~/.config/syndesktop/theme.mss` поверх.

use syndesktop_common::{paths, Config};

const BASE: &str = include_str!("../styles/shell.mss");

pub fn build(cfg: &Config) -> String {
    let mut out = cfg.appearance.mss_variables();
    let pal = cfg.appearance.palette();
    let font = if cfg.appearance.font.trim().is_empty() {
        String::new()
    } else {
        format!("  font-family: \"{}\";\n", cfg.appearance.font.trim())
    };
    out.push_str(&format!(
        ":root {{\n  --shadow: {};\n  --scrim: {};\n}}\n",
        if cfg.appearance.color_scheme == syndesktop_common::config::ColorScheme::Dark {
            "#00000080"
        } else {
            "#00000038"
        },
        pal.bg.with_alpha(0.35).hex(),
    ));
    out.push_str(BASE);
    // Общий шрифт — на всё, что рисует текст.
    if !font.is_empty() {
        out.push_str(&format!("Text, Button, TextField {{\n{font}}}\n"));
    }
    // Градиент обоев без картинки.
    let w = &cfg.wallpaper;
    let c1 = if w.color.is_empty() { "#1b2233" } else { w.color.as_str() };
    let c2 = if w.color2.is_empty() { c1 } else { w.color2.as_str() };
    out.push_str(&format!(
        ".wallpaper-color {{ background: linear-gradient(to bottom, {c1}, {c2}); }}\n"
    ));
    if let Ok(user) = std::fs::read_to_string(paths::user_theme_file()) {
        out.push_str("\n/* ~/.config/syndesktop/theme.mss */\n");
        out.push_str(&user);
    }
    out
}
