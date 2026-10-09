//! Звук трансляции (`Rpc::Audio`, `link::Request::Audio`): звук смотримого
//! устройства (монитор его выхода по умолчанию PipeWire) играет у зрителя, а
//! микрофон зрителя (источник по умолчанию) появляется на устройстве
//! виртуальным источником `synlink-mic` — на время звука он там источник по
//! умолчанию, потом прежний возвращается.
//!
//! Запись и воспроизведение — `pw-cat` подпроцессами (сырой PCM s16le,
//! 48 кГц, стерео через каналы): libpipewire в сборке не нужен, а нет
//! PipeWire — понятная ошибка, а не зависший демон. Виртуальный микрофон —
//! поток воспроизведения `pw-cat` с `media.class = Audio/Source` без цели:
//! программы той стороны видят его обычным источником, пропадает он вместе с
//! процессом.
//!
//! Задержка: куски по 10 мс; у отправителя очередь ≤ 60 мс (лишнее
//! выбрасывается), у получателя ≤ 150 мс — отстали (Wi-Fi застрял и выдал
//! пачку, часы устройств разошлись) — накопленное сбрасывается до 40 мс, а не
//! копится. Канал к `pw-cat` — 4 КБ (≈ 20 мс). Тишина идёт не байтами, а
//! длиной.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::json;
use tokio::sync::{mpsc, oneshot};

use crate::daemon::D;
use crate::proto::{self, AudioMsg, Reply, Rpc};
use synshell_tr::t;

/// s16 × 2 канала.
const FRAME: usize = 4;
/// 10 мс при 48 кГц.
const CHUNK: usize = 480 * FRAME;
/// Очередь отправителя, кусков.
const SEND_QUEUE: usize = 6;
/// У получателя больше этого — сбросить до `PLAY_KEEP`.
const PLAY_MAX: usize = CHUNK * 15;
const PLAY_KEEP: usize = CHUNK * 4;
/// Имя виртуального микрофона на смотримом устройстве.
pub const MIC_NODE: &str = "synlink-mic";
/// С какой версии протокола устройство умеет звук.
pub const MIN_PROTO: u32 = 3;
const DEFAULT_SOURCE: &str = "default.configured.audio.source";

/// Звук, включённый с этой стороны (мы — зритель): устройство → (номер, задача).
pub type Sessions = Mutex<HashMap<String, (u64, tokio::task::AbortHandle)>>;

// ─── pw-cat ──────────────────────────────────────────────────────────────────

/// `pw-cat` записи (`-r`) или воспроизведения (`-p`): сырой PCM через
/// stdout/stdin. Не соединился с PipeWire — ошибка с его сообщением.
fn pw_cat(mode: &str, latency: &str, target: Option<&str>, props: serde_json::Value) -> Result<Child> {
    let mut c = Command::new("pw-cat");
    c.args([mode, "--raw", "--format", "s16", "--rate", "48000", "--channels", "2", "--latency", latency]);
    if let Some(t) = target {
        c.args(["--target", t]);
    }
    c.arg("-P").arg(props.to_string()).arg("-");
    if mode == "-p" {
        c.stdin(Stdio::piped()).stdout(Stdio::null());
    } else {
        c.stdin(Stdio::null()).stdout(Stdio::piped());
    }
    c.stderr(Stdio::piped());
    let mut child = c.spawn().context("pw-cat (пакет pipewire) не запускается")?;
    // Нет PipeWire или не тот формат — pw-cat выходит сразу.
    std::thread::sleep(Duration::from_millis(150));
    if let Some(st) = child.try_wait()? {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
        let why = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
        bail!("pw-cat: {} ({st})", if why.is_empty() { "не запустился".into() } else { why });
    }
    if let Some(e) = child.stderr.take() {
        std::thread::spawn(move || {
            for l in std::io::BufReader::new(e).lines().map_while(Result::ok) {
                tracing::debug!("pw-cat: {l}");
            }
        });
    }
    Ok(child)
}

fn term(pid: u32) {
    unsafe { libc::kill(pid as i32, libc::SIGTERM) };
}

/// Запись: куски по 10 мс в канал; не успевают уходить — лишние выбрасываются.
struct Capture {
    rx: mpsc::Receiver<Vec<u8>>,
    pid: u32,
    /// Процесс уже дождались (pid мог достаться другому).
    exited: Arc<Mutex<bool>>,
}

