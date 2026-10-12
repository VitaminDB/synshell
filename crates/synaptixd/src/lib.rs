pub mod params;
pub mod wire;

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use params::{ParamKey, ParamType, ParamValue, Params};
pub use wire::Mapped;

pub const SOCKET: &str = "/run/synaptix/synaptixd.sock";
pub const VERSION: u32 = 1;
pub const SYSTEM_MODELS: &str = "/usr/share/synaptix/models";

pub fn socket_path() -> PathBuf {
    std::env::var_os("SYNAPTIXD_SOCKET").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(SOCKET))
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    #[default]
    Auto,
    Cpu,
    Cuda,
    Gpu,
    Npu,
}

impl DeviceKind {
    pub const ALL: [DeviceKind; 5] = [DeviceKind::Auto, DeviceKind::Npu, DeviceKind::Cpu, DeviceKind::Gpu, DeviceKind::Cuda];

    pub fn id(self) -> &'static str {
        match self {
            DeviceKind::Auto => "auto",
            DeviceKind::Cpu => "cpu",
            DeviceKind::Cuda => "cuda",
            DeviceKind::Gpu => "gpu",
            DeviceKind::Npu => "npu",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "auto" => DeviceKind::Auto,
            "cpu" => DeviceKind::Cpu,
            "cuda" => DeviceKind::Cuda,
            "gpu" | "vulkan" | "wgpu" => DeviceKind::Gpu,
            "npu" => DeviceKind::Npu,
            _ => return None,
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Ctc,
    Mfa,
    Asr,
    Llm,
    Tts,
    Embed,
    Image,
    Video,
    Music,
}

impl ModelKind {
    pub const ALL: [ModelKind; 9] = [
        ModelKind::Llm,
        ModelKind::Image,
        ModelKind::Video,
        ModelKind::Music,
        ModelKind::Tts,
        ModelKind::Asr,
        ModelKind::Embed,
        ModelKind::Ctc,
        ModelKind::Mfa,
    ];

    pub const DEFAULTABLE: [ModelKind; 7] =
        [ModelKind::Llm, ModelKind::Image, ModelKind::Video, ModelKind::Music, ModelKind::Tts, ModelKind::Asr, ModelKind::Embed];

