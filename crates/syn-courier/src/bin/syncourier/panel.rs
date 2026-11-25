//! Пульт Курьера — мини-приложение Telegram (syngui → WebAssembly, крейт `syn-courier-panel`).
//!
//! Служба раздаёт страницу пульта и его API на `127.0.0.1:PORT` и сама поднимает
//! `cloudflared tunnel --url` (quick tunnel, без учётной записи): полученный адрес
//! `https://….trycloudflare.com` ставится кнопкой меню бота («Пульт»). Каждый запрос API
//! несёт `initData` мини-приложения — подпись проверяется токеном бота, и пускается только
//! владелец бота (тот же, кому бот отвечает).
//!
//! API (JSON):
//! - `GET  /api/state` — Claude «courier», каналы, уведомления, последние сообщения, живые сеансы;
//! - `POST /api/send {text, to?}` — сообщение Claude «courier» или сеансу `to` (как `/to`);
//! - `POST /api/new {dir?}`, `POST /api/stop` — новый разговор, прервать ответ;
//! - `POST /api/shot` — снимок экрана компьютера в чат.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use crate::hub::Hub;

/// Порт пульта на localhost (переопределяется `SYNCOURIER_PANEL_PORT`).
const PORT: u16 = 51871;
/// Подпись initData годна сутки.
const INIT_DATA_TTL: u64 = 24 * 3600;

/// Текущий адрес туннеля (для `syncourier status` и логов).
pub static URL: Mutex<Option<String>> = Mutex::new(None);

/// Каталог собранного пульта: `SYNCOURIER_PANEL_DIR`, иначе установленный `/usr/share/syncourier/panel`.
fn panel_dir() -> PathBuf {
    std::env::var_os("SYNCOURIER_PANEL_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/usr/share/syncourier/panel"))
}

fn port() -> u16 {
    std::env::var("SYNCOURIER_PANEL_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(PORT)
}

pub fn start(hub: Arc<Hub>) {
    if !hub.config().panel.enabled {
        tracing::info!("пульт выключен ([panel] enabled = false)");
        return;
    }
    if !panel_dir().join("index.html").is_file() {
        tracing::info!("пульт: нет {}/index.html — мини-приложение не поднимаю", panel_dir().display());
        return;
    }
    let port = port();
    let h = hub.clone();
    tokio::spawn(async move {
        let listener = match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("пульт: порт {port}: {e}");
                return;
            }
        };
        tracing::info!("пульт: http://127.0.0.1:{port}");
        loop {
            if let Ok((conn, _)) = listener.accept().await {
                let h = h.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve(h, conn).await {
                        tracing::debug!("пульт: {e:#}");
                    }
                });
            }
        }
    });
    tokio::spawn(tunnel(hub, port));
}

/// Туннель Cloudflare. Именованный (`[panel] tunnel`, `hostname`) — постоянный адрес, ставится в меню
/// бота сразу; quick tunnel — адрес меняется при каждом запуске и ставится, когда cloudflared его напечатает.
async fn tunnel(hub: Arc<Hub>, port: u16) {
    let mut backoff = 5u64;
    loop {
        let cfg = hub.config().panel;
        let local = format!("http://127.0.0.1:{port}");
        let named = !cfg.tunnel.trim().is_empty() && !cfg.hostname.trim().is_empty();
        let mut args: Vec<String> = vec!["tunnel".into(), "--no-autoupdate".into()];
        if named {
            args.extend(["run".into(), "--url".into(), local.clone(), cfg.tunnel.trim().to_string()]);
            let url = format!("https://{}", cfg.hostname.trim().trim_start_matches("https://").trim_end_matches('/'));
            *URL.lock().unwrap() = Some(url.clone());
            let h = hub.clone();
            tokio::spawn(async move {
                if let Err(e) = h.tg.set_menu_button(&url).await {
                    tracing::warn!("пульт: кнопка меню бота: {e:#}");
                }
            });
        } else {
            args.extend(["--url".into(), local.clone()]);
        }
        let child = tokio::process::Command::new("cloudflared")
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("пульт: cloudflared не запустился ({e}) — пульт доступен только локально");
                return;
            }
        };
        let started = std::time::Instant::now();
        if let Some(err) = child.stderr.take() {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(url) = find_tunnel_url(&line).filter(|_| !named) {
                    tracing::info!("пульт: {url}");
                    *URL.lock().unwrap() = Some(url.clone());
                    // Туннель поднят, но DNS имени ещё расходится — подождать, иначе Telegram откроет ошибку.
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    if let Err(e) = hub.tg.set_menu_button(&url).await {
                        tracing::warn!("пульт: кнопка меню бота: {e:#}");
                    }
                }
            }
        }
        let _ = child.wait().await;
        *URL.lock().unwrap() = None;
        backoff = if started.elapsed() > Duration::from_secs(300) { 5 } else { (backoff * 2).min(300) };
        tracing::warn!("пульт: cloudflared завершился, перезапуск через {backoff} с");
        tokio::time::sleep(Duration::from_secs(backoff)).await;
    }
}

