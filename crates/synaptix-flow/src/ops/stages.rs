//! Стадии моделей — ноды, которые исполняет только служба (исполнителей
//! подставляет она, см. [`super::provide`]). Между стадиями идут
//! дескрипторы: модель, условие (закодированное описание), латент.
//! Результат стадии кэшируется службой: повторный прогон с теми же
//! входами и полями берёт его из кэша (сменили сид — заново только сэмплер).
//!
//! Картинки — одни ноды на все семейства (FLUX.1, FLUX.2, Qwen-Image,
//! Qwen-Image 2.1, SDXL; семейство — по модели):
//! модель → описание (+ референсы) → пустой латент или картинка в латент →
//! сэмплер → декодер. Видео MiniMax-H3: модель → латент (размер и
//! длительность) → описание с ключевыми кадрами и референсами → сэмплер →
//! декодеры видео и звука. LTX-2.3: модель → описание (и NAG) → Stage1 →
//! апскейл ×2 → Stage2, либо Retake / IC-LoRA / LipDub / по звуку → декодеры.
//! Музыка ACE-Step: модель → [запись в латент] → генерация (все режимы);
//! YuE2: модель → песня (звук, латент, партитура) → [декодер]; запись в
//! партитуру (SheetSage2).

use crate::schema::{field, port, FieldType as F, NodeSpec, PortType as P};

const IMG: &str = "Модели картинок";
const H3: &str = "MiniMax-H3";
const LTX: &str = "LTX-2.3";
const MUSIC: &str = "Музыка";

/// Режимы ACE-Step (порядок — как у прежней ноды Synthos).
pub const ACE_MODES: &[&str] = &["text2music", "retake", "repaint", "extend", "edit", "cover", "extract"];
/// LM кодов ACE-Step: полная или малая.
pub const ACE_LMS: &[&str] = &["4b", "1.7b"];
/// Тональность; пусто — выводит LM.
pub const KEYSCALES: &[&str] = &[
    "", "C major", "C minor", "C# major", "C# minor", "D major", "D minor", "D# major", "D# minor", "E major", "E minor", "F major", "F minor", "F# major", "F# minor", "G major",
    "G minor", "G# major", "G# minor", "A major", "A minor", "A# major", "A# minor", "B major", "B minor",
];
/// Размер такта; пусто — выводит LM.
pub const TIMESIGS: &[&str] = &["", "4/4", "3/4", "6/8", "2/4", "5/4", "7/8", "9/8", "12/8"];
pub const NORMS: &[&str] = &["peak", "rms", "off"];
pub const DCW_MODES: &[&str] = &["off", "double", "low", "high", "pix"];
/// Дорожки режима extract.
pub const TRACKS: &[&str] = &["vocals", "backing_vocals", "drums", "bass", "guitar", "keyboard", "percussion", "strings", "synth", "fx", "brass", "woodwinds"];
/// Партитура YuE2: с аккордами, только мелодия, без партитуры.
pub const COTS: &[&str] = &["full", "melody", "off"];
pub const TRANSCRIBE_MODES: &[&str] = &["melody", "full"];
pub const VOICES: &[&str] = &["both", "vocal", "ins"];

/// Кадров в секунду у LTX-2.3.
pub const LTX_FPS: &[&str] = &["24", "25", "48", "50"];
/// Управляющий сигнал IC-LoRA.
pub const LTX_CONTROLS: &[&str] = &["none", "canny", "depth"];

/// Квант трансформера: пусто — из Параметров службы.
pub const QUANTS: &[&str] = &["", "nvfp4", "mxfp8", "bf16"];
/// Размер картинок-референсов H3: под площадь кадра или крупно (2048).
pub const REF_SIZES: &[&str] = &["match", "max"];
/// Разрешение условия Qwen-Image 2.1; пусто — по модели (1024).
pub const RESOLUTIONS: &[&str] = &["", "512", "768", "1024", "1536", "2048"];
/// Как вписать картинку в размер латента.
pub const FITS: &[&str] = &["cover", "stretch"];

