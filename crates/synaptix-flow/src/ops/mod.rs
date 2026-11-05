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

const GEN: &str = n_!("Генерация");
const EDIT: &str = n_!("Монтаж");
const DATA: &str = n_!("Данные");
const SPEECH: &str = n_!("Речь");

pub static TEXT: NodeSpec = NodeSpec {
    kind: "text",
    title: n_!("Текст"),
    description: n_!("Постоянный текст"),
    category: DATA,
    inputs: &[],
    outputs: &[port("text", n_!("Текст"), P::Text)],
    fields: &[field("text", n_!("Текст"), F::LongText, "")],
    stage: false,
};

pub static FILE: NodeSpec = NodeSpec {
    kind: "file",
    title: n_!("Файл"),
    description: n_!("Картинка, звук или ролик с диска"),
    category: DATA,
    inputs: &[],
    outputs: &[port("file", n_!("Файл"), P::Media)],
    fields: &[field("path", n_!("Путь"), F::File, "")],
    stage: false,
};

pub static FORMAT: NodeSpec = NodeSpec {
    kind: "format",
    title: n_!("Шаблон текста"),
    description: n_!("Текст с подстановками {{a}} … {{f}} из входов"),
    category: DATA,
    inputs: &[
        port("a", "a", P::Any),
        port("b", "b", P::Any),
        port("c", "c", P::Any),
        port("d", "d", P::Any),
        port("e", "e", P::Any),
        port("f", "f", P::Any),
    ],
    outputs: &[port("text", n_!("Текст"), P::Text)],
    fields: &[field("template", n_!("Шаблон"), F::LongText, "")],
    stage: false,
};

pub static LINES: NodeSpec = NodeSpec {
    kind: "lines",
    title: n_!("Строки"),
    description: n_!("Текст → список непустых строк (без маркеров списка) — пакет для следующих нод"),
    category: DATA,
    inputs: &[port("text", n_!("Текст"), P::Text)],
    outputs: &[port("list", n_!("Список"), P::List)],
    fields: &[],
    stage: false,
};

pub static LIST: NodeSpec = NodeSpec {
    kind: "list",
    title: n_!("Список"),
    description: n_!("Собрать входы в список (пакет); списки на входах раскрываются"),
    category: DATA,
    inputs: &[
        port("in1", "1", P::List),
        port("in2", "2", P::List),
        port("in3", "3", P::List),
        port("in4", "4", P::List),
        port("in5", "5", P::List),
        port("in6", "6", P::List),
    ],
    outputs: &[port("list", n_!("Список"), P::List)],
    fields: &[field("items", n_!("Элементы (по строке)"), F::LongText, "")],
    stage: false,
};

pub static LLM: NodeSpec = NodeSpec {
    kind: "llm",
    title: n_!("Языковая модель"),
    description: n_!("Ответ языковой модели службы на запрос"),
    category: GEN,
    inputs: &[port("prompt", n_!("Запрос"), P::Text), port("system", n_!("Система"), P::Text)],
    outputs: &[port("text", n_!("Ответ"), P::Text), port("thinking", n_!("Размышления"), P::Text)],
    fields: &[
        field("prompt", n_!("Запрос"), F::LongText, ""),
        field("system", n_!("Система"), F::LongText, ""),
        field("model", n_!("Модель"), F::Text, ""),
        field("max_tokens", n_!("Токенов"), F::Number, "0"),
        field("think", n_!("Размышления"), F::Bool, "false"),
        field("context", n_!("Контекст (0 — по модели)"), F::Number, "0"),
        field("temperature", n_!("Температура (-1 — по модели)"), F::Number, "-1.0"),
        field("top_p", n_!("Top-p (-1 — по модели)"), F::Number, "-1.0"),
        field("top_k", n_!("Top-k (-1 — по модели)"), F::Number, "-1"),
        field("min_p", n_!("Min-p (-1 — по модели)"), F::Number, "-1.0"),
        field("repeat_penalty", n_!("Штраф повторов (-1 — по модели)"), F::Number, "-1.0"),
        field("seed", n_!("Сид (-1 — случайный)"), F::Number, "-1"),
    ],
    stage: false,
};

