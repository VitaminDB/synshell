//! Общая память DLSS 5: заголовок `ShmHeader` один в один с `crates/syn-wine/dlss5/common/shm_protocol.h`
//! (v20). Слой, помощник и это окно видят одни и те же атомики в файле `/tmp/dlssnr-$UID/shm.bin`;
//! раскладка сверяется тестом с C++ (`tests` внизу).

use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::OnceLock;

pub const MAGIC: u32 = 0x3252_4E47;
pub const VERSION: u32 = 20;
pub const MAX_W: usize = 7680;
pub const MAX_H: usize = 4320;
pub const MAX_FRAME: usize = MAX_W * MAX_H * 8;
pub const HEADER_BYTES: usize = 65536;
pub const MAX_PASSES: usize = 30;
pub const REASON_BYTES: usize = 192;
pub const NAME_BYTES: usize = 128;

pub fn total_bytes() -> usize {
    HEADER_BYTES + MAX_FRAME * 2
}

// Биты переопределений прохода (PassOverrideBit).
pub const OV_INTENSITY: u32 = 1 << 0;
pub const OV_LOCAL_STRUCTURE: u32 = 1 << 1;
pub const OV_LOCAL_TONE: u32 = 1 << 2;
pub const OV_SKIN_STRUCTURE: u32 = 1 << 3;
pub const OV_STYLE: u32 = 1 << 4;
pub const OV_PRESET: u32 = 1 << 5;
pub const OV_AUTO_MASK: u32 = 1 << 6;
pub const OV_SHARPNESS: u32 = 1 << 7;

// HelperState.
pub const HELPER_STARTING: u32 = 0;
pub const HELPER_NO_VULKAN: u32 = 1;
pub const HELPER_NO_BINARIES: u32 = 2;
pub const HELPER_MODEL_FAILED: u32 = 3;
pub const HELPER_RUNNING: u32 = 4;
pub const HELPER_STOPPED: u32 = 5;

#[repr(C)]
pub struct PassControl {
    pub override_mask: AtomicU32,
    pub intensity_bits: AtomicU32,
    pub local_tone_bits: AtomicU32,
    pub local_structure_bits: AtomicU32,
    pub skin_structure_bits: AtomicU32,
    pub sharpness_bits: AtomicU32,
    pub style: AtomicU32,
    pub preset: AtomicU32,
    pub auto_mask: AtomicU32,
}