pub fn find_tunnel_url(line: &str) -> Option<String> {
    let i = line.find("https://")?;
    let url: String = line[i..].chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '/' | '.' | '-')).collect();
    url.ends_with(".trycloudflare.com").then_some(url)
}

struct Request {
    method: String,
    path: String,
    init_data: String,
    gzip: bool,
    if_none_match: String,
    body: Vec<u8>,
}

async fn read_request(conn: &mut BufReader<TcpStream>) -> anyhow::Result<Request> {
    let mut line = String::new();
    conn.read_line(&mut line).await?;
    let mut it = line.split_whitespace();
    let method = it.next().unwrap_or("").to_string();
    let path = it.next().unwrap_or("/").to_string();
    let mut len = 0usize;
    let mut init_data = String::new();
    let mut gzip = false;
    let mut if_none_match = String::new();
    loop {
        let mut h = String::new();
        if conn.read_line(&mut h).await? == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "content-length" => len = v.trim().parse().unwrap_or(0),
                "x-telegram-init-data" => init_data = v.trim().to_string(),
                "accept-encoding" => gzip = v.contains("gzip"),
                "if-none-match" => if_none_match = v.trim().to_string(),
                _ => {}
            }
        }
    }
    anyhow::ensure!(len <= 64 * 1024, "слишком большой запрос");
    let mut body = vec![0u8; len];
    conn.read_exact(&mut body).await?;
    Ok(Request { method, path, init_data, gzip, if_none_match, body })
}

async fn respond(conn: &mut BufReader<TcpStream>, code: u16, ctype: &str, body: &[u8]) -> anyhow::Result<()> {
    respond_enc(conn, code, ctype, body, "").await
}

/// `extra` — готовые строки заголовков (`Имя: значение\r\n`).
async fn respond_enc(conn: &mut BufReader<TcpStream>, code: u16, ctype: &str, body: &[u8], extra: &str) -> anyhow::Result<()> {
    let reason = match code {
        200 => "OK",
        304 => "Not Modified",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n{}Cache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len(),
        extra
    );
    let s = conn.get_mut();
    s.write_all(head.as_bytes()).await?;
    s.write_all(body).await?;
    s.flush().await?;
    Ok(())
}

