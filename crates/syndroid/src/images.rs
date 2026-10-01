//! Образы Android: наборы `system.img` + `vendor.img` в `/var/lib/syndroid/images/<имя>/`.
//!
//! Источник — OTA-каналы Waydroid: `<system_channel>/<rom_type>/waydroid_<arch>/<system_type>.json` и
//! `<vendor_channel>/waydroid_<arch>/<vendor_type>.json`; ответ — `{"response": [{datetime, filename, id, romtype,
//! size, url, version}, …]}`, `id` — sha256 zip-архива, внутри которого `system.img`/`vendor.img`.

use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

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
/// Операцию отменили — прервать.
pub type Cancel<'a> = &'a (dyn Fn() -> bool + Sync);

/// Загрузку отменил пользователь.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("отменено")
    }
}

impl std::error::Error for Cancelled {}

pub fn system_ota_url(c: &Config) -> String {
    format!("{}/{}/waydroid_{}/{}.json", c.system_channel, c.rom_type, c.arch, c.system_type)
}
pub fn vendor_ota_url(c: &Config) -> String {
    format!("{}/waydroid_{}/{}.json", c.vendor_channel, c.arch, c.vendor_type)
}

/// Временная ошибка сети или сервера (нет связи, 5xx, оборванная или зависшая загрузка, страница вместо файла):
/// загрузку стоит повторить позже. Остальные ошибки (404, нет места…) повтором не лечатся.
#[derive(Debug)]
pub struct Transient(pub String);

impl std::fmt::Display for Transient {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Transient {}

/// Ошибка временная — см. [`Transient`].
pub fn is_transient(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.is::<Transient>())
}

/// Ошибка запроса ureq → понятный текст; сетевые сбои и 5xx/408/429 — [`Transient`].
fn http_error(what: &str, host: &str, url: &str, e: ureq::Error) -> anyhow::Error {
    use ureq::Error as E;
    match e {
        E::StatusCode(code) if code >= 500 || code == 408 || code == 429 => {
            Transient(format!("{what}: сервер {host} недоступен (HTTP {code}) — попробуйте позже ({url})")).into()
        }
        E::StatusCode(code) => anyhow::anyhow!("{what}: сервер {host} ответил HTTP {code} ({url})"),
        E::Timeout(t) => Transient(format!("{what}: сервер {host} не ответил вовремя ({t}) ({url})")).into(),
        E::HostNotFound => Transient(format!("{what}: не найден сервер {host} — нет сети? ({url})")).into(),
        e @ (E::Io(_) | E::ConnectionFailed | E::Tls(_) | E::Protocol(_) | E::TooManyRedirects | E::RedirectFailed) => {
            Transient(format!("{what}: связь с {host} не удалась: {e} ({url})")).into()
        }
        e => anyhow::anyhow!("{what}: {host}: {e} ({url})"),
    }
}

fn host_of(url: &str) -> &str {
    url.split('/').nth(2).unwrap_or(url)
}

