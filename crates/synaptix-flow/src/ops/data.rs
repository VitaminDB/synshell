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

/// Подставить `{{a}}` … `{{f}}` (а также `{1}` … `{6}` и варианты с пробелами).
pub fn format(cx: &mut Cx) -> R {
    let mut s = cx.val("template").text();
    for (idx, k) in ["a", "b", "c", "d", "e", "f"].iter().enumerate() {
        let v = cx.val(k).text();
        let n = idx + 1;
        s = s
            .replace(&format!("{{{{{k}}}}}"), &v)
            .replace(&format!("{{{{ {k} }}}}"), &v)
            .replace(&format!("{{{n}}}"), &v);
    }
    cx.out("text", Val::Text(s));
    Ok(())
}

/// Непустые строки без маркеров списка («- », «1. », «• », «Сцена 1:»).
/// Пункты, слитые LLM в одну строку («1) … 2) … 3) …»), делятся на отдельные.
pub fn split_lines(text: &str) -> Vec<String> {
    text.lines()
        .flat_map(split_inline)
        .map(strip_marker)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

fn strip_marker(l: &str) -> &str {
    let l = l.trim().trim_start_matches(['-', '*', '•']).trim_start();
    for w in ["Сцена", "Scene", "сцена", "scene"] {
        if let Some(r) = l.strip_prefix(w) {
            let r = r.trim_start();
            let d = r.len() - r.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            if d > 0 {
                return r[d..].trim_start_matches([':', '.', ')', ' ']).trim();
            }
        }
    }
    l.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')').trim()
}

/// Позиция маркера пункта `n` в `s` начиная с `from`: «n)», «n.», «Сцена n:»
/// после пробела (или в начале строки) и перед пробелом.
fn marker_at(s: &str, n: usize, from: usize) -> Option<usize> {
    let pats = [format!("{n})"), format!("{n}."), format!("Сцена {n}:"), format!("Scene {n}:"), format!("Сцена {n}."), format!("Scene {n}.")];
    let mut best: Option<usize> = None;
    for p in &pats {
        let mut i = from;
        while let Some(off) = s[i..].find(p.as_str()) {
            let at = i + off;
            let before_ok = at == 0 || s[..at].ends_with(char::is_whitespace);
            let end = at + p.len();
            let after_ok = end == s.len() || s[end..].starts_with(char::is_whitespace);
            if before_ok && after_ok {
                best = Some(best.map_or(at, |b: usize| b.min(at)));
                break;
            }
            i = at + p.len();
        }
    }
    best
}

fn split_inline(line: &str) -> Vec<&str> {
    let t = line.trim().trim_start_matches(['-', '*', '•']).trim_start();
    if marker_at(t, 1, 0) != Some(0) {
        return vec![line];
    }
    let mut out = Vec::new();
    let (mut start, mut n) = (0usize, 2usize);
    while let Some(at) = marker_at(t, n, start + t[start..].chars().next().map_or(1, char::len_utf8)) {
        out.push(&t[start..at]);
        start = at;
        n += 1;
    }
    out.push(&t[start..]);
    out
}

/// Лишние строки сверх ожидаемых (вступление «Here are the scenes:»,
/// пояснение в конце) короче описаний сцен — оставить `n` самых длинных
/// в исходном порядке.
fn keep_longest(items: Vec<String>, n: usize) -> Vec<String> {
    let mut idx: Vec<usize> = (0..items.len()).collect();
    idx.sort_by_key(|&i| std::cmp::Reverse(items[i].chars().count()));
    let mut keep: Vec<usize> = idx.into_iter().take(n).collect();
    keep.sort_unstable();
    keep.into_iter().map(|i| items[i].clone()).collect()
}

pub fn lines(cx: &mut Cx) -> R {
    let t = cx.val("text").text();
    let mut items = split_lines(&t);
    // Ожидаемое число пунктов (рецепт: сцен столько, сколько заказано) —
    // иначе ошибка, а не молча меньше роликов.
    if let Some(n) = cx.pos("expect").map(|v| v.round() as usize) {
        if items.len() > n && n > 0 {
            items = keep_longest(items, n);
        }
        if items.len() != n {
            let msg = cx.str("expect_msg").unwrap_or_else(|| "получено {k} строк из {n}".into());
            return Err(msg.replace("{k}", &items.len().to_string()).replace("{n}", &n.to_string()));
        }
    }
    cx.out("list", Val::List(items.into_iter().map(Val::Text).collect()));
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

#[cfg(test)]
mod keep_tests {
    #[test]
    fn drops_preamble() {
        let v = vec!["Here are the two scenes:".to_string(), "A long first scene description with the duck and the troll".into(), "A long second scene description where the troll agrees".into()];
        let k = super::keep_longest(v, 2);
        assert_eq!(k.len(), 2);
        assert!(k[0].starts_with("A long first"));
    }
}

#[cfg(test)]
mod tests {
    use super::split_lines;

    #[test]
    fn split_lines_inline_numbered() {
        // Реальный ответ LLM из story-video: шесть сцен одной строкой.
        let t = "1) A troll cook at the counter, \"Grimza, hurry!\". 2) Grimza growls, steam. 3) Same kitchen, 12 spoons fall. 4) Kreya hops. 5) Both dive in. 6) Banquet, fanfare.";
        let v = split_lines(t);
        assert_eq!(v.len(), 6, "{v:?}");
        assert_eq!(v[0], "A troll cook at the counter, \"Grimza, hurry!\".");
        assert_eq!(v[2], "Same kitchen, 12 spoons fall.");
        assert_eq!(v[5], "Banquet, fanfare.");
    }

    #[test]
    fn split_lines_scene_words_and_plain() {
        let v = split_lines("Сцена 1: утка говорит. Сцена 2: тролль ворчит.\n\nСцена 3: суп готов.");
        assert_eq!(v, vec!["утка говорит.", "тролль ворчит.", "суп готов."]);
        let v = split_lines("- first scene with 2. items\n2. second\n• third");
        assert_eq!(v, vec!["first scene with 2. items", "second", "third"]);
        // Строка без маркера «1» не режется по случайным «2.».
        assert_eq!(split_lines("Walks 2. steps then 3. more").len(), 1);
    }
}
