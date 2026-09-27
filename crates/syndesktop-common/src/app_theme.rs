//! Цвета программ под тему syndesktop (`[appearance] app_colors`).
//!
//! Из итоговой палитры (тема, схема, акцент, `[appearance.colors]`)
//! генерируются:
//! - `~/.config/gtk-{3,4}.0/colors.css` — цвета темы GTK Breeze (`*_breeze`,
//!   те же, что пишет kde-gtk-config), `gtk.css` импортирует их и получает
//!   блок с именованными цветами libadwaita/adw-gtk3 (`window_bg_color`, …);
//! - `settings.ini` — `gtk-application-prefer-dark-theme`;
//! - `~/.config/kdeglobals` — группы `[Colors:*]` и `[WM]` для Qt/KDE (и для
//!   kded6, который по ним пересобирает `colors.css`), плюс схема
//!   `~/.local/share/color-schemes/Syndesktop.colors`;
//! - `~/.config/GIMP/<версия>/gimp.css` — цвета стандартной темы GIMP 3.
//!
//! Файлы переписываются, только если содержимое изменилось; чужие строки
//! `gtk.css`, `gimp.css` и `kdeglobals` сохраняются. Перед первой правкой
//! `kdeglobals` рядом кладётся копия `kdeglobals.syndesktop-backup`.

use std::path::{Path, PathBuf};

use crate::config::{Config, Rgba};
use crate::paths;

/// Имя цветовой схемы KDE.
pub const KDE_SCHEME: &str = "Syndesktop";

const BEGIN: &str = "/* syndesktop: начало — генерируется по теме, правки затрутся */";
const END: &str = "/* syndesktop: конец */";

const BLACK: Rgba = Rgba::rgb(0, 0, 0);
const WHITE: Rgba = Rgba::rgb(0xff, 0xff, 0xff);

/// Роли цветов для программ — из палитры syndesktop.
#[derive(Debug, Clone, PartialEq)]
pub struct Colors {
    pub dark: bool,
    /// Фон окна (панели инструментов, диалоги).
    pub window: Rgba,
    /// Фон содержимого: списки, поля ввода, холсты.
    pub view: Rgba,
    pub button: Rgba,
    /// Меню, всплывающие окна.
    pub popover: Rgba,
    pub tooltip: Rgba,
    /// Заголовок активного/неактивного окна.
    pub header: Rgba,
    pub header_inactive: Rgba,
    pub fg: Rgba,
    pub muted: Rgba,
    pub border: Rgba,
    pub accent: Rgba,
    pub accent_fg: Rgba,
    pub danger: Rgba,
    pub success: Rgba,
    pub warning: Rgba,
}

impl Colors {
    pub fn from_config(cfg: &Config) -> Self {
        let a = &cfg.appearance;
        let p = a.palette();
        let dark = a.is_dark();
        let (header, header_inactive) = a.titlebar_colors(&cfg.decorations);
        // В тёмной схеме содержимое темнее окна, в светлой — светлее.
        let (window, view) = if dark { (p.surface, p.bg) } else { (p.bg, p.surface) };
        Colors {
            dark,
            window,
            view,
            button: if dark { p.surface_alt } else { p.surface },
            popover: p.surface,
            tooltip: p.surface_alt,
            header,
            header_inactive,
            fg: p.fg,
            muted: p.muted,
            border: p.border.mix(p.fg, 0.12),
            accent: p.accent,
            accent_fg: p.accent_fg,
            danger: p.danger,
            success: p.success,
            warning: p.warning,
        }
    }

    /// Сдвинуть к «крайнему» цвету схемы: к чёрному в тёмной, к белому в
    /// светлой.
    fn deeper(&self, c: Rgba, t: f32) -> Rgba {
        c.mix(if self.dark { BLACK } else { WHITE }, t)
    }

    fn insensitive(&self, fg: Rgba, bg: Rgba) -> Rgba {
        fg.mix(bg, 0.55)
    }
}

fn hex(c: Rgba) -> String {
    c.with_alpha(1.0).hex()
}

/// Цвет для CSS GTK: GTK 3 не понимает `#rrggbbaa`.
fn css(c: Rgba) -> String {
    if c.a == 255 {
        c.hex()
    } else {
        format!("rgba({},{},{},{:.2})", c.r, c.g, c.b, c.a as f32 / 255.0)
    }
}

