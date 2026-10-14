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

    /// Пакет с этой DLL для архитектуры.
    pub fn for_dll(&self, dll: &str, arch: Arch) -> Option<&Package> {
        self.packages.iter().find(|p| p.provides(dll) && p.pick(arch).is_some())
    }
}
