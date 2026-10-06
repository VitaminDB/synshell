//! Локальный сокет демона (`synshell_common::link`): оболочка, настройки,
//! окно трансляции, CLI и MCP. Запросы к устройству уходят вызовами по
//! его соединению; устройство `local` — эта машина, без сети.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use synshell_common::link::{self, ExecFrame, Request, Response, ScreenHeader};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::daemon::D;
use crate::proto::{self, ExecIn, ExecOut, Reply, Rpc, ScreenAck, ScreenAckV2, ScreenFrame, ScreenFrameV2, VideoParams};

pub async fn run(d: D) -> Result<()> {
    let path = link::socket_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Живой демон уже есть — второй не нужен.
    if std::os::unix::net::UnixStream::connect(&path).is_ok() {
        bail!("synlink уже запущен ({})", path.display());
    }
    let _ = std::fs::remove_file(&path);
    let l = UnixListener::bind(&path)?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    tracing::info!(path = %path.display(), "локальный сокет");
    loop {
        let (s, _) = l.accept().await?;
        let d = d.clone();
        tokio::spawn(async move {
            if let Err(e) = conn(d, s).await {
                tracing::debug!("клиент сокета: {e:#}");
            }
        });
    }
}

enum Target {
    Local,
    Remote(quinn::Connection),
}

fn target(d: &D, device: &str) -> Result<Target> {
    if device == "local" || device == "self" || device.is_empty() {
        return Ok(Target::Local);
    }
    let id = d.resolve(device).with_context(|| format!("нет устройства «{device}»"))?;
    if id == d.id.id {
        return Ok(Target::Local);
    }
    let (c, _) = d.conn(&id).with_context(|| format!("«{device}» не соединено"))?;
    Ok(Target::Remote(c))
}

async fn write_line<W: AsyncWrite + Unpin>(w: &mut W, r: &impl serde::Serialize) -> Result<()> {
    let mut s = serde_json::to_string(r)?;
    s.push('\n');
    w.write_all(s.as_bytes()).await?;
    Ok(())
}

async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, data: &[u8]) -> Result<()> {
    w.write_all(&(data.len() as u32).to_be_bytes()).await?;
    w.write_all(data).await?;
    w.flush().await?;
    Ok(())
}

async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let mut b = vec![0u8; u32::from_be_bytes(len) as usize];
    r.read_exact(&mut b).await?;
    Ok(Some(b))
}

