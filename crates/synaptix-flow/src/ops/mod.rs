//! Виды нод: данные (текст, файл, шаблон, строки), генерация в службе
//! (LLM, картинка, видео, музыка, речь, расшифровка; OmniVoice, VoxCPM2,
//! VibeVoice, разметка говорящих), стадии моделей
//! ([`stages`]: модель → условие → латент → картинка/ролик) и монтаж ffmpeg
//! (склейка, наложение звука, кадр, обрезка, картинка в ролик).

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use crate::run::OpFn;
use crate::schema::{field, port, FieldType as F, NodeSpec, PortType as P};

mod data;
pub mod media;
pub mod stages;

const GEN: &str = "Генерация";
const EDIT: &str = "Монтаж";
const DATA: &str = "Данные";
const SPEECH: &str = "Речь";

pub static TEXT: NodeSpec = NodeSpec {
    kind: "text",
    title: "Текст",
    description: "Постоянный текст",
    category: DATA,
    inputs: &[],
    outputs: &[port("text", "Текст", P::Text)],
    fields: &[field("text", "Текст", F::LongText, "")],
    stage: false,
};

pub static FILE: NodeSpec = NodeSpec {
    kind: "file",
    title: "Файл",
    description: "Картинка, звук или ролик с диска",
    category: DATA,
    inputs: &[],
    outputs: &[port("file", "Файл", P::Media)],
    fields: &[field("path", "Путь", F::File, "")],
    stage: false,
};

pub static FORMAT: NodeSpec = NodeSpec {
    kind: "format",
    title: "Шаблон текста",
    description: "Текст с подстановками {{a}} … {{f}} из входов",
    category: DATA,
    inputs: &[
        port("a", "a", P::Any),
        port("b", "b", P::Any),
        port("c", "c", P::Any),
        port("d", "d", P::Any),
        port("e", "e", P::Any),
        port("f", "f", P::Any),
    ],
    outputs: &[port("text", "Текст", P::Text)],
    fields: &[field("template", "Шаблон", F::LongText, "")],
    stage: false,
};

pub static LINES: NodeSpec = NodeSpec {
    kind: "lines",
    title: "Строки",
    description: "Текст → список непустых строк (без маркеров списка) — пакет для следующих нод",
    category: DATA,
    inputs: &[port("text", "Текст", P::Text)],
    outputs: &[port("list", "Список", P::List)],
    fields: &[],
    stage: false,
};

pub static LIST: NodeSpec = NodeSpec {
    kind: "list",
    title: "Список",
    description: "Собрать входы в список (пакет); списки на входах раскрываются",
    category: DATA,
    inputs: &[
        port("in1", "1", P::List),
        port("in2", "2", P::List),
        port("in3", "3", P::List),
        port("in4", "4", P::List),
        port("in5", "5", P::List),
        port("in6", "6", P::List),
    ],
    outputs: &[port("list", "Список", P::List)],
    fields: &[field("items", "Элементы (по строке)", F::LongText, "")],
    stage: false,
};

pub static LLM: NodeSpec = NodeSpec {
    kind: "llm",
    title: "Языковая модель",
    description: "Ответ языковой модели службы на запрос",
    category: GEN,
    inputs: &[port("prompt", "Запрос", P::Text), port("system", "Система", P::Text)],
    outputs: &[port("text", "Ответ", P::Text), port("thinking", "Размышления", P::Text)],
    fields: &[
        field("prompt", "Запрос", F::LongText, ""),
        field("system", "Система", F::LongText, ""),
        field("model", "Модель", F::Text, ""),
        field("max_tokens", "Токенов", F::Number, "0"),
        field("think", "Размышления", F::Bool, "false"),
        field("context", "Контекст (0 — по модели)", F::Number, "0"),
        field("temperature", "Температура (-1 — по модели)", F::Number, "-1.0"),
        field("top_p", "Top-p (-1 — по модели)", F::Number, "-1.0"),
        field("top_k", "Top-k (-1 — по модели)", F::Number, "-1"),
        field("min_p", "Min-p (-1 — по модели)", F::Number, "-1.0"),
        field("repeat_penalty", "Штраф повторов (-1 — по модели)", F::Number, "-1.0"),
        field("seed", "Сид (-1 — случайный)", F::Number, "-1"),
    ],
    stage: false,
};

