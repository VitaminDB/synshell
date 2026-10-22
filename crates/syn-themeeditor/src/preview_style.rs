//! Стили предпросмотра: настоящие таблицы оболочки и «Параметров системы»
//! с переменными темы-черновика, перенесённые в области `.te-scope-shell` и
//! `.te-scope-settings` (см. [`crate::scope`]).

use std::sync::Arc;

use synshell_common::config::{ColorScheme, Rgba};
use synshell_common::theme::Theme;
use synshell_common::Config;

use crate::scope::scope_mss;

pub const SHELL_SCOPE: &str = ".te-scope-shell";
pub const SETTINGS_SCOPE: &str = ".te-scope-settings";

const SHELL_MSS: &str = include_str!("../../synshell-ui/styles/shell.mss");
const SETTINGS_MSS: &str = include_str!("../../synsettings/styles/settings.mss");

/// Конфиг, каким его увидят программы с этой темой: схема `dark`, тема как
/// есть (без своего акцента и цветов пользователя), рекомендуемая форма.
pub fn preview_config(base: &Config, theme: &Theme, dark: bool) -> Config {
    let mut cfg = base.clone();
    let a = &mut cfg.appearance;
    a.theme = theme.id.clone();
    a.color_scheme = if dark { ColorScheme::Dark } else { ColorScheme::Light };
    a.accent.clear();
    a.colors.clear();
    let num = |k: &str| theme.appearance.get(k).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)));
    if let Some(r) = num("corner_radius") {
        a.corner_radius = r as f32;
    }
    if let Some(o) = num("panel_opacity") {
        a.panel_opacity = o as f32;
    }
    if let Some(f) = theme.appearance.get("font").and_then(|v| v.as_str()) {
        a.font = f.to_string();
    }
    a.resolved = Some(Arc::new(theme.clone()));
    if let Some(r) = theme.decorations.get("corner_radius").and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))) {
        cfg.decorations.corner_radius = r as f32;
    }
    cfg
}

/// Таблица предпросмотра для окна редактора.
pub fn build(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let p = a.palette();
    let dark = a.is_dark();

    // Оболочка — как synshell-ui::theme::build.
    let mut shell = a.mss_variables();
    shell.push_str(&format!(
        ":root {{\n  --shadow: {};\n  --scrim: {};\n}}\n",
        if dark { "#00000080" } else { "#00000038" },
        p.bg.with_alpha(0.35).hex()
    ));
    shell.push_str(&a.theme_mss_variables());
    shell.push_str(SHELL_MSS);
    if !a.font.trim().is_empty() {
        shell.push_str(&format!("Text, Button, TextField {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    shell.push_str(a.theme_mss(false));
    shell.push('\n');
    shell.push_str(&format!(".wallpaper-color {{ background: {}; }}\n", a.wallpaper_background(&Default::default())));
    // Заголовки окон — цвета композитора.
    let (active, inactive) = a.titlebar_colors(&cfg.decorations);
    shell.push_str(&format!(
        ".te-win-title {{ background-color: {}; }}\n.te-win-title-inactive {{ background-color: {}; }}\n.te-win-title-text {{ color: {}; }}\n.te-win-title-inactive .te-win-title-text {{ color: {}; }}\n.te-win {{ border-radius: {}px; }}\n",
        active.hex(),
        inactive.hex(),
        title_fg(active).hex(),
        title_fg(inactive).fade(0.7).hex(),
        cfg.decorations.corner_radius.max(0.0),
    ));

    // «Параметры системы» — как synsettings::state::theme_mss.
    let sidebar = if dark { p.bg.mix(p.surface, 0.45) } else { p.surface_alt.mix(p.bg, 0.4) };
    let input = if dark { p.bg.mix(p.surface, 0.35) } else { p.surface };
    let mut settings = a.mss_variables();
    settings.push_str(&format!(
        ":root {{\n  --sidebar-bg: {};\n  --card-bg: {};\n  --input-bg: {};\n  --shadow: {};\n  --accent-hover: {};\n  --danger-soft: {};\n}}\n",
        sidebar.hex(),
        p.surface.hex(),
        input.hex(),
        if dark { "#00000066" } else { "#1a203014" },
        p.accent.mix(p.fg, 0.15).hex(),
        p.danger.with_alpha(0.16).hex(),
    ));
    settings.push_str(&a.theme_mss_variables());
    settings.push_str(SETTINGS_MSS);
    settings.push_str(a.theme_mss(true));
    settings.push('\n');

    let mut out = scope_mss(&shell, SHELL_SCOPE);
    out.push_str(&scope_mss(&settings, SETTINGS_SCOPE));
    out
}

/// Цвет текста заголовка окна (как у композитора: контрастный к фону).
fn title_fg(bg: Rgba) -> Rgba {
    if crate::color::luminance(bg) > 0.45 {
        Rgba::rgb(0x1d, 0x20, 0x26)
    } else {
        Rgba::rgb(0xe8, 0xea, 0xef)
    }
}
