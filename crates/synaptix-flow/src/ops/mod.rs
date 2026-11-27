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
pub use data::split_lines;
pub mod media;
pub mod stages;

const GEN: &str = n_!("Генерация");
const EDIT: &str = n_!("Монтаж");
const DATA: &str = n_!("Данные");
const SPEECH: &str = n_!("Речь");

pub static TEXT: NodeSpec = NodeSpec {
    kind: "text",
    title: n_!("Текст"),
    description: n_!("Хранит произвольный текст (идея, стиль, список строк) и передаёт его дальше по проводу"),
    category: DATA,
    inputs: &[],
    outputs: &[port("text", n_!("Текст"), P::Text).hint(n_!("Содержимое поля «Текст»"))],
    fields: &[field("text", n_!("Текст"), F::LongText, "").hint(n_!("Введите текст или несколько строк (по одной на сцену)"))],
    stage: false,
};

pub static FILE: NodeSpec = NodeSpec {
    kind: "file",
    title: n_!("Файл"),
    description: n_!("Загружает картинку, звук или видео с диска и передаёт на вход другой ноды"),
    category: DATA,
    inputs: &[],
    outputs: &[port("file", n_!("Файл"), P::Media).hint(n_!("Выбранный медиафайл (картинка, звук или видео)"))],
    fields: &[field("path", n_!("Путь"), F::File, "").hint(n_!("Выберите файл с диска или вставьте путь"))],
    stage: false,
};

pub static FORMAT: NodeSpec = NodeSpec {
    kind: "format",
    title: n_!("Шаблон текста"),
    description: n_!("Склеивает текст из нескольких входов по шаблону. В тексте шаблона пишите {{a}}, {{b}}, … (или {1}, {2}, …) — на их место подставятся значения с соответствующих входов. Если на вход подан список, шаблон выполнится для каждого элемента."),
    category: DATA,
    inputs: &[
        port("a", n_!("{{a}} Вставка 1"), P::Any).hint(n_!("Подставляется в шаблон вместо {{a}} или {1}")),
        port("b", n_!("{{b}} Вставка 2"), P::Any).hint(n_!("Подставляется в шаблон вместо {{b}} или {2}")),
        port("c", n_!("{{c}} Вставка 3"), P::Any).hint(n_!("Подставляется в шаблон вместо {{c}} или {3}")),
        port("d", n_!("{{d}} Вставка 4"), P::Any).hint(n_!("Подставляется в шаблон вместо {{d}} или {4}")),
        port("e", n_!("{{e}} Вставка 5"), P::Any).hint(n_!("Подставляется в шаблон вместо {{e}} или {5}")),
        port("f", n_!("{{f}} Вставка 6"), P::Any).hint(n_!("Подставляется в шаблон вместо {{f}} или {6}")),
    ],
    outputs: &[port("text", n_!("Готовый текст"), P::Text).hint(n_!("Результат подстановки (или список текстов, если на входе был список)"))],
    fields: &[field("template", n_!("Шаблон"), F::LongText, "{{a}}").hint(n_!("Например: «{{a}}, стиль {{b}}, кинематографичный свет». Нажмите кнопку-чип выше поля, чтобы вставить метку."))],
    stage: false,
};

pub static LINES: NodeSpec = NodeSpec {
    kind: "lines",
    title: n_!("Строки → список"),
    description: n_!("Разбивает многострочный текст на список отдельных элементов (маркеры «-», «1.» убираются). Следующая нода запустится по очереди для каждой строки — удобно для генерации серий сцен."),
    category: DATA,
    inputs: &[port("text", n_!("Многострочный текст"), P::Text).hint(n_!("Текст с переносами строк (например, ответ LLM со списком сцен)"))],
    outputs: &[port("list", n_!("Список строк"), P::List).hint(n_!("Пакет строк: подключите ко входу «Описание» или «Шаблон текста», чтобы прогнать каждую строку отдельно"))],
    fields: &[field("expect", n_!("Ожидается строк"), F::Number, "0").hint(n_!("Если строк получилось другое число — ошибка (0 — не проверять)")).advanced()],
    stage: false,
};

