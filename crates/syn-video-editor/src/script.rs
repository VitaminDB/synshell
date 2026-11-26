//! Сценарии Rhai (F1):
//! - виды-раскладки: `fn layout()` (или последнее выражение) собирает док из
//!   `tabs([...])`, `tabs([...], "имя")`, `row(a, b, доля)`, `column(a, b, доля)`;
//!   описание — строки `//! name: …`, `//! description: …`, `//! order: …`;
//! - массовые правки проекта за один вызов (`script.run`): `cmd(id, #{…})` — любая
//!   команда таблицы (ошибка прерывает сценарий и откатывает его целиком),
//!   `project()` — проект данными, `selection()`, `playhead()`, `print(…)` — в ответ.

use std::cell::RefCell;
use std::rc::Rc;

use rhai::{Array, Dynamic, Engine, EvalAltResult, Map, Position, Scope, AST};
use serde_json::Value;
use syngui::widgets::DockSpec;

use crate::editor::Editor;

/// Предел операций сценария (защита от вечного цикла).
const MAX_OPS: u64 = 50_000_000;

fn base_engine() -> Engine {
    let mut e = Engine::new();
    e.set_max_operations(MAX_OPS);
    e.set_max_expr_depths(128, 64);
    e
}

fn err(msg: impl Into<String>) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorRuntime(Dynamic::from(msg.into()), Position::NONE))
}

fn strings(a: &Array) -> Vec<String> {
    a.iter().map(|v| v.clone().into_string().unwrap_or_else(|_| v.to_string())).collect()
}

/// Движок раскладок.
fn layout_engine() -> Engine {
    let mut e = base_engine();
    e.register_type_with_name::<DockSpec>("Dock");
    e.register_fn("tabs", |a: Array| DockSpec::Tabs { panels: strings(&a), active: 0, name: String::new() });
    e.register_fn("tabs", |a: Array, name: &str| DockSpec::Tabs { panels: strings(&a), active: 0, name: name.to_string() });
    e.register_fn("tabs", |a: Array, name: &str, active: i64| {
        let panels = strings(&a);
        let active = (active.max(0) as usize).min(panels.len().saturating_sub(1));
        DockSpec::Tabs { panels, active, name: name.to_string() }
    });
    e.register_fn("row", |a: DockSpec, b: DockSpec, r: f64| DockSpec::row(a, b, r as f32));
    e.register_fn("row", |a: DockSpec, b: DockSpec| DockSpec::row(a, b, 0.5));
    e.register_fn("column", |a: DockSpec, b: DockSpec, r: f64| DockSpec::column(a, b, r as f32));
    e.register_fn("column", |a: DockSpec, b: DockSpec| DockSpec::column(a, b, 0.5));
    e
}

/// Заголовок сценария: `//! ключ: значение`.
pub fn header(code: &str, key: &str) -> Option<String> {
    code.lines().map(str::trim).filter_map(|l| l.strip_prefix("//!")).find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == key).then(|| v.trim().to_string())
    })
}

/// Вид из сценария: (название, описание, порядок, раскладка).
pub fn eval_layout(code: &str) -> Result<(String, String, i32, DockSpec), String> {
    let e = layout_engine();
    let ast: AST = e.compile(code).map_err(|x| x.to_string())?;
    let mut scope = Scope::new();
    let has_fn = ast.iter_functions().any(|f| f.name == "layout" && f.params.is_empty());
    let v: Dynamic = if has_fn { e.call_fn(&mut scope, &ast, "layout", ()) } else { e.eval_ast_with_scope(&mut scope, &ast) }.map_err(|x| x.to_string())?;
    let spec = v.try_cast::<DockSpec>().ok_or("сценарий вида должен вернуть tabs/row/column")?;
    let name = header(code, "name").unwrap_or_default();
    let desc = header(code, "description").unwrap_or_default();
    let order = header(code, "order").and_then(|o| o.parse().ok()).unwrap_or(0);
    Ok((name, desc, order, spec))
}

fn quote(s: &str) -> String {
    format!("{s:?}")
}

