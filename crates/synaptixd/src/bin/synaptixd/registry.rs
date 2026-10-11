use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use synaptixd::{DeviceKind, ModelKind, ModelSettings};

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
    #[serde(skip)]
    pub dir: PathBuf,
}

impl Manifest {
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(dir.join("model.toml"))?;
        let mut m: Manifest = toml::from_str(&text)?;
        if m.variants.is_empty() {
            anyhow::bail!("нет ни одного [[variant]]");
        }
        m.dir = dir.to_path_buf();
        Ok(m)
    }

    pub fn package(&self) -> String {
        self.package.clone().unwrap_or_else(|| format!("synaptix-model-{}", self.id))
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

    pub fn path(&self, v: &Variant) -> PathBuf {
        self.dir.join(&v.file)
    }

    pub fn disk_bytes(&self) -> u64 {
        let mut seen = Vec::new();
        let mut total = 0;
        for v in &self.variants {
            if seen.contains(&v.file) {
                continue;
            }
            seen.push(v.file.clone());
            total += std::fs::metadata(self.path(v)).map(|m| m.len()).unwrap_or(0);
        }
        total
    }

    pub fn ram_estimate(&self, dev: DeviceKind) -> u64 {
        if let Some(mb) = self.resources.ram.get(&dev) {
            return mb << 20;
        }
        let file = self.variant_for(dev).and_then(|v| std::fs::metadata(self.path(v)).ok()).map(|m| m.len()).unwrap_or(0);
        match dev {
            DeviceKind::Cpu => file * 2,
            DeviceKind::Cuda | DeviceKind::Gpu => file / 4,
            _ => file,
        }
    }

    pub fn npu_cache_dir(&self) -> PathBuf {
        PathBuf::from(crate::config::NPU_CACHE_RO).join(&self.id)
    }
}

pub fn scan(dirs: &[PathBuf]) -> BTreeMap<String, Manifest> {
    let mut out = BTreeMap::new();
    for root in dirs {
        let Ok(rd) = std::fs::read_dir(root) else { continue };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.join("model.toml").is_file()).collect();
        entries.sort();
        for dir in entries {
            match Manifest::load(&dir) {
                Ok(m) => {
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
