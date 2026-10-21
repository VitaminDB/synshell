//! `config.ini` (общий с `dlssnr-helper`) и профили: ключи помощника (runner_type, runner_path, binaries,
//! shm, log, dxvk_*) и все настройки `set_*` в виде чисел, как у Qt-окна форка — файлы взаимозаменяемы.

use std::path::Path;

use crate::shm::{b2f, f2b, Field, PassField, ShmHeader, MAX_PASSES};

/// Ключи и значения в порядке файла; неизвестные ключи сохраняются.
#[derive(Debug, Clone, Default)]
pub struct Ini {
    pub entries: Vec<(String, String)>,
}

impl Ini {
    pub fn parse(text: &str) -> Self {
        let mut entries = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let mut v = v.trim();
            if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
                v = &v[1..v.len() - 1];
            }
            entries.push((k.trim().to_string(), v.to_string()));
        }
        Self { entries }
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    pub fn load() -> Self {
        Self::load_from(&crate::paths::config_file())
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        match self.entries.iter_mut().find(|(k, _)| k == key) {
            Some(e) => e.1 = value,
            None => self.entries.push((key.to_string(), value)),
        }
    }

    pub fn text(&self) -> String {
        self.entries.iter().map(|(k, v)| format!("{k}={v}\n")).collect()
    }

    /// Атомарная запись (временный файл и переименование): помощник может читать файл в этот момент.
    pub fn save_to(&self, path: &Path, header: &str) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
        std::fs::write(&tmp, format!("{header}{}", self.text()))?;
        std::fs::rename(tmp, path)
    }
}

/// Настройка заголовка, хранимая в файле: ключ, поле, дробное ли, обратное ли (чекбокс «включено»
/// над полем «обход», как композиция над compositionBypass).
pub struct Setting {
    pub key: &'static str,
    pub field: Field,
    pub float: bool,
    pub invert: bool,
}

macro_rules! fld {
    ($n:ident) => {{
        fn g(h: &$crate::shm::ShmHeader) -> &std::sync::atomic::AtomicU32 {
            &h.$n
        }
        g as $crate::shm::Field
    }};
}
pub(crate) use fld;

macro_rules! pfld {
    ($n:ident) => {{
        fn g(p: &$crate::shm::PassControl) -> &std::sync::atomic::AtomicU32 {
            &p.$n
        }
        g as $crate::shm::PassField
    }};
}
pub(crate) use pfld;

const fn s(key: &'static str, field: Field, float: bool) -> Setting {
    Setting { key, field, float, invert: false }
}

