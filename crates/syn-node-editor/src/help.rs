//! Справка и красивые rich-tooltip блоки для карточек нод, портов и полей.
//!
//! Источник текстов:
//! - для нод ядра `synaptix-flow` — описания и подсказки напрямую из `NodeSpec`
//!   плюс понятные советы «как подключить» для ключевых нод;
//! - для встроенных (родных) нод редактора — короткие человеческие описания.

use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::{Column, DecoratedBox, Padding, Row, Tooltip, TooltipPosition};

use crate::registry::NodeKindMeta;
use crate::types::{NodeKind, PortKind, PortSchema, PortSide};

pub struct NodeHelp {
    pub title: String,
    pub badge: String,
    pub description: String,
    pub tip: Option<String>,
}

fn port_type_badge(kind: NodeKind, side: PortSide, schema: PortSchema) -> &'static str {
    if let Some(fk) = kind.flow_kind() {
        if let Some(spec) = synaptix_flow::ops::spec(fk) {
            let list = match side {
                PortSide::Input => spec.inputs,
                PortSide::Output => spec.outputs,
            };
            if let Some(p) = list.iter().find(|p| p.name == schema.name) {
                return match p.ty {
                    synaptix_flow::PortType::Text => "Текст",
                    synaptix_flow::PortType::Number => "Число",
                    synaptix_flow::PortType::Image => "Картинка",
                    synaptix_flow::PortType::Audio => "Звук",
                    synaptix_flow::PortType::Video => "Видео",
                    synaptix_flow::PortType::Media => "Медиа",
                    synaptix_flow::PortType::List => "Список",
                    synaptix_flow::PortType::Any => "Любой",
                    synaptix_flow::PortType::Model => "Модель",
                    synaptix_flow::PortType::Cond => "Условие",
                    synaptix_flow::PortType::Latent => "Латент",
                };
            }
        }
    }
    match schema.kind {
        PortKind::Text => "Текст",
        PortKind::Image => "Картинка",
        PortKind::Audio => "Звук",
        PortKind::Video => "Видео",
        PortKind::Control => "Управление",
        PortKind::Data => "Данные",
        PortKind::Custom(name) => name,
    }
}

fn flow_tip(fk: &str) -> Option<&'static str> {
    match fk {
        "format" => Some(
            "Пример: подключите сцену в «{{a}} Вставка 1», стиль в «{{b}} Вставка 2» и напишите в шаблоне «{{a}}, стиль {{b}}». Также работают {1}…{6}.",
        ),
        "concat" => Some(
            "Два способа склейки:\n• один провод в «Список клипов» (с пакетной ноды Видео);\n• отдельные ролики в «Клип 1», «Клип 2», «Клип 3», «Клип 4» по порядку.",
        ),
        "lines" => Some(
            "Подключите ответ LLM со списком сцен → выход «Список строк» подключите к «Видео → Описание» (или к «Шаблон текста → {{a}}»), и каждая строка сгенерирует свой ролик.",
        ),
        "list" => Some(
            "Соберите несколько картинок/клипов в один список, чтобы подать их на порт «Референсы» или «Список клипов».",
        ),
        "video" => Some(
            "Для бесшовного продолжения: соедините выход «Хвост (латент)» первой сцены со входом «Хвост (продолжить)» второй, либо подайте список строк на «Описание» и включите «Цепочкой».",
        ),
        "mux" => Some(
            "Режим replace полностью заменяет звук видео; mix смешивает исходный звук ролика с подключённой музыкой или речью.",
        ),
        "llm" => Some(
            "Попросите модель выдать по одной сцене на строку без нумерации — тогда нода «Строки → список» сразу превратит ответ в пакет сцен для Видео.",
        ),
        "h3_sample" => Some(
            "Выход «Хвост (латент)» соедините со входом «Хвост (продолжить)» следующего «Кодировщика H3» — получите продолжение сцены без потери света и движения.",
        ),
        _ => None,
    }
}

