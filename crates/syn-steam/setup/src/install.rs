//! Установка клиента: манифест с CDN Valve → выбранные компоненты в кэш (`<корень>/package/syn-steam-setup`,
//! размер и sha256 из манифеста) → распаковка в корень клиента → кэш удаляется. Дальше клиент обновляет себя сам.
//! Пока распаковка не закончена, в корне лежит метка [`INCOMPLETE`]: прерванная установка повторяется.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use syngui::t;

use crate::manifest::{self, Channel, Component, Manifest};

/// Серверы обновлений клиента (как у самого клиента), по порядку.
pub const CDNS: &[&str] = &[
    "https://client-update.fastly.steamstatic.com",
    "https://client-update.akamai.steamstatic.com",
    "https://client-update.steamstatic.com",
];

/// Метка незаконченной установки в корне клиента.
pub const INCOMPLETE: &str = ".syn-steam-incomplete";
/// Что поставлено первой установкой: версия и канал.
pub const STAMP: &str = ".syn-steam-setup";
/// Исполняемый файл клиента в корне.
pub const CLIENT: &str = "steamrtarm64/steam";

/// Нет связи с серверами (нет сети, DNS, сервер недоступен) — для понятного сообщения.
#[derive(Debug)]
pub struct Offline(pub String);

impl std::fmt::Display for Offline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Offline {}

/// Отменено пользователем.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

pub fn is_offline(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.downcast_ref::<Offline>().is_some())
}

pub fn is_cancelled(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.downcast_ref::<Cancelled>().is_some())
}

