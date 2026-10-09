//! Перевод строк интерфейса в крейтах synshell без syngui (synsystem,
//! synmodem, synshell-common, synwm, CLI): тот же синтаксис, что у syngui —
//! `t!("Процессор")`, `t!("{v} ГБ", v = x)`, `tn!(n, "{n} файл", "{n} файла",
//! "{n} файлов")`, `n_!("…")` — и те же каталоги `"исходник" = "перевод"`
//! (`syngui/scripts/i18n-extract.py`).
//!
//! Программа с syngui отдаёт сюда свой переводчик ([`set_translator`]) —
//! строки библиотек переводятся её каталогами и вместе с ней переключают
//! язык. Программа без syngui подключает каталоги сама ([`init`]).

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::{OnceLock, RwLock};

type Translate = fn(&str) -> String;
type TranslateN = fn(u64, &[&str]) -> String;

static HOOK: OnceLock<(Translate, TranslateN)> = OnceLock::new();
static OWN: RwLock<Option<Own>> = RwLock::new(None);
/// Язык интерфейса (`ru`, `en-US`…), пусто — не задан ([`set_language`]).
static LANG: RwLock<String> = RwLock::new(String::new());

/// Запомнить язык интерфейса программы: даты, числа, описания типов файлов
/// библиотек следуют ему, а не только LANG.
pub fn set_language(lang: &str) {
    if let Ok(mut l) = LANG.write() {
        *l = lang.trim().to_string();
    }
}

/// Язык интерфейса (`set_language`), иначе из окружения (`LC_ALL`,
/// `LC_MESSAGES`, `LANG`), в виде `ru_RU.UTF-8`/`en`.
pub fn language() -> String {
    let set = LANG.read().map(|l| l.clone()).unwrap_or_default();
    if !set.is_empty() {
        return set;
    }
    ["LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|k| std::env::var(k).ok()).find(|v| !v.is_empty()).unwrap_or_default()
}

/// Десятичный разделитель языка интерфейса: запятая у русского, немецкого,
/// французского…, точка у английского, китайского, японского, корейского.
pub fn decimal_separator() -> char {
    let lang = language();
    let base = lang.split(['_', '-', '.', '@']).next().unwrap_or("").to_ascii_lowercase();
    match base.as_str() {
        "" | "c" | "posix" | "en" | "zh" | "ja" | "ko" | "he" | "th" => '.',
        _ => ',',
    }
}

/// Число с `prec` знаками после разделителя языка интерфейса (`1,5` / `1.5`).
pub fn decimal(x: f64, prec: usize) -> String {
    let s = format!("{x:.prec$}");
    match decimal_separator() {
        '.' => s,
        c => s.replace('.', &c.to_string()),
    }
}

struct Own {
    /// Язык исходных строк — его не переводим.
    source_base: String,
    base: String,
    entries: HashMap<String, String>,
}

/// Перевод — переводчиком программы (syngui), см. `synshell_common::i18n`.
pub fn set_translator(t: Translate, tn: TranslateN) {
    let _ = HOOK.set((t, tn));
}

/// Свои каталоги для программы без syngui: язык `lang` (`ru_RU.UTF-8`,
/// `en`), исходники — на `source`; без каталога на язык — английский.
pub fn init(source: &str, lang: &str, catalogs: &[&str]) {
    set_language(lang);
    let base = lang.split(['_', '-', '.', '@']).next().unwrap_or("").to_ascii_lowercase();
    let source_base = source.split(['_', '-']).next().unwrap_or("").to_string();
    let mut by_lang: HashMap<String, HashMap<String, String>> = HashMap::new();
    for text in catalogs {
        let (tag, entries) = parse(text);
        by_lang.entry(tag).or_default().extend(entries);
    }
    let pick = if by_lang.contains_key(&base) || base == source_base { base.clone() } else { "en".into() };
    let entries = by_lang.remove(&pick).unwrap_or_default();
    if let Ok(mut own) = OWN.write() {
        *own = Some(Own { source_base, base: pick, entries });
    }
}

/// Каталог `.lang`: `@tag` и пары `"исходник" = "перевод"` (формы числа —
/// `"{n} файл".other`, ключ `исходник\u{1}форма`).
fn parse(text: &str) -> (String, HashMap<String, String>) {
    let mut tag = String::new();
    let mut map = HashMap::new();
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("@tag") {
            if let Some((_, v)) = rest.split_once('=') {
                tag = unquote(v.trim()).0.split(['-', '_']).next().unwrap_or("").to_string();
            }
            continue;
        }
        if !l.starts_with('"') {
            continue;
        }
        let (key, rest) = unquote(l);
        let (form, rest) = match rest.trim_start().strip_prefix('.') {
            Some(r) => {
                let end = r.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(r.len());
                (Some(&r[..end]), &r[end..])
            }
            None => (None, rest),
        };
        let Some(v) = rest.trim_start().strip_prefix('=') else { continue };
        let (value, _) = unquote(v.trim_start());
        if value.is_empty() {
            continue;
        }
        let key = match form {
            Some(f) => format!("{key}\u{1}{f}"),
            None => key,
        };
        map.insert(key, value);
    }
    (tag, map)
}

