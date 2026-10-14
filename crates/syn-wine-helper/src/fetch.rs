//! Скачивание источника в кэш: адреса по порядку, затем те же в архиве web.archive.org
//! (старые редистрибутивы Microsoft со временем пропадают; `id_` — файл как есть, ближайший к 2020 снимок). Принимается только файл
//! с размером и sha256 из базы.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::db::Source;

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_body(Some(Duration::from_secs(120)))
        .user_agent("syn-wine-helper")
        .build()
        .into()
}

pub fn cached(src: &Source) -> PathBuf {
    crate::store::cache_dir().join(format!("{}-{}", &src.sha256[..12], src.file))
}

/// Путь к файлу источника (скачивает, если в кэше нет). `progress(получено, всего)`.
pub fn fetch(src: &Source, progress: &mut dyn FnMut(u64, u64)) -> Result<PathBuf> {
    let path = cached(src);
    if path.is_file() && sha256_file(&path).ok().as_deref() == Some(src.sha256.as_str()) {
        return Ok(path);
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    let mut urls: Vec<String> = src.urls.clone();
    urls.extend(src.urls.iter().map(|u| format!("https://web.archive.org/web/2020id_/{u}")));
    let mut errors = Vec::new();
    for url in &urls {
        match try_url(url, src, &path, progress) {
            Ok(()) => return Ok(path),
            Err(e) => {
                tracing::warn!("{url}: {e:#}");
                errors.push(format!("{url}: {e:#}"));
            }
        }
    }
    bail!("{}", errors.join("\n"))
}

fn try_url(url: &str, src: &Source, path: &PathBuf, progress: &mut dyn FnMut(u64, u64)) -> Result<()> {
    let mut resp = agent().get(url).call()?;
    let part = path.with_extension("part");
    let mut out = std::fs::File::create(&part).with_context(|| part.display().to_string())?;
    let mut r = resp.body_mut().with_config().limit(src.size + 1).reader();
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut got = 0u64;
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        out.write_all(&buf[..n])?;
        got += n as u64;
        progress(got, src.size);
    }
    drop(out);
    let sum = hex(&h.finalize());
    if got != src.size || sum != src.sha256 {
        let _ = std::fs::remove_file(&part);
        bail!("{}", syngui::t!("другой файл: {got} байт, sha256 {sum}", got = got, sum = sum));
    }
    std::fs::rename(&part, path)?;
    Ok(())
}

pub fn sha256_file(p: &std::path::Path) -> Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(hex(&h.finalize()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