/// Клиент поставлен и установка не прервана.
pub fn installed(root: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    !root.join(INCOMPLETE).exists()
        && std::fs::metadata(root.join(CLIENT))
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Корень клиента по умолчанию — как у запускателя steam: `$STEAM_ARM64_ROOT` или `$XDG_DATA_HOME/Steam`.
pub fn default_root() -> PathBuf {
    if let Some(r) = std::env::var_os("STEAM_ARM64_ROOT").filter(|r| !r.is_empty()) {
        return PathBuf::from(r);
    }
    let data = std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        });
    data.join("Steam")
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Скачано `done` из `total` байт (компонент `index` из `count`).
    Download {
        done: u64,
        total: u64,
        index: usize,
        count: usize,
    },
    /// Распаковка компонента `index` из `count`.
    Unpack { index: usize, count: usize },
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_body(Some(Duration::from_secs(60)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent("syn-steam-setup")
        .build()
        .into()
}

/// Ошибка ureq: «нет связи» или прочая.
fn net_error(url: &str, e: ureq::Error) -> anyhow::Error {
    use ureq::Error as E;
    match e {
        E::HostNotFound | E::ConnectionFailed | E::Timeout(_) | E::Io(_) => {
            anyhow::Error::new(Offline(format!("{url}: {e}")))
        }
        e => anyhow::anyhow!("{url}: {e}"),
    }
}

/// Манифест канала с первого ответившего сервера → (манифест, сервер).
pub fn fetch_manifest(channel: Channel) -> Result<(Manifest, &'static str)> {
    let mut last: Option<anyhow::Error> = None;
    for cdn in CDNS {
        let url = format!("{cdn}/{}", channel.manifest());
        let text = match agent().get(&url).call() {
            Ok(mut r) => r
                .body_mut()
                .with_config()
                .limit(4 << 20)
                .read_to_string()
                .map_err(|e| net_error(&url, e)),
            Err(e) => Err(net_error(&url, e)),
        };
        match text {
            Ok(t) => return Ok((manifest::parse(&t).with_context(|| url.clone())?, cdn)),
            Err(e) => {
                tracing::warn!("{e:#}");
                last = Some(e);
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no CDN")))
}

pub fn sha256_file(p: &Path) -> Result<String> {
    let mut f = File::open(p)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(hex(&h.finalize()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Файл совпадает с компонентом манифеста (размер и sha256).
pub fn verify(path: &Path, c: &Component) -> Result<()> {
    let len = std::fs::metadata(path)?.len();
    let sum = sha256_file(path)?;
    if len != c.size || sum != c.sha256 {
        bail!(
            "{}",
            t!(
                "{f}: другой файл — {got} байт, sha256 {sum} (в манифесте {size} байт, {want})",
                f = &c.file,
                got = len,
                sum = sum,
                size = c.size,
                want = &c.sha256
            )
        );
    }
    Ok(())
}

/// Скачивание компонента в `path` с проверкой размера и sha256 по ходу; `progress(получено)`.
fn download(
    cdn: &str,
    c: &Component,
    path: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<()> {
    let url = format!("{cdn}/{}", c.file);
    let mut resp = agent().get(&url).call().map_err(|e| net_error(&url, e))?;
    let part = path.with_extension("part");
    let mut out = File::create(&part).with_context(|| part.display().to_string())?;
    let mut r = resp.body_mut().with_config().limit(c.size + 1).reader();
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut got = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(out);
            let _ = std::fs::remove_file(&part);
            return Err(Cancelled.into());
        }
        let n = match r.read(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                return Err(anyhow::Error::new(Offline(format!("{url}: {e}"))));
            }
        };
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        out.write_all(&buf[..n])
            .with_context(|| part.display().to_string())?;
        got += n as u64;
        progress(got);
    }
    drop(out);
    let sum = hex(&h.finalize());
    if got != c.size || sum != c.sha256 {
        let _ = std::fs::remove_file(&part);
        bail!(
            "{}",
            t!(
                "{f}: другой файл — {got} байт, sha256 {sum} (в манифесте {size} байт, {want})",
                f = &c.file,
                got = got,
                sum = sum,
                size = c.size,
                want = &c.sha256
            )
        );
    }
    std::fs::rename(&part, path)?;
    Ok(())
}

/// Путь внутри корня по имени из архива Valve (разделители — и «/», и «\»); `None` — пустой или выходит за корень.
pub fn entry_path(name: &str) -> Option<PathBuf> {
    let mut p = PathBuf::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            s => p.push(s),
        }
    }
    (!p.as_os_str().is_empty()).then_some(p)
}

/// ELF или скрипт — исполняемый (в архивах Valve права не всегда записаны).
fn looks_executable(head: &[u8]) -> bool {
    head.starts_with(b"\x7fELF") || head.starts_with(b"#!")
}

/// Распаковка архива компонента в корень клиента.
pub fn unpack(zip_path: &Path, root: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let f = File::open(zip_path).with_context(|| zip_path.display().to_string())?;
    let mut z = zip::ZipArchive::new(f).with_context(|| zip_path.display().to_string())?;
    for i in 0..z.len() {
        let mut e = z.by_index(i)?;
        let Some(rel) = entry_path(e.name()) else {
            continue;
        };
        let to = root.join(&rel);
        let mode = e.unix_mode();
        if e.is_dir() {
            std::fs::create_dir_all(&to)?;
            continue;
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // символьная ссылка (S_IFLNK)
        if mode.is_some_and(|m| m & 0o170000 == 0o120000) {
            let mut target = String::new();
            e.read_to_string(&mut target)?;
            let _ = std::fs::remove_file(&to);
            std::os::unix::fs::symlink(target, &to).with_context(|| to.display().to_string())?;
            continue;
        }
        let tmp = to.with_file_name(format!(
            ".{}.syn-tmp",
            to.file_name().unwrap_or_default().to_string_lossy()
        ));
        let mut out = File::create(&tmp).with_context(|| tmp.display().to_string())?;
        let mut head = [0u8; 4];
        let mut n_head = 0;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = e
                .read(&mut buf)
                .with_context(|| format!("{}: {}", zip_path.display(), rel.display()))?;
            if n == 0 {
                break;
            }
            if n_head < 4 {
                let k = (4 - n_head).min(n);
                head[n_head..n_head + k].copy_from_slice(&buf[..k]);
                n_head += k;
            }
            out.write_all(&buf[..n])
                .with_context(|| tmp.display().to_string())?;
        }
        drop(out);
        let exec = mode.is_some_and(|m| m & 0o111 != 0) || looks_executable(&head[..n_head]);
        std::fs::set_permissions(
            &tmp,
            std::fs::Permissions::from_mode(if exec { 0o755 } else { 0o644 }),
        )?;
        std::fs::rename(&tmp, &to).with_context(|| to.display().to_string())?;
    }
    Ok(())
}

/// Полная установка в `root`. `progress` — из этого же потока.
pub fn install(
    root: &Path,
    channel: Channel,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Step),
) -> Result<Manifest> {
    let (m, cdn) = fetch_manifest(channel)?;
    let comps = m.selected();
    let count = comps.len();
    let total: u64 = comps.iter().map(|c| c.size).sum();
    let cache = root.join("package/syn-steam-setup");
    std::fs::create_dir_all(&cache).with_context(|| cache.display().to_string())?;
    tracing::info!(
        "Steam {} ({}): {} компонентов, {} байт с {cdn}",
        m.version,
        channel.name(),
        count,
        total
    );

    // скачивание (уже скачанное и верное не трогаем — повтор после обрыва)
    let mut done = 0u64;
    let mut paths = Vec::with_capacity(count);
    for (i, c) in comps.iter().enumerate() {
        let path = cache.join(&c.file);
        if !(path.is_file() && verify(&path, c).is_ok()) {
            let base = done;
            download(cdn, c, &path, cancel, &mut |got| {
                progress(Step::Download {
                    done: base + got,
                    total,
                    index: i,
                    count,
                })
            })?;
        }
        done += c.size;
        progress(Step::Download {
            done,
            total,
            index: i,
            count,
        });
        paths.push(path);
    }

    // распаковка; метка — пока не закончили
    std::fs::write(root.join(INCOMPLETE), &m.version)?;
    for (i, p) in paths.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        progress(Step::Unpack { index: i, count });
        unpack(p, root)?;
    }
    if !std::fs::metadata(root.join(CLIENT)).is_ok_and(|m| m.is_file()) {
        bail!("{}", t!("в архивах клиента нет {f}", f = CLIENT));
    }
    std::fs::write(
        root.join(STAMP),
        format!("version={}\nchannel={}\n", m.version, channel.name()),
    )?;
    std::fs::remove_file(root.join(INCOMPLETE))?;
    let _ = std::fs::remove_dir_all(&cache);
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "syn-steam-setup-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn verify_checks_size_and_sum() {
        let d = tmpdir("verify");
        let f = d.join("a.zip.1");
        std::fs::write(&f, b"hello").unwrap();
        // sha256("hello")
        let good = Component {
            name: "a_all".into(),
            file: "a.zip.1".into(),
            size: 5,
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".into(),
        };
        assert!(verify(&f, &good).is_ok());
        let bad_sum = Component {
            sha256: "0".repeat(64),
            ..good.clone()
        };
        assert!(verify(&f, &bad_sum).is_err());
        let bad_size = Component { size: 6, ..good };
        assert!(verify(&f, &bad_size).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn manifest_sums_match_files() {
        // придуманный манифест с суммами настоящих файлов
        let d = tmpdir("manifest");
        std::fs::write(d.join("x_all.zip.aa"), b"hello").unwrap();
        std::fs::write(d.join("y_linuxarm64_linuxarm64.zip.bb"), b"world").unwrap();
        let text = r#""linuxarm64" {
            "version" "42"
            "x_all" { "file" "x_all.zip.aa" "size" "5" "sha2" "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824" }
            "y_linuxarm64_linuxarm64" { "file" "y_linuxarm64_linuxarm64.zip.bb" "size" "5" "sha2" "0000000000000000000000000000000000000000000000000000000000000000" }
        }"#;
        let m = manifest::parse(text).unwrap();
        let sel = m.selected();
        assert_eq!(sel.len(), 2);
        assert!(verify(&d.join(&sel[0].file), sel[0]).is_ok());
        // у второго в манифесте чужая сумма — файл отвергается
        assert!(verify(&d.join(&sel[1].file), sel[1]).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn entry_paths() {
        assert_eq!(
            entry_path("steamrtarm64\\steam"),
            Some(PathBuf::from("steamrtarm64/steam"))
        );
        assert_eq!(entry_path("a/b\\c"), Some(PathBuf::from("a/b/c")));
        assert_eq!(entry_path("/abs/x"), Some(PathBuf::from("abs/x")));
        assert_eq!(entry_path("a\\..\\..\\etc"), None);
        assert_eq!(entry_path(""), None);
    }

    #[test]
    fn unpack_backslash_paths_and_exec() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmpdir("unpack");
        let zp = d.join("t.zip");
        {
            let mut w = zip::ZipWriter::new(File::create(&zp).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            w.start_file("steamrtarm64\\steam", o).unwrap();
            w.write_all(b"\x7fELF....").unwrap();
            w.start_file("public/readme.txt", o).unwrap();
            w.write_all(b"text").unwrap();
            w.start_file("..\\evil", o).unwrap();
            w.write_all(b"x").unwrap();
            w.finish().unwrap();
        }
        let root = d.join("root");
        std::fs::create_dir_all(&root).unwrap();
        unpack(&zp, &root).unwrap();
        let m = std::fs::metadata(root.join("steamrtarm64/steam")).unwrap();
        assert_eq!(m.permissions().mode() & 0o777, 0o755);
        assert_eq!(
            std::fs::metadata(root.join("public/readme.txt"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o644
        );
        assert!(!d.join("evil").exists());
        assert!(installed(&root));
        std::fs::write(root.join(INCOMPLETE), "1").unwrap();
        assert!(!installed(&root));
        let _ = std::fs::remove_dir_all(&d);
    }
}
