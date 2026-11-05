//! Звук записи: микрофон (cpal → PipeWire) и внешние дорожки ([`Track`]: голос собеседника, озвучка)
//! смешиваются в один поток 48 кГц → AAC → MP4.
//!
//! Время звука — непрерывный счётчик отсчётов от первого кадра видео; пауза отбрасывает отсчёты
//! микрофона, а дорожки ждут продолжения. Нет микрофона — счётчик идёт по часам (тишина + дорожки).

use std::collections::VecDeque;
use std::ffi::c_int;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use ffmpeg_next::ffi;

use crate::mux::{averr, Mux};
use crate::recorder::Shared;

pub const RATE: u32 = 48000;
/// Хвост приглушения микрофона после куска дорожки (задержка динамика и эхо), отсчёты.
const DUCK_TAIL: u64 = RATE as u64 * 2 / 5;

/// Смеситель: что и когда звучит поверх микрофона.
pub struct Mixer {
    /// Куски дорожек: (первый отсчёт на шкале записи, моно 48 кГц, громкость, номер дорожки,
    /// приглушение микрофона на время куска: 1 — нет).
    parts: Mutex<Vec<(u64, Vec<f32>, f32, u64, f32)>>,
    next_track: AtomicU64,
    /// Отсчётов (на канал) уже отдано кодеру.
    pos: AtomicU64,
    /// Отсчётов микрофона в очереди (ещё не отданы).
    queued: AtomicU64,
    /// Запись началась (первый кадр видео).
    started: AtomicBool,
}

impl Mixer {
    fn new() -> Arc<Mixer> {
        Arc::new(Mixer { parts: Mutex::new(Vec::new()), next_track: AtomicU64::new(1), pos: AtomicU64::new(0), queued: AtomicU64::new(0), started: AtomicBool::new(false) })
    }

    /// Записанный звук, мкс.
    pub fn position_us(&self) -> u64 {
        self.pos.load(Ordering::SeqCst) * 1_000_000 / RATE as u64
    }

    /// Текущее место на шкале записи, отсчёты.
    fn now(&self) -> u64 {
        self.pos.load(Ordering::SeqCst) + self.queued.load(Ordering::SeqCst)
    }

    /// Смешать дорожки в кадр `out` (`ch` каналов вперемешку), начиная с отсчёта `at`.
    fn mix(&self, at: u64, out: &mut [f32], ch: usize) {
        let n = (out.len() / ch) as u64;
        let mut parts = self.parts.lock().unwrap();
        // Сначала приглушить микрофон, пока звучит дорожка с приглушением (и
        // хвост DUCK_TAIL: динамик слышен микрофону с задержкой) — иначе голос
        // собеседника попадает в запись дважды: чисто и эхом из динамика.
        for (start, data, _, _, duck) in parts.iter() {
            if *duck >= 1.0 {
                continue;
            }
            let end = start + data.len() as u64 + DUCK_TAIL;
            let (from, to) = (at.max(*start), (at + n).min(end));
            for t in from..to.max(from) {
                let i = (t - at) as usize * ch;
                for c in 0..ch {
                    out[i + c] *= duck;
                }
            }
        }
        for (start, data, gain, _, _) in parts.iter() {
            let end = start + data.len() as u64;
            if end <= at || *start >= at + n {
                continue;
            }
            let from = at.max(*start);
            let to = (at + n).min(end);
            for t in from..to {
                let v = data[(t - start) as usize] * gain;
                let i = (t - at) as usize * ch;
                for c in 0..ch {
                    out[i + c] = (out[i + c] + v).clamp(-1.0, 1.0);
                }
            }
        }
        parts.retain(|(start, data, _, _, duck)| start + data.len() as u64 + if *duck < 1.0 { DUCK_TAIL } else { 0 } > at + n);
    }
}

