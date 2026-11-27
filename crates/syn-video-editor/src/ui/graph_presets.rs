//! Библиотека готовых графов видеоредактора (`syn-video-editor`, вкладка «Графы»):
//! 25 проверенных заготовок по 4 разделам (Видео и сцены, Монтаж и VFX,
//! Картинки, Музыка и речь) — каждая с чистой раскладкой без наложений,
//! памяткой Markdown и выводом прямо «На таймлайн».

use std::collections::BTreeMap;

use syn_node_editor::model::{
    ConnData, FieldValueData, MarkdownViewStateData, NodeData, NodeStateData, NodeStyleData, PointData, Template,
    TemplateKind,
};
use syn_node_editor::types::{ExtKind, NodeKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetCat {
    Video,
    Montage,
    Image,
    Audio,
}

impl PresetCat {
    pub const ALL: &'static [(PresetCat, &'static str)] = &[
        (PresetCat::Video, "Видео и сцены"),
        (PresetCat::Montage, "Монтаж и VFX"),
        (PresetCat::Image, "Картинки"),
        (PresetCat::Audio, "Музыка и речь"),
    ];

    pub fn label(self) -> &'static str {
        Self::ALL.iter().find(|(c, _)| *c == self).map(|(_, l)| *l).unwrap_or("")
    }
}

pub struct GraphPreset {
    pub id: &'static str,
    pub cat: PresetCat,
    pub title: &'static str,
    pub chain: &'static str,
    pub description: &'static str,
    pub badge: &'static str,
    pub build: fn() -> Template,
}