fn spec_code(s: &DockSpec, ind: usize, out: &mut String) {
    let pad = "    ".repeat(ind);
    match s {
        DockSpec::Tabs { panels, active, name } => {
            let list = panels.iter().map(|p| quote(p)).collect::<Vec<_>>().join(", ");
            out.push_str(&pad);
            if *active > 0 {
                out.push_str(&format!("tabs([{list}], {}, {active})", quote(name)));
            } else if !name.is_empty() {
                out.push_str(&format!("tabs([{list}], {})", quote(name)));
            } else {
                out.push_str(&format!("tabs([{list}])"));
            }
        }
        DockSpec::Split { vertical, ratio, first, second } => {
            out.push_str(&pad);
            out.push_str(if *vertical { "column(\n" } else { "row(\n" });
            spec_code(first, ind + 1, out);
            out.push_str(",\n");
            spec_code(second, ind + 1, out);
            out.push_str(&format!(",\n{pad}    {:.3}\n{pad})", ratio.clamp(0.05, 0.95)));
        }
    }
}

/// Сценарий вида из раскладки (для «Сохранить вид шаблоном»).
pub fn layout_code(spec: &DockSpec, name: &str, description: &str, order: i32) -> String {
    let mut s = String::new();
    s.push_str(&format!("//! name: {}\n", name.replace('\n', " ")));
    if !description.trim().is_empty() {
        s.push_str(&format!("//! description: {}\n", description.replace('\n', " ")));
    }
    if order != 0 {
        s.push_str(&format!("//! order: {order}\n"));
    }
    s.push_str("// Вид редактора: tabs([панели], \"имя\", активная), row(слева, справа, доля), column(сверху, снизу, доля).\n");
    s.push_str("// Панели: ");
    s.push_str(&crate::layout::PANELS.iter().map(|p| p.0).collect::<Vec<_>>().join(", "));
    s.push_str("\nfn layout() {\n");
    spec_code(spec, 1, &mut s);
    s.push_str("\n}\n");
    s
}

fn to_dyn(v: &Value) -> Dynamic {
    rhai::serde::to_dynamic(v).unwrap_or(Dynamic::UNIT)
}

fn from_dyn(d: &Dynamic) -> Result<Value, Box<EvalAltResult>> {
    rhai::serde::from_dynamic::<Value>(d)
}

/// Выполнить сценарий правок над редактором. Всё — один шаг отмены; ошибка
/// откатывает сделанное сценарием. Возвращает вывод `print` и результаты.
pub fn run(ed: &mut Editor, code: &str) -> Result<String, String> {
    run_with(ed, code, Value::Null)
}