async fn conn(d: D, s: UnixStream) -> Result<()> {
    let (rd, mut wr) = s.into_split();
    let mut rd = BufReader::new(rd);
    let mut line = String::new();
    loop {
        line.clear();
        if rd.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        if line.trim().is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                write_line(&mut wr, &Response::Error { message: format!("неверный запрос: {e}") }).await?;
                continue;
            }
        };
        match req {
            Request::Subscribe => {
                let mut rx = d.events.subscribe();
                write_line(&mut wr, &Response::Ok).await?;
                write_line(&mut wr, &link::Event::Status { status: d.status() }).await?;
                loop {
                    match rx.recv().await {
                        Ok(e) => write_line(&mut wr, &e).await?,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            write_line(&mut wr, &link::Event::Status { status: d.status() }).await?
                        }
                        Err(_) => return Ok(()),
                    }
                }
            }
            Request::Exec { device, argv, cwd, stream: true, pty, .. } => {
                let (w, r) = match exec_streams(&d, &device, argv, cwd, pty).await {
                    Ok(x) => x,
                    Err(e) => {
                        write_line(&mut wr, &Response::Error { message: format!("{e:#}") }).await?;
                        continue;
                    }
                };
                write_line(&mut wr, &Response::Ok).await?;
                return bridge_exec(rd, wr, w, r).await;
            }
            Request::Tcp { device, port } => {
                let Target::Remote(c) = (match target(&d, &device) {
                    Ok(t) => t,
                    Err(e) => {
                        write_line(&mut wr, &Response::Error { message: format!("{e:#}") }).await?;
                        continue;
                    }
                }) else {
                    write_line(&mut wr, &Response::Error { message: "туннель к себе не нужен".into() }).await?;
                    continue;
                };
                let (w, mut r) = crate::rpc::open(&c, &Rpc::Tcp { port }).await?;
                match proto::recv::<Reply>(&mut r).await? {
                    Some(Reply::Done) => {}
                    Some(Reply::Err(e)) => {
                        write_line(&mut wr, &Response::Error { message: e }).await?;
                        continue;
                    }
                    _ => {
                        write_line(&mut wr, &Response::Error { message: "нет ответа".into() }).await?;
                        continue;
                    }
                }
                write_line(&mut wr, &Response::Ok).await?;
                let mut local = tokio::io::join(rd, wr);
                let mut quic = tokio::io::join(r, w);
                let _ = tokio::io::copy_bidirectional(&mut local, &mut quic).await;
                return Ok(());
            }
            Request::Screen { device, output, cursor, video } => {
                let c = match target(&d, &device) {
                    Ok(Target::Remote(c)) => c,
                    Ok(Target::Local) => {
                        write_line(&mut wr, &Response::Error { message: "трансляция своего экрана не нужна".into() }).await?;
                        continue;
                    }
                    Err(e) => {
                        write_line(&mut wr, &Response::Error { message: format!("{e:#}") }).await?;
                        continue;
                    }
                };
                // Устройство с proto ≥ 2 — трансляция с видео (если клиент его
                // декодирует и `[link] screen_codec` не `lossless`).
                let info = d.resolve(&device).and_then(|id| d.session_info(&id));
                if let Some((_, transport)) = info.filter(|(p, _)| *p >= 2) {
                    let params = if video { video_params(transport) } else { None };
                    let (w, r) = crate::rpc::open(&c, &Rpc::ScreenV2 { output, cursor, video: params }).await?;
                    write_line(&mut wr, &Response::Ok).await?;
                    return bridge_screen_v2(rd, wr, w, r).await;
                }
                let (w, r) = crate::rpc::open(&c, &Rpc::Screen { output, cursor }).await?;
                write_line(&mut wr, &Response::Ok).await?;
                return bridge_screen(rd, wr, w, r).await;
            }
            Request::Audio { device, on, mic, hold } => {
                if !on {
                    let resp = match d.resolve(&device) {
                        Some(id) => {
                            crate::audio::stop(&d, &id);
                            Response::Ok
                        }
                        None => Response::Error { message: format!("нет устройства «{device}»") },
                    };
                    write_line(&mut wr, &resp).await?;
                    continue;
                }
                let (id, gen, done) = match crate::audio::start(&d, &device, mic).await {
                    Ok(x) => x,
                    Err(e) => {
                        write_line(&mut wr, &Response::Error { message: format!("{e:#}") }).await?;
                        continue;
                    }
                };
                write_line(&mut wr, &Response::Ok).await?;
                if !hold {
                    continue;
                }
                // Звук живёт, пока клиент держит соединение; оборвался — сказать ему.
                let closed = async {
                    let mut l = String::new();
                    loop {
                        l.clear();
                        if matches!(rd.read_line(&mut l).await, Ok(0) | Err(_)) {
                            break;
                        }
                    }
                };
                tokio::select! {
                    _ = closed => crate::audio::stop_gen(&d, &id, gen),
                    m = done => {
                        let message = m.unwrap_or_else(|_| "звук выключен".into());
                        write_line(&mut wr, &Response::Error { message }).await?;
                    }
                }
                return Ok(());
            }
            other => {
                let resp = match handle(&d, other).await {
                    Ok(r) => r,
                    Err(e) => Response::Error { message: format!("{e:#}") },
                };
                write_line(&mut wr, &resp).await?;
            }
        }
    }
}

type BoxW = Box<dyn AsyncWrite + Unpin + Send>;
type BoxR = Box<dyn AsyncRead + Unpin + Send>;

/// Потоки вызова `Exec`: к устройству или к себе (через `duplex`).
async fn exec_streams(d: &D, device: &str, argv: Vec<String>, cwd: Option<String>, pty: Option<[u16; 2]>) -> Result<(BoxW, BoxR)> {
    let pty = pty.map(|[c, r]| (c, r));
    match target(d, device)? {
        Target::Remote(c) => {
            let (w, r) = crate::rpc::open(&c, &Rpc::Exec { argv, cwd, pty }).await?;
            Ok((Box::new(w), Box::new(r)))
        }
        Target::Local => {
            let (a, b) = tokio::io::duplex(1 << 16);
            let (br, bw) = tokio::io::split(b);
            tokio::spawn(async move {
                let _ = crate::exec::serve(bw, br, argv, cwd, pty).await;
            });
            let (ar, aw) = tokio::io::split(a);
            Ok((Box::new(aw), Box::new(ar)))
        }
    }
}