pub static IMAGE: NodeSpec = NodeSpec {
    kind: "image",
    title: "Картинка",
    description: "Картинка по описанию (FLUX, Qwen-Image, SDXL), референсы и img2img",
    category: GEN,
    inputs: &[port("prompt", "Описание", P::Text), port("images", "Референсы", P::List), port("init", "Исходная", P::Image)],
    outputs: &[port("image", "Картинка", P::Image)],
    fields: &[
        field("prompt", "Описание", F::LongText, ""),
        field("negative", "Без", F::Text, ""),
        field("model", "Модель", F::Text, ""),
        field("width", "Ширина", F::Number, "0"),
        field("height", "Высота", F::Number, "0"),
        field("steps", "Шагов", F::Number, "0"),
        field("guidance", "CFG", F::Number, "0"),
        field("strength", "Сила (img2img)", F::Number, "0"),
        field("seed", "Сид", F::Number, "0"),
    ],
    stage: false,
};

pub const CONTROLS: &[&str] = &["", "union", "canny", "motion-track"];

pub static VIDEO: NodeSpec = NodeSpec {
    kind: "video",
    title: "Видео",
    description: "Ролик (MiniMax-H3, LTX-2.3): по описанию, с ключевых кадров, по звуку (липсинк), референсы, перерисовка, retake, продление, IC-LoRA",
    category: GEN,
    inputs: &[
        port("prompt", "Описание", P::Text),
        port("first_frame", "Первый кадр", P::Image),
        port("last_frame", "Последний кадр", P::Image),
        port("refs", "Референсы", P::List),
        port("audio", "Звук", P::Audio),
        port("source_video", "Исходный ролик", P::Video),
        port("control_video", "Управляющий ролик", P::Video),
    ],
    outputs: &[port("video", "Ролик", P::Video)],
    fields: &[
        field("prompt", "Описание", F::LongText, ""),
        field("negative", "Без", F::Text, ""),
        field("model", "Модель", F::Text, ""),
        field("width", "Ширина", F::Number, "0"),
        field("height", "Высота", F::Number, "0"),
        field("seconds", "Секунд", F::Number, "0"),
        field("steps", "Шагов", F::Number, "0"),
        field("guidance", "CFG", F::Number, "0"),
        field("seed", "Сид", F::Number, "0"),
        field("control", "IC-LoRA", F::Choice(CONTROLS), ""),
        field("restyle", "Перерисовка (H3)", F::Bool, "false"),
        field("extend", "Продление", F::Bool, "false"),
        field("retake_start", "Retake с, с", F::Number, "0"),
        field("retake_end", "Retake по, с", F::Number, "0"),
        field("strength", "Сила", F::Number, "0"),
        field("two_stage", "Две стадии", F::Bool, "false"),
        field("chain", "Цепочкой (с последнего кадра)", F::Bool, "false"),
    ],
    stage: false,
};

pub static MUSIC: NodeSpec = NodeSpec {
    kind: "music",
    title: "Музыка",
    description: "Песня или инструментал (ACE-Step, YuE2)",
    category: GEN,
    inputs: &[port("prompt", "Стиль", P::Text), port("lyrics", "Стихи", P::Text)],
    outputs: &[port("audio", "Звук", P::Audio)],
    fields: &[
        field("prompt", "Стиль", F::LongText, ""),
        field("lyrics", "Стихи", F::LongText, ""),
        field("model", "Модель", F::Text, ""),
        field("seconds", "Секунд", F::Number, "0"),
        field("seed", "Сид", F::Number, "0"),
    ],
    stage: false,
};

pub static SPEAK: NodeSpec = NodeSpec {
    kind: "speak",
    title: "Речь",
    description: "Синтез речи службой (голос — образец или имя)",
    category: GEN,
    inputs: &[port("text", "Текст", P::Text), port("voice", "Голос", P::Audio)],
    outputs: &[port("audio", "Звук", P::Audio)],
    fields: &[
        field("text", "Текст", F::LongText, ""),
        field("voice", "Голос", F::Text, ""),
        field("language", "Язык", F::Text, ""),
        field("model", "Модель", F::Text, ""),
    ],
    stage: false,
};

pub static TRANSCRIBE: NodeSpec = NodeSpec {
    kind: "transcribe",
    title: "Расшифровка",
    description: "Речь из звука или ролика в текст",
    category: GEN,
    inputs: &[port("file", "Звук или ролик", P::Media)],
    outputs: &[port("text", "Текст", P::Text)],
    fields: &[field("language", "Язык", F::Text, ""), field("model", "Модель", F::Text, "")],
    stage: false,
};