// ─── GTK ────────────────────────────────────────────────────────────────────

/// `colors.css` для GTK-темы Breeze (имена как у kde-gtk-config).
pub fn gtk_breeze_css(c: &Colors) -> String {
    let ins = |fg: Rgba, bg: Rgba| c.insensitive(fg, bg);
    let sel_unfocused = c.accent.mix(c.window, 0.5);
    let ins_border = c.border.mix(c.window, 0.5);
    let pairs: Vec<(&str, Rgba)> = vec![
        ("borders", c.border),
        ("content_view_bg", c.view),
        ("error_color_backdrop", c.danger),
        ("error_color", c.danger),
        ("error_color_insensitive_backdrop", ins(c.danger, c.window)),
        ("error_color_insensitive", ins(c.danger, c.window)),
        ("insensitive_base_color", c.view),
        ("insensitive_base_fg_color", ins(c.fg, c.view)),
        ("insensitive_bg_color", c.window),
        ("insensitive_borders", ins_border),
        ("insensitive_fg_color", ins(c.fg, c.window)),
        ("insensitive_selected_bg_color", c.window),
        ("insensitive_selected_fg_color", ins(c.fg, c.window)),
        ("insensitive_unfocused_bg_color", c.window),
        ("insensitive_unfocused_fg_color", ins(c.fg, c.window)),
        ("insensitive_unfocused_selected_bg_color", c.window),
        ("insensitive_unfocused_selected_fg_color", ins(c.fg, c.window)),
        ("link_color", c.accent),
        ("link_visited_color", c.accent.mix(c.muted, 0.5)),
        ("success_color_backdrop", c.success),
        ("success_color", c.success),
        ("success_color_insensitive_backdrop", ins(c.success, c.window)),
        ("success_color_insensitive", ins(c.success, c.window)),
        ("theme_base_color", c.view),
        ("theme_bg_color", c.window),
        ("theme_button_background_backdrop", c.button),
        ("theme_button_background_backdrop_insensitive", c.button.mix(c.window, 0.5)),
        ("theme_button_background_insensitive", c.button.mix(c.window, 0.5)),
        ("theme_button_background_normal", c.button),
        ("theme_button_decoration_focus_backdrop", c.accent),
        ("theme_button_decoration_focus_backdrop_insensitive", ins(c.accent, c.window)),
        ("theme_button_decoration_focus", c.accent),
        ("theme_button_decoration_focus_insensitive", ins(c.accent, c.window)),
        ("theme_button_decoration_hover_backdrop", c.accent),
        ("theme_button_decoration_hover_backdrop_insensitive", ins(c.accent, c.window)),
        ("theme_button_decoration_hover", c.accent),
        ("theme_button_decoration_hover_insensitive", ins(c.accent, c.window)),
        ("theme_button_foreground_active_backdrop", c.accent_fg),
        ("theme_button_foreground_active_backdrop_insensitive", ins(c.fg, c.button)),
        ("theme_button_foreground_active", c.accent_fg),
        ("theme_button_foreground_active_insensitive", ins(c.fg, c.button)),
        ("theme_button_foreground_backdrop", c.fg),
        ("theme_button_foreground_backdrop_insensitive", ins(c.fg, c.button)),
        ("theme_button_foreground_insensitive", ins(c.fg, c.button)),
        ("theme_button_foreground_normal", c.fg),
        ("theme_fg_color", c.fg),
        ("theme_header_background_backdrop", c.header_inactive),
        ("theme_header_background", c.header),
        ("theme_header_background_light", c.header_inactive),
        ("theme_header_foreground_backdrop", c.muted),
        ("theme_header_foreground", c.fg),
        ("theme_header_foreground_insensitive_backdrop", ins(c.fg, c.header_inactive)),
        ("theme_header_foreground_insensitive", ins(c.fg, c.header)),
        ("theme_hovering_selected_bg_color", c.accent.mix(c.fg, 0.2)),
        ("theme_selected_bg_color", c.accent),
        ("theme_selected_fg_color", c.accent_fg),
        ("theme_text_color", c.fg),
        ("theme_titlebar_background_backdrop", c.header_inactive),
        ("theme_titlebar_background", c.header),
        ("theme_titlebar_background_light", c.header_inactive),
        ("theme_titlebar_foreground_backdrop", c.muted),
        ("theme_titlebar_foreground", c.fg),
        ("theme_titlebar_foreground_insensitive_backdrop", ins(c.fg, c.header_inactive)),
        ("theme_titlebar_foreground_insensitive", ins(c.fg, c.header)),
        ("theme_unfocused_base_color", c.view),
        ("theme_unfocused_bg_color", c.window),
        ("theme_unfocused_fg_color", c.fg),
        ("theme_unfocused_selected_bg_color_alt", sel_unfocused),
        ("theme_unfocused_selected_bg_color", sel_unfocused),
        ("theme_unfocused_selected_fg_color", c.accent_fg),
        ("theme_unfocused_text_color", c.fg),
        ("theme_unfocused_view_bg_color", c.view),
        ("theme_unfocused_view_text_color", ins(c.fg, c.view)),
        ("theme_view_active_decoration_color", c.accent),
        ("theme_view_hover_decoration_color", c.accent),
        ("tooltip_background", c.tooltip),
        ("tooltip_border", c.border),
        ("tooltip_text", c.fg),
        ("unfocused_borders", c.border),
        ("unfocused_insensitive_borders", ins_border),
        ("warning_color_backdrop", c.warning),
        ("warning_color", c.warning),
        ("warning_color_insensitive_backdrop", ins(c.warning, c.window)),
        ("warning_color_insensitive", ins(c.warning, c.window)),
    ];
    let mut out = String::new();
    for (k, v) in pairs {
        out.push_str(&format!("@define-color {k}_breeze {};\n", hex(v)));
    }
    out
}

