pub mod flow;
pub mod params;
pub mod wire;

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use params::{ParamKind, ParamSpec, ParamValue, Params};
pub use wire::Mapped;

pub const SOCKET: &str = "/run/synaptix/synaptixd.sock";
pub const VERSION: u32 = 1;

static LABEL: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

thread_local! {
    static THREAD_LABEL: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Роль программы для всех её соединений со службой (показывается рядом с
/// именем программы в метриках): «нодовый редактор», «чат»…
pub fn set_label(label: Option<&str>) {
    *LABEL.lock().unwrap_or_else(|e| e.into_inner()) = label.map(String::from);
}

/// Роль для соединений этого потока на время `f` (субагент в своём потоке).
pub fn with_label<R>(label: &str, f: impl FnOnce() -> R) -> R {
    let prev = THREAD_LABEL.with(|l| l.replace(Some(label.to_string())));
    let r = f();
    THREAD_LABEL.with(|l| *l.borrow_mut() = prev);
    r
}

fn current_label() -> Option<String> {
    THREAD_LABEL.with(|l| l.borrow().clone()).or_else(|| LABEL.lock().unwrap_or_else(|e| e.into_inner()).clone())
}

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
    /// Кто когда говорит (Sortformer).
    Diarize,
    /// Переранжирование найденного (кросс-энкодер).
    Rerank,
}

impl ModelKind {
    pub const ALL: [ModelKind; 11] = [
        ModelKind::Llm,
        ModelKind::Image,
        ModelKind::Video,
        ModelKind::Music,
        ModelKind::Tts,
        ModelKind::Asr,
        ModelKind::Embed,
        ModelKind::Ctc,
        ModelKind::Mfa,
        ModelKind::Diarize,
        ModelKind::Rerank,
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
            ModelKind::Diarize => "diarize",
            ModelKind::Rerank => "rerank",
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
    /// Сколько минут держать в RAM KV-кэш чатов выгруженной языковой модели:
    /// загрузится снова — диалоги продолжатся без префилла истории. `None` —
    /// как в конфиге службы, 0 — не держать.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kv_keep_min: Option<u32>,
    #[serde(default)]
    pub preload: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quant: Option<String>,
    #[serde(default, skip_serializing_if = "Params::is_empty")]
    pub params: Params,
    /// Профиль движка языковой модели (ручные правки поверх выверенного).
    #[serde(default, skip_serializing_if = "EngineProfile::is_default")]
    pub engine: EngineProfile,
    /// Адаптеры LoRA языковой модели по умолчанию (запрос может задать свой
    /// набор). Накладываются при генерации поверх весов, без слияния.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loras: Vec<LoraUse>,
}

fn one() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}

/// Адаптер LoRA в наборе модели: имя в библиотеке, включён ли, сила.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct LoraUse {
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Множитель масштаба адаптера (1 — как учился).
    #[serde(default = "one")]
    pub strength: f32,
}

impl LoraUse {
    pub fn on(name: impl Into<String>) -> Self {
        Self { name: name.into(), enabled: true, strength: 1.0 }
    }
}

/// Адаптер LoRA в библиотеке (`<библиотека>/lora/<имя>/`).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct LoraEntry {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub library: Option<String>,
    /// Модель, на которой учился (id в службе, если нашлась), и её файл.
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub base_path: String,
    pub rank: usize,
    pub alpha: f32,
    #[serde(default)]
    pub targets: Vec<String>,
    pub bytes: u64,
    #[serde(default)]
    pub modified_ms: i64,
    /// Сведения обучения (`synaptix_train.json`): датасет, шаги, лосс…
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub train: Value,
}

/// Датасет в библиотеке (`<библиотека>/datasets/*.jsonl|json`).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct DatasetEntry {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub library: Option<String>,
    pub bytes: u64,
    /// Строк JSONL (примеров), у JSON-массива — его длина.
    pub rows: usize,
    #[serde(default)]
    pub modified_ms: i64,
}

/// Проверка датасета токенизатором модели.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct DatasetCheck {
    pub samples: usize,
    pub skipped: usize,
    pub truncated: usize,
    pub tokens: usize,
    pub target_tokens: usize,
    pub last_turn_only: usize,
}

/// Параметры обучения адаптера (пустое — по умолчанию движка).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct TrainOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lora_r: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lora_alpha: Option<f32>,
    /// `all`, `attn` или список проекций.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targets: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lr: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epochs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_len: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grad_accum: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warmup: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_fraction: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_every: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_every: Option<usize>,
    /// Формат базы: `nvfp4` (по умолчанию), `mxfp8`, `sqN`, `bf16`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quant: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub thinking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_samples: Option<usize>,
    /// Продолжить с адаптера библиотеки (то же имя — дообучить его).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrainState {
    /// Ждёт своей очереди (обучение идёт по одному).
    #[default]
    Queued,
    /// Разметка датасета и загрузка базы.
    Preparing,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl TrainState {
    pub fn finished(self) -> bool {
        matches!(self, TrainState::Done | TrainState::Failed | TrainState::Cancelled)
    }
}