/// Внешняя дорожка записи (голос собеседника и т. п.).
#[derive(Clone)]
pub struct Track {
    mixer: Arc<Mixer>,
    /// Конец последнего куска этой дорожки, отсчёты.
    end: Arc<AtomicU64>,
    gain: f32,
    id: u64,
    /// Громкость микрофона, пока звучит дорожка (1 — не трогать).
    duck: f32,
}

impl Track {
    pub(crate) fn new(mixer: Arc<Mixer>, gain: f32) -> Track {
        let id = mixer.next_track.fetch_add(1, Ordering::Relaxed);
        Track { mixer, end: Arc::new(AtomicU64::new(0)), gain, id, duck: 1.0 }
    }

    /// Приглушать микрофон, пока звучит эта дорожка (0..1; 0,05 — почти
    /// тишина): голос из динамика не попадёт в запись второй раз эхом.
    pub fn duck_mic(mut self, level: f32) -> Track {
        self.duck = level.clamp(0.0, 1.0);
        self
    }

    /// Добавить звук (моно, любая частота): сразу после прошлого куска этой дорожки, если он ещё
    /// звучит, иначе «сейчас». Возвращает время начала куска в записи — для транскрипта.
    pub fn play(&self, pcm: &[f32], sample_rate: u32) -> Duration {
        let data = resample(pcm, sample_rate, RATE);
        let start = self.mixer.now().max(self.end.load(Ordering::SeqCst));
        self.end.store(start + data.len() as u64, Ordering::SeqCst);
        self.mixer.parts.lock().unwrap().push((start, data, self.gain, self.id, self.duck));
        Duration::from_micros(start * 1_000_000 / RATE as u64)
    }

    /// Звучит ли ещё дорожка (её куски не доиграли).
    pub fn playing(&self) -> bool {
        self.end.load(Ordering::SeqCst) > self.mixer.now()
    }

    /// Оборвать дорожку: что не доиграло — не попадёт в запись.
    pub fn stop(&self) {
        let now = self.mixer.now();
        let mut parts = self.mixer.parts.lock().unwrap();
        for (start, data, _, id, _) in parts.iter_mut() {
            if *id == self.id && *start + data.len() as u64 > now {
                data.truncate(now.saturating_sub(*start) as usize);
            }
        }
        self.end.store(now, Ordering::SeqCst);
    }
}

/// Линейная передискретизация (речь, не музыка).
pub fn resample(pcm: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || pcm.is_empty() || from == 0 {
        return pcm.to_vec();
    }
    let n = (pcm.len() as u64 * to as u64 / from as u64) as usize;
    let k = from as f64 / to as f64;
    (0..n)
        .map(|i| {
            let x = i as f64 * k;
            let j = x as usize;
            let f = (x - j as f64) as f32;
            let a = pcm[j.min(pcm.len() - 1)];
            let b = pcm[(j + 1).min(pcm.len() - 1)];
            a + (b - a) * f
        })
        .collect()
}

struct SendPtr<T>(*mut T);
// SAFETY: кодер используется только потоком звука
unsafe impl<T> Send for SendPtr<T> {}

pub(crate) struct Audio {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
    pub mixer: Arc<Mixer>,
}

impl Audio {
    /// Поток звука в `mux`; `mic` — писать микрофон (нет — только дорожки).
    pub fn start(mux: Arc<Mutex<Mux>>, sh: Arc<Shared>, t0: Arc<AtomicU64>, mic: bool) -> Result<Audio> {
        let channels: u16 = if mic { mic_channels().unwrap_or(1) } else { 1 };
        let enc = open_aac(RATE, channels)?;
        if let Err(e) = mux.lock().unwrap().add_audio(enc) {
            // SAFETY: кодер из open_aac
            unsafe { ffi::avcodec_free_context(&mut (enc as *mut _)) };
            return Err(e);
        }
        let mixer = Mixer::new();
        let stop = Arc::new(AtomicBool::new(false));
        let (st, mx) = (stop.clone(), mixer.clone());
        let enc = SendPtr(enc);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
        let thread = std::thread::Builder::new().name("video-audio".into()).spawn(move || {
            let enc = enc;
            let buf: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
            let stream = if mic {
                match open_mic(buf.clone(), channels) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        tracing::warn!("микрофон для записи: {e:#} — пишется без него");
                        None
                    }
                }
            } else {
                None
            };
            let _ = ready_tx.send(Ok(()));
            audio_loop(enc.0, &buf, stream.is_some(), &mux, &sh, &t0, &st, channels as usize, &mx);
            drop(stream);
            // SAFETY: кодер из open_aac
            unsafe { ffi::avcodec_free_context(&mut (enc.0 as *mut _)) };
        })?;
        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(r) => r.map(|_| Audio { stop, thread, mixer }),
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                Err(anyhow!("микрофон не отвечает"))
            }
        }
    }

    pub fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.thread.join();
    }
}

