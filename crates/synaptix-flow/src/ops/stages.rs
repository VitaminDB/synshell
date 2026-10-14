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
//! декодеры видео и звука.

use crate::schema::{field, port, FieldType as F, NodeSpec, PortType as P};

const IMG: &str = "Модели картинок";
const H3: &str = "MiniMax-H3";

/// Квант трансформера: пусто — из Параметров службы.
pub const QUANTS: &[&str] = &["", "nvfp4", "mxfp8", "bf16"];
/// Как вписать картинку в размер латента.
pub const FITS: &[&str] = &["cover", "stretch"];

pub static IMAGE_MODEL: NodeSpec = NodeSpec {
    kind: "image_model",
    title: "Модель картинок",
    description: "FLUX.1, FLUX.2, Qwen-Image, Qwen-Image 2.1 или SDXL из библиотеки службы (пусто — модель по умолчанию)",
    category: IMG,
    inputs: &[],
    outputs: &[port("model", "Модель", P::Model)],
    fields: &[field("model", "Модель", F::Text, ""), field("quant", "Квант", F::Choice(QUANTS), "")],
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
    description: "MiniMax-H3 из библиотеки службы: FL2VA (описание, ключевые кадры) или Ref2VA (референсы)",
    category: H3,
    inputs: &[],
    outputs: &[port("model", "Модель", P::Model)],
    fields: &[field("model", "Модель", F::Text, ""), field("quant", "Квант", F::Choice(QUANTS), "")],
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
    fields: &[field("prompt", "Описание", F::LongText, ""), field("negative", "Без", F::Text, "")],
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
];
