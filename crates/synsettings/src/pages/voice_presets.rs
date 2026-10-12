//! Готовые описания голоса для VoxCPM2 (voice design): модель слушает только
//! подробные описания по-английски — кто, возраст, высота и тембр, манера.

use syngui::n_;

pub struct Preset {
    pub title: &'static str,
    pub voice: &'static str,
}

pub struct Category {
    pub title: &'static str,
    pub presets: &'static [Preset],
}

const fn p(title: &'static str, voice: &'static str) -> Preset {
    Preset { title, voice }
}

pub const CATEGORIES: &[Category] = &[
    Category {
        title: n_!("Дети"),
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

/// Категория, в которой лежит описание (для начальной вкладки).
pub fn category_of(voice: &str) -> Option<usize> {
    CATEGORIES.iter().position(|c| c.presets.iter().any(|p| p.voice == voice))
}
