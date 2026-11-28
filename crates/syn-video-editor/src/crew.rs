//! Съёмочная группа агентов: роли, машина состояний сцены, бриф шага,
//! записки между ролями с эскалацией, вызов роли кодом.
//!
//! Конвейер ведёт режиссёр (основной агент): по статусу сцены код знает,
//! какой шаг следующий и чья это работа; `crew.run` запускает ход этой роли
//! отдельным агентом — только её навыки и бриф (выжимка проекта, сцена
//! источника, карточки героев, мест и реквизита сцены, её ассеты, открытые
//! записки), а не все документы фильма. Итог хода — строкой в журнал.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Роль съёмочной группы.
pub struct Role {
    pub id: &'static str,
    pub title: &'static str,
    /// Навыки, которые роль грузит (свой `crew-<id>` и общие по её работе).
    pub skills: &'static [&'static str],
    /// Документы, которые роль ведёт.
    pub owns: &'static [&'static str],
    /// Системный промпт роли по умолчанию: кто она, за что отвечает, чем
    /// владеет, кому сдаёт работу, как пишет записки (навыки — отдельно, это знания).
    pub prompt: &'static str,
}

pub const ROLES: &[Role] = &[
    Role {
        id: "director",
        title: "Режиссёр",
        skills: &["crew-director", "video-story", "screenplay"],
        owns: &["01_библия", "03_структура", "журнал"],
        prompt: "Ты — режиссёр съёмочной группы. Отвечаешь за фильм целиком: ведёшь конвейер сцена за сценой (crew.status), поручаешь шаги ролям (crew.run), принимаешь их работу и сам шаги ролей не делаешь. Владеешь документами 01_библия, 03_структура и журнал. Споры по запискам, не решённые за два круга, решаешь сам; ещё два круга — спрашиваешь пользователя и ждёшь ответа. На вехах (сценарий и структура, лукбук эталонов, первая снятая сцена) показываешь результат пользователю. Записки пишешь коротко: кому, сцена, предмет, что не так и что сделать.",
    },
    Role {
        id: "writer",
        title: "Сценарист",
        skills: &["crew-writer", "screenplay", "russian-speech"],
        owns: &["00_оригинал", "scene_NN"],
        prompt: "Ты — сценарист съёмочной группы. Отвечаешь за текст: дословный перевод сцены источника в 00_оригинал и документ сцены scene_NN (адаптация действия и реплик, кадры с ассетами, свет, звук, преемственность). Владеешь 00_оригинал и scene_NN. Работу сдаёшь скрипт-супервайзеру на сверку. Замечания из записок исправляешь и закрываешь записку с итогом (crew.note.set); несогласие — ответной запиской. Пишешь только по источнику и библии, ничего не выдумываешь вопреки им.",
    },
    Role {
        id: "supervisor",
        title: "Скрипт-супервайзер",
        skills: &["crew-supervisor", "screenplay"],
        owns: &["02_сверка"],
        prompt: "Ты — скрипт-супервайзер съёмочной группы. Отвечаешь за соответствие источнику и непрерывность: сверяешь сцену с источником факт за фактом и ведёшь 02_сверка с ✓/✗, проверяешь эталоны и кадры роликов глазами (frame_check). Владеешь 02_сверка. Сам чужую работу не переделываешь: каждое расхождение — записка автору (сценаристу, художнику, оператору) с предметом, фактом источника и тем, что не так. Проверяешь только по источнику и библии, а не по рассуждениям автора.",
    },
    Role {
        id: "designer",
        title: "Художник-постановщик",
        skills: &["crew-designer", "qwen-image"],
        owns: &["04_ассеты"],
        prompt: "Ты — художник-постановщик съёмочной группы. Отвечаешь за внешний вид: базовые эталоны героев, мест и реквизита и их варианты (ракурс, состояние, точка съёмки, время суток) — только правкой базового эталона. Владеешь каталогом 04_ассеты. Работу сдаёшь скрипт-супервайзеру. Внешность, возраст, происхождение, одежда, эпоха и быт — строго по библии и источнику. Записки по ассетам исправляешь и закрываешь с итогом.",
    },
    Role {
        id: "camera",
        title: "Оператор",
        skills: &["crew-camera", "minimax-h3"],
        owns: &["раскадровка"],
        prompt: "Ты — оператор съёмочной группы. Отвечаешь за съёмку: план сцены (scene.plan), промпты роликов MiniMax-H3 по кадрам документа сцены, съёмку ref-scenes и пересъёмку забракованных роликов. Владеешь раскадровкой. Ролики сдаёшь скрипт-супервайзеру на проверку. Подключаешь к ролику только ассеты его кадров. Записки о браке исправляешь пересъёмкой и закрываешь с итогом.",
    },
    Role {
        id: "sound",
        title: "Звукорежиссёр",
        skills: &["crew-sound", "drama3", "music-for-video"],
        owns: &["диалоги"],
        prompt: "Ты — звукорежиссёр съёмочной группы. Отвечаешь за звук: голоса героев (voice.cast) по их карточкам, реплики сцены (voice.say, один дубль, переозвучка только по проверке), звуки окружения сцены и музыкальную тему. Владеешь документом диалоги. Реплики озвучиваешь ровно в утверждённом тексте сцены, ничего не смягчая и не меняя. Недостающие звуки окружения — записка оператору.",
    },
    Role {
        id: "editor",
        title: "Монтажёр",
        skills: &["crew-editor", "montage", "music-for-video"],
        owns: &["монтаж"],
        prompt: "Ты — монтажёр съёмочной группы. Отвечаешь за сборку: склейка роликов сцены на таймлайне по порядку, в конце — монтаж фильма, титры, музыка, экспорт. Владеешь документом монтаж. Работу сдаёшь режиссёру. Ничего не вырезаешь из сюжета без решения режиссёра.",
    },
];

/// Системный промпт роли: свой из настроек или по умолчанию.
pub fn role_system(r: &Role, s: &crate::settings::Settings) -> String {
    s.crew_prompts.get(r.id).map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).unwrap_or_else(|| r.prompt.to_string())
}

/// Модель роли: своя из настроек или модель агента.
pub fn role_model(r: &Role, s: &crate::settings::Settings) -> String {
    s.crew_models.get(r.id).map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).unwrap_or_else(|| s.ai_model.trim().to_string())
}