/// Именованные цвета libadwaita (GTK 4) и adw-gtk3 (GTK 3) — блок в `gtk.css`.
pub fn gtk_adwaita_css(c: &Colors, gtk4: bool) -> String {
    let pairs: Vec<(&str, String)> = vec![
        ("accent_color", hex(c.accent)),
        ("accent_bg_color", hex(c.accent)),
        ("accent_fg_color", hex(c.accent_fg)),
        ("destructive_color", hex(c.danger)),
        ("destructive_bg_color", hex(c.danger)),
        ("destructive_fg_color", hex(c.danger.contrast_fg())),
        ("success_color", hex(c.success)),
        ("success_bg_color", hex(c.success)),
        ("success_fg_color", hex(c.success.contrast_fg())),
        ("warning_color", hex(c.warning)),
        ("warning_bg_color", hex(c.warning)),
        ("warning_fg_color", hex(c.warning.contrast_fg())),
        ("error_color", hex(c.danger)),
        ("error_bg_color", hex(c.danger)),
        ("error_fg_color", hex(c.danger.contrast_fg())),
        ("window_bg_color", hex(c.window)),
        ("window_fg_color", hex(c.fg)),
        ("view_bg_color", hex(c.view)),
        ("view_fg_color", hex(c.fg)),
        ("headerbar_bg_color", hex(c.header)),
        ("headerbar_fg_color", hex(c.fg)),
        ("headerbar_border_color", hex(c.fg)),
        ("headerbar_backdrop_color", hex(c.header_inactive)),
        ("headerbar_shade_color", css(c.border.with_alpha(0.6))),
        ("headerbar_darker_shade_color", c.border.hex()),
        ("sidebar_bg_color", hex(c.window)),
        ("sidebar_fg_color", hex(c.fg)),
        ("sidebar_backdrop_color", hex(c.window)),
        ("sidebar_shade_color", css(c.border.with_alpha(0.6))),
        ("sidebar_border_color", css(c.border.with_alpha(0.6))),
        ("secondary_sidebar_bg_color", hex(c.window)),
        ("secondary_sidebar_fg_color", hex(c.fg)),
        ("secondary_sidebar_backdrop_color", hex(c.window)),
        ("secondary_sidebar_shade_color", css(c.border.with_alpha(0.6))),
        ("secondary_sidebar_border_color", css(c.border.with_alpha(0.6))),
        ("card_bg_color", hex(c.popover.mix(c.fg, 0.04))),
        ("card_fg_color", hex(c.fg)),
        ("card_shade_color", css(c.border.with_alpha(0.6))),
        ("dialog_bg_color", hex(c.popover)),
        ("dialog_fg_color", hex(c.fg)),
        ("popover_bg_color", hex(c.popover)),
        ("popover_fg_color", hex(c.fg)),
        ("popover_shade_color", css(c.border.with_alpha(0.6))),
        ("thumbnail_bg_color", hex(c.popover)),
        ("thumbnail_fg_color", hex(c.fg)),
        ("shade_color", css(BLACK.with_alpha(if c.dark { 0.36 } else { 0.07 }))),
        ("scrollbar_outline_color", css(BLACK.with_alpha(if c.dark { 0.5 } else { 0.0 }))),
        ("borders", hex(c.border)),
        // Стандартные имена GTK 3 — для тем, которые не знают *_breeze.
        ("theme_bg_color", hex(c.window)),
        ("theme_fg_color", hex(c.fg)),
        ("theme_base_color", hex(c.view)),
        ("theme_text_color", hex(c.fg)),
        ("theme_selected_bg_color", hex(c.accent)),
        ("theme_selected_fg_color", hex(c.accent_fg)),
    ];
    let mut out = String::new();
    for (k, v) in &pairs {
        out.push_str(&format!("@define-color {k} {v};\n"));
    }
    if gtk4 {
        // libadwaita 1.6+ берёт цвета из CSS-переменных.
        out.push_str(":root {\n");
        for (k, v) in &pairs {
            if !k.starts_with("theme_") && *k != "borders" {
                out.push_str(&format!("  --{}: {v};\n", k.replace('_', "-")));
            }
        }
        out.push_str("}\n");
    }
    out
}