#[repr(C)]
pub struct ShmHeader {
    pub magic: AtomicU32,
    pub version: AtomicU32,
    pub seq_req: AtomicU32,
    pub seq_resp: AtomicU32,
    pub width: AtomicU32,
    pub height: AtomicU32,
    pub format: AtomicU32,
    pub quit: AtomicU32,
    pub heartbeat: AtomicU32,
    pub control_seq: AtomicU32,
    pub tuning_seq: AtomicU32,
    // модель
    pub enabled: AtomicU32,
    pub passes: AtomicU32,
    pub unlock_passes: AtomicU32,
    pub preset: AtomicU32,
    pub style: AtomicU32,
    pub auto_mask: AtomicU32,
    pub intensity_bits: AtomicU32,
    pub local_tone_bits: AtomicU32,
    pub local_structure_bits: AtomicU32,
    pub skin_structure_bits: AtomicU32,
    pub sharpness_bits: AtomicU32,
    // композиция
    pub transfer_strength_bits: AtomicU32,
    pub colour_strength_bits: AtomicU32,
    pub max_ratio_bits: AtomicU32,
    pub transfer: AtomicU32,
    pub debug_view: AtomicU32,
    pub debug_scale_bits: AtomicU32,
    pub white_point_bits: AtomicU32,
    pub white_point_scale_bits: AtomicU32,
    pub white_point_source: AtomicU32,
    pub white_point_trim_bits: AtomicU32,
    pub working_scale_bits: AtomicU32,
    pub compare_mode: AtomicU32,
    pub compare_split_bits: AtomicU32,
    pub compare_zoom_bits: AtomicU32,
    pub compare_swap: AtomicU32,
    pub colour_mode: AtomicU32,
    pub capture_request: AtomicU32,
    pub toggle_key: AtomicU32,
    pub reversible_mode: AtomicU32,
    pub apply_model: AtomicU32,
    pub hold_frame: AtomicU32,
    pub scaling_downscaler: AtomicU32,
    // состояние помощника
    pub helper_state: AtomicU32,
    pub model_up: AtomicU32,
    pub helper_frames_lo: AtomicU32,
    pub helper_frames_hi: AtomicU32,
    pub helper_eval_ms_bits: AtomicU32,
    pub helper_upload_ms_bits: AtomicU32,
    pub helper_readback_ms_bits: AtomicU32,
    pub helper_vram_mb: AtomicU32,
    pub helper_features: AtomicU32,
    pub helper_pass_ceiling: AtomicU32,
    // состояние слоя
    pub layer_pid: AtomicU32,
    pub layer_frames_lo: AtomicU32,
    pub layer_frames_hi: AtomicU32,
    pub layer_width: AtomicU32,
    pub layer_height: AtomicU32,
    pub layer_format: AtomicU32,
    pub layer_composition_up: AtomicU32,
    pub layer_ms_bits: AtomicU32,
    pub layer_measured_white_bits: AtomicU32,
    pub layer_heartbeat: AtomicU32,
    pub helper_reason_seq: AtomicU32,
    pub helper_reason: [AtomicU8; REASON_BYTES],
    pub layer_reason_seq: AtomicU32,
    pub layer_reason: [AtomicU8; REASON_BYTES],
    pub game_name_seq: AtomicU32,
    pub game_name: [AtomicU8; NAME_BYTES],
    pub pass: [PassControl; MAX_PASSES],
    pub mvec_enabled: AtomicU32,
    pub mvec_scale_mode: AtomicU32,
    pub mvec_quality: AtomicU32,
    pub seq_ok: AtomicU32,
    pub composition_bypass: AtomicU32,
    pub rebuild_settle_ms: AtomicU32,
    pub answered_w: AtomicU32,
    pub answered_h: AtomicU32,
    pub proxy_export_seq: AtomicU32,
    pub proxy_pid: AtomicU32,
    pub proxy_fd: AtomicU32,
    pub proxy_gen: AtomicU32,
    pub answer_export_seq: AtomicU32,
    pub answer_pid: AtomicU32,
    pub answer_fd: AtomicU32,
    pub answer_gen: AtomicU32,
    pub layer_proxy_seq: AtomicU32,
    pub layer_answer_seq: AtomicU32,
    pub hdr_mode: AtomicU32,
    pub hdr_detected: AtomicU32,
    pub hdr_active: AtomicU32,
    pub proxy_format: AtomicU32,
    pub hdr_encode: AtomicU32,
    pub colour_trust_percent: AtomicU32,
    pub ratio_smooth_percent: AtomicU32,
    pub sdr16_multipass: AtomicU32,
    pub mvec_pixel_size: AtomicU32,
}

// Те же закрепления, что static_assert в shm_protocol.h.
const _: () = assert!(std::mem::size_of::<ShmHeader>() == 1968);
const _: () = assert!(std::mem::offset_of!(ShmHeader, enabled) == 44);
const _: () = assert!(std::mem::offset_of!(ShmHeader, transfer_strength_bits) == 88);
const _: () = assert!(std::mem::offset_of!(ShmHeader, helper_state) == 176);
const _: () = assert!(std::mem::offset_of!(ShmHeader, pass) == 780);
const _: () = assert!(std::mem::offset_of!(ShmHeader, mvec_enabled) == 1860);

pub fn f2b(f: f32) -> u32 {
    f.to_bits()
}

pub fn b2f(u: u32) -> f32 {
    f32::from_bits(u)
}