/// То же с параметрами: в сценарии — переменная `params` (объект).
pub fn run_with(ed: &mut Editor, code: &str, params: Value) -> Result<String, String> {
    let shared = Rc::new(RefCell::new(std::mem::take(ed)));
    let out = Rc::new(RefCell::new(Vec::<String>::new()));
    let calls = Rc::new(RefCell::new(0usize));
    let res = {
        let mut e = base_engine();
        let o = out.clone();
        e.on_print(move |s| o.borrow_mut().push(s.to_string()));
        let o = out.clone();
        e.on_debug(move |s, _, _| o.borrow_mut().push(s.to_string()));
        let call = {
            let ed = shared.clone();
            let calls = calls.clone();
            move |id: &str, args: Dynamic| -> Result<Map, Box<EvalAltResult>> {
                let c = crate::cmd::find(id).ok_or_else(|| err(format!("нет команды «{id}»")))?;
                if c.id == "script.run" {
                    return Err(err("script.run внутри сценария нельзя"));
                }
                if c.danger {
                    return Err(err(format!("{}: необратимая команда — только отдельным вызовом", c.id)));
                }
                let args = if args.is_unit() { Value::Null } else { from_dyn(&args)? };
                *calls.borrow_mut() += 1;
                let r = crate::cmd::run_cmd(&mut ed.borrow_mut(), c, &args).map_err(err)?;
                let mut m = Map::new();
                m.insert("text".into(), Dynamic::from(r.text));
                // нет данных — пустой объект: `r.data.поле` даёт (), а не ошибку
                m.insert("data".into(), if r.data.is_null() { Dynamic::from(Map::new()) } else { to_dyn(&r.data) });
                Ok(m)
            }
        };
        let c2 = call.clone();
        e.register_fn("cmd", call);
        e.register_fn("cmd", move |id: &str| c2(id, Dynamic::UNIT));
        let ed = shared.clone();
        e.register_fn("project", move || serde_json::to_value(&ed.borrow().project).map(|v| to_dyn(&v)).unwrap_or(Dynamic::UNIT));
        let ed = shared.clone();
        e.register_fn("dump", move || ed.borrow().project.dump());
        let ed = shared.clone();
        e.register_fn("selection", move || -> Array { ed.borrow().selection.iter().map(|i| Dynamic::from(*i as i64)).collect() });
        let ed = shared.clone();
        e.register_fn("playhead", move || ed.borrow().playhead);
        // файлы библиотеки ассетов (DaVinci Resolve: Футажи, Звуки, LUT…):
        // library("Звуки/Свуши", "whoosh", 10) — пути, отобранные по словам
        e.register_fn("library", |sub: &str, query: &str, limit: i64| -> Array {
            let root = crate::fusion::library().join(sub);
            let q: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
            let mut v: Vec<std::path::PathBuf> = walkdir_files(&root);
            v.retain(|p| {
                let n = p.to_string_lossy().to_lowercase();
                q.iter().all(|w| n.contains(w.as_str()))
            });
            v.sort();
            v.into_iter().take(limit.max(1) as usize).map(|p| Dynamic::from(p.display().to_string())).collect()
        });
        // список из JSON: пустые поля проекта не пишутся — () → []
        e.register_fn("list", |d: Dynamic| -> Array { d.try_cast::<Array>().unwrap_or_default() });
        // число из JSON (целое или дробное) → дробное
        e.register_fn("num", |d: Dynamic| -> f64 { d.as_float().or_else(|_| d.as_int().map(|i| i as f64)).unwrap_or(0.0) });
        e.register_fn("num", |d: Dynamic, def: f64| -> f64 { if d.is_unit() { def } else { d.as_float().or_else(|_| d.as_int().map(|i| i as f64)).unwrap_or(def) } });
        let mut scope = Scope::new();
        let p = if params.is_null() { Dynamic::from(Map::new()) } else { to_dyn(&params) };
        scope.push("params", p);
        e.eval_with_scope::<Dynamic>(&mut scope, code)
    };
    *ed = Rc::try_unwrap(shared).map(RefCell::into_inner).unwrap_or_default();
    let mut lines = out.take();
    let n = *calls.borrow();
    match res {
        Ok(v) => {
            if !v.is_unit() {
                lines.push(format!("= {v}"));
            }
            lines.push(format!("сценарий: команд {n}"));
            Ok(lines.join("\n"))
        }
        Err(x) => {
            lines.push(format!("ошибка сценария: {x}"));
            Err(lines.join("\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_roundtrip() {
        let spec = DockSpec::column(
            DockSpec::row(DockSpec::Tabs { panels: vec!["media".into(), "effects".into()], active: 1, name: String::new() }, DockSpec::tabs(["viewer"]).named("viewer"), 0.3),
            DockSpec::tabs(["timeline", "curves"]),
            0.6,
        );
        let code = layout_code(&spec, "Мой вид", "две колонки", 55);
        let (n, d, o, back) = eval_layout(&code).unwrap();
        assert_eq!((n.as_str(), d.as_str(), o), ("Мой вид", "две колонки", 55));
        assert_eq!(back, spec);
        // без fn layout — последнее выражение
        let (_, _, _, s) = eval_layout("row(tabs([\"viewer\"]), tabs([\"ai\"]), 0.7)").unwrap();
        assert_eq!(s, DockSpec::row(DockSpec::tabs(["viewer"]), DockSpec::tabs(["ai"]), 0.7));
        assert!(eval_layout("42").is_err());
    }
}

/// Проверка синтаксиса сценария без запуска: `Ok(())` или ошибка с позицией
/// («строка N, позиция M: …»).
pub fn check(code: &str) -> Result<(), String> {
    base_engine().compile(code).map(|_| ()).map_err(|e| {
        let p = e.position();
        match (p.line(), p.position()) {
            (Some(l), Some(c)) => format!("строка {l}, позиция {c}: {}", e.err_type()),
            (Some(l), None) => format!("строка {l}: {}", e.err_type()),
            _ => e.to_string(),
        }
    })
}

/// Файлы каталога рекурсивно (медиа, звук, .setting, .cube).
fn walkdir_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out
}