pub static IMAGE_MODEL: NodeSpec = NodeSpec {
    kind: "image_model",
    title: "Модель картинок",
    description: "FLUX.1, FLUX.2, Qwen-Image, Qwen-Image 2.1 или SDXL из библиотеки службы (пусто — модель по умолчанию)",
    category: IMG,
    inputs: &[],
    outputs: &[port("model", "Модель", P::Model)],
    fields: &[
        field("model", "Модель", F::Text, ""),
        field("quant", "Квант", F::Choice(QUANTS), ""),
        field("resolution", "Разрешение условия (Qwen-Image 2.1)", F::Choice(RESOLUTIONS), ""),
    ],
    stage: true,
};

pub static IMAGE_ENCODE: NodeSpec = NodeSpec {
    kind: "image_encode",
    title: "Кодировщик описания",
    description: "Описание (и негатив) в условие; Qwen-Image видит в описании и картинки (со входа или из референсов)",
    category: IMG,
    inputs: &[port("model", "Модель", P::Model), port("prompt", "Описание", P::Text), port("negative", "Без", P::Text), port("images", "Картинки", P::List), port("refs", "Референсы", P::Cond)],
    outputs: &[port("cond", "Условие", P::Cond)],
    fields: &[field("prompt", "Описание", F::LongText, ""), field("negative", "Без", F::Text, ""), field("seq_len", "Токенов текста (0 — по модели)", F::Number, "0")],
    stage: true,
};

pub static IMAGE_REFS: NodeSpec = NodeSpec {
    kind: "image_refs",
    title: "Референсы",
    description: "Картинки-референсы для правки (FLUX.2 — до 10, Qwen-Image Edit и 2.1); «Ещё референсы» — цепочка таких нод",
    category: IMG,
    inputs: &[port("model", "Модель", P::Model), port("images", "Картинки", P::List), port("refs", "Ещё референсы", P::Cond)],
    outputs: &[port("refs", "Референсы", P::Cond)],
    fields: &[],
    stage: true,
};

pub static IMAGE_LATENT: NodeSpec = NodeSpec {
    kind: "image_latent",
    title: "Пустой латент",
    description: "Размер картинки (0 — по модели или по первому референсу)",
    category: IMG,
    inputs: &[],
    outputs: &[port("latent", "Латент", P::Latent)],
    fields: &[field("width", "Ширина", F::Number, "1024"), field("height", "Высота", F::Number, "1024")],
    stage: true,
};

pub static IMAGE_TO_LATENT: NodeSpec = NodeSpec {
    kind: "image_to_latent",
    title: "Картинка в латент",
    description: "Исходная картинка для img2img (FLUX.1, FLUX.2, SDXL); размер — с латента на входе или картинки",
    category: IMG,
    inputs: &[port("model", "Модель", P::Model), port("image", "Картинка", P::Image), port("size", "Размер", P::Latent)],
    outputs: &[port("latent", "Латент", P::Latent)],
    fields: &[field("fit", "Вписать", F::Choice(FITS), "cover")],
    stage: true,
};

pub static IMAGE_SAMPLE: NodeSpec = NodeSpec {
    kind: "image_sample",
    title: "Сэмплер",
    description: "Денойз по условию: с пустого латента — новая картинка, с картинки — img2img; референсы — правка",
    category: IMG,
    inputs: &[port("model", "Модель", P::Model), port("cond", "Условие", P::Cond), port("latent", "Латент", P::Latent), port("refs", "Референсы", P::Cond)],
    outputs: &[port("latent", "Латент", P::Latent)],
    fields: &[
        field("steps", "Шагов (0 — по модели)", F::Number, "0"),
        field("guidance", "CFG (0 — по модели)", F::Number, "0"),
        field("seed", "Сид", F::Number, "0"),
        field("denoise", "Сила img2img (0 — по модели)", F::Number, "0"),
    ],
    stage: true,
};

pub static IMAGE_DECODE: NodeSpec = NodeSpec {
    kind: "image_decode",
    title: "Декодер картинки",
    description: "Латент в картинку (VAE)",
    category: IMG,
    inputs: &[port("model", "Модель", P::Model), port("latent", "Латент", P::Latent)],
    outputs: &[port("image", "Картинка", P::Image)],
    fields: &[],
    stage: true,
};