/// Поле заголовка: и для таблиц настроек, и для привязки элементов окна.
pub type Field = fn(&ShmHeader) -> &AtomicU32;
pub type PassField = fn(&PassControl) -> &AtomicU32;

const SC: Ordering = Ordering::SeqCst;

impl ShmHeader {
    pub fn get(&self, f: Field) -> u32 {
        f(self).load(SC)
    }

    /// Запись настройки: `controlSeq` всегда, `tuningSeq` — если модель читает поле при создании
    /// своей функции (иначе помощник не пересоберёт её).
    pub fn set(&self, f: Field, raw: u32, latch: bool) {
        f(self).store(raw, SC);
        self.control_seq.fetch_add(1, SC);
        if latch {
            self.tuning_seq.fetch_add(1, SC);
        }
    }

    pub fn valid(&self) -> bool {
        self.magic.load(SC) == MAGIC && self.version.load(SC) == VERSION
    }

    pub fn load64(lo: &AtomicU32, hi: &AtomicU32) -> u64 {
        ((hi.load(SC) as u64) << 32) | lo.load(SC) as u64
    }

    /// Строка под своим счётчиком: счётчик не изменился за чтение — строка целая.
    pub fn load_string(seq: &AtomicU32, src: &[AtomicU8]) -> String {
        for _ in 0..4 {
            let before = seq.load(SC);
            let bytes: Vec<u8> = src.iter().map(|b| b.load(SC)).take_while(|&b| b != 0).collect();
            if seq.load(SC) == before {
                return String::from_utf8_lossy(&bytes).into_owned();
            }
        }
        String::new()
    }

    pub fn helper_reason(&self) -> String {
        Self::load_string(&self.helper_reason_seq, &self.helper_reason)
    }

    pub fn layer_reason(&self) -> String {
        Self::load_string(&self.layer_reason_seq, &self.layer_reason)
    }

    pub fn game_name(&self) -> String {
        Self::load_string(&self.game_name_seq, &self.game_name)
    }

    /// ShmInitDefaults: весь заголовок — нули, затем значения по умолчанию.
    pub fn init_defaults(&self) {
        // SAFETY: заголовок — атомики u32/u8 без инвариантов, нули допустимы для всех полей.
        unsafe { std::ptr::write_bytes(self as *const Self as *mut u8, 0, std::mem::size_of::<Self>()) };
        let s = |a: &AtomicU32, v: u32| a.store(v, SC);
        s(&self.magic, MAGIC);
        s(&self.version, VERSION);
        s(&self.helper_state, HELPER_STOPPED);
        s(&self.format, 1);
        s(&self.passes, 1);
        s(&self.enabled, 1);
        s(&self.auto_mask, 1);
        s(&self.intensity_bits, f2b(1.0));
        s(&self.local_tone_bits, f2b(1.0));
        s(&self.local_structure_bits, f2b(1.0));
        s(&self.skin_structure_bits, f2b(-1.0));
        s(&self.sharpness_bits, f2b(0.0));
        s(&self.transfer_strength_bits, f2b(1.0));
        s(&self.colour_strength_bits, f2b(1.0));
        s(&self.max_ratio_bits, f2b(2.0));
        s(&self.transfer, 1);
        s(&self.debug_scale_bits, f2b(1.0));
        s(&self.white_point_bits, f2b(1.0));
        s(&self.white_point_scale_bits, f2b(1.0));
        s(&self.white_point_trim_bits, f2b(1.0));
        s(&self.white_point_source, 0);
        s(&self.working_scale_bits, f2b(1.0));
        s(&self.compare_split_bits, f2b(0.5));
        s(&self.compare_zoom_bits, f2b(1.0));
        s(&self.colour_mode, 0);
        s(&self.reversible_mode, 0);
        s(&self.apply_model, 1);
        s(&self.hold_frame, 0);
        s(&self.scaling_downscaler, 4);
        s(&self.hdr_mode, 0);
        s(&self.hdr_detected, 0);
        s(&self.hdr_active, 0);
        s(&self.proxy_format, 1);
        s(&self.mvec_enabled, 1);
        s(&self.mvec_scale_mode, 1);
        s(&self.mvec_quality, 1);
        s(&self.mvec_pixel_size, 2);
        s(&self.seq_ok, 0);
        s(&self.composition_bypass, 1);
        s(&self.rebuild_settle_ms, 100);
        s(&self.colour_trust_percent, 200);
        s(&self.ratio_smooth_percent, 100);
        s(&self.sdr16_multipass, 0);
        for p in &self.pass {
            s(&p.override_mask, 0);
            s(&p.intensity_bits, f2b(1.0));
            s(&p.local_tone_bits, f2b(1.0));
            s(&p.local_structure_bits, f2b(1.0));
            s(&p.skin_structure_bits, f2b(-1.0));
            s(&p.sharpness_bits, f2b(0.0));
            s(&p.style, 0);
            s(&p.preset, 0);
            s(&p.auto_mask, 1);
        }
    }