pub fn all() -> Vec<GraphPreset> {
    vec![
        // ── Видео и сцены (MiniMax-H3 и LTX-2.3) ─────────────────────────
        GraphPreset {
            id: "ve-h3-chain-timeline",
            cat: PresetCat::Video,
            title: "История из 2 сцен (H3, хвост)",
            chain: "картинка → видео 1 → хвост → видео 2 → склейка → таймлайн",
            description: "Бесшовное продолжение по латенту (tail → context) на MiniMax-H3, склейка двух сцен и автопостановка на таймлайн.",
            badge: "MiniMax-H3 · 2 сцены",
            build: h3_chain_timeline,
        },
        GraphPreset {
            id: "ve-story-film-timeline",
            cat: PresetCat::Video,
            title: "Авто-фильм по идее (LLM → H3)",
            chain: "идея + стиль → LLM → строки → видео× → склейка → таймлайн",
            description: "Задайте идею и стиль: LLM разобьёт историю на сцены, Видео снимет их цепочкой по хвосту, Склейка соберёт фильм на таймлайн.",
            badge: "Идея → фильм",
            build: story_film_timeline,
        },
        GraphPreset {
            id: "ve-ltx-chain-timeline",
            cat: PresetCat::Video,
            title: "Быстрая цепочка 2 сцен (LTX-2.3)",
            chain: "видео 1 (LTX) → хвост → видео 2 → склейка → таймлайн",
            description: "Быстрая 2-сценная цепочка на LTX-2.3 Distilled (8 шагов) с латентным продолжением и склейкой на таймлайн.",
            badge: "LTX-2.3 · быстро",
            build: ltx_chain_timeline,
        },
        GraphPreset {
            id: "ve-h3-text-to-video",
            cat: PresetCat::Video,
            title: "H3: Текст → Видео со звуком",
            chain: "описание → видео MiniMax-H3 → таймлайн",
            description: "Одиночный кинематографичный ролик со звуком (речь, шумы, музыка сцены) по текстовому описанию.",
            badge: "MiniMax-H3 · T2V",
            build: h3_text_to_video,
        },
        GraphPreset {
            id: "ve-h3-first-frame",
            cat: PresetCat::Video,
            title: "H3: Оживить картинку (1-й кадр)",
            chain: "файл картинки → видео H3 (первый кадр) → таймлайн",
            description: "Анимация фотографии или рендера: выбранная картинка становится первым кадром видео со звуком.",
            badge: "MiniMax-H3 · I2V",
            build: h3_first_frame,
        },
        GraphPreset {
            id: "ve-h3-first-last",
            cat: PresetCat::Video,
            title: "H3: Переход Первый → Последний кадр",
            chain: "кадр А + кадр Б → видео H3 (FL2VA) → таймлайн",
            description: "Плавный морфинг и пролёт камеры точно от начальной картинки к конечной (First+Last frame).",
            badge: "MiniMax-H3 · FL2VA",
            build: h3_first_last,
        },
        GraphPreset {
            id: "ve-h3-staged-chain",
            cat: PresetCat::Video,
            title: "H3: Стадийная цепочка (tail → context)",
            chain: "модель → encode → sample → tail → сцена 2 → декодеры → склейка",
            description: "Полный контроль над стадиями MiniMax-H3 (кэш энкодера, сид, шаги, ручная передача латентного хвоста).",
            badge: "MiniMax-H3 · стадии",
            build: h3_staged_chain,
        },
        GraphPreset {
            id: "ve-h3-restyle",
            cat: PresetCat::Video,
            title: "H3: Перерисовка видео в новом стиле",
            chain: "исходный ролик → видео H3 (restyle) → таймлайн",
            description: "Сохраняет движение и композицию исходного видео, перерисовывая его в новом стиле (аниме, 3D, кино).",
            badge: "MiniMax-H3 · Restyle",
            build: h3_restyle,
        },
        GraphPreset {
            id: "ve-ltx-retake",
            cat: PresetCat::Video,
            title: "LTX-2.3: Пересъёмка отрезка (Retake)",
            chain: "исходный ролик → видео LTX (retake с/по) → таймлайн",
            description: "Перегенерирует только заданный отрезок времени внутри готового ролика, оставляя остальное нетронутым.",
            badge: "LTX-2.3 · Retake",
            build: ltx_retake,
        },
        GraphPreset {
            id: "ve-ltx-ic-lora",
            cat: PresetCat::Video,
            title: "LTX-2.3: Видео по контурам/движению",
            chain: "управляющее видео → видео LTX (IC-LoRA canny) → таймлайн",
            description: "Переносит структуру и движение из образца видео в новую сцену через IC-LoRA (canny / union / motion-track).",
            badge: "LTX-2.3 · IC-LoRA",
            build: ltx_ic_lora,
        },
        GraphPreset {
            id: "ve-ltx-lipdub",
            cat: PresetCat::Video,
            title: "LTX-2.3: Переозвучка и липсинк (LipDub)",
            chain: "ролик + новая речь → видео LTX (LipDub) → таймлайн",
            description: "Озвучивает новый текст (например перевод) и синхронизирует движение губ человека в исходном видео.",
            badge: "LTX-2.3 · LipDub",
            build: ltx_lipdub,
        },
        GraphPreset {
            id: "ve-ltx-a2v",
            cat: PresetCat::Video,
            title: "LTX-2.3: Видео в ритм звука/музыки",
            chain: "картинка + звук → видео LTX (по звуку) → таймлайн",
            description: "Строит движение кадра и камеры синхронно с подключённой музыкой или голосом.",
            badge: "LTX-2.3 · Audio→Video",
            build: ltx_a2v,
        },
        // ── Монтаж и VFX ─────────────────────────────────────────────────
        GraphPreset {
            id: "ve-montage-music-timeline",
            cat: PresetCat::Montage,
            title: "Склейка клипов + фоновая музыка",
            chain: "файл 1 + файл 2 → склейка + музыка → наложение (mix) → таймлайн",
            description: "Склеивает выбранные ролики по порядку, подмешивает сгенерированную фоновую музыку и кладёт результат на таймлайн.",
            badge: "Монтаж + звук",
            build: montage_music_timeline,
        },
        GraphPreset {
            id: "ve-auto-edit-project",
            cat: PresetCat::Montage,
            title: "Автомонтаж: паузы + субтитры",
            chain: "медиатека проекта → вырезать паузы + субтитры из речи",
            description: "Берёт файлы текущего проекта, вырезает тишину на клипах и автоматически строит дорожку субтитров по речи.",
            badge: "Проект · монтаж",
            build: auto_edit_project,
        },
        GraphPreset {
            id: "ve-vfx-upscale-rife",
            cat: PresetCat::Montage,
            title: "Апскейл ×2 + 60 к/с (RIFE)",
            chain: "файл → нейроувеличение ×2 → плавность 60 fps → таймлайн",
            description: "Увеличивает разрешение ролика нейрофильтром, достраивает кадры до 60 к/с через RIFE и добавляет на таймлайн.",
            badge: "Нейрофильтры VFX",
            build: vfx_upscale_rife,
        },
        GraphPreset {
            id: "ve-trim-continue",
            cat: PresetCat::Montage,
            title: "Обрезать ролик и дорисовать финал",
            chain: "файл → обрезка → последний кадр → видео H3 → склейка → таймлайн",
            description: "Обрезает готовый ролик в нужной точке, берёт последний кадр, генерирует продолжение и склеивает в один файл.",
            badge: "Обрезка + продолжение",
            build: trim_and_continue,
        },
        GraphPreset {
            id: "ve-still-voiceover",
            cat: PresetCat::Montage,
            title: "Картинка + озвучка → ролик",
            chain: "картинка + речь → картинка в ролик → таймлайн",
            description: "Озвучивает дикторский текст и превращает статичную иллюстрацию/слайд в видео точно под длину речи.",
            badge: "Слайд + диктор",
            build: still_voiceover,
        },
        // ── Картинки и обложки ───────────────────────────────────────────
        GraphPreset {
            id: "ve-cover-llm-image",
            cat: PresetCat::Image,
            title: "Стартовый кадр / обложка через LLM",
            chain: "идея → LLM (английский промпт) → картинка 1344×768 → таймлайн",
            description: "Пишете идею по-русски — LLM разворачивает её в детальный кинопромпт, а модель картинок рисует кадр.",
            badge: "LLM → Картинка",
            build: cover_llm_image,
        },
        GraphPreset {
            id: "ve-image-refs-edit",
            cat: PresetCat::Image,
            title: "Правка кадра по референсам",
            chain: "фото 1 + фото 2 → список → картинка (референсы) → таймлайн",
            description: "Переносит персонажа или объект с референсных картинок в новую сцену (Qwen-Image 2.1 / FLUX.2).",
            badge: "Референсы · правка",
            build: image_refs_edit,
        },
        GraphPreset {
            id: "ve-video-frame-stylize",
            cat: PresetCat::Image,
            title: "Стилизация кадра из видео (img2img)",
            chain: "видео → кадр → картинка (img2img) → таймлайн",
            description: "Вынимает кадр из ролика и перерисовывает его в новом художественном стиле для заставки или перебивки.",
            badge: "Кадр → img2img",
            build: video_frame_stylize,
        },
        // ── Музыка, речь и аватары ───────────────────────────────────────
        GraphPreset {
            id: "ve-talking-avatar-timeline",
            cat: PresetCat::Audio,
            title: "Говорящий аватар с фото",
            chain: "фото + речь → список → видео H3 Ref2VA → звук → таймлайн",
            description: "Оживляет портретное фото под озвученный текст с синхронными губами, заменяет звук чистой речью и ставит на таймлайн.",
            badge: "Аватар · липсинк",
            build: talking_avatar_timeline,
        },
        GraphPreset {
            id: "ve-music-video-timeline",
            cat: PresetCat::Audio,
            title: "Клип под песню (LLM → Музыка → Видео)",
            chain: "тема → стихи + обложка → песня → видео LTX под звук → таймлайн",
            description: "Сочиняет текст песни, генерирует трек и обложку, а затем оживляет видео под ритм музыки.",
            badge: "Песня + клип",
            build: music_video_timeline,
        },
        GraphPreset {
            id: "ve-bgm-acestep",
            cat: PresetCat::Audio,
            title: "Фоновая музыка для монтажа (ACE-Step)",
            chain: "стиль → музыка ACE-Step → таймлайн",
            description: "Генерирует инструментальный саундтрек нужной длины и сразу добавляет на аудиодорожку таймлайна.",
            badge: "ACE-Step · музыка",
            build: bgm_acestep,
        },
        GraphPreset {
            id: "ve-voice-clone-omnivoice",
            cat: PresetCat::Audio,
            title: "Клон голоса / озвучка (OmniVoice)",
            chain: "образец голоса + текст → OmniVoice → таймлайн",
            description: "Клонирует голос с короткого образца (или по описанию) и кладёт готовую дикторскую дорожку на таймлайн.",
            badge: "OmniVoice · голос",
            build: voice_clone_omnivoice,
        },
        GraphPreset {
            id: "ve-podcast-vibevoice",
            cat: PresetCat::Audio,
            title: "Подкаст / диалог на 2 голоса",
            chain: "тема → LLM диалог → VibeVoice → таймлайн",
            description: "LLM пишет диалог «Speaker 1 / Speaker 2», VibeVoice озвучивает его двумя голосами прямо на таймлайн.",
            badge: "VibeVoice · диалог",
            build: podcast_vibevoice,
        },
    ]
}

fn f_text(s: &str) -> FieldValueData {
    FieldValueData::Text(s.to_string())
}
fn f_num(v: f32) -> FieldValueData {
    FieldValueData::Float(v)
}
fn f_bool(v: bool) -> FieldValueData {
    FieldValueData::Bool(v)
}
fn f_choice(i: usize) -> FieldValueData {
    FieldValueData::Choice(i)
}

fn node(id: u64, kind: NodeKind, x: f32, y: f32, tint: Option<&str>, fields: &[(&str, FieldValueData)]) -> NodeData {
    let mut map = BTreeMap::new();
    for (k, v) in fields {
        map.insert((*k).to_string(), v.clone());
    }
    NodeData {
        id,
        kind,
        pos: PointData { x, y },
        fields: map,
        style: NodeStyleData { tint: tint.map(String::from), shadow: true },
        enabled: true,
        state: None,
    }
}

fn note(id: u64, x: f32, y: f32, w: f32, h: f32, md: &str) -> NodeData {
    NodeData {
        id,
        kind: NodeKind::MarkdownView,
        pos: PointData { x, y },
        fields: BTreeMap::new(),
        style: NodeStyleData { tint: Some("#3B82F6".into()), shadow: true },
        enabled: true,
        state: Some(NodeStateData::MarkdownView(MarkdownViewStateData {
            content: md.to_string(),
            width: w,
            height: h,
            edit_mode: false,
        })),
    }
}

