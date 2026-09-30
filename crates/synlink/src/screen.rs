//! Трансляция экрана: кадры из потока композитора (только изменившиеся
//! прямоугольники), сжатие zstd, не больше двух кадров «в пути» — на
//! медленном Wi-Fi частота падает сама, а не копится очередь. С видео
//! (`ScreenV2`) крупные изменения приходят от композитора пакетом
//! аппаратного кодера (synwm `stream.rs`), он уходит как есть.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use synshell_common::ipc::{FrameInfo, Request, Response, VideoRequest};
use tokio::sync::mpsc;

use crate::proto::{self, ScreenAck, ScreenAckV2, ScreenFrame, ScreenFrameV2, VideoPacket, VideoParams};

/// Сколько кадров может быть отправлено без подтверждения. Кадры без потерь
/// бывают по мегабайту — больше двух в пути только копили бы задержку; видеокадры
/// по несколько КБ — окно шире, иначе частоту ограничивает время ответа Wi-Fi.
const IN_FLIGHT: u64 = 2;
const IN_FLIGHT_VIDEO: u64 = 6;
/// Кадр «маленький» (окно видео), если короче этого.
const SMALL_FRAME: usize = 256 << 10;

pub async fn serve(mut w: quinn::SendStream, mut r: quinn::RecvStream, output: Option<String>, cursor: bool) -> Result<()> {
    // Старый вызов (proto 1): без видео, свой формат кадра и подтверждения.
    let (ftx, mut frx) = mpsc::channel::<Result<ScreenFrameV2, String>>(2);
    let ctl = Arc::new(Ctl::default());
    spawn_capture(output, cursor, None, ftx, ctl.clone())?;
    let reader = {
        let ctl = ctl.clone();
        tokio::spawn(async move {
            while let Ok(Some(a)) = proto::recv::<ScreenAck>(&mut r).await {
                ctl.acked.store(a.seq, Ordering::Release);
            }
        })
    };
    let res = async {
        while let Some(f) = frx.recv().await {
            let f = f.map_err(|e| anyhow::anyhow!("{e}"))?;
            let old = ScreenFrame { output: f.output, width: f.width, height: f.height, scale: f.scale, seq: f.seq, rects: f.rects, pointer: f.pointer };
            proto::send(&mut w, &old).await?;
        }
        Ok(())
    }
    .await;
    ctl.stop.store(true, Ordering::Release);
    reader.abort();
    let _ = w.finish();
    res
}

/// Трансляция с видео: `video` — параметры кодера композитора.
pub async fn serve_v2(
    mut w: quinn::SendStream,
    mut r: quinn::RecvStream,
    output: Option<String>,
    cursor: bool,
    video: Option<VideoParams>,
) -> Result<()> {
    let (ftx, mut frx) = mpsc::channel::<Result<ScreenFrameV2, String>>(2);
    let ctl = Arc::new(Ctl::default());
    spawn_capture(output, cursor, video, ftx, ctl.clone())?;
    let reader = {
        let ctl = ctl.clone();
        tokio::spawn(async move {
            while let Ok(Some(a)) = proto::recv::<ScreenAckV2>(&mut r).await {
                if a.key {
                    ctl.key.store(true, Ordering::Release);
                }
                ctl.acked.store(a.seq, Ordering::Release);
            }
        })
    };
    let res = async {
        while let Some(f) = frx.recv().await {
            match f {
                Ok(frame) => proto::send(&mut w, &frame).await?,
                Err(e) => bail!("{e}"),
            }
        }
        Ok(())
    }
    .await;
    ctl.stop.store(true, Ordering::Release);
    reader.abort();
    let _ = w.finish();
    res
}

/// Связь сетевой части с потоком захвата.
struct Ctl {
    stop: AtomicBool,
    acked: AtomicU64,
    /// Зритель просит ключевой видеокадр.
    key: AtomicBool,
}

impl Default for Ctl {
    fn default() -> Self {
        Self { stop: AtomicBool::new(false), acked: AtomicU64::new(u64::MAX), key: AtomicBool::new(false) }
    }
}

fn spawn_capture(
    output: Option<String>,
    cursor: bool,
    video: Option<VideoParams>,
    tx: mpsc::Sender<Result<ScreenFrameV2, String>>,
    ctl: Arc<Ctl>,
) -> Result<()> {
    std::thread::Builder::new().name("synlink-screen".into()).spawn(move || {
        if let Err(e) = capture_loop(output, cursor, video, &tx, &ctl) {
            let _ = tx.blocking_send(Err(format!("{e:#}")));
        }
    })?;
    Ok(())
}

/// Своё соединение с композитором: чтение с тайм-аутом, чтобы поток
/// замечал отключение зрителя, даже когда экран неподвижен.
struct Wm {
    rd: BufReader<UnixStream>,
    wr: UnixStream,
}

