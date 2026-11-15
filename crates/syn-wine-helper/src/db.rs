//! База пакетов: встроенная `data/packages.toml` и свои пакеты пользователя
//! (`$XDG_DATA_HOME/syn-wine/packages.toml`, дополняют и заменяют встроенные по id).

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::pe::Arch;

#[derive(Debug, Clone, Deserialize)]
pub struct Source {
    pub id: String,
    pub file: String,
    pub size: u64,
    pub sha256: String,
    pub urls: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pick {
    pub source: String,
    /// DLL → путь в контейнере (шаблон с `*`), если её нельзя узнать по имени в таблице экспорта.
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Package {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub dlls: Vec<String>,
    /// Пакеты, без которых эти DLL не загрузятся (их импорты); ставятся вместе с этим —
    /// для архитектуры, которая у них есть.
    #[serde(default)]
    pub requires: Vec<String>,
    pub x86: Option<Pick>,
    pub x64: Option<Pick>,
}

impl Package {
    pub fn pick(&self, arch: Arch) -> Option<&Pick> {
        match arch {
            Arch::X86 => self.x86.as_ref(),
            Arch::X64 => self.x64.as_ref(),
        }
    }

    pub fn archs(&self) -> Vec<Arch> {
        Arch::ALL.into_iter().filter(|a| self.pick(*a).is_some()).collect()
    }

    /// Название на языке интерфейса (у встроенных пакетов — из каталогов переводов).
    pub fn title(&self) -> String {
        syngui::i18n::t(&self.name)
    }

    /// Описание на языке интерфейса.
    pub fn about(&self) -> String {
        syngui::i18n::t(&self.description)
    }

    pub fn provides(&self, dll: &str) -> bool {
        self.dlls.iter().any(|d| d.eq_ignore_ascii_case(dll))
    }
}

#[derive(Debug, Default, Deserialize)]
struct File {
    #[serde(default)]
    source: Vec<Source>,
    #[serde(default)]
    package: Vec<Package>,
}

#[derive(Debug, Default, Clone)]
pub struct Db {
    pub sources: Vec<Source>,
    pub packages: Vec<Package>,
}

impl Db {
    pub fn load() -> Db {
        let mut db = Db::default();
        db.merge(toml::from_str(include_str!("../data/packages.toml")).expect("data/packages.toml"));
        let user = crate::store::base_dir().join("packages.toml");
        if let Ok(s) = std::fs::read_to_string(&user) {
            match toml::from_str::<File>(&s) {
                Ok(f) => db.merge(f),
                Err(e) => tracing::warn!("{}: {e}", user.display()),
            }
        }
        db
    }

    fn merge(&mut self, f: File) {
        for s in f.source {
            self.sources.retain(|x| x.id != s.id);
            self.sources.push(s);
        }
        for p in f.package {
            self.packages.retain(|x| x.id != p.id);
            self.packages.push(p);
        }
    }

    pub fn source(&self, id: &str) -> Option<&Source> {
        self.sources.iter().find(|s| s.id == id)
    }

    pub fn package(&self, id: &str) -> Option<&Package> {
        self.packages.iter().find(|p| p.id.eq_ignore_ascii_case(id))
    }

    /// Пакеты вместе с зависимостями (`requires`, рекурсивно), без повторов.
    pub fn with_requires(&self, items: &[(String, Arch)]) -> Vec<(String, Arch)> {
        let mut out: Vec<(String, Arch)> = Vec::new();
        let mut queue: Vec<(String, Arch)> = items.iter().rev().cloned().collect();
        while let Some((id, arch)) = queue.pop() {
            if out.iter().any(|(i, a)| i.eq_ignore_ascii_case(&id) && *a == arch) {
                continue;
            }
            if let Some(p) = self.package(&id) {
                for r in p.requires.iter().rev() {
                    if self.package(r).is_some_and(|rp| rp.pick(arch).is_some()) {
                        queue.push((r.clone(), arch));
                    }
                }
            }
            out.push((id, arch));
        }
        out
    }

    /// Пакет с этой DLL для архитектуры.
    pub fn for_dll(&self, dll: &str, arch: Arch) -> Option<&Package> {
        self.packages.iter().find(|p| p.provides(dll) && p.pick(arch).is_some())
    }
}

/// Строки встроенной `data/packages.toml` для каталогов переводов (`n_!` собирает i18n-extract).
#[allow(dead_code)]
const PACKAGE_STRINGS: &[&str] = &[
    syngui::n_!("Microsoft Foundation Classes из Visual C++ 6.0 — программы и игры 1998–2005 годов"),
    syngui::n_!("Microsoft Foundation Classes из Visual Studio .NET 2002"),
    syngui::n_!("Active Template Library из Visual Studio .NET 2002"),
    syngui::n_!("Microsoft Foundation Classes из Visual Studio .NET 2003"),
    syngui::n_!("Microsoft Foundation Classes из Visual C++ 2005"),
    syngui::n_!("Microsoft Foundation Classes из Visual C++ 2008"),
    syngui::n_!("Microsoft Foundation Classes из Visual C++ 2010"),
    syngui::n_!("Microsoft Foundation Classes из Visual C++ 2012"),
    syngui::n_!("Microsoft Foundation Classes из Visual C++ 2013"),
    syngui::n_!("Microsoft Foundation Classes из Visual C++ 2015–2022"),
    syngui::n_!("Потоки C++ 2022"),
    syngui::n_!("vcruntime140_threads.dll из Visual C++ 2022 (17.10+): у Wine её нет"),
    syngui::n_!("Среда выполнения Visual Basic 5.0 (msvbvm50.dll)"),
    syngui::n_!("Среда выполнения Visual Basic 6.0 SP6 (msvbvm60.dll)"),
    syngui::n_!("Рантайм C++ MinGW"),
    syngui::n_!("Стандартная библиотека C++ GCC (libstdc++-6.dll) для программ, собранных MinGW: обычно её кладут рядом с программой"),
    syngui::n_!("Рантайм GCC MinGW"),
    syngui::n_!("Поддержка исключений и арифметики GCC (libgcc_s) для программ, собранных MinGW"),
    syngui::n_!("Потоки POSIX MinGW"),
    syngui::n_!("Потоки POSIX для Windows (libwinpthread-1.dll) — нужны программам и библиотекам MinGW"),
    syngui::n_!("Звук OpenAL для игр — открытая реализация OpenAL Soft вместо установщика Creative"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_follow_arch() {
        let db = Db::load();
        let ids = |a| db.with_requires(&[("libstdc++".into(), a)]).into_iter().map(|(i, _)| i).collect::<Vec<_>>();
        assert_eq!(ids(Arch::X64), ["libstdc++", "libgcc-seh", "libwinpthread"]);
        assert_eq!(ids(Arch::X86), ["libstdc++", "libgcc-dw2", "libwinpthread"]);
        assert_eq!(db.for_dll("libgcc_s_seh-1.dll", Arch::X64).map(|p| p.id.as_str()), Some("libgcc-seh"));
        assert!(db.for_dll("libgcc_s_seh-1.dll", Arch::X86).is_none());
    }
}
