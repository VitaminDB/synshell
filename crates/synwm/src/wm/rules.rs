//! Правила окон (`[[rule]]`): условия — регулярные выражения по app_id и
//! заголовку, действия — всё, что не задано, остаётся по умолчанию.

use regex::Regex;
use synshell_common::config::WindowRule;

pub struct CompiledRule {
    pub app_id: Option<Regex>,
    pub title: Option<Regex>,
    pub rule: WindowRule,
}

pub fn compile(rules: &[WindowRule]) -> (Vec<CompiledRule>, Vec<String>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (i, r) in rules.iter().enumerate() {
        let re = |s: &Option<String>, what: &str, errors: &mut Vec<String>| -> Result<Option<Regex>, ()> {
            match s {
                None => Ok(None),
                Some(p) => Regex::new(p).map(Some).map_err(|e| {
                    errors.push(format!("правило #{}: {what}: {e}", i + 1));
                }),
            }
        };
        let (Ok(app_id), Ok(title)) = (re(&r.app_id, "app_id", &mut errors), re(&r.title, "title", &mut errors)) else {
            continue;
        };
        out.push(CompiledRule { app_id, title, rule: r.clone() });
    }
    (out, errors)
}

/// Итог применения всех подходящих правил (поздние перекрывают ранние).
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    pub floating: Option<bool>,
    pub workspace: Option<u32>,
    pub output: Option<String>,
    pub size: Option<[i32; 2]>,
    pub position: Option<[i32; 2]>,
    pub center: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub sticky: bool,
    pub always_on_top: bool,
    pub opacity: Option<f32>,
    pub decorations: Option<bool>,
    pub no_focus: bool,
    pub skip_taskbar: bool,
    pub min_size: Option<[i32; 2]>,
    pub max_size: Option<[i32; 2]>,
}

pub fn resolve(rules: &[CompiledRule], app_id: &str, title: &str) -> Resolved {
    let mut r = Resolved::default();
    for c in rules {
        if c.app_id.as_ref().is_some_and(|re| !re.is_match(app_id)) {
            continue;
        }
        if c.title.as_ref().is_some_and(|re| !re.is_match(title)) {
            continue;
        }
        if c.app_id.is_none() && c.title.is_none() {
            // Правило без условий применяется ко всем окнам — это допустимо.
        }
        let w = &c.rule;
        macro_rules! take {
            ($($f:ident),*) => { $( if w.$f.is_some() { r.$f = w.$f.clone(); } )* };
        }
        take!(floating, workspace, output, size, position, opacity, decorations, min_size, max_size);
        if let Some(v) = w.center {
            r.center = v;
        }
        if let Some(v) = w.maximized {
            r.maximized = v;
        }
        if let Some(v) = w.fullscreen {
            r.fullscreen = v;
        }
        if let Some(v) = w.sticky {
            r.sticky = v;
        }
        if let Some(v) = w.always_on_top {
            r.always_on_top = v;
        }
        if let Some(v) = w.no_focus {
            r.no_focus = v;
        }
        if let Some(v) = w.skip_taskbar {
            r.skip_taskbar = v;
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_and_overrides() {
        let rules = vec![
            WindowRule { app_id: Some("^firefox$".into()), workspace: Some(2), ..Default::default() },
            WindowRule { title: Some("Picture".into()), always_on_top: Some(true), ..Default::default() },
            WindowRule { app_id: Some("fire".into()), workspace: Some(3), ..Default::default() },
        ];
        let (c, e) = compile(&rules);
        assert!(e.is_empty());
        let r = resolve(&c, "firefox", "Picture-in-Picture");
        assert_eq!(r.workspace, Some(3));
        assert!(r.always_on_top);
        let r = resolve(&c, "konsole", "bash");
        assert_eq!(r.workspace, None);
    }

    #[test]
    fn bad_regex_reported() {
        let (c, e) = compile(&[WindowRule { app_id: Some("(".into()), ..Default::default() }]);
        assert!(c.is_empty());
        assert_eq!(e.len(), 1);
    }
}