pub static LIST: NodeSpec = NodeSpec {
    kind: "list",
    title: n_!("Список"),
    description: n_!("Собирает до 6 отдельных входов (и строки из поля) в один общий список — например, чтобы подать несколько референсов или клипов одним проводом."),
    category: DATA,
    inputs: &[
        port("in1", n_!("Элемент 1"), P::List).hint(n_!("Первый элемент или подсписок")),
        port("in2", n_!("Элемент 2"), P::List).hint(n_!("Второй элемент или подсписок")),
        port("in3", n_!("Элемент 3"), P::List).hint(n_!("Третий элемент или подсписок")),
        port("in4", n_!("Элемент 4"), P::List).hint(n_!("Четвёртый элемент или подсписок")),
        port("in5", n_!("Элемент 5"), P::List).hint(n_!("Пятый элемент или подсписок")),
        port("in6", n_!("Элемент 6"), P::List).hint(n_!("Шестой элемент или подсписок")),
    ],
    outputs: &[port("list", n_!("Список"), P::List).hint(n_!("Объединённый список всех элементов по порядку"))],
    fields: &[field("items", n_!("Свои строки"), F::LongText, "").hint(n_!("Необязательно: дополнительные элементы по одному на строку (идут перед входами)"))],
    stage: false,
};

pub static LLM: NodeSpec = NodeSpec {
    kind: "llm",
    title: n_!("Языковая модель"),
    description: n_!("Генерирует текст локальной LLM через службу: сценарий по сценам, промпты, стихи для песни, диалоги."),
    category: GEN,
    inputs: &[
        port("prompt", n_!("Запрос"), P::Text).hint(n_!("Основной текст запроса (перекрывает поле «Запрос»)")),
        port("system", n_!("Система"), P::Text).hint(n_!("Системная роль или правила формата ответа")),
    ],
    outputs: &[
        port("text", n_!("Ответ"), P::Text).hint(n_!("Сгенерированный текст ответа")),
        port("thinking", n_!("Размышления"), P::Text).hint(n_!("Цепочка рассуждений (если включены размышления)")),
    ],
    fields: &[
        field("prompt", n_!("Запрос"), F::LongText, "").hint(n_!("Что нужно написать или сгенерировать. Не используется, если подключён вход «Запрос».")),
        field("model", n_!("Модель"), F::Text, "").hint(n_!("Оставьте «по умолчанию», чтобы использовать основную LLM службы")),
        field("system", n_!("Система"), F::LongText, "").hint(n_!("Системная инструкция для модели (тон, язык, ограничения)")).advanced(),
        field("max_tokens", n_!("Макс. токенов"), F::Number, "0").hint(n_!("0 — предел по умолчанию модели")).advanced(),
        field("think", n_!("Размышления"), F::Bool, "false").hint(n_!("Включить режим рассуждений (<think>) для поддерживающих моделей")).advanced(),
        field("context", n_!("Контекст"), F::Number, "0").hint(n_!("Размер окна контекста в токенах (0 — по модели)")).advanced(),
        field("temperature", n_!("Температура"), F::Number, "-1.0").hint(n_!("Креативность ответа: 0.2 — точнее, 0.8 — свободнее (−1 — по модели)")).advanced(),
        field("top_p", "Top-p", F::Number, "-1.0").hint(n_!("Отсечение по вероятности (−1 — по модели)")).advanced(),
        field("top_k", "Top-k", F::Number, "-1").hint(n_!("Число лучших кандидатов (−1 — по модели)")).advanced(),
        field("min_p", "Min-p", F::Number, "-1.0").hint(n_!("Минимальная относительная вероятность (−1 — по модели)")).advanced(),
        field("repeat_penalty", n_!("Штраф повторов"), F::Number, "-1.0").hint(n_!("Снижает повторы фраз (−1 — по модели)")).advanced(),
        field("seed", n_!("Сид"), F::Number, "-1").hint(n_!("Фиксированное зерно случайности (−1 — случайный каждый раз)")).advanced(),
    ],
    stage: false,
};