pub static IMAGE: NodeSpec = NodeSpec {
    kind: "image",
    title: n_!("Картинка"),
    description: n_!("Картинка по описанию (FLUX, Qwen-Image, SDXL), референсы и img2img"),
    category: GEN,
    inputs: &[port("prompt", n_!("Описание"), P::Text), port("images", n_!("Референсы"), P::List), port("init", n_!("Исходная"), P::Image)],
    outputs: &[port("image", n_!("Картинка"), P::Image)],
    fields: &[
        field("prompt", n_!("Описание"), F::LongText, ""),
        field("negative", n_!("Без"), F::Text, ""),
        field("model", n_!("Модель"), F::Text, ""),
        field("width", n_!("Ширина"), F::Number, "0"),
        field("height", n_!("Высота"), F::Number, "0"),
        field("steps", n_!("Шагов"), F::Number, "0"),
        field("guidance", "CFG", F::Number, "0"),
        field("strength", n_!("Сила (img2img)"), F::Number, "0"),
        field("seed", n_!("Сид"), F::Number, "0"),
    ],
    stage: false,
};

pub const CONTROLS: &[&str] = &["", "union", "canny", "motion-track"];

pub static VIDEO: NodeSpec = NodeSpec {
    kind: "video",
    title: n_!("Видео"),
    description: n_!("Ролик (MiniMax-H3, LTX-2.3): по описанию, с ключевых кадров, по звуку (липсинк), референсы, перерисовка, retake, продление, IC-LoRA"),
    category: GEN,
    inputs: &[
        port("prompt", n_!("Описание"), P::Text),
        port("first_frame", n_!("Первый кадр"), P::Image),
        port("last_frame", n_!("Последний кадр"), P::Image),
        port("refs", n_!("Референсы"), P::List),
        port("audio", n_!("Звук"), P::Audio),
        port("source_video", n_!("Исходный ролик"), P::Video),
        port("control_video", n_!("Управляющий ролик"), P::Video),
    ],
    outputs: &[port("video", n_!("Ролик"), P::Video)],
    fields: &[
        field("prompt", n_!("Описание"), F::LongText, ""),
        field("negative", n_!("Без"), F::Text, ""),
        field("model", n_!("Модель"), F::Text, ""),
        field("width", n_!("Ширина"), F::Number, "0"),
        field("height", n_!("Высота"), F::Number, "0"),
        field("seconds", n_!("Секунд"), F::Number, "0"),
        field("steps", n_!("Шагов"), F::Number, "0"),
        field("guidance", "CFG", F::Number, "0"),
        field("seed", n_!("Сид"), F::Number, "0"),
        field("control", "IC-LoRA", F::Choice(CONTROLS), ""),
        field("restyle", n_!("Перерисовка (H3)"), F::Bool, "false"),
        field("extend", n_!("Продление"), F::Bool, "false"),
        field("retake_start", n_!("Retake с, с"), F::Number, "0"),
        field("retake_end", n_!("Retake по, с"), F::Number, "0"),
        field("strength", n_!("Сила"), F::Number, "0"),
        field("two_stage", n_!("Две стадии"), F::Bool, "false"),
        field("chain", n_!("Цепочкой (с последнего кадра)"), F::Bool, "false"),
    ],
    stage: false,
};

pub static MUSIC: NodeSpec = NodeSpec {
    kind: "music",
    title: n_!("Музыка"),
    description: n_!("Песня или инструментал (ACE-Step, YuE2)"),
    category: GEN,
    inputs: &[port("prompt", n_!("Стиль"), P::Text), port("lyrics", n_!("Стихи"), P::Text)],
    outputs: &[port("audio", n_!("Звук"), P::Audio)],
    fields: &[
        field("prompt", n_!("Стиль"), F::LongText, ""),
        field("lyrics", n_!("Стихи"), F::LongText, ""),
        field("model", n_!("Модель"), F::Text, ""),
        field("seconds", n_!("Секунд"), F::Number, "0"),
        field("seed", n_!("Сид"), F::Number, "0"),
    ],
    stage: false,
};

