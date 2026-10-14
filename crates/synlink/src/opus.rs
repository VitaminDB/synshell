//! Opus для звука трансляции: libopus загружается `dlopen` (synlink от неё
//! не зависит — нет библиотеки, звук идёт прежним PCM). 48 кГц, стерео,
//! кадр 10 мс (как кусок записи), режим «аудио» на 128 кбит/с — в 12 раз
//! меньше сырого PCM, на медленном Wi-Fi звук больше не рвётся.

use std::ffi::{c_int, c_void, CString};
use std::sync::OnceLock;

const RATE: c_int = 48_000;
const CHANNELS: c_int = 2;
/// Отсчётов на канал в кадре 10 мс.
pub const FRAME_SAMPLES: usize = 480;
const APPLICATION_AUDIO: c_int = 2049;
const SET_BITRATE: c_int = 4002;
const SET_COMPLEXITY: c_int = 4010;
const BITRATE: c_int = 128_000;
/// Самый большой пакет Opus.
const MAX_PACKET: usize = 1275;

type EncCreate = unsafe extern "C" fn(c_int, c_int, c_int, *mut c_int) -> *mut c_void;
type Encode = unsafe extern "C" fn(*mut c_void, *const i16, c_int, *mut u8, i32) -> i32;
type EncCtl = unsafe extern "C" fn(*mut c_void, c_int, ...) -> c_int;
type DecCreate = unsafe extern "C" fn(c_int, c_int, *mut c_int) -> *mut c_void;
type Decode = unsafe extern "C" fn(*mut c_void, *const u8, i32, *mut i16, c_int, c_int) -> c_int;
type Destroy = unsafe extern "C" fn(*mut c_void);

struct Lib {
    enc_create: EncCreate,
    encode: Encode,
    enc_ctl: EncCtl,
    enc_destroy: Destroy,
    dec_create: DecCreate,
    decode: Decode,
    dec_destroy: Destroy,
}

fn lib() -> Option<&'static Lib> {
    static LIB: OnceLock<Option<Lib>> = OnceLock::new();
    LIB.get_or_init(|| unsafe {
        let h = ["libopus.so.0", "libopus.so"].iter().find_map(|n| {
            let c = CString::new(*n).ok()?;
            let h = libc::dlopen(c.as_ptr(), libc::RTLD_NOW);
            (!h.is_null()).then_some(h)
        })?;
        let sym = |n: &str| -> Option<*mut c_void> {
            let c = CString::new(n).ok()?;
            let p = libc::dlsym(h, c.as_ptr());
            (!p.is_null()).then_some(p)
        };
        Some(Lib {
            enc_create: std::mem::transmute::<*mut c_void, EncCreate>(sym("opus_encoder_create")?),
            encode: std::mem::transmute::<*mut c_void, Encode>(sym("opus_encode")?),
            enc_ctl: std::mem::transmute::<*mut c_void, EncCtl>(sym("opus_encoder_ctl")?),
            enc_destroy: std::mem::transmute::<*mut c_void, Destroy>(sym("opus_encoder_destroy")?),
            dec_create: std::mem::transmute::<*mut c_void, DecCreate>(sym("opus_decoder_create")?),
            decode: std::mem::transmute::<*mut c_void, Decode>(sym("opus_decode")?),
            dec_destroy: std::mem::transmute::<*mut c_void, Destroy>(sym("opus_decoder_destroy")?),
        })
    })
    .as_ref()
}

/// Есть ли libopus на этой машине.
pub fn available() -> bool {
    lib().is_some()
}

pub struct Encoder {
    st: *mut c_void,
    lib: &'static Lib,
}

// Кодер живёт в одном потоке за раз (перекачка звука).
unsafe impl Send for Encoder {}

impl Encoder {
    pub fn new() -> Option<Self> {
        let lib = lib()?;
        let mut err = 0;
        let st = unsafe { (lib.enc_create)(RATE, CHANNELS, APPLICATION_AUDIO, &mut err) };
        if st.is_null() || err != 0 {
            return None;
        }
        unsafe {
            (lib.enc_ctl)(st, SET_BITRATE, BITRATE);
            (lib.enc_ctl)(st, SET_COMPLEXITY, 8 as c_int);
        }
        Some(Self { st, lib })
    }

    /// Кадр 10 мс: s16le, перемежённые каналы (`FRAME_SAMPLES * 2` отсчётов).
    pub fn encode(&mut self, pcm: &[u8]) -> Option<Vec<u8>> {
        let samples: Vec<i16> = pcm.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        if samples.len() != FRAME_SAMPLES * CHANNELS as usize {
            return None;
        }
        let mut out = vec![0u8; MAX_PACKET];
        let n = unsafe { (self.lib.encode)(self.st, samples.as_ptr(), FRAME_SAMPLES as c_int, out.as_mut_ptr(), MAX_PACKET as i32) };
        (n > 0).then(|| {
            out.truncate(n as usize);
            out
        })
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe { (self.lib.enc_destroy)(self.st) }
    }
}

pub struct Decoder {
    st: *mut c_void,
    lib: &'static Lib,
}

unsafe impl Send for Decoder {}

impl Decoder {
    pub fn new() -> Option<Self> {
        let lib = lib()?;
        let mut err = 0;
        let st = unsafe { (lib.dec_create)(RATE, CHANNELS, &mut err) };
        if st.is_null() || err != 0 {
            return None;
        }
        Some(Self { st, lib })
    }

    /// Пакет → s16le перемежённые каналы.
    pub fn decode(&mut self, packet: &[u8]) -> Option<Vec<u8>> {
        // Самый длинный кадр Opus — 120 мс.
        let max = 5760;
        let mut pcm = vec![0i16; max * CHANNELS as usize];
        let n = unsafe { (self.lib.decode)(self.st, packet.as_ptr(), packet.len() as i32, pcm.as_mut_ptr(), max as c_int, 0) };
        if n <= 0 {
            return None;
        }
        pcm.truncate(n as usize * CHANNELS as usize);
        Some(pcm.iter().flat_map(|s| s.to_le_bytes()).collect())
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { (self.lib.dec_destroy)(self.st) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Синус через кодер и декодер: длина та же, сигнал узнаваем.
    #[test]
    fn round_trip_keeps_length_and_energy() {
        let (Some(mut e), Some(mut d)) = (Encoder::new(), Decoder::new()) else {
            eprintln!("нет libopus — пропуск");
            return;
        };
        let mut energy_in = 0f64;
        let mut energy_out = 0f64;
        for f in 0..20 {
            let pcm: Vec<u8> = (0..FRAME_SAMPLES)
                .flat_map(|i| {
                    let t = (f * FRAME_SAMPLES + i) as f64 / 48_000.0;
                    let s = ((t * 440.0 * std::f64::consts::TAU).sin() * 8000.0) as i16;
                    [s.to_le_bytes(), s.to_le_bytes()].concat()
                })
                .collect();
            let pkt = e.encode(&pcm).expect("кодирование");
            assert!(pkt.len() < pcm.len() / 4, "пакет {} байт", pkt.len());
            let out = d.decode(&pkt).expect("декодирование");
            assert_eq!(out.len(), pcm.len());
            if f > 4 {
                energy_in += pcm.chunks_exact(2).map(|b| (i16::from_le_bytes([b[0], b[1]]) as f64).powi(2)).sum::<f64>();
                energy_out += out.chunks_exact(2).map(|b| (i16::from_le_bytes([b[0], b[1]]) as f64).powi(2)).sum::<f64>();
            }
        }
        let ratio = energy_out / energy_in;
        assert!((0.7..1.3).contains(&ratio), "энергия {ratio}");
    }
}