pub static IMAGE: NodeSpec = NodeSpec {
    kind: "image",
    title: n_!("Картинка"),
    description: n_!("Генерирует картинку (FLUX, Qwen-Image, SDXL) по описанию, с референсами или в режиме img2img (от исходной картинки)."),
    category: GEN,
    inputs: &[
        port("prompt", n_!("Описание"), P::Text).hint(n_!("Текст промпта (перекрывает поле «Описание»)")),
        port("images", n_!("Референсы"), P::List).hint(n_!("Одна картинка или список картинок-образцов для редактирования")),
        port("init", n_!("Исходная"), P::Image).hint(n_!("Базовая картинка для перерисовки (img2img)")),
    ],
    outputs: &[port("image", n_!("Картинка"), P::Image).hint(n_!("Сгенерированное изображение PNG"))],
    fields: &[
        field("prompt", n_!("Описание"), F::LongText, "").hint(n_!("Что должно быть на картинке (лучше на английском для FLUX/SDXL)")),
        field("model", n_!("Модель"), F::Text, "").hint(n_!("Оставьте «по умолчанию» или выберите конкретную модель")),
        field("width", n_!("Ширина"), F::Number, "0").hint(n_!("Ширина в пикселях (0 — размер по умолчанию модели)")),
        field("height", n_!("Высота"), F::Number, "0").hint(n_!("Высота в пикселях (0 — размер по умолчанию модели)")),
        field("negative", n_!("Исключить"), F::Text, "").hint(n_!("Чего избегать на картинке (negative prompt)")).advanced(),
        field("steps", n_!("Шагов"), F::Number, "0").hint(n_!("Число шагов денойза (0 — оптимальное по модели)")).advanced(),
        field("guidance", "CFG", F::Number, "0").hint(n_!("Сила следования описанию (0 — по модели)")).advanced(),
        field("strength", n_!("Сила img2img"), F::Number, "0").hint(n_!("Насколько сильно менять исходную картинку: 0.3 — бережно, 0.8 — радикально")).advanced(),
        field("seed", n_!("Сид"), F::Number, "0").hint(n_!("Зерно генерации (0 — случайное каждый раз)")).advanced(),
    ],
    stage: false,
};

pub const CONTROLS: &[&str] = &["", "union", "canny", "motion-track"];