// ─── KDE / Qt ───────────────────────────────────────────────────────────────

/// Группы цветовой схемы KDE: `(группа, [(ключ, цвет)])`.
pub fn kde_groups(c: &Colors) -> Vec<(String, Vec<(&'static str, Rgba)>)> {
    let visited = c.accent.mix(c.muted, 0.5);
    let set = |bg: Rgba, alt: Rgba, fg: Rgba, inactive: Rgba, active: Rgba, link: Rgba| -> Vec<(&'static str, Rgba)> {
        vec![
            ("BackgroundAlternate", alt),
            ("BackgroundNormal", bg),
            ("DecorationFocus", c.accent),
            ("DecorationHover", c.accent),
            ("ForegroundActive", active),
            ("ForegroundInactive", inactive),
            ("ForegroundLink", link),
            ("ForegroundNegative", c.danger),
            ("ForegroundNeutral", c.warning),
            ("ForegroundNormal", fg),
            ("ForegroundPositive", c.success),
            ("ForegroundVisited", visited),
        ]
    };
    let plain = |bg: Rgba, alt: Rgba| set(bg, alt, c.fg, c.muted, c.accent, c.accent);
    let complementary = if c.dark { c.view } else { c.fg };
    let comp_fg = complementary.contrast_fg();
    vec![
        ("Colors:Button".into(), plain(c.button, c.button.mix(c.fg, 0.05))),
        (
            "Colors:Complementary".into(),
            set(complementary, complementary.mix(comp_fg, 0.05), comp_fg, comp_fg.mix(complementary, 0.4), c.accent, c.accent),
        ),
        ("Colors:Header".into(), plain(c.header, c.header_inactive)),
        ("Colors:Header][Inactive".into(), plain(c.header_inactive, c.header)),
        (
            "Colors:Selection".into(),
            set(c.accent, c.deeper(c.accent, 0.25), c.accent_fg, c.accent_fg.mix(c.accent, 0.3), c.accent_fg, c.accent_fg),
        ),
        ("Colors:Tooltip".into(), plain(c.tooltip, c.tooltip.mix(c.fg, 0.05))),
        ("Colors:View".into(), plain(c.view, c.view.mix(c.fg, 0.04))),
        ("Colors:Window".into(), plain(c.window, c.window.mix(c.fg, 0.05))),
    ]
}

fn kde_rgb(c: Rgba) -> String {
    format!("{},{},{}", c.r, c.g, c.b)
}

fn kde_wm(c: &Colors) -> Vec<(&'static str, Rgba)> {
    vec![
        ("activeBackground", c.header),
        ("activeBlend", c.fg),
        ("activeForeground", c.fg),
        ("inactiveBackground", c.header_inactive),
        ("inactiveBlend", c.muted),
        ("inactiveForeground", c.muted),
    ]
}

/// Секции INI схемы: группы цветов и `[WM]`.
fn kde_sections(c: &Colors) -> Vec<(String, Vec<(String, String)>)> {
    let mut v: Vec<(String, Vec<(String, String)>)> = kde_groups(c)
        .into_iter()
        .map(|(g, kv)| (g, kv.into_iter().map(|(k, c)| (k.to_string(), kde_rgb(c))).collect()))
        .collect();
    v.push(("WM".into(), kde_wm(c).into_iter().map(|(k, c)| (k.to_string(), kde_rgb(c))).collect()));
    v
}

/// Файл схемы `Syndesktop.colors`.
pub fn kde_color_scheme(c: &Colors) -> String {
    let mut out = format!("[General]\nColorScheme={KDE_SCHEME}\nName={KDE_SCHEME}\n\n");
    for (g, kv) in kde_sections(c) {
        out.push_str(&format!("[{g}]\n"));
        for (k, v) in kv {
            out.push_str(&format!("{k}={v}\n"));
        }
        out.push('\n');
    }
    out
}

/// Разбор INI на секции: `(заголовок без скобок, строки)`; строки до
/// первой секции — под пустым заголовком.
fn ini_sections(text: &str) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = vec![(String::new(), Vec::new())];
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            out.push((t[1..t.len() - 1].to_string(), Vec::new()));
        } else {
            out.last_mut().expect("есть первая секция").1.push(line.to_string());
        }
    }
    out
}