pub static H3_MODEL: NodeSpec = NodeSpec {
    kind: "h3_model",
    title: "Модель H3",
    description: "MiniMax-H3 из библиотеки службы: FL2VA (описание, ключевые кадры) или Ref2VA (референсы); LoRA — например Turbo (6 шагов без CFG)",
    category: H3,
    inputs: &[],
    outputs: &[port("model", "Модель", P::Model)],
    fields: &[
        field("model", "Модель", F::Text, ""),
        field("quant", "Квант", F::Choice(QUANTS), ""),
        field("lora", "LoRA", F::File, ""),
        field("lora_strength", "Сила LoRA", F::Number, "1"),
    ],
    stage: true,
};

pub static H3_LATENT: NodeSpec = NodeSpec {
    kind: "h3_latent",
    title: "Латент ролика H3",
    description: "Размер и длительность ролика со звуком (голос в референсах задаёт длительность сам)",
    category: H3,
    inputs: &[],
    outputs: &[port("latent", "Латент", P::Latent)],
    fields: &[field("width", "Ширина", F::Number, "1344"), field("height", "Высота", F::Number, "768"), field("seconds", "Секунд", F::Number, "5")],
    stage: true,
};

pub static H3_ENCODE: NodeSpec = NodeSpec {
    kind: "h3_encode",
    title: "Кодировщик H3",
    description: "Описание с ключевыми кадрами (первый, последний) или референсами (картинки, ролики, звук — до 12) в условие",
    category: H3,
    inputs: &[
        port("model", "Модель", P::Model),
        port("latent", "Латент", P::Latent),
        port("prompt", "Описание", P::Text),
        port("negative", "Без", P::Text),
        port("first_frame", "Первый кадр", P::Image),
        port("last_frame", "Последний кадр", P::Image),
        port("refs", "Референсы", P::List),
    ],
    outputs: &[port("cond", "Условие", P::Cond)],
    fields: &[
        field("prompt", "Описание", F::LongText, ""),
        field("negative", "Без", F::Text, ""),
        field("ref_size", "Картинки-референсы", F::Choice(REF_SIZES), "match"),
    ],
    stage: true,
};

pub static H3_SAMPLE: NodeSpec = NodeSpec {
    kind: "h3_sample",
    title: "Сэмплер H3",
    description: "Денойз видео и звука по условию; исходный ролик — перерисовка с силой",
    category: H3,
    inputs: &[port("model", "Модель", P::Model), port("cond", "Условие", P::Cond), port("source_video", "Исходный ролик", P::Video)],
    outputs: &[port("latent", "Латент", P::Latent)],
    fields: &[
        field("steps", "Шагов", F::Number, "20"),
        field("guidance", "CFG", F::Number, "5"),
        field("seed", "Сид", F::Number, "0"),
        field("strength", "Сила перерисовки", F::Number, "0.6"),
    ],
    stage: true,
};

pub static H3_DECODE_VIDEO: NodeSpec = NodeSpec {
    kind: "h3_decode_video",
    title: "Декодер видео H3",
    description: "Латент в ролик без звука",
    category: H3,
    inputs: &[port("model", "Модель", P::Model), port("latent", "Латент", P::Latent)],
    outputs: &[port("video", "Ролик", P::Video)],
    fields: &[],
    stage: true,
};

pub static H3_DECODE_AUDIO: NodeSpec = NodeSpec {
    kind: "h3_decode_audio",
    title: "Декодер звука H3",
    description: "Латент в звук ролика",
    category: H3,
    inputs: &[port("model", "Модель", P::Model), port("latent", "Латент", P::Latent)],
    outputs: &[port("audio", "Звук", P::Audio)],
    fields: &[],
    stage: true,
};

pub static LTX_MODEL: NodeSpec = NodeSpec {
    kind: "ltx_model",
    title: "Модель LTX",
    description: "LTX-2.3 из библиотеки службы (Gemma, апскейлер и IC-LoRA — её компоненты). LoRA: путь к файлу или union / motion / lipdub / distilled",
    category: LTX,
    inputs: &[],
    outputs: &[port("model", "Модель", P::Model)],
    fields: &[
        field("model", "Модель", F::Text, ""),
        field("quant", "Квант DiT", F::Choice(QUANTS), ""),
        field("lora", "LoRA", F::File, ""),
        field("lora_strength", "Сила LoRA", F::Number, "1"),
        field("depth", "Depth Anything (каталог)", F::File, ""),
    ],
    stage: true,
};

