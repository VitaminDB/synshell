//! Трансляция экрана: кадры из потока композитора (только изменившиеся
//! прямоугольники), сжатие zstd, не больше двух кадров «в пути» — на
//! медленном Wi-Fi частота падает сама, а не копится очередь.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use synshell_common::ipc::{FrameInfo, Request, Response};
use tokio::sync::mpsc;

use crate::proto::{self, ScreenAck, ScreenFrame};

/// Сколько кадров может быть отправлено без подтверждения.
const IN_FLIGHT: u64 = 2;

pub async fn serve(mut w: quinn::SendStream, mut r: quinn::RecvStream, output: Option<String>, cursor: bool) -> Result<()> {
    let (ftx, mut frx) = mpsc::channel::<Result<ScreenFrame, String>>(2);
    let stop = Arc::new(AtomicBool::new(false));
    let acked = Arc::new(AtomicU64::new(u64::MAX));
    {
        let (stop, acked) = (stop.clone(), acked.clone());
        std::thread::Builder::new()
            .name("synlink-screen".into())
            .spawn(move || {
                if let Err(e) = capture_loop(output, cursor, &ftx, &stop, &acked) {
                    let _ = ftx.blocking_send(Err(format!("{e:#}")));
                }
            })?;
    }
    let reader = {
        let acked = acked.clone();
        tokio::spawn(async move {
            while let Ok(Some(a)) = proto::recv::<ScreenAck>(&mut r).await {
                acked.store(a.seq, Ordering::Release);
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
    stop.store(true, Ordering::Release);
    reader.abort();
    let _ = w.finish();
    res
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
    tx: &mpsc::Sender<Result<ScreenFrame, String>>,
    stop: &AtomicBool,
    acked: &AtomicU64,
) -> Result<()> {
    let mut wm = Wm::connect()?;
    wm.send(&Request::FrameStream { output, cursor })?;
    let Some(mut info) = wm.wait(stop)? else { return Ok(()) };
    let file = std::fs::File::open(&info.path).context("файл кадра")?;
    let mut sent = 0u64;
    loop {
        let mut frame = pack(&file, &info)?;
        // Свой сквозной номер: у композитора полный кадр снова с нуля.
        frame.seq = sent;
        if tx.blocking_send(Ok(frame)).is_err() {
            return Ok(());
        }
        sent += 1;
        // Ждать подтверждения, пока в пути слишком много.
        while !stop.load(Ordering::Acquire) {
            let a = acked.load(Ordering::Acquire);
            let done = if a == u64::MAX { 0 } else { a + 1 };
            if sent.saturating_sub(done) < IN_FLIGHT {
                break;
            }
            std::thread::sleep(Duration::from_millis(4));
        }
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        wm.send(&Request::FrameNext)?;
        match wm.wait(stop)? {
            Some(i) => info = i,
            None => return Ok(()),
        }
    }
}

fn pack(file: &std::fs::File, info: &FrameInfo) -> Result<ScreenFrame> {
    let stride = info.width as u64 * 4;
    let mut rects = Vec::with_capacity(info.rects.len());
    let mut raw = Vec::new();
    for [x, y, w, h] in info.rects.iter().copied() {
        let (x, y, w, h) = (x.max(0) as u32, y.max(0) as u32, w.max(0) as u32, h.max(0) as u32);
        let row = w as usize * 4;
        raw.resize(row * h as usize, 0);
        for i in 0..h as usize {
            let off = (y as u64 + i as u64) * stride + x as u64 * 4;
            file.read_exact_at(&mut raw[i * row..(i + 1) * row], off)?;
        }
        let z = zstd::bulk::compress(&raw, 1)?;
        rects.push((x, y, w, h, z));
    }
    Ok(ScreenFrame {
        output: info.output.clone(),
        width: info.width,
        height: info.height,
        scale: info.scale,
        seq: info.seq,
        rects,
        pointer: info.pointer,
    })
}
