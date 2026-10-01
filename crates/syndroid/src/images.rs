//! Образы Android: наборы `system.img` + `vendor.img` в `/var/lib/syndroid/images/<имя>/`.
//!
//! Источник — OTA-каналы Waydroid: `<system_channel>/<rom_type>/waydroid_<arch>/<system_type>.json` и
//! `<vendor_channel>/waydroid_<arch>/<vendor_type>.json`; ответ — `{"response": [{datetime, filename, id, romtype,
//! size, url, version}, …]}`, `id` — sha256 zip-архива, внутри которого `system.img`/`vendor.img`.

use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OtaEntry {
    #[serde(default)]
    pub datetime: i64,
    pub filename: String,
    pub id: String,
    #[serde(default)]
    pub romtype: String,
    #[serde(default)]
    pub size: u64,
    pub url: String,
    #[serde(default)]
    pub version: String,
}

#[derive(Deserialize)]
struct OtaResponse {
    response: Vec<OtaEntry>,
}

/// Откуда взят образ.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Origin {
    Ota(OtaEntry),
    Local { file: String },
}

/// `images/<имя>/info.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageSet {
    pub name: String,
    pub system: Origin,
    pub vendor: Origin,
    /// Unix-время установки.
    pub installed: i64,
    /// Размер обоих образов, байт.
    #[serde(default)]
    pub size: u64,
}

/// Ход длинной операции: (что делается, сделано, всего) — байты.
pub type Progress<'a> = &'a (dyn Fn(&str, u64, u64) + Sync);

pub fn system_ota_url(c: &Config) -> String {
    format!("{}/{}/waydroid_{}/{}.json", c.system_channel, c.rom_type, c.arch, c.system_type)
}
pub fn vendor_ota_url(c: &Config) -> String {
    format!("{}/waydroid_{}/{}.json", c.vendor_channel, c.arch, c.vendor_type)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(concat!("syndroid/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Последние сборки каналов (system, vendor).
pub fn ota_latest(c: &Config) -> Result<(OtaEntry, OtaEntry)> {
    let get = |url: &str| -> Result<OtaEntry> {
        let mut r = agent().get(url).call().with_context(|| format!("OTA {url}"))?;
        let body: OtaResponse = serde_json::from_reader(r.body_mut().as_reader())?;
        body.response.into_iter().max_by_key(|e| e.datetime).with_context(|| format!("пустой канал {url}"))
    };
    Ok((get(&system_ota_url(c))?, get(&vendor_ota_url(c))?))
}

/// Имя набора по имени system-архива: `lineage-20.0-20260927-VANILLA-waydroid_arm64-system.zip` →
/// `lineage-20.0-20260927-VANILLA`.
pub fn set_name(system_file: &str) -> String {
    let base = Path::new(system_file).file_name().and_then(|s| s.to_str()).unwrap_or(system_file);
    let base = base.trim_end_matches(".zip").trim_end_matches(".img");
    let base = base.trim_end_matches("-system");
    match base.rfind("-waydroid_") {
        Some(i) => base[..i].to_string(),
        None => base.to_string(),
    }
}

/// Экземпляр Android, которому принадлежит набор: имя без даты сборки (`lineage-20.0-20260927-VANILLA` →
/// `lineage-20.0-VANILLA`). Обновления одной линейки — тот же экземпляр (те же данные и приложения), разные
/// версии и VANILLA/GAPPS — разные.
pub fn instance_of(set: &str) -> String {
    let parts: Vec<&str> = set.split('-').filter(|p| !(p.len() == 8 && p.chars().all(|c| c.is_ascii_digit()))).collect();
    parts.join("-")
}

/// Название экземпляра для людей: «LineageOS 20.0», «LineageOS 20.0 · GApps».
pub fn instance_title(instance: &str) -> String {
    let mut it = instance.split('-');
    match (it.next(), it.next()) {
        (Some("lineage"), Some(ver)) => {
            let gapps = it.any(|r| r.eq_ignore_ascii_case("GAPPS"));
            format!("LineageOS {ver}{}", if gapps { " · GApps" } else { "" })
        }
        _ => instance.to_string(),
    }
}

/// Самый свежий установленный набор экземпляра.
pub fn latest_of(instance: &str) -> Option<ImageSet> {
    list().into_iter().filter(|s| instance_of(&s.name) == instance).max_by_key(|s| s.installed)
}

pub fn list() -> Vec<ImageSet> {
    let mut v: Vec<ImageSet> = fs::read_dir(paths::images())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| fs::read_to_string(e.path().join("info.toml")).ok())
        .filter_map(|s| toml::from_str(&s).ok())
        .collect();
    v.sort_by_key(|s| std::cmp::Reverse(s.installed));
    v
}

pub fn dir(name: &str) -> PathBuf {
    paths::images().join(name)
}

pub fn exists(name: &str) -> bool {
    dir(name).join("system.img").is_file() && dir(name).join("vendor.img").is_file()
}

fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name.starts_with('.') || name.contains('/') {
        bail!("недопустимое имя набора «{name}»");
    }
    Ok(())
}

pub fn remove(name: &str) -> Result<()> {
    check_name(name)?;
    fs::remove_dir_all(dir(name)).with_context(|| format!("набор {name}"))?;
    let _ = fs::remove_dir_all(paths::STATE.to_string() + "/overlay_rw/" + name);
    let _ = fs::remove_dir_all(paths::STATE.to_string() + "/overlay_work/" + name);
    Ok(())
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Скачать URL в файл, считая sha256; `expect` — ожидаемая сумма (hex).
fn download(url: &str, to: &Path, expect: &str, what: &str, progress: Progress) -> Result<()> {
    let mut r = agent().get(url).call().with_context(|| format!("загрузка {url}"))?;
    let total = r
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut src = r.body_mut().as_reader();
    let mut out = File::create(to)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        out.write_all(&buf[..n])?;
        done += n as u64;
        progress(what, done, total);
    }
    out.sync_all()?;
    let got = format!("{:x}", hash.finalize());
    if !expect.is_empty() && !got.eq_ignore_ascii_case(expect) {
        bail!("{what}: sha256 {got} ≠ {expect}");
    }
    Ok(())
}

/// Достать `<part>.img` из zip (или скопировать, если это уже .img).
fn unpack(src: &Path, part: &str, to: &Path, progress: Progress) -> Result<()> {
    let what = format!("распаковка {part}.img");
    let mut f = File::open(src)?;
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic)?;
    drop(f);
    let tmp = to.with_extension("img.part");
    if &magic == b"PK\x03\x04" {
        let mut z = zip::ZipArchive::new(BufReader::new(File::open(src)?))?;
        let idx = (0..z.len())
            .find(|&i| z.by_index(i).map(|e| e.name().ends_with(&format!("{part}.img"))).unwrap_or(false))
            .with_context(|| format!("в {} нет {part}.img", src.display()))?;
        let mut e = z.by_index(idx)?;
        copy(&mut e, &tmp, &what, progress)?;
    } else {
        copy(&mut File::open(src)?, &tmp, &what, progress)?;
    }
    fs::rename(&tmp, to)?;
    Ok(())
}

