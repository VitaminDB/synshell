//! Тема окна заметок. Стили перенесены из synthos вместе с кодом и говорят
//! его переменными (`--text`, `--primary`, `--surface-hover` …); здесь они
//! выражаются через палитру оформления synshell (`[appearance]`: светлая или
//! тёмная тема, акцент, тема оформления), так что заметки выглядят как
//! остальные программы системы и меняются вместе с ними.

use synshell_common::config::Rgba;
use synshell_common::Config;

const STYLES: &[&str] = &[
    include_str!("../../styles/variables.mss"),
    include_str!("../../styles/reset.mss"),
    include_str!("../../styles/layout.mss"),
    include_str!("../../styles/scrollbars.mss"),
    include_str!("../../styles/notification.mss"),
    include_str!("../../styles/workspace_frame.mss"),
    include_str!("../../styles/panel_header.mss"),
    include_str!("../../styles/right_panel.mss"),
    include_str!("../../styles/code_editor_dialogs.mss"),
    include_str!("../../styles/notes.mss"),
    include_str!("../../styles/app.mss"),
];

/// Переменные synthos из палитры synshell: имя → значение.
fn variables(cfg: &Config) -> Vec<(&'static str, String)> {
    let a = &cfg.appearance;
    let p = a.palette();
    let dark = a.is_dark();
    let r = a.corner_radius.max(0.0);
    let hex = |c: Rgba| c.hex();
    // Полупрозрачные слои — с альфой под линейное смешивание syngui, иначе
    // наведения, кромки и сетка над тёмным фоном выходят серыми и резкими.
    let soft = |c: Rgba| c.perceptual_over(p.bg).hex();
    let vars: Vec<(&str, String)> = vec![
        ("bg-window", hex(p.bg)),
        ("bg-shell", hex(p.bg)),
        ("bg-rail", hex(p.bg)),
        ("bg-chats", hex(p.surface)),
        ("bg-chat", hex(p.bg)),
        ("bg-chat-dots", soft(p.fg.with_alpha(0.06))),
        ("bg-panel", hex(p.surface)),
        ("bg-search", hex(p.surface_alt)),
        ("glass-card-bg", hex(p.surface.with_alpha(0.92))),
        ("glass-field-bg", hex(p.surface_alt)),
        ("glass-border", soft(p.border)),
        ("glass-shadow", if dark { "#00000080".into() } else { "#0000002a".into() }),
        ("shadow-shell", if dark { "#00000080".into() } else { "#00000026".into() }),
        ("primary", hex(p.accent)),
        ("primary-hover", hex(p.accent.with_alpha(0.85))),
        ("primary-soft", soft(p.accent.with_alpha(0.18))),
        ("on-primary", hex(p.accent_fg)),
        ("text", hex(p.fg)),
        ("text-muted", hex(p.muted)),
        ("text-subtle", hex(p.muted.with_alpha(0.75))),
        ("text-inverse", hex(p.bg)),
        ("surface-hover", soft(p.fg.with_alpha(0.07))),
        ("surface-selected", soft(p.accent.with_alpha(0.16))),
        ("border", soft(p.border)),
        ("border-soft", soft(p.border.fade(0.7))),
        ("border-strong", soft(p.fg.with_alpha(0.28))),
        ("error", hex(p.danger)),
        ("error-hover", hex(p.danger.with_alpha(0.85))),
        ("success", hex(p.success)),
        ("warning", hex(p.warning)),
        ("warning-soft", soft(p.warning.with_alpha(0.18))),
        ("radius-shell", format!("{r}px")),
        ("radius-panel", format!("{}px", (r * 0.8).round())),
    ];
    vars
}

/// Базовые токены synthos без тех, что заданы палитрой: MSS берёт первое
/// объявление переменной, повторное ниже его не перекрыло бы.
fn base_variables(skip: &[(&str, String)]) -> String {
    let mut out = String::new();
    for line in STYLES[0].lines() {
        let name = line.trim_start().strip_prefix("--").and_then(|r| r.split(':').next()).map(str::trim);
        if name.is_some_and(|n| skip.iter().any(|(k, _)| *k == n)) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Полная таблица стилей окна.
pub fn mss(cfg: &Config) -> String {
    let vars = variables(cfg);
    let mut s = String::from(":root {\n");
    for (k, v) in &vars {
        s.push_str(&format!("  --{k}: {v};\n"));
    }
    s.push_str("}\n");
    s.push_str(&base_variables(&vars));
    s.push_str(&cfg.appearance.mss_variables());
    for part in &STYLES[1..] {
        s.push_str(part);
        s.push('\n');
    }
    let font = cfg.appearance.font.trim();
    if !font.is_empty() {
        s.push_str(&format!("Text, Button, TextField {{ font-family: \"{font}\"; }}\n"));
    }
    s
}