/// Микрофон: через ALSA-плагины PipeWire/Pulse (системный выбор источника), иначе умолчание cpal —
/// голое ALSA «default» на компьютере с PipeWire открывает dsnoop и пишет тишину.
fn input_device() -> Option<cpal::Device> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    if let Ok(devs) = host.input_devices() {
        let devs: Vec<cpal::Device> = devs.collect();
        for want in ["pipewire", "pulse"] {
            if let Some(d) = devs.iter().find(|d| d.name().is_ok_and(|n| n == want)) {
                return Some(d.clone());
            }
        }
    }
    host.default_input_device()
}

fn mic_channels() -> Option<u16> {
    use cpal::traits::DeviceTrait;
    let dev = input_device()?;
    Some(dev.default_input_config().ok()?.channels().clamp(1, 2))
}

/// Узел PipeWire микрофона записи (`None` — системный по умолчанию).
static MIC_NODE: Mutex<Option<String>> = Mutex::new(None);

/// Писать звук записи с этого узла PipeWire (например, микрофон телефона без
/// системного шумоподавления `palaudio.mic.bottom`): тот же, что у других
/// записей приложения, — на телефоне одновременно пишет только один источник.
pub fn set_mic_node(node: Option<String>) {
    *MIC_NODE.lock().unwrap() = node.filter(|n| !n.is_empty());
}

fn open_mic(buf: Arc<Mutex<VecDeque<f32>>>, channels: u16) -> Result<cpal::Stream> {
    // ALSA-плагин pipewire берёт источник из PIPEWIRE_NODE при открытии
    // (переменная — только на время открытия, как в syngui).
    let node = MIC_NODE.lock().unwrap().clone();
    if let Some(n) = &node {
        // SAFETY: переменная нужна лишь на время открытия устройства ниже
        unsafe { std::env::set_var("PIPEWIRE_NODE", n) };
    }
    let r = open_mic_dev(buf, channels);
    if node.is_some() {
        // SAFETY: см. выше
        unsafe { std::env::remove_var("PIPEWIRE_NODE") };
    }
    r
}

fn open_mic_dev(buf: Arc<Mutex<VecDeque<f32>>>, channels: u16) -> Result<cpal::Stream> {
    use cpal::traits::{DeviceTrait, StreamTrait};
    let dev = input_device().context("нет микрофона")?;
    let def = dev.default_input_config().context("настройки микрофона")?;
    let cfg = cpal::StreamConfig { channels, sample_rate: cpal::SampleRate(RATE), buffer_size: cpal::BufferSize::Default };
    let err_fn = |e| tracing::warn!("микрофон: {e}");
    let stream = match def.sample_format() {
        cpal::SampleFormat::I16 => dev.build_input_stream(&cfg, move |d: &[i16], _: &_| buf.lock().unwrap().extend(d.iter().map(|&v| v as f32 / 32768.0)), err_fn, None),
        _ => dev.build_input_stream(&cfg, move |d: &[f32], _: &_| buf.lock().unwrap().extend(d.iter().copied()), err_fn, None),
    }
    .map_err(|e| anyhow!("микрофон: {e}"))?;
    stream.play().map_err(|e| anyhow!("микрофон: {e}"))?;
    Ok(stream)
}