/// Точка кривой обучения.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug, Default)]
pub struct TrainPoint {
    pub step: usize,
    pub loss: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_loss: Option<f32>,
}

/// Задача обучения адаптера в службе.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct TrainJob {
    pub id: u64,
    pub model: String,
    #[serde(default)]
    pub model_title: String,
    pub dataset: String,
    /// Имя адаптера и куда он пишется.
    pub name: String,
    pub out: String,
    pub options: TrainOptions,
    pub state: TrainState,
    pub created_ms: i64,
    #[serde(default)]
    pub started_ms: Option<i64>,
    #[serde(default)]
    pub finished_ms: Option<i64>,
    pub step: usize,
    pub total_steps: usize,
    pub epoch: usize,
    pub loss: f32,
    #[serde(default)]
    pub eval_loss: Option<f32>,
    pub lr: f32,
    pub grad_norm: f32,
    pub tokens_per_sec: f32,
    pub eta_secs: f32,
    pub elapsed_secs: f32,
    #[serde(default)]
    pub history: Vec<TrainPoint>,
    #[serde(default)]
    pub dataset_check: Option<DatasetCheck>,
    #[serde(default)]
    pub train_samples: usize,
    #[serde(default)]
    pub eval_samples: usize,
    #[serde(default)]
    pub trainable_params: u64,
    #[serde(default)]
    pub error: Option<String>,
    /// Кто запустил.
    #[serde(default)]
    pub app: AppInfo,
}

/// Профиль движка языковой модели. В настройках — только ручные правки
/// (`custom`; пустое поле — как выверено движком), в описании модели
/// (`ModelEntry::engine`) — действующие значения целиком.
///
/// Типы — имена synaptix: веса `nvfp4|mxfp8|bf16|f16|f32|sqN`, счёт
/// `bf16|f16|f32`, KV `auto|f16|bf16|f32|fp8e4m3|mxfp8`, синхронизация слоёв
/// `auto|on|off`.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
pub struct EngineProfile {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub custom: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weights: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compute: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kv_dtype: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lm_head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embed: Option<String>,
    /// CUDA-граф на декоде.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_decode: Option<bool>,
    /// Спекулятивный декод (MTP, DFlash).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speculation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer_sync: Option<String>,
    /// Перекодировать уже квантованные веса в форматы профиля (двойной квант).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcode: Option<bool>,
    /// Квант-форматы весов в самом файле (GGUF, квантованный `.syn`), самый
    /// частый первым; пусто у плотных. Только в действующем профиле: без
    /// перекодировки такие веса остаются как в файле.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source: Vec<String>,
}

impl EngineProfile {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Правки, которые действуют (вне `custom` — никакие).
    pub fn effective(&self) -> Self {
        if self.custom {
            self.clone()
        } else {
            Self::default()
        }
    }