impl Capture {
    fn start(props: serde_json::Value) -> Result<Self> {
        let mut child = pw_cat("-r", "10ms", None, props)?;
        let mut out = child.stdout.take().context("stdout pw-cat")?;
        let pid = child.id();
        let (tx, rx) = mpsc::channel(SEND_QUEUE);
        let exited = Arc::new(Mutex::new(false));
        let ex = exited.clone();
        std::thread::Builder::new().name("synlink-rec".into()).spawn(move || {
            loop {
                let mut b = vec![0u8; CHUNK];
                if out.read_exact(&mut b).is_err() {
                    break;
                }
                if let Err(mpsc::error::TrySendError::Closed(_)) = tx.try_send(b) {
                    break;
                }
            }
            let mut g = ex.lock().unwrap();
            if !*g {
                term(child.id());
            }
            let _ = child.wait();
            *g = true;
        })?;
        Ok(Self { rx, pid, exited })
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        // Поток ждёт в чтении — конец записи разбудит его.
        self.rx.close();
        let mut g = self.exited.lock().unwrap();
        if !*g {
            term(self.pid);
            *g = true;
        }
    }
}

#[derive(Default)]
struct PlayQ {
    data: VecDeque<u8>,
    closed: bool,
    exited: bool,
}

/// Воспроизведение: очередь с ограничением задержки, пишет в `pw-cat` свой поток.
struct Player {
    q: Arc<(Mutex<PlayQ>, Condvar)>,
    pid: u32,
}

impl Player {
    fn start(target: Option<&str>, props: serde_json::Value) -> Result<Self> {
        let mut child = pw_cat("-p", "20ms", target, props)?;
        let mut stdin = child.stdin.take().context("stdin pw-cat")?;
        {
            use std::os::fd::AsRawFd;
            // Маленький канал: в нём не копится задержка (по умолчанию 64 КБ ≈ 340 мс).
            unsafe { libc::fcntl(stdin.as_raw_fd(), libc::F_SETPIPE_SZ, 4096) };
        }
        let pid = child.id();
        let q: Arc<(Mutex<PlayQ>, Condvar)> = Arc::default();
        let q2 = q.clone();
        std::thread::Builder::new().name("synlink-play".into()).spawn(move || {
            let (m, cv) = &*q2;
            loop {
                let chunk: Vec<u8> = {
                    let mut g = m.lock().unwrap();
                    while g.data.is_empty() && !g.closed {
                        g = cv.wait(g).unwrap();
                    }
                    if g.closed {
                        break;
                    }
                    let n = g.data.len().min(CHUNK);
                    g.data.drain(..n).collect()
                };
                // Никто не слушает виртуальный микрофон — pw-cat не читает,
                // запись ждёт; очередь тем временем обрезается в `push`.
                if stdin.write_all(&chunk).is_err() {
                    break;
                }
            }
            drop(stdin);
            let mut g = m.lock().unwrap();
            if !g.exited {
                term(child.id());
            }
            let _ = child.wait();
            g.exited = true;
        })?;
        Ok(Self { q, pid })
    }

    fn push(&self, pcm: &[u8]) {
        let (m, cv) = &*self.q;
        let mut g = m.lock().unwrap();
        g.data.extend(pcm);
        if g.data.len() > PLAY_MAX {
            let mut cut = g.data.len() - PLAY_KEEP;
            cut -= cut % FRAME;
            g.data.drain(..cut);
        }
        cv.notify_one();
    }

    fn push_silence(&self, n: usize) {
        self.push(&vec![0u8; n.min(PLAY_MAX) / FRAME * FRAME]);
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let (m, cv) = &*self.q;
        let mut g = m.lock().unwrap();
        g.closed = true;
        // Поток мог застрять в записи в канал — конец pw-cat его отпустит.
        if !g.exited {
            term(self.pid);
            g.exited = true;
        }
        cv.notify_one();
    }
}