async fn bridge_exec<LR, LW>(mut lr: LR, mut lw: LW, mut w: BoxW, mut r: BoxR) -> Result<()>
where
    LR: AsyncRead + Unpin + Send + 'static,
    LW: AsyncWrite + Unpin + Send + 'static,
{
    let up = tokio::spawn(async move {
        while let Ok(Some(f)) = read_frame(&mut lr).await {
            let Ok(f) = serde_json::from_slice::<ExecFrame>(&f) else { continue };
            let m = match f {
                ExecFrame::Stdin { data } => ExecIn::Stdin(data),
                ExecFrame::Eof => ExecIn::Eof,
                ExecFrame::Resize { cols, rows } => ExecIn::Resize(cols, rows),
                _ => continue,
            };
            if proto::send(&mut w, &m).await.is_err() {
                break;
            }
        }
        let _ = proto::send(&mut w, &ExecIn::Kill).await;
    });
    while let Some(m) = proto::recv::<ExecOut>(&mut r).await? {
        let f = match m {
            ExecOut::Started => continue,
            ExecOut::Stdout(data) => ExecFrame::Stdout { data },
            ExecOut::Stderr(data) => ExecFrame::Stderr { data },
            ExecOut::Exit(code) => ExecFrame::Exit { code },
            ExecOut::Error(message) => ExecFrame::Error { message },
        };
        let last = matches!(f, ExecFrame::Exit { .. } | ExecFrame::Error { .. });
        write_frame(&mut lw, &serde_json::to_vec(&f)?).await?;
        if last {
            break;
        }
    }
    up.abort();
    Ok(())
}

/// Видео трансляции по `[link] screen_codec` / `screen_bitrate`.
fn video_params(transport: link::Transport) -> Option<VideoParams> {
    let (cfg, _) = synshell_common::config::Config::load();
    let l = cfg.link;
    let codec = l.screen_codec.trim().to_lowercase();
    let (codec, mixed) = match codec.as_str() {
        "lossless" | "off" | "none" => return None,
        "h264" | "hevc" => (codec, false),
        _ => ("auto".to_string(), true),
    };
    let mbps = match (l.screen_bitrate, transport) {
        (0, link::Transport::Usb) => 80,
        (0, link::Transport::Wifi) => 30,
        (m, _) => m.clamp(1, 400),
    };
    Some(VideoParams { codec, bitrate: mbps * 1_000_000, mixed })
}