fn ini_join(sections: &[(String, Vec<String>)]) -> String {
    let mut out = String::new();
    for (i, (h, lines)) in sections.iter().enumerate() {
        if i > 0 || !h.is_empty() {
            out.push_str(&format!("[{h}]\n"));
        }
        // Пустые строки в конце секции — одна, для читаемости.
        let mut lines: Vec<&String> = lines.iter().collect();
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }
        for l in &lines {
            out.push_str(l);
            out.push('\n');
        }
        if (i > 0 || !h.is_empty() || !lines.is_empty()) && i + 1 < sections.len() {
            out.push('\n');
        }
    }
    out
}

/// Поставить `key=value` в секции `section` (создать секцию при нужде).
fn ini_set(sections: &mut Vec<(String, Vec<String>)>, section: &str, key: &str, value: &str) {
    let line = format!("{key}={value}");
    let Some(s) = sections.iter_mut().find(|s| s.0 == section) else {
        sections.push((section.to_string(), vec![line]));
        return;
    };
    let is_key = |l: &String| l.split_once('=').is_some_and(|(k, _)| k.trim() == key);
    match s.1.iter_mut().find(|l| is_key(l)) {
        Some(l) => *l = line,
        None => {
            // Перед хвостовыми пустыми строками.
            let at = s.1.iter().rposition(|l| !l.trim().is_empty()).map(|i| i + 1).unwrap_or(0);
            s.1.insert(at, line);
        }
    }
}

fn ini_remove(sections: &mut [(String, Vec<String>)], section: &str, key: &str) {
    if let Some(s) = sections.iter_mut().find(|s| s.0 == section) {
        s.1.retain(|l| l.split_once('=').is_none_or(|(k, _)| k.trim() != key));
    }
}

/// `kdeglobals` со схемой syndesktop: группы цветов заменены, остальное
/// как было.
pub fn kdeglobals_with(text: &str, c: &Colors) -> String {
    let mut sections = ini_sections(text);
    sections.retain(|(h, _)| !h.starts_with("Colors:"));
    ini_set(&mut sections, "General", "ColorScheme", KDE_SCHEME);
    ini_remove(&mut sections, "General", "ColorSchemeHash");
    // В [WM] есть и шрифты заголовков — меняются только цвета; группы
    // цветов целиком — в конец (после [WM], чтобы порядок не менялся).
    let (colors, other): (Vec<_>, Vec<_>) = kde_sections(c).into_iter().partition(|(g, _)| g.starts_with("Colors:"));
    for (g, kv) in other {
        for (k, v) in kv {
            ini_set(&mut sections, &g, &k, &v);
        }
    }
    for (g, kv) in colors {
        sections.push((g, kv.into_iter().map(|(k, v)| format!("{k}={v}")).collect()));
    }
    ini_join(&sections)
}