fn native_help(kind: NodeKind) -> (&'static str, Option<&'static str>) {
    match kind {
        NodeKind::FfmpegPlayer => (
            "Встроенный видеоплеер на холсте: воспроизводит ролик с выхода «Видео», «Склейка» или «Наложение звука».",
            Some("Подключите выход «Склеенный файл» или «Ролик» ко входу плеера, чтобы смотреть результат прямо в графе."),
        ),
        NodeKind::TextView => (
            "Текстовое поле и просмотрщик: можно ввести промпт вручную или подключить вход, чтобы видеть ответ LLM.",
            Some("ПКМ → «Изменить размер», чтобы сделать окно шире или выше."),
        ),
        NodeKind::MarkdownView => (
            "Заметка-шпаргалка в формате Markdown на холсте (описание графа, подсказки, ссылки).",
            Some("ПКМ → «Редактировать» для правки текста."),
        ),
        NodeKind::ImageLoad => (
            "Загружает картинку с диска (PNG/JPEG/WebP) и отдаёт её в граф (например, как «Первый кадр» или референс).",
            None,
        ),
        NodeKind::ImageSave => (
            "Сохраняет картинку с входа в файл на диске.",
            None,
        ),
        NodeKind::AudioFile => ("Загружает аудиофайл с диска (WAV/MP3/FLAC/OGG).", None),
        NodeKind::AudioPlayer => ("Воспроизводит аудиосигнал с волновой формой прямо в графе.", None),
        NodeKind::AudioRecorder => ("Записывает голос с микрофона для клонирования или обработки.", None),
        NodeKind::SaveToFile => ("Сохраняет аудиопоток в файл WAV на диске.", None),
        NodeKind::Mixer => ("Смешивает несколько звуковых дорожек с индивидуальной громкостью.", None),
        NodeKind::Number => ("Задаёт числовую константу.", None),
        NodeKind::Add => ("Складывает два числа (a + b).", None),
        NodeKind::Output => ("Показывает итоговое числовое или текстовое значение.", None),
        NodeKind::Ext(ext) => match ext.0 {
            "ve_project_media" => (
                "Отдаёт список всех медиафайлов, импортированных в текущий проект видеоредактора.",
                Some("Подключите выход «файлы» в «Вырезать паузы», «Субтитры из речи», «Нейроувеличение» или «Склейку»."),
            ),
            "ve_to_timeline" => (
                "Импортирует полученные ролики/файлы в медиатеку проекта и автоматически ставит их на таймлайн друг за другом.",
                Some("Подключите сюда выход «Склеенный файл» ноды Склейка или «Ролик» ноды Видео."),
            ),
            "ve_cut_silence" => (
                "Находит и вырезает тихие паузы в клипах проекта по порогу громкости (дБ) и минимальной длительности.",
                Some("Подключите «Медиатека проекта» → «файлы» и нажмите Запуск."),
            ),
            "ve_subtitles" => (
                "Распознаёт речь и создаёт дорожку субтитров на таймлайне проекта.",
                Some("Подключите «Медиатека проекта» → «файлы» и задайте длину фразы."),
            ),
            "ve_vfx_upscale" => (
                "Нейросетевое увеличение разрешения видео (×2 или ×4) на видеокарте и добавление результата в проект.",
                Some("Подключите видеофайл или «Медиатека проекта» → выход «файлы» можно подать в «Плавность RIFE» или «На таймлайн»."),
            ),
            "ve_vfx_interpolate" => (
                "Нейросетевая интерполяция кадров RIFE (например, до 60 к/с) для плавного движения.",
                Some("Подключите ролик на вход «файлы», а выход «файлы» — в «На таймлайн»."),
            ),
            "ve_export" => (
                "Рендерит текущий таймлайн проекта в готовый MP4-файл.",
                Some("Если путь пуст, сохранит в ~/Videos/<имя проекта>.mp4."),
            ),
            _ => ("Узел обработки данных графа.", None),
        },
        _ => ("Узел обработки данных графа.", None),
    }
}

pub fn node_help(meta: &'static NodeKindMeta) -> NodeHelp {
    let title = crate::i18n::node_title(meta);
    if let Some(fk) = meta.kind.flow_kind() {
        if let Some(spec) = synaptix_flow::ops::spec(fk) {
            return NodeHelp {
                title,
                badge: syngui::i18n::t(spec.category),
                description: syngui::i18n::t(spec.description),
                tip: flow_tip(fk).map(String::from),
            };
        }
    }
    let (desc, tip) = native_help(meta.kind);
    NodeHelp {
        title,
        badge: crate::i18n::node_category_label(meta.category),
        description: desc.to_string(),
        tip: tip.map(String::from),
    }
}

