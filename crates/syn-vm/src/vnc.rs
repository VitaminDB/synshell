//! Экран машины: клиент RFB (VNC) на unix-сокете QEMU. Кадр — `LiveFrame` (RGBA),
//! курсор гостя рисуется поверх кадра у позиции мыши (псевдокодировка Cursor),
//! клавиши — расширенными событиями QEMU (номер клавиши, а не символ), размер
//! гостя под окно — SetDesktopSize, буфер обмена — расширенный (UTF-8, zlib).
//!
//! Потоки: читатель (разбор сообщений сервера, кадр в памяти) и публикатор
//! (до 60 раз в секунду собирает кадр с курсором и отдаёт виджету).

use std::io::{BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use syngui::widgets::LiveFrame;

const ENC_RAW: i32 = 0;
const ENC_COPYRECT: i32 = 1;
const ENC_CURSOR: i32 = -239;
const ENC_DESKTOP_SIZE: i32 = -223;
const ENC_POINTER_TYPE: i32 = -257;
const ENC_EXT_KEY: i32 = -258;
const ENC_EXT_DESKTOP_SIZE: i32 = -308;
const ENC_CLIPBOARD_EXT: i32 = 0xc0a1e5ceu32 as i32;

// Флаги расширенного буфера обмена.
const CB_TEXT: u32 = 1;
const CB_CAPS: u32 = 1 << 24;
const CB_REQUEST: u32 = 1 << 25;
const CB_PEEK: u32 = 1 << 26;
const CB_NOTIFY: u32 = 1 << 27;
const CB_PROVIDE: u32 = 1 << 28;

/// Что сообщает сеанс интерфейсу.
#[derive(Clone, Debug)]
pub enum Note {
    Connected { width: u32, height: u32 },
    Resized { width: u32, height: u32 },
    Disconnected,
    /// Гость скопировал текст.
    Clipboard(String),
}

struct Cursor {
    w: u32,
    h: u32,
    hx: i32,
    hy: i32,
    rgba: Vec<u8>,
}

#[derive(Default)]
struct Shared {
    w: u32,
    h: u32,
    fb: Vec<u8>,
    cursor: Option<Cursor>,
    /// Позиция мыши в пикселях гостя; `None` — мышь не над экраном.
    ptr: Option<(i32, i32)>,
    dirty: bool,
    ext_keys: bool,
    ext_desktop: bool,
    clipboard_ext: bool,
    /// Текст, который гость может запросить у нас.
    clip_out: Option<String>,
}

pub struct Session {
    pub frame: Arc<LiveFrame>,
    writer: Mutex<Option<UnixStream>>,
    sh: Mutex<Shared>,
    alive: AtomicBool,
    buttons: Mutex<u8>,
    wheel: Mutex<(f32, f32)>,
    /// Последний размер, о котором просили гостя.
    want_size: Mutex<Option<(u32, u32)>>,
}

type NoteFn = Arc<dyn Fn(Note) + Send + Sync>;

impl Session {
    /// Подключиться к сокету (ждёт его появления до `wait`), запустить потоки.
    pub fn open(sock: PathBuf, wait: Duration, on_frame: Arc<dyn Fn() + Send + Sync>, on_note: NoteFn) -> Arc<Session> {
        let s = Arc::new(Session {
            frame: LiveFrame::new(),
            writer: Mutex::new(None),
            sh: Mutex::new(Shared::default()),
            alive: AtomicBool::new(true),
            buttons: Mutex::new(0),
            wheel: Mutex::new((0.0, 0.0)),
            want_size: Mutex::new(None),
        });
        let me = s.clone();
        std::thread::Builder::new()
            .name("vnc-read".into())
            .spawn(move || {
                let t0 = Instant::now();
                let stream = loop {
                    match UnixStream::connect(&sock) {
                        Ok(st) => break Some(st),
                        Err(_) if t0.elapsed() < wait && me.alive.load(Ordering::Relaxed) => std::thread::sleep(Duration::from_millis(100)),
                        Err(e) => {
                            tracing::warn!("vnc {}: {e}", sock.display());
                            break None;
                        }
                    }
                };
                if let Some(stream) = stream {
                    let pub_me = me.clone();
                    let pub_frame = on_frame.clone();
                    std::thread::Builder::new().name("vnc-publish".into()).spawn(move || pub_me.publisher(pub_frame)).ok();
                    if let Err(e) = me.run(stream, &on_note) {
                        tracing::info!("vnc: {e}");
                    }
                }
                me.alive.store(false, Ordering::Relaxed);
                on_note(Note::Disconnected);
            })
            .ok();
        s
    }

    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    pub fn close(&self) {
        self.alive.store(false, Ordering::Relaxed);
        if let Some(w) = self.writer.lock().unwrap().take() {
            let _ = w.shutdown(std::net::Shutdown::Both);
        }
    }

    pub fn size(&self) -> (u32, u32) {
        let g = self.sh.lock().unwrap();
        (g.w, g.h)
    }

    fn send(&self, bytes: &[u8]) {
        let mut g = self.writer.lock().unwrap();
        if let Some(w) = g.as_mut() {
            if w.write_all(bytes).is_err() {
                *g = None;
            }
        }
    }

    // ─── Ввод ───────────────────────────────────────────────────────────

    /// Мышь в долях кадра.
    pub fn pointer(&self, fx: f32, fy: f32) {
        let (x, y) = {
            let mut g = self.sh.lock().unwrap();
            if g.w == 0 {
                return;
            }
            let x = (fx * g.w as f32).floor().clamp(0.0, g.w as f32 - 1.0) as i32;
            let y = (fy * g.h as f32).floor().clamp(0.0, g.h as f32 - 1.0) as i32;
            let inside = (0.0..=1.0).contains(&fx) && (0.0..=1.0).contains(&fy);
            let np = if inside { Some((x, y)) } else { None };
            if g.ptr != np {
                g.ptr = np;
                if g.cursor.is_some() {
                    g.dirty = true;
                }
            }
            (x, y)
        };
        let b = *self.buttons.lock().unwrap();
        self.send_pointer(b, x, y);
    }

    /// Кнопка мыши: 0 — левая, 1 — средняя, 2 — правая.
    pub fn button(&self, idx: u8, down: bool, fx: f32, fy: f32) {
        {
            let mut b = self.buttons.lock().unwrap();
            if down {
                *b |= 1 << idx;
            } else {
                *b &= !(1 << idx);
            }
        }
        self.pointer(fx, fy);
    }

    /// Колесо в шагах (дробные копятся: тачпад).
    pub fn wheel(&self, dx: f32, dy: f32) {
        let (sx, sy) = {
            let mut w = self.wheel.lock().unwrap();
            w.0 += dx;
            w.1 += dy;
            let sx = w.0.trunc() as i32;
            let sy = w.1.trunc() as i32;
            w.0 -= sx as f32;
            w.1 -= sy as f32;
            (sx, sy)
        };
        let (x, y) = self.sh.lock().unwrap().ptr.unwrap_or((0, 0));
        let b = *self.buttons.lock().unwrap();
        let click = |bit: u8, n: i32| {
            for _ in 0..n.unsigned_abs().min(10) {
                self.send_pointer(b | bit, x, y);
                self.send_pointer(b, x, y);
            }
        };
        if sy != 0 {
            click(if sy > 0 { 16 } else { 8 }, sy);
        }
        if sx != 0 {
            click(if sx > 0 { 64 } else { 32 }, sx);
        }
    }

    fn send_pointer(&self, mask: u8, x: i32, y: i32) {
        let mut m = [5u8, mask, 0, 0, 0, 0];
        m[2..4].copy_from_slice(&(x.max(0) as u16).to_be_bytes());
        m[4..6].copy_from_slice(&(y.max(0) as u16).to_be_bytes());
        self.send(&m);
    }

    /// Клавиша по номеру QEMU; `keysym` — для сервера без расширенных событий.
    pub fn key(&self, qnum: u32, keysym: u32, down: bool) {
        let ext = self.sh.lock().unwrap().ext_keys;
        if ext {
            let mut m = [0u8; 12];
            m[0] = 255;
            m[1] = 0;
            m[2..4].copy_from_slice(&(down as u16).to_be_bytes());
            m[4..8].copy_from_slice(&keysym.to_be_bytes());
            m[8..12].copy_from_slice(&qnum.to_be_bytes());
            self.send(&m);
        } else if keysym != 0 {
            let mut m = [4u8, down as u8, 0, 0, 0, 0, 0, 0];
            m[4..8].copy_from_slice(&keysym.to_be_bytes());
            self.send(&m);
        }
    }

    /// Сочетание: нажать по порядку, отпустить в обратном.
    pub fn combo(&self, keys: &[u32]) {
        for k in keys {
            self.key(*k, 0, true);
        }
        for k in keys.iter().rev() {
            self.key(*k, 0, false);
        }
    }

    /// Напечатать текст: символ за символом через keysym (латиница и знаки ASCII).
    pub fn type_text(&self, text: &str) {
        for c in text.chars() {
            let ks = match c {
                '\n' => 0xff0d,
                '\t' => 0xff09,
                c if (' '..='~').contains(&c) => c as u32,
                c => 0x0100_0000 + c as u32,
            };
            let mut m = [4u8, 1, 0, 0, 0, 0, 0, 0];
            m[4..8].copy_from_slice(&ks.to_be_bytes());
            self.send(&m);
            m[1] = 0;
            self.send(&m);
        }
    }

    /// Попросить гостя сменить разрешение (нужен virtio-vga/QXL и драйвер в госте).
    pub fn request_size(&self, w: u32, h: u32) {
        let w = (w.clamp(640, 8192) / 8) * 8;
        let h = h.clamp(480, 8192);
        {
            let mut want = self.want_size.lock().unwrap();
            *want = Some((w, h));
        }
        let ok = {
            let g = self.sh.lock().unwrap();
            g.ext_desktop && (g.w, g.h) != (w, h)
        };
        if ok {
            self.send_desktop_size(w, h);
        }
    }

    fn send_desktop_size(&self, w: u32, h: u32) {
        let mut m = Vec::with_capacity(24);
        m.extend_from_slice(&[251, 0]);
        m.extend_from_slice(&(w as u16).to_be_bytes());
        m.extend_from_slice(&(h as u16).to_be_bytes());
        m.extend_from_slice(&[1, 0]);
        m.extend_from_slice(&0u32.to_be_bytes());
        m.extend_from_slice(&0u16.to_be_bytes());
        m.extend_from_slice(&0u16.to_be_bytes());
        m.extend_from_slice(&(w as u16).to_be_bytes());
        m.extend_from_slice(&(h as u16).to_be_bytes());
        m.extend_from_slice(&0u32.to_be_bytes());
        self.send(&m);
    }

    /// Предложить гостю текст хоста (он запросит его, когда вставит).
    pub fn offer_clipboard(&self, text: String) {
        let ext = {
            let mut g = self.sh.lock().unwrap();
            if g.clip_out.as_deref() == Some(text.as_str()) {
                return;
            }
            g.clip_out = Some(text.clone());
            g.clipboard_ext
        };
        if ext {
            self.send_cut_ext(CB_NOTIFY | CB_TEXT, &[]);
        } else {
            // Простой ClientCutText — только Latin-1.
            let bytes: Vec<u8> = text.chars().filter_map(|c| u8::try_from(c as u32).ok()).collect();
            let mut m = vec![6u8, 0, 0, 0];
            m.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            m.extend_from_slice(&bytes);
            self.send(&m);
        }
    }

    fn send_cut_ext(&self, flags: u32, payload: &[u8]) {
        let mut m = vec![6u8, 0, 0, 0];
        let len = -((4 + payload.len()) as i32);
        m.extend_from_slice(&len.to_be_bytes());
        m.extend_from_slice(&flags.to_be_bytes());
        m.extend_from_slice(payload);
        self.send(&m);
    }

    fn provide_clipboard(&self) {
        let text = self.sh.lock().unwrap().clip_out.clone().unwrap_or_default();
        let mut raw = Vec::with_capacity(text.len() + 5);
        let body: Vec<u8> = text.replace('\n', "\r\n").into_bytes().into_iter().chain([0]).collect();
        raw.extend_from_slice(&(body.len() as u32).to_be_bytes());
        raw.extend_from_slice(&body);
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        let _ = z.write_all(&raw);
        let data = z.finish().unwrap_or_default();
        self.send_cut_ext(CB_PROVIDE | CB_TEXT, &data);
    }

    // ─── Кадр ───────────────────────────────────────────────────────────

    fn publisher(self: Arc<Self>, on_frame: Arc<dyn Fn() + Send + Sync>) {
        let period = Duration::from_millis(16);
        while self.alive() {
            std::thread::sleep(period);
            let out = {
                let mut g = self.sh.lock().unwrap();
                if !g.dirty || g.w == 0 {
                    continue;
                }
                g.dirty = false;
                let mut buf = g.fb.clone();
                if let (Some(c), Some((px, py))) = (&g.cursor, g.ptr) {
                    blend_cursor(&mut buf, g.w, g.h, c, px - c.hx, py - c.hy);
                }
                (g.w, g.h, buf)
            };
            self.frame.set(out.0, out.1, Arc::from(out.2));
            on_frame();
        }
    }

    fn run(&self, stream: UnixStream, on_note: &NoteFn) -> Result<()> {
        *self.writer.lock().unwrap() = Some(stream.try_clone()?);
        let mut r = BufReader::with_capacity(1 << 20, stream);
        // Версия.
        let mut ver = [0u8; 12];
        r.read_exact(&mut ver)?;
        if &ver[..4] != b"RFB " {
            bail!("не RFB");
        }
        self.send(b"RFB 003.008\n");
        // Безопасность: только None.
        let n = read_u8(&mut r)?;
        if n == 0 {
            let len = read_u32(&mut r)? as usize;
            let mut why = vec![0; len];
            r.read_exact(&mut why)?;
            bail!("VNC: {}", String::from_utf8_lossy(&why));
        }
        let mut types = vec![0u8; n as usize];
        r.read_exact(&mut types)?;
        if !types.contains(&1) {
            bail!("VNC: нужен вход без пароля");
        }
        self.send(&[1]);
        if read_u32(&mut r)? != 0 {
            bail!("VNC: вход отклонён");
        }
        self.send(&[1]); // ClientInit: общий доступ
        let w = read_u16(&mut r)? as u32;
        let h = read_u16(&mut r)? as u32;
        let mut pf = [0u8; 16];
        r.read_exact(&mut pf)?;
        let nl = read_u32(&mut r)? as usize;
        let mut name = vec![0; nl];
        r.read_exact(&mut name)?;
        self.resize(w, h);
        on_note(Note::Connected { width: w, height: h });

        // Формат точки: 32 бита, R G B X по возрастанию адресов.
        let mut m = vec![0u8, 0, 0, 0, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 0, 8, 16, 0, 0, 0];
        self.send(&m);
        let encs = [ENC_COPYRECT, ENC_RAW, ENC_CURSOR, ENC_DESKTOP_SIZE, ENC_EXT_DESKTOP_SIZE, ENC_EXT_KEY, ENC_POINTER_TYPE, ENC_CLIPBOARD_EXT];
        m = vec![2, 0];
        m.extend_from_slice(&(encs.len() as u16).to_be_bytes());
        for e in encs {
            m.extend_from_slice(&e.to_be_bytes());
        }
        self.send(&m);
        self.request(false);

        let mut tmp: Vec<u8> = Vec::new();
        loop {
            if !self.alive() {
                return Ok(());
            }
            match read_u8(&mut r)? {
                0 => {
                    let _ = read_u8(&mut r)?;
                    let n = read_u16(&mut r)?;
                    for _ in 0..n {
                        let x = read_u16(&mut r)? as u32;
                        let y = read_u16(&mut r)? as u32;
                        let w = read_u16(&mut r)? as u32;
                        let h = read_u16(&mut r)? as u32;
                        let enc = read_i32(&mut r)?;
                        self.rect(&mut r, &mut tmp, x, y, w, h, enc, on_note)?;
                    }
                    self.sh.lock().unwrap().dirty = true;
                    self.request(true);
                }
                1 => {
                    let mut hdr = [0u8; 5];
                    r.read_exact(&mut hdr)?;
                    let n = u16::from_be_bytes([hdr[3], hdr[4]]) as usize;
                    skip(&mut r, n * 6)?;
                }
                2 => {}
                3 => {
                    let mut pad = [0u8; 3];
                    r.read_exact(&mut pad)?;
                    let len = read_i32(&mut r)?;
                    if len >= 0 {
                        let mut b = vec![0u8; len as usize];
                        r.read_exact(&mut b)?;
                        let text: String = b.iter().map(|&c| c as char).collect();
                        on_note(Note::Clipboard(text));
                    } else {
                        let mut b = vec![0u8; (-len) as usize];
                        r.read_exact(&mut b)?;
                        self.cut_ext(&b, on_note);
                    }
                }
                t => bail!("VNC: неизвестное сообщение {t}"),
            }
        }
    }

    fn request(&self, incremental: bool) {
        let (w, h) = self.size();
        let mut m = [3u8, incremental as u8, 0, 0, 0, 0, 0, 0, 0, 0];
        m[6..8].copy_from_slice(&(w as u16).to_be_bytes());
        m[8..10].copy_from_slice(&(h as u16).to_be_bytes());
        self.send(&m);
    }

    fn resize(&self, w: u32, h: u32) {
        let mut g = self.sh.lock().unwrap();
        if g.w != w || g.h != h || g.fb.is_empty() {
            g.w = w;
            g.h = h;
            g.fb = vec![0; (w * h * 4) as usize];
            for px in g.fb.chunks_exact_mut(4) {
                px[3] = 255;
            }
            g.dirty = true;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn rect(&self, r: &mut impl Read, tmp: &mut Vec<u8>, x: u32, y: u32, w: u32, h: u32, enc: i32, on_note: &NoteFn) -> Result<()> {
        match enc {
            ENC_RAW => {
                let n = (w * h * 4) as usize;
                tmp.resize(n, 0);
                r.read_exact(&mut tmp[..n])?;
                let mut g = self.sh.lock().unwrap();
                let (fw, fh) = (g.w, g.h);
                if x + w > fw || y + h > fh {
                    return Ok(());
                }
                for row in 0..h {
                    let src = &tmp[(row * w * 4) as usize..((row + 1) * w * 4) as usize];
                    let off = (((y + row) * fw + x) * 4) as usize;
                    let dst = &mut g.fb[off..off + (w * 4) as usize];
                    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                        d[0] = s[0];
                        d[1] = s[1];
                        d[2] = s[2];
                        d[3] = 255;
                    }
                }
            }
            ENC_COPYRECT => {
                let sx = read_u16(r)? as u32;
                let sy = read_u16(r)? as u32;
                let mut g = self.sh.lock().unwrap();
                let fw = g.w;
                if sx + w > g.w || sy + h > g.h || x + w > g.w || y + h > g.h {
                    return Ok(());
                }
                let mut block = vec![0u8; (w * h * 4) as usize];
                for row in 0..h {
                    let off = (((sy + row) * fw + sx) * 4) as usize;
                    block[(row * w * 4) as usize..((row + 1) * w * 4) as usize].copy_from_slice(&g.fb[off..off + (w * 4) as usize]);
                }
                for row in 0..h {
                    let off = (((y + row) * fw + x) * 4) as usize;
                    g.fb[off..off + (w * 4) as usize].copy_from_slice(&block[(row * w * 4) as usize..((row + 1) * w * 4) as usize]);
                }
            }
            ENC_CURSOR => {
                let n = (w * h * 4) as usize;
                let mask_row = w.div_ceil(8) as usize;
                let mut px = vec![0u8; n];
                let mut mask = vec![0u8; mask_row * h as usize];
                r.read_exact(&mut px)?;
                r.read_exact(&mut mask)?;
                for yy in 0..h as usize {
                    for xx in 0..w as usize {
                        let on = mask[yy * mask_row + xx / 8] & (0x80 >> (xx % 8)) != 0;
                        px[(yy * w as usize + xx) * 4 + 3] = if on { 255 } else { 0 };
                    }
                }
                let mut g = self.sh.lock().unwrap();
                g.cursor = if w > 0 && h > 0 { Some(Cursor { w, h, hx: x as i32, hy: y as i32, rgba: px }) } else { None };
                g.dirty = true;
            }
            ENC_DESKTOP_SIZE => {
                self.resize(w, h);
                on_note(Note::Resized { width: w, height: h });
            }
            ENC_EXT_DESKTOP_SIZE => {
                let n = read_u8(r)? as usize;
                skip(r, 3 + n * 16)?;
                let first = {
                    let mut g = self.sh.lock().unwrap();
                    let first = !g.ext_desktop;
                    g.ext_desktop = true;
                    first
                };
                if w > 0 && h > 0 {
                    self.resize(w, h);
                    on_note(Note::Resized { width: w, height: h });
                }
                // Размер просили до того, как сервер сообщил о поддержке.
                if first {
                    if let Some((ww, wh)) = *self.want_size.lock().unwrap() {
                        if (ww, wh) != (w, h) {
                            self.send_desktop_size(ww, wh);
                        }
                    }
                }
            }
            ENC_EXT_KEY => self.sh.lock().unwrap().ext_keys = true,
            ENC_POINTER_TYPE => {}
            other => bail!("VNC: кодировка {other} не заказывалась"),
        }
        Ok(())
    }

    fn cut_ext(&self, b: &[u8], on_note: &NoteFn) {
        if b.len() < 4 {
            return;
        }
        let flags = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        if flags & CB_CAPS != 0 {
            self.sh.lock().unwrap().clipboard_ext = true;
            // Наши возможности: текст до 1 МБ.
            let mut caps = Vec::new();
            caps.extend_from_slice(&(1u32 << 20).to_be_bytes());
            self.send_cut_ext(CB_CAPS | CB_TEXT | CB_REQUEST | CB_PEEK | CB_NOTIFY | CB_PROVIDE, &caps);
            return;
        }
        if flags & CB_REQUEST != 0 && flags & CB_TEXT != 0 {
            self.provide_clipboard();
        }
        if flags & CB_NOTIFY != 0 && flags & CB_TEXT != 0 {
            // У гостя новый текст — запросить.
            self.send_cut_ext(CB_REQUEST | CB_TEXT, &[]);
        }
        if flags & CB_PROVIDE != 0 && flags & CB_TEXT != 0 {
            let mut d = flate2::read::ZlibDecoder::new(&b[4..]);
            let mut raw = Vec::new();
            if d.read_to_end(&mut raw).is_err() && raw.len() < 4 {
                return;
            }
            if raw.len() >= 4 {
                let n = (u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize).min(raw.len() - 4);
                let text = String::from_utf8_lossy(&raw[4..4 + n]).trim_end_matches('\0').replace("\r\n", "\n");
                self.sh.lock().unwrap().clip_out = Some(text.clone());
                on_note(Note::Clipboard(text));
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}

fn blend_cursor(buf: &mut [u8], w: u32, h: u32, c: &Cursor, ox: i32, oy: i32) {
    for cy in 0..c.h as i32 {
        let y = oy + cy;
        if y < 0 || y >= h as i32 {
            continue;
        }
        for cx in 0..c.w as i32 {
            let x = ox + cx;
            if x < 0 || x >= w as i32 {
                continue;
            }
            let s = ((cy * c.w as i32 + cx) * 4) as usize;
            if c.rgba[s + 3] == 0 {
                continue;
            }
            let d = ((y * w as i32 + x) * 4) as usize;
            buf[d..d + 3].copy_from_slice(&c.rgba[s..s + 3]);
        }
    }
}

fn read_u8(r: &mut impl Read) -> std::io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

fn read_u16(r: &mut impl Read) -> std::io::Result<u16> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_be_bytes(b))
}

fn read_u32(r: &mut impl Read) -> std::io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}

fn read_i32(r: &mut impl Read) -> std::io::Result<i32> {
    Ok(read_u32(r)? as i32)
}

fn skip(r: &mut impl Read, n: usize) -> std::io::Result<()> {
    std::io::copy(&mut r.take(n as u64), &mut std::io::sink()).map(|_| ())
}