impl Wm {
    fn connect() -> Result<Self> {
        let s = UnixStream::connect(synshell_common::paths::socket_path()).context("нет связи с композитором")?;
        s.set_read_timeout(Some(Duration::from_millis(500)))?;
        Ok(Self { wr: s.try_clone()?, rd: BufReader::new(s) })
    }
    fn send(&mut self, req: &Request) -> Result<()> {
        let mut line = serde_json::to_string(req)?;
        line.push('\n');
        self.wr.write_all(line.as_bytes())?;
        Ok(())
    }
    /// Ждать ответ; `None` — остановлено.
    fn wait(&mut self, stop: &AtomicBool) -> Result<Option<FrameInfo>> {
        let mut line = String::new();
        loop {
            if stop.load(Ordering::Acquire) {
                return Ok(None);
            }
            match self.rd.read_line(&mut line) {
                Ok(0) => bail!("композитор закрыл соединение"),
                Ok(_) => break,
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
                Err(e) => return Err(e.into()),
            }
        }
        match serde_json::from_str::<Response>(&line)? {
            Response::Frame { frame } => Ok(Some(frame)),
            Response::Error { message } => bail!("{}", crate::services::old_wm(&message).map(String::from).unwrap_or(message)),
            other => bail!("неожиданный ответ: {other:?}"),
        }
    }
}

fn capture_loop(
    output: Option<String>,
    cursor: bool,
    video: Option<VideoParams>,
    tx: &mpsc::Sender<Result<ScreenFrameV2, String>>,
    ctl: &Ctl,
) -> Result<()> {
    let stop = &ctl.stop;
    let mut wm = Wm::connect()?;
    let video = video.map(|v| VideoRequest { codec: v.codec, bitrate: v.bitrate, mixed: v.mixed });
    wm.send(&Request::FrameStream { output, cursor, video })?;
    let Some(mut info) = wm.wait(stop)? else { return Ok(()) };
    let file = std::fs::File::open(&info.path).context("файл кадра")?;
    let mut sent = 0u64;
    loop {
        let mut frame = pack(&file, &info)?;
        let small = frame.rects.iter().map(|r| r.4.len()).sum::<usize>() + frame.video.as_ref().map_or(0, |v| v.data.len()) < SMALL_FRAME;
        let window = if small && frame.video.is_some() { IN_FLIGHT_VIDEO } else { IN_FLIGHT };
        // Свой сквозной номер: у композитора полный кадр снова с нуля.
        frame.seq = sent;
        if tx.blocking_send(Ok(frame)).is_err() {
            return Ok(());
        }
        sent += 1;
        // Ждать подтверждения, пока в пути слишком много.
        while !stop.load(Ordering::Acquire) {
            let a = ctl.acked.load(Ordering::Acquire);
            let done = if a == u64::MAX { 0 } else { a + 1 };
            if sent.saturating_sub(done) < window {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        let key = ctl.key.swap(false, Ordering::AcqRel);
        wm.send(&Request::FrameNext { key })?;
        match wm.wait(stop)? {
            Some(i) => info = i,
            None => return Ok(()),
        }
    }
}

/// Файл потока в памяти на время упаковки кадра.
struct Map {
    ptr: *mut u8,
    len: usize,
}

impl Map {
    fn new(file: &std::fs::File) -> Result<Self> {
        use std::os::fd::AsRawFd;
        let len = file.metadata()?.len() as usize;
        if len == 0 {
            return Ok(Self { ptr: std::ptr::null_mut(), len: 0 });
        }
        let p = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, file.as_raw_fd(), 0) };
        if p == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self { ptr: p.cast(), len })
    }
    fn get(&self, off: usize, n: usize) -> Result<&[u8]> {
        if off + n > self.len {
            bail!("файл кадра короче ожидаемого");
        }
        // SAFETY: в пределах отображения; композитор пишет в файл только до ответа.
        Ok(unsafe { std::slice::from_raw_parts(self.ptr.add(off), n) })
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { libc::munmap(self.ptr.cast(), self.len) };
        }
    }
}

fn pack(file: &std::fs::File, info: &FrameInfo) -> Result<ScreenFrameV2> {
    let map = Map::new(file)?;
    let stride = info.width as usize * 4;
    let mut rects = Vec::with_capacity(info.rects.len());
    let mut raw = Vec::new();
    for [x, y, w, h] in info.rects.iter().copied() {
        let (x, y, w, h) = (x.max(0) as u32, y.max(0) as u32, w.max(0) as u32, h.max(0) as u32);
        let row = w as usize * 4;
        raw.clear();
        raw.reserve(row * h as usize);
        for i in 0..h as usize {
            let off = (y as usize + i) * stride + x as usize * 4;
            raw.extend_from_slice(map.get(off, row)?);
        }
        let z = zstd::bulk::compress(&raw, 1)?;
        rects.push((x, y, w, h, z));
    }
    let video = match &info.video {
        Some(v) => Some(VideoPacket {
            codec: v.codec.clone(),
            key: v.key,
            data: map.get(v.offset as usize, v.len as usize)?.to_vec(),
            rects: v.rects.iter().map(|r| [r[0].max(0) as u32, r[1].max(0) as u32, r[2].max(0) as u32, r[3].max(0) as u32]).collect(),
        }),
        None => None,
    };
    Ok(ScreenFrameV2 {
        output: info.output.clone(),
        width: info.width,
        height: info.height,
        scale: info.scale,
        seq: info.seq,
        rects,
        pointer: info.pointer,
        video,
        video_error: info.video_error.clone(),
    })
}