fn conn(from_node: u64, from_port: &str, to_node: u64, to_port: &str) -> ConnData {
    ConnData {
        from_node,
        from_port: from_port.to_string(),
        to_node,
        to_port: to_port.to_string(),
    }
}

fn tpl(id: &str, name: &str, description: &str, nodes: Vec<NodeData>, connections: Vec<ConnData>) -> Template {
    Template {
        id: id.into(),
        builtin: true,
        name: name.into(),
        description: description.into(),
        kind: TemplateKind::Full,
        nodes,
        connections,
        viewport: None,
    }
}

fn h3_chain_timeline() -> Template {
    tpl(
        "ve-h3-chain-timeline",
        "История из 2 сцен (H3 Ref2VA, хвост + якорь)",
        "Картинка-якорь → Список референсов → Видео 1 → Хвост (латент) + якорь → Видео 2 → Склейка → На таймлайн",
        vec![
            note(
                1,
                40.0,
                30.0,
                600.0,
                165.0,
                "### Бесшовная цепочка MiniMax-H3 Ref2VA с удержанием героя\n- **Список референсов** подаёт исходный кадр (`<Picture 1>`) в **обе сцены** — персонаж и окружение не «уплывают».\n- **Хвост (латент)** из Сцены 1 идёт в **Хвост (продолжить)** Сцены 2 — движение, поза и звук переходят плавно.\n- **Склейка** объединяет `Клип 1` и `Клип 2`, а **На таймлайн** сразу ставит ролик в проект.",
            ),
            node(
                2,
                NodeKind::FlowImage,
                40.0,
                275.0,
                Some("#F59E0B"),
                &[
                    ("prompt", f_text("orange tabby cat with white paws sitting in a cozy dusty attic next to an old brass lantern, 3D cartoon, warm golden light, film still")),
                    ("width", f_num(1344.0)),
                    ("height", f_num(768.0)),
                ],
            ),
            node(7, NodeKind::FlowList, 360.0, 470.0, None, &[]),
            node(
                3,
                NodeKind::FlowVideo,
                680.0,
                275.0,
                Some("#EE5E48"),
                &[
                    ("prompt", f_text("the orange tabby cat with white paws from <Picture 1> looks around the dim dusty attic curiously, camera slowly pushes in, soft creaking floorboards and gentle wind, quiet meow, 3D cartoon")),
                    ("model", f_text("minimax-h3-ref2va")),
                    ("seconds", f_num(5.0)),
                ],
            ),
            node(
                4,
                NodeKind::FlowVideo,
                1090.0,
                275.0,
                Some("#EE5E48"),
                &[
                    ("prompt", f_text("the same orange tabby cat with white paws from <Picture 1> reaches out with its front paw to the brass lantern in the dusty attic and turns the knob, warm golden glow illuminates dusty boxes, camera gently orbits, soft click and quiet purring, 3D cartoon")),
                    ("model", f_text("minimax-h3-ref2va")),
                    ("seconds", f_num(5.0)),
                ],
            ),
            node(5, NodeKind::FlowConcat, 1500.0, 275.0, Some("#EC4899"), &[]),
            node(6, NodeKind::Ext(ExtKind("ve_to_timeline")), 1850.0, 275.0, Some("#10B981"), &[]),
        ],
        vec![
            conn(2, "image", 7, "in1"),
            conn(2, "image", 3, "first_frame"),
            conn(7, "list", 3, "refs"),
            conn(7, "list", 4, "refs"),
            conn(3, "tail", 4, "context"),
            conn(3, "video", 5, "in1"),
            conn(4, "video", 5, "in2"),
            conn(5, "file", 6, "media"),
        ],
    )
}

fn story_film_timeline() -> Template {
    tpl(
        "ve-story-film-timeline",
        "Авто-фильм по идее (LLM → H3 Ref2VA)",
        "Идея и стиль → LLM пишет сцены → Опорный кадр-якорь → Видео цепочкой по хвосту + референсу → Склейка → На таймлайн",
        vec![
            note(
                1,
                40.0,
                30.0,
                600.0,
                168.0,
                "### Авто-фильм по вашей идее\n1. Поменяйте текст в **Идее** и **Стиле** слева.\n2. Нажмите **▶ Запуск** — LLM напишет сцены по строкам, **Картинка** создаст якорный референс `<Picture 1>`, а **Видео (H3 Ref2VA)** снимет сцены **цепочкой по хвосту с удержанием героя и мира по якорю**.\n3. **Склейка** соберёт все клипы в один ролик и отправит **На таймлайн**.",
            ),
            node(2, NodeKind::FlowText, 40.0, 275.0, Some("#10B981"), &[("text", f_text("рыжий кот находит на старом чердаке фонарик и включает его"))]),
            node(3, NodeKind::FlowText, 40.0, 520.0, Some("#10B981"), &[("text", f_text("3D cartoon, warm cinematic lighting"))]),
            node(
                4,
                NodeKind::FlowFormat,
                390.0,
                235.0,
                None,
                &[(
                    "template",
                    f_text("Раздели историю «{{a}}» на 2 сцены, которые продолжают друг друга. Для каждой сцены дай одну строку по-английски — подробное описание: внешность персонажа целиком (не пиши «Same ...»), действие, окружение, движение камеры и звук сцены. Без нумерации и пояснений. Ровно 2 строки."),
                )],
            ),
            node(5, NodeKind::FlowLlm, 750.0, 235.0, Some("#6366F1"), &[]),
            node(6, NodeKind::FlowLines, 1150.0, 235.0, None, &[]),
            node(7, NodeKind::FlowFormat, 1500.0, 235.0, None, &[("template", f_text("Keep the exact character, outfit, lighting and world from <Picture 1>. {{a}}, {{b}}"))]),
            node(8, NodeKind::FlowFormat, 390.0, 720.0, None, &[("template", f_text("{{a}}, opening scene, {{b}}, film still"))]),
            node(9, NodeKind::FlowImage, 750.0, 720.0, Some("#F59E0B"), &[("width", f_num(1344.0)), ("height", f_num(768.0))]),
            node(13, NodeKind::FlowList, 1150.0, 720.0, None, &[]),
            node(
                10,
                NodeKind::FlowVideo,
                1860.0,
                235.0,
                Some("#EE5E48"),
                &[("model", f_text("minimax-h3-ref2va")), ("seconds", f_num(5.0)), ("chain", f_bool(true))],
            ),
            node(11, NodeKind::FlowConcat, 2270.0, 235.0, Some("#EC4899"), &[]),
            node(12, NodeKind::Ext(ExtKind("ve_to_timeline")), 2620.0, 235.0, Some("#10B981"), &[]),
        ],
        vec![
            conn(2, "text", 4, "a"),
            conn(4, "text", 5, "prompt"),
            conn(5, "text", 6, "text"),
            conn(6, "list", 7, "a"),
            conn(3, "text", 7, "b"),
            conn(2, "text", 8, "a"),
            conn(3, "text", 8, "b"),
            conn(8, "text", 9, "prompt"),
            conn(9, "image", 13, "in1"),
            conn(7, "text", 10, "prompt"),
            conn(13, "list", 10, "refs"),
            conn(9, "image", 10, "first_frame"),
            conn(10, "video", 11, "files"),
            conn(11, "file", 12, "media"),
        ],
    )
}