// ---------------------------------------------------------------- правила проекта

/// Мат разрешён в задании пользователя (упомянут и не запрещён).
pub fn mat_allowed(task: &str) -> bool {
    let low = task.to_lowercase();
    let mention = ["мат", "матом", "матер", "брань", "ругательств"].iter().any(|k| low.contains(k));
    let deny = ["без мата", "мат не ", "не матер", "без брани", "мата не", "мат запрещ", "не ругат"].iter().any(|k| low.contains(k));
    mention && !deny
}

/// Обязательные правила проекта из задания пользователя (файл `правила.md`).
pub fn rules_from_task(task: &str) -> String {
    let mut out = String::from("# Правила проекта (заданы пользователем — обязательны для всех ролей)\n\n");
    out.push_str(&format!("Задание пользователя: «{}»\n\n", task.trim()));
    out.push_str("- Язык, стиль, длина и всё прочее из задания — исполнять как сказано, своими решениями не подменять.\n");
    if mat_allowed(task) {
        out.push_str("- **Мат разрешён пользователем.** Грубую брань оригинала передавай русским матом не слабее оригинала (можно жёстче, если пользователь так сказал), без смягчений, эвфемизмов, звёздочек и пропусков — в документах (включая библию и её правила брани), в репликах, в озвучке и в промптах видео. Самовольное смягчение — ошибка, такую реплику переделывают.\n");
    }
    out
}

/// Записать правила проекта по первому заданию пользователя (если их ещё нет).
pub fn save_rules(project: &Path, task: &str) {
    let p = dir(project).join("правила.md");
    if !p.is_file() && !task.trim().is_empty() {
        let _ = std::fs::write(&p, rules_from_task(task));
    }
}

/// Блок правил проекта для системного промпта (пусто — правил нет).
pub fn rules(project: &Path) -> String {
    std::fs::read_to_string(dir(project).join("правила.md")).unwrap_or_default()
}

pub fn role(id: &str) -> Option<&'static Role> {
    let low = id.trim().to_lowercase();
    ROLES.iter().find(|r| r.id == low || r.title.to_lowercase() == low)
}

/// Шаг конвейера сцены (статус — первый невыполненный шаг).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    Original,
    Script,
    Facts,
    Assets,
    Voice,
    Plan,
    Shoot,
    Check,
    Cut,
    Done,
}

impl Stage {
    pub const ALL: [Stage; 10] = [Stage::Original, Stage::Script, Stage::Facts, Stage::Assets, Stage::Voice, Stage::Plan, Stage::Shoot, Stage::Check, Stage::Cut, Stage::Done];

    pub fn title(self) -> &'static str {
        match self {
            Stage::Original => "оригинал",
            Stage::Script => "сценарий",
            Stage::Facts => "сверка",
            Stage::Assets => "ассеты",
            Stage::Voice => "озвучка",
            Stage::Plan => "раскадровка",
            Stage::Shoot => "съёмка",
            Stage::Check => "проверка",
            Stage::Cut => "склейка",
            Stage::Done => "готово",
        }
    }

    /// Роль-автор и роль-проверяющий шага.
    pub fn roles(self) -> (&'static str, &'static str) {
        match self {
            Stage::Original | Stage::Script => ("writer", "supervisor"),
            Stage::Facts => ("supervisor", "director"),
            Stage::Assets => ("designer", "supervisor"),
            Stage::Voice => ("sound", "writer"),
            Stage::Plan => ("camera", "director"),
            Stage::Shoot => ("camera", "supervisor"),
            Stage::Check => ("supervisor", "director"),
            Stage::Cut | Stage::Done => ("editor", "director"),
        }
    }

    /// Что сделать на шаге (задание роли).
    pub fn task(self, n: u32) -> String {
        match self {
            Stage::Original => format!("Возьми source.scene {n} и допиши в 00_оригинал раздел «## Сцена {n} (стр. …)» — дословный перевод всей сцены 1:1, без сокращений (doc.write append)."),
            Stage::Script => format!("Напиши документ scene_{n:02} по навыку: перевод действия и реплик (`ИМЯ [подача]: «реплика»`), кадры с ассетами @id, свет, звук по кадрам, преемственность, ссылки на раздел 00_оригинал и строку 02_сверка; устрани замечания проверок."),
            Stage::Facts => format!("Сверь scene_{n:02} с источником (source.scene {n}) факт за фактом и запиши строку сцены {n} в 02_сверка с ✓/✗ по каждому факту; ✗ — записка сценаристу (crew.note)."),
            Stage::Assets => format!("Подготовь ассеты сцены {n}: каждый @id из scene_{n:02} — базовый эталон или вариант правкой базового (ref-image с base), строкой в 04_ассеты."),
            Stage::Voice => format!("Озвучь реплики сцены {n} из scene_{n:02}: голоса героев, которых ещё нет (voice.cast), реплики voice.say с scene={n}, один дубль, переозвучка только по проверке."),
            Stage::Plan => format!("План сцены {n}: scene.plan по репликам и действию scene_{n:02}."),
            Stage::Shoot => format!("Сними сцену {n}: ref-scenes по каркасам плана, кадры и ассеты из scene_{n:02}, asset «Сцена {n}»."),
            Stage::Check => format!("Проверь ролики сцены {n}: замечания ответа съёмки (речь, звук), ключевые события scene_{n:02} — frame_check по файлу ролика; брак — записка оператору."),
            Stage::Cut => format!("Склей сцену {n}: ролики на таймлайн по порядку (clip_add), статус и итог — в scene_{n:02} и журнал."),
            Stage::Done => format!("Сцена {n} готова."),
        }
    }
}

fn dir(project: &Path) -> PathBuf {
    crate::store::media_dir(project)
}

fn read(project: &Path, name: &str) -> String {
    std::fs::read_to_string(dir(project).join(format!("{name}.md"))).unwrap_or_default()
}