/// `settings.ini` GTK с нужной схемой (светлая/тёмная).
pub fn gtk_settings_ini_with(text: &str, dark: bool) -> String {
    let mut sections = ini_sections(text);
    ini_set(&mut sections, "Settings", "gtk-application-prefer-dark-theme", if dark { "true" } else { "false" });
    ini_join(&sections)
}

// ─── GIMP ───────────────────────────────────────────────────────────────────

/// Цвета стандартной темы GIMP 3 (`themes/Default/gimp-*.css`).
pub fn gimp_css(c: &Colors) -> String {
    let w = c.window;
    let f = c.fg;
    let pairs: Vec<(&str, String)> = if c.dark {
        vec![
            ("fg-color", hex(f)),
            ("bg-color", hex(w)),
            ("border-color", hex(c.deeper(w, 0.55))),
            ("dimmed-fg-color", hex(c.muted)),
            ("disabled-fg-color", hex(f.mix(w, 0.45))),
            ("disabled-button-color", hex(f.mix(w, 0.75))),
            ("hover-color", hex(w.mix(f, 0.18))),
            ("widget-bg-color", hex(c.view)),
            ("selected-color", hex(c.deeper(w, 0.45))),
            ("extreme-bg-color", hex(c.deeper(w, 0.35))),
            ("extreme-selected-color", hex(w.mix(f, 0.06))),
            ("strong-border-color", hex(c.deeper(w, 0.3))),
            ("stronger-border-color", hex(w.mix(f, 0.1))),
            ("edge-border-color", hex(c.deeper(w, 0.55))),
            ("scrollbar-slider-color", hex(w.mix(f, 0.5))),
            ("scrollbar-trough-color", hex(w)),
            ("ruler-color", css(c.deeper(w, 0.4).with_alpha(0.3))),
        ]
    } else {
        vec![
            ("fg-color", hex(f)),
            ("bg-color", hex(w)),
            ("border-color", hex(w.mix(f, 0.06))),
            ("dimmed-fg-color", hex(c.muted)),
            ("disabled-fg-color", hex(f.mix(w, 0.45))),
            ("disabled-button-color", hex(f.mix(w, 0.8))),
            ("hover-color", hex(c.deeper(w, 0.6))),
            ("widget-bg-color", hex(w.mix(f, 0.06))),
            ("selected-color", hex(c.deeper(w, 0.7))),
            ("extreme-bg-color", hex(c.deeper(w, 0.7))),
            ("extreme-selected-color", hex(w.mix(f, 0.06))),
            ("strong-border-color", hex(w.mix(f, 0.14))),
            ("stronger-border-color", hex(w.mix(f, 0.2))),
            ("edge-border-color", hex(w.mix(f, 0.2))),
            ("scrollbar-slider-color", hex(w.mix(f, 0.5))),
            ("scrollbar-trough-color", hex(c.deeper(w, 0.7))),
            ("ruler-color", css(w.mix(f, 0.06).with_alpha(0.3))),
        ]
    };
    let mut out = String::new();
    for (k, v) in pairs {
        out.push_str(&format!("@define-color {k} {v};\n"));
    }
    out
}

// ─── Запись ─────────────────────────────────────────────────────────────────

/// Вставить (заменить) блок syndesktop в CSS-файле; `import` — строка,
/// которая должна быть в начале файла (`@import 'colors.css';`).
pub fn css_with_block(text: &str, block: &str, import: Option<&str>) -> String {
    let mut rest = String::new();
    let mut inside = false;
    for line in text.lines() {
        if line.trim() == BEGIN {
            inside = true;
            continue;
        }
        if inside {
            if line.trim() == END {
                inside = false;
            }
            continue;
        }
        rest.push_str(line);
        rest.push('\n');
    }
    let mut out = String::new();
    if let Some(imp) = import {
        if !rest.lines().any(|l| l.trim() == imp) {
            out.push_str(imp);
            out.push('\n');
        }
    }
    out.push_str(&rest);
    let mut out = out.trim_end().to_string();
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(BEGIN);
    out.push('\n');
    out.push_str(block);
    out.push_str(END);
    out.push('\n');
    out
}