fn ltx_chain_timeline() -> Template {
    tpl(
        "ve-ltx-chain-timeline",
        "Быстрая цепочка 2 сцен (LTX-2.3)",
        "Сцена 1 (LTX Distilled) → Хвост → Сцена 2 → Склейка → На таймлайн",
        vec![
            note(
                1,
                40.0,
                30.0,
                520.0,
                145.0,
                "### Быстрая цепочка на LTX-2.3 Distilled\n- Генерация идёт за **8 шагов** (~1 минута на 2 сцены).\n- Провод **Хвост (латент) → Хвост (продолжить)** стыкует видео и звук без шва.\n- Итог склеивается и сразу появляется на таймлайне.",
            ),
            node(
                2,
                NodeKind::FlowVideo,
                40.0,
                250.0,
                Some("#EE5E48"),
                &[
                    ("prompt", f_text("cinematic shot of a red vintage tram gliding along a rainy European street at dusk, warm glowing windows, soft rain and distant tram bell")),
                    ("model", f_text("ltx-2-3-22b-distilled-1-1")),
                    ("width", f_num(1024.0)),
                    ("height", f_num(576.0)),
                    ("seconds", f_num(4.0)),
                ],
            ),
            node(
                3,
                NodeKind::FlowVideo,
                450.0,
                250.0,
                Some("#EE5E48"),
                &[
                    ("prompt", f_text("the red vintage tram turns smoothly around a cobblestone corner, golden cafe lights reflecting on wet pavement, camera tracks alongside, gentle rain")),
                    ("model", f_text("ltx-2-3-22b-distilled-1-1")),
                    ("width", f_num(1024.0)),
                    ("height", f_num(576.0)),
                    ("seconds", f_num(4.0)),
                ],
            ),
            node(4, NodeKind::FlowConcat, 860.0, 250.0, Some("#EC4899"), &[]),
            node(5, NodeKind::Ext(ExtKind("ve_to_timeline")), 1210.0, 250.0, Some("#10B981"), &[]),
        ],
        vec![
            conn(2, "tail", 3, "context"),
            conn(2, "video", 4, "in1"),
            conn(3, "video", 4, "in2"),
            conn(4, "file", 5, "media"),
        ],
    )
}

fn h3_text_to_video() -> Template {
    tpl(
        "ve-h3-text-to-video",
        "H3: Текст → Видео со звуком",
        "Описание сцены → MiniMax-H3 (видео + звук) → На таймлайн",
        vec![
            node(
                1,
                NodeKind::FlowVideo,
                60.0,
                120.0,
                Some("#EE5E48"),
                &[
                    ("prompt", f_text("a woman in a cozy cafe talking warmly to the camera, soft morning daylight through the window, gentle background chatter and clink of cups, cinematic 35mm")),
                    ("model", f_text("minimax-h3-fl2va")),
                    ("seconds", f_num(5.0)),
                ],
            ),
            node(2, NodeKind::Ext(ExtKind("ve_to_timeline")), 470.0, 140.0, Some("#10B981"), &[]),
        ],
        vec![conn(1, "video", 2, "media")],
    )
}

fn h3_first_frame() -> Template {
    tpl(
        "ve-h3-first-frame",
        "H3: Оживить картинку (1-й кадр)",
        "Файл картинки → Видео H3 (первый кадр) → На таймлайн",
        vec![
            node(1, NodeKind::FlowFile, 40.0, 120.0, Some("#10B981"), &[("path", f_text(""))]),
            node(
                2,
                NodeKind::FlowVideo,
                390.0,
                100.0,
                Some("#EE5E48"),
                &[
                    ("prompt", f_text("the scene gently comes alive, natural camera motion, subtle breeze and ambient atmosphere, realistic sound")),
                    ("model", f_text("minimax-h3-fl2va")),
                    ("seconds", f_num(5.0)),
                ],
            ),
            node(3, NodeKind::Ext(ExtKind("ve_to_timeline")), 800.0, 120.0, Some("#10B981"), &[]),
        ],
        vec![conn(1, "file", 2, "first_frame"), conn(2, "video", 3, "media")],
    )
}

fn h3_first_last() -> Template {
    tpl(
        "ve-h3-first-last",
        "H3: Переход Первый → Последний кадр",
        "Кадр А + Кадр Б → MiniMax-H3 FL2VA → На таймлайн",
        vec![
            node(1, NodeKind::FlowFile, 40.0, 80.0, Some("#10B981"), &[("path", f_text(""))]),
            node(2, NodeKind::FlowFile, 40.0, 250.0, Some("#10B981"), &[("path", f_text(""))]),
            node(
                3,
                NodeKind::FlowVideo,
                390.0,
                100.0,
                Some("#EE5E48"),
                &[
                    ("prompt", f_text("smooth cinematic camera transition from the first scene to the final frame, continuous natural lighting and atmospheric sound")),
                    ("model", f_text("minimax-h3-fl2va")),
                    ("seconds", f_num(5.0)),
                ],
            ),
            node(4, NodeKind::Ext(ExtKind("ve_to_timeline")), 800.0, 120.0, Some("#10B981"), &[]),
        ],
        vec![
            conn(1, "file", 3, "first_frame"),
            conn(2, "file", 3, "last_frame"),
            conn(3, "video", 4, "media"),
        ],
    )
}