pub static LTX_ENCODE: NodeSpec = NodeSpec {
    kind: "ltx_encode",
    title: "Кодировщик LTX",
    description: "Описание → контексты видео и звука (Gemma → коннекторы)",
    category: LTX,
    inputs: &[port("model", "Модель", P::Model), port("prompt", "Описание", P::Text)],
    outputs: &[port("cond", "Условие", P::Cond)],
    fields: &[field("prompt", "Описание", F::LongText, "")],
    stage: true,
};

pub static LTX_NAG: NodeSpec = NodeSpec {
    kind: "ltx_nag",
    title: "NAG LTX",
    description: "Чего не должно быть в ролике (Normalized Attention Guidance) — для Stage1",
    category: LTX,
    inputs: &[port("model", "Модель", P::Model), port("prompt", "Без", P::Text)],
    outputs: &[port("nag", "NAG", P::Cond)],
    fields: &[
        field("prompt", "Без (пусто — по умолчанию)", F::LongText, ""),
        field("scale", "Масштаб", F::Number, "11"),
        field("alpha", "Alpha", F::Number, "0.25"),
        field("tau", "Tau", F::Number, "2.5"),
    ],
    stage: true,
};

pub static LTX_IMAGE: NodeSpec = NodeSpec {
    kind: "ltx_image",
    title: "Кадр-условие LTX",
    description: "Картинка в ролик: кадр 0 — первый кадр, дальше — ключевой кадр на этом месте",
    category: LTX,
    inputs: &[port("image", "Картинка", P::Image)],
    outputs: &[port("image", "Кадр", P::Cond)],
    fields: &[field("strength", "Сила", F::Number, "1"), field("frame", "Кадр", F::Number, "0")],
    stage: true,
};

const LTX_SHAPE: [crate::schema::FieldSpec; 4] = [
    field("width", "Ширина", F::Number, "1024"),
    field("height", "Высота", F::Number, "576"),
    field("seconds", "Секунд", F::Number, "10"),
    field("fps", "Кадров/с", F::Choice(LTX_FPS), "24"),
];

pub static LTX_STAGE1: NodeSpec = NodeSpec {
    kind: "ltx_stage1",
    title: "Stage1 LTX",
    description: "Денойз видео и звука на половинной сетке (дальше — апскейл ×2 и Stage2)",
    category: LTX,
    inputs: &[port("model", "Модель", P::Model), port("cond", "Условие", P::Cond), port("nag", "NAG", P::Cond), port("image", "Кадр", P::Cond)],
    outputs: &[port("latent", "Латент", P::Latent)],
    fields: &[LTX_SHAPE[0], LTX_SHAPE[1], LTX_SHAPE[2], LTX_SHAPE[3], field("seed", "Сид", F::Number, "0")],
    stage: true,
};

pub static LTX_UPSCALE: NodeSpec = NodeSpec {
    kind: "ltx_upscale",
    title: "Апскейл LTX ×2",
    description: "Латент видео ×2 по сторонам (апскейлер модели)",
    category: LTX,
    inputs: &[port("model", "Модель", P::Model), port("latent", "Латент", P::Latent)],
    outputs: &[port("latent", "Латент", P::Latent)],
    fields: &[],
    stage: true,
};

pub static LTX_STAGE2: NodeSpec = NodeSpec {
    kind: "ltx_stage2",
    title: "Stage2 LTX",
    description: "Доводка латента после апскейла (то же условие и кадр)",
    category: LTX,
    inputs: &[port("model", "Модель", P::Model), port("latent", "Латент", P::Latent), port("cond", "Условие", P::Cond), port("image", "Кадр", P::Cond)],
    outputs: &[port("latent", "Латент", P::Latent)],
    fields: &[field("seed", "Сид", F::Number, "0")],
    stage: true,
};