/// Все настройки, которые пользователь задаёт (без holdFrame/captureRequest — это живые инструменты),
/// с ключами Qt-окна.
pub fn settings() -> &'static [Setting] {
    static T: std::sync::OnceLock<Vec<Setting>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        vec![
            s("set_enabled", fld!(enabled), false),
            s("set_passes", fld!(passes), false),
            s("set_rebuild_settle_ms", fld!(rebuild_settle_ms), false),
            s("set_model_resolution", fld!(working_scale_bits), true),
            s("set_down_leg_filter", fld!(scaling_downscaler), false),
            s("set_detail_strength", fld!(transfer_strength_bits), true),
            s("set_colour_strength", fld!(colour_strength_bits), true),
            s("set_highlight_guard", fld!(max_ratio_bits), true),
            s("set_enlargement", fld!(transfer), false),
            Setting { key: "set_composition_enabled", field: fld!(composition_bypass), float: false, invert: true },
            s("set_preset", fld!(preset), false),
            s("set_style", fld!(style), false),
            s("set_intensity", fld!(intensity_bits), true),
            s("set_local_structure", fld!(local_structure_bits), true),
            s("set_local_tone", fld!(local_tone_bits), true),
            s("set_skin_structure", fld!(skin_structure_bits), true),
            s("set_auto_mask", fld!(auto_mask), false),
            s("set_sharpness", fld!(sharpness_bits), true),
            s("set_motion_enabled", fld!(mvec_enabled), false),
            s("set_motion_quality", fld!(mvec_quality), false),
            s("set_motion_units", fld!(mvec_scale_mode), false),
            s("set_colour_mode", fld!(colour_mode), false),
            s("set_hdr_mode", fld!(hdr_mode), false),
            s("set_sdr_16bit_multipass", fld!(sdr16_multipass), false),
            s("set_white_point_source", fld!(white_point_source), false),
            s("set_paper_white", fld!(white_point_bits), true),
            s("set_white_point_scale", fld!(white_point_scale_bits), true),
            s("set_white_point_trim", fld!(white_point_trim_bits), true),
            s("set_apply_model", fld!(apply_model), false),
            s("set_proxy", fld!(reversible_mode), false),
            s("set_debug_view", fld!(debug_view), false),
            s("set_debug_scale", fld!(debug_scale_bits), true),
            s("set_compare", fld!(compare_mode), false),
            s("set_compare_split", fld!(compare_split_bits), true),
            s("set_compare_zoom", fld!(compare_zoom_bits), true),
            s("set_compare_swap", fld!(compare_swap), false),
            s("set_toggle_key", fld!(toggle_key), false),
            // Не было в таблице Qt-окна — в файле с нашими ключами, Qt их просто пропустит.
            s("set_motion_pixel_size", fld!(mvec_pixel_size), false),
            s("set_colour_bound", fld!(colour_trust_percent), false),
            s("set_ratio_smooth", fld!(ratio_smooth_percent), false),
        ]
    })
}

pub struct PassSetting {
    pub suffix: &'static str,
    pub float: bool,
    pub field: PassField,
}

pub fn pass_settings() -> &'static [PassSetting] {
    static T: std::sync::OnceLock<Vec<PassSetting>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let p = |suffix, float, field| PassSetting { suffix, float, field };
        vec![
            p("override_mask", false, pfld!(override_mask)),
            p("intensity", true, pfld!(intensity_bits)),
            p("local_structure", true, pfld!(local_structure_bits)),
            p("local_tone", true, pfld!(local_tone_bits)),
            p("skin_structure", true, pfld!(skin_structure_bits)),
            p("sharpness", true, pfld!(sharpness_bits)),
            p("style", false, pfld!(style)),
            p("preset", false, pfld!(preset)),
            p("auto_mask", false, pfld!(auto_mask)),
        ]
    })
}

fn num_text(raw: u32, float: bool) -> String {
    if float {
        let f = b2f(raw);
        // как QString::number(f, 'g', 9): без лишних нулей
        let s = format!("{f}");
        if s.contains('e') { format!("{f:e}") } else { s }
    } else {
        raw.to_string()
    }
}

fn parse_raw(v: &str, float: bool) -> Option<u32> {
    if float {
        v.trim().parse::<f32>().ok().map(f2b)
    } else {
        v.trim().parse::<u32>().ok().or_else(|| v.trim().parse::<f64>().ok().map(|f| f.max(0.0) as u32))
    }
}

/// Все настройки заголовка парами «ключ = значение» (порядок таблиц).
pub fn settings_pairs(h: &ShmHeader) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for e in settings() {
        let raw = h.get(e.field);
        let v = if e.invert { (if raw != 0 { 0 } else { 1 }).to_string() } else { num_text(raw, e.float) };
        out.push((e.key.to_string(), v));
    }
    for (i, pc) in h.pass.iter().enumerate() {
        for e in pass_settings() {
            out.push((format!("set_pass_{i}_{}", e.suffix), num_text((e.field)(pc).load(std::sync::atomic::Ordering::SeqCst), e.float)));
        }
    }
    out
}

/// Отпечаток настроек: изменился — кто-то (окно, dlssnr-shmctl, скрипт) поменял настройку.
pub fn settings_blob(h: &ShmHeader) -> String {
    settings_pairs(h).into_iter().map(|(k, v)| format!("{k}={v};")).collect()
}