    pub fn id(self) -> &'static str {
        match self {
            ModelKind::Ctc => "ctc",
            ModelKind::Mfa => "mfa",
            ModelKind::Asr => "asr",
            ModelKind::Llm => "llm",
            ModelKind::Tts => "tts",
            ModelKind::Embed => "embed",
            ModelKind::Image => "image",
            ModelKind::Video => "video",
            ModelKind::Music => "music",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == s.trim())
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum LoadState {
    #[default]
    Unloaded,
    Loading,
    Loaded,
    Failed,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct ModelSettings {
    #[serde(default)]
    pub device: DeviceKind,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_unload_min: Option<u32>,
    #[serde(default)]
    pub preload: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quant: Option<String>,
    #[serde(default, skip_serializing_if = "Params::is_empty")]
    pub params: Params,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct QuantOption {
    pub id: String,
    pub devices: Vec<DeviceKind>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct VariantInfo {
    pub precision: String,
    pub devices: Vec<DeviceKind>,
    pub bytes: u64,
}

pub fn precision_label(p: &str) -> String {
    match p {
        "w8a16" | "q8row" => "int8 по каналу (веса), счёт F16".into(),
        "w8" => "int8 по каналу (веса)".into(),
        "auto" => "оптимальное для устройства".into(),
        "int8" | "sq8" => "int8".into(),
        "int4" | "sq4" => "int4".into(),
        "nvfp4" => "NVFP4".into(),
        "mxfp8" => "MXFP8".into(),
        "f16" => "F16".into(),
        "bf16" => "BF16".into(),
        "f32" => "F32".into(),
        "" => "—".into(),
        other => other.to_uppercase(),
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct ModelEntry {
    pub id: String,
    pub title: String,
    pub kind: ModelKind,
    pub arch: String,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub package: String,
    pub disk_bytes: u64,
    pub devices: Vec<DeviceKind>,
    pub settings: ModelSettings,
    pub state: LoadState,
    pub device: Option<DeviceKind>,
    #[serde(default)]
    pub precision: String,
    #[serde(default)]
    pub fallback: bool,
    pub mem_bytes: u64,
    #[serde(default)]
    pub vram_bytes: u64,
    pub clients: u32,
    pub busy: bool,
    pub last_used_ms: i64,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub npu_cache: bool,
    #[serde(default)]
    pub variants: Vec<VariantInfo>,
    #[serde(default)]
    pub quant_options: Vec<QuantOption>,
    #[serde(default)]
    pub defaults: Params,
    #[serde(default)]
    pub runnable: bool,
}

impl ModelEntry {
    pub fn effective(&self) -> Params {
        self.defaults.over(&self.settings.params)
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct DefaultModel {
    pub kind: ModelKind,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub resolved: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct Status {
    pub version: u32,
    pub since_ms: i64,
    pub mem_total: u64,
    pub mem_available: u64,
    pub models_bytes: u64,
    pub loaded: u32,
    pub evictions: u64,
    pub devices: Vec<DeviceKind>,
    pub min_available_mb: u64,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct CtcInfo {
    pub vocab: Vec<String>,
    pub blank_id: u32,
    pub hidden_size: usize,
    pub frame_sec: f32,
    pub sample_rate: u32,
    pub length_buckets: Vec<usize>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct ModelInfo {
    pub entry: ModelEntry,
    #[serde(default)]
    pub ctc: Option<CtcInfo>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct AlignWord {
    pub phones: Vec<String>,
    #[serde(default)]
    pub silence_after: Option<f32>,
    #[serde(default)]
    pub silence_before_correction: Option<f32>,
    #[serde(default)]
    pub non_silence_before_correction: Option<f32>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct AlignWordAlts {
    pub alts: Vec<(AlignWord, f32)>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct AlignOptions {
    pub acoustic_scale: f32,
    pub transition_scale: f32,
    pub self_loop_scale: f32,
    pub beam: Option<f32>,
    pub retry_beam: Option<f32>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct PhoneSpan {
    pub phone: String,
    pub word: Option<usize>,
    pub alt: usize,
    pub start: usize,
    pub end: usize,
    pub loglike: f32,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Alignment {
    pub phones: Vec<PhoneSpan>,
    pub frame_shift: f32,
    pub score: f32,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct FdRef {
    pub fd: u32,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Hello {
        client: String,
    },
    List,
    Status,
    Open {
        model: String,
    },
    Close {
        model: String,
    },
    Load {
        model: String,
    },
    Unload {
        model: String,
    },
    Set {
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<DeviceKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pinned: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "double_option")]
        idle_unload_min: Option<Option<u32>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preload: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "double_option")]
        quant: Option<Option<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        params: Option<Params>,
    },
    Rescan,
    Defaults,
    SetDefault {
        kind: ModelKind,
        #[serde(default)]
        model: Option<String>,
    },
    Resolve {
        kind: ModelKind,
    },
    Ctc {
        model: String,
        pcm: FdRef,
        samples: usize,
        sample_rate: u32,
        hidden: bool,
    },
    MfaAlign {
        model: String,
        pcm: FdRef,
        samples: usize,
        words: Vec<AlignWordAlts>,
        #[serde(default)]
        options: Option<AlignOptions>,
    },
    Transcribe {
        model: String,
        pcm: FdRef,
        samples: usize,
        sample_rate: u32,
        #[serde(default)]
        language: Option<String>,
    },
    Speak {
        model: String,
        text: String,
        #[serde(default)]
        language: Option<String>,
        #[serde(default)]
        instruct: Option<String>,
        #[serde(default)]
        speed: Option<f32>,
        #[serde(default)]
        steps: Option<usize>,
    },
    Generate {
        model: String,
        messages: Vec<Message>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_tokens: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        temperature: Option<f32>,
        #[serde(default, skip_serializing_if = "Params::is_empty")]
        params: Params,
    },
    Subscribe,
}

fn double_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> std::result::Result<Option<Option<T>>, D::Error> {
    Deserialize::deserialize(d).map(Some)
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Envelope {
    pub id: u64,
    #[serde(flatten)]
    pub req: Request,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Unavailable,
    Protocol,
    NoModel,
    WrongKind,
    NoMemory,
    Device,
    Denied,
    Failed,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Token { text: String },
    ModelState { model: ModelEntry },
    ModelsChanged,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct Reply {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<ErrorCode>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<Event>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Hello {
    pub version: u32,
    pub devices: Vec<DeviceKind>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct CtcReply {
    pub frames: usize,
    pub vocab: usize,
    pub frame_sec: f32,
    pub logprobs: FdRef,
    pub hidden: Option<FdRef>,
    pub hidden_dim: usize,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct SpeechReply {
    pub samples: usize,
    pub sample_rate: u32,
    pub audio: FdRef,
}

pub struct Speech {
    pub sample_rate: u32,
    pub audio: Mapped,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Segment {
    pub start: f32,
    pub end: f32,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Transcript {
    pub text: String,
    #[serde(default)]
    pub segments: Vec<Segment>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Generated {
    pub text: String,
    pub tokens: usize,
}

#[derive(Debug, Clone)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::new(ErrorCode::Unavailable, format!("служба нейросетей: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct CtcOutput {
    pub frames: usize,
    pub vocab: usize,
    pub frame_sec: f32,
    pub logprobs: Mapped,
    pub hidden: Option<Mapped>,
    pub hidden_dim: usize,
}

pub struct Client {
    sock: OwnedFd,
    next_id: u64,
}

impl Client {
    pub fn connect() -> Result<Self> {
        Self::connect_to(&socket_path())
    }

    pub fn connect_to(path: &std::path::Path) -> Result<Self> {
        let sock = wire::connect(path)?;
        Ok(Self { sock, next_id: 1 })
    }

    pub fn send(&mut self, req: Request, fds: &[BorrowedFd]) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        let line = serde_json::to_vec(&Envelope { id, req }).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))?;
        wire::send(self.sock.as_fd(), &line, fds)?;
        Ok(id)
    }

    pub fn recv(&mut self) -> Result<(Reply, Vec<OwnedFd>)> {
        let (bytes, fds) = wire::recv(self.sock.as_fd())?;
        let reply: Reply = serde_json::from_slice(&bytes).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))?;
        Ok((reply, fds))
    }

    pub fn call_events(
        &mut self,
        req: Request,
        fds: &[BorrowedFd],
        mut on_event: impl FnMut(Event),
    ) -> Result<(Value, Vec<OwnedFd>)> {
        let id = self.send(req, fds)?;
        loop {
            let (reply, fds) = self.recv()?;
            if reply.id != id {
                continue;
            }
            if let Some(ev) = reply.event {
                on_event(ev);
                continue;
            }
            if reply.ok == Some(true) {
                return Ok((reply.data, fds));
            }
            return Err(Error::new(
                reply.code.unwrap_or(ErrorCode::Failed),
                reply.error.unwrap_or_else(|| "ошибка службы нейросетей".into()),
            ));
        }
    }

    pub fn call(&mut self, req: Request, fds: &[BorrowedFd]) -> Result<(Value, Vec<OwnedFd>)> {
        self.call_events(req, fds, |_| {})
    }

    pub fn call_typed<T: DeserializeOwned>(&mut self, req: Request) -> Result<T> {
        let (v, _) = self.call(req, &[])?;
        serde_json::from_value(v).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))
    }

    pub fn hello(&mut self, client: &str) -> Result<Hello> {
        self.call_typed(Request::Hello { client: client.into() })
    }

    pub fn list(&mut self) -> Result<Vec<ModelEntry>> {
        self.call_typed(Request::List)
    }

    pub fn status(&mut self) -> Result<Status> {
        self.call_typed(Request::Status)
    }

    pub fn open(&mut self, model: &str) -> Result<ModelInfo> {
        self.call_typed(Request::Open { model: model.into() })
    }

    pub fn load(&mut self, model: &str) -> Result<ModelEntry> {
        self.call_typed(Request::Load { model: model.into() })
    }

    pub fn unload(&mut self, model: &str) -> Result<()> {
        self.call(Request::Unload { model: model.into() }, &[]).map(|_| ())
    }

    pub fn set(&mut self, model: &str, change: SettingsChange) -> Result<ModelEntry> {
        self.call_typed(Request::Set {
            model: model.into(),
            device: change.device,
            pinned: change.pinned,
            idle_unload_min: change.idle_unload_min,
            preload: change.preload,
            quant: change.quant,
            params: change.params,
        })
    }

    pub fn defaults(&mut self) -> Result<Vec<DefaultModel>> {
        self.call_typed(Request::Defaults)
    }

    pub fn set_default(&mut self, kind: ModelKind, model: Option<&str>) -> Result<Vec<DefaultModel>> {
        self.call_typed(Request::SetDefault { kind, model: model.map(Into::into) })
    }

    pub fn resolve(&mut self, kind: ModelKind) -> Result<ModelEntry> {
        self.call_typed(Request::Resolve { kind })
    }

    pub fn ctc(&mut self, model: &str, pcm: &[f32], sample_rate: u32, hidden: bool) -> Result<CtcOutput> {
        let fd = wire::memfd_f32("pcm", pcm)?;
        let req = Request::Ctc { model: model.into(), pcm: FdRef { fd: 0 }, samples: pcm.len(), sample_rate, hidden };
        let (v, fds) = self.call(req, &[fd.as_fd()])?;
        let r: CtcReply = serde_json::from_value(v).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))?;
        let mut fds: Vec<Option<OwnedFd>> = fds.into_iter().map(Some).collect();
        let mut take = |r: FdRef, n: usize| -> Result<Mapped> {
            let fd = fds
                .get_mut(r.fd as usize)
                .and_then(Option::take)
                .ok_or_else(|| Error::new(ErrorCode::Protocol, "нет файла данных в ответе"))?;
            Ok(Mapped::map(fd, n * 4)?)
        };
        let logprobs = take(r.logprobs, r.frames * r.vocab)?;
        let hidden = match r.hidden {
            Some(h) => Some(take(h, r.frames * r.hidden_dim)?),
            None => None,
        };
        Ok(CtcOutput { frames: r.frames, vocab: r.vocab, frame_sec: r.frame_sec, logprobs, hidden, hidden_dim: r.hidden_dim })
    }

    pub fn mfa_align(
        &mut self,
        model: &str,
        pcm16k: &[f32],
        words: Vec<AlignWordAlts>,
        options: Option<AlignOptions>,
    ) -> Result<Alignment> {
        let fd = wire::memfd_f32("pcm", pcm16k)?;
        let req = Request::MfaAlign { model: model.into(), pcm: FdRef { fd: 0 }, samples: pcm16k.len(), words, options };
        let (v, _) = self.call(req, &[fd.as_fd()])?;
        serde_json::from_value(v).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))
    }

    pub fn transcribe(&mut self, model: &str, pcm: &[f32], sample_rate: u32, language: Option<&str>) -> Result<Transcript> {
        let fd = wire::memfd_f32("pcm", pcm)?;
        let req = Request::Transcribe {
            model: model.into(),
            pcm: FdRef { fd: 0 },
            samples: pcm.len(),
            sample_rate,
            language: language.map(Into::into),
        };
        let (v, _) = self.call(req, &[fd.as_fd()])?;
        serde_json::from_value(v).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))
    }

    pub fn speak_with(&mut self, model: &str, text: &str, params: &Params) -> Result<Speech> {
        self.speak_steps(
            model,
            text,
            params.language.as_deref(),
            params.voice.as_deref(),
            params.speed,
            params.steps.map(|s| s as usize),
        )
    }

    pub fn speak(&mut self, model: &str, text: &str, language: Option<&str>, instruct: Option<&str>, speed: Option<f32>) -> Result<Speech> {
        self.speak_steps(model, text, language, instruct, speed, None)
    }

    pub fn speak_steps(
        &mut self,
        model: &str,
        text: &str,
        language: Option<&str>,
        instruct: Option<&str>,
        speed: Option<f32>,
        steps: Option<usize>,
    ) -> Result<Speech> {
        let req = Request::Speak {
            model: model.into(),
            text: text.into(),
            language: language.map(Into::into),
            instruct: instruct.map(Into::into),
            speed,
            steps,
        };
        let (v, fds) = self.call(req, &[])?;
        let r: SpeechReply = serde_json::from_value(v).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))?;
        let fd = fds
            .into_iter()
            .nth(r.audio.fd as usize)
            .ok_or_else(|| Error::new(ErrorCode::Protocol, "нет файла с аудио в ответе"))?;
        Ok(Speech { sample_rate: r.sample_rate, audio: Mapped::map(fd, r.samples * 4)? })
    }

    pub fn generate(
        &mut self,
        model: &str,
        messages: Vec<Message>,
        max_tokens: usize,
        temperature: Option<f32>,
        on_token: impl FnMut(&str),
    ) -> Result<Generated> {
        let params = Params { max_tokens: Some(max_tokens as u32), temperature, ..Default::default() };
        self.generate_with(model, messages, &params, on_token)
    }

    pub fn generate_with(
        &mut self,
        model: &str,
        messages: Vec<Message>,
        params: &Params,
        mut on_token: impl FnMut(&str),
    ) -> Result<Generated> {
        let req = Request::Generate { model: model.into(), messages, max_tokens: None, temperature: None, params: params.clone() };
        let (v, _) = self.call_events(req, &[], |ev| {
            if let Event::Token { text } = ev {
                on_token(&text);
            }
        })?;
        serde_json::from_value(v).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))
    }

    pub fn subscribe(mut self, mut on_event: impl FnMut(Event) -> bool) -> Result<()> {
        let id = self.send(Request::Subscribe, &[])?;
        loop {
            let (reply, _) = self.recv()?;
            if reply.id != id {
                continue;
            }
            if let Some(ev) = reply.event {
                if !on_event(ev) {
                    return Ok(());
                }
            } else if reply.ok == Some(false) {
                return Err(Error::new(reply.code.unwrap_or(ErrorCode::Failed), reply.error.unwrap_or_default()));
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsChange {
    pub device: Option<DeviceKind>,
    pub pinned: Option<bool>,
    pub idle_unload_min: Option<Option<u32>>,
    pub preload: Option<bool>,
    pub quant: Option<Option<String>>,
    pub params: Option<Params>,
}

pub fn model_id_from_file(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    for ext in [".syn", ".mfa", ".gguf", ".safetensors"] {
        if let Some(s) = base.strip_suffix(ext) {
            return s.to_string();
        }
    }
    base.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_roundtrip() {
        let e = Envelope {
            id: 3,
            req: Request::Set {
                model: "m".into(),
                device: Some(DeviceKind::Npu),
                pinned: None,
                idle_unload_min: Some(None),
                preload: None,
                quant: Some(None),
                params: None,
            },
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"op\":\"set\""));
        assert_eq!(serde_json::from_str::<Envelope>(&s).unwrap(), e);
    }

    #[test]
    fn generate_compat() {
        let old = r#"{"id":1,"op":"generate","model":"m","messages":[],"max_tokens":5,"temperature":0.4}"#;
        let Envelope { req: Request::Generate { max_tokens, temperature, params, .. }, .. } = serde_json::from_str(old).unwrap() else { panic!() };
        assert_eq!((max_tokens, temperature, params.is_empty()), (Some(5), Some(0.4), true));
    }

    #[test]
    fn model_ids() {
        assert_eq!(model_id_from_file("w2v2-lv60-espeak.syn"), "w2v2-lv60-espeak");
        assert_eq!(model_id_from_file("/a/mfa-kk.mfa"), "mfa-kk");
    }
}