pub static LTX_RETAKE: NodeSpec = NodeSpec {
    kind: "ltx_retake",
    title: "Retake LTX",
    description: "Перегенерировать отрезок исходного ролика по описанию",
    category: LTX,
    inputs: &[port("model", "Модель", P::Model), port("cond", "Условие", P::Cond), port("video", "Ролик", P::Video)],
    outputs: &[port("latent", "Латент", P::Latent)],
    fields: &[
        LTX_SHAPE[0],
        LTX_SHAPE[1],
        LTX_SHAPE[2],
        LTX_SHAPE[3],
        field("start", "С, с", F::Number, "0"),
        field("end", "По, с", F::Number, "2"),
        field("seed", "Сид", F::Number, "0"),
    ],
    stage: true,
};

pub static LTX_IC_LORA: NodeSpec = NodeSpec {
    kind: "ltx_ic_lora",
    title: "IC-LoRA LTX",
    description: "Ролик по образцу: как есть, по контурам (Canny) или глубине; адаптер — LoRA модели",
    category: LTX,
    inputs: &[port("model", "Модель", P::Model), port("cond", "Условие", P::Cond), port("video", "Образец", P::Video)],
    outputs: &[port("latent", "Латент", P::Latent)],
    fields: &[
        LTX_SHAPE[0],
        LTX_SHAPE[1],
        LTX_SHAPE[2],
        LTX_SHAPE[3],
        field("downscale", "Уменьшить образец", F::Number, "2"),
        field("strength", "Сила образца", F::Number, "1"),
        field("control", "Сигнал", F::Choice(LTX_CONTROLS), "none"),
        field("canny_low", "Canny low", F::Number, "0.1"),
        field("canny_high", "Canny high", F::Number, "0.3"),
        field("seed", "Сид", F::Number, "0"),
    ],
    stage: true,
};

pub static LTX_LIPDUB: NodeSpec = NodeSpec {
    kind: "ltx_lipdub",
    title: "LipDub LTX",
    description: "Переозвучка ролика новой речью: губы под звук (обе стадии сразу; LoRA модели — lipdub)",
    category: LTX,
    inputs: &[port("model", "Модель", P::Model), port("cond", "Условие", P::Cond), port("video", "Ролик", P::Video), port("audio", "Речь", P::Audio)],
    outputs: &[port("latent", "Латент", P::Latent)],
    fields: &[LTX_SHAPE[0], LTX_SHAPE[1], LTX_SHAPE[2], LTX_SHAPE[3]],
    stage: true,
};

pub static LTX_A2V: NodeSpec = NodeSpec {
    kind: "ltx_a2v",
    title: "Видео по звуку LTX",
    description: "Ролик под звук (речь — липсинк, музыка — движение); обе стадии сразу",
    category: LTX,
    inputs: &[port("model", "Модель", P::Model), port("cond", "Условие", P::Cond), port("audio", "Звук", P::Audio)],
    outputs: &[port("latent", "Латент", P::Latent)],
    fields: &[LTX_SHAPE[0], LTX_SHAPE[1], LTX_SHAPE[2], LTX_SHAPE[3], field("seed", "Сид", F::Number, "0")],
    stage: true,
};

pub static LTX_DECODE_VIDEO: NodeSpec = NodeSpec {
    kind: "ltx_decode_video",
    title: "Декодер видео LTX",
    description: "Латент в ролик без звука",
    category: LTX,
    inputs: &[port("model", "Модель", P::Model), port("latent", "Латент", P::Latent)],
    outputs: &[port("video", "Ролик", P::Video)],
    fields: &[],
    stage: true,
};

pub static LTX_DECODE_AUDIO: NodeSpec = NodeSpec {
    kind: "ltx_decode_audio",
    title: "Декодер звука LTX",
    description: "Звуковые токены в звук (48 кГц, стерео)",
    category: LTX,
    inputs: &[port("model", "Модель", P::Model), port("latent", "Латент", P::Latent)],
    outputs: &[port("audio", "Звук", P::Audio)],
    fields: &[],
    stage: true,
};