    /// Отличаются ли поля, нужные при загрузке весов (их смена перезагружает
    /// модель; остальное применяется на лету).
    pub fn reload_differs(&self, other: &Self) -> bool {
        let (a, b) = (self.effective(), other.effective());
        (&a.weights, &a.compute, &a.kv_dtype, &a.lm_head, &a.embed, a.transcode, a.speculation)
            != (&b.weights, &b.compute, &b.kv_dtype, &b.lm_head, &b.embed, b.transcode, b.speculation)
    }
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
    pub schema: Vec<ParamSpec>,
    #[serde(default)]
    pub defaults: Params,
    #[serde(default)]
    pub runnable: bool,
    #[serde(default)]
    pub library: Option<String>,
    #[serde(default)]
    pub components: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
    /// Действующий профиль движка (у языковых моделей).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<EngineProfile>,
    /// Основной файл весов (для устройства, на котором модель работает или
    /// будет загружена).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct LibraryInfo {
    pub path: String,
    pub name: String,
    pub online: bool,
    pub default: bool,
    pub total: u64,
    pub free: u64,
    pub models: u32,
    pub used: u64,
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

/// Программа, которая обратилась к службе: процесс и его ярлык `.desktop`.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default, Hash)]
pub struct AppInfo {
    pub pid: u32,
    pub uid: u32,
    /// Имя программы (из `.desktop`, иначе имя процесса).
    pub name: String,
    /// Значок из `.desktop` (имя темы или путь).
    #[serde(default)]
    pub icon: String,
    /// Ярлык `.desktop` без расширения или имя исполняемого файла.
    pub id: String,
    /// Роль внутри программы из `Hello` («субагент», «нодовый редактор»…).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

impl AppInfo {
    /// «Synthos · субагент».
    pub fn title(&self) -> String {
        if self.label.is_empty() || self.label.eq_ignore_ascii_case(&self.name) || self.label.eq_ignore_ascii_case(&self.id) {
            self.name.clone()
        } else {
            format!("{} · {}", self.name, self.label)
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActivityState {
    /// Ждёт загрузки модели или освобождения устройства.
    #[default]
    Waiting,
    Running,
    Done,
    Failed,
    Cancelled,
}

/// Одна работа службы: запрос программы (генерация, картинка, граф…).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct Activity {
    pub id: u64,
    /// Операция запроса: `generate`, `image`, `video`, `music`, `speak`,
    /// `transcribe`, `embed`, `rerank`, `flow`…
    pub op: String,
    pub app: AppInfo,
    /// Модели, которые работа заняла (у графа их может быть несколько).
    pub models: Vec<String>,
    pub state: ActivityState,
    pub started_ms: i64,
    #[serde(default)]
    pub finished_ms: Option<i64>,
    /// Шаг генерации (картинка, видео, музыка, нода графа).
    #[serde(default)]
    pub step: Option<(usize, usize)>,
    /// Языковая модель: промпт, из кэша, ответ, контекст.
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub cached_tokens: u32,
    #[serde(default)]
    pub tokens: u32,
    #[serde(default)]
    pub context: u32,
    #[serde(default)]
    pub prefill_ms: u64,
    #[serde(default)]
    pub decode_ms: u64,
    /// Скорость ответа, токенов в секунду (идёт — текущая, готово — итоговая).
    #[serde(default)]
    pub tps: f32,
    #[serde(default)]
    pub error: Option<String>,
}

/// Точка истории: раз в секунду.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct MetricsSample {
    pub t_ms: i64,
    /// Токенов в секунду всех языковых моделей.
    pub tps: f32,
    /// Работ идёт / ждёт.
    pub running: u32,
    pub waiting: u32,
    /// Память моделей службы: RAM и видеопамять, байт.
    pub ram: u64,
    pub vram: u64,
}

/// KV-сессия языковой модели (контекст чата программы).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct KvUsage {
    pub model: String,
    /// Ключ сессии (чат программы).
    pub key: String,
    pub app: AppInfo,
    pub tokens: u32,
    pub capacity: u32,
    /// Байт на устройстве (припаркованная — в RAM).
    pub bytes: u64,
    pub parked: bool,
}

/// Программа и что она держит в службе.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct AppUsage {
    pub app: AppInfo,
    /// Открытых соединений (подписки, открытые модели).
    pub connections: u32,
    /// Модели, открытые программой или занятые её работами.
    pub models: Vec<String>,
    /// Байт KV-сессий программы.
    pub kv_bytes: u64,
    pub requests: u64,
    pub tokens: u64,
    pub last_ms: i64,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct Metrics {
    pub now_ms: i64,
    /// Идут и ждут.
    pub running: Vec<Activity>,
    /// Последние завершённые, новые первыми.
    pub recent: Vec<Activity>,
    pub apps: Vec<AppUsage>,
    pub kv: Vec<KvUsage>,
    /// История за 5 минут (по запросу `history`).
    #[serde(default)]
    pub history: Vec<MetricsSample>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Картинки и видео к сообщению: модель со зрением их видит (служба
    /// кодирует их башней зрения и ставит блок перед текстом), без зрения —
    /// получает строку с причиной.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<Media>,
}

/// Маркер места вложения в тексте сообщения: служба ставит блок картинки
/// (или строку-замену) ровно сюда, по порядку `media`. Без маркеров блоки
/// идут перед текстом.
pub const MEDIA_MARK: char = '\u{FFFC}';

/// Медиа-вложение сообщения: файл на диске (служба читает его сама).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Media {
    pub kind: MediaKind,
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Video,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct ToolCall {
    pub name: String,
    pub arguments: String,
}

impl Message {
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Self { role: role.into(), content: content.into(), name: None, reasoning: None, tool_calls: Vec::new(), media: Vec::new() }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new("system", content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new("user", content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new("assistant", content)
    }

    pub fn tool(name: &str, content: impl Into<String>) -> Self {
        Self { name: Some(name.into()), ..Self::new("tool", content) }
    }
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
        #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "double_option")]
        kv_keep_min: Option<Option<u32>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preload: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "double_option")]
        quant: Option<Option<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        params: Option<Params>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        engine: Option<EngineProfile>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        loras: Option<Vec<LoraUse>>,
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
    Libraries,
    AddLibrary {
        path: String,
        #[serde(default)]
        name: Option<String>,
    },
    RemoveLibrary {
        path: String,
    },
    SetDefaultLibrary {
        path: String,
    },
    DeleteModel {
        model: String,
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
    /// Картинка по описанию (и по референсам/исходной картинке); прогресс —
    /// событиями `progress`, PNG — fd 0 ответа.
    Image {
        model: String,
        prompt: String,
        #[serde(default)]
        negative: Option<String>,
        #[serde(default)]
        width: Option<usize>,
        #[serde(default)]
        height: Option<usize>,
        #[serde(default)]
        steps: Option<usize>,
        #[serde(default)]
        guidance: Option<f32>,
        #[serde(default)]
        seed: Option<u64>,
        /// Референсы (пути; служба читает сама).
        #[serde(default)]
        images: Vec<String>,
        /// Исходная картинка для img2img.
        #[serde(default)]
        init: Option<String>,
        #[serde(default)]
        strength: Option<f32>,
    },
    /// Видео (со звуком, где модель умеет); mp4 — fd 0 ответа.
    Video {
        model: String,
        prompt: String,
        #[serde(default)]
        negative: Option<String>,
        #[serde(default)]
        width: Option<usize>,
        #[serde(default)]
        height: Option<usize>,
        #[serde(default)]
        seconds: Option<f64>,
        #[serde(default)]
        steps: Option<usize>,
        #[serde(default)]
        guidance: Option<f32>,
        #[serde(default)]
        seed: Option<u64>,
        #[serde(default)]
        first_frame: Option<String>,
        #[serde(default)]
        last_frame: Option<String>,
        /// Референсы: картинки, видео, звук (до 12).
        #[serde(default)]
        refs: Vec<String>,
        /// LTX: звук (липсинк; с control_video — LipDub).
        #[serde(default)]
        audio: Option<String>,
        /// LTX: исходное видео — retake отрезка или продление.
        #[serde(default)]
        source_video: Option<String>,
        #[serde(default)]
        retake: Option<(f64, f64)>,
        #[serde(default)]
        extend: bool,
        /// LTX IC-LoRA: видео-образец; control — canny | motion.
        #[serde(default)]
        control_video: Option<String>,
        #[serde(default)]
        control: Option<String>,
        /// LTX: две стадии (апскейлер ×2).
        #[serde(default)]
        two_stage: bool,
        /// H3 restyle: сила перерисовки source_video (0..1).
        #[serde(default)]
        strength: Option<f32>,
    },
    /// Музыка или песня; WAV — fd 0 ответа.
    Music {
        model: String,
        prompt: String,
        #[serde(default)]
        lyrics: String,
        #[serde(default)]
        seconds: Option<u32>,
        #[serde(default)]
        seed: Option<u64>,
        #[serde(default)]
        steps: Option<usize>,
    },
    /// Эмбеддинги текстов (модель вида embed, например bge-m3).
    Embed {
        model: String,
        texts: Vec<String>,
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
        #[serde(default, skip_serializing_if = "Params::is_empty")]
        params: Params,
        /// Не брать и не запоминать голос прошлых озвучек: голос задают
        /// только параметры и зерно (проверка настроек модели).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        fresh: bool,
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
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tools: Vec<Value>,
        /// Свой набор адаптеров LoRA на этот запрос (`None` — из настроек
        /// модели; пустой — без адаптеров).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        loras: Option<Vec<LoraUse>>,
    },
    /// Оценки пар (запрос, документ) кросс-энкодером.
    Rerank {
        model: String,
        pairs: Vec<(String, String)>,
    },
    /// Число токенов текстов по токенизатору языковой модели (бюджеты
    /// контекста у клиента).
    Tokenize {
        model: String,
        texts: Vec<String>,
    },
    /// Сведения о языковой модели: окно, зрение, пресеты сэмплинга и глубины
    /// размышлений. Модель загружается, если ещё не в памяти.
    LlmInfo {
        model: String,
    },
    /// Граф ядра synaptix-flow целиком в службе (генерация, стадии моделей,
    /// монтаж). Входные файлы — fd запроса: в полях и входах они записаны
    /// как `@in:<номер>`, имена — в `files`. Ход и выходы нод — события
    /// `flow`, файлы выходов — fd событий `flow_files`.
    Flow {
        flow: Value,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        inputs: Vec<FlowInput>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        files: Vec<String>,
    },
    Subscribe,
    /// Что служба делает сейчас: работы, программы, история скорости.
    Metrics {
        #[serde(default)]
        history: bool,
    },
    /// Упаковать модель в `.syn` с квантованием на устройстве службы.
    /// `plan` — `synaptix_bundle::PackPlan`, `options` —
    /// `synaptix::facade::pack::PackOptions` (JSON). Файлы читаются и
    /// пишутся с правами программы-клиента.
    Pack {
        plan: Value,
        out: String,
        #[serde(default)]
        options: Value,
    },
    /// Остановить работу (свою — любой, чужую — root и wheel).
    Cancel {
        activity: u64,
    },
    /// Адаптеры LoRA в библиотеках.
    Loras,
    /// Удалить адаптер из библиотеки (с правами программы-клиента).
    DeleteLora {
        name: String,
    },
    /// Датасеты в библиотеках.
    Datasets,
    /// Разметить датасет токенизатором модели и посчитать примеры и токены.
    DatasetCheck {
        model: String,
        path: String,
        #[serde(default)]
        options: TrainOptions,
    },
    /// Поставить обучение адаптера в очередь. Файлы читаются и пишутся с
    /// правами программы-клиента; задача живёт в службе и после отключения
    /// программы, ход — событиями `train` у подписчиков.
    TrainStart {
        model: String,
        dataset: String,
        name: String,
        #[serde(default)]
        options: TrainOptions,
    },
    /// Задачи обучения: идущие, в очереди и завершённые.
    TrainJobs,
    /// Остановить задачу (адаптер на этом шаге сохраняется); в очереди —
    /// убрать. Свою — любой, чужую — root и wheel.
    TrainStop {
        job: u64,
    },
    /// Убрать завершённую задачу из истории.
    TrainForget {
        job: u64,
    },
}

/// Готовое значение на входе ноды графа (с ноды вне графа).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct FlowInput {
    pub node: u64,
    pub port: String,
    pub value: synaptix_flow::Val,
}

/// Итог прогона графа в службе (выходы нод пришли событиями).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct FlowReply {
    pub steps: Vec<String>,
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
    Thinking { text: String },
    ToolText { text: String },
    ModelState { model: ModelEntry },
    /// Ход генерации: шаг из всего.
    Progress { step: usize, total: usize },
    ModelsChanged,
    /// Событие прогона графа; файлы в выходах — `@out:<номер>`.
    Flow { node: synaptix_flow::Event },
    /// Файлы выходов графа: fd сообщения — `@out:<first>`, `@out:<first+1>`, …
    FlowFiles { first: usize, names: Vec<String> },
    /// Ход упаковки: событие `synaptix_bundle::ProgressEvent` в JSON.
    Pack { progress: Value },
    /// Задача обучения изменилась (шаг, состояние).
    Train { job: TrainJob },
    /// Изменился список адаптеров или датасетов в библиотеках.
    LorasChanged,
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
    /// Зерно, с которым синтезировано (у движков со случайным зерном).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

pub struct Speech {
    pub sample_rate: u32,
    pub audio: Mapped,
    pub seed: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Segment {
    pub start: f32,
    pub end: f32,
    pub text: String,
}

/// Готовая картинка: PNG в fd ответа.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct ImageReply {
    pub width: usize,
    pub height: usize,
    pub seed: u64,
    pub label: String,
    pub png: FdRef,
    pub bytes: usize,
}

/// Готовое видео: mp4 в fd ответа.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct VideoReply {
    pub width: usize,
    pub height: usize,
    pub frames: usize,
    pub fps: f64,
    pub seed: u64,
    pub label: String,
    pub mp4: FdRef,
    pub bytes: usize,
}

/// Параметры видео для [`Client::video`].
#[derive(Clone, Debug, Default)]
pub struct VideoParams {
    pub negative: Option<String>,
    pub width: Option<usize>,
    pub height: Option<usize>,
    pub seconds: Option<f64>,
    pub steps: Option<usize>,
    pub guidance: Option<f32>,
    pub seed: Option<u64>,
    pub first_frame: Option<String>,
    pub last_frame: Option<String>,
    pub refs: Vec<String>,
    pub audio: Option<String>,
    pub source_video: Option<String>,
    pub retake: Option<(f64, f64)>,
    pub extend: bool,
    pub control_video: Option<String>,
    pub control: Option<String>,
    pub two_stage: bool,
    pub strength: Option<f32>,
}

/// Готовая музыка: WAV в fd ответа.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct MusicReply {
    pub seconds: f32,
    pub sample_rate: u32,
    pub channels: u16,
    pub seed: u64,
    pub label: String,
    pub wav: FdRef,
    pub bytes: usize,
}