/// Применить `set_*` из файла к заголовку. Возвращает число применённых ключей.
pub fn apply_settings(h: &ShmHeader, ini: &Ini) -> usize {
    use std::sync::atomic::Ordering::SeqCst;
    let mut n = 0;
    for (k, v) in &ini.entries {
        if let Some(e) = settings().iter().find(|e| e.key == k) {
            let raw = if e.invert {
                parse_raw(v, false).map(|x| if x != 0 { 0 } else { 1 })
            } else {
                parse_raw(v, e.float)
            };
            if let Some(raw) = raw {
                (e.field)(h).store(raw, SeqCst);
                n += 1;
            }
        } else if let Some(rest) = k.strip_prefix("set_pass_") {
            let Some((idx, suffix)) = rest.split_once('_') else { continue };
            let Ok(i) = idx.parse::<usize>() else { continue };
            if i >= MAX_PASSES {
                continue;
            }
            if let Some(e) = pass_settings().iter().find(|e| e.suffix == suffix) {
                if let Some(raw) = parse_raw(v, e.float) {
                    (e.field)(&h.pass[i]).store(raw, SeqCst);
                    n += 1;
                }
            }
        }
    }
    if n > 0 {
        h.control_seq.fetch_add(1, SeqCst);
        h.tuning_seq.fetch_add(1, SeqCst);
    }
    n
}

/// Сохранить config.ini: ключи помощника как есть, настройки — из заголовка.
pub fn save_config(h: Option<&ShmHeader>, extra: &[(&str, String)]) {
    let path = crate::paths::config_file();
    let mut ini = Ini::load_from(&path);
    for k in ["runner_type", "runner_path", "binaries", "shm", "log", "dxvk_vendor", "dxvk_device"] {
        if ini.get(k).is_none() {
            ini.set(k, "");
        }
    }
    for (k, v) in extra {
        ini.set(k, v.clone());
    }
    if let Some(h) = h {
        for (k, v) in settings_pairs(h) {
            ini.set(&k, v);
        }
    }
    if let Err(e) = ini.save_to(&path, "") {
        tracing::warn!("config.ini: {e}");
    }
}

/// Профили — `~/.config/dlssnr/profiles/*.ini`, имя файла без `.ini`.
pub fn profiles() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(crate::paths::profiles_dir())
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".ini").map(str::to_string)))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|s| s.to_lowercase());
    v
}

pub fn profile_path(name: &str) -> std::path::PathBuf {
    let clean: String = name.trim().chars().map(|c| if c == '/' || c == '\0' { '_' } else { c }).collect();
    crate::paths::profiles_dir().join(format!("{clean}.ini"))
}

pub fn save_profile(h: &ShmHeader, name: &str) -> std::io::Result<()> {
    let ini = Ini { entries: settings_pairs(h) };
    ini.save_to(&profile_path(name), "# DLSS5VKLayer settings\n# Generated by syn-dlss\n\n")
}

pub fn load_profile(h: &ShmHeader, name: &str) -> usize {
    apply_settings(h, &Ini::load_from(&profile_path(name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> (Vec<u32>, &'static ShmHeader) {
        let mut buf = vec![0u32; std::mem::size_of::<ShmHeader>() / 4 + 2];
        let h = unsafe { &*(buf.as_mut_ptr() as *const ShmHeader) };
        h.init_defaults();
        (buf, h)
    }

    #[test]
    fn roundtrip() {
        let (_b1, a) = header();
        a.set(fld!(intensity_bits), f2b(1.75), true);
        a.set(fld!(composition_bypass), 0, false);
        a.pass[3].override_mask.store(5, std::sync::atomic::Ordering::SeqCst);
        let ini = Ini::parse(&Ini { entries: settings_pairs(a) }.text());
        assert_eq!(ini.get("set_composition_enabled"), Some("1"));
        assert_eq!(ini.get("set_intensity"), Some("1.75"));
        let (_b2, b) = header();
        assert!(apply_settings(b, &ini) > 0);
        assert_eq!(settings_blob(a), settings_blob(b));
    }
}