    /// ShmResetSettings: настройки — по умолчанию, состояние транспорта, помощника и слоя — как было.
    pub fn reset_settings(&self) {
        let keep: Vec<(&AtomicU32, u32)> = self.status_fields().into_iter().map(|a| (a, a.load(SC))).collect();
        let control = self.control_seq.load(SC);
        let tuning = self.tuning_seq.load(SC);
        let strings: Vec<Vec<u8>> = [&self.helper_reason[..], &self.layer_reason[..], &self.game_name[..]]
            .iter()
            .map(|s| s.iter().map(|b| b.load(SC)).collect())
            .collect();
        self.init_defaults();
        for (a, v) in keep {
            a.store(v, SC);
        }
        for (dst, src) in [&self.helper_reason[..], &self.layer_reason[..], &self.game_name[..]].iter().zip(strings) {
            for (d, b) in dst.iter().zip(src) {
                d.store(b, SC);
            }
        }
        self.control_seq.store(control.wrapping_add(1), SC);
        self.tuning_seq.store(tuning.wrapping_add(1), SC);
    }

    /// Поля, которые пишут о себе слой и помощник (ShmResetSettings их сохраняет).
    fn status_fields(&self) -> Vec<&AtomicU32> {
        vec![
            &self.seq_req, &self.seq_resp, &self.width, &self.height, &self.quit, &self.heartbeat, &self.seq_ok,
            &self.helper_state, &self.model_up, &self.helper_frames_lo, &self.helper_frames_hi,
            &self.helper_eval_ms_bits, &self.helper_upload_ms_bits, &self.helper_readback_ms_bits,
            &self.helper_vram_mb, &self.helper_features, &self.helper_pass_ceiling, &self.helper_reason_seq,
            &self.layer_pid, &self.layer_frames_lo, &self.layer_frames_hi, &self.layer_width, &self.layer_height,
            &self.layer_format, &self.layer_composition_up, &self.layer_ms_bits, &self.layer_measured_white_bits,
            &self.layer_heartbeat, &self.layer_reason_seq, &self.game_name_seq, &self.answered_w, &self.answered_h,
            &self.proxy_export_seq, &self.proxy_pid, &self.proxy_fd, &self.proxy_gen, &self.answer_export_seq,
            &self.answer_pid, &self.answer_fd, &self.answer_gen, &self.layer_proxy_seq, &self.layer_answer_seq,
            &self.hdr_detected, &self.hdr_active, &self.proxy_format, &self.hdr_encode,
        ]
    }
}

static MAP: OnceLock<Option<&'static ShmHeader>> = OnceLock::new();