fn copy(src: &mut dyn Read, to: &Path, what: &str, progress: Progress) -> Result<()> {
    let mut out = File::create(to)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        done += n as u64;
        progress(what, done, 0);
    }
    out.sync_all()?;
    Ok(())
}

fn finish(name: &str, staging: &Path, system: Origin, vendor: Origin) -> Result<ImageSet> {
    let size = ["system.img", "vendor.img"]
        .iter()
        .filter_map(|f| fs::metadata(staging.join(f)).ok())
        .map(|m| m.len())
        .sum();
    let set = ImageSet { name: name.to_string(), system, vendor, installed: now(), size };
    fs::write(staging.join("info.toml"), toml::to_string_pretty(&set)?)?;
    let target = dir(name);
    if target.exists() {
        fs::remove_dir_all(&target)?;
    }
    fs::rename(staging, &target)?;
    Ok(set)
}

fn staging(name: &str) -> Result<PathBuf> {
    check_name(name)?;
    let s = paths::images().join(format!(".{name}.part"));
    let _ = fs::remove_dir_all(&s);
    fs::create_dir_all(&s)?;
    Ok(s)
}

/// Скачать набор из OTA (последние сборки каналов, если `entries` не заданы).
pub fn fetch(c: &Config, entries: Option<(OtaEntry, OtaEntry)>, progress: Progress) -> Result<ImageSet> {
    progress("запрос OTA-каналов", 0, 0);
    let (sys, ven) = match entries {
        Some(e) => e,
        None => ota_latest(c)?,
    };
    let name = set_name(&sys.filename);
    let st = staging(&name)?;
    let r = (|| {
        for (e, part) in [(&sys, "system"), (&ven, "vendor")] {
            let zip = st.join(&e.filename);
            download(&e.url, &zip, &e.id, &format!("загрузка {part}"), progress)?;
            unpack(&zip, part, &st.join(format!("{part}.img")), progress)?;
            fs::remove_file(&zip)?;
        }
        finish(&name, &st, Origin::Ota(sys.clone()), Origin::Ota(ven.clone()))
    })();
    if r.is_err() {
        let _ = fs::remove_dir_all(&st);
    }
    r
}

/// Набор из локальных файлов: zip из OTA или голые .img.
pub fn import(system: &Path, vendor: &Path, name: Option<&str>, progress: Progress) -> Result<ImageSet> {
    let name = name.map(str::to_string).unwrap_or_else(|| set_name(&system.to_string_lossy()));
    let st = staging(&name)?;
    let r = (|| {
        unpack(system, "system", &st.join("system.img"), progress)?;
        unpack(vendor, "vendor", &st.join("vendor.img"), progress)?;
        finish(
            &name,
            &st,
            Origin::Local { file: system.display().to_string() },
            Origin::Local { file: vendor.display().to_string() },
        )
    })();
    if r.is_err() {
        let _ = fs::remove_dir_all(&st);
    }
    r
}