/// Строка в кавычках в начале `s` и остаток.
fn unquote(s: &str) -> (String, &str) {
    let mut out = String::new();
    let mut it = s.char_indices();
    if it.next().map(|c| c.1) != Some('"') {
        return (out, s);
    }
    while let Some((i, c)) = it.next() {
        match c {
            '\\' => match it.next().map(|c| c.1) {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => {}
            },
            '"' => return (out, &s[i + 1..]),
            c => out.push(c),
        }
    }
    (out, "")
}

/// Перевод исходной строки; нет перевода — она сама.
pub fn t(text: &str) -> String {
    if let Some((f, _)) = HOOK.get() {
        return f(text);
    }
    if let Ok(own) = OWN.read() {
        if let Some(o) = own.as_ref() {
            if o.base != o.source_base {
                if let Some(v) = o.entries.get(text) {
                    return v.clone();
                }
            }
        }
    }
    text.to_string()
}

pub fn t_args(text: &str, args: &[(&str, &dyn Display)]) -> String {
    substitute(&t(text), args)
}

/// Форма по числу: формы языка исходников по порядку (у русского — один,
/// несколько, много); `{n}` подставляется само.
pub fn tn_args(n: u64, forms: &[&str], args: &[(&str, &dyn Display)]) -> String {
    let template = match HOOK.get() {
        Some((_, f)) => f(n, forms),
        None => {
            let own = OWN.read().ok();
            let translated = own.as_ref().and_then(|o| o.as_ref()).filter(|o| o.base != o.source_base).and_then(|o| {
                let form = if n == 1 { "one" } else { "other" };
                o.entries.get(&format!("{}\u{1}{form}", forms.first().copied().unwrap_or(""))).cloned()
            });
            translated.unwrap_or_else(|| forms.get(ru_form(n)).or(forms.last()).copied().unwrap_or("").to_string())
        }
    };
    let mut all: Vec<(&str, &dyn Display)> = vec![("n", &n)];
    all.extend_from_slice(args);
    substitute(&template, &all)
}

/// Русская форма: 0 — «один», 1 — «несколько», 2 — «много».
fn ru_form(n: u64) -> usize {
    let (m10, m100) = (n % 10, n % 100);
    if m10 == 1 && m100 != 11 {
        0
    } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
        1
    } else {
        2
    }
}

/// `{имя}` → значение; неизвестные и одиночные скобки — как есть.
pub fn substitute(template: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let name = &after[..close];
                match args.iter().find(|(k, _)| *k == name) {
                    Some((_, v)) => out.push_str(&v.to_string()),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Перевод: `t!("Процессор")`, `t!("{v} ГБ", v = x)`.
#[macro_export]
macro_rules! t {
    ($text:expr $(,)?) => {
        $crate::t($text)
    };
    ($text:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::t_args($text, &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+])
    };
}

/// Форма по числу: `tn!(n, "{n} файл", "{n} файла", "{n} файлов")`.
#[macro_export]
macro_rules! tn {
    ($n:expr, $($form:expr),+ $(,)?) => {
        $crate::tn_args($n as u64, &[$($form),+], &[])
    };
    ($n:expr, $($form:expr),+ ; $($name:ident = $value:expr),+ $(,)?) => {
        $crate::tn_args($n as u64, &[$($form),+], &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+])
    };
}

/// Пометка строки для сборщика каталога; переводится при показе (`t(x)`).
#[macro_export]
macro_rules! n_ {
    ($text:literal) => {
        $text
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_catalog_and_forms() {
        init("ru", "en_US.UTF-8", &["@tag = \"en\"\n@name = \"English\"\n\"{v} ГБ\" = \"{v} GB\"\n\"{n} файл\".one = \"{n} file\"\n\"{n} файл\".other = \"{n} files\"\n\"Пусто\" = \"\"\n"]);
        assert_eq!(t!("{v} ГБ", v = 1.5), "1.5 GB");
        assert_eq!(t!("Пусто"), "Пусто");
        assert_eq!(tn!(3, "{n} файл", "{n} файла", "{n} файлов"), "3 files");
        init("ru", "ru_RU.UTF-8", &["@tag = \"en\"\n@name = \"English\"\n\"{v} ГБ\" = \"{v} GB\"\n"]);
        assert_eq!(t!("{v} ГБ", v = 2), "2 ГБ");
        assert_eq!(tn!(3, "{n} файл", "{n} файла", "{n} файлов"), "3 файла");
        assert_eq!(tn!(21, "{n} файл", "{n} файла", "{n} файлов"), "21 файл");
        assert_eq!(substitute("{a} {b} {", &[("a", &1)]), "1 {b} {");
        set_language("en_US.UTF-8");
        assert_eq!(decimal(1.25, 1), "1.2");
        set_language("ru");
        assert_eq!(decimal(2.0, 1), "2,0");
    }
}