pub static SPEAK: NodeSpec = NodeSpec {
    kind: "speak",
    title: n_!("Речь"),
    description: n_!("Синтез речи службой (голос — образец или имя)"),
    category: GEN,
    inputs: &[port("text", n_!("Текст"), P::Text), port("voice", n_!("Голос"), P::Audio)],
    outputs: &[port("audio", n_!("Звук"), P::Audio)],
    fields: &[
        field("text", n_!("Текст"), F::LongText, ""),
        field("voice", n_!("Голос"), F::Text, ""),
        field("language", n_!("Язык"), F::Text, ""),
        field("model", n_!("Модель"), F::Text, ""),
    ],
    stage: false,
};

pub static TRANSCRIBE: NodeSpec = NodeSpec {
    kind: "transcribe",
    title: n_!("Расшифровка"),
    description: n_!("Речь из звука или ролика в текст"),
    category: GEN,
    inputs: &[port("file", n_!("Звук или ролик"), P::Media)],
    outputs: &[port("text", n_!("Текст"), P::Text)],
    fields: &[field("language", n_!("Язык"), F::Text, ""), field("model", n_!("Модель"), F::Text, "")],
    stage: false,
};

pub static OMNIVOICE: NodeSpec = NodeSpec {
    kind: "omnivoice",
    title: "OmniVoice",
    description: n_!("Речь на 600+ языках: голос с образца (клон), по описанию или свой"),
    category: SPEECH,
    inputs: &[port("text", n_!("Текст"), P::Text), port("ref_audio", n_!("Образец голоса"), P::Audio), port("ref_text", n_!("Текст образца"), P::Text)],
    outputs: &[port("audio", n_!("Звук"), P::Audio)],
    fields: &[
        field("text", n_!("Текст"), F::LongText, ""),
        field("instruct", n_!("Описание голоса (без образца)"), F::Text, ""),
        field("ref_text", n_!("Текст образца"), F::Text, ""),
        field("language", n_!("Язык"), F::Text, ""),
        field("model", n_!("Модель"), F::Text, "omnivoice"),
        field("steps", n_!("Шагов"), F::Number, "32"),
        field("guidance", "CFG", F::Number, "2.0"),
        field("t_shift", n_!("Сдвиг времени"), F::Number, "0.1"),
        field("speed", n_!("Скорость"), F::Number, "1.0"),
        field("seed", n_!("Сид (0 — случайный)"), F::Number, "0"),
    ],
    stage: false,
};

pub static VOXCPM: NodeSpec = NodeSpec {
    kind: "voxcpm",
    title: "VoxCPM2",
    description: n_!("Речь: тембр с образца, продолжение записи (её текст обязателен) или оба сразу"),
    category: SPEECH,
    inputs: &[port("text", n_!("Текст"), P::Text), port("ref_audio", n_!("Образец тембра"), P::Audio), port("prompt_audio", n_!("Запись для продолжения"), P::Audio), port("prompt_text", n_!("Текст записи"), P::Text)],
    outputs: &[port("audio", n_!("Звук"), P::Audio)],
    fields: &[
        field("text", n_!("Текст"), F::LongText, ""),
        field("prompt_text", n_!("Текст записи"), F::Text, ""),
        field("model", n_!("Модель"), F::Text, "voxcpm2"),
        field("guidance", "CFG", F::Number, "2.0"),
        field("steps", n_!("Шагов"), F::Number, "10"),
        field("max_len", n_!("Предел длины, токенов"), F::Number, "2000"),
        field("seed", n_!("Сид (0 — случайный)"), F::Number, "1988"),
    ],
    stage: false,
};

