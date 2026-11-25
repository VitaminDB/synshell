//! Пресеты движения (вход, движение по клипу, выход): применяют набор ключевых
//! кадров эффекта `transform` (x, y, scale, rotation, opacity) со сглаживанием.
//! После применения ключи можно править в инспекторе, в панели «Кривые» и
//! рамкой прямо на просмотре.

use serde_json::{json, Map, Value};

use crate::doc::{Effect, Id, Key, Project};

/// Куда ложится пресет во времени клипа.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Вход в начале клипа `[0, d]`.
    In,
    /// Движение на всю длину клипа `[0, dur]`.
    Full,
    /// Выход в конце клипа `[dur - d, dur]`.
    Out,
}

/// Описание пресета движения.
#[derive(Clone, Copy, Debug)]
pub struct Preset {
    pub id: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    pub desc: &'static str,
    pub slot: Slot,
    /// Длительность по умолчанию, с (`0.0` — на всю длину клипа).
    pub default_dur: f64,
}

pub const PRESETS: &[Preset] = &[
    // ── вход ──
    Preset { id: "slide_in_left", title: "Выезд слева", category: "Вход", desc: "плавный выезд слева с проявлением", slot: Slot::In, default_dur: 0.6 },
    Preset { id: "slide_in_right", title: "Выезд справа", category: "Вход", desc: "плавный выезд справа с проявлением", slot: Slot::In, default_dur: 0.6 },
    Preset { id: "slide_in_up", title: "Подъём снизу", category: "Вход", desc: "подъём снизу с проявлением", slot: Slot::In, default_dur: 0.6 },
    Preset { id: "pop_in", title: "Выпрыгивание", category: "Вход", desc: "увеличение с лёгким перелётом (back)", slot: Slot::In, default_dur: 0.55 },
    Preset { id: "bounce_in", title: "Отскок", category: "Вход", desc: "падение сверху с отскоком (bounce)", slot: Slot::In, default_dur: 0.75 },
    Preset { id: "spin_in", title: "Влёт с поворотом", category: "Вход", desc: "увеличение и разворот на место", slot: Slot::In, default_dur: 0.65 },
    // ── движение по клипу ──
    Preset { id: "ken_burns_in", title: "Наезд", category: "Движение", desc: "плавное приближение (Ken Burns)", slot: Slot::Full, default_dur: 0.0 },
    Preset { id: "ken_burns_out", title: "Отъезд", category: "Движение", desc: "плавное отдаление (Ken Burns)", slot: Slot::Full, default_dur: 0.0 },
    Preset { id: "pan_left", title: "Панорама влево", category: "Движение", desc: "лёгкий зум и ведение кадра влево", slot: Slot::Full, default_dur: 0.0 },
    Preset { id: "pan_right", title: "Панорама вправо", category: "Движение", desc: "лёгкий зум и ведение кадра вправо", slot: Slot::Full, default_dur: 0.0 },
    Preset { id: "float", title: "Покачивание", category: "Движение", desc: "мягкое плавание по вертикали и углу", slot: Slot::Full, default_dur: 0.0 },
    Preset { id: "pulse", title: "Пульсация", category: "Движение", desc: "ритмичное увеличение и возврат", slot: Slot::Full, default_dur: 0.0 },
    // ── выход ──
    Preset { id: "zoom_out_exit", title: "Уход вдаль", category: "Выход", desc: "уменьшение и растворение в конце клипа", slot: Slot::Out, default_dur: 0.6 },
    Preset { id: "slide_out_down", title: "Уход вниз", category: "Выход", desc: "сдвиг вниз и растворение в конце клипа", slot: Slot::Out, default_dur: 0.6 },
];

pub fn find(id: &str) -> Option<&'static Preset> {
    let q = id.trim().to_lowercase();
    PRESETS.iter().find(|p| p.id == q || p.title.to_lowercase() == q)
}