/// Заголовок общей памяти: файл создаётся и дорастает до полного размера (разреженный), отображается
/// только заголовок. Чужой версии или пустой файл — заголовок по умолчанию, как у Qt-окна.
pub fn header() -> Option<&'static ShmHeader> {
    *MAP.get_or_init(|| match map(&crate::paths::shm_path()) {
        Ok(h) => Some(h),
        Err(e) => {
            tracing::warn!("общая память DLSS: {e}");
            None
        }
    })
}

fn map(path: &std::path::Path) -> std::io::Result<&'static ShmHeader> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::fs::OpenOptions::new().read(true).write(true).create(true).mode(0o666).open(path)?;
    let total = total_bytes() as u64;
    if file.metadata()?.len() < total {
        file.set_len(total)?;
    }
    use std::os::fd::AsRawFd;
    // SAFETY: отображение заголовка файла; живёт до конца процесса (никогда не снимается).
    let p = unsafe {
        libc::mmap(std::ptr::null_mut(), HEADER_BYTES, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, file.as_raw_fd(), 0)
    };
    if p == libc::MAP_FAILED {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: страница выровнена, размер >= size_of::<ShmHeader>(), все поля — атомики.
    let h: &'static ShmHeader = unsafe { &*(p as *const ShmHeader) };
    if !h.valid() || h.passes.load(SC) == 0 {
        h.init_defaults();
    }
    Ok(h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;

    /// Смещения всех полей против C++: собирается маленькая программа на shm_protocol.h.
    #[test]
    fn layout_matches_cpp() {
        macro_rules! fields {
            ($($r:ident = $c:literal),* $(,)?) => { vec![$((stringify!($r), $c, offset_of!(ShmHeader, $r))),*] };
        }
        let list = fields![
            magic = "magic", version = "version", seq_req = "seq_req", seq_resp = "seq_resp", width = "width",
            height = "height", format = "format", quit = "quit", heartbeat = "heartbeat", control_seq = "controlSeq",
            tuning_seq = "tuningSeq", enabled = "enabled", passes = "passes", unlock_passes = "unlockPasses",
            preset = "preset", style = "style", auto_mask = "autoMask", intensity_bits = "intensityBits",
            local_tone_bits = "localToneBits", local_structure_bits = "localStructureBits",
            skin_structure_bits = "skinStructureBits", sharpness_bits = "sharpnessBits",
            transfer_strength_bits = "transferStrengthBits", colour_strength_bits = "colourStrengthBits",
            max_ratio_bits = "maxRatioBits", transfer = "transfer", debug_view = "debugView",
            debug_scale_bits = "debugScaleBits", white_point_bits = "whitePointBits",
            white_point_scale_bits = "whitePointScaleBits", white_point_source = "whitePointSource",
            white_point_trim_bits = "whitePointTrimBits", working_scale_bits = "workingScaleBits",
            compare_mode = "compareMode", compare_split_bits = "compareSplitBits", compare_zoom_bits = "compareZoomBits",
            compare_swap = "compareSwap", colour_mode = "colourMode", capture_request = "captureRequest",
            toggle_key = "toggleKey", reversible_mode = "reversibleMode", apply_model = "applyModel",
            hold_frame = "holdFrame", scaling_downscaler = "scalingDownscaler", helper_state = "helperState",
            model_up = "modelUp", helper_frames_lo = "helperFramesLo", helper_frames_hi = "helperFramesHi",
            helper_eval_ms_bits = "helperEvalMsBits", helper_upload_ms_bits = "helperUploadMsBits",
            helper_readback_ms_bits = "helperReadbackMsBits", helper_vram_mb = "helperVramMB",
            helper_features = "helperFeatures", helper_pass_ceiling = "helperPassCeiling", layer_pid = "layerPid",
            layer_frames_lo = "layerFramesLo", layer_frames_hi = "layerFramesHi", layer_width = "layerWidth",
            layer_height = "layerHeight", layer_format = "layerFormat", layer_composition_up = "layerCompositionUp",
            layer_ms_bits = "layerMsBits", layer_measured_white_bits = "layerMeasuredWhiteBits",
            layer_heartbeat = "layerHeartbeat", helper_reason_seq = "helperReasonSeq", helper_reason = "helperReason",
            layer_reason_seq = "layerReasonSeq", layer_reason = "layerReason", game_name_seq = "gameNameSeq",
            game_name = "gameName", pass = "pass", mvec_enabled = "mvecEnabled", mvec_scale_mode = "mvecScaleMode",
            mvec_quality = "mvecQuality", seq_ok = "seq_ok", composition_bypass = "compositionBypass",
            rebuild_settle_ms = "rebuildSettleMs", answered_w = "answeredW", answered_h = "answeredH",
            proxy_export_seq = "proxyExportSeq", proxy_pid = "proxyPid", proxy_fd = "proxyFd", proxy_gen = "proxyGen",
            answer_export_seq = "answerExportSeq", answer_pid = "answerPid", answer_fd = "answerFd",
            answer_gen = "answerGen", layer_proxy_seq = "layerProxySeq", layer_answer_seq = "layerAnswerSeq",
            hdr_mode = "hdrMode", hdr_detected = "hdrDetected", hdr_active = "hdrActive", proxy_format = "proxyFormat",
            hdr_encode = "hdrEncode", colour_trust_percent = "colourTrustPercent",
            ratio_smooth_percent = "ratioSmoothPercent", sdr16_multipass = "sdr16Multipass",
            mvec_pixel_size = "mvecPixelSize",
        ];
        let Ok(cxx) = which("c++") else {
            eprintln!("нет c++ — сверка с C++ пропущена");
            return;
        };
        let dir = std::env::temp_dir().join(format!("syn-dlss-layout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let inc = concat!(env!("CARGO_MANIFEST_DIR"), "/../syn-wine/dlss5/common");
        let mut src = String::from("#include \"shm_protocol.h\"\n#include <cstdio>\nint main(){\n");
        for (_, c, _) in &list {
            src.push_str(&format!("printf(\"%zu\\n\", offsetof(ShmHeader, {c}));\n"));
        }
        src.push_str("printf(\"%zu\\n%zu\\n\", sizeof(ShmHeader), sizeof(PassControl));}\n");
        std::fs::write(dir.join("l.cpp"), src).unwrap();
        let exe = dir.join("l");
        let st = std::process::Command::new(cxx)
            .args(["-std=c++17", "-Wno-invalid-offsetof", "-I", inc, "-o"])
            .arg(&exe)
            .arg(dir.join("l.cpp"))
            .status()
            .unwrap();
        assert!(st.success(), "C++ не собрался");
        let out = String::from_utf8(std::process::Command::new(&exe).output().unwrap().stdout).unwrap();
        let nums: Vec<usize> = out.lines().map(|l| l.parse().unwrap()).collect();
        for ((r, c, off), cpp) in list.iter().zip(&nums) {
            assert_eq!(off, cpp, "{r} ({c})");
        }
        assert_eq!(nums[list.len()], std::mem::size_of::<ShmHeader>());
        assert_eq!(nums[list.len() + 1], std::mem::size_of::<PassControl>());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn which(cmd: &str) -> Result<std::path::PathBuf, ()> {
        std::env::var_os("PATH")
            .and_then(|p| std::env::split_paths(&p).map(|d| d.join(cmd)).find(|p| p.is_file()))
            .ok_or(())
    }

    #[test]
    fn reset_keeps_status() {
        let mut buf = vec![0u32; std::mem::size_of::<ShmHeader>() / 4 + 2];
        let h = unsafe { &*(buf.as_mut_ptr() as *const ShmHeader) };
        h.init_defaults();
        h.helper_state.store(HELPER_RUNNING, SC);
        h.passes.store(5, SC);
        h.reset_settings();
        assert_eq!(h.helper_state.load(SC), HELPER_RUNNING);
        assert_eq!(h.passes.load(SC), 1);
        assert_eq!(h.control_seq.load(SC), 1);
    }
}
