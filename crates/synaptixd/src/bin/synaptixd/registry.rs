use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use synaptixd::{DeviceKind, ModelKind, ModelSettings, Params};

#[derive(Deserialize, Clone, Debug)]
pub struct Variant {
    pub file: String,
    pub devices: Vec<DeviceKind>,
    #[serde(default)]
    pub precision: String,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Resources {
    #[serde(default)]
    pub ram: HashMap<DeviceKind, u64>,
    #[serde(default)]
    pub vram: HashMap<DeviceKind, u64>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Manifest {
    pub id: String,
    pub title: String,
    pub kind: ModelKind,
    pub arch: String,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(rename = "variant")]
    pub variants: Vec<Variant>,
    #[serde(default)]
    pub resources: Resources,
    #[serde(default)]
    pub defaults: Params,
    #[serde(default)]
    pub components: BTreeMap<String, String>,
    #[serde(skip)]
    pub dir: PathBuf,
    #[serde(skip)]
    pub library: Option<PathBuf>,
}

impl Manifest {
    /// Компоненты модели (роль → абсолютный путь).
    pub fn component_paths(&self) -> std::collections::BTreeMap<String, std::path::PathBuf> {
        self.components.iter().map(|(r, f)| (r.clone(), self.dir.join(f))).collect()
    }

    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(dir.join("model.toml"))?;
        let mut m: Manifest = toml::from_str(&text)?;
        if m.variants.is_empty() {
            anyhow::bail!("нет ни одного [[variant]]");
        }
        m.dir = dir.to_path_buf();
        Ok(m)
    }

    pub fn devices(&self) -> Vec<DeviceKind> {
        let mut v: Vec<DeviceKind> = Vec::new();
        for var in &self.variants {
            for d in &var.devices {
                if !v.contains(d) {
                    v.push(*d);
                }
            }
        }
        v
    }

    pub fn variant_for(&self, dev: DeviceKind) -> Option<&Variant> {
        self.variants.iter().find(|v| v.devices.contains(&dev))
    }

    pub fn variant_quant(&self, dev: DeviceKind, quant: Option<&str>) -> Option<&Variant> {
        let by_quant = quant.filter(|q| self.kind != ModelKind::Llm && *q != "auto");
        by_quant
            .and_then(|q| self.variants.iter().find(|v| v.devices.contains(&dev) && v.precision == q))
            .or_else(|| self.variant_for(dev))
    }

    pub fn path(&self, v: &Variant) -> PathBuf {
        self.dir.join(&v.file)
    }

    pub fn disk_bytes(&self) -> u64 {
        let mut seen: Vec<PathBuf> = Vec::new();
        let mut total = 0;
        let files = self.variants.iter().map(|v| self.path(v)).chain(self.components.values().map(|f| self.dir.join(f)));
        for p in files {
            let canon = std::fs::canonicalize(&p).unwrap_or(p.clone());
            if seen.contains(&canon) {
                continue;
            }
            total += std::fs::metadata(&canon).map(|m| m.len()).unwrap_or(0);
            seen.push(canon);
        }
        total
    }

    pub fn ram_estimate(&self, dev: DeviceKind, quant: Option<&str>) -> u64 {
        if let Some(mb) = self.resources.ram.get(&dev) {
            return mb << 20;
        }
        let variant = self.variant_quant(dev, quant);
        let file = variant.and_then(|v| std::fs::metadata(self.path(v)).ok()).map(|m| m.len()).unwrap_or(0);
        let int8 = variant.is_some_and(|v| v.precision.starts_with("w8"));
        match dev {
            DeviceKind::Cpu if int8 => file * 55 / 100,
            DeviceKind::Cpu => file * 2,
            DeviceKind::Cuda | DeviceKind::Gpu => file / 4,
            _ => file,
        }
    }

    /// Оценка видеопамяти: из манифеста, иначе размер файла весов с поправкой
    /// на квантование при загрузке (файл обычно в BF16/F16: NVFP4 — около
    /// трети, int8/MXFP8 — чуть больше половины, F32 — вдвое больше).
    pub fn vram_estimate(&self, dev: DeviceKind, quant: Option<&str>) -> u64 {
        if let Some(mb) = self.resources.vram.get(&dev) {
            return mb << 20;
        }
        let file = self.variant_quant(dev, quant).and_then(|v| std::fs::metadata(self.path(v)).ok()).map(|m| m.len()).unwrap_or(0);
        match quant {
            Some("nvfp4") => file * 30 / 100,
            Some("int8") | Some("mxfp8") => file * 55 / 100,
            Some("f32") => file * 2,
            _ => file,
        }
    }

    pub fn npu_cache_dir(&self) -> PathBuf {
        PathBuf::from(crate::config::NPU_CACHE_RO).join(&self.id)
    }
}

pub fn scan(dirs: &[(PathBuf, Option<PathBuf>)]) -> BTreeMap<String, Manifest> {
    let mut out = BTreeMap::new();
    for (root, library) in dirs {
        let Ok(rd) = std::fs::read_dir(root) else { continue };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.join("model.toml").is_file()).collect();
        entries.sort();
        for dir in entries {
            match Manifest::load(&dir) {
                Ok(mut m) => {
                    m.library = library.clone();
                    if !out.contains_key(&m.id) {
                        out.insert(m.id.clone(), m);
                    }
                }
                Err(e) => tracing::warn!("{}: {e}", dir.display()),
            }
        }
    }
    out
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct StateFile {
    #[serde(default)]
    pub models: BTreeMap<String, ModelSettings>,
    #[serde(default)]
    pub defaults: BTreeMap<String, String>,
    #[serde(default)]
    pub libraries: Vec<LibraryConf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_library: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LibraryConf {
    pub path: String,
    #[serde(default)]
    pub name: String,
}

impl StateFile {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}
