//! Адаптеры LoRA и датасеты в библиотеках моделей.
//!
//! ```text
//! <библиотека>/lora/<имя>/adapter_model.safetensors   адаптер PEFT
//!                         adapter_config.json
//!                         synaptix_train.json         сведения обучения (служба)
//! <библиотека>/datasets/<имя>.jsonl                   датасет (или .json)
//! ```

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::Value;
use synaptixd::{DatasetEntry, LoraEntry, LoraUse};

pub const LORA_DIR: &str = "lora";
pub const DATASET_DIR: &str = "datasets";
pub const TRAIN_META: &str = "synaptix_train.json";

fn modified_ms(p: &Path) -> i64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn dir_bytes(p: &Path) -> u64 {
    std::fs::read_dir(p).map(|rd| rd.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum()).unwrap_or(0)
}

/// Адаптер из каталога (`None` — не адаптер PEFT).
pub fn read_lora(dir: &Path, library: Option<&Path>) -> Option<LoraEntry> {
    let cfg: Value = serde_json::from_slice(&std::fs::read(dir.join("adapter_config.json")).ok()?).ok()?;
    if !dir.join("adapter_model.safetensors").is_file() {
        return None;
    }
    let rank = cfg.get("r").and_then(Value::as_u64).unwrap_or(0) as usize;
    let alpha = cfg.get("lora_alpha").and_then(Value::as_f64).unwrap_or(rank as f64) as f32;
    let targets = match cfg.get("target_modules") {
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    };
    let train: Value = std::fs::read(dir.join(TRAIN_META)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    let base_path = train
        .get("base_path")
        .and_then(Value::as_str)
        .map(String::from)
        .or_else(|| cfg.get("base_model_name_or_path").and_then(Value::as_str).map(String::from))
        .unwrap_or_default();
    Some(LoraEntry {
        name: dir.file_name()?.to_string_lossy().into_owned(),
        path: dir.to_string_lossy().into_owned(),
        library: library.map(|l| l.to_string_lossy().into_owned()),
        base: train.get("base").and_then(Value::as_str).map(String::from),
        base_path,
        rank,
        alpha,
        targets,
        bytes: dir_bytes(dir),
        modified_ms: modified_ms(&dir.join("adapter_model.safetensors")),
        train,
    })
}

/// Адаптеры всех библиотек (имя — уникально; первое найденное).
pub fn scan_loras(libs: &[PathBuf]) -> Vec<LoraEntry> {
    let mut out: Vec<LoraEntry> = Vec::new();
    for lib in libs {
        let Ok(rd) = std::fs::read_dir(lib.join(LORA_DIR)) else { continue };
        let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for d in dirs {
            let name = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if name.starts_with('.') || name.starts_with('_') || out.iter().any(|l| l.name == name) {
                continue;
            }
            if let Some(e) = read_lora(&d, Some(lib)) {
                out.push(e);
            }
        }
    }
    out
}

pub fn find_lora(libs: &[PathBuf], name: &str) -> Option<LoraEntry> {
    scan_loras(libs).into_iter().find(|l| l.name == name)
}

/// Число примеров: строки JSONL или длина JSON-массива.
fn count_rows(p: &Path) -> usize {
    let Ok(text) = std::fs::read_to_string(p) else { return 0 };
    let t = text.trim_start();
    if t.starts_with('[') {
        return serde_json::from_str::<Value>(t).ok().and_then(|v| v.as_array().map(Vec::len)).unwrap_or(0);
    }
    text.lines().filter(|l| !l.trim().is_empty()).count()
}

pub fn scan_datasets(libs: &[PathBuf]) -> Vec<DatasetEntry> {
    let mut out = Vec::new();
    for lib in libs {
        let Ok(rd) = std::fs::read_dir(lib.join(DATASET_DIR)) else { continue };
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "jsonl" || e == "json"))
            .collect();
        files.sort();
        for f in files {
            let name = f.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if name.starts_with('.') || name.starts_with('_') {
                continue;
            }
            out.push(DatasetEntry {
                name,
                path: f.to_string_lossy().into_owned(),
                library: Some(lib.to_string_lossy().into_owned()),
                bytes: std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0),
                rows: count_rows(&f),
                modified_ms: modified_ms(&f),
            });
        }
    }
    out
}

/// Имя адаптера годится для каталога.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 120 && !name.starts_with('.') && !name.starts_with('_') && !name.contains('/') && !name.contains('\0')
}

/// Набор адаптеров к загрузке: `(имя, каталог, включён, сила)`.
pub type Resolved = Vec<(LoraUse, PathBuf)>;

/// Найти каталоги адаптеров набора (включённых; выключенные не нужны).
pub fn resolve(libs: &[PathBuf], want: &[LoraUse]) -> Result<Resolved, String> {
    let all = scan_loras(libs);
    let mut out = Vec::new();
    for u in want.iter().filter(|u| u.enabled && u.strength != 0.0) {
        let e = all.iter().find(|l| l.name == u.name).ok_or_else(|| format!("адаптер LoRA «{}» не найден в библиотеках", u.name))?;
        out.push((u.clone(), PathBuf::from(&e.path)));
    }
    Ok(out)
}