/// Статус сцены по файлам проекта: первый невыполненный шаг.
pub fn stage(project: &Path, n: u32, timeline: Option<&crate::doc::Project>) -> Stage {
    let src = crate::cmd::doc::source();
    let orig = read(project, "00_оригинал");
    if let Some(s) = &src {
        if crate::cmd::doc::original_check(&orig, s, n).is_some() {
            return Stage::Original;
        }
    }
    let scene = read(project, &format!("scene_{n:02}"));
    if scene.trim().is_empty() {
        return Stage::Script;
    }
    if crate::cmd::story::facts_gate(project, n).is_err() || review_ok(project, n, "script") != Some(true) {
        return Stage::Facts;
    }
    let catalog = read(project, "04_ассеты");
    if crate::cmd::doc::asset_refs(&scene).is_empty() || !crate::cmd::doc::scene_assets_missing(&scene, &catalog).is_empty() || review_ok(project, n, "assets") != Some(true) {
        return Stage::Assets;
    }
    // реплики в сцене есть — нужна их озвучка (файлы speech/сцена-N-*)
    let has_lines = scene.contains('«');
    let voiced = std::fs::read_dir(dir(project).join("speech")).into_iter().flatten().flatten().any(|e| {
        let f = e.file_name().to_string_lossy().to_lowercase();
        (f.starts_with(&format!("сцена-{n}-")) || f.starts_with(&format!("сцена-{n:02}-"))) && !f.contains("-ролик-")
    });
    if has_lines && (!voiced || review_ok(project, n, "dialogue") != Some(true)) {
        return Stage::Voice;
    }
    let story = crate::cmd::story::load(Some(project));
    if !story.planned.contains_key(&n) {
        return Stage::Plan;
    }
    if !story.done.contains(&n) {
        return Stage::Shoot;
    }
    if review_ok(project, n, "shots") != Some(true) || notes(project).iter().any(|x| x.scene == n && x.status != Status::Closed) {
        return Stage::Check;
    }
    if let Some(p) = timeline {
        let tag = format!("сцена {n} —");
        let on = p.tracks.iter().flat_map(|t| &t.clips).any(|c| p.media.iter().any(|m| Some(m.id) == c.media() && m.path.to_string_lossy().to_lowercase().contains(&tag)));
        if !on {
            return Stage::Cut;
        }
    }
    Stage::Done
}

// ---------------------------------------------------------------- проверки (правило двух ключей)

/// Приёмка проверяющим: сценарий сцены, её эталоны, озвучка (диалоги), ролики.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Review {
    pub scene: u32,
    /// script | assets | dialogue | shots
    pub what: String,
    pub ok: bool,
    pub by: String,
    pub text: String,
    pub at: String,
}

/// Что проверяется: (id, название, роль-проверяющий, роль-автор).
pub const REVIEWS: &[(&str, &str, &str, &str)] = &[
    ("script", "сценарий сцены", "supervisor", "writer"),
    ("assets", "эталоны сцены", "supervisor", "designer"),
    ("dialogue", "диалоги и озвучка", "writer", "sound"),
    ("shots", "ролики сцены", "supervisor", "camera"),
];

