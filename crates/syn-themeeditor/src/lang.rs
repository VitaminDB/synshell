//! Языковой сервис MSS для режима «Код» (`mss-analysis`): переменные темы,
//! классы и `@keyframes` оболочки и «Параметров системы» — в контекст
//! анализатора; файл контекста для `mss-lsp`.

use std::sync::{Arc, LazyLock};

use mss_analysis::{Context, VarInfo};

const SOURCES: &[&str] = &[include_str!("../../synshell-ui/styles/shell.mss"), include_str!("../../synsettings/styles/settings.mss")];

/// Классы и анимации встроенных стилей (один раз за запуск).
static BUILTIN: LazyLock<(Vec<String>, Vec<String>)> = LazyLock::new(|| {
    let mut classes = std::collections::BTreeSet::new();
    let mut keyframes = std::collections::BTreeSet::new();
    for src in SOURCES {
        let b = src.as_bytes();
        let mut i = 0;
        while i < b.len() {
            // Комментарии пропускаются.
            if src[i..].starts_with("/*") {
                i = src[i + 2..].find("*/").map(|e| i + 2 + e + 2).unwrap_or(b.len());
                continue;
            }
            if src[i..].starts_with("@keyframes") {
                let rest = src[i + 10..].trim_start();
                let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect();
                if !name.is_empty() {
                    keyframes.insert(name);
                }
                i += 10;
                continue;
            }
            // `.класс` в селекторе: перед ним не цифра (не «0.5»).
            if b[i] == b'.' && (i == 0 || !b[i - 1].is_ascii_digit()) {
                let name: String = src[i + 1..].chars().take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect();
                if name.chars().next().is_some_and(|c| c.is_alphabetic()) {
                    classes.insert(name);
                }
            }
            i += 1;
        }
    }
    (classes.into_iter().collect(), keyframes.into_iter().collect())
});

/// Контекст анализатора: переменные (имя → значение черновика и описание из
/// реестра токенов), классы и анимации встроенных стилей.
pub fn context(values: &std::collections::BTreeMap<String, String>) -> Arc<Context> {
    let mut vars: Vec<VarInfo> = Vec::new();
    for tok in synshell_common::tokens::TOKENS {
        vars.push(VarInfo {
            name: format!("--{}", tok.name),
            value: values.get(tok.name).cloned().unwrap_or_default(),
            doc: format!("{} — {}", synshell_tr::t(tok.label), synshell_tr::t(tok.doc)),
        });
    }
    for (k, v) in values {
        if k.starts_with('@') || synshell_common::tokens::find(k).is_some() {
            continue;
        }
        vars.push(VarInfo { name: format!("--{k}"), value: v.clone(), doc: synshell_tr::t("Своя переменная темы") });
    }
    Arc::new(Context { variables: vars, known_classes: BUILTIN.0.clone(), keyframes: BUILTIN.1.clone() })
}

/// Файл контекста для `mss-lsp` (`.mss-context.json`).
pub fn context_json(values: &std::collections::BTreeMap<String, String>) -> String {
    let ctx = context(values);
    let v = serde_json::json!({
        "variables": ctx.variables.iter().map(|v| serde_json::json!({ "name": v.name, "value": v.value, "doc": v.doc })).collect::<Vec<_>>(),
        "classes": ctx.known_classes,
        "keyframes": ctx.keyframes,
    });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// Контекст для всего репозитория: сверх [`context_json`] — переменные всех
/// тем (`[dark.vars]`, `[light.vars]`) и свои переменные программ, которые
/// задаются в Rust (`"--sidebar: {}"` в исходниках каталога `scan`).
pub fn repo_context_json(values: &std::collections::BTreeMap<String, String>, scan: &std::path::Path) -> String {
    let mut all = values.clone();
    for t in synshell_common::theme::list() {
        for v in [&t.dark, &t.light].into_iter().flatten() {
            for (k, val) in &v.vars {
                all.entry(k.trim_start_matches('-').to_string()).or_insert_with(|| val.clone());
            }
        }
    }
    let mut stack = vec![scan.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name();
            if p.is_dir() {
                if name != "target" && !name.to_string_lossy().starts_with('.') {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|x| x == "rs") {
                let Ok(src) = std::fs::read_to_string(&p) else { continue };
                // Кортежи `("plum", "#a974f0", …)` и `("sidebar", x.hex())`.
                for line in src.lines().filter(|l| l.contains(".hex()") || l.contains("\"#") || l.contains("px(")) {
                    let mut r = line;
                    while let Some(i) = r.find("(\"") {
                        let t = &r[i + 2..];
                        let n: String = t.chars().take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-').collect();
                        if !n.is_empty() && t[n.len()..].starts_with("\", ") {
                            all.entry(n.clone()).or_default();
                        }
                        r = &t[n.len()..];
                    }
                }
                let mut rest = src.as_str();
                while let Some(i) = rest.find("--") {
                    let tail = &rest[i + 2..];
                    let n: String = tail.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
                    let after = &tail[n.len()..];
                    if !n.is_empty() && after.starts_with(": {") {
                        all.entry(n.clone()).or_default();
                    }
                    let step = if n.is_empty() { tail.chars().next().map_or(0, char::len_utf8) } else { n.len() };
                    if step == 0 {
                        break;
                    }
                    rest = &tail[step..];
                }
            }
        }
    }
    context_json(&all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_classes_and_keyframes() {
        assert!(BUILTIN.0.iter().any(|c| c == "panel"));
        assert!(BUILTIN.0.iter().any(|c| c == "nav-item"));
        assert!(!BUILTIN.0.iter().any(|c| c.starts_with(char::is_numeric)));
    }

    #[test]
    fn theme_layer_has_no_unknown_vars() {
        let t = synshell_common::theme::Theme::find("tokyo-night").unwrap();
        let mut values = std::collections::BTreeMap::new();
        for (k, v) in &t.dark.as_ref().unwrap().vars {
            values.insert(k.clone(), v.clone());
        }
        let ctx = context(&values);
        let diags = mss_analysis::diagnostics(t.app_mss("shell"), &ctx);
        let unknown: Vec<_> = diags.iter().filter(|d| d.message.contains("--")).collect();
        assert!(unknown.is_empty(), "{unknown:?}");
    }
}