pub static VIBEVOICE: NodeSpec = NodeSpec {
    kind: "vibevoice",
    title: "VibeVoice",
    description: n_!("Диалог до 4 голосов: строки «Speaker 1: …» (или простой текст), образцы голосов по порядку говорящих"),
    category: SPEECH,
    inputs: &[
        port("script", n_!("Сценарий"), P::Text),
        port("voice1", n_!("Голос 1"), P::Audio),
        port("voice2", n_!("Голос 2"), P::Audio),
        port("voice3", n_!("Голос 3"), P::Audio),
        port("voice4", n_!("Голос 4"), P::Audio),
    ],
    outputs: &[port("audio", n_!("Звук"), P::Audio)],
    fields: &[
        field("script", n_!("Сценарий"), F::LongText, ""),
        field("model", n_!("Модель"), F::Text, "vibevoice-1-5b"),
        field("guidance", "CFG", F::Number, "1.3"),
        field("steps", n_!("Шагов"), F::Number, "20"),
        field("max_length_times", n_!("Предел длины, ×текста"), F::Number, "2.0"),
        field("seed", n_!("Сид (0 — случайный)"), F::Number, "0"),
    ],
    stage: false,
};

pub static DIARIZE: NodeSpec = NodeSpec {
    kind: "diarize",
    title: n_!("Кто говорит"),
    description: n_!("Разметка говорящих (Sortformer, до 4 голосов): сводка и JSON отрезков"),
    category: SPEECH,
    inputs: &[port("audio", n_!("Звук или ролик"), P::Media)],
    outputs: &[port("text", n_!("Сводка"), P::Text), port("json", "JSON", P::Text)],
    fields: &[
        field("model", n_!("Модель"), F::Text, ""),
        field("threshold", n_!("Порог"), F::Number, "0.5"),
        field("overlap", n_!("Наложения голосов"), F::Bool, "true"),
    ],
    stage: false,
};

pub static CONCAT: NodeSpec = NodeSpec {
    kind: "concat",
    title: n_!("Склейка"),
    description: n_!("Ролики друг за другом (к размеру первого, 24 к/с, тишина без звука) или дорожки"),
    category: EDIT,
    inputs: &[
        port("files", n_!("Список"), P::List),
        port("in1", "1", P::Media),
        port("in2", "2", P::Media),
        port("in3", "3", P::Media),
        port("in4", "4", P::Media),
    ],
    outputs: &[port("file", n_!("Результат"), P::Media)],
    fields: &[],
    stage: false,
};

pub const MUX_MODES: &[&str] = &["replace", "mix"];

pub static MUX: NodeSpec = NodeSpec {
    kind: "mux",
    title: n_!("Наложение звука"),
    description: n_!("Звук на ролик: заменить дорожку или смешать с исходной"),
    category: EDIT,
    inputs: &[port("video", n_!("Ролик"), P::Video), port("audio", n_!("Звук"), P::Audio)],
    outputs: &[port("video", n_!("Ролик"), P::Video)],
    fields: &[field("mode", n_!("Режим"), F::Choice(MUX_MODES), "replace"), field("volume", n_!("Громкость (mix)"), F::Number, "0.8")],
    stage: false,
};

pub static FRAME: NodeSpec = NodeSpec {
    kind: "frame",
    title: n_!("Кадр"),
    description: n_!("Кадр ролика: первый, последний или на секунде"),
    category: EDIT,
    inputs: &[port("video", n_!("Ролик"), P::Video)],
    outputs: &[port("image", n_!("Кадр"), P::Image)],
    fields: &[field("at", n_!("Где (first / last / секунда)"), F::Text, "last")],
    stage: false,
};

pub static TRIM: NodeSpec = NodeSpec {
    kind: "trim",
    title: n_!("Обрезка"),
    description: n_!("Отрезок ролика или звука"),
    category: EDIT,
    inputs: &[port("file", n_!("Файл"), P::Media)],
    outputs: &[port("file", n_!("Отрезок"), P::Media)],
    fields: &[field("start", n_!("С, с"), F::Number, "0"), field("end", n_!("По, с (0 — до конца)"), F::Number, "0")],
    stage: false,
};

pub static STILL: NodeSpec = NodeSpec {
    kind: "still",
    title: n_!("Картинка в ролик"),
    description: n_!("Неподвижная картинка на длину звука или заданное время"),
    category: EDIT,
    inputs: &[port("image", n_!("Картинка"), P::Image), port("audio", n_!("Звук"), P::Audio)],
    outputs: &[port("video", n_!("Ролик"), P::Video)],
    fields: &[field("seconds", n_!("Секунд (без звука)"), F::Number, "5")],
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
