use synaptix::facade::params as sp;
use synaptixd::{DeviceKind, ModelKind, ParamKind, ParamSpec, ParamValue, Params, QuantOption};

use crate::registry::Manifest;

pub const SYSTEM_PROMPT: &str = "Ты — помощник, встроенный в операционную систему. Отвечай по делу, кратко и на языке пользователя.";

fn task(k: ModelKind) -> sp::Task {
    match k {
        ModelKind::Llm => sp::Task::Llm,
        ModelKind::Tts => sp::Task::Tts,
        ModelKind::Asr => sp::Task::Asr,
        ModelKind::Image => sp::Task::Image,
        ModelKind::Video => sp::Task::Video,
        ModelKind::Music => sp::Task::Music,
        ModelKind::Ctc => sp::Task::Ctc,
        ModelKind::Mfa => sp::Task::Mfa,
        ModelKind::Embed => sp::Task::Embed,
        ModelKind::Diarize => sp::Task::Diarize,
    }
}

pub fn kind_of(t: sp::Task) -> ModelKind {
    match t {
        sp::Task::Llm => ModelKind::Llm,
        sp::Task::Tts => ModelKind::Tts,
        sp::Task::Asr => ModelKind::Asr,
        sp::Task::Image => ModelKind::Image,
        sp::Task::Video => ModelKind::Video,
        sp::Task::Music => ModelKind::Music,
        sp::Task::Ctc => ModelKind::Ctc,
        sp::Task::Mfa => ModelKind::Mfa,
        sp::Task::Embed => ModelKind::Embed,
        sp::Task::Diarize => ModelKind::Diarize,
    }
}

fn value(v: sp::ParamValue) -> ParamValue {
    match v {
        sp::ParamValue::Bool(b) => ParamValue::Bool(b),
        sp::ParamValue::Int(i) => ParamValue::Int(i),
        sp::ParamValue::Float(f) => ParamValue::Float(f),
        sp::ParamValue::Text(t) => ParamValue::Text(t),
    }
}

fn kind(k: sp::ParamKind) -> ParamKind {
    match k {
        sp::ParamKind::Text { multiline } => ParamKind::Text { multiline },
        sp::ParamKind::Int { min, max, step } => ParamKind::Int { min, max, step },
        sp::ParamKind::Float { min, max, step, decimals } => ParamKind::Float { min, max, step, decimals },
        sp::ParamKind::Bool => ParamKind::Bool,
        sp::ParamKind::Choice { options } => ParamKind::Choice { options },
        sp::ParamKind::File { patterns } => ParamKind::File { patterns },
    }
}

pub fn schema(m: &Manifest) -> Vec<ParamSpec> {
    let path = m.variants.first().map(|v| m.path(v));
    let mut out: Vec<ParamSpec> = sp::param_schema(task(m.kind), &m.arch, &m.id, path.as_deref())
        .into_iter()
        .map(|s| ParamSpec { key: s.key.into(), label: s.label.into(), hint: s.hint.into(), kind: kind(s.kind), default: s.default.map(value) })
        .collect();
    for s in out.iter_mut() {
        if s.key == "system_prompt" && s.default.is_none() {
            s.default = Some(ParamValue::Text(SYSTEM_PROMPT.into()));
        }
        if let Some(v) = m.defaults.get(&s.key).and_then(|v| s.normalize(v)) {
            s.default = Some(v);
        }
    }
    out
}

pub fn optimal(schema: &[ParamSpec]) -> Params {
    Params::from_schema(schema)
}

pub fn quant_options(m: &Manifest) -> Vec<QuantOption> {
    let devices = m.devices();
    let mut out = vec![QuantOption { id: "auto".into(), devices: devices.clone() }];
    if m.kind == ModelKind::Llm {
        out.push(QuantOption { id: "int8".into(), devices: devices.clone() });
        if devices.contains(&DeviceKind::Cuda) {
            for id in ["nvfp4", "mxfp8", "bf16"] {
                out.push(QuantOption { id: id.into(), devices: vec![DeviceKind::Cuda] });
            }
        }
        return out;
    }
    let mut precisions: Vec<&str> = Vec::new();
    for v in &m.variants {
        if !v.precision.is_empty() && !precisions.contains(&v.precision.as_str()) {
            precisions.push(&v.precision);
        }
    }
    if precisions.len() > 1 {
        for p in precisions {
            let devs = m.variants.iter().filter(|v| v.precision == p).flat_map(|v| v.devices.iter().copied()).fold(Vec::new(), |mut acc, d| {
                if !acc.contains(&d) {
                    acc.push(d);
                }
                acc
            });
            out.push(QuantOption { id: p.to_string(), devices: devs });
        }
    }
    // Аудио-модели: линейные слои квантуются на лету при загрузке (int8 —
    // везде, NVFP4/MXFP8 — на CUDA) независимо от формата весов, либо
    // считаются в выбранной плотной точности (половинная — на ускорителях,
    // F32 — везде).
    if matches!(m.kind, ModelKind::Asr | ModelKind::Tts | ModelKind::Ctc) {
        for p in crate::runners::AUDIO_PRECISIONS {
            if out.iter().any(|q| q.id == p) {
                continue;
            }
            let cuda = |d: &DeviceKind| matches!(d, DeviceKind::Cuda | DeviceKind::Gpu);
            let devs: Vec<DeviceKind> = match p {
                "int8" | "f32" => devices.clone(),
                "nvfp4" | "mxfp8" | "bf16" => devices.iter().copied().filter(cuda).collect(),
                _ => devices.iter().copied().filter(|d| *d != DeviceKind::Cpu).collect(),
            };
            if !devs.is_empty() {
                out.push(QuantOption { id: p.into(), devices: devs });
            }
        }
    }
    out
}