/// Тайм-ауты из настроек: соединение и ожидание ответа — `connect_timeout`, у запросов OTA (маленький JSON) —
/// ещё и весь запрос.
fn agent(c: &Config, whole: bool) -> ureq::Agent {
    let t = Duration::from_secs(c.connect_timeout.max(1) as u64);
    let mut b = ureq::Agent::config_builder()
        .user_agent(concat!("syndroid/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Some(t))
        .timeout_recv_response(Some(t));
    if whole {
        b = b.timeout_global(Some(t * 2));
    }
    b.build().into()
}

/// Последние сборки каналов (system, vendor).
pub fn ota_latest(c: &Config) -> Result<(OtaEntry, OtaEntry)> {
    let get = |url: &str| -> Result<OtaEntry> {
        let host = host_of(url);
        let mut r = agent(c, true).get(url).call().map_err(|e| http_error("OTA-канал", host, url, e))?;
        let body: OtaResponse = serde_json::from_reader(r.body_mut().as_reader())
            .map_err(|e| Transient(format!("OTA-канал {host}: не JSON ({e}) ({url})")))?;
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

/// Прямая ссылка на файл SourceForge: `sourceforge.net/projects/P/files/ПУТЬ/download` →
/// `downloads.sourceforge.net/project/P/ПУТЬ`. Страница `/download` при сбоях SourceForge («Disaster
/// Recovery») отдаёт вместо файла HTML со сценарием выбора зеркала, а прямая ссылка ведёт на зеркало сразу.
fn direct_url(url: &str) -> String {
    let Some(rest) = url.strip_prefix("https://sourceforge.net/projects/") else { return url.to_string() };
    let Some(rest) = rest.strip_suffix("/download") else { return url.to_string() };
    match rest.split_once("/files/") {
        Some((proj, path)) => format!("https://downloads.sourceforge.net/project/{proj}/{path}"),
        None => url.to_string(),
    }
}

/// Скачать URL в файл, считая sha256; `expect` — ожидаемая сумма (hex), `size` — ожидаемый размер (0 — неизвестен).
/// Уже скачанная часть файла докачивается (Range); если за `stall_timeout` не пришло ни байта — обрыв
/// ([`Transient`]), повтор продолжит с того же места.
fn download(c: &Config, url: &str, to: &Path, expect: &str, size: u64, what: &str, progress: Progress, cancel: Cancel) -> Result<()> {
    let url = &direct_url(url);
    let host = host_of(url);
    let mut have = fs::metadata(to).map(|m| m.len()).unwrap_or(0);
    if size > 0 && have > size {
        fs::remove_file(to)?;
        have = 0;
    }
    let mut hash = Sha256::new();
    let mut total = size;
    let mut body = None;
    if size == 0 || have < size {
        let mut req = agent(c, false).get(url);
        if have > 0 {
            req = req.header("Range", format!("bytes={have}-"));
        }
        let r = req.call().map_err(|e| http_error(what, host, url, e))?;
        let html = r
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/html"));
        if html {
            return Err(Transient(format!(
                "{what}: {host} вернул веб-страницу вместо файла — сайт, видимо, недоступен, попробуйте позже ({url})"
            ))
            .into());
        }
        let len: u64 = r.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);
        if have > 0 && r.status().as_u16() != 206 {
            // Сервер не умеет докачку — сначала
            have = 0;
        }
        if total == 0 {
            total = have + len;
        }
        body = Some(r.into_body().into_reader());
    }
    // Уже скачанное — в сумму
    let mut out = fs::OpenOptions::new().create(true).write(true).truncate(have == 0).open(to)?;
    if have > 0 {
        let mut f = File::open(to)?;
        let mut buf = vec![0u8; 1 << 20];
        let mut done = 0u64;
        while done < have {
            let n = f.read(&mut buf[..((have - done).min(1 << 20)) as usize])?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
            done += n as u64;
            progress("проверка скачанного", done, have);
        }
        out.set_len(have)?;
        use std::io::Seek;
        out.seek(std::io::SeekFrom::Start(have))?;
    }
    let mut done = have;
    if let Some(mut src) = body {
        // Чтение — в отдельном потоке: зависшее соединение блокирует read() без срока, а мы ждём данные не
        // дольше stall_timeout. Брошенный поток доживает до ошибки сокета.
        let (tx, rx) = std::sync::mpsc::sync_channel::<std::io::Result<Vec<u8>>>(8);
        std::thread::spawn(move || loop {
            let mut buf = vec![0u8; 256 << 10];
            match src.read(&mut buf) {
                Ok(n) => {
                    buf.truncate(n);
                    if tx.send(Ok(buf)).is_err() || n == 0 {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                    break;
                }
            }
        });
        let stall = Duration::from_secs(c.stall_timeout.max(5) as u64);
        progress(what, done, total);
        let mut last = std::time::Instant::now();
        loop {
            if cancel() {
                return Err(Cancelled.into());
            }
            let chunk = match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(Ok(b)) => b,
                Ok(Err(e)) => {
                    out.sync_all()?;
                    return Err(Transient(format!("{what}: связь с {host} оборвалась: {e}")).into());
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) if last.elapsed() < stall => continue,
                Err(_) => {
                    out.sync_all()?;
                    return Err(Transient(format!("{what}: от {host} нет данных {} с — связь зависла", stall.as_secs())).into());
                }
            };
            last = std::time::Instant::now();
            if chunk.is_empty() {
                break;
            }
            hash.update(&chunk);
            out.write_all(&chunk)?;
            done += chunk.len() as u64;
            progress(what, done, total);
        }
    }
    out.sync_all()?;
    if size > 0 && done < size {
        return Err(Transient(format!("{what}: {host} отдал {done} байт из {size} — повтор докачает")).into());
    }
    let got = format!("{:x}", hash.finalize());
    if !expect.is_empty() && !got.eq_ignore_ascii_case(expect) {
        // Испорчено при передаче (или подменено) — заново
        let _ = fs::remove_file(to);
        return Err(Transient(format!("{what}: файл повреждён (sha256 {got} ≠ {expect}) — будет скачан заново")).into());
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
pub fn fetch(c: &Config, entries: Option<(OtaEntry, OtaEntry)>, progress: Progress, cancel: Cancel) -> Result<ImageSet> {
    progress("запрос OTA-каналов", 0, 0);
    let (sys, ven) = match entries {
        Some(e) => e,
        None => ota_latest(c)?,
    };
    let name = set_name(&sys.filename);
    // Каталог сборки не чистится: после обрыва повтор докачивает начатое и не трогает готовые .img
    check_name(&name)?;
    let st = paths::images().join(format!(".{name}.part"));
    fs::create_dir_all(&st)?;
    for e in fs::read_dir(paths::images())?.flatten() {
        let n = e.file_name();
        let n = n.to_string_lossy();
        if n.starts_with('.') && n.ends_with(".part") && e.path() != st {
            let _ = fs::remove_dir_all(e.path());
        }
    }
    let r = (|| {
        for (e, part) in [(&sys, "system"), (&ven, "vendor")] {
            let img = st.join(format!("{part}.img"));
            if img.is_file() {
                continue;
            }
            let zip = st.join(&e.filename);
            download(c, &e.url, &zip, &e.id, e.size, &format!("загрузка {part}"), progress, cancel)?;
            unpack(&zip, part, &img, progress)?;
            fs::remove_file(&zip)?;
        }
        finish(&name, &st, Origin::Ota(sys.clone()), Origin::Ota(ven.clone()))
    })();
    if r.as_ref().is_err_and(|e| !is_transient(e) || cancel()) {
        let _ = fs::remove_dir_all(&st);
    }
    r
}

/// Бросить начатую загрузку: недокачанные файлы.
pub fn drop_partial() {
    for e in fs::read_dir(paths::images()).into_iter().flatten().flatten() {
        let n = e.file_name();
        let n = n.to_string_lossy();
        if n.starts_with('.') && n.ends_with(".part") {
            let _ = fs::remove_dir_all(e.path());
        }
    }
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
