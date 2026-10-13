//! Виды нод: данные (текст, файл, шаблон, строки), генерация через службу
//! (LLM, картинка, видео, музыка, речь, расшифровка) и монтаж ffmpeg
//! (склейка, наложение звука, кадр, обрезка, картинка в ролик).

use crate::run::OpFn;
use crate::schema::{field, port, FieldType as F, NodeSpec, PortType as P};

mod data;
pub mod media;
mod service;

const GEN: &str = "Генерация";
const EDIT: &str = "Монтаж";
const DATA: &str = "Данные";

pub static TEXT: NodeSpec = NodeSpec {
    kind: "text",
    title: "Текст",
    description: "Постоянный текст",
    category: DATA,
    inputs: &[],
    outputs: &[port("text", "Текст", P::Text)],
    fields: &[field("text", "Текст", F::LongText, "")],
};

pub static FILE: NodeSpec = NodeSpec {
    kind: "file",
    title: "Файл",
    description: "Картинка, звук или ролик с диска",
    category: DATA,
    inputs: &[],
    outputs: &[port("file", "Файл", P::Media)],
    fields: &[field("path", "Путь", F::File, "")],
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
};

pub static LINES: NodeSpec = NodeSpec {
    kind: "lines",
    title: "Строки",
    description: "Текст → список непустых строк (без маркеров списка) — пакет для следующих нод",
    category: DATA,
    inputs: &[port("text", "Текст", P::Text)],
    outputs: &[port("list", "Список", P::List)],
    fields: &[],
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
};

pub static LLM: NodeSpec = NodeSpec {
    kind: "llm",
    title: "Языковая модель",
    description: "Ответ языковой модели службы на запрос",
    category: GEN,
    inputs: &[port("prompt", "Запрос", P::Text), port("system", "Система", P::Text)],
    outputs: &[port("text", "Ответ", P::Text)],
    fields: &[
        field("prompt", "Запрос", F::LongText, ""),
        field("system", "Система", F::LongText, ""),
        field("model", "Модель", F::Text, ""),
        field("max_tokens", "Токенов", F::Number, "0"),
    ],
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
    ],
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
};

pub static TRANSCRIBE: NodeSpec = NodeSpec {
    kind: "transcribe",
    title: "Расшифровка",
    description: "Речь из звука или ролика в текст",
    category: GEN,
    inputs: &[port("file", "Звук или ролик", P::Media)],
    outputs: &[port("text", "Текст", P::Text)],
    fields: &[field("language", "Язык", F::Text, "")],
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
};

pub static FRAME: NodeSpec = NodeSpec {
    kind: "frame",
    title: "Кадр",
    description: "Кадр ролика: первый, последний или на секунде",
    category: EDIT,
    inputs: &[port("video", "Ролик", P::Video)],
    outputs: &[port("image", "Кадр", P::Image)],
    fields: &[field("at", "Где (first / last / секунда)", F::Text, "last")],
};

pub static TRIM: NodeSpec = NodeSpec {
    kind: "trim",
    title: "Обрезка",
    description: "Отрезок ролика или звука",
    category: EDIT,
    inputs: &[port("file", "Файл", P::Media)],
    outputs: &[port("file", "Отрезок", P::Media)],
    fields: &[field("start", "С, с", F::Number, "0"), field("end", "По, с (0 — до конца)", F::Number, "0")],
};

pub static STILL: NodeSpec = NodeSpec {
    kind: "still",
    title: "Картинка в ролик",
    description: "Неподвижная картинка на длину звука или заданное время",
    category: EDIT,
    inputs: &[port("image", "Картинка", P::Image), port("audio", "Звук", P::Audio)],
    outputs: &[port("video", "Ролик", P::Video)],
    fields: &[field("seconds", "Секунд (без звука)", F::Number, "5")],
};

static ALL: &[(&NodeSpec, OpFn)] = &[
    (&TEXT, data::text),
    (&FILE, data::file),
    (&FORMAT, data::format),
    (&LINES, data::lines),
    (&LIST, data::list),
    (&LLM, service::llm),
    (&IMAGE, service::image),
    (&VIDEO, service::video),
    (&MUSIC, service::music),
    (&SPEAK, service::speak),
    (&TRANSCRIBE, service::transcribe),
    (&CONCAT, media::concat),
    (&MUX, media::mux),
    (&FRAME, media::frame),
    (&TRIM, media::trim),
    (&STILL, media::still),
];

pub fn lookup(kind: &str) -> Option<(&'static NodeSpec, OpFn)> {
    ALL.iter().find(|(s, _)| s.kind == kind).map(|&(s, f)| (s, f))
}

pub fn spec(kind: &str) -> Option<&'static NodeSpec> {
    lookup(kind).map(|(s, _)| s)
}

/// Все виды нод.
pub fn specs() -> impl Iterator<Item = &'static NodeSpec> {
    ALL.iter().map(|(s, _)| *s)
}

pub fn kinds() -> Vec<&'static str> {
    specs().map(|s| s.kind).collect()
}