pub static OMNIVOICE: NodeSpec = NodeSpec {
    kind: "omnivoice",
    title: "OmniVoice",
    description: "Речь на 600+ языках: голос с образца (клон), по описанию или свой",
    category: SPEECH,
    inputs: &[port("text", "Текст", P::Text), port("ref_audio", "Образец голоса", P::Audio), port("ref_text", "Текст образца", P::Text)],
    outputs: &[port("audio", "Звук", P::Audio)],
    fields: &[
        field("text", "Текст", F::LongText, ""),
        field("instruct", "Описание голоса (без образца)", F::Text, ""),
        field("ref_text", "Текст образца", F::Text, ""),
        field("language", "Язык", F::Text, ""),
        field("model", "Модель", F::Text, "omnivoice"),
        field("steps", "Шагов", F::Number, "32"),
        field("guidance", "CFG", F::Number, "2.0"),
        field("t_shift", "Сдвиг времени", F::Number, "0.1"),
        field("speed", "Скорость", F::Number, "1.0"),
        field("seed", "Сид (0 — случайный)", F::Number, "0"),
    ],
    stage: false,
};

pub static VOXCPM: NodeSpec = NodeSpec {
    kind: "voxcpm",
    title: "VoxCPM2",
    description: "Речь: тембр с образца, продолжение записи (её текст обязателен) или оба сразу",
    category: SPEECH,
    inputs: &[port("text", "Текст", P::Text), port("ref_audio", "Образец тембра", P::Audio), port("prompt_audio", "Запись для продолжения", P::Audio), port("prompt_text", "Текст записи", P::Text)],
    outputs: &[port("audio", "Звук", P::Audio)],
    fields: &[
        field("text", "Текст", F::LongText, ""),
        field("prompt_text", "Текст записи", F::Text, ""),
        field("model", "Модель", F::Text, "voxcpm2"),
        field("guidance", "CFG", F::Number, "2.0"),
        field("steps", "Шагов", F::Number, "10"),
        field("max_len", "Предел длины, токенов", F::Number, "2000"),
        field("seed", "Сид (0 — случайный)", F::Number, "1988"),
    ],
    stage: false,
};

pub static VIBEVOICE: NodeSpec = NodeSpec {
    kind: "vibevoice",
    title: "VibeVoice",
    description: "Диалог до 4 голосов: строки «Speaker 1: …» (или простой текст), образцы голосов по порядку говорящих",
    category: SPEECH,
    inputs: &[
        port("script", "Сценарий", P::Text),
        port("voice1", "Голос 1", P::Audio),
        port("voice2", "Голос 2", P::Audio),
        port("voice3", "Голос 3", P::Audio),
        port("voice4", "Голос 4", P::Audio),
    ],
    outputs: &[port("audio", "Звук", P::Audio)],
    fields: &[
        field("script", "Сценарий", F::LongText, ""),
        field("model", "Модель", F::Text, "vibevoice-1-5b"),
        field("guidance", "CFG", F::Number, "1.3"),
        field("steps", "Шагов", F::Number, "20"),
        field("max_length_times", "Предел длины, ×текста", F::Number, "2.0"),
        field("seed", "Сид (0 — случайный)", F::Number, "0"),
    ],
    stage: false,
};

pub static DIARIZE: NodeSpec = NodeSpec {
    kind: "diarize",
    title: "Кто говорит",
    description: "Разметка говорящих (Sortformer, до 4 голосов): сводка и JSON отрезков",
    category: SPEECH,
    inputs: &[port("audio", "Звук или ролик", P::Media)],
    outputs: &[port("text", "Сводка", P::Text), port("json", "JSON", P::Text)],
    fields: &[
        field("model", "Модель", F::Text, ""),
        field("threshold", "Порог", F::Number, "0.5"),
        field("overlap", "Наложения голосов", F::Bool, "true"),
    ],
    stage: false,
};

pub static CONCAT: NodeSpec = NodeSpec {
    kind: "concat",
    title: "Склейка",
    description: "Ролики друг за другом (к размеру первого, 24 к/с, тишина без звука) или дорожки",
    category: EDIT,
    inputs: &[
        port("files", "Список", P::List),
        port("in1", "1", P::Media),
        port("in2", "2", P::Media),
        port("in3", "3", P::Media),
        port("in4", "4", P::Media),
    ],
    outputs: &[port("file", "Результат", P::Media)],
    fields: &[],
    stage: false,
};