/// Перекачка: запись этой стороны → туда, оттуда → воспроизведение.
async fn pump(mut w: quinn::SendStream, mut r: quinn::RecvStream, cap: Option<Capture>, play: Option<Player>) -> Result<()> {
    let recv = async move {
        while let Some(m) = proto::recv::<AudioMsg>(&mut r).await? {
            match m {
                AudioMsg::Pcm(p) => {
                    if let Some(pl) = &play {
                        pl.push(&p);
                    }
                }
                AudioMsg::Silence(n) => {
                    if let Some(pl) = &play {
                        pl.push_silence(n as usize);
                    }
                }
                AudioMsg::Error(e) => bail!("{e}"),
            }
        }
        Ok(())
    };
    let send = async move {
        let Some(mut cap) = cap else {
            return std::future::pending::<Result<()>>().await;
        };
        while let Some(b) = cap.rx.recv().await {
            let m = if b.iter().all(|&x| x == 0) { AudioMsg::Silence(b.len() as u32) } else { AudioMsg::Pcm(b) };
            proto::send(&mut w, &m).await?;
        }
        let why = t!("запись звука прервалась (pw-cat)");
        let _ = proto::send(&mut w, &AudioMsg::Error(why.clone())).await;
        bail!(why)
    };
    tokio::select! {
        r = recv => r,
        r = send => r,
    }
}

// ─── источник по умолчанию ──────────────────────────────────────────────────

/// Значение ключа метаданных `default` (JSON) через `pw-metadata`.
fn metadata_get(key: &str) -> Option<String> {
    let out = Command::new("pw-metadata").args(["0", key]).stderr(Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.contains(&format!("key:'{key}'")))?;
    let v = line.split_once("value:'")?.1;
    Some(v.rsplit_once("' type:")?.0.to_string())
}

fn metadata_set(key: &str, value: Option<&str>) {
    let mut c = Command::new("pw-metadata");
    match value {
        Some(v) => c.args(["0", key, v, "Spa:String:JSON"]),
        None => c.args(["-d", "0", key]),
    };
    if let Err(e) = c.stdout(Stdio::null()).stderr(Stdio::null()).status() {
        tracing::warn!("pw-metadata: {e}");
    }
}

/// Виртуальный микрофон — источник по умолчанию, пока жив этот страж.
struct DefaultMic {
    prev: Option<String>,
}

impl DefaultMic {
    fn take() -> Self {
        // Остался от упавшего демона — не «прежний».
        let prev = metadata_get(DEFAULT_SOURCE).filter(|v| !v.contains(MIC_NODE));
        metadata_set(DEFAULT_SOURCE, Some(&json!({ "name": MIC_NODE }).to_string()));
        Self { prev }
    }
}

impl Drop for DefaultMic {
    fn drop(&mut self) {
        let prev = self.prev.take();
        std::thread::spawn(move || metadata_set(DEFAULT_SOURCE, prev.as_deref()));
    }
}

/// При запуске демона: источник по умолчанию указывает на `synlink-mic`
/// (демон упал, не вернув прежний) — убрать, WirePlumber выберет сам.
pub fn cleanup_stale() {
    if metadata_get(DEFAULT_SOURCE).is_some_and(|v| v.contains(MIC_NODE)) {
        metadata_set(DEFAULT_SOURCE, None);
    }
}

// ─── смотримое устройство (сервер вызова) ─────────────────────────────────────

/// `Rpc::Audio` от зрителя `peer`: монитор выхода → туда, его микрофон →
/// виртуальный источник.
pub async fn serve(d: D, peer: String, mut w: quinn::SendStream, r: quinn::RecvStream, mic: bool) -> Result<()> {
    let name = d.peer_info(&peer).map(|p| p.name).unwrap_or_else(|| t!("устройства").into());
    let n = name.clone();
    let started = tokio::task::spawn_blocking(move || -> Result<(Capture, Option<Player>, Option<DefaultMic>)> {
        let cap = Capture::start(json!({
            "stream.capture.sink": true,
            "node.name": "synlink-monitor",
            "node.description": t!("Звук для «{n}»", n = n),
            "media.name": t!("Звук для «{n}»", n = n),
            "application.name": "synlink",
        }))?;
        let (play, default) = if mic {
            let p = Player::start(
                Some("0"),
                json!({
                    "media.class": "Audio/Source",
                    "node.name": MIC_NODE,
                    "node.description": t!("Микрофон {n}", n = n),
                    "media.name": t!("Микрофон {n}", n = n),
                    "application.name": "synlink",
                }),
            )?;
            (Some(p), Some(DefaultMic::take()))
        } else {
            (None, None)
        };
        Ok((cap, play, default))
    })
    .await?;
    let (cap, play, default) = match started {
        Ok(x) => x,
        Err(e) => {
            tracing::warn!("звук трансляции для «{name}»: {e:#}");
            proto::send(&mut w, &Reply::Err(format!("{e:#}"))).await?;
            return Ok(());
        }
    };
    proto::send(&mut w, &Reply::Done).await?;
    tracing::info!(mic, "звук трансляции для «{name}»: начат");
    let res = pump(w, r, Some(cap), play).await;
    drop(default);
    tracing::info!("звук трансляции для «{name}»: конец ({})", res.as_ref().map(|_| "закрыт".to_string()).unwrap_or_else(|e| format!("{e:#}")));
    Ok(())
}

