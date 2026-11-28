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
    if crate::cmd::story::facts_gate(project, n).is_err() {
        return Stage::Facts;
    }
    let catalog = read(project, "04_ассеты");
    if crate::cmd::doc::asset_refs(&scene).is_empty() || !crate::cmd::doc::scene_assets_missing(&scene, &catalog).is_empty() {
        return Stage::Assets;
    }
    // реплики в сцене есть — нужна их озвучка (файлы speech/сцена-N-*)
    let has_lines = scene.contains('«');
    let voiced = std::fs::read_dir(dir(project).join("speech")).into_iter().flatten().flatten().any(|e| {
        let f = e.file_name().to_string_lossy().to_lowercase();
        f.starts_with(&format!("сцена-{n}-")) && !f.contains("-ролик-")
    });
    if has_lines && !voiced {
        return Stage::Voice;
    }
    let story = crate::cmd::story::load(Some(project));
    if !story.planned.contains_key(&n) {
        return Stage::Plan;
    }
    if !story.done.contains(&n) {
        return Stage::Shoot;
    }
    if notes(project).iter().any(|x| x.scene == n && x.status != Status::Closed) {
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
        // без 00_оригинал/02_сверка сверка не требуется — дальше ассеты
        assert_eq!(stage(&proj, 1, None), Stage::Assets);
        let f = dir.join("k.png");
        std::fs::write(&f, b"x").unwrap();
        std::fs::write(dir.join("04_ассеты.md"), format!("| id | тип | вариант | base | файл | сцены |\n|---|---|---|---|---|---|\n| кухня | локация | базовый | — | {} | 1 |\n", f.display())).unwrap();
        assert_eq!(stage(&proj, 1, None), Stage::Voice);
        std::fs::create_dir_all(dir.join("speech")).unwrap();
        std::fs::write(dir.join("speech").join("сцена-1-анна-1-01.wav"), b"x").unwrap();
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