pub static VIDEO: NodeSpec = NodeSpec {
    kind: "video",
    title: n_!("Видео"),
    description: n_!("Генерирует ролик со звуком (MiniMax-H3 или LTX-2.3). Поддерживает одиночный ролик, оживление первого/последнего кадра, бесшовное продолжение по латентному хвосту (tail → context), пакетную цепочку сцен (галочка «Цепочкой») и клипы под звук."),
    category: GEN,
    inputs: &[
        port("prompt", n_!("Описание"), P::Text).hint(n_!("Описание сцены или список описаний (при списке ролик сгенерируется для каждой строки)")),
        port("context", n_!("Хвост (продолжить)"), P::Latent).hint(n_!("Подключите выход «Хвост» предыдущей ноды Видео — сцена продолжится бесшовно по латенту (видео + звук) без склейки через PNG")),
        port("first_frame", n_!("Первый кадр"), P::Image).hint(n_!("Стартовая картинка (используется, если не подключён «Хвост»)")),
        port("last_frame", n_!("Последний кадр"), P::Image).hint(n_!("Финальный кадр, к которому должна прийти сцена (MiniMax-H3 FL2VA)")),
        port("refs", n_!("Референсы"), P::List).hint(n_!("Картинки, ролики или голос-образцы для MiniMax-H3 Ref2VA")),
        port("audio", n_!("Звук (липсинк)"), P::Audio).hint(n_!("Аудиодорожка, под которую LTX-2.3 строит движение и губы")),
        port("source_video", n_!("Исходный ролик"), P::Video).hint(n_!("Ролик для перерисовки (restyle), пересъёмки отрезка (retake) или продления")),
        port("control_video", n_!("Управляющее видео"), P::Video).hint(n_!("Образец движения/контуров для IC-LoRA или лицо для LipDub (LTX-2.3)")),
    ],
    outputs: &[
        port("video", n_!("Ролик"), P::Video).hint(n_!("Готовый MP4 (или список MP4 при пакетном входе — подключите к «Склейка → Список клипов»)")),
        port("tail", n_!("Хвост (латент)"), P::Latent).hint(n_!("Сжатый хвост видео + звука до декодирования. Подключите ко входу «Хвост» следующей ноды Видео для бесшовного продолжения.")),
    ],
    fields: &[
        field("prompt", n_!("Описание"), F::LongText, "").hint(n_!("Сцена, действие, движение камеры и звук. Не используется, если подключён вход «Описание».")),
        field("model", n_!("Модель"), F::Text, "").hint(n_!("По умолчанию: MiniMax-H3 (для ключевых кадров/сцен) или LTX-2.3 (для звука/IC-LoRA)")),
        field("seconds", n_!("Длительность, с"), F::Number, "0").hint(n_!("Длина ролика в секундах (0 — по умолчанию модели, обычно 4–5 с)")),
        field("chain", n_!("Цепочкой"), F::Bool, "false").hint(n_!("Для списка сцен на входе «Описание»: каждая следующая сцена автоматически продолжается с латентного хвоста предыдущей")),
        field("width", n_!("Ширина"), F::Number, "0").hint(n_!("Ширина кадра (0 — по умолчанию модели, например 1344 для H3)")).advanced(),
        field("height", n_!("Высота"), F::Number, "0").hint(n_!("Высота кадра (0 — по умолчанию модели, например 768 для H3)")).advanced(),
        field("context_frames", n_!("Кадров хвоста"), F::Number, "0").hint(n_!("Сколько латентных кадров сохранять в хвост (0 — оптимум модели: 7 для H3, 3 для LTX)")).advanced(),
        field("negative", n_!("Исключить"), F::Text, "").hint(n_!("Нежелательные артефакты (negative prompt)")).advanced(),
        field("steps", n_!("Шагов"), F::Number, "0").hint(n_!("Число шагов денойза (0 — по модели: 20 для H3, 8 для LTX distilled)")).advanced(),
        field("guidance", "CFG", F::Number, "0").hint(n_!("Сила следования описанию (0 — по модели)")).advanced(),
        field("seed", n_!("Сид"), F::Number, "0").hint(n_!("Зерно генерации (0 — случайное)")).advanced(),
        field("restyle", n_!("Перерисовка"), F::Bool, "false").hint(n_!("Перерисовать «Исходный ролик» в новом стиле через MiniMax-H3")).advanced(),
        field("strength", n_!("Сила правки"), F::Number, "0").hint(n_!("Сила изменения исходного ролика (0.4–0.8)")).advanced(),
        field("extend", n_!("Продление"), F::Bool, "false").hint(n_!("Продлить «Исходный ролик» (LTX-2.3)")).advanced(),
        field("retake_start", n_!("Retake с, с"), F::Number, "0").hint(n_!("Начало перерисовываемого отрезка (LTX-2.3)")).advanced(),
        field("retake_end", n_!("Retake по, с"), F::Number, "0").hint(n_!("Конец перерисовываемого отрезка (LTX-2.3)")).advanced(),
        field("control", "IC-LoRA", F::Choice(CONTROLS), "").hint(n_!("Режим управляющего ролика для LTX-2.3")).advanced(),
        field("two_stage", n_!("Две стадии"), F::Bool, "false").hint(n_!("LTX-2.3: половинная сетка → апскейл ×2 → доводка")).advanced(),
        field("enhance", n_!("Улучшить описание"), F::Bool, "false").hint(n_!("LTX-2.5: короткое описание разворачивается в подробное (улучшитель промпта модели)")).advanced(),
        field("dfr", n_!("DFR (детализация)"), F::Bool, "false").hint(n_!("LTX-2.5: половинное разрешение с ключевыми кадрами, затем полное с LoRA детализации — резче лица, текстуры и надписи; ширина и высота кратны 64")).advanced(),
        field("fps_x2", n_!("48 кадр/с"), F::Bool, "false").hint(n_!("LTX-2.5: удвоить частоту кадров временным апскейлером")).advanced(),
    ],
    stage: false,
};

pub static MUSIC: NodeSpec = NodeSpec {
    kind: "music",
    title: n_!("Музыка"),
    description: n_!("Генерирует инструментальный трек или песню со стихами (ACE-Step, YuE2)."),
    category: GEN,
    inputs: &[
        port("prompt", n_!("Стиль и жанр"), P::Text).hint(n_!("Жанр, инструменты, настроение (по-английски)")),
        port("lyrics", n_!("Текст песни"), P::Text).hint(n_!("Стихи с метками [verse], [chorus] — оставьте пустым для инструментала")),
    ],
    outputs: &[port("audio", n_!("Звук"), P::Audio).hint(n_!("Готовый аудиофайл WAV"))],
    fields: &[
        field("prompt", n_!("Стиль"), F::LongText, "").hint(n_!("Например: warm acoustic indie, gentle piano, uplifting")),
        field("lyrics", n_!("Стихи"), F::LongText, "").hint(n_!("Оставьте пустым для инструментальной музыки")),
        field("seconds", n_!("Длительность, с"), F::Number, "0").hint(n_!("Желаемая длина трека в секундах (0 — по умолчанию)")),
        field("model", n_!("Модель"), F::Text, "").hint(n_!("По умолчанию: ACE-Step для инструментала")).advanced(),
        field("seed", n_!("Сид"), F::Number, "0").hint(n_!("0 — случайное зерно")).advanced(),
    ],
    stage: false,
};

