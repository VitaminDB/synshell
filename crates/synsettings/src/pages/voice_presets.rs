//! Готовые описания голоса (voice design) для моделей, которые их понимают:
//! VoxCPM2 слушает только подробные описания по-английски — кто, возраст,
//! высота и тембр, манера; OmniVoice — только признаки из своего словаря
//! (пол, возраст, высота, whisper, акцент) через запятую.

use syngui::n_;

pub struct Preset {
    pub title: &'static str,
    pub voice: &'static str,
}

pub struct Category {
    pub title: &'static str,
    /// Пояснение под вкладкой (например, где действует акцент).
    pub note: Option<&'static str>,
    pub presets: &'static [Preset],
}

/// Набор пресетов движка и подсказка к нему.
pub struct Presets {
    pub hint: &'static str,
    pub categories: &'static [Category],
}

const fn p(title: &'static str, voice: &'static str) -> Preset {
    Preset { title, voice }
}

const VOXCPM: &[Category] = &[
    Category {
        title: n_!("Дети"),
        note: None,
        presets: &[
            p(n_!("Мальчик 6 лет"), "A little boy, about 6 years old, high-pitched childish voice, curious and playful"),
            p(n_!("Девочка 6 лет"), "A little girl, about 6 years old, high-pitched sweet childish voice, cheerful"),
            p(n_!("Мальчик 10 лет"), "A boy around 10 years old, clear bright child voice, energetic and lively"),
            p(n_!("Девочка 10 лет"), "A girl around 10 years old, bright clear child voice, friendly and talkative"),
            p(n_!("Подросток-парень"), "A teenage boy, about 15 years old, slightly breaking youthful voice, casual and relaxed"),
            p(n_!("Подросток-девушка"), "A teenage girl, about 15 years old, light youthful voice, lively and expressive"),
            p(n_!("Робкий ребёнок"), "A shy little child, soft quiet high-pitched voice, hesitant and gentle"),
            p(n_!("Озорной мальчишка"), "A mischievous young boy, high-pitched excited voice, fast and giggly"),
            p(n_!("Девочка-сказочница"), "A young girl telling a fairy tale, soft high voice, dreamy and full of wonder"),
            p(n_!("Капризный малыш"), "A small toddler, very high-pitched voice, whiny and pouting"),
        ],
    },
    Category {
        title: n_!("Женщины"),
        note: None,
        presets: &[
            p(n_!("Тёплый"), "A young woman, warm and gentle voice, soft and friendly"),
            p(n_!("Диктор новостей"), "A professional female news anchor, clear confident mid-pitched voice, neutral and precise"),
            p(n_!("Бодрая ведущая"), "An energetic young female radio host, bright upbeat voice, smiling"),
            p(n_!("Деловой"), "A middle-aged businesswoman, calm assertive voice, articulate and composed"),
            p(n_!("Шёпот"), "A young woman whispering softly, breathy intimate voice, calm and slow"),
            p(n_!("Бархатный низкий"), "A woman with a low velvety contralto voice, smooth and sultry, relaxed"),
            p(n_!("Учительница"), "A kind female teacher, clear patient voice, warm and encouraging, moderate pace"),
            p(n_!("Ассистентка"), "A friendly female virtual assistant, clear neutral voice, polite and helpful"),
            p(n_!("Уставший"), "A tired young woman, soft low-energy voice, sighing and slow"),
            p(n_!("Строгий"), "A strict middle-aged woman, firm sharp voice, serious and commanding"),
        ],
    },
    Category {
        title: n_!("Мужчины"),
        note: None,
        presets: &[
            p(n_!("Спокойный"), "A young man, calm and steady mid-range voice, friendly and relaxed"),
            p(n_!("Диктор новостей"), "A professional male news anchor, deep clear authoritative voice, neutral and precise"),
            p(n_!("Глубокий бас"), "A man with a very deep resonant bass voice, slow and powerful"),
            p(n_!("Бодрый ведущий"), "An energetic young male radio host, bright upbeat voice, fast and enthusiastic"),
            p(n_!("Рассказчик"), "A middle-aged male storyteller, warm rich baritone voice, expressive and engaging"),
            p(n_!("Хриплый"), "A rough man with a raspy husky voice, low and gritty"),
            p(n_!("Ассистент"), "A friendly male virtual assistant, clear neutral voice, polite and helpful"),
            p(n_!("Шёпот"), "A man whispering quietly, low breathy voice, secretive and slow"),
            p(n_!("Командир"), "A military commander, loud firm barking voice, stern and commanding"),
            p(n_!("Нервный"), "A nervous young man, slightly high shaky voice, fast and stammering"),
        ],
    },
    Category {
        title: n_!("Пожилые"),
        note: None,
        presets: &[
            p(n_!("Бабушка"), "An elderly grandmother, about 75 years old, soft warm shaky voice, kind and slow"),
            p(n_!("Дедушка"), "An elderly grandfather, about 75 years old, deep husky voice, slow and wise"),
            p(n_!("Старый профессор"), "An old male professor, dry measured voice, thoughtful and precise"),
            p(n_!("Пожилая дама"), "An elegant elderly lady, refined gentle voice, calm and graceful"),
            p(n_!("Ворчливый старик"), "A grumpy old man, raspy low voice, irritated and mumbling"),
            p(n_!("Весёлая старушка"), "A cheerful old woman, high wavering voice, chuckling and lively"),
            p(n_!("Мудрый старец"), "A very old wise man, slow deep breathy voice, calm and solemn"),
            p(n_!("Пожилой диктор"), "A senior male broadcaster, rich mature baritone voice, slow and dignified"),
            p(n_!("Слабый голос"), "A frail elderly person, weak thin trembling voice, quiet and slow"),
            p(n_!("Деревенский дед"), "An old village man, warm hoarse voice, simple and good-natured"),
        ],
    },
    Category {
        title: n_!("Характерные"),
        note: None,
        presets: &[
            p(n_!("Злодей"), "A sinister villain, low cold menacing voice, slow and sneering"),
            p(n_!("Робот"), "A robotic monotone voice, flat and mechanical, evenly paced"),
            p(n_!("Пират"), "A rough pirate captain, loud gravelly voice, boisterous and hearty"),
            p(n_!("Сказка на ночь"), "A gentle bedtime storyteller, soft warm voice, slow and soothing"),
            p(n_!("Трейлер фильма"), "An epic movie trailer narrator, very deep dramatic voice, slow and intense"),
            p(n_!("Спорткомментатор"), "An excited sports commentator, loud fast energetic voice, shouting with excitement"),
            p(n_!("Мультяшный"), "A cartoon character, squeaky high-pitched voice, silly and bouncy"),
            p(n_!("Медитация"), "A meditation guide, very calm soft low voice, slow and peaceful"),
            p(n_!("Аристократ"), "A posh aristocrat, refined slightly nasal voice, haughty and slow"),
            p(n_!("Грустный"), "A sad person on the verge of tears, trembling quiet voice, sobbing softly"),
        ],
    },
];