pub fn port_hint(kind: NodeKind, side: PortSide, schema: PortSchema) -> String {
    if let Some(fk) = kind.flow_kind() {
        if let Some(spec) = synaptix_flow::ops::spec(fk) {
            let list = match side {
                PortSide::Input => spec.inputs,
                PortSide::Output => spec.outputs,
            };
            if let Some(p) = list.iter().find(|p| p.name == schema.name) {
                if !p.hint.is_empty() {
                    return syngui::i18n::t(p.hint);
                }
            }
        }
    }
    match side {
        PortSide::Input => format!("Входной порт ({})", port_type_badge(kind, side, schema)),
        PortSide::Output => format!("Выходной порт ({})", port_type_badge(kind, side, schema)),
    }
}

/// Красивая карточка справки для кнопки `?` в шапке ноды.
pub fn node_tooltip_content(meta: &'static NodeKindMeta) -> impl Widget {
    let info = node_help(meta);
    let header = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .children(vec![
            Box::new(
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(vec![
                        Box::new(Text::new(meta.icon).class("ne-tip-icon")) as Box<dyn Widget>,
                        Box::new(Text::new(info.title).class("ne-tip-title")) as Box<dyn Widget>,
                    ]),
            ) as Box<dyn Widget>,
            Box::new(
                DecoratedBox::new()
                    .child(Padding::symmetric(6.0, 2.0).child(Text::new(info.badge).class("ne-tip-badge-text")))
                    .class("ne-tip-badge"),
            ) as Box<dyn Widget>,
        ]);

    let mut children: Vec<Box<dyn Widget>> = vec![
        Box::new(header),
        Box::new(Text::new(info.description).class("ne-tip-desc")),
    ];
    if let Some(tip) = info.tip {
        let tip_box = DecoratedBox::new()
            .child(
                Padding::symmetric(8.0, 6.0).child(
                    Column::new()
                        .gap(2.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .children(vec![
                            Box::new(Text::new("Как подключить").class("ne-tip-callout-title")) as Box<dyn Widget>,
                            Box::new(Text::new(tip).class("ne-tip-callout-text")) as Box<dyn Widget>,
                        ]),
                ),
            )
            .class("ne-tip-callout");
        children.push(Box::new(tip_box));
    }

    Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("ne-tip-body")
        .children(children)
}

/// Красивая карточка справки при наведении на порт.
pub fn port_tooltip_content(kind: NodeKind, side: PortSide, schema: PortSchema) -> impl Widget {
    let label = syngui::i18n::t(schema.label);
    let badge = port_type_badge(kind, side, schema);
    let side_txt = match side {
        PortSide::Input => "Вход",
        PortSide::Output => "Выход",
    };
    let hint = port_hint(kind, side, schema);

    let header = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .children(vec![
            Box::new(Text::new(label).class("ne-tip-title")) as Box<dyn Widget>,
            Box::new(
                DecoratedBox::new()
                    .child(Padding::symmetric(6.0, 2.0).child(Text::new(format!("{side_txt} · {badge}")).class("ne-tip-badge-text")))
                    .class("ne-tip-badge"),
            ) as Box<dyn Widget>,
        ]);

    Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("ne-tip-body-compact")
        .children(vec![
            Box::new(header) as Box<dyn Widget>,
            Box::new(Text::new(hint).class("ne-tip-desc")),
        ])
}

/// Красивая карточка справки при наведении на поле ноды.
pub fn field_tooltip_content(label: &str, hint: &str, default: &str) -> impl Widget {
    let mut children: Vec<Box<dyn Widget>> = vec![
        Box::new(Text::new(label.to_string()).class("ne-tip-title")),
        Box::new(Text::new(hint.to_string()).class("ne-tip-desc")),
    ];
    if !default.is_empty() {
        children.push(Box::new(
            Text::new(format!("По умолчанию: {default}")).class("ne-tip-meta"),
        ));
    }
    Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("ne-tip-body-compact")
        .children(children)
}

/// Обернуть виджет в стилизованный rich-tooltip.
pub fn with_rich_tooltip<C: Widget + 'static, T: Widget + 'static>(
    child: C,
    content: T,
    pos: TooltipPosition,
    max_width: f32,
) -> impl Widget {
    Tooltip::rich(child, content)
        .position(pos)
        .delay_ms(200)
        .max_width(max_width)
        .class("ne-rich-tooltip")
}