/// Параметры картинки для [`Client::image`].
#[derive(Clone, Debug, Default)]
pub struct ImageParams {
    pub negative: Option<String>,
    pub width: Option<usize>,
    pub height: Option<usize>,
    pub steps: Option<usize>,
    pub guidance: Option<f32>,
    pub seed: Option<u64>,
    pub images: Vec<String>,
    pub init: Option<String>,
    pub strength: Option<f32>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct Embeddings {
    pub dim: usize,
    pub vectors: Vec<Vec<f32>>,
    /// Предел длины текста модели, токенов (0 — не сообщён).
    #[serde(default)]
    pub max_tokens: usize,
}

/// Ответ `Rerank`: оценки пар (запрос, документ) и предел длины пары.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct RerankScores {
    pub scores: Vec<f32>,
    #[serde(default)]
    pub max_tokens: usize,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Transcript {
    pub text: String,
    #[serde(default)]
    pub segments: Vec<Segment>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct Generated {
    pub text: String,
    pub tokens: usize,
    #[serde(default)]
    pub thinking: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub prompt_tokens: usize,
    #[serde(default)]
    pub context: usize,
    /// Токенов промпта, взятых из KV-кэша сессии.
    #[serde(default)]
    pub cached_tokens: usize,
    /// До первого токена ответа (префилл), мс.
    #[serde(default)]
    pub prefill_ms: u64,
    /// Декод ответа, мс.
    #[serde(default)]
    pub decode_ms: u64,
}

/// Ответ `Tokenize`: по числу на текст.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct TokenCounts {
    pub counts: Vec<usize>,
}

/// Пресет сэмплинга модели (`thinking`: для режима с размышлениями, без них
/// или для обоих).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SamplingPreset {
    pub id: String,
    #[serde(default)]
    pub thinking: Option<bool>,
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: usize,
    pub min_p: f32,
    pub presence_penalty: f32,
    pub repetition_penalty: f32,
}