fn h3_staged_chain() -> Template {
    tpl(
        "ve-h3-staged-chain",
        "H3: Стадийная цепочка (tail → context)",
        "Ручные стадии MiniMax-H3: модель → encode → sample → хвост → сцена 2 → декодеры → склейка → таймлайн",
        vec![
            node(1, NodeKind::FlowH3Model, 40.0, 40.0, Some("#6366F1"), &[("model", f_text("minimax-h3-fl2va"))]),
            node(
                2,
                NodeKind::FlowH3Latent,
                40.0,
                300.0,
                None,
                &[("width", f_num(1344.0)), ("height", f_num(768.0)), ("seconds", f_num(4.0))],
            ),
            node(
                3,
                NodeKind::FlowH3Encode,
                400.0,
                40.0,
                Some("#EE5E48"),
                &[("prompt", f_text("a ginger cat sitting in a cozy attic near an old lantern, camera slowly approaches, creaking floorboards, 3D cartoon"))],
            ),
            node(
                4,
                NodeKind::FlowH3Sample,
                810.0,
                40.0,
                Some("#EE5E48"),
                &[("steps", f_num(20.0)), ("guidance", f_num(5.0)), ("seed", f_num(42.0))],
            ),
            node(5, NodeKind::FlowH3DecodeVideo, 1180.0, 20.0, None, &[]),
            node(6, NodeKind::FlowH3DecodeAudio, 1180.0, 170.0, None, &[]),
            node(7, NodeKind::FlowMux, 1540.0, 50.0, None, &[("mode", f_choice(0))]),
            node(
                8,
                NodeKind::FlowH3Encode,
                400.0,
                480.0,
                Some("#EE5E48"),
                &[("prompt", f_text("the ginger cat reaches its paw out to the lantern and turns it on, warm golden glow illuminates dusty boxes, soft click and purring, 3D cartoon"))],
            ),
            node(
                9,
                NodeKind::FlowH3Sample,
                810.0,
                480.0,
                Some("#EE5E48"),
                &[("steps", f_num(20.0)), ("guidance", f_num(5.0)), ("seed", f_num(42.0))],
            ),
            node(10, NodeKind::FlowH3DecodeVideo, 1180.0, 440.0, None, &[]),
            node(11, NodeKind::FlowH3DecodeAudio, 1180.0, 590.0, None, &[]),
            node(12, NodeKind::FlowMux, 1540.0, 480.0, None, &[("mode", f_choice(0))]),
            node(13, NodeKind::FlowConcat, 1920.0, 260.0, Some("#EC4899"), &[]),
            node(14, NodeKind::Ext(ExtKind("ve_to_timeline")), 2280.0, 260.0, Some("#10B981"), &[]),
        ],
        vec![
            conn(1, "model", 3, "model"),
            conn(1, "model", 4, "model"),
            conn(1, "model", 5, "model"),
            conn(1, "model", 6, "model"),
            conn(1, "model", 8, "model"),
            conn(1, "model", 9, "model"),
            conn(1, "model", 10, "model"),
            conn(1, "model", 11, "model"),
            conn(2, "latent", 3, "latent"),
            conn(2, "latent", 8, "latent"),
            conn(3, "cond", 4, "cond"),
            conn(4, "latent", 5, "latent"),
            conn(4, "latent", 6, "latent"),
            conn(5, "video", 7, "video"),
            conn(6, "audio", 7, "audio"),
            conn(4, "tail", 8, "context"),
            conn(8, "cond", 9, "cond"),
            conn(9, "latent", 10, "latent"),
            conn(9, "latent", 11, "latent"),
            conn(10, "video", 12, "video"),
            conn(11, "audio", 12, "audio"),
            conn(7, "video", 13, "in1"),
            conn(12, "video", 13, "in2"),
            conn(13, "file", 14, "media"),
        ],
    )
}

fn h3_restyle() -> Template {
    tpl(
        "ve-h3-restyle",
        "H3: Перерисовка видео в новом стиле",
        "Исходный ролик → MiniMax-H3 (restyle) → На таймлайн",
        vec![
            node(1, NodeKind::FlowFile, 40.0, 120.0, Some("#10B981"), &[("path", f_text(""))]),
            node(
                2,
                NodeKind::FlowVideo,
                390.0,
                100.0,
                Some("#EE5E48"),
                &[
                    ("prompt", f_text("stylized hand-painted anime film look, vibrant twilight colors, crisp linework, smooth cinematic lighting")),
                    ("model", f_text("minimax-h3-fl2va")),
                    ("restyle", f_bool(true)),
                    ("strength", f_num(0.6)),
                ],
            ),
            node(3, NodeKind::Ext(ExtKind("ve_to_timeline")), 800.0, 120.0, Some("#10B981"), &[]),
        ],
        vec![conn(1, "file", 2, "source_video"), conn(2, "video", 3, "media")],
    )
}

fn ltx_retake() -> Template {
    tpl(
        "ve-ltx-retake",
        "LTX-2.3: Пересъёмка отрезка (Retake)",
        "Исходный ролик → LTX-2.3 (перерисовать секунды 1.0–3.0) → На таймлайн",
        vec![
            node(1, NodeKind::FlowFile, 40.0, 120.0, Some("#10B981"), &[("path", f_text(""))]),
            node(
                2,
                NodeKind::FlowVideo,
                390.0,
                100.0,
                Some("#EE5E48"),
                &[
                    ("prompt", f_text("warm golden sunset glow suddenly lights up the scene, gentle camera movement")),
                    ("model", f_text("ltx-2-3-22b-distilled-1-1")),
                    ("retake_start", f_num(1.0)),
                    ("retake_end", f_num(3.0)),
                ],
            ),
            node(3, NodeKind::Ext(ExtKind("ve_to_timeline")), 800.0, 120.0, Some("#10B981"), &[]),
        ],
        vec![conn(1, "file", 2, "source_video"), conn(2, "video", 3, "media")],
    )
}

fn ltx_ic_lora() -> Template {
    tpl(
        "ve-ltx-ic-lora",
        "LTX-2.3: Видео по контурам/движению",
        "Управляющее видео → LTX-2.3 IC-LoRA (canny) → На таймлайн",
        vec![
            node(1, NodeKind::FlowFile, 40.0, 120.0, Some("#10B981"), &[("path", f_text(""))]),
            node(
                2,
                NodeKind::FlowVideo,
                390.0,
                100.0,
                Some("#EE5E48"),
                &[
                    ("prompt", f_text("cyberpunk neon street scene following the exact motion and framing, wet reflections, cinematic")),
                    ("model", f_text("ltx-2-3-22b-distilled-1-1")),
                    ("control", f_choice(2)),
                    ("seconds", f_num(5.0)),
                ],
            ),
            node(3, NodeKind::Ext(ExtKind("ve_to_timeline")), 800.0, 120.0, Some("#10B981"), &[]),
        ],
        vec![conn(1, "file", 2, "control_video"), conn(2, "video", 3, "media")],
    )
}

fn ltx_lipdub() -> Template {
    tpl(
        "ve-ltx-lipdub",
        "LTX-2.3: Переозвучка и липсинк (LipDub)",
        "Ролик + новая озвучка → LTX-2.3 LipDub → На таймлайн",
        vec![
            node(1, NodeKind::FlowFile, 40.0, 80.0, Some("#10B981"), &[("path", f_text(""))]),
            node(
                2,
                NodeKind::FlowSpeak,
                40.0,
                260.0,
                Some("#8B5CF6"),
                &[
                    ("text", f_text("Добро пожаловать! Это автоматическая переозвучка с синхронизацией губ.")),
                    ("voice", f_text("clear natural voice")),
                ],
            ),
            node(
                3,
                NodeKind::FlowVideo,
                450.0,
                120.0,
                Some("#EE5E48"),
                &[
                    ("prompt", f_text("the person speaks naturally with accurate lip sync, same lighting and framing")),
                    ("model", f_text("ltx-2-3-22b-distilled-1-1")),
                ],
            ),
            node(4, NodeKind::Ext(ExtKind("ve_to_timeline")), 860.0, 140.0, Some("#10B981"), &[]),
        ],
        vec![
            conn(1, "file", 3, "control_video"),
            conn(2, "audio", 3, "audio"),
            conn(3, "video", 4, "media"),
        ],
    )
}