pub static ACE_MODEL: NodeSpec = NodeSpec {
    kind: "acestep_model",
    title: "Модель ACE-Step",
    description: "ACE-Step v1.5 из библиотеки службы (DiT base/turbo/sft; LM кодов, текстовый энкодер и VAE — её компоненты)",
    category: MUSIC,
    inputs: &[],
    outputs: &[port("model", "Модель", P::Model)],
    fields: &[field("model", "Модель", F::Text, ""), field("lm", "LM кодов", F::Choice(ACE_LMS), "4b"), field("quant", "Квант", F::Choice(QUANTS), "")],
    stage: true,
};

pub static ACE_ENCODE: NodeSpec = NodeSpec {
    kind: "acestep_encode",
    title: "Запись в латент ACE-Step",
    description: "Звук в латент VAE (25 кадров в секунду) — исходник для retake, repaint, extend, edit, cover и extract",
    category: MUSIC,
    inputs: &[port("model", "Модель", P::Model), port("audio", "Звук", P::Audio)],
    outputs: &[port("latent", "Латент", P::Latent)],
    fields: &[field("chunk", "Окно, с", F::Number, "30"), field("overlap", "Контекст окна, с", F::Number, "0.5")],
    stage: true,
};

pub static ACE_GENERATE: NodeSpec = NodeSpec {
    kind: "acestep_generate",
    title: "Генерация ACE-Step",
    description: "Музыка по тегам и стихам или правка записи (латент на входе): retake, repaint, extend, edit, cover, extract. Нули в шагах, CFG и сдвиге — умолчания варианта DiT",
    category: MUSIC,
    inputs: &[port("model", "Модель", P::Model), port("prompt", "Теги", P::Text), port("lyrics", "Стихи", P::Text), port("latent", "Исходный латент", P::Latent)],
    outputs: &[port("audio", "Звук", P::Audio), port("latent", "Латент", P::Latent)],
    fields: &[
        field("mode", "Режим", F::Choice(ACE_MODES), "text2music"),
        field("prompt", "Теги", F::LongText, ""),
        field("lyrics", "Стихи", F::LongText, ""),
        field("seconds", "Секунд (≤0 — решает LM)", F::Number, "-1"),
        field("steps", "Шагов (0 — по модели)", F::Number, "0"),
        field("guidance", "CFG (0 — по модели)", F::Number, "0"),
        field("shift", "Сдвиг (0 — по модели)", F::Number, "0"),
        field("seed", "Сид (0 — случайный)", F::Number, "0"),
        field("temperature", "Температура LM", F::Number, "0.85"),
        field("top_p", "Top-p LM", F::Number, "0.9"),
        field("top_k", "Top-k LM (0 — выкл.)", F::Number, "0"),
        field("min_p", "Min-p LM", F::Number, "0"),
        field("lm_guidance", "CFG LM", F::Number, "2"),
        field("cot", "LM выводит метаданные (CoT)", F::Bool, "true"),
        field("ar", "LM кодов (выкл. — только turbo)", F::Bool, "true"),
        field("bpm", "Темп (0 — решает LM)", F::Number, "0"),
        field("keyscale", "Тональность", F::Choice(KEYSCALES), ""),
        field("timesig", "Размер", F::Choice(TIMESIGS), ""),
        field("norm", "Нормализация", F::Choice(NORMS), "peak"),
        field("dcw", "DCW", F::Choice(DCW_MODES), "off"),
        field("dcw_scaler", "DCW: низкие", F::Number, "0.02"),
        field("dcw_high_scaler", "DCW: высокие", F::Number, "0.06"),
        field("retake_variance", "Retake: разброс", F::Number, "0.5"),
        field("retake_seed", "Retake: сид (0 — случайный)", F::Number, "0"),
        field("repaint_start", "Repaint: начало, с", F::Number, "0"),
        field("repaint_end", "Repaint: конец, с (-1 — до конца)", F::Number, "-1"),
        field("repaint_strength", "Repaint: сила", F::Number, "0.7"),
        field("edit_n_min", "Edit: от доли шагов", F::Number, "0"),
        field("edit_n_max", "Edit: до доли шагов", F::Number, "1"),
        field("track", "Extract: дорожка", F::Choice(TRACKS), "vocals"),
    ],
    stage: true,
};

