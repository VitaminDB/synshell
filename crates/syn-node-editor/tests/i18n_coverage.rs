//! Каждый ключ `tr!`/`trn!`/`try_tr` в исходниках крейта есть в каталогах
//! `i18n/en.lang` и `i18n/ru.lang` (иначе `tr!` молча показывает сам ключ), и
//! все 14 каталогов разбираются.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Ключи вызовов `marker"…"` (с пробелами и переводом строки после скобки).
fn used_keys() -> BTreeSet<String> {
    let mut files = Vec::new();
    rust_files(&root().join("src"), &mut files);
    let mut keys = BTreeSet::new();
    for f in files {
        let src = std::fs::read_to_string(&f).unwrap();
        for marker in ["tr!(", "trn!(", "try_tr("] {
            let mut rest = src.as_str();
            while let Some(pos) = rest.find(marker) {
                let ok = pos == 0 || !rest.as_bytes()[pos - 1].is_ascii_alphanumeric();
                let after = rest[pos + marker.len()..].trim_start();
                rest = &rest[pos + marker.len()..];
                if !ok {
                    continue;
                }
                if let Some(lit) = after.strip_prefix('"') {
                    if let Some(end) = lit.find('"') {
                        keys.insert(lit[..end].to_string());
                    }
                }
            }
        }
    }
    keys
}

fn catalog_keys(lang: &str) -> BTreeSet<String> {
    let text = std::fs::read_to_string(root().join("i18n").join(format!("{lang}.lang"))).unwrap();
    text.lines()
        .filter_map(|l| l.split_once(" = ").map(|(k, _)| k.trim()))
        .filter(|k| !k.starts_with('@') && !k.starts_with('#'))
        .map(|k| {
            for form in [".zero", ".one", ".two", ".few", ".many", ".other"] {
                if let Some(base) = k.strip_suffix(form) {
                    return base.to_string();
                }
            }
            k.to_string()
        })
        .collect()
}

#[test]
fn every_key_is_in_en_and_ru() {
    let used = used_keys();
    assert!(used.len() > 50, "ключи не нашлись: {}", used.len());
    for lang in ["en", "ru"] {
        let have = catalog_keys(lang);
        let missing: Vec<_> = used.iter().filter(|k| !have.contains(*k)).collect();
        assert!(missing.is_empty(), "{lang}.lang: нет ключей {missing:?}");
    }
}

#[test]
fn catalogs_parse() {
    for text in syn_node_editor::i18n::CATALOGS {
        let c = syngui::i18n::Catalog::parse(text).expect("каталог разбирается");
        assert!(c.len() > 100, "{}: {} строк", c.name, c.len());
    }
}