fn ltx_a2v() -> Template {
    tpl(
        "ve-ltx-a2v",
        "LTX-2.3: Видео в ритм звука/музыки",
        "Стартовый кадр + аудио → LTX-2.3 Audio-to-Video → На таймлайн",
        vec![
            node(1, NodeKind::FlowFile, 40.0, 80.0, Some("#F59E0B"), &[("path", f_text(""))]),
            node(2, NodeKind::FlowFile, 40.0, 250.0, Some("#8B5CF6"), &[("path", f_text(""))]),
            node(
                3,
                NodeKind::FlowVideo,
                390.0,
                100.0,
                Some("#EE5E48"),
                &[
                    ("prompt", f_text("cinematic music video scene moving smoothly to the beat, dynamic lighting and subtle camera sway")),
                    ("model", f_text("ltx-2-3-22b-distilled-1-1")),
                    ("seconds", f_num(10.0)),
                ],
            ),
            node(4, NodeKind::Ext(ExtKind("ve_to_timeline")), 800.0, 120.0, Some("#10B981"), &[]),
        ],
        vec![
            conn(1, "file", 3, "first_frame"),
            conn(2, "file", 3, "audio"),
            conn(3, "video", 4, "media"),
        ],
    )
}

fn montage_music_timeline() -> Template {
    tpl(
        "ve-montage-music-timeline",
        "Склейка клипов + музыка",
        "Два файла → Склейка → Наложение сгенерированной музыки (mix) → На таймлайн",
        vec![
            note(
                1,
                40.0,
                30.0,
                520.0,
                160.0,
                "### Склейка клипов с фоновой музыкой\n1. Выберите 2 ролика в нодах **Файл** (или подключите выходы нод **Видео**).\n2. **Склейка** объединит `Клип 1` и `Клип 2` в один ролик.\n3. **Наложение звука** в режиме `mix` подмешает сгенерированную музыку (`0.45`) и отправит **На таймлайн**.",
            ),
            node(2, NodeKind::FlowFile, 40.0, 270.0, Some("#10B981"), &[("path", f_text(""))]),
            node(3, NodeKind::FlowFile, 40.0, 440.0, Some("#10B981"), &[("path", f_text(""))]),
            node(4, NodeKind::FlowConcat, 390.0, 270.0, Some("#EC4899"), &[]),
            node(
                5,
                NodeKind::FlowMusic,
                390.0,
                540.0,
                Some("#8B5CF6"),
                &[
                    ("prompt", f_text("warm acoustic guitar and soft piano, cozy cinematic ambient, gentle rhythm")),
                    ("lyrics", f_text("")),
                    ("seconds", f_num(12.0)),
                ],
            ),
            node(6, NodeKind::FlowMux, 790.0, 320.0, Some("#EC4899"), &[("mode", f_choice(1)), ("volume", f_num(0.45))]),
            node(7, NodeKind::Ext(ExtKind("ve_to_timeline")), 1140.0, 320.0, Some("#10B981"), &[]),
        ],
        vec![
            conn(2, "file", 4, "in1"),
            conn(3, "file", 4, "in2"),
            conn(4, "file", 6, "video"),
            conn(5, "audio", 6, "audio"),
            conn(6, "video", 7, "media"),
        ],
    )
}

fn auto_edit_project() -> Template {
    tpl(
        "ve-auto-edit-project",
        "Автомонтаж: паузы + субтитры",
        "Медиатека проекта → Вырезать паузы + Субтитры из речи",
        vec![
            note(
                1,
                40.0,
                30.0,
                520.0,
                148.0,
                "### Автомонтаж текущего проекта\n1. Добавьте клипы с речью на таймлайн проекта.\n2. Нажмите **▶ Запуск** — граф возьмёт файлы из **Медиатеки проекта**, вырежет тишину (тише `−35 дБ`) и создаст дорожку субтитров по речи.",
            ),
            node(2, NodeKind::Ext(ExtKind("ve_project_media")), 40.0, 260.0, Some("#3B82F6"), &[]),
            node(
                3,
                NodeKind::Ext(ExtKind("ve_cut_silence")),
                390.0,
                210.0,
                Some("#EE5E48"),
                &[("threshold_db", f_num(-35.0)), ("min", f_num(0.6))],
            ),
            node(
                4,
                NodeKind::Ext(ExtKind("ve_subtitles")),
                390.0,
                420.0,
                Some("#8B5CF6"),
                &[("max_chars", f_num(42.0)), ("language", f_text(""))],
            ),
        ],
        vec![conn(2, "media", 3, "media"), conn(2, "media", 4, "media")],
    )
}

fn vfx_upscale_rife() -> Template {
    tpl(
        "ve-vfx-upscale-rife",
        "Апскейл ×2 + 60 к/с (RIFE)",
        "Файл → Нейроувеличение ×2 → Плавность RIFE (60 fps) → На таймлайн",
        vec![
            note(
                1,
                40.0,
                30.0,
                520.0,
                145.0,
                "### Нейроулучшение видео (CUDA)\n1. Выберите исходный видеофайл слева.\n2. **Нейроувеличение** поднимет разрешение в 2 раза, **Плавность RIFE** достроит кадры до 60 к/с, а **На таймлайн** добавит улучшенный ролик в проект.",
            ),
            node(2, NodeKind::FlowFile, 40.0, 250.0, Some("#10B981"), &[("path", f_text(""))]),
            node(3, NodeKind::Ext(ExtKind("ve_vfx_upscale")), 390.0, 250.0, Some("#F59E0B"), &[("scale", f_choice(0))]),
            node(4, NodeKind::Ext(ExtKind("ve_vfx_interpolate")), 760.0, 250.0, Some("#8B5CF6"), &[("fps", f_num(60.0))]),
            node(5, NodeKind::Ext(ExtKind("ve_to_timeline")), 1120.0, 250.0, Some("#10B981"), &[]),
        ],
        vec![
            conn(2, "file", 3, "media"),
            conn(3, "out", 4, "media"),
            conn(4, "out", 5, "media"),
        ],
    )
}

fn trim_and_continue() -> Template {
    tpl(
        "ve-trim-continue",
        "Обрезать ролик и дорисовать финал",
        "Файл → Обрезка → Кадр (last) → Видео H3 → Склейка → На таймлайн",
        vec![
            node(1, NodeKind::FlowFile, 40.0, 120.0, Some("#10B981"), &[("path", f_text(""))]),
            node(2, NodeKind::FlowTrim, 380.0, 120.0, Some("#3B82F6"), &[("start", f_num(0.0)), ("end", f_num(4.0))]),
            node(3, NodeKind::FlowFrame, 720.0, 260.0, Some("#F59E0B"), &[("at", f_text("last"))]),
            node(
                4,
                NodeKind::FlowVideo,
                1080.0,
                220.0,
                Some("#EE5E48"),
                &[
                    ("prompt", f_text("smooth continuation of the scene, camera slowly pulls back revealing the wide view, warm cinematic atmosphere")),
                    ("model", f_text("minimax-h3-fl2va")),
                    ("seconds", f_num(4.0)),
                ],
            ),
            node(5, NodeKind::FlowConcat, 1490.0, 140.0, Some("#EC4899"), &[]),
            node(6, NodeKind::Ext(ExtKind("ve_to_timeline")), 1840.0, 140.0, Some("#10B981"), &[]),
        ],
        vec![
            conn(1, "file", 2, "file"),
            conn(2, "file", 3, "video"),
            conn(3, "image", 4, "first_frame"),
            conn(2, "file", 5, "in1"),
            conn(4, "video", 5, "in2"),
            conn(5, "file", 6, "media"),
        ],
    )
}