pub static YUE2_MODEL: NodeSpec = NodeSpec {
    kind: "yue2_model",
    title: "Модель YuE2",
    description: "YuE2 из библиотеки службы: песня с вокалом по стилю и стихам; декодер и транскриптор SheetSage2 — её компоненты",
    category: MUSIC,
    inputs: &[],
    outputs: &[port("model", "Модель", P::Model)],
    fields: &[field("model", "Модель", F::Text, ""), field("quant", "Квант", F::Choice(QUANTS), "")],
    stage: true,
};

pub static YUE2_GENERATE: NodeSpec = NodeSpec {
    kind: "yue2_generate",
    title: "Песня YuE2",
    description: "Партитура (ABC) и музыка по стилю и стихам; готовая партитура на входе сильнее сгенерированной",
    category: MUSIC,
    inputs: &[port("model", "Модель", P::Model), port("prompt", "Стиль", P::Text), port("lyrics", "Стихи", P::Text), port("abc", "Партитура", P::Text)],
    outputs: &[port("audio", "Звук", P::Audio), port("latent", "Латент", P::Latent), port("score", "Партитура", P::Text)],
    fields: &[
        field("prompt", "Стиль", F::LongText, ""),
        field("lyrics", "Стихи", F::LongText, ""),
        field("cot", "Партитура", F::Choice(COTS), "full"),
        field("seconds", "Секунд (0 — до конца)", F::Number, "60"),
        field("steps", "Шагов акустики", F::Number, "32"),
        field("guidance", "CFG (0 — по режиму)", F::Number, "0"),
        field("seed", "Сид (0 — случайный)", F::Number, "0"),
        field("temperature", "Температура", F::Number, "1"),
        field("top_p", "Top-p", F::Number, "0.95"),
        field("top_k", "Top-k", F::Number, "100"),
        field("repetition_penalty", "Штраф повторов", F::Number, "1.2"),
        field("core_frames", "Тайл декодера, кадров", F::Number, "1024"),
    ],
    stage: true,
};

pub static YUE2_DECODE: NodeSpec = NodeSpec {
    kind: "yue2_decode",
    title: "Декодер YuE2",
    description: "Латент песни в звук (48 кГц, стерео)",
    category: MUSIC,
    inputs: &[port("model", "Модель", P::Model), port("latent", "Латент", P::Latent)],
    outputs: &[port("audio", "Звук", P::Audio)],
    fields: &[field("core_frames", "Тайл декодера, кадров", F::Number, "1024")],
    stage: true,
};

pub static YUE2_TRANSCRIBE: NodeSpec = NodeSpec {
    kind: "yue2_transcribe",
    title: "Запись в партитуру",
    description: "SheetSage2: мелодии записи в партитуру ABC для YuE2 (кавер — только мелодия)",
    category: MUSIC,
    inputs: &[port("model", "Модель", P::Model), port("audio", "Звук", P::Audio)],
    outputs: &[port("score", "Партитура", P::Text)],
    fields: &[field("mode", "Что писать", F::Choice(TRANSCRIBE_MODES), "melody"), field("voices", "Мелодии", F::Choice(VOICES), "both")],
    stage: true,
};

pub static ALL: &[&NodeSpec] = &[
    &IMAGE_MODEL,
    &IMAGE_ENCODE,
    &IMAGE_REFS,
    &IMAGE_LATENT,
    &IMAGE_TO_LATENT,
    &IMAGE_SAMPLE,
    &IMAGE_DECODE,
    &H3_MODEL,
    &H3_LATENT,
    &H3_ENCODE,
    &H3_SAMPLE,
    &H3_DECODE_VIDEO,
    &H3_DECODE_AUDIO,
    &LTX_MODEL,
    &LTX_ENCODE,
    &LTX_NAG,
    &LTX_IMAGE,
    &LTX_STAGE1,
    &LTX_UPSCALE,
    &LTX_STAGE2,
    &LTX_RETAKE,
    &LTX_IC_LORA,
    &LTX_LIPDUB,
    &LTX_A2V,
    &LTX_DECODE_VIDEO,
    &LTX_DECODE_AUDIO,
    &ACE_MODEL,
    &ACE_ENCODE,
    &ACE_GENERATE,
    &YUE2_MODEL,
    &YUE2_GENERATE,
    &YUE2_DECODE,
    &YUE2_TRANSCRIBE,
];
