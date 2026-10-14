//! Установка пакетов в общий каталог: каждый источник скачивается и обходится один раз,
//! DLL берутся по PE (тип машины + имя в таблице экспорта) или по явному пути из базы.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use syngui::t;

use crate::db::Db;
use crate::pe::Arch;
use crate::unpack::{self, Flow};

#[derive(Debug, Clone)]
pub struct Progress {
    /// Этап без чисел (для вывода в терминал — по одной строке на этап).
    pub stage: String,
    pub text: String,
    pub frac: Option<f32>,
}

/// Что нужно взять из одного источника.
struct Want {
    arch: Arch,
    dll: String,
    /// Шаблон пути внутри контейнера (если DLL лежит под другим именем).
    pattern: Option<String>,
}

/// Ставит пакеты (id, архитектура); возвращает записанные файлы.
pub fn install(db: &Db, items: &[(String, Arch)], report: &mut dyn FnMut(Progress)) -> Result<Vec<PathBuf>> {
    let mut by_source: BTreeMap<String, Vec<Want>> = BTreeMap::new();
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    for (id, arch) in items {
        let pkg = db.package(id).with_context(|| t!("нет пакета «{id}»", id = id))?;
        let pick = pkg.pick(*arch).with_context(|| t!("у пакета «{id}» нет варианта {arch}", id = id, arch = arch.name()))?;
        db.source(&pick.source).with_context(|| t!("в базе нет источника «{s}»", s = &pick.source))?;
        names.entry(pick.source.clone()).or_insert_with(|| pkg.name.clone());
        let wants = by_source.entry(pick.source.clone()).or_default();
        for dll in &pkg.dlls {
            if !wants.iter().any(|w| w.arch == *arch && w.dll.eq_ignore_ascii_case(dll)) {
                wants.push(Want { arch: *arch, dll: dll.to_ascii_lowercase(), pattern: pick.files.get(dll).cloned() });
            }
        }
    }
    let mut written = Vec::new();
    let count = by_source.len();
    for (n, (sid, wants)) in by_source.into_iter().enumerate() {
        let src = db.source(&sid).unwrap();
        let title = names.get(&sid).cloned().unwrap_or_default();
        let step = if count > 1 { format!(" ({}/{count})", n + 1) } else { String::new() };
        let mb = |b: u64| synshell_common::decimal(b as f64 / 1048576.0, 1);
        let stage = t!("{name}: скачивание {file}{step}", name = &title, file = &src.file, step = &step);
        report(Progress { stage: stage.clone(), text: stage.clone(), frac: Some(0.0) });
        let path = crate::fetch::fetch(src, &mut |got, total| {
            let frac = if total > 0 { got as f32 / total as f32 } else { 0.0 };
            report(Progress { stage: stage.clone(), text: t!("{name}: скачивание {got} из {total} МБ{step}", name = &title, got = mb(got), total = mb(total), step = &step), frac: Some(frac) });
        })
        .with_context(|| t!("не удалось скачать {file}", file = &src.file))?;
        let stage = t!("{name}: распаковка{step}", name = &title, step = &step);
        report(Progress { stage: stage.clone(), text: stage, frac: None });
        let data = std::fs::read(&path)?;
        let found = collect(&src.file, &data, &wants);
        for w in &wants {
            let Some(bytes) = found.get(&(w.arch, w.dll.clone())) else {
                bail!("{}", t!("в {file} нет {dll} ({arch})", file = &src.file, dll = &w.dll, arch = w.arch.name()));
            };
            written.push(put(w.arch, &w.dll, bytes)?);
        }
    }
    Ok(written)
}

fn collect(name: &str, data: &[u8], wants: &[Want]) -> BTreeMap<(Arch, String), Vec<u8>> {
    let mut found: BTreeMap<(Arch, String), Vec<u8>> = BTreeMap::new();
    unpack::walk(name, data, &mut |path, blob| {
        let member = path.rsplit('!').next().unwrap_or(path);
        let info = crate::pe::parse(blob);
        for w in wants {
            let key = (w.arch, w.dll.clone());
            if found.contains_key(&key) {
                continue;
            }
            let Some(info) = &info else { continue };
            if info.arch() != Some(w.arch) || !info.is_dll {
                continue;
            }
            let hit = match &w.pattern {
                Some(p) => glob(p, member),
                None => info.export_name.as_deref() == Some(w.dll.as_str()),
            };
            if hit {
                tracing::debug!("{} ({}) <- {path}", w.dll, w.arch.name());
                found.insert(key, blob.to_vec());
            }
        }
        if found.len() == wants.len() {
            Flow::Stop
        } else {
            Flow::Continue
        }
    });
    found
}

fn put(arch: Arch, dll: &str, bytes: &[u8]) -> Result<PathBuf> {
    let dir = crate::store::arch_dir(arch);
    std::fs::create_dir_all(&dir).with_context(|| dir.display().to_string())?;
    let dst = dir.join(dll);
    let tmp = dir.join(format!(".{dll}.part"));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &dst)?;
    Ok(dst)
}

/// Удаляет DLL пакета из общего каталога.
pub fn remove(db: &Db, id: &str) -> Result<usize> {
    let pkg = db.package(id).with_context(|| t!("нет пакета «{id}»", id = id))?;
    let mut n = 0;
    for arch in Arch::ALL {
        for d in &pkg.dlls {
            if let Some(p) = crate::store::find_dll(arch, d) {
                std::fs::remove_file(&p)?;
                n += 1;
            }
        }
    }
    Ok(n)
}

/// Шаблон с `*` (любые символы, в том числе `/`), без учёта регистра.
pub fn glob(pattern: &str, s: &str) -> bool {
    let p = pattern.to_ascii_lowercase();
    let s = s.to_ascii_lowercase();
    let parts: Vec<&str> = p.split('*').collect();
    if parts.len() == 1 {
        return p == s;
    }
    let mut rest = s.as_str();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            if !rest.starts_with(part) {
                return false;
            }
            rest = &rest[part.len()..];
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else if let Some(at) = rest.find(part) {
            rest = &rest[at + part.len()..];
        } else {
            return false;
        }
    }
    true
}
