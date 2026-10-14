//! Пакетное переименование: новое имя по шаблону (`{name}` — прежнее имя без
//! расширения, `{ext}` — расширение, `{n}` — номер), замена текста или
//! регулярного выражения, смена регистра. Предпросмотр и проверка до
//! переименования; всё переименование отменяется одним Ctrl+Z.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use syngui::t;

use crate::ops;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Case {
    #[default]
    Keep,
    Lower,
    Upper,
    Title,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub find: String,
    pub replace: String,
    pub regex: bool,
    pub ignore_case: bool,
    /// Пусто — `{name}{ext}`.
    pub template: String,
    pub start: u32,
    pub case: Case,
}

impl Default for Rule {
    fn default() -> Self {
        Self { find: String::new(), replace: String::new(), regex: false, ignore_case: false, template: "{name}".into(), start: 1, case: Case::Keep }
    }
}

/// Имя и расширение с точкой; у папок и «скрытых» без точки внутри — без расширения.
pub fn split_ext(name: &str, is_dir: bool) -> (&str, &str) {
    if is_dir {
        return (name, "");
    }
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start = true;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if start {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            start = false;
        } else {
            out.push(c);
            start = true;
        }
    }
    out
}

/// Новые имена для `(имя, папка ли)`; ошибка — в шаблоне или регулярке.
pub fn new_names(items: &[(String, bool)], r: &Rule) -> Result<Vec<String>, String> {
    let re = if r.find.is_empty() {
        None
    } else {
        let pat = if r.regex { r.find.clone() } else { regex::escape(&r.find) };
        Some(regex::RegexBuilder::new(&pat).case_insensitive(r.ignore_case).build().map_err(|e| t!("Ошибка в выражении: {e}", e = e))?)
    };
    let template = if r.template.trim().is_empty() { "{name}" } else { r.template.as_str() };
    let width = ((r.start as usize + items.len()).saturating_sub(1)).to_string().len();
    Ok(items
        .iter()
        .enumerate()
        .map(|(i, (name, is_dir))| {
            let (stem, ext) = split_ext(name, *is_dir);
            let stem = match &re {
                Some(re) if r.regex => re.replace_all(stem, r.replace.as_str()).into_owned(),
                Some(re) => re.replace_all(stem, regex::NoExpand(&r.replace)).into_owned(),
                None => stem.to_string(),
            };
            let num = format!("{:0width$}", r.start as usize + i);
            let mut out = template.replace("{name}", &stem).replace("{n}", &num);
            out = if out.contains("{ext}") { out.replace("{ext}", ext.trim_start_matches('.')) } else { format!("{out}{ext}") };
            match r.case {
                Case::Keep => out,
                Case::Lower => out.to_lowercase(),
                Case::Upper => out.to_uppercase(),
                Case::Title => title_case(&out),
            }
        })
        .collect())
}

/// Чем плохо новое имя `i`-го файла (`None` — можно).
pub fn problems(paths: &[PathBuf], names: &[String]) -> Vec<Option<String>> {
    let sources: HashSet<&Path> = paths.iter().map(PathBuf::as_path).collect();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut dup: HashSet<PathBuf> = HashSet::new();
    let targets: Vec<PathBuf> = paths.iter().zip(names).map(|(p, n)| p.parent().unwrap_or(Path::new("/")).join(n.trim())).collect();
    for t in &targets {
        if !seen.insert(t.clone()) {
            dup.insert(t.clone());
        }
    }
    paths
        .iter()
        .zip(names)
        .zip(&targets)
        .map(|((p, n), t)| {
            if let Err(e) = ops::check_name(n) {
                return Some(e);
            }
            if dup.contains(t) {
                return Some(t!("одинаковые имена").into());
            }
            // Занято чужим файлом (не из переименовываемых).
            if t != p && !sources.contains(t.as_path()) && t.symlink_metadata().is_ok() {
                return Some(t!("«{v}» уже существует", v = n.trim()));
            }
            None
        })
        .collect()
}