/// Ответ `LlmInfo`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct LlmInfo {
    /// Окно контекста, токенов.
    pub window: usize,
    /// Принимает картинки и видео.
    pub media: bool,
    pub presets: Vec<SamplingPreset>,
    /// Уровни глубины размышлений (пусто — только вкл./выкл.) и умолчание.
    #[serde(default)]
    pub reasoning_levels: Vec<String>,
    #[serde(default)]
    pub reasoning_default: Option<String>,
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

/// Отмена запроса клиента (см. [`Client::canceller`]).
pub struct Canceller(OwnedFd);

impl Canceller {
    pub fn cancel(&self) {
        use std::os::fd::AsRawFd;
        unsafe {
            libc::shutdown(self.0.as_raw_fd(), libc::SHUT_RDWR);
        }
    }
}

impl Client {
    pub fn connect() -> Result<Self> {
        Self::connect_to(&socket_path())
    }

    pub fn connect_to(path: &std::path::Path) -> Result<Self> {
        let sock = wire::connect(path)?;
        let mut c = Self { sock, next_id: 1 };
        if let Some(label) = current_label() {
            c.hello(&label)?;
        }
        Ok(c)
    }

    /// Отмена из другого потока: закрывает соединение, текущий запрос
    /// получает ошибку, служба видит обрыв и прерывает генерацию.
    pub fn canceller(&self) -> Option<Canceller> {
        self.sock.try_clone().ok().map(Canceller)
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

    /// Работы, программы и KV-сессии службы; `history` — и история за 5 минут.
    pub fn metrics(&mut self, history: bool) -> Result<Metrics> {
        self.call_typed(Request::Metrics { history })
    }

    /// Упаковка модели службой (см. [`Request::Pack`]); `on_event` — ход.
    pub fn pack(&mut self, plan: Value, out: &str, options: Value, mut on_event: impl FnMut(Value)) -> Result<()> {
        let req = Request::Pack { plan, out: out.into(), options };
        self.call_events(req, &[], |ev| {
            if let Event::Pack { progress } = ev {
                on_event(progress);
            }
        })
        .map(|_| ())
    }

    pub fn cancel_activity(&mut self, activity: u64) -> Result<()> {
        self.call(Request::Cancel { activity }, &[]).map(|_| ())
    }

    /// Роль соединения внутри программы («субагент», «нодовый редактор»):
    /// служба показывает её рядом с именем программы.
    pub fn label(&mut self, label: &str) -> Result<Hello> {
        self.hello(label)
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
            kv_keep_min: change.kv_keep_min,
            preload: change.preload,
            quant: change.quant,
            params: change.params,
            engine: change.engine,
            loras: change.loras,
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

    pub fn libraries(&mut self) -> Result<Vec<LibraryInfo>> {
        self.call_typed(Request::Libraries)
    }

    pub fn delete_model(&mut self, model: &str) -> Result<()> {
        self.call(Request::DeleteModel { model: model.into() }, &[]).map(|_| ())
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

    /// Картинка: PNG-байты и описание; `on_progress(шаг, всего)`.
    pub fn image(&mut self, model: &str, prompt: &str, p: &ImageParams, mut on_progress: impl FnMut(usize, usize)) -> Result<(ImageReply, Vec<u8>)> {
        let req = Request::Image {
            model: model.into(),
            prompt: prompt.into(),
            negative: p.negative.clone(),
            width: p.width,
            height: p.height,
            steps: p.steps,
            guidance: p.guidance,
            seed: p.seed,
            images: p.images.clone(),
            init: p.init.clone(),
            strength: p.strength,
        };
        let (v, fds) = self.call_events(req, &[], |ev| {
            if let Event::Progress { step, total } = ev {
                on_progress(step, total);
            }
        })?;
        let reply: ImageReply = serde_json::from_value(v).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))?;
        let fd = fds.into_iter().next().ok_or_else(|| Error::new(ErrorCode::Protocol, "нет fd с картинкой"))?;
        let mut png = Vec::with_capacity(reply.bytes);
        use std::io::Read;
        std::fs::File::from(fd).read_to_end(&mut png).map_err(|e| Error::new(ErrorCode::Failed, e.to_string()))?;
        Ok((reply, png))
    }

    /// Видео: mp4-байты и описание; `on_progress(шаг, всего)`.
    pub fn video(&mut self, model: &str, prompt: &str, p: &VideoParams, mut on_progress: impl FnMut(usize, usize)) -> Result<(VideoReply, Vec<u8>)> {
        let req = Request::Video {
            model: model.into(),
            prompt: prompt.into(),
            negative: p.negative.clone(),
            width: p.width,
            height: p.height,
            seconds: p.seconds,
            steps: p.steps,
            guidance: p.guidance,
            seed: p.seed,
            first_frame: p.first_frame.clone(),
            last_frame: p.last_frame.clone(),
            refs: p.refs.clone(),
            audio: p.audio.clone(),
            source_video: p.source_video.clone(),
            retake: p.retake,
            extend: p.extend,
            control_video: p.control_video.clone(),
            control: p.control.clone(),
            two_stage: p.two_stage,
            strength: p.strength,
        };
        let (v, fds) = self.call_events(req, &[], |ev| {
            if let Event::Progress { step, total } = ev {
                on_progress(step, total);
            }
        })?;
        let reply: VideoReply = serde_json::from_value(v).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))?;
        let fd = fds.into_iter().next().ok_or_else(|| Error::new(ErrorCode::Protocol, "нет fd с видео"))?;
        let mut mp4 = Vec::with_capacity(reply.bytes);
        use std::io::Read;
        std::fs::File::from(fd).read_to_end(&mut mp4).map_err(|e| Error::new(ErrorCode::Failed, e.to_string()))?;
        Ok((reply, mp4))
    }

    /// Музыка: WAV-байты и описание; `on_progress(шаг, всего)` (где модель его даёт).
    pub fn music(&mut self, model: &str, prompt: &str, lyrics: &str, seconds: Option<u32>, seed: Option<u64>, mut on_progress: impl FnMut(usize, usize)) -> Result<(MusicReply, Vec<u8>)> {
        let req = Request::Music { model: model.into(), prompt: prompt.into(), lyrics: lyrics.into(), seconds, seed, steps: None };
        let (v, fds) = self.call_events(req, &[], |ev| {
            if let Event::Progress { step, total } = ev {
                on_progress(step, total);
            }
        })?;
        let reply: MusicReply = serde_json::from_value(v).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))?;
        let fd = fds.into_iter().next().ok_or_else(|| Error::new(ErrorCode::Protocol, "нет fd с музыкой"))?;
        let mut wav = Vec::with_capacity(reply.bytes);
        use std::io::Read;
        std::fs::File::from(fd).read_to_end(&mut wav).map_err(|e| Error::new(ErrorCode::Failed, e.to_string()))?;
        Ok((reply, wav))
    }

    /// Эмбеддинги текстов; пустая модель — по умолчанию для embed. Большие
    /// списки — частями (предел сообщения 4 МБ).
    pub fn embed(&mut self, model: &str, texts: &[String]) -> Result<Embeddings> {
        let mut out = Embeddings::default();
        for chunk in texts.chunks(32) {
            let (v, _) = self.call(Request::Embed { model: model.into(), texts: chunk.to_vec() }, &[])?;
            let part: Embeddings = serde_json::from_value(v).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))?;
            out.dim = part.dim;
            out.max_tokens = part.max_tokens;
            out.vectors.extend(part.vectors);
        }
        Ok(out)
    }

    pub fn speak_with(&mut self, model: &str, text: &str, params: &Params) -> Result<Speech> {
        let req = Request::Speak { model: model.into(), text: text.into(), language: None, instruct: None, speed: None, steps: None, params: params.clone(), fresh: false };
        self.speech(req)
    }

    /// Озвучка для проверки настроек: голос не продолжает прошлые озвучки,
    /// в ответе — зерно, которым его можно закрепить.
    pub fn speak_fresh(&mut self, model: &str, text: &str, params: &Params) -> Result<Speech> {
        let req = Request::Speak { model: model.into(), text: text.into(), language: None, instruct: None, speed: None, steps: None, params: params.clone(), fresh: true };
        self.speech(req)
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
            params: Params::default(),
            fresh: false,
        };
        self.speech(req)
    }

    fn speech(&mut self, req: Request) -> Result<Speech> {
        let (v, fds) = self.call(req, &[])?;
        let r: SpeechReply = serde_json::from_value(v).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))?;
        let fd = fds
            .into_iter()
            .nth(r.audio.fd as usize)
            .ok_or_else(|| Error::new(ErrorCode::Protocol, "нет файла с аудио в ответе"))?;
        Ok(Speech { sample_rate: r.sample_rate, audio: Mapped::map(fd, r.samples * 4)?, seed: r.seed })
    }

    pub fn generate(
        &mut self,
        model: &str,
        messages: Vec<Message>,
        max_tokens: usize,
        temperature: Option<f32>,
        on_token: impl FnMut(&str),
    ) -> Result<Generated> {
        let mut params = Params::new().with("max_tokens", max_tokens as i64);
        if let Some(t) = temperature {
            params = params.with("temperature", t);
        }
        self.generate_with(model, messages, &params, on_token)
    }

    pub fn generate_with(
        &mut self,
        model: &str,
        messages: Vec<Message>,
        params: &Params,
        mut on_token: impl FnMut(&str),
    ) -> Result<Generated> {
        self.chat(model, messages, Vec::new(), params, |ev| {
            if let Event::Token { text } = ev {
                on_token(&text);
            }
        })
    }

    pub fn chat(&mut self, model: &str, messages: Vec<Message>, tools: Vec<Value>, params: &Params, on_event: impl FnMut(Event)) -> Result<Generated> {
        self.chat_loras(model, messages, tools, params, None, on_event)
    }

    /// Как [`Self::chat`], но со своим набором адаптеров LoRA на этот запрос
    /// (`None` — из настроек модели).
    pub fn chat_loras(
        &mut self,
        model: &str,
        messages: Vec<Message>,
        tools: Vec<Value>,
        params: &Params,
        loras: Option<Vec<LoraUse>>,
        on_event: impl FnMut(Event),
    ) -> Result<Generated> {
        let req = Request::Generate { model: model.into(), messages, max_tokens: None, temperature: None, params: params.clone(), tools, loras };
        let (v, _) = self.call_events(req, &[], on_event)?;
        serde_json::from_value(v).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))
    }

    pub fn loras(&mut self) -> Result<Vec<LoraEntry>> {
        self.call_typed(Request::Loras)
    }

    pub fn delete_lora(&mut self, name: &str) -> Result<()> {
        self.call(Request::DeleteLora { name: name.into() }, &[]).map(|_| ())
    }

    pub fn datasets(&mut self) -> Result<Vec<DatasetEntry>> {
        self.call_typed(Request::Datasets)
    }

    pub fn dataset_check(&mut self, model: &str, path: &str, options: TrainOptions) -> Result<DatasetCheck> {
        self.call_typed(Request::DatasetCheck { model: model.into(), path: path.into(), options })
    }

    pub fn train_start(&mut self, model: &str, dataset: &str, name: &str, options: TrainOptions) -> Result<TrainJob> {
        self.call_typed(Request::TrainStart { model: model.into(), dataset: dataset.into(), name: name.into(), options })
    }

    pub fn train_jobs(&mut self) -> Result<Vec<TrainJob>> {
        self.call_typed(Request::TrainJobs)
    }

    pub fn train_stop(&mut self, job: u64) -> Result<()> {
        self.call(Request::TrainStop { job }, &[]).map(|_| ())
    }

    pub fn train_forget(&mut self, job: u64) -> Result<()> {
        self.call(Request::TrainForget { job }, &[]).map(|_| ())
    }

    /// Оценки пар (запрос, документ); пачками по 32.
    pub fn rerank_pairs(&mut self, model: &str, pairs: &[(String, String)]) -> Result<RerankScores> {
        let mut out = RerankScores::default();
        for chunk in pairs.chunks(32) {
            let (v, _) = self.call(Request::Rerank { model: model.into(), pairs: chunk.to_vec() }, &[])?;
            let part: RerankScores = serde_json::from_value(v).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))?;
            out.max_tokens = part.max_tokens;
            out.scores.extend(part.scores);
        }
        Ok(out)
    }

    /// Число токенов текстов по токенизатору модели.
    pub fn count_tokens(&mut self, model: &str, texts: &[String]) -> Result<Vec<usize>> {
        let (v, _) = self.call(Request::Tokenize { model: model.into(), texts: texts.to_vec() }, &[])?;
        let r: TokenCounts = serde_json::from_value(v).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))?;
        Ok(r.counts)
    }

    pub fn llm_info(&mut self, model: &str) -> Result<LlmInfo> {
        let (v, _) = self.call(Request::LlmInfo { model: model.into() }, &[])?;
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
    pub kv_keep_min: Option<Option<u32>>,
    pub preload: Option<bool>,
    pub quant: Option<Option<String>>,
    pub params: Option<Params>,
    pub engine: Option<EngineProfile>,
    pub loras: Option<Vec<LoraUse>>,
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
                kv_keep_min: Some(Some(30)),
                preload: None,
                quant: Some(None),
                params: None,
                engine: Some(EngineProfile { custom: true, graph_decode: Some(false), kv_dtype: Some("bf16".into()), ..Default::default() }),
                loras: Some(vec![LoraUse::on("стиль")]),
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