async fn bridge_screen_v2<LR, LW>(mut lr: LR, mut lw: LW, mut w: quinn::SendStream, mut r: quinn::RecvStream) -> Result<()>
where
    LR: AsyncRead + Unpin + Send + 'static,
    LW: AsyncWrite + Unpin + Send + 'static,
{
    let up = tokio::spawn(async move {
        while let Ok(Some(f)) = read_frame(&mut lr).await {
            let Ok(a) = serde_json::from_slice::<link::ScreenAck>(&f) else { continue };
            if proto::send(&mut w, &ScreenAckV2 { seq: a.seq, key: a.key }).await.is_err() {
                break;
            }
        }
        let _ = w.finish();
    });
    let res = async {
        while let Some(f) = proto::recv::<ScreenFrameV2>(&mut r).await? {
            let header = ScreenHeader {
                output: f.output,
                width: f.width,
                height: f.height,
                scale: f.scale,
                seq: f.seq,
                rects: f.rects.iter().map(|(x, y, w, h, z)| [*x, *y, *w, *h, z.len() as u32]).collect(),
                pointer: f.pointer,
                video: f.video.as_ref().map(|v| link::ScreenVideo {
                    codec: v.codec.clone(),
                    key: v.key,
                    len: v.data.len() as u32,
                    rects: v.rects.clone(),
                }),
                video_error: f.video_error,
            };
            let hj = serde_json::to_vec(&header)?;
            let data: usize = f.rects.iter().map(|r| r.4.len()).sum::<usize>() + f.video.as_ref().map_or(0, |v| v.data.len());
            let mut pkt = Vec::with_capacity(4 + hj.len() + data);
            pkt.extend_from_slice(&(hj.len() as u32).to_be_bytes());
            pkt.extend_from_slice(&hj);
            for (.., z) in &f.rects {
                pkt.extend_from_slice(z);
            }
            if let Some(v) = &f.video {
                pkt.extend_from_slice(&v.data);
            }
            write_frame(&mut lw, &pkt).await?;
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;
    up.abort();
    res
}

async fn bridge_screen<LR, LW>(mut lr: LR, mut lw: LW, mut w: quinn::SendStream, mut r: quinn::RecvStream) -> Result<()>
where
    LR: AsyncRead + Unpin + Send + 'static,
    LW: AsyncWrite + Unpin + Send + 'static,
{
    let up = tokio::spawn(async move {
        while let Ok(Some(f)) = read_frame(&mut lr).await {
            let Ok(a) = serde_json::from_slice::<link::ScreenAck>(&f) else { continue };
            if proto::send(&mut w, &ScreenAck { seq: a.seq }).await.is_err() {
                break;
            }
        }
        let _ = w.finish();
    });
    let res = async {
        while let Some(f) = proto::recv::<ScreenFrame>(&mut r).await? {
            let header = ScreenHeader {
                output: f.output,
                width: f.width,
                height: f.height,
                scale: f.scale,
                seq: f.seq,
                rects: f.rects.iter().map(|(x, y, w, h, z)| [*x, *y, *w, *h, z.len() as u32]).collect(),
                pointer: f.pointer,
                video: None,
                video_error: None,
            };
            let hj = serde_json::to_vec(&header)?;
            let mut pkt = Vec::with_capacity(4 + hj.len() + f.rects.iter().map(|r| r.4.len()).sum::<usize>());
            pkt.extend_from_slice(&(hj.len() as u32).to_be_bytes());
            pkt.extend_from_slice(&hj);
            for (.., z) in &f.rects {
                pkt.extend_from_slice(z);
            }
            write_frame(&mut lw, &pkt).await?;
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;
    up.abort();
    res
}

async fn handle(d: &D, req: Request) -> Result<Response> {
    Ok(match req {
        Request::Status => Response::Status { status: d.status() },
        Request::Pair { device } => {
            let id = d.resolve(&device).with_context(|| format!("нет устройства «{device}»"))?;
            d.clone().pair(id).await?;
            Response::Ok
        }
        Request::PairReply { device, accept } => {
            let id = d.resolve(&device).unwrap_or(device);
            d.pair_reply(&id, accept)?;
            Response::Ok
        }
        Request::Unpair { device } => {
            let id = d.resolve(&device).with_context(|| format!("нет устройства «{device}»"))?;
            d.unpair(&id)?;
            Response::Ok
        }
        Request::Disconnect { device } => {
            let id = d.resolve(&device).with_context(|| format!("нет устройства «{device}»"))?;
            d.disconnect(&id)?;
            Response::Ok
        }
        Request::Configure { name, discoverable } => {
            d.configure(name, discoverable);
            Response::Ok
        }
        Request::Screenshot { device, output, path, max_size } => {
            let (name, mut w, mut h, mut png) = match target(d, &device)? {
                Target::Local => tokio::task::spawn_blocking(move || crate::services::screenshot(output)).await??,
                Target::Remote(c) => match crate::rpc::call(&c, &Rpc::Screenshot { output }).await? {
                    Reply::Shot { output, width, height, png } => (output, width, height, png),
                    _ => bail!("неожиданный ответ"),
                },
            };
            if let Some(m) = max_size.filter(|m| *m > 0) {
                let (nw, nh, p) = tokio::task::spawn_blocking(move || crate::services::downscale_png(&png, m)).await??;
                (w, h, png) = (nw, nh, p);
            }
            let path = match path {
                Some(p) => std::path::PathBuf::from(p),
                None => {
                    let dir = synshell_common::paths::runtime_dir().join("synlink-shots");
                    std::fs::create_dir_all(&dir)?;
                    let dev = if device.is_empty() { "local".to_string() } else { crate::ssh::slug(&device) };
                    dir.join(format!("{dev}-{}.png", crate::identity::now()))
                }
            };
            std::fs::write(&path, &png)?;
            Response::Screenshot { path: path.to_string_lossy().into_owned(), width: w, height: h, output: name }
        }
        Request::Input { device, output, events } => {
            match target(d, &device)? {
                Target::Local => tokio::task::spawn_blocking(move || crate::services::input(output, events)).await??,
                Target::Remote(c) => {
                    crate::rpc::call(&c, &Rpc::Input { output, events: serde_json::to_string(&events)? }).await?;
                }
            }
            Response::Ok
        }
        Request::Wm { device, wm } => {
            let resp = match target(d, &device)? {
                Target::Local => tokio::task::spawn_blocking(move || crate::services::wm(wm)).await??,
                Target::Remote(c) => match crate::rpc::call(&c, &Rpc::Wm { request: serde_json::to_string(&wm)? }).await? {
                    Reply::Json(j) => serde_json::from_str(&j)?,
                    _ => bail!("неожиданный ответ"),
                },
            };
            Response::Wm { wm: resp }
        }
        Request::Exec { device, argv, cwd, pty, timeout_ms, .. } => {
            let (w, r) = exec_streams(d, &device, argv, cwd, pty).await?;
            let timeout = Duration::from_millis(timeout_ms.unwrap_or(120_000));
            let (code, out, err) = crate::exec::collect(w, r, None, timeout).await?;
            Response::Exec {
                code,
                stdout: String::from_utf8_lossy(&out).into_owned(),
                stderr: String::from_utf8_lossy(&err).into_owned(),
            }
        }
        Request::Mount { device, mount } => {
            let id = d.resolve(&device).with_context(|| format!("нет устройства «{device}»"))?;
            if mount {
                Response::Mount { path: Some(crate::fuse::mount(d, &id).await?) }
            } else {
                crate::fuse::unmount(d, &id);
                Response::Mount { path: None }
            }
        }
        Request::Notifications { device } => {
            let id = device.and_then(|x| d.resolve(&x));
            Response::Notifications { notifications: d.notes(id.as_deref()) }
        }
        Request::Subscribe | Request::Tcp { .. } | Request::Screen { .. } | Request::Audio { .. } => {
            Response::Error { message: "не здесь".into() }
        }
    })
}