pub static SPEAK: NodeSpec = NodeSpec {
    kind: "speak",
    title: n_!("Речь"),
    description: n_!("Озвучивает текст голосом по умолчанию, по описанию голоса или клонирует голос с аудиообразца."),
    category: GEN,
    inputs: &[
        port("text", n_!("Текст"), P::Text).hint(n_!("Текст для озвучивания")),
        port("voice", n_!("Образец голоса"), P::Audio).hint(n_!("Необязательно: короткая запись голоса для клонирования")),
    ],
    outputs: &[port("audio", n_!("Звук"), P::Audio).hint(n_!("Синтезированная речь WAV"))],
    fields: &[
        field("text", n_!("Текст"), F::LongText, "").hint(n_!("Фраза или дикторский текст")),
        field("voice", n_!("Голос"), F::Text, "").hint(n_!("Имя голоса или описание по-английски (например, warm female voice)")),
        field("language", n_!("Язык"), F::Text, "").hint(n_!("Код языка (ru, en…); пусто — автоопределение")).advanced(),
        field("model", n_!("Модель"), F::Text, "").hint(n_!("Оставьте «по умолчанию» для стандартного TTS службы")).advanced(),
    ],
    stage: false,
};

pub static TRANSCRIBE: NodeSpec = NodeSpec {
    kind: "transcribe",
    title: n_!("Расшифровка"),
    description: n_!("Распознаёт речь из аудио или видеофайла в текст."),
    category: GEN,
    inputs: &[port("file", n_!("Звук или ролик"), P::Media).hint(n_!("Аудио- или видеофайл с речью"))],
    outputs: &[port("text", n_!("Текст"), P::Text).hint(n_!("Распознанный текст"))],
    fields: &[
        field("language", n_!("Язык"), F::Text, "").hint(n_!("Код языка (ru, en…) или пусто для автоопределения")),
        field("model", n_!("Модель"), F::Text, "").hint(n_!("Модель распознавания речи")).advanced(),
    ],
    stage: false,
};

pub static OMNIVOICE: NodeSpec = NodeSpec {
    kind: "omnivoice",
    title: "OmniVoice",
    description: n_!("Речь на 600+ языках: клон голоса с образца, по словесному описанию или голос по умолчанию."),
    category: SPEECH,
    inputs: &[
        port("text", n_!("Текст"), P::Text).hint(n_!("Текст для синтеза")),
        port("ref_audio", n_!("Образец голоса"), P::Audio).hint(n_!("Короткая запись голоса (3–15 с) для клонирования")),
        port("ref_text", n_!("Текст образца"), P::Text).hint(n_!("Точная расшифровка образца (улучшает качество клона)")),
    ],
    outputs: &[port("audio", n_!("Звук"), P::Audio).hint(n_!("Готовая озвучка WAV"))],
    fields: &[
        field("text", n_!("Текст"), F::LongText, "").hint(n_!("Что озвучить")),
        field("instruct", n_!("Описание голоса"), F::Text, "").hint(n_!("Используется без образца (например: calm male narrator)")),
        field("ref_text", n_!("Текст образца"), F::Text, "").hint(n_!("Что говорится в образце голоса")).advanced(),
        field("language", n_!("Язык"), F::Text, "").hint(n_!("Язык речи (пусто — авто)")).advanced(),
        field("model", n_!("Модель"), F::Text, "omnivoice").advanced(),
        field("steps", n_!("Шагов"), F::Number, "32").advanced(),
        field("guidance", "CFG", F::Number, "2.0").advanced(),
        field("t_shift", n_!("Сдвиг времени"), F::Number, "0.1").advanced(),
        field("speed", n_!("Скорость"), F::Number, "1.0").advanced(),
        field("seed", n_!("Сид"), F::Number, "0").hint(n_!("0 — случайный")).advanced(),
    ],
    stage: false,
};