pub const MUX_MODES: &[&str] = &["replace", "mix"];

pub static MUX: NodeSpec = NodeSpec {
    kind: "mux",
    title: "Наложение звука",
    description: "Звук на ролик: заменить дорожку или смешать с исходной",
    category: EDIT,
    inputs: &[port("video", "Ролик", P::Video), port("audio", "Звук", P::Audio)],
    outputs: &[port("video", "Ролик", P::Video)],
    fields: &[field("mode", "Режим", F::Choice(MUX_MODES), "replace"), field("volume", "Громкость (mix)", F::Number, "0.8")],
    stage: false,
};

pub static FRAME: NodeSpec = NodeSpec {
    kind: "frame",
    title: "Кадр",
    description: "Кадр ролика: первый, последний или на секунде",
    category: EDIT,
    inputs: &[port("video", "Ролик", P::Video)],
    outputs: &[port("image", "Кадр", P::Image)],
    fields: &[field("at", "Где (first / last / секунда)", F::Text, "last")],
    stage: false,
};

pub static TRIM: NodeSpec = NodeSpec {
    kind: "trim",
    title: "Обрезка",
    description: "Отрезок ролика или звука",
    category: EDIT,
    inputs: &[port("file", "Файл", P::Media)],
    outputs: &[port("file", "Отрезок", P::Media)],
    fields: &[field("start", "С, с", F::Number, "0"), field("end", "По, с (0 — до конца)", F::Number, "0")],
    stage: false,
};

pub static STILL: NodeSpec = NodeSpec {
    kind: "still",
    title: "Картинка в ролик",
    description: "Неподвижная картинка на длину звука или заданное время",
    category: EDIT,
    inputs: &[port("image", "Картинка", P::Image), port("audio", "Звук", P::Audio)],
    outputs: &[port("video", "Ролик", P::Video)],
    fields: &[field("seconds", "Секунд (без звука)", F::Number, "5")],
    stage: false,
};

/// Виды нод и их исполнители в этом процессе. Генерация (`None`) —
/// только в службе: она подставляет исполнителей через [`provide`].
static ALL: &[(&NodeSpec, Option<OpFn>)] = &[
    (&TEXT, Some(data::text)),
    (&FILE, Some(data::file)),
    (&FORMAT, Some(data::format)),
    (&LINES, Some(data::lines)),
    (&LIST, Some(data::list)),
    (&LLM, None),
    (&IMAGE, None),
    (&VIDEO, None),
    (&MUSIC, None),
    (&SPEAK, None),
    (&TRANSCRIBE, None),
    (&OMNIVOICE, None),
    (&VOXCPM, None),
    (&VIBEVOICE, None),
    (&DIARIZE, None),
    (&CONCAT, Some(media::concat)),
    (&MUX, Some(media::mux)),
    (&FRAME, Some(media::frame)),
    (&TRIM, Some(media::trim)),
    (&STILL, Some(media::still)),
];

fn provided() -> &'static RwLock<HashMap<&'static str, OpFn>> {
    static P: OnceLock<RwLock<HashMap<&'static str, OpFn>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

/// Исполнитель вида ноды для этого процесса (служба — генерация и стадии
/// моделей). Перекрывает встроенный.
pub fn provide(kind: &'static str, op: OpFn) {
    if let Ok(mut p) = provided().write() {
        p.insert(kind, op);
    }
}

/// Описание вида ноды и исполнитель, если он есть в этом процессе.
pub fn lookup(kind: &str) -> Option<(&'static NodeSpec, Option<OpFn>)> {
    let spec = spec(kind)?;
    let own = provided().read().ok().and_then(|p| p.get(kind).copied());
    Some((spec, own.or_else(|| ALL.iter().find(|(s, _)| s.kind == kind).and_then(|(_, f)| *f))))
}

pub fn spec(kind: &str) -> Option<&'static NodeSpec> {
    specs().find(|s| s.kind == kind)
}

/// Все виды нод.
pub fn specs() -> impl Iterator<Item = &'static NodeSpec> {
    ALL.iter().map(|(s, _)| *s).chain(stages::ALL.iter().copied())
}

pub fn kinds() -> Vec<&'static str> {
    specs().map(|s| s.kind).collect()
}