const OMNIVOICE: &[Category] = &[
    Category {
        title: n_!("Дети"),
        note: None,
        presets: &[
            p(n_!("Мальчик"), "male, child"),
            p(n_!("Мальчик, средний голос"), "male, child, moderate pitch"),
            p(n_!("Мальчик, высокий голос"), "male, child, high pitch"),
            p(n_!("Мальчик, очень высокий"), "male, child, very high pitch"),
            p(n_!("Мальчик шёпотом"), "male, child, whisper"),
            p(n_!("Девочка"), "female, child"),
            p(n_!("Девочка, средний голос"), "female, child, moderate pitch"),
            p(n_!("Девочка, высокий голос"), "female, child, high pitch"),
            p(n_!("Девочка, очень высокий"), "female, child, very high pitch"),
            p(n_!("Девочка шёпотом"), "female, child, whisper"),
        ],
    },
    Category {
        title: n_!("Молодые"),
        note: None,
        presets: &[
            p(n_!("Подросток-парень"), "male, teenager"),
            p(n_!("Подросток-парень, высокий"), "male, teenager, high pitch"),
            p(n_!("Подросток-девушка"), "female, teenager"),
            p(n_!("Подросток-девушка, высокий"), "female, teenager, high pitch"),
            p(n_!("Молодой мужчина"), "male, young adult"),
            p(n_!("Молодой мужчина, низкий"), "male, young adult, low pitch"),
            p(n_!("Молодой мужчина, высокий"), "male, young adult, high pitch"),
            p(n_!("Молодая женщина"), "female, young adult"),
            p(n_!("Молодая женщина, низкий"), "female, young adult, low pitch"),
            p(n_!("Молодая женщина, высокий"), "female, young adult, high pitch"),
        ],
    },
    Category {
        title: n_!("Взрослые"),
        note: None,
        presets: &[
            p(n_!("Мужчина"), "male, middle-aged"),
            p(n_!("Мужчина, очень низкий"), "male, middle-aged, very low pitch"),
            p(n_!("Мужчина, низкий"), "male, middle-aged, low pitch"),
            p(n_!("Мужчина, средний"), "male, middle-aged, moderate pitch"),
            p(n_!("Мужчина, высокий"), "male, middle-aged, high pitch"),
            p(n_!("Женщина"), "female, middle-aged"),
            p(n_!("Женщина, очень низкий"), "female, middle-aged, very low pitch"),
            p(n_!("Женщина, низкий"), "female, middle-aged, low pitch"),
            p(n_!("Женщина, средний"), "female, middle-aged, moderate pitch"),
            p(n_!("Женщина, высокий"), "female, middle-aged, high pitch"),
        ],
    },
    Category {
        title: n_!("Пожилые"),
        note: None,
        presets: &[
            p(n_!("Пожилой мужчина"), "male, elderly"),
            p(n_!("Пожилой, очень низкий"), "male, elderly, very low pitch"),
            p(n_!("Пожилой, низкий"), "male, elderly, low pitch"),
            p(n_!("Пожилой, средний"), "male, elderly, moderate pitch"),
            p(n_!("Пожилой, высокий"), "male, elderly, high pitch"),
            p(n_!("Пожилая женщина"), "female, elderly"),
            p(n_!("Пожилая, очень низкий"), "female, elderly, very low pitch"),
            p(n_!("Пожилая, низкий"), "female, elderly, low pitch"),
            p(n_!("Пожилая, средний"), "female, elderly, moderate pitch"),
            p(n_!("Пожилая, высокий"), "female, elderly, high pitch"),
        ],
    },
    Category {
        title: n_!("Шёпот и акценты"),
        note: Some(n_!("Акцент слышен только в английском тексте")),
        presets: &[
            p(n_!("Шёпот, мужчина"), "male, young adult, whisper"),
            p(n_!("Шёпот, женщина"), "female, young adult, whisper"),
            p(n_!("Шёпот, пожилой"), "male, elderly, whisper"),
            p(n_!("Шёпот, пожилая"), "female, elderly, whisper"),
            p(n_!("Британский, мужчина"), "male, middle-aged, british accent"),
            p(n_!("Британский, женщина"), "female, young adult, british accent"),
            p(n_!("Американский, мужчина"), "male, young adult, american accent"),
            p(n_!("Американский, женщина"), "female, middle-aged, american accent"),
            p(n_!("Русский акцент, мужчина"), "male, middle-aged, russian accent"),
            p(n_!("Индийский, женщина"), "female, young adult, indian accent"),
        ],
    },
];

/// Пресеты для архитектуры модели; `None` — описаний голоса она не понимает.
pub fn for_arch(arch: &str) -> Option<&'static Presets> {
    static VOX: Presets = Presets {
        hint: n_!("Нажмите — описание встанет в «Голос» и сразу прозвучит фраза ниже. Голос немного меняется от зерна: понравился — закрепите зерно"),
        categories: VOXCPM,
    };
    static OMNI: Presets = Presets {
        hint: n_!("Нажмите — признаки встанут в «Голос» и сразу прозвучит фраза ниже. OmniVoice понимает только пол, возраст, высоту, шёпот и акцент через запятую"),
        categories: OMNIVOICE,
    };
    let arch = arch.to_ascii_lowercase();
    if arch.starts_with("voxcpm") {
        Some(&VOX)
    } else if arch.contains("omnivoice") {
        Some(&OMNI)
    } else {
        None
    }
}

impl Presets {
    /// Категория, в которой лежит описание (для начальной вкладки).
    pub fn category_of(&self, voice: &str) -> Option<usize> {
        self.categories.iter().position(|c| c.presets.iter().any(|p| p.voice == voice))
    }
}