pub static VOXCPM: NodeSpec = NodeSpec {
    kind: "voxcpm",
    title: "VoxCPM2",
    description: n_!("Клонирование тембра и/или естественное продолжение голосовой записи."),
    category: SPEECH,
    inputs: &[
        port("text", n_!("Текст"), P::Text).hint(n_!("Новый текст для озвучивания")),
        port("ref_audio", n_!("Образец тембра"), P::Audio).hint(n_!("Запись для копирования тембра голоса")),
        port("prompt_audio", n_!("Продолжить запись"), P::Audio).hint(n_!("Запись, которую нужно бесшовно продолжить")),
        port("prompt_text", n_!("Текст записи"), P::Text).hint(n_!("Обязательный текст для «Продолжить запись»")),
    ],
    outputs: &[port("audio", n_!("Звук"), P::Audio)],
    fields: &[
        field("text", n_!("Текст"), F::LongText, ""),
        field("prompt_text", n_!("Текст записи"), F::Text, "").advanced(),
        field("model", n_!("Модель"), F::Text, "voxcpm2").advanced(),
        field("guidance", "CFG", F::Number, "2.0").advanced(),
        field("steps", n_!("Шагов"), F::Number, "10").advanced(),
        field("max_len", n_!("Предел токенов"), F::Number, "2000").advanced(),
        field("seed", n_!("Сид"), F::Number, "1988").hint(n_!("0 — случайный")).advanced(),
    ],
    stage: false,
};

pub static VIBEVOICE: NodeSpec = NodeSpec {
    kind: "vibevoice",
    title: "VibeVoice",
    description: n_!("Диалог или подкаст до 4 голосов: пишите реплики вида «Speaker 1: …» и подключите образцы голосов по порядку."),
    category: SPEECH,
    inputs: &[
        port("script", n_!("Сценарий"), P::Text).hint(n_!("Текст диалога со строками «Speaker 1: …», «Speaker 2: …»")),
        port("voice1", n_!("Голос 1"), P::Audio).hint(n_!("Образец для Speaker 1")),
        port("voice2", n_!("Голос 2"), P::Audio).hint(n_!("Образец для Speaker 2")),
        port("voice3", n_!("Голос 3"), P::Audio).hint(n_!("Образец для Speaker 3")),
        port("voice4", n_!("Голос 4"), P::Audio).hint(n_!("Образец для Speaker 4")),
    ],
    outputs: &[port("audio", n_!("Звук"), P::Audio)],
    fields: &[
        field("script", n_!("Сценарий"), F::LongText, ""),
        field("model", n_!("Модель"), F::Text, "vibevoice-1-5b").advanced(),
        field("guidance", "CFG", F::Number, "1.3").advanced(),
        field("steps", n_!("Шагов"), F::Number, "20").advanced(),
        field("max_length_times", n_!("Запас длины"), F::Number, "2.0").advanced(),
        field("seed", n_!("Сид"), F::Number, "0").hint(n_!("0 — случайный")).advanced(),
    ],
    stage: false,
};

pub static DIARIZE: NodeSpec = NodeSpec {
    kind: "diarize",
    title: n_!("Кто говорит"),
    description: n_!("Определяет отрезки речи до 4 говорящих (Sortformer): выдаёт читаемую сводку и JSON таймкодов."),
    category: SPEECH,
    inputs: &[port("audio", n_!("Звук или ролик"), P::Media).hint(n_!("Запись разговора или видео"))],
    outputs: &[
        port("text", n_!("Сводка"), P::Text).hint(n_!("Список отрезков с таймкодами и номерами говорящих")),
        port("json", "JSON", P::Text).hint(n_!("Машиночитаемый JSON сегментов")),
    ],
    fields: &[
        field("threshold", n_!("Порог"), F::Number, "0.5").hint(n_!("Чувствительность детектора речи (0.3–0.7)")),
        field("overlap", n_!("Наложения"), F::Bool, "true").hint(n_!("Учитывать одновременную речь нескольких людей")),
        field("model", n_!("Модель"), F::Text, "").advanced(),
    ],
    stage: false,
};

