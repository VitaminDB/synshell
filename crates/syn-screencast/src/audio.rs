use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;
use pipewire as pw;
use pw::spa;

use crate::enc::RATE;

#[derive(Default)]
pub struct Ring {
    q: VecDeque<f32>,
    start: i64,
    anchored: bool,
    pub chunks: u64,
}

pub struct Clock(pub Mutex<Option<Instant>>);

impl Clock {
    fn index(&self, now: Instant) -> Option<i64> {
        let t0 = (*self.0.lock().unwrap())?;
        Some(if now >= t0 { ((now - t0).as_secs_f64() * RATE as f64) as i64 } else { -(((t0 - now).as_secs_f64() * RATE as f64) as i64) })
    }
}

impl Ring {
    fn push(&mut self, end_index: i64, samples: &[f32]) {
        let frames = (samples.len() / 2) as i64;
        let begin = end_index - frames;
        let next = self.start + (self.q.len() / 2) as i64;
        if !self.anchored || (next - begin).abs() > RATE / 10 {
            self.q.clear();
            self.start = begin;
            self.anchored = true;
        }
        self.q.extend(samples.iter().copied());
        self.chunks += 1;
        let max = (RATE as usize) * 2 * 4;
        if self.q.len() > max {
            let extra = (self.q.len() - max) & !1;
            self.q.drain(..extra);
            self.start += (extra / 2) as i64;
        }
    }

    pub fn mix_into(&mut self, from: i64, out: &mut [f32]) {
        let n = (out.len() / 2) as i64;
        let to = from + n;
        while self.start < from && !self.q.is_empty() {
            self.q.pop_front();
            self.q.pop_front();
            self.start += 1;
        }
        if self.q.is_empty() {
            if self.start < to {
                self.start = self.start.max(to);
            }
            return;
        }
        let first = self.start.max(from);
        let mut i = first;
        while i < to && !self.q.is_empty() {
            let l = self.q.pop_front().unwrap_or(0.0);
            let r = self.q.pop_front().unwrap_or(0.0);
            let k = ((i - from) * 2) as usize;
            out[k] += l;
            out[k + 1] += r;
            i += 1;
            self.start += 1;
        }
    }
}

pub struct Capture {
    pub system: Option<Arc<Mutex<Ring>>>,
    pub mic: Option<Arc<Mutex<Ring>>>,
    pub clock: Arc<Clock>,
    quit: Option<pw::channel::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn format_pod() -> Vec<u8> {
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_rate(RATE as u32);
    info.set_channels(2);
    let mut pos = [0u32; spa::param::audio::MAX_CHANNELS];
    pos[0] = libspa_sys_pos(true);
    pos[1] = libspa_sys_pos(false);
    info.set_position(pos);
    let obj = spa::pod::Object { type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(), id: spa::param::ParamType::EnumFormat.as_raw(), properties: info.into() };
    spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &spa::pod::Value::Object(obj)).unwrap().0.into_inner()
}

fn libspa_sys_pos(left: bool) -> u32 {
    if left {
        pw::spa::sys::SPA_AUDIO_CHANNEL_FL
    } else {
        pw::spa::sys::SPA_AUDIO_CHANNEL_FR
    }
}