fn open_aac(rate: u32, channels: u16) -> Result<*mut ffi::AVCodecContext> {
    // SAFETY: FFI FFmpeg; при ошибке контекст освобождается
    unsafe {
        let codec = ffi::avcodec_find_encoder(ffi::AVCodecID::AV_CODEC_ID_AAC);
        if codec.is_null() {
            bail!("нет кодера AAC");
        }
        let c = ffi::avcodec_alloc_context3(codec);
        (*c).sample_rate = rate as c_int;
        (*c).sample_fmt = ffi::AVSampleFormat::AV_SAMPLE_FMT_FLTP;
        ffi::av_channel_layout_default(&mut (*c).ch_layout, channels as c_int);
        (*c).bit_rate = if channels > 1 { 192_000 } else { 128_000 };
        (*c).time_base = ffi::AVRational { num: 1, den: rate as c_int };
        (*c).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as c_int;
        let r = ffi::avcodec_open2(c, codec, std::ptr::null_mut());
        if r < 0 {
            let mut c = c;
            ffi::avcodec_free_context(&mut c);
            bail!("кодер AAC: {}", averr(r));
        }
        Ok(c)
    }
}

#[allow(clippy::too_many_arguments)]
fn audio_loop(
    enc: *mut ffi::AVCodecContext,
    buf: &Mutex<VecDeque<f32>>,
    mic: bool,
    mux: &Mutex<Mux>,
    sh: &Shared,
    t0: &AtomicU64,
    stop: &AtomicBool,
    ch: usize,
    mixer: &Mixer,
) {
    // SAFETY: FFI FFmpeg; кадр и пакет освобождаются в конце
    unsafe {
        let fs = ((*enc).frame_size.max(1024)) as usize;
        let tb = (*enc).time_base;
        let frame = ffi::av_frame_alloc();
        (*frame).nb_samples = fs as c_int;
        (*frame).format = ffi::AVSampleFormat::AV_SAMPLE_FMT_FLTP as c_int;
        ffi::av_channel_layout_copy(&mut (*frame).ch_layout, &(*enc).ch_layout);
        (*frame).sample_rate = (*enc).sample_rate;
        ffi::av_frame_get_buffer(frame, 0);
        let pkt = ffi::av_packet_alloc();
        let mut pts: i64 = 0;
        let drain = |pkt: *mut ffi::AVPacket| {
            while ffi::avcodec_receive_packet(enc, pkt) >= 0 {
                let d = std::slice::from_raw_parts((*pkt).data, (*pkt).size as usize);
                let mut m = mux.lock().unwrap();
                if m.header {
                    let ast = m.ast;
                    if let Err(e) = m.write(ast, d, (*pkt).pts, tb, true, (*pkt).duration) {
                        tracing::warn!("звук: {e:#}");
                    }
                } else {
                    m.pending.push((d.to_vec(), (*pkt).pts, (*pkt).duration));
                }
                drop(m);
                ffi::av_packet_unref(pkt);
            }
        };
        // Без микрофона — тишина по часам (паузы не считаются).
        let mut clock: Option<(Instant, u64)> = None;
        loop {
            let stopping = stop.load(Ordering::SeqCst);
            let started = t0.load(Ordering::SeqCst) != 0;
            let paused = sh.paused.load(Ordering::Relaxed);
            if started {
                mixer.started.store(true, Ordering::SeqCst);
            }
            if !mic {
                if started && !paused {
                    let (since, base) = *clock.get_or_insert((Instant::now(), pts as u64));
                    let want = base + (since.elapsed().as_secs_f64() * RATE as f64) as u64;
                    let have = pts as u64 + buf.lock().unwrap().len() as u64 / ch as u64;
                    if want > have {
                        buf.lock().unwrap().extend(std::iter::repeat_n(0.0f32, (want - have) as usize * ch));
                    }
                } else {
                    clock = None;
                }
            }
            // до первого кадра видео и на паузе — отсчёты микрофона выбрасываются
            let take: Option<Vec<f32>> = {
                let mut b = buf.lock().unwrap();
                let r = if !started || paused {
                    b.clear();
                    None
                } else if b.len() >= fs * ch {
                    Some(b.drain(..fs * ch).collect())
                } else {
                    None
                };
                mixer.queued.store((b.len() / ch) as u64, Ordering::SeqCst);
                r
            };
            match take {
                Some(mut s) => {
                    if sh.mic_muted.load(Ordering::Relaxed) {
                        s.iter_mut().for_each(|v| *v = 0.0);
                    }
                    mixer.mix(pts as u64, &mut s, ch);
                    ffi::av_frame_make_writable(frame);
                    for c in 0..ch {
                        let plane = std::slice::from_raw_parts_mut((*frame).data[c] as *mut f32, fs);
                        for (i, v) in plane.iter_mut().enumerate() {
                            *v = s[i * ch + c];
                        }
                    }
                    (*frame).pts = pts;
                    pts += fs as i64;
                    mixer.pos.store(pts as u64, Ordering::SeqCst);
                    if ffi::avcodec_send_frame(enc, frame) >= 0 {
                        drain(pkt);
                    }
                }
                None if stopping => break,
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        ffi::avcodec_send_frame(enc, std::ptr::null());
        drain(pkt);
        let mut f = frame;
        ffi::av_frame_free(&mut f);
        let mut p = pkt;
        ffi::av_packet_free(&mut p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_follows_and_mixes() {
        let m = Mixer::new();
        let t = Track::new(m.clone(), 1.0);
        assert_eq!(t.play(&[0.5; 100], RATE), Duration::ZERO);
        // второй кусок — сразу после первого
        assert_eq!(t.play(&[0.25; 100], RATE), Duration::from_micros(100 * 1_000_000 / RATE as u64));
        let mut out = vec![0.1f32; 300];
        m.mix(0, &mut out, 1);
        assert!((out[0] - 0.6).abs() < 1e-6);
        assert!((out[150] - 0.35).abs() < 1e-6);
        assert!((out[250] - 0.1).abs() < 1e-6);
        assert!(m.parts.lock().unwrap().is_empty());
    }

    #[test]
    fn resample_len() {
        assert_eq!(resample(&[0.0; 24000], 24000, 48000).len(), 48000);
        assert_eq!(resample(&[0.0; 16000], 16000, 48000).len(), 48000);
    }
}

#[cfg(test)]
mod duck_tests {
    use super::*;

    #[test]
    fn mic_ducked_while_track_plays_and_tail() {
        let m = Mixer::new();
        let t = Track::new(m.clone(), 1.0).duck_mic(0.1);
        // Кусок 0,1 с с начала шкалы.
        t.play(&vec![0.5; (RATE / 10) as usize], RATE);
        let n = (RATE as usize) / 10 + DUCK_TAIL as usize + 100;
        let mut out = vec![1.0f32; n]; // «микрофон»: громкость 1, моно
        m.mix(0, &mut out, 1);
        let len = (RATE / 10) as usize;
        assert!((out[10] - (0.1 + 0.5)).abs() < 1e-5, "{}", out[10]);
        assert!((out[len + 10] - 0.1).abs() < 1e-5, "хвост: {}", out[len + 10]);
        assert!((out[n - 1] - 1.0).abs() < 1e-5, "после хвоста: {}", out[n - 1]);
        // Дорожка без приглушения микрофон не трогает.
        let m2 = Mixer::new();
        Track::new(m2.clone(), 1.0).play(&vec![0.5; 100], RATE);
        let mut out2 = vec![0.2f32; 200];
        m2.mix(0, &mut out2, 1);
        assert!((out2[150] - 0.2).abs() < 1e-5);
    }
}