pub static CONCAT: NodeSpec = NodeSpec {
    kind: "concat",
    title: n_!("Склейка"),
    description: n_!("Склеивает ролики (или аудиодорожки) друг за другом в один файл. Подключите либо один провод в «Список клипов» (например, с выхода пакетной ноды Видео), либо отдельные клипы по порядку в «Клип 1»…«Клип 4». Все ролики автоматически приводятся к размеру первого и 24 к/с."),
    category: EDIT,
    inputs: &[
        port("files", n_!("Список клипов"), P::List).hint(n_!("Пакет роликов или аудио одним проводом (например, с выхода «Ролик» ноды Видео при цепочке сцен)")),
        port("in1", n_!("Клип 1"), P::Media).hint(n_!("Первый ролик или аудиофайл (задаёт разрешение итогового видео)")),
        port("in2", n_!("Клип 2"), P::Media).hint(n_!("Второй ролик или аудиофайл (пойдёт сразу после Клипа 1)")),
        port("in3", n_!("Клип 3"), P::Media).hint(n_!("Третий ролик или аудиофайл")),
        port("in4", n_!("Клип 4"), P::Media).hint(n_!("Четвёртый ролик или аудиофайл")),
    ],
    outputs: &[port("file", n_!("Склеенный файл"), P::Media).hint(n_!("Итоговый MP4 (или WAV для звука) — можно сразу подать в «Видео-плеер»"))],
    fields: &[],
    stage: false,
};

pub const MUX_MODES: &[&str] = &["replace", "mix"];

pub static MUX: NodeSpec = NodeSpec {
    kind: "mux",
    title: n_!("Наложение звука"),
    description: n_!("Накладывает звуковую дорожку (музыку, озвучку) на видео: заменяет исходный звук (replace) или подмешивает поверх (mix)."),
    category: EDIT,
    inputs: &[
        port("video", n_!("Ролик"), P::Video).hint(n_!("Исходное видео")),
        port("audio", n_!("Звук"), P::Audio).hint(n_!("Аудиодорожка (речь, музыка, эффекты)")),
    ],
    outputs: &[port("video", n_!("Ролик со звуком"), P::Video).hint(n_!("Видеофайл MP4 с новой звуковой дорожкой"))],
    fields: &[
        field("mode", n_!("Режим"), F::Choice(MUX_MODES), "replace").hint(n_!("replace — заменить звук ролика; mix — смешать с исходным звуком")),
        field("volume", n_!("Громкость mix"), F::Number, "0.8").hint(n_!("Громкость накладываемого звука в режиме mix (0.2–1.0)")),
    ],
    stage: false,
};

pub static FRAME: NodeSpec = NodeSpec {
    kind: "frame",
    title: n_!("Кадр"),
    description: n_!("Вынимает один кадр (PNG) из видео — первый, последний или на указанной секунде."),
    category: EDIT,
    inputs: &[port("video", n_!("Ролик"), P::Video).hint(n_!("Видео, из которого нужно взять кадр"))],
    outputs: &[port("image", n_!("Кадр PNG"), P::Image).hint(n_!("Извлечённый кадр-картинка"))],
    fields: &[field("at", n_!("Момент"), F::Text, "last").hint(n_!("first — первый кадр, last — последний, или число секунд (например 2.5)"))],
    stage: false,
};

pub static TRIM: NodeSpec = NodeSpec {
    kind: "trim",
    title: n_!("Обрезка"),
    description: n_!("Вырезает отрезок видео или звука по времени в секундах."),
    category: EDIT,
    inputs: &[port("file", n_!("Файл"), P::Media).hint(n_!("Видео или аудио для обрезки"))],
    outputs: &[port("file", n_!("Отрезок"), P::Media).hint(n_!("Обрезанный медиафайл"))],
    fields: &[
        field("start", n_!("Начало, с"), F::Number, "0").hint(n_!("С какой секунды начать")),
        field("end", n_!("Конец, с"), F::Number, "0").hint(n_!("На какой секунде закончить (0 — до конца файла)")),
    ],
    stage: false,
};