pub fn reviews(project: &Path) -> Vec<Review> {
    std::fs::read_to_string(dir(project).join("проверки.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Последний вердикт по предмету сцены (None — не проверялось). Проверка
/// устаревает, если проверяемое поменялось позже неё.
pub fn review_ok(project: &Path, n: u32, what: &str) -> Option<bool> {
    let r = reviews(project).into_iter().rev().find(|r| r.scene == n && r.what == what)?;
    let at = chrono_secs(&r.at);
    if r.ok && review_inputs(project, n, what).iter().any(|f| mtime(f) > at + 1) {
        return None;
    }
    Some(r.ok)
}

fn mtime(f: &Path) -> u64 {
    std::fs::metadata(f).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn chrono_secs(s: &str) -> u64 {
    s.parse().unwrap_or(0)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn add_review(project: &Path, n: u32, what: &str, ok: bool, by: &str, text: &str) {
    let mut v = reviews(project);
    v.push(Review { scene: n, what: what.into(), ok, by: by.into(), text: text.trim().into(), at: now_secs().to_string() });
    let _ = std::fs::write(dir(project).join("проверки.json"), serde_json::to_string_pretty(&v).unwrap_or_default());
}

/// Какая проверка ждёт на текущем шаге сцены (автор свою часть сделал).
pub fn review_due(project: &Path, n: u32, st: Stage) -> Option<&'static str> {
    let scene = read(project, &format!("scene_{n:02}"));
    let what = match st {
        Stage::Facts if crate::cmd::story::facts_gate(project, n).is_ok() => "script",
        Stage::Assets if !crate::cmd::doc::asset_refs(&scene).is_empty() && crate::cmd::doc::scene_assets_missing(&scene, &read(project, "04_ассеты")).is_empty() => "assets",
        Stage::Voice if voiced(project, n) => "dialogue",
        Stage::Check => "shots",
        _ => return None,
    };
    (review_ok(project, n, what) != Some(true)).then_some(what)
}

fn voiced(project: &Path, n: u32) -> bool {
    std::fs::read_dir(dir(project).join("speech")).into_iter().flatten().flatten().any(|e| {
        let f = e.file_name().to_string_lossy().to_lowercase();
        (f.starts_with(&format!("сцена-{n}-")) || f.starts_with(&format!("сцена-{n:02}-"))) && !f.contains("-ролик-")
    })
}

/// Ролики сцены: файлы съёмки с меткой сцены (без склеек и хвостов).
pub fn scene_rolls(project: &Path, n: u32) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir(project).join("generated"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("ref-scenes-"))
        .flat_map(|e| std::fs::read_dir(e.path()).into_iter().flatten().flatten().map(|f| f.path()))
        .filter(|f| {
            let name = f.file_name().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
            name.ends_with(".mp4") && name.contains("ролик") && crate::cmd::story::scene_number(&name) == Some(n)
        })
        .collect();
    v.sort();
    v
}

/// Файлы, которые смотрит проверка (по ним же видно, что вещь поменялась).
pub fn review_inputs(project: &Path, n: u32, what: &str) -> Vec<PathBuf> {
    let d = dir(project);
    match what {
        "script" => vec![d.join(format!("scene_{n:02}.md"))],
        "assets" => {
            let scene = read(project, &format!("scene_{n:02}"));
            let cat = crate::cmd::doc::catalog(&read(project, "04_ассеты"));
            crate::cmd::doc::asset_refs(&scene).iter().filter_map(|id| cat.iter().find(|a| &a.id == id)).map(|a| PathBuf::from(&a.file)).filter(|f| f.is_file()).collect()
        }
        "dialogue" => std::fs::read_dir(d.join("speech"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|f| {
                let s = f.file_name().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
                (s.starts_with(&format!("сцена-{n}-")) || s.starts_with(&format!("сцена-{n:02}-"))) && !s.contains("-ролик-")
            })
            .collect(),
        "shots" => scene_rolls(project, n),
        _ => Vec::new(),
    }
}

/// Кадры ролика для проверки зрением: начало, середина, конец (PNG рядом во временной папке).
pub fn roll_frames(roll: &Path, out: &Path) -> Vec<PathBuf> {
    let secs = crate::cmd::story::media_seconds(roll).unwrap_or(0.0);
    let mut v = Vec::new();
    for (k, at) in [0.3, secs / 2.0, (secs - 0.3).max(0.0)].into_iter().enumerate() {
        let f = out.join(format!("{}-{k}.png", roll.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()));
        let ok = std::process::Command::new("ffmpeg").args(["-v", "error", "-y", "-ss", &format!("{at:.2}")]).arg("-i").arg(roll).args(["-frames:v", "1", "-vf", "scale=768:-2"]).arg(&f).status().is_ok_and(|s| s.success());
        if ok && f.is_file() {
            v.push(f);
        }
    }
    v
}

/// Бриф проверяющего со свежим контекстом: только оригинал сцены, выжимка
/// библии с правилами и проверяемая вещь (без рассуждений автора), и
/// вложения для зрения (эталоны, кадры роликов).
pub fn review_brief(project: &Path, n: u32, what: &str) -> (String, Vec<PathBuf>) {
    let title = REVIEWS.iter().find(|r| r.0 == what).map(|r| r.1).unwrap_or(what);
    crate::cmd::doc::load_project_source(project);
    let src = crate::cmd::doc::source().and_then(|s| s.scene_texts.get(n as usize - 1).cloned()).unwrap_or_default();
    let scene = read(project, &format!("scene_{n:02}"));
    let mut files = Vec::new();
    let mut b = format!("# Проверка: {title}, сцена {n}\nТы проверяешь чужую работу свежим взглядом: перед тобой только оригинал сцены, правила и сама вещь. Сравни факт за фактом.\n\n## Оригинал сцены {n} (источник)\n{}\n\n## Документ сцены scene_{n:02}\n{}\n", if src.trim().is_empty() { "(источник не приложен — сверяйся с 00_оригинал)".into() } else { src }, scene.trim());
    match what {
        "script" => b.push_str("\n## Что проверить\n- все события, герои, места и реквизит оригинала на месте, ничего не выдумано;\n- реплики переданы полностью и по смыслу, язык и сила брани — по правилам проекта;\n- нет черновых рассуждений, мусорных символов, ссылок «см. источник» вместо текста;\n- кадры с ассетами @id, свет и звук по кадрам указаны.\n"),
        "assets" => {
            let cat = crate::cmd::doc::catalog(&read(project, "04_ассеты"));
            b.push_str("\n## Эталоны сцены (картинки во вложениях, по порядку)\n");
            for id in crate::cmd::doc::asset_refs(&scene) {
                if let Some(a) = cat.iter().find(|a| a.id == id) {
                    b.push_str(&format!("- @{}: {} {} — {}\n", a.id, a.kind, a.variant, a.file));
                    if std::path::Path::new(&a.file).is_file() {
                        files.push(PathBuf::from(&a.file));
                    }
                }
            }
            b.push_str("\n## Что проверить (смотри картинки)\n- тот ли герой: возраст, пол, телосложение, внешность по библии; тот ли вариант (костюм, рана, грим) для этой сцены;\n- место и время суток как в сцене; эпоха и стиль как в правилах (стиль фильма — одинаковый у всех эталонов);\n- реквизит тот, что в сцене; на картинке нет надписей, рамок, лишних людей;\n- лицо читаемо, герой во весь рост (для героя).\n");
        }
        "dialogue" => {
            b.push_str("\n## Записи реплик сцены\n");
            for f in review_inputs(project, n, "dialogue") {
                let heard = crate::cmd::audio::transcribe_file(&f, Some("ru")).map(|t| t.text).unwrap_or_default();
                b.push_str(&format!("- {}: слышно «{}»\n", f.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default(), heard.trim()));
            }
            b.push_str("\n## Что проверить (редактор диалогов)\n- каждая реплика сцены озвучена, слова совпадают с документом;\n- язык живой, без кальки; род и число согласованы с героем;\n- брань — по правилам проекта, не слабее оригинала;\n- голос подходит герою (пол, возраст), подача по ремарке.\n");
        }
        "shots" => {
            let tmp = std::env::temp_dir().join(format!("syn-video-editor-review-{}-{n}", std::process::id()));
            let _ = std::fs::create_dir_all(&tmp);
            b.push_str("\n## Ролики сцены (кадры во вложениях: начало, середина, конец каждого ролика)\n");
            for r in scene_rolls(project, n) {
                let heard = crate::cmd::audio::transcribe_file(&r, Some("ru")).map(|t| t.text).unwrap_or_default();
                b.push_str(&format!("- {}: речь «{}»\n", r.display(), heard.trim()));
                files.extend(roll_frames(&r, &tmp));
            }
            b.push_str("\n## Что проверить (смотри кадры; при сомнении — frame_check по файлу ролика и секунде)\n- ключевые события сцены в кадре, в нужном порядке;\n- герои те же, что на эталонах, в нужных костюмах; лица видны;\n- место то же, обстановка не плывёт, свет как в сцене; стык роликов без скачка;\n- речь — только реплики сцены, без лишних слов и повторов;\n- нет надписей, мусора, разваленных рук и лиц.\n");
        }
        _ => {}
    }
    b.push_str("\n## Ответ\nПоследней строкой — вердикт ровно в одном из видов:\nВЕРДИКТ: ✓\nВЕРДИКТ: ✗ — <что не так, по пунктам, конкретно: что и где исправить>\n");
    (b, files)
}

/// Вердикт из ответа проверяющего: Some(true) — ✓, Some(false) — ✗, None — нет вердикта.
pub fn parse_verdict(text: &str) -> Option<bool> {
    let line = text.lines().rev().find(|l| l.to_uppercase().contains("ВЕРДИКТ"))?;
    if line.contains('✗') || line.to_lowercase().contains("не принят") {
        Some(false)
    } else if line.contains('✓') {
        Some(true)
    } else {
        None
    }
}

// ---------------------------------------------------------------- состав под формат

/// Форматы: (id, название, роли состава).
pub const FORMATS: &[(&str, &str, &[&str])] = &[
    ("film", "фильм", &["director", "writer", "supervisor", "designer", "camera", "sound", "editor"]),
    ("cartoon", "мультфильм", &["director", "writer", "supervisor", "designer", "camera", "sound", "editor"]),
    ("clip", "клип", &["director", "supervisor", "designer", "camera", "sound", "editor"]),
    ("reels", "рилс", &["director", "designer", "camera", "editor"]),
    ("podcast", "подкаст", &["director", "writer", "sound", "editor"]),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lineup {
    pub format: String,
    pub roles: Vec<String>,
}

impl Default for Lineup {
    fn default() -> Self {
        Lineup { format: "film".into(), roles: FORMATS[0].2.iter().map(|s| s.to_string()).collect() }
    }
}

pub fn lineup(project: &Path) -> Lineup {
    std::fs::read_to_string(dir(project).join("состав.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save_lineup(project: &Path, l: &Lineup) {
    let _ = std::fs::write(dir(project).join("состав.json"), serde_json::to_string_pretty(l).unwrap_or_default());
}

/// Кто в составе выполняет работу роли: сама роль или, если её нет в
/// составе, режиссёр.
pub fn who(project: &Path, role_id: &'static str) -> &'static str {
    if role_id == "director" || lineup(project).roles.iter().any(|r| r == role_id) {
        role_id
    } else {
        "director"
    }
}

// ---------------------------------------------------------------- прерванный ход (кнопка «Продолжить»)

/// Ход, который шёл, когда окно остановили или закрыли: роль (или режиссёр — "director"
/// для главного чата), сцена и задание. Пишется при старте хода, стирается при обычном конце.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Pending {
    pub role: String,
    #[serde(default)]
    pub scene: u32,
    #[serde(default)]
    pub stage: String,
    pub task: String,
}

fn pending_path(project: &Path, main: bool) -> PathBuf {
    dir(project).join(if main { "ход-режиссёра.json" } else { "ход-роли.json" })
}

pub fn set_pending(project: &Path, main: bool, p: Option<&Pending>) {
    let f = pending_path(project, main);
    match p {
        Some(p) => {
            let _ = std::fs::write(&f, serde_json::to_string_pretty(p).unwrap_or_default());
        }
        None => {
            let _ = std::fs::remove_file(&f);
        }
    }
}

pub fn pending(project: &Path, main: bool) -> Option<Pending> {
    std::fs::read_to_string(pending_path(project, main)).ok().and_then(|t| serde_json::from_str(&t).ok())
}

// ---------------------------------------------------------------- продюсер

/// Сколько роликов сцены можно снять сверх плана (пересъёмки), во сколько раз.
pub const RETAKES: usize = 3;

/// Лимит пересъёмок: снято роликов сцены больше плана × RETAKES — дальше
/// только с согласия пользователя (записка ему).
pub fn retake_limit(project: &Path, n: u32) -> Option<String> {
    let s = crate::cmd::story::load(Some(project));
    let planned = *s.planned.get(&n)?;
    let shot = scene_rolls(project, n).len();
    (planned > 0 && shot >= planned * RETAKES).then(|| {
        let _ = add_note(project, "director", "user", n, "лимит пересъёмок", &format!("Сцена {n}: снято {shot} роликов при плане {planned} — лимит пересъёмок ({RETAKES}×) исчерпан. Что не так — в проверках и записках сцены; переснимать ещё?"));
        format!("Сцена {n}: лимит пересъёмок исчерпан ({shot} роликов при плане {planned}, предел {}). Спроси пользователя, переснимать ли ещё, и дождись ответа.", planned * RETAKES)
    })
}

/// Скорость съёмки по прошлым прогонам: секунд работы на секунду ролика.
pub fn shoot_rate(project: &Path) -> Option<f64> {
    let (mut work, mut video) = (0.0f64, 0.0f64);
    for e in std::fs::read_dir(dir(project).join("generated")).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(ms) = name.strip_prefix("ref-scenes-").and_then(|x| x.parse::<u64>().ok()) else { continue };
        let rep = e.path().join("report.json");
        let Ok(end) = std::fs::metadata(&rep).and_then(|m| m.modified()) else { continue };
        let end = end.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let rolls: f64 = std::fs::read_dir(e.path()).into_iter().flatten().flatten().map(|f| f.path()).filter(|f| f.to_string_lossy().contains("ролик") && f.extension().is_some_and(|x| x == "mp4")).filter_map(|f| crate::cmd::story::media_seconds(&f).ok()).sum();
        if end > ms && rolls > 0.0 {
            work += (end - ms) as f64 / 1000.0;
            video += rolls;
        }
    }
    (video > 0.0).then(|| work / video)
}

/// Оценка оставшейся съёмки: секунды неснятых сцен по структуре × скорость.
pub fn estimate_left(project: &Path, timeline: Option<&crate::doc::Project>) -> Option<String> {
    let rate = shoot_rate(project)?;
    let structure = read(project, "03_структура");
    let n = crate::cmd::crew::scene_count(project);
    let mut left = 0.0;
    for s in 1..=n {
        if stage(project, s, timeline) >= Stage::Check {
            continue;
        }
        let row = structure.lines().find(|l| {
            let c = l.trim().trim_start_matches('|').split('|').next().unwrap_or("").trim().to_lowercase();
            c.trim_start_matches("сцена").trim().parse::<u32>().ok() == Some(s)
        });
        let secs = row.and_then(|r| r.rsplit('|').map(|c| c.trim().trim_start_matches('~')).find_map(|c| c.parse::<f64>().ok())).unwrap_or(30.0);
        left += secs;
    }
    let min = left * rate / 60.0;
    Some(format!("съёмка: ≈{:.0} с работы на секунду ролика, осталось ≈{:.0} мин ({:.0} с фильма)", rate, min, left))
}

// ---------------------------------------------------------------- вехи согласования

/// Вехи: (id, название, что показать пользователю).
pub const MILESTONES: &[(&str, &str, &str)] = &[
    ("script", "сценарий", "библию и структуру фильма (01_библия, 03_структура)"),
    ("lookbook", "лукбук", "все базовые эталоны героев, мест и реквизита (04_ассеты) — одной подборкой"),
    ("first_scene", "первая сцена", "снятую и склеенную первую сцену"),
];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Milestones {
    pub enabled: bool,
    #[serde(default)]
    pub approved: Vec<String>,
}

pub fn milestones(project: &Path) -> Milestones {
    std::fs::read_to_string(dir(project).join("вехи.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save_milestones(project: &Path, m: &Milestones) {
    let _ = std::fs::write(dir(project).join("вехи.json"), serde_json::to_string_pretty(m).unwrap_or_default());
}

/// Веха, которая ждёт согласия пользователя (вехи включены, работа до неё сделана).
pub fn milestone_due(project: &Path, timeline: Option<&crate::doc::Project>) -> Option<&'static (&'static str, &'static str, &'static str)> {
    let m = milestones(project);
    if !m.enabled {
        return None;
    }
    let pending = |id: &str| !m.approved.iter().any(|a| a == id);
    let d = dir(project);
    let has = |f: &str| std::fs::read_to_string(d.join(f)).is_ok_and(|t| !t.trim().is_empty());
    let cat = crate::cmd::doc::catalog(&read(project, "04_ассеты"));
    let s1 = stage(project, 1, timeline);
    for ms in MILESTONES {
        let ready = match ms.0 {
            "script" => has("01_библия.md") && has("03_структура.md"),
            "lookbook" => !cat.is_empty() && cat.iter().all(|a| std::path::Path::new(&a.file).is_file()) && s1 > Stage::Assets,
            "first_scene" => s1 == Stage::Done,
            _ => false,
        };
        if ready && pending(ms.0) {
            return Some(ms);
        }
    }
    None
}

// ---------------------------------------------------------------- записки

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Open,
    Work,
    Closed,
}

/// Записка между ролями.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: u32,
    pub from: String,
    pub to: String,
    pub scene: u32,
    /// Предмет (что именно: «scene_03 место», «ассет @x»): по нему считаются круги.
    pub subject: String,
    pub text: String,
    pub status: Status,
    /// Сколько раз предмет возвращался на правку.
    pub rounds: u32,
    #[serde(default)]
    pub result: String,
}

fn notes_path(project: &Path) -> PathBuf {
    dir(project).join("записки.json")
}

pub fn notes(project: &Path) -> Vec<Note> {
    std::fs::read(notes_path(project)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_notes(project: &Path, v: &[Note]) -> Result<(), String> {
    std::fs::write(notes_path(project), serde_json::to_vec_pretty(v).unwrap_or_default()).map_err(|e| e.to_string())
}

/// Кругов «замечание → правка» до эскалации режиссёру, затем пользователю.
pub const ROUNDS: u32 = 2;

/// Новая записка; тот же предмет снова — следующий круг; больше двух — режиссёру, ещё два — пользователю.
pub fn add_note(project: &Path, from: &str, to: &str, scene: u32, subject: &str, text: &str) -> Result<Note, String> {
    let mut v = notes(project);
    let subject = if subject.trim().is_empty() { format!("сцена {scene}") } else { subject.trim().to_string() };
    let prev = v.iter().filter(|x| x.scene == scene && x.subject == subject).map(|x| x.rounds).max();
    let rounds = prev.map(|r| r + 1).unwrap_or(1);
    let to = if rounds > ROUNDS * 2 {
        "user".to_string()
    } else if rounds > ROUNDS {
        "director".to_string()
    } else {
        to.to_string()
    };
    let id = v.iter().map(|x| x.id).max().unwrap_or(0) + 1;
    let n = Note { id, from: from.into(), to, scene, subject, text: text.into(), status: Status::Open, rounds, result: String::new() };
    v.push(n.clone());
    save_notes(project, &v)?;
    Ok(n)
}

pub fn set_note(project: &Path, id: u32, status: Status, result: &str) -> Result<Note, String> {
    let mut v = notes(project);
    let n = v.iter_mut().find(|x| x.id == id).ok_or_else(|| format!("нет записки {id}"))?;
    n.status = status;
    if !result.trim().is_empty() {
        n.result = result.trim().to_string();
    }
    let out = n.clone();
    save_notes(project, &v)?;
    Ok(out)
}

pub fn note_line(n: &Note) -> String {
    let st = match n.status {
        Status::Open => "открыта",
        Status::Work => "в работе",
        Status::Closed => "закрыта",
    };
    let who = |id: &str| role(id).map(|r| r.title).unwrap_or(if id == "user" { "пользователь" } else { "?" }).to_string();
    format!("#{} [{st}, круг {}] {} → {}, сцена {}, {}: {}{}", n.id, n.rounds, who(&n.from), who(&n.to), n.scene, n.subject, n.text, if n.result.is_empty() { String::new() } else { format!(" — итог: {}", n.result) })
}

// ---------------------------------------------------------------- бриф

/// Разделы документа уровня `##`/`###` (заголовок, текст).
fn sections(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for l in text.lines() {
        if l.starts_with("##") {
            out.push((l.trim_start_matches('#').trim().to_string(), String::new()));
        } else if let Some((_, body)) = out.last_mut() {
            body.push_str(l);
            body.push('\n');
        }
    }
    out
}

/// Карточки библии (герои, места, реквизит), упомянутые в тексте сцены.
pub fn cards_for(bible: &str, scene_text: &str) -> Vec<String> {
    let low = scene_text.to_lowercase();
    sections(bible)
        .into_iter()
        .filter(|(h, _)| {
            // имя в заголовке карточки (первое слово-имя) встречается в сцене
            h.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 3 && w.chars().next().is_some_and(char::is_uppercase)).any(|w| low.contains(&w.to_lowercase()))
        })
        .map(|(h, b)| format!("### {h}\n{}", b.trim()))
        .collect()
}

/// Выжимка проекта — общая и стабильная часть контекста всех ходов (голова,
/// которую служба кэширует): шапка библии, словарь имён, стиль, правила, структура.
pub fn summary(project: &Path) -> String {
    let mut out = String::from("# Выжимка проекта\n");
    let bible = read(project, "01_библия");
    let head: String = bible.lines().take_while(|l| !l.starts_with("##")).collect::<Vec<_>>().join("\n");
    if !head.trim().is_empty() {
        out.push_str(&format!("{}\n", head.trim()));
    }
    for (h, b) in sections(&bible) {
        let hl = h.to_lowercase();
        if ["имен", "имён", "стиль", "мат", "брань", "правил"].iter().any(|k| hl.contains(k)) {
            out.push_str(&format!("\n### {h}\n{}\n", b.trim()));
        }
    }
    let structure = read(project, "03_структура");
    if !structure.trim().is_empty() {
        out.push_str(&format!("\n## Структура фильма\n{}\n", structure.trim()));
    }
    out
}

/// Бриф шага роли: выжимка проекта и бриф сцены.
pub fn brief(project: &Path, role_id: &str, n: u32, st: Stage) -> String {
    format!("{}\n{}", summary(project), scene_brief(project, role_id, n, st))
}

/// Бриф сцены: задача шага и только нужное для сцены `n` (меняется от шага к шагу).
pub fn scene_brief(project: &Path, role_id: &str, n: u32, st: Stage) -> String {
    let r = role(role_id);
    let mut out = String::new();
    out.push_str(&format!("# Бриф: {}, сцена {n}, шаг «{}»\n\n", r.map(|r| r.title).unwrap_or(role_id), st.title()));
    out.push_str(&format!("**Задача:** {}\n\n", st.task(n)));
    let bible = read(project, "01_библия");
    // место сцены в структуре
    let structure = read(project, "03_структура");
    let row = structure.lines().find(|l| {
        let c = l.trim().trim_start_matches('|').split('|').next().unwrap_or("").trim().to_lowercase();
        c.trim_start_matches("сцена").trim().split(|ch: char| !ch.is_ascii_digit()).next() == Some(&n.to_string())
    });
    if let Some(row) = row {
        out.push_str(&format!("\n## Сцена в структуре\n{}\n", row.trim()));
    }
    // текст сцены источника
    let src_text = crate::cmd::doc::source().and_then(|s| s.scene_texts.get(n as usize - 1).cloned()).unwrap_or_default();
    if !src_text.is_empty() {
        out.push_str(&format!("\n## Сцена {n} источника\n{}\n", src_text.trim()));
    }
    let scene = read(project, &format!("scene_{n:02}"));
    if !scene.trim().is_empty() {
        out.push_str(&format!("\n## Документ сцены scene_{n:02}\n{}\n", scene.trim()));
    }
    // карточки героев, мест, реквизита сцены
    let cards = cards_for(&bible, &format!("{src_text}\n{scene}"));
    if !cards.is_empty() {
        out.push_str(&format!("\n## Карточки сцены\n{}\n", cards.join("\n\n")));
    }
    // ассеты сцены
    let catalog = read(project, "04_ассеты");
    let refs = crate::cmd::doc::asset_refs(&scene);
    let rows: Vec<String> = crate::cmd::doc::catalog(&catalog).into_iter().filter(|a| refs.contains(&a.id)).map(|a| format!("- @{} — {}, {}: {}", a.id, a.kind, a.variant, a.file)).collect();
    if !rows.is_empty() {
        out.push_str(&format!("\n## Ассеты сцены\n{}\n", rows.join("\n")));
    }
    // открытые записки этой роли и сцены
    let open: Vec<String> = notes(project).into_iter().filter(|x| x.status != Status::Closed && x.scene == n && (x.to == role_id || role_id == "director")).map(|x| format!("- {}", note_line(&x))).collect();
    if !open.is_empty() {
        out.push_str(&format!("\n## Открытые записки\n{}\n", open.join("\n")));
    }
    out
}

/// Строка в журнал проекта.
/// Строки журнала проекта о роли (по её названию).
pub fn journal_of(project: &Path, title: &str) -> Vec<String> {
    if title.is_empty() {
        return Vec::new();
    }
    read(project, "журнал").lines().filter(|l| l.contains(title)).map(|l| l.trim_start_matches("- ").to_string()).collect()
}

pub fn journal(project: &Path, line: &str) {
    let p = dir(project).join("журнал.md");
    let mut body = std::fs::read_to_string(&p).unwrap_or_else(|_| "# Журнал\n\n".into());
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (h, m) = (((t / 3600) + 5) % 24, (t / 60) % 60);
    body.push_str(&format!("- {h:02}:{m:02} {line}\n"));
    let _ = std::fs::write(&p, body);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_and_stages() {
        assert!(role("writer").is_some() && role("Оператор").is_some());
        for st in Stage::ALL {
            let (a, b) = st.roles();
            assert!(role(a).is_some() && role(b).is_some(), "{st:?}");
        }
        assert!(Stage::Original < Stage::Done);
    }

    #[test]
    fn two_keys_reviews() {
        assert_eq!(parse_verdict("всё хорошо\nВЕРДИКТ: ✓"), Some(true));
        assert_eq!(parse_verdict("ВЕРДИКТ: ✗ — у героя не тот костюм"), Some(false));
        assert_eq!(parse_verdict("без вердикта"), None);
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t").join("t.syn");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "{}").unwrap();
        let sc = p.parent().unwrap().join("scene_01.md");
        std::fs::write(&sc, "# Сцена 1").unwrap();
        assert_eq!(review_ok(&p, 1, "script"), None);
        add_review(&p, 1, "script", false, "supervisor", "ВЕРДИКТ: ✗ — нет события");
        assert_eq!(review_ok(&p, 1, "script"), Some(false));
        add_review(&p, 1, "script", true, "supervisor", "ВЕРДИКТ: ✓");
        assert_eq!(review_ok(&p, 1, "script"), Some(true));
        // документ поменялся после приёмки — нужна новая проверка
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(10);
        std::fs::File::options().write(true).open(&sc).unwrap().set_modified(later).unwrap();
        assert_eq!(review_ok(&p, 1, "script"), None);
    }

    #[test]
    fn lineup_and_versions() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("l").join("l.syn");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        assert_eq!(who(&p, "writer"), "writer");
        save_lineup(&p, &Lineup { format: "clip".into(), roles: FORMATS[2].2.iter().map(|s| s.to_string()).collect() });
        assert_eq!(who(&p, "writer"), "director");
        assert_eq!(who(&p, "camera"), "camera");
        let doc = p.parent().unwrap().join("scene_01.md");
        assert_eq!(crate::cmd::doc::save_version(&doc, "новое"), None);
        std::fs::write(&doc, "первая").unwrap();
        assert_eq!(crate::cmd::doc::save_version(&doc, "вторая"), Some(1));
        assert_eq!(crate::cmd::doc::save_version(&doc, "первая"), None);
        std::fs::write(&doc, "вторая").unwrap();
        assert_eq!(crate::cmd::doc::save_version(&doc, "третья"), Some(2));
        let v = crate::cmd::doc::versions(&doc);
        assert_eq!(v.len(), 2);
        assert_eq!(std::fs::read_to_string(&v[0].1).unwrap(), "первая");
    }

    #[test]
    fn milestones_gate() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("m").join("m.syn");
        let dir = p.parent().unwrap();
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("01_библия.md"), "# Б").unwrap();
        std::fs::write(dir.join("03_структура.md"), "# С").unwrap();
        // выключены — не держат
        assert!(milestone_due(&p, None).is_none());
        save_milestones(&p, &Milestones { enabled: true, approved: vec![] });
        assert_eq!(milestone_due(&p, None).map(|m| m.0), Some("script"));
        save_milestones(&p, &Milestones { enabled: true, approved: vec!["script".into()] });
        assert!(milestone_due(&p, None).is_none());
    }

    #[test]
    fn project_rules_and_mat() {
        assert!(mat_allowed("Мат — на русском как есть, можно жёстче"));
        assert!(!mat_allowed("сними фильм без мата"));
        assert!(!mat_allowed("сними фильм"));
        assert!(rules_from_task("мат как есть").contains("Мат разрешён пользователем"));
        assert!(!rules_from_task("без мата").contains("Мат разрешён"));
        let mut s = crate::settings::Settings::default();
        let r = role("writer").unwrap();
        assert!(role_system(r, &s).starts_with("Ты — сценарист"));
        s.crew_prompts.insert("writer".into(), "свой".into());
        assert_eq!(role_system(r, &s), "свой");
    }

    #[test]
    fn notes_escalate_after_two_rounds() {
        let d = tempfile::tempdir().unwrap();
        let proj = d.path().join("p").join("p.syn");
        std::fs::create_dir_all(proj.parent().unwrap()).unwrap();
        let a = add_note(&proj, "supervisor", "writer", 2, "место", "не та комната").unwrap();
        assert_eq!((a.rounds, a.to.as_str()), (1, "writer"));
        add_note(&proj, "supervisor", "writer", 2, "место", "всё ещё не та").unwrap();
        let c = add_note(&proj, "supervisor", "writer", 2, "место", "третий раз").unwrap();
        assert_eq!((c.rounds, c.to.as_str()), (3, "director"));
        add_note(&proj, "supervisor", "writer", 2, "место", "4").unwrap();
        let e = add_note(&proj, "supervisor", "writer", 2, "место", "5").unwrap();
        assert_eq!(e.to, "user");
        let closed = set_note(&proj, a.id, Status::Closed, "исправлено").unwrap();
        assert_eq!(closed.status, Status::Closed);
        assert!(note_line(&closed).contains("итог: исправлено"));
    }

    #[test]
    fn cards_by_names_in_scene() {
        let bible = "# Фильм\n## Герои\n### Анна — мать\nвнешность А\n### Борис — сосед\nвнешность Б\n## Места\n### Кухня Анны\nсветлая\n";
        let c = cards_for(bible, "Анна входит на кухню");
        assert!(c.iter().any(|x| x.contains("Анна — мать")));
        assert!(!c.iter().any(|x| x.contains("Борис")));
    }

    #[test]
    fn stage_from_files() {
        let d = tempfile::tempdir().unwrap();
        let proj = d.path().join("p").join("p.syn");
        let dir = proj.parent().unwrap();
        std::fs::create_dir_all(dir).unwrap();
        assert_eq!(stage(&proj, 1, None), Stage::Script);
        std::fs::write(dir.join("scene_01.md"), "## Сцена 1\nкадр @кухня\nАННА: «Привет.»").unwrap();
        // сценарий написан — ждёт приёмки проверяющим (правило двух ключей)
        assert_eq!(stage(&proj, 1, None), Stage::Facts);
        assert_eq!(review_due(&proj, 1, Stage::Facts), Some("script"));
        add_review(&proj, 1, "script", true, "supervisor", "ВЕРДИКТ: ✓");
        assert_eq!(stage(&proj, 1, None), Stage::Assets);
        assert_eq!(review_due(&proj, 1, Stage::Assets), None);
        let f = dir.join("k.png");
        std::fs::write(&f, b"x").unwrap();
        std::fs::write(dir.join("04_ассеты.md"), format!("| id | тип | вариант | base | файл | сцены |\n|---|---|---|---|---|---|\n| кухня | локация | базовый | — | {} | 1 |\n", f.display())).unwrap();
        assert_eq!(stage(&proj, 1, None), Stage::Assets);
        assert_eq!(review_due(&proj, 1, Stage::Assets), Some("assets"));
        add_review(&proj, 1, "assets", true, "supervisor", "ВЕРДИКТ: ✓");
        assert_eq!(stage(&proj, 1, None), Stage::Voice);
        std::fs::create_dir_all(dir.join("speech")).unwrap();
        std::fs::write(dir.join("speech").join("сцена-1-анна-1-01.wav"), b"x").unwrap();
        assert_eq!(review_due(&proj, 1, Stage::Voice), Some("dialogue"));
        add_review(&proj, 1, "dialogue", true, "writer", "ВЕРДИКТ: ✓");
        assert_eq!(stage(&proj, 1, None), Stage::Plan);
    }
}

#[cfg(test)]
mod real {
    /// `SYN_PROJ=проект.syn SYN_SCENE=1 cargo test -p syn-video-editor crew::real -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_brief() {
        let p = std::path::PathBuf::from(std::env::var("SYN_PROJ").unwrap());
        let n: u32 = std::env::var("SYN_SCENE").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
        let st = super::stage(&p, n, None);
        let b = super::brief(&p, st.roles().0, n, st);
        let all: usize = std::fs::read_dir(crate::store::media_dir(&p)).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "md")).map(|e| std::fs::read_to_string(e.path()).unwrap_or_default().chars().count()).sum();
        println!("шаг {:?}, бриф {} знаков, все документы {all} знаков\n{b}", st, b.chars().count());
    }
}