/// Переименовать: сначала во временные имена, затем в новые — так меняются
/// местами и сдвигаются номера. Возвращает пары `(откуда, куда)`.
pub fn apply(paths: &[PathBuf], names: &[String]) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    if let Some(e) = problems(paths, names).into_iter().flatten().next() {
        return Err(e);
    }
    let pairs: Vec<(PathBuf, PathBuf)> = paths
        .iter()
        .zip(names)
        .map(|(p, n)| (p.clone(), p.parent().unwrap_or(Path::new("/")).join(n.trim())))
        .filter(|(a, b)| a != b)
        .collect();
    let tag = format!(".synfiles-rename-{}", std::process::id());
    let mut staged: Vec<(PathBuf, PathBuf, PathBuf)> = Vec::new();
    let rollback = |staged: &[(PathBuf, PathBuf, PathBuf)]| {
        for (from, tmp, _) in staged.iter().rev() {
            let _ = std::fs::rename(tmp, from);
        }
    };
    for (i, (from, to)) in pairs.iter().enumerate() {
        let tmp = from.parent().unwrap_or(Path::new("/")).join(format!("{tag}-{i}"));
        if let Err(e) = std::fs::rename(from, &tmp) {
            rollback(&staged);
            return Err(format!("{}: {e}", ops::name_of(from)));
        }
        staged.push((from.clone(), tmp, to.clone()));
    }
    let mut done = Vec::new();
    for (k, (from, tmp, to)) in staged.iter().enumerate() {
        if let Err(e) = std::fs::rename(tmp, to) {
            // Уже переименованные вернуть, остальные — из временных имён.
            for (f, t) in done.iter().rev() {
                let _ = std::fs::rename(t, f);
            }
            rollback(&staged[k..]);
            return Err(format!("{}: {e}", ops::name_of(from)));
        }
        done.push((from.clone(), to.clone()));
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(v: &[&str]) -> Vec<(String, bool)> {
        v.iter().map(|s| (s.to_string(), false)).collect()
    }

    #[test]
    fn template_numbers_and_ext() {
        let r = Rule { template: "Отпуск {n}".into(), ..Rule::default() };
        let v = new_names(&items(&["a.JPG", "b.jpg", ".hidden"]), &r).unwrap();
        assert_eq!(v, ["Отпуск 1.JPG", "Отпуск 2.jpg", "Отпуск 3"]);
        let r = Rule { template: "{n}-{name}.{ext}".into(), start: 9, case: Case::Lower, ..Rule::default() };
        assert_eq!(new_names(&items(&["A.TXT", "B.txt"]), &r).unwrap(), ["09-a.txt", "10-b.txt"]);
    }

    #[test]
    fn find_replace_plain_and_regex() {
        let r = Rule { find: "IMG_".into(), replace: "$фото ".into(), ignore_case: true, ..Rule::default() };
        assert_eq!(new_names(&items(&["img_001.png"]), &r).unwrap(), ["$фото 001.png"]);
        let r = Rule { find: r"(\d+)-(\d+)".into(), replace: "$2-$1".into(), regex: true, ..Rule::default() };
        assert_eq!(new_names(&items(&["10-20.txt"]), &r).unwrap(), ["20-10.txt"]);
        let r = Rule { find: "(".into(), regex: true, ..Rule::default() };
        assert!(new_names(&items(&["x"]), &r).is_err());
        let r = Rule { case: Case::Title, ..Rule::default() };
        assert_eq!(new_names(&[("мой отчёт-final".into(), true)], &r).unwrap(), ["Мой Отчёт-Final"]);
    }

    #[test]
    fn swap_and_conflicts() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("1.txt");
        let b = d.path().join("2.txt");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        std::fs::write(d.path().join("чужой.txt"), "").unwrap();
        let paths = vec![a.clone(), b.clone()];
        // Обмен именами проходит.
        apply(&paths, &["2.txt".into(), "1.txt".into()]).unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "b");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "a");
        // Дубли и занятое чужим — нет, файлы не тронуты.
        let p = problems(&paths, &["x.txt".into(), "x.txt".into()]);
        assert!(p.iter().all(Option::is_some));
        assert!(apply(&paths, &["чужой.txt".into(), "3.txt".into()]).is_err());
        assert!(a.exists() && b.exists());
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 3);
    }
}