pub static STILL: NodeSpec = NodeSpec {
    kind: "still",
    title: n_!("Картинка в ролик"),
    description: n_!("Превращает статичную картинку в видео MP4 заданной длины или ровно под длину подключённого звука."),
    category: EDIT,
    inputs: &[
        port("image", n_!("Картинка"), P::Image).hint(n_!("Изображение для видео")),
        port("audio", n_!("Звук"), P::Audio).hint(n_!("Необязательно: аудиодорожка (задаёт длительность ролика)")),
    ],
    outputs: &[port("video", n_!("Ролик"), P::Video).hint(n_!("Готовый MP4-ролик"))],
    fields: &[field("seconds", n_!("Длительность, с"), F::Number, "5").hint(n_!("Длина ролика в секундах (если звук не подключён)"))],
    stage: false,
};

pub const UPSCALE_FACTORS: &[&str] = &["2", "4"];

pub static VFX_UPSCALE: NodeSpec = NodeSpec {
    kind: "vfx_upscale",
    title: n_!("Нейроувеличение"),
    description: n_!("Увеличивает ролик или картинку в 2 или 4 раза нейросетью synaptix-vfx (PLKSR, SPAN, Compact) покадрово на видеокарте; звук ролика сохраняется. Нужна CUDA и модели в папке vfx библиотеки нейросетей."),
    category: EDIT,
    inputs: &[port("video", n_!("Ролик или картинка"), P::Media).hint(n_!("Видео (MP4 и др.) или изображение для увеличения"))],
    outputs: &[port("video", n_!("Увеличенный"), P::Media).hint(n_!("MP4 (или PNG для картинки) в 2–4 раза больше"))],
    fields: &[
        field("scale", n_!("Во сколько раз"), F::Choice(UPSCALE_FACTORS), "2").hint(n_!("2 или 4; модель другого масштаба доводится до него")),
        field("model", n_!("Модель"), F::Text, "").hint(n_!("Часть имени или вид модели: plksr, span, compact; пусто — первая подходящая")).advanced(),
    ],
    stage: false,
};

pub static VFX_INTERPOLATE: NodeSpec = NodeSpec {
    kind: "vfx_interpolate",
    title: n_!("Плавность RIFE"),
    description: n_!("Дорисовывает промежуточные кадры нейросетью RIFE (synaptix-vfx): поднимает частоту ролика до нужной (например 24 → 60 к/с) или делает плавное замедление; на склейках сцен промежуточные не рисуются. Звук сохраняется (при замедлении — растягивается). Нужна CUDA и модель rife в папке vfx."),
    category: EDIT,
    inputs: &[port("video", n_!("Ролик"), P::Video).hint(n_!("Исходное видео"))],
    outputs: &[port("video", n_!("Плавный ролик"), P::Video).hint(n_!("MP4 с новой частотой кадров"))],
    fields: &[
        field("fps", n_!("Частота, к/с"), F::Number, "60").hint(n_!("Частота кадров результата")),
        field("slowmo", n_!("Замедление, раз"), F::Number, "1").hint(n_!("1 — без замедления; 2 — вдвое медленнее (кадры дорисовываются)")),
    ],
    stage: false,
};

pub static SUBFLOW: NodeSpec = NodeSpec {
    kind: "subflow",
    title: n_!("Подграф"),
    description: n_!("Несколько нод, свёрнутых в одну: внутри — свой граф, снаружи — до 4 входов и 4 выходов. Объединить выделенные ноды — «Объединить в подграф», развернуть обратно — «Разгруппировать»."),
    category: DATA,
    inputs: &[
        port("in1", n_!("Вход 1"), P::Any),
        port("in2", n_!("Вход 2"), P::Any),
        port("in3", n_!("Вход 3"), P::Any),
        port("in4", n_!("Вход 4"), P::Any),
    ],
    outputs: &[
        port("out1", n_!("Выход 1"), P::Any),
        port("out2", n_!("Выход 2"), P::Any),
        port("out3", n_!("Выход 3"), P::Any),
        port("out4", n_!("Выход 4"), P::Any),
    ],
    fields: &[
        field("title", n_!("Название"), F::Text, "Подграф").hint(n_!("Имя подграфа в шапке ноды")),
        field("subflow", n_!("Внутренний граф"), F::LongText, "").hidden(),
    ],
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
    // нейрофильтры synaptix-vfx: исполнитель — в службе (CUDA)
    (&VFX_UPSCALE, None),
    (&VFX_INTERPOLATE, None),
    (&SUBFLOW, Some(crate::run::subflow)),
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
