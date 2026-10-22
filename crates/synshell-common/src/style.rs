//! Таблица стилей программы — одинаковые слои у всех программ synshell:
//!
//! 1. переменные палитры ([`crate::tokens`]);
//! 2. свои производные переменные программы ([`AppStyle::vars`]);
//! 3. переменные темы (`shadow`, `scrim`, `[dark.vars]`) — перекрывают 1–2;
//! 4. встроенный MSS программы ([`AppStyle::base`]) и дополнительные куски;
//! 5. слой темы для программы (`apps/<id>.mss` или `shell.mss`, …);
//! 6. шрифт интерфейса из `[appearance] font`;
//! 7. пользовательский слой `~/.config/synshell/apps/<id>.mss`.
//!
//! ```ignore
//! let mss = AppStyle::new(cfg, "synpkg")
//!     .vars(&[("sidebar", sidebar.hex()), ("window-radius", px(r))])
//!     .base(include_str!("../styles/synpkg.mss"))
//!     .build();
//! ```

use crate::{paths, Config};

pub struct AppStyle<'a> {
    cfg: &'a Config,
    app: &'a str,
    out: String,
    theme_vars: bool,
    font_selector: &'a str,
}

/// Значение в px для [`AppStyle::vars`].
pub fn px(v: f32) -> String {
    format!("{v}px")
}

/// Пользовательский слой программы: `~/.config/synshell/apps/<app>.mss`.
pub fn user_layer_file(app: &str) -> std::path::PathBuf {
    paths::config_dir().join("apps").join(format!("{app}.mss"))
}

impl<'a> AppStyle<'a> {
    /// `app` — имя слоя программы в теме и в `~/.config/synshell/apps/`.
    pub fn new(cfg: &'a Config, app: &'a str) -> Self {
        Self { cfg, app, out: cfg.appearance.mss_variables(), theme_vars: false, font_selector: "Text, Button, TextField" }
    }

    /// Свои производные переменные программы (`:root`); тема может их
    /// перекрыть в `[dark.vars]`/`[light.vars]`.
    pub fn vars(mut self, vars: &[(&str, String)]) -> Self {
        debug_assert!(!self.theme_vars, "vars() — до base()");
        if vars.is_empty() {
            return self;
        }
        self.out.push_str(":root {\n");
        for (k, v) in vars {
            self.out.push_str(&format!("  --{}: {v};\n", k.trim_start_matches('-')));
        }
        self.out.push_str("}\n");
        self
    }

    fn theme_vars(&mut self) {
        if !self.theme_vars {
            self.theme_vars = true;
            self.out.push_str(&self.cfg.appearance.theme_mss_variables());
        }
    }

    /// Встроенные стили программы.
    pub fn base(mut self, mss: &str) -> Self {
        self.theme_vars();
        self.out.push_str(mss);
        self.out.push('\n');
        self
    }

    /// Дополнительный кусок MSS после встроенных стилей (до слоя темы).
    pub fn push(mut self, mss: &str) -> Self {
        self.theme_vars();
        self.out.push_str(mss);
        self
    }

    /// Каким виджетам ставить шрифт интерфейса (по умолчанию
    /// `Text, Button, TextField`).
    pub fn font_selector(mut self, sel: &'a str) -> Self {
        self.font_selector = sel;
        self
    }

    pub fn build(mut self) -> String {
        self.theme_vars();
        let a = &self.cfg.appearance;
        let theme = a.theme_app_mss(self.app);
        if !theme.is_empty() {
            self.out.push_str(&format!("\n/* тема {}: {} */\n", a.theme, crate::theme::file_of_layer(self.app)));
            self.out.push_str(theme);
            self.out.push('\n');
        }
        if !a.font.trim().is_empty() && !self.font_selector.is_empty() {
            self.out.push_str(&format!("{} {{ font-family: \"{}\"; }}\n", self.font_selector, a.font.trim()));
        }
        if let Ok(user) = std::fs::read_to_string(user_layer_file(self.app)) {
            self.out.push_str(&format!("\n/* ~/.config/synshell/apps/{}.mss */\n", self.app));
            self.out.push_str(&user);
        }
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_order() {
        let mut cfg = Config::default();
        cfg.appearance.theme = "tokyo-night".into();
        cfg.appearance.load_theme();
        cfg.appearance.font = "Inter".into();
        let s = AppStyle::new(&cfg, "settings").vars(&[("sidebar", "#123456".into())]).base(".base { }").build();
        let i_root = s.find("--bg:").unwrap();
        let i_app = s.find("--sidebar: #123456").unwrap();
        let i_theme_vars = s.find("--magenta:").unwrap();
        let i_base = s.find(".base").unwrap();
        let i_layer = s.find(".nav-item.active").unwrap();
        let i_font = s.find("font-family: \"Inter\"").unwrap();
        assert!(i_root < i_app && i_app < i_theme_vars && i_theme_vars < i_base && i_base < i_layer && i_layer < i_font);
    }
}