fn still_voiceover() -> Template {
    tpl(
        "ve-still-voiceover",
        "Картинка + озвучка → ролик",
        "Файл картинки + Речь → Картинка в ролик → На таймлайн",
        vec![
            node(1, NodeKind::FlowFile, 40.0, 80.0, Some("#F59E0B"), &[("path", f_text(""))]),
            node(
                2,
                NodeKind::FlowSpeak,
                40.0,
                250.0,
                Some("#8B5CF6"),
                &[
                    ("text", f_text("В этой главе мы разберём, как собрать короткий ролик прямо в редакторе графов.")),
                    ("voice", f_text("calm narrator voice")),
                ],
            ),
            node(3, NodeKind::FlowStill, 440.0, 140.0, Some("#EC4899"), &[("seconds", f_num(5.0))]),
            node(4, NodeKind::Ext(ExtKind("ve_to_timeline")), 790.0, 140.0, Some("#10B981"), &[]),
        ],
        vec![
            conn(1, "file", 3, "image"),
            conn(2, "audio", 3, "audio"),
            conn(3, "video", 4, "media"),
        ],
    )
}

fn cover_llm_image() -> Template {
    tpl(
        "ve-cover-llm-image",
        "Стартовый кадр / обложка через LLM",
        "Идея по-русски → LLM улучшает промпт → Картинка 1344×768 → На таймлайн",
        vec![
            node(1, NodeKind::FlowText, 40.0, 120.0, Some("#10B981"), &[("text", f_text("уютная деревянная мастерская на закате, лучи света сквозь пыль"))]),
            node(
                2,
                NodeKind::FlowFormat,
                390.0,
                100.0,
                None,
                &[(
                    "template",
                    f_text("Преврати идею «{{a}}» в один детальный кинематографичный промпт на английском для генерации кадра 16:9 (свет, оптика, цвета, атмосфера). Только промпт."),
                )],
            ),
            node(3, NodeKind::FlowLlm, 750.0, 100.0, Some("#6366F1"), &[]),
            node(4, NodeKind::FlowImage, 1150.0, 100.0, Some("#F59E0B"), &[("width", f_num(1344.0)), ("height", f_num(768.0))]),
            node(5, NodeKind::Ext(ExtKind("ve_to_timeline")), 1560.0, 120.0, Some("#10B981"), &[]),
        ],
        vec![
            conn(1, "text", 2, "a"),
            conn(2, "text", 3, "prompt"),
            conn(3, "text", 4, "prompt"),
            conn(4, "image", 5, "media"),
        ],
    )
}

fn image_refs_edit() -> Template {
    tpl(
        "ve-image-refs-edit",
        "Правка кадра по референсам",
        "Картинка 1 + Картинка 2 → Список референсов → Картинка → На таймлайн",
        vec![
            node(1, NodeKind::FlowFile, 40.0, 80.0, Some("#10B981"), &[("path", f_text(""))]),
            node(2, NodeKind::FlowFile, 40.0, 250.0, Some("#10B981"), &[("path", f_text(""))]),
            node(3, NodeKind::FlowList, 380.0, 120.0, None, &[]),
            node(
                4,
                NodeKind::FlowImage,
                740.0,
                100.0,
                Some("#F59E0B"),
                &[
                    ("prompt", f_text("place the character from the first reference inside the environment from the second reference, matching warm cinematic light")),
                    ("width", f_num(1344.0)),
                    ("height", f_num(768.0)),
                ],
            ),
            node(5, NodeKind::Ext(ExtKind("ve_to_timeline")), 1150.0, 120.0, Some("#10B981"), &[]),
        ],
        vec![
            conn(1, "file", 3, "in1"),
            conn(2, "file", 3, "in2"),
            conn(3, "list", 4, "images"),
            conn(4, "image", 5, "media"),
        ],
    )
}

fn video_frame_stylize() -> Template {
    tpl(
        "ve-video-frame-stylize",
        "Стилизация кадра из видео (img2img)",
        "Видео → Кадр (на секунде) → Картинка img2img → На таймлайн",
        vec![
            node(1, NodeKind::FlowFile, 40.0, 120.0, Some("#10B981"), &[("path", f_text(""))]),
            node(2, NodeKind::FlowFrame, 380.0, 120.0, Some("#F59E0B"), &[("at", f_text("first"))]),
            node(
                3,
                NodeKind::FlowImage,
                740.0,
                100.0,
                Some("#F59E0B"),
                &[
                    ("prompt", f_text("oil painting cinema poster style, dramatic warm rim light, rich texture")),
                    ("strength", f_num(0.55)),
                ],
            ),
            node(4, NodeKind::Ext(ExtKind("ve_to_timeline")), 1150.0, 120.0, Some("#10B981"), &[]),
        ],
        vec![
            conn(1, "file", 2, "video"),
            conn(2, "image", 3, "init"),
            conn(3, "image", 4, "media"),
        ],
    )
}

fn talking_avatar_timeline() -> Template {
    tpl(
        "ve-talking-avatar-timeline",
        "Говорящий аватар с фото",
        "Фото + Речь → Список референсов → Видео H3 Ref2VA → Наложение звука → На таймлайн",
        vec![
            note(
                1,
                40.0,
                30.0,
                520.0,
                155.0,
                "### Говорящий аватар (MiniMax-H3 Ref2VA)\n1. Выберите портретное фото в ноде **Файл** и напишите реплику в ноде **Речь**.\n2. **Список** объединит фото и речь в референсы для H3.\n3. **Наложение звука** подставит чистую озвучку и отправит ролик **На таймлайн**.",
            ),
            node(2, NodeKind::FlowFile, 40.0, 260.0, Some("#10B981"), &[("path", f_text(""))]),
            node(
                3,
                NodeKind::FlowSpeak,
                40.0,
                440.0,
                Some("#8B5CF6"),
                &[
                    ("text", f_text("Привет! Это пример говорящего аватара в видеоредакторе.")),
                    ("voice", f_text("warm natural voice")),
                ],
            ),
            node(4, NodeKind::FlowList, 440.0, 270.0, None, &[]),
            node(
                5,
                NodeKind::FlowVideo,
                800.0,
                270.0,
                Some("#EE5E48"),
                &[
                    ("prompt", f_text("the character from the reference image talks naturally to the camera saying the words from the reference audio, clear lip sync, subtle expressions, same outfit and lighting")),
                    ("model", f_text("minimax-h3-ref2va")),
                ],
            ),
            node(6, NodeKind::FlowMux, 1210.0, 310.0, Some("#EC4899"), &[("mode", f_choice(0))]),
            node(7, NodeKind::Ext(ExtKind("ve_to_timeline")), 1560.0, 310.0, Some("#10B981"), &[]),
        ],
        vec![
            conn(2, "file", 4, "in1"),
            conn(3, "audio", 4, "in2"),
            conn(4, "list", 5, "refs"),
            conn(5, "video", 6, "video"),
            conn(3, "audio", 6, "audio"),
            conn(6, "video", 7, "media"),
        ],
    )
}