/// Ключи пресета: (имя поля `transform`, доля времени 0–1 в окне, значение, сглаживание).
fn spec(id: &str) -> (&'static [(&'static str, f64)], &'static [(&'static str, f64, f64, &'static str)]) {
    match id {
        "slide_in_left" => (&[], &[("x", 0.0, -0.85, "out"), ("x", 1.0, 0.0, "linear"), ("opacity", 0.0, 0.0, "out"), ("opacity", 1.0, 1.0, "linear")]),
        "slide_in_right" => (&[], &[("x", 0.0, 0.85, "out"), ("x", 1.0, 0.0, "linear"), ("opacity", 0.0, 0.0, "out"), ("opacity", 1.0, 1.0, "linear")]),
        "slide_in_up" => (&[], &[("y", 0.0, 0.55, "out"), ("y", 1.0, 0.0, "linear"), ("opacity", 0.0, 0.0, "out"), ("opacity", 1.0, 1.0, "linear")]),
        "pop_in" => (&[], &[("scale", 0.0, 0.25, "back"), ("scale", 1.0, 1.0, "linear"), ("opacity", 0.0, 0.0, "out"), ("opacity", 1.0, 1.0, "linear")]),
        "bounce_in" => (&[], &[("y", 0.0, -0.65, "bounce"), ("y", 1.0, 0.0, "linear"), ("opacity", 0.0, 0.0, "out"), ("opacity", 0.4, 1.0, "linear")]),
        "spin_in" => (&[], &[("scale", 0.0, 0.25, "back"), ("scale", 1.0, 1.0, "linear"), ("rotation", 0.0, -35.0, "out"), ("rotation", 1.0, 0.0, "linear"), ("opacity", 0.0, 0.0, "out"), ("opacity", 1.0, 1.0, "linear")]),
        "ken_burns_in" => (&[], &[("scale", 0.0, 1.0, "ease"), ("scale", 1.0, 1.22, "linear")]),
        "ken_burns_out" => (&[], &[("scale", 0.0, 1.22, "ease"), ("scale", 1.0, 1.0, "linear")]),
        "pan_left" => (&[("scale", 1.16)], &[("x", 0.0, 0.07, "ease"), ("x", 1.0, -0.07, "linear")]),
        "pan_right" => (&[("scale", 1.16)], &[("x", 0.0, -0.07, "ease"), ("x", 1.0, 0.07, "linear")]),
        "float" => (
            &[],
            &[
                ("y", 0.0, 0.0, "ease"),
                ("y", 0.25, -0.035, "ease"),
                ("y", 0.5, 0.0, "ease"),
                ("y", 0.75, 0.035, "ease"),
                ("y", 1.0, 0.0, "linear"),
                ("rotation", 0.0, 0.0, "ease"),
                ("rotation", 0.25, 2.2, "ease"),
                ("rotation", 0.5, 0.0, "ease"),
                ("rotation", 0.75, -2.2, "ease"),
                ("rotation", 1.0, 0.0, "linear"),
            ],
        ),
        "pulse" => (&[], &[("scale", 0.0, 1.0, "ease"), ("scale", 0.25, 1.12, "ease"), ("scale", 0.5, 1.0, "ease"), ("scale", 0.75, 1.12, "ease"), ("scale", 1.0, 1.0, "linear")]),
        "zoom_out_exit" => (&[], &[("scale", 0.0, 1.0, "in"), ("scale", 1.0, 0.3, "linear"), ("opacity", 0.0, 1.0, "in"), ("opacity", 1.0, 0.0, "linear")]),
        "slide_out_down" => (&[], &[("y", 0.0, 0.0, "in"), ("y", 1.0, 0.55, "linear"), ("opacity", 0.0, 1.0, "in"), ("opacity", 1.0, 0.0, "linear")]),
        _ => (&[], &[]),
    }
}

/// Применить пресет движения к клипу `cid`. Вход и выход не затирают друг друга,
/// повторный вход/выход заменяет прежний в своём окне, движение на весь клип
/// заменяет все ключи `transform`. Возвращает id эффекта `transform`.
pub fn apply(p: &mut Project, cid: Id, preset: &Preset, dur: Option<f64>) -> Result<Id, String> {
    let fps = p.settings.fps.max(1.0);
    let clip_dur = p.clip(cid).ok_or_else(|| format!("нет клипа c{cid}"))?.duration;
    let frame = 1.0 / fps;
    let d = match preset.slot {
        Slot::Full => dur.unwrap_or(clip_dur).clamp(frame, clip_dur),
        Slot::In | Slot::Out => dur.unwrap_or(preset.default_dur).clamp(frame, clip_dur),
    };
    let (t0, t1) = match preset.slot {
        Slot::In => (0.0, d),
        Slot::Full => (0.0, d),
        Slot::Out => ((clip_dur - d).max(0.0), clip_dur),
    };
    let eid = match p.clip(cid).unwrap().effects.iter().find(|e| e.kind == "transform").map(|e| e.id) {
        Some(id) => id,
        None => {
            let id = p.alloc();
            p.clip_mut(cid).unwrap().effects.push(Effect { id, kind: "transform".into(), params: Map::new() });
            id
        }
    };
    let snap = |t: f64| ((t.clamp(0.0, clip_dur) * fps).round() / fps * 1e6).round() / 1e6;
    let prefix = format!("e{eid}.");
    let half = 0.5 / fps;
    let (consts, keys_spec) = spec(preset.id);

    let cm = p.clip_mut(cid).unwrap();
    // очистить прежние ключи transform в окне пресета
    let paths: Vec<String> = cm.keys.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
    for path in paths {
        let remove_all = preset.slot == Slot::Full;
        if remove_all {
            cm.keys.remove(&path);
        } else if let Some(ks) = cm.keys.get_mut(&path) {
            match preset.slot {
                Slot::In => ks.retain(|k| k.t > t1.max(clip_dur * 0.5) + half),
                Slot::Out => ks.retain(|k| k.t < t0.min(clip_dur * 0.5) - half),
                Slot::Full => ks.clear(),
            }
            if ks.is_empty() {
                cm.keys.remove(&path);
            }
        }
    }
    if let Some(ef) = cm.effects.iter_mut().find(|e| e.id == eid) {
        if preset.slot == Slot::Full {
            ef.params.clear();
        }
        for &(k, v) in consts {
            ef.params.insert(k.to_string(), json!(v));
        }
    }
    for &(param, frac, val, ease) in keys_spec {
        let t = snap(t0 + (t1 - t0) * frac);
        let path = format!("{prefix}{param}");
        let ks = cm.keys.entry(path).or_default();
        ks.retain(|k| (k.t - t).abs() > half);
        ks.push(Key { t, v: Value::from(val), ease: ease.to_string() });
        ks.sort_by(|a, b| a.t.total_cmp(&b.t));
    }
    Ok(eid)
}