async fn serve(hub: Arc<Hub>, conn: TcpStream) -> anyhow::Result<()> {
    let mut conn = BufReader::new(conn);
    let req = read_request(&mut conn).await?;
    let path = req.path.split('?').next().unwrap_or("/").to_string();
    if let Some(api) = path.strip_prefix("/api/") {
        let token = hub.tg.token_pub();
        let owner = hub.tg.owner_pub();
        if !authorized(&req.init_data, &token, owner, now_secs()) {
            return respond(&mut conn, 401, "application/json", br#"{"error":"unauthorized"}"#).await;
        }
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
        let out = api_call(&hub, &req.method, api, &body).await;
        return match out {
            Ok(v) => respond(&mut conn, 200, "application/json", v.to_string().as_bytes()).await,
            Err(e) => respond(&mut conn, 500, "application/json", serde_json::json!({ "error": format!("{e:#}") }).to_string().as_bytes()).await,
        };
    }
    // Кэш браузера: ETag по размеру и времени файла; совпал — 304 без тела (wasm не качается заново).
    let Some(etag) = file_etag(&panel_dir(), &path) else {
        return respond(&mut conn, 404, "text/plain; charset=utf-8", b"not found").await;
    };
    let cache = format!("ETag: {etag}\r\nVary: Accept-Encoding\r\n");
    if req.if_none_match.split(',').any(|t| t.trim().trim_start_matches("W/") == etag) {
        return respond_enc(&mut conn, 304, "text/plain", b"", &cache).await;
    }
    // Рядом с большим файлом может лежать сжатый `<файл>.gz` (wasm ужимается втрое) — отдать его.
    if req.gzip {
        let ctype = content_type(Path::new(if path.ends_with('/') { "index.html" } else { &path }));
        if let Some((gz, _)) = static_file(&panel_dir(), &format!("{path}.gz")) {
            return respond_enc(&mut conn, 200, ctype, &gz, &format!("{cache}Content-Encoding: gzip\r\n")).await;
        }
    }
    match static_file(&panel_dir(), &path) {
        Some((bytes, ctype)) => respond_enc(&mut conn, 200, ctype, &bytes, &cache).await,
        None => respond(&mut conn, 404, "text/plain; charset=utf-8", b"not found").await,
    }
}

/// ETag файла пульта: размер и время изменения (меняется при новой сборке).
fn file_etag(root: &Path, path: &str) -> Option<String> {
    let rel = path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if rel.split('/').any(|p| p == ".." || p.starts_with('.')) {
        return None;
    }
    let m = std::fs::metadata(root.join(rel)).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(format!("\"{:x}-{:x}\"", m.len(), t))
}

fn static_file(root: &Path, path: &str) -> Option<(Vec<u8>, &'static str)> {
    let rel = path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if rel.split('/').any(|p| p == ".." || p.starts_with('.')) {
        return None;
    }
    let p = root.join(rel);
    let bytes = std::fs::read(&p).ok()?;
    Some((bytes, content_type(&p)))
}

fn content_type(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn now_secs() -> u64 {
    crate::hub::now_ms() / 1000
}

fn short_path(p: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    match p.strip_prefix(&home) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => p.to_string(),
    }
}

async fn api_call(hub: &Arc<Hub>, method: &str, api: &str, body: &serde_json::Value) -> anyhow::Result<serde_json::Value> {
    let s = |k: &str| body.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    match (method, api) {
        ("GET", "state") => {
            let st = hub.snapshot();
            let agents = crate::router::agents(&hub.config().claude).await.unwrap_or_default();
            Ok(serde_json::json!({
                "claude": {
                    "name": st.claude.name,
                    "running": st.claude.running,
                    "busy": st.claude.busy,
                    "cwd": short_path(&syn_courier::expand_home(&st.claude.cwd).to_string_lossy()),
                },
                "channels": st.channels.iter().map(|c| serde_json::json!({ "id": c.id, "title": c.title, "online": c.state == syn_courier::ChannelState::Online })).collect::<Vec<_>>(),
                "recent": st.recent.iter().rev().take(30).rev().map(|r| serde_json::to_value(r).unwrap_or_default()).collect::<Vec<_>>(),
                "agents": agents.iter().filter(|a| a.name != st.claude.name).map(|a| serde_json::json!({ "name": a.name, "status": a.status, "cwd": short_path(&a.cwd) })).collect::<Vec<_>>(),
            }))
        }
        ("POST", "send") => {
            let text = s("text");
            anyhow::ensure!(!text.is_empty(), "пустое сообщение");
            let to = s("to");
            let msg = if to.is_empty() { text } else { format!("/to {to} {text}") };
            hub.incoming("telegram", msg);
            Ok(serde_json::json!({ "ok": true }))
        }
        ("POST", "new") => {
            let dir = s("dir");
            hub.incoming("telegram", if dir.is_empty() { "/new".into() } else { format!("/new {dir}") });
            Ok(serde_json::json!({ "ok": true }))
        }
        ("POST", "stop") => {
            hub.incoming("telegram", "/stop".into());
            Ok(serde_json::json!({ "ok": true }))
        }
        ("POST", "shot") => {
            let path = syn_courier::data_dir().join("panel-shot.png");
            let st = tokio::process::Command::new("synshot").arg("--full").arg(&path).status().await?;
            anyhow::ensure!(st.success() && path.exists(), "synshot не снял экран");
            hub.send_file(&path, "Экран компьютера", false, false).await?;
            let _ = std::fs::remove_file(&path);
            Ok(serde_json::json!({ "ok": true }))
        }
        _ => anyhow::bail!("нет такого вызова: {method} /api/{api}"),
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = (b[i] == b'%').then(|| b.get(i + 1..i + 3)).flatten();
        match (b[i], hex.and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            (b'+', _) => {
                out.push(b' ');
                i += 1;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hmac256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut m = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC принимает ключ любой длины");
    m.update(msg);
    m.finalize().into_bytes().to_vec()
}

/// Проверка `initData` мини-приложения (core.telegram.org/bots/webapps#validating-data-received-via-the-mini-app):
/// подпись токеном бота, свежесть и что открыл владелец бота.
pub fn authorized(init_data: &str, token: &str, owner: Option<i64>, now: u64) -> bool {
    let (Some(owner), false) = (owner, token.is_empty() || init_data.is_empty()) else { return false };
    let mut hash = String::new();
    let mut pairs: Vec<(String, String)> = Vec::new();
    for kv in init_data.split('&') {
        let Some((k, v)) = kv.split_once('=') else { continue };
        let (k, v) = (percent_decode(k), percent_decode(v));
        if k == "hash" {
            hash = v;
        } else {
            pairs.push((k, v));
        }
    }
    if hash.len() != 64 {
        return false;
    }
    pairs.sort();
    let check = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n");
    let secret = hmac256(b"WebAppData", token.as_bytes());
    let calc: String = hmac256(&secret, check.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    if calc != hash.to_ascii_lowercase() {
        return false;
    }
    let get = |k: &str| pairs.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.as_str());
    let fresh = get("auth_date").and_then(|d| d.parse::<u64>().ok()).is_some_and(|d| now.saturating_sub(d) <= INIT_DATA_TTL);
    let user = get("user").and_then(|u| serde_json::from_str::<serde_json::Value>(u).ok()).and_then(|u| u.get("id")?.as_i64());
    fresh && user == Some(owner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(token: &str, fields: &[(&str, &str)]) -> String {
        let mut pairs: Vec<(String, String)> = fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        pairs.sort();
        let check = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n");
        let secret = hmac256(b"WebAppData", token.as_bytes());
        let hash: String = hmac256(&secret, check.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        let enc = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
        let mut q: Vec<String> = fields.iter().map(|(k, v)| format!("{k}={}", enc(v))).collect();
        q.push(format!("hash={hash}"));
        q.join("&")
    }

    #[test]
    fn init_data() {
        let token = "123:ABC";
        let user = r#"{"id":42,"first_name":"Виталий"}"#;
        let ok = sign(token, &[("auth_date", "1000"), ("query_id", "AAE"), ("user", user)]);
        assert!(authorized(&ok, token, Some(42), 1500));
        assert!(!authorized(&ok, token, Some(43), 1500), "чужой пользователь");
        assert!(!authorized(&ok, "123:XYZ", Some(42), 1500), "другой токен");
        assert!(!authorized(&ok, token, Some(42), 1000 + INIT_DATA_TTL + 1), "просрочено");
        assert!(!authorized(&ok.replace("auth_date=1000", "auth_date=1001"), token, Some(42), 1500), "подделка");
        assert!(!authorized("", token, Some(42), 1500));
        assert!(!authorized(&ok, token, None, 1500));
    }

    #[test]
    fn tunnel_url() {
        let l = "2026-11-23T08:00:00Z INF |  https://quiet-river-1234.trycloudflare.com                                  |";
        assert_eq!(find_tunnel_url(l).as_deref(), Some("https://quiet-river-1234.trycloudflare.com"));
        assert_eq!(find_tunnel_url("INF Requesting new quick Tunnel on trycloudflare.com..."), None);
        assert_eq!(find_tunnel_url("see https://developers.cloudflare.com/x"), None);
    }

    #[test]
    fn static_paths() {
        let d = std::env::temp_dir().join(format!("panel-test-{}", std::process::id()));
        std::fs::create_dir_all(d.join("pkg")).unwrap();
        std::fs::write(d.join("index.html"), "<html>").unwrap();
        std::fs::write(d.join("pkg/a.wasm"), [0u8, 97, 115, 109]).unwrap();
        assert_eq!(static_file(&d, "/").unwrap().1, "text/html; charset=utf-8");
        assert_eq!(static_file(&d, "/pkg/a.wasm").unwrap().1, "application/wasm");
        assert!(static_file(&d, "/../etc/passwd").is_none());
        assert!(static_file(&d, "/.git/config").is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
