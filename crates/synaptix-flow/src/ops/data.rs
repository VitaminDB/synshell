//! Ноды данных.

use std::path::PathBuf;

use crate::run::Cx;
use crate::value::Val;

type R = Result<(), String>;

pub fn text(cx: &mut Cx) -> R {
    let t = cx.val("text").text();
    cx.out("text", Val::Text(t));
    Ok(())
}

pub fn file(cx: &mut Cx) -> R {
    let p = cx.need_file("path")?;
    let p = PathBuf::from(expand_home(&p));
    if !p.exists() {
        return Err(format!("нет файла {}", p.display()));
    }
    cx.out("file", Val::File(p));
    Ok(())
}

fn expand_home(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", std::env::var("HOME").unwrap_or_default()),
        None => p.to_string(),
    }
}

/// Подставить `{{a}}` … `{{f}}` (и с пробелами внутри скобок).
pub fn format(cx: &mut Cx) -> R {
    let mut s = cx.val("template").text();
    for k in ["a", "b", "c", "d", "e", "f"] {
        let v = cx.val(k).text();
        s = s.replace(&format!("{{{{{k}}}}}"), &v).replace(&format!("{{{{ {k} }}}}"), &v);
    }
    cx.out("text", Val::Text(s));
    Ok(())
}

/// Непустые строки без маркеров списка («- », «1. », «• »).
pub fn split_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim().trim_start_matches(|c: char| c == '-' || c == '*' || c == '•' || c.is_ascii_digit() || c == '.' || c == ')').trim())
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

pub fn lines(cx: &mut Cx) -> R {
    let t = cx.val("text").text();
    cx.out("list", Val::List(split_lines(&t).into_iter().map(Val::Text).collect()));
    Ok(())
}

/// Статические элементы (JSON-массив или строки поля), затем входы по порядку.
pub fn list(cx: &mut Cx) -> R {
    let mut out: Vec<Val> = match cx.node.fields.get("items") {
        Some(serde_json::Value::Array(a)) => a.iter().map(Val::from_json).collect(),
        Some(serde_json::Value::String(s)) => s.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| Val::Text(l.into())).collect(),
        _ => Vec::new(),
    };
    for k in ["in1", "in2", "in3", "in4", "in5", "in6"] {
        out.extend(cx.val(k).items());
    }
    cx.out("list", Val::List(out));
    Ok(())
}