// ─── зритель (запрос с локального сокета) ────────────────────────────────────

/// Включить звук с устройством: его звук играет здесь, наш микрофон (`mic`)
/// уходит туда. Ответ — id устройства, номер сеанса и приёмник причины конца.
pub async fn start(d: &D, device: &str, mic: bool) -> Result<(String, u64, oneshot::Receiver<String>)> {
    let id = d.resolve(device).with_context(|| format!("нет устройства «{device}»"))?;
    if id == d.id.id {
        bail!("звук своего устройства не нужен");
    }
    let (conn, _) = d.conn(&id).with_context(|| format!("«{device}» не соединено"))?;
    let name = d.peer_info(&id).map(|p| p.name).unwrap_or_else(|| device.to_string());
    let proto_ver = d.session_info(&id).map(|(p, _)| p).unwrap_or(0);
    if proto_ver < MIN_PROTO {
        bail!("на «{name}» synlink без звука трансляции — обновите synshell там");
    }
    stop(d, &id);
    let (w, mut r) = crate::rpc::open(&conn, &Rpc::Audio { mic }).await?;
    match tokio::time::timeout(Duration::from_secs(8), proto::recv::<Reply>(&mut r)).await {
        Ok(Ok(Some(Reply::Done))) => {}
        Ok(Ok(Some(Reply::Err(e)))) => bail!("«{name}»: {e}"),
        Ok(Ok(_)) => bail!("«{name}» не ответило"),
        Ok(Err(e)) => bail!("«{name}»: {e:#}"),
        Err(_) => bail!("«{name}» не ответило вовремя"),
    }
    let n = name.clone();
    let (play, cap) = tokio::task::spawn_blocking(move || -> Result<(Player, Option<Capture>)> {
        let play = Player::start(
            None,
            json!({
                "node.name": "synlink-audio",
                "node.description": t!("Звук {n}", n = n),
                "media.name": t!("Звук «{n}»", n = n),
                "application.name": "synlink",
            }),
        )?;
        let cap = if mic {
            Some(Capture::start(json!({
                "node.name": "synlink-mic-capture",
                "node.description": t!("Микрофон для «{n}»", n = n),
                "media.name": t!("Микрофон для «{n}»", n = n),
                "application.name": "synlink",
            }))?)
        } else {
            None
        };
        Ok((play, cap))
    })
    .await??;
    let (done_tx, done_rx) = oneshot::channel();
    let gen = {
        static GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    };
    {
        // Под замком: задача, закончившись сразу, найдёт себя в списке.
        let mut map = d.audio.lock().unwrap();
        let d2 = d.clone();
        let id2 = id.clone();
        let task = tokio::spawn(async move {
            let res = pump(w, r, cap, Some(play)).await;
            let msg = match res {
                Ok(()) => t!("«{name}» закончило звук", name = name),
                Err(e) => t!("звук «{name}»: {e}", name = name, e = format!("{:#}", e)),
            };
            tracing::info!("{msg}");
            let removed = {
                let mut map = d2.audio.lock().unwrap();
                if map.get(&id2).is_some_and(|(g, _)| *g == gen) {
                    map.remove(&id2);
                    true
                } else {
                    false
                }
            };
            if removed {
                d2.emit_status();
            }
            let _ = done_tx.send(msg);
        });
        map.insert(id.clone(), (gen, task.abort_handle()));
    }
    tracing::info!(mic, "звук трансляции с «{device}»: начат");
    d.emit_status();
    Ok((id, gen, done_rx))
}

/// Выключить звук с устройством.
pub fn stop(d: &D, id: &str) -> bool {
    let run = d.audio.lock().unwrap().remove(id);
    if let Some((_, h)) = &run {
        h.abort();
        d.emit_status();
    }
    run.is_some()
}

/// Выключить, если это всё ещё сеанс `gen` (окно закрылось).
pub fn stop_gen(d: &D, id: &str, gen: u64) {
    let run = {
        let mut map = d.audio.lock().unwrap();
        if map.get(id).is_some_and(|(g, _)| *g == gen) {
            map.remove(id)
        } else {
            None
        }
    };
    if let Some((_, h)) = run {
        h.abort();
        d.emit_status();
    }
}