fn music_video_timeline() -> Template {
    tpl(
        "ve-music-video-timeline",
        "Клип под песню (LLM → Музыка → Видео)",
        "Тема + Стиль → стихи + обложка → песня → видео LTX под звук → На таймлайн",
        vec![
            node(1, NodeKind::FlowText, 40.0, 80.0, Some("#10B981"), &[("text", f_text("ночной город после дождя и тёплые огни витрин"))]),
            node(2, NodeKind::FlowText, 40.0, 310.0, Some("#10B981"), &[("text", f_text("indie pop, warm synth, uplifting, soft female vocal"))]),
            node(
                3,
                NodeKind::FlowFormat,
                390.0,
                60.0,
                None,
                &[("template", f_text("Напиши короткий текст песни на тему «{{a}}»: куплет и припев по 4 строки, с метками [verse] и [chorus]. Только текст песни."))],
            ),
            node(4, NodeKind::FlowLlm, 750.0, 60.0, Some("#6366F1"), &[]),
            node(5, NodeKind::FlowMusic, 1150.0, 60.0, Some("#8B5CF6"), &[("seconds", f_num(12.0))]),
            node(
                6,
                NodeKind::FlowFormat,
                390.0,
                420.0,
                None,
                &[("template", f_text("Опиши одним предложением по-английски яркий кинематографичный кадр для клипа о «{{a}}» в стиле {{b}}. Только описание кадра."))],
            ),
            node(7, NodeKind::FlowLlm, 750.0, 420.0, Some("#6366F1"), &[]),
            node(8, NodeKind::FlowImage, 1150.0, 420.0, Some("#F59E0B"), &[("width", f_num(1024.0)), ("height", f_num(576.0))]),
            node(
                9,
                NodeKind::FlowVideo,
                1560.0,
                220.0,
                Some("#EE5E48"),
                &[("model", f_text("ltx-2-3-22b-distilled-1-1")), ("seconds", f_num(12.0))],
            ),
            node(10, NodeKind::Ext(ExtKind("ve_to_timeline")), 1970.0, 240.0, Some("#10B981"), &[]),
        ],
        vec![
            conn(1, "text", 3, "a"),
            conn(3, "text", 4, "prompt"),
            conn(2, "text", 5, "prompt"),
            conn(4, "text", 5, "lyrics"),
            conn(1, "text", 6, "a"),
            conn(2, "text", 6, "b"),
            conn(6, "text", 7, "prompt"),
            conn(7, "text", 8, "prompt"),
            conn(7, "text", 9, "prompt"),
            conn(8, "image", 9, "first_frame"),
            conn(5, "audio", 9, "audio"),
            conn(9, "video", 10, "media"),
        ],
    )
}

fn bgm_acestep() -> Template {
    tpl(
        "ve-bgm-acestep",
        "Фоновая музыка для монтажа (ACE-Step)",
        "Стиль → Музыка ACE-Step → На таймлайн",
        vec![
            node(
                1,
                NodeKind::FlowMusic,
                60.0,
                120.0,
                Some("#8B5CF6"),
                &[
                    ("prompt", f_text("modern lo-fi chill beat, warm Rhodes piano, soft vinyl crackle, relaxing background for vlog")),
                    ("lyrics", f_text("")),
                    ("seconds", f_num(20.0)),
                ],
            ),
            node(2, NodeKind::Ext(ExtKind("ve_to_timeline")), 470.0, 140.0, Some("#10B981"), &[]),
        ],
        vec![conn(1, "audio", 2, "media")],
    )
}

fn voice_clone_omnivoice() -> Template {
    tpl(
        "ve-voice-clone-omnivoice",
        "Клон голоса / озвучка (OmniVoice)",
        "Образец голоса + Текст → OmniVoice → На таймлайн",
        vec![
            node(1, NodeKind::FlowFile, 40.0, 120.0, Some("#10B981"), &[("path", f_text(""))]),
            node(
                2,
                NodeKind::FlowOmnivoice,
                400.0,
                100.0,
                Some("#8B5CF6"),
                &[
                    ("text", f_text("Привет! Этот закадровый голос сгенерирован локально и сразу добавлен на дорожку таймлайна.")),
                    ("instruct", f_text("warm expressive narrator")),
                ],
            ),
            node(3, NodeKind::Ext(ExtKind("ve_to_timeline")), 810.0, 120.0, Some("#10B981"), &[]),
        ],
        vec![conn(1, "file", 2, "ref_audio"), conn(2, "audio", 3, "media")],
    )
}

fn podcast_vibevoice() -> Template {
    tpl(
        "ve-podcast-vibevoice",
        "Подкаст / диалог на 2 голоса",
        "Тема → LLM диалог → VibeVoice → На таймлайн",
        vec![
            node(1, NodeKind::FlowText, 40.0, 120.0, Some("#10B981"), &[("text", f_text("почему локальные нейросети меняют видеомонтаж"))]),
            node(
                2,
                NodeKind::FlowFormat,
                390.0,
                100.0,
                None,
                &[(
                    "template",
                    f_text("Напиши короткий живой диалог-подкаст из 4 реплик на тему «{{a}}» в формате:\nSpeaker 1: ...\nSpeaker 2: ...\nТолько реплики."),
                )],
            ),
            node(3, NodeKind::FlowLlm, 750.0, 100.0, Some("#6366F1"), &[]),
            node(4, NodeKind::FlowVibevoice, 1150.0, 100.0, Some("#8B5CF6"), &[]),
            node(5, NodeKind::Ext(ExtKind("ve_to_timeline")), 1560.0, 120.0, Some("#10B981"), &[]),
        ],
        vec![
            conn(1, "text", 2, "a"),
            conn(2, "text", 3, "prompt"),
            conn(3, "text", 4, "script"),
            conn(4, "audio", 5, "media"),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn_node_editor::convert::{resolve_field_name, resolve_port_name};
    use syn_node_editor::types::PortSide;

    #[test]
    fn video_editor_presets_resolve_all_ports_and_fields() {
        crate::ui::graph::init();
        let list = all();
        assert!(list.len() >= 25, "ожидалось ≥25 заготовок, найдено {}", list.len());
        for (cat, _) in PresetCat::ALL {
            assert!(list.iter().any(|p| p.cat == *cat), "пустая категория {cat:?}");
        }
        for p in list {
            let t = (p.build)();
            assert!(!t.nodes.is_empty(), "{}", p.id);
            for n in &t.nodes {
                for f in n.fields.keys() {
                    assert!(resolve_field_name(n.kind, f).is_some(), "{}: неизвестное поле {:?}.{f}", p.id, n.kind);
                }
            }
            for c in &t.connections {
                let fk = t.kind_of(c.from_node);
                let tk = t.kind_of(c.to_node);
                assert!(resolve_port_name(fk, PortSide::Output, &c.from_port).is_some(), "{}: неизвестный выход {:?}.{}", p.id, fk, c.from_port);
                assert!(resolve_port_name(tk, PortSide::Input, &c.to_port).is_some(), "{}: неизвестный вход {:?}.{}", p.id, tk, c.to_port);
            }
        }
    }
}