impl Capture {
    pub fn start(system: bool, mic: bool) -> Result<Capture> {
        let clock = Arc::new(Clock(Mutex::new(None)));
        let sys_ring = system.then(|| Arc::new(Mutex::new(Ring::default())));
        let mic_ring = mic.then(|| Arc::new(Mutex::new(Ring::default())));
        let (quit_tx, quit_rx) = pw::channel::channel::<()>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let (c2, s2, m2) = (clock.clone(), sys_ring.clone(), mic_ring.clone());
        let thread = std::thread::Builder::new().name("screenrec-audio".into()).spawn(move || {
            let r = (|| -> Result<()> {
                pw::init();
                let mainloop = pw::main_loop::MainLoopRc::new(None)?;
                let context = pw::context::ContextRc::new(&mainloop, None)?;
                let core = context.connect_rc(None)?;
                let ml = mainloop.clone();
                let _quit = quit_rx.attach(mainloop.loop_(), move |_| ml.quit());
                let mut keep = Vec::new();
                for (ring, sink) in [(s2, true), (m2, false)] {
                    let Some(ring) = ring else { continue };
                    let mut props = pw::properties::properties! {
                        *pw::keys::MEDIA_TYPE => "Audio",
                        *pw::keys::MEDIA_CATEGORY => "Capture",
                        *pw::keys::MEDIA_ROLE => if sink { "Screen" } else { "Communication" },
                        *pw::keys::APP_NAME => "synshell",
                        *pw::keys::NODE_NAME => if sink { "synshell-screenrec-system" } else { "synshell-screenrec-mic" },
                        *pw::keys::NODE_DESCRIPTION => if sink { "Запись экрана: звук" } else { "Запись экрана: микрофон" },
                        *pw::keys::NODE_LATENCY => "1024/48000",
                    };
                    if sink {
                        props.insert("stream.capture.sink", "true");
                    }
                    if let Ok(t) = std::env::var(if sink { "SYN_SCREENREC_SYSTEM_TARGET" } else { "SYN_SCREENREC_MIC_TARGET" }) {
                        props.insert(*pw::keys::TARGET_OBJECT, t);
                    }
                    let stream = pw::stream::StreamRc::new(core.clone(), if sink { "screenrec-system" } else { "screenrec-mic" }, props)?;
                    let clock = c2.clone();
                    let listener = stream
                        .add_local_listener::<()>()
                        .process(move |st, _| {
                            let Some(mut b) = st.dequeue_buffer() else { return };
                            let now = Instant::now();
                            let datas = b.datas_mut();
                            if datas.is_empty() {
                                return;
                            }
                            let d = &mut datas[0];
                            let size = d.chunk().size() as usize;
                            let off = d.chunk().offset() as usize;
                            let Some(bytes) = d.data() else { return };
                            let end = (off + size).min(bytes.len());
                            let raw = &bytes[off.min(end)..end];
                            let Some(idx) = clock.index(now) else { return };
                            let samples: Vec<f32> = raw.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
                            if samples.len() < 2 {
                                return;
                            }
                            ring.lock().unwrap().push(idx, &samples[..samples.len() & !1]);
                        })
                        .register()?;
                    let pod = format_pod();
                    let mut params = [spa::pod::Pod::from_bytes(&pod).unwrap()];
                    stream.connect(spa::utils::Direction::Input, None, pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS, &mut params)?;
                    keep.push((stream, listener));
                }
                let _ = ready_tx.send(Ok(()));
                mainloop.run();
                drop(keep);
                Ok(())
            })();
            if let Err(e) = r {
                let _ = ready_tx.send(Err(format!("{e:#}")));
            }
        })?;
        match ready_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => anyhow::bail!("PipeWire: {e}"),
            Err(_) => anyhow::bail!("PipeWire: нет ответа"),
        }
        Ok(Capture { system: sys_ring, mic: mic_ring, clock, quit: Some(quit_tx), thread: Some(thread) })
    }

    pub fn set_start(&self, t0: Instant) {
        *self.clock.0.lock().unwrap() = Some(t0);
    }

    pub fn mix(&self, from: i64, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * 2];
        if let Some(r) = &self.system {
            r.lock().unwrap().mix_into(from, &mut out);
        }
        if let Some(r) = &self.mic {
            r.lock().unwrap().mix_into(from, &mut out);
        }
        if self.system.is_some() && self.mic.is_some() {
            for s in out.iter_mut() {
                *s = s.clamp(-1.0, 1.0);
            }
        }
        out
    }

    pub fn chunks(&self) -> u64 {
        [&self.system, &self.mic].iter().filter_map(|r| r.as_ref()).map(|r| r.lock().unwrap().chunks).sum()
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(q) = self.quit.take() {
            let _ = q.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