/// Записать, если содержимое другое. `true` — файл изменён.
fn write_if_changed(path: &Path, text: &str) -> bool {
    if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
        return false;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Через временный файл: GTK и kded читают эти файлы на лету.
    let tmp = path.with_extension("syndesktop-tmp");
    match std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, path)) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("{}: {e}", path.display());
            let _ = std::fs::remove_file(&tmp);
            false
        }
    }
}

/// Что поменялось при [`apply`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Changed {
    pub gtk: bool,
    pub kde: bool,
    pub gimp: bool,
}

impl Changed {
    pub fn any(self) -> bool {
        self.gtk || self.kde || self.gimp
    }
}

/// Записать цвета программ в каталоги пользователя (`$XDG_CONFIG_HOME`,
/// `$XDG_DATA_HOME`).
pub fn apply(cfg: &Config) -> Changed {
    apply_in(&paths::xdg_config_home(), &paths::data_home(), &Colors::from_config(cfg))
}

pub fn apply_in(config_home: &Path, data_home: &Path, c: &Colors) -> Changed {
    let mut ch = Changed::default();
    let read = |p: &PathBuf| std::fs::read_to_string(p).unwrap_or_default();
    let colors = gtk_breeze_css(c);
    for (dir, gtk4) in [("gtk-3.0", false), ("gtk-4.0", true)] {
        let d = config_home.join(dir);
        ch.gtk |= write_if_changed(&d.join("colors.css"), &colors);
        let css = d.join("gtk.css");
        ch.gtk |= write_if_changed(&css, &css_with_block(&read(&css), &gtk_adwaita_css(c, gtk4), Some("@import 'colors.css';")));
        let ini = d.join("settings.ini");
        ch.gtk |= write_if_changed(&ini, &gtk_settings_ini_with(&read(&ini), c.dark));
    }

    let kdeglobals = config_home.join("kdeglobals");
    let old = read(&kdeglobals);
    let backup = config_home.join("kdeglobals.syndesktop-backup");
    if !old.is_empty() && !backup.exists() && !old.contains(&format!("ColorScheme={KDE_SCHEME}")) {
        let _ = std::fs::write(&backup, &old);
    }
    ch.kde |= write_if_changed(&kdeglobals, &kdeglobals_with(&old, c));
    ch.kde |= write_if_changed(&data_home.join("color-schemes").join(format!("{KDE_SCHEME}.colors")), &kde_color_scheme(c));

    // GIMP 3 подключает свой `gimp.css` после темы — только каталоги
    // версий, где GIMP уже запускался.
    if let Ok(rd) = std::fs::read_dir(config_home.join("GIMP")) {
        let block = gimp_css(c);
        for e in rd.flatten() {
            let dir = e.path();
            if dir.join("theme.css").is_file() {
                let css = dir.join("gimp.css");
                ch.gimp |= write_if_changed(&css, &css_with_block(&read(&css), &block, None));
            }
        }
    }
    ch
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors(theme: &str, dark: bool) -> Colors {
        let mut cfg = Config::default();
        cfg.appearance.theme = theme.into();
        cfg.appearance.color_scheme = if dark { crate::config::ColorScheme::Dark } else { crate::config::ColorScheme::Light };
        cfg.appearance.load_theme();
        Colors::from_config(&cfg)
    }

    #[test]
    fn breeze_names_complete() {
        let css = gtk_breeze_css(&colors("tokyo-night", true));
        assert_eq!(css.lines().count(), 84);
        assert!(css.contains("@define-color theme_bg_color_breeze #1a1b26;"));
        assert!(css.contains("@define-color theme_base_color_breeze #16161e;"));
        assert!(css.contains("@define-color theme_selected_bg_color_breeze #7aa2f7;"));
    }

    #[test]
    fn light_view_is_lighter() {
        let c = colors("tokyo-night", false);
        assert!(!c.dark);
        assert!(c.view.luminance() > c.window.luminance());
    }

    #[test]
    fn css_block_replaced_and_user_lines_kept() {
        let c = colors("nord", true);
        let first = css_with_block("@import 'colors.css';\nwindow { opacity: 1; }\n", &gtk_adwaita_css(&c, true), Some("@import 'colors.css';"));
        assert!(first.starts_with("@import 'colors.css';\nwindow { opacity: 1; }\n/* syndesktop"));
        assert!(first.contains("--window-bg-color:"));
        let c2 = colors("dracula", true);
        let second = css_with_block(&first, &gtk_adwaita_css(&c2, true), Some("@import 'colors.css';"));
        assert_eq!(second.matches(BEGIN).count(), 1);
        assert!(second.contains("window { opacity: 1; }"));
        assert_eq!(css_with_block(&second, &gtk_adwaita_css(&c2, true), Some("@import 'colors.css';")), second);
        // Без импорта в исходном файле — добавляется.
        assert!(css_with_block("", "x\n", Some("@import 'colors.css';")).starts_with("@import 'colors.css';\n"));
        // В GTK 3 нет :root.
        assert!(!gtk_adwaita_css(&c, false).contains(":root"));
    }

    #[test]
    fn kdeglobals_keeps_other_groups() {
        let old = "[$Version]\nupdate_info=x\n\n[Colors:Window]\nBackgroundNormal=1,2,3\n\n[General]\nColorScheme=Other\nColorSchemeHash=abc\nfixed=Hack,10\n\n[Colors:Header][Inactive]\nBackgroundNormal=1,1,1\n\n[KDE]\nwidgetStyle=Breeze\n\n[WM]\nactiveFont=Hack,10\nactiveBackground=1,1,1\n";
        let c = colors("tokyo-night", true);
        let new = kdeglobals_with(old, &c);
        assert!(new.contains("[$Version]\nupdate_info=x\n"));
        assert!(new.contains("fixed=Hack,10"));
        assert!(new.contains("ColorScheme=Syndesktop"));
        assert!(!new.contains("ColorSchemeHash"));
        assert!(new.contains("[KDE]\nwidgetStyle=Breeze\n"));
        assert!(!new.contains("BackgroundNormal=1,2,3"));
        assert_eq!(new.matches("[Colors:Window]").count(), 1);
        assert_eq!(new.matches("[Colors:Header][Inactive]").count(), 1);
        assert!(new.contains("[Colors:View]\nBackgroundAlternate="));
        assert!(new.contains("BackgroundNormal=22,22,30"));
        assert!(new.contains("activeFont=Hack,10"));
        assert!(new.contains("activeBackground=26,27,38"));
        assert_eq!(new.matches("[WM]").count(), 1);
        assert_eq!(kdeglobals_with(&new, &c), new);
    }

    #[test]
    fn settings_ini() {
        let s = gtk_settings_ini_with("[Settings]\ngtk-theme-name=Breeze\ngtk-application-prefer-dark-theme=true\n", false);
        assert_eq!(s, "[Settings]\ngtk-theme-name=Breeze\ngtk-application-prefer-dark-theme=false\n");
        assert_eq!(gtk_settings_ini_with("", true), "[Settings]\ngtk-application-prefer-dark-theme=true\n");
    }

    #[test]
    fn apply_writes_once() {
        let dir = std::env::temp_dir().join(format!("syndesktop-app-theme-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (cfg_home, data) = (dir.join("config"), dir.join("data"));
        std::fs::create_dir_all(cfg_home.join("GIMP/3.2")).unwrap();
        std::fs::write(cfg_home.join("GIMP/3.2/theme.css"), "").unwrap();
        std::fs::create_dir_all(cfg_home.join("GIMP/2.10")).unwrap();
        std::fs::write(cfg_home.join("kdeglobals"), "[General]\nColorScheme=BreezeDark\n").unwrap();
        let c = colors("gruvbox", true);
        let ch = apply_in(&cfg_home, &data, &c);
        assert!(ch.gtk && ch.kde && ch.gimp);
        assert!(cfg_home.join("gtk-4.0/colors.css").is_file());
        assert!(cfg_home.join("GIMP/3.2/gimp.css").is_file());
        assert!(!cfg_home.join("GIMP/2.10/gimp.css").exists());
        assert!(data.join("color-schemes/Syndesktop.colors").is_file());
        assert!(std::fs::read_to_string(cfg_home.join("kdeglobals.syndesktop-backup")).unwrap().contains("BreezeDark"));
        assert_eq!(apply_in(&cfg_home, &data, &c), Changed::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
