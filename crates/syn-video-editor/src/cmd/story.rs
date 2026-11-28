//! Фильм сцена за сценой: план съёмки сцены по готовой озвучке и порядок.
//!
//! `scene.plan` — после озвучки сцены: длина сцены = большее из «нужно
//! действию» (сценарий) и «нужно речи» (реплики + паузы); сцена длиннее
//! предела ролика H3 делится на ролики только в тишине между репликами;
//! для каждого ролика — дорожки говорящих по его окну: H3 (fully_copy)
//! вставляет их в звук ролика и синхронизирует губы, поэтому звук сцены —
//! это звук роликов (отдельная дорожка речи поверх дублировала бы речь и
//! расходилась бы с роликом). План — в документ «раскадровка».
//!
//! Порядок — в коде: озвучка, план и съёмка сцены N+1 отказывают, пока сцена
//! N не снята (если пользователь не просил «все сцены разом» — `story.mode`).
//! Состояние — `<папка проекта>/story.json`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{arg, req, Args, Command as Cmd, Out, Ty};

pub static COMMANDS: &[&Cmd] = &[&PLAN, &MODE, &STATUS];

/// Долгая (ffmpeg по дорожкам) — без блокировки документа.
pub const HEAVY: &[&str] = &["scene.plan"];

/// Предел длины одного ролика MiniMax-H3, с (решение пользователя: 18 с при любом разрешении).
pub const SHOT_LIMIT: f64 = 18.0;
const LEAD: f64 = 0.4;
const GAP: f64 = 0.5;
const TAIL: f64 = 0.6;

/// Длина ролика по сетке кадров MiniMax-H3 (5 + 17·k кадров при 24 кадр/с),
/// вверх: служба округляет длину ролика так же, и если дорожка короче ролика,
/// в хвосте без референса H3 досочиняет речь (повторяет реплику). Дорожка
/// ролика — ровно на всю его длину.
pub fn h3_seconds(s: f64) -> f64 {
    let f = ((s * 24.0).round() as usize).max(5);
    let k = (f - 5).div_ceil(17);
    (17 * k + 5) as f64 / 24.0
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Story {
    /// Все сцены разом (только по прямой просьбе пользователя).
    #[serde(default)]
    pub all_at_once: bool,
    /// Снятые сцены.
    #[serde(default)]
    pub done: BTreeSet<u32>,
    /// Планы сцен: число роликов.
    #[serde(default)]
    pub planned: BTreeMap<u32, usize>,
    /// Снято роликов по сценам.
    #[serde(default)]
    pub shot: BTreeMap<u32, usize>,
    /// Хвост латента последнего ролика сцены — следующая сцена той же
    /// локации продолжается с него (бесшовно между прогонами).
    #[serde(default)]
    pub tail: BTreeMap<u32, PathBuf>,
}

fn file(project: &Path) -> PathBuf {
    crate::store::media_dir(project).join("story.json")
}

pub fn load(project: Option<&Path>) -> Story {
    project.and_then(|p| std::fs::read_to_string(file(p)).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(project: &Path, s: &Story) {
    let f = file(project);
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, serde_json::to_string_pretty(s).unwrap_or_default());
}

/// Номер сцены из «3», «Сцена 3», «Сцена 3 из 8: …».
pub fn scene_number(s: &str) -> Option<u32> {
    let t = s.trim();
    let low = t.to_lowercase();
    // «Сцена 1», «Сцена-01», «сцена_3», «Scene 2», «3»
    let t = ["сцена", "scene"].iter().find_map(|p| low.strip_prefix(p)).unwrap_or(&low).trim_start_matches(['-', '_', ' ', '№', '#']);
    let d: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    d.parse().ok()
}

/// Можно ли работать со сценой `n`: снята каждая сцена до неё (или режим «все разом»).
pub fn gate(project: Option<&Path>, n: u32, what: &str) -> Result<(), String> {
    let Some(p) = project else { return Ok(()) };
    // озвучка, план и съёмка сцены — только после сверки её фактов с источником
    if what != "Документ" {
        facts_gate(p, n)?;
    }
    let s = load(Some(p));
    if s.all_at_once || n <= 1 {
        return Ok(());
    }
    if let Some(m) = (1..n).find(|m| !s.done.contains(m)) {
        return Err(format!(
            "{what} сцены {n} — рано: фильм делается сцена за сценой, сначала доведи и сними сцену {m} (её реплики → scene.plan → ref-scenes). Все сцены разом — только если пользователь прямо попросил (story.mode all_at_once)."
        ));
    }
    Ok(())
}

/// Хвост сцены `n`, если файл на месте.
pub fn tail_of(project: Option<&Path>, n: u32) -> Option<PathBuf> {
    load(project).tail.get(&n).filter(|p| p.is_file()).cloned()
}

/// Запомнить хвост сцены `n`.
pub fn set_tail(project: Option<&Path>, n: u32, t: &Path) {
    let Some(p) = project else { return };
    let mut s = load(Some(p));
    s.tail.insert(n, t.to_path_buf());
    save(p, &s);
}

/// Строки сцены `n` в таблице сверки (`02_сверка.md`): первая ячейка — номер.
pub fn facts_rows(table: &str, n: u32) -> Vec<String> {
    table
        .lines()
        .filter(|l| l.trim_start().starts_with('|'))
        .filter(|l| {
            let cell = l.trim().trim_start_matches('|').split('|').next().unwrap_or("").trim().to_lowercase();
            let d: String = cell.trim_start_matches("сцена").trim().chars().take_while(|c| c.is_ascii_digit()).collect();
            d.parse::<u32>().ok() == Some(n)
        })
        .map(|l| l.trim().to_string())
        .collect()
}

/// Сверка фактов сцены `n` пройдена: в проекте по готовому сценарию (есть
/// `00_оригинал` или `02_сверка`) строка сцены есть и в ней нет ✗.
pub fn facts_gate(project: &Path, n: u32) -> Result<(), String> {
    let dir = crate::store::media_dir(project);
    let table = dir.join("02_сверка.md");
    if !table.is_file() && !dir.join("00_оригинал.md").is_file() {
        return Ok(());
    }
    let text = std::fs::read_to_string(&table).unwrap_or_default();
    let rows = facts_rows(&text, n);
    if rows.is_empty() {
        return Err(format!("Сцена {n}: нет строки в 02_сверка — сначала заполни её (место и чьё оно, кто в кадре, что делают, реквизит — где и у кого, кто притворяется или скрывается; ✓ или ✗ по каждому факту против источника)"));
    }
    if let Some(bad) = rows.iter().find(|r| r.contains('✗')) {
        return Err(format!("Сцена {n}: в 02_сверка есть ✗ — исправь scene_{n:02} по источнику (source.scene {n}) и обнови строку сверки: {bad}"));
    }
    Ok(())
}

/// Сняты ещё `k` роликов сцены `n`; сцена снята, когда сняты все ролики её
/// плана (без плана — с первого успешного ref-scenes).
pub fn add_shots(project: Option<&Path>, n: u32, k: usize) {
    let Some(p) = project else { return };
    let mut s = load(Some(p));
    let got = s.shot.entry(n).or_insert(0);
    *got += k;
    if *got >= s.planned.get(&n).copied().unwrap_or(1) {
        s.done.insert(n);
    }
    save(p, &s);
}

/// Реплика сцены в плане (или её кусок в ролике).
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub speaker: String,
    pub file: PathBuf,
    /// Начало звучащей части в файле (пустота в начале записи отрезана).
    pub offset: f64,
    /// Длина звучащей части.
    pub seconds: f64,
    /// Паузы внутри реплики (от её начала): где можно аккуратно разрезать.
    pub pauses: Vec<(f64, f64)>,
    /// Начало в сцене (у куска в ролике — от начала ролика).
    pub at: f64,
    /// Сколько секунд действия без речи перед репликой (по сценарию: герой
    /// бежит, готовит, падает — и только потом говорит); 0 — обычная пауза.
    pub before: f64,
}

/// Ролик плана: окно сцены и куски реплик в нём.
#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub start: f64,
    pub seconds: f64,
    pub lines: Vec<Line>,
    /// Окна без речи от начала ролика (от 1 с): время для действия.
    pub quiet: Vec<(f64, f64)>,
}

/// Раскладка сцены: реплики по порядку с паузами, длина — большее из
/// действия и речи, деление на ролики ≤ `limit` только в тишине — между
/// репликами или в паузе внутри длинной фразы (запись режется, а не
/// переозвучивается). Без единой паузы в пределе — разрез на границе предела.
pub fn layout(mut lines: Vec<Line>, action: f64, limit: f64) -> Result<(f64, Vec<Line>, Vec<Shot>), String> {
    let mut t = 0.0;
    for (i, l) in lines.iter_mut().enumerate() {
        t += if i > 0 { GAP } else { LEAD }.max(l.before);
        l.at = t;
        t += l.seconds;
    }
    let speech = if lines.is_empty() { 0.0 } else { t + TAIL };
    let total = speech.max(action).max(2.0);
    // тишина: до первой реплики, между репликами, паузы внутри реплик, после последней
    let mut free: Vec<(f64, f64)> = Vec::new();
    let mut prev = 0.0;
    for l in &lines {
        free.push((prev, l.at));
        for &(a, b) in &l.pauses {
            free.push((l.at + a, l.at + b));
        }
        prev = l.at + l.seconds;
    }
    free.push((prev, total));
    let mut cuts = Vec::new();
    let mut start = 0.0;
    while total - start > limit + 1e-6 {
        let left = total - start;
        let k = (left / limit).ceil();
        let target = start + left / k;
        let mut best: Option<f64> = None;
        for &(a, b) in &free {
            let (lo, hi) = (a.max(start + 1.0), b.min(start + limit));
            if lo > hi {
                continue;
            }
            // точка паузы, ближайшая к цели (ролики примерно равной длины)
            let p = target.clamp(lo, hi);
            if best.is_none_or(|q| (p - target).abs() < (q - target).abs()) {
                best = Some(p);
            }
        }
        let cut = best.unwrap_or(start + limit);
        cuts.push(cut);
        start = cut;
    }
    let mut bounds = vec![0.0];
    bounds.extend(cuts);
    bounds.push(total);
    let out: Vec<Shot> = bounds
        .windows(2)
        .map(|w| {
            let (s, e) = (w[0], w[1]);
            let pieces = lines
                .iter()
                .filter_map(|l| {
                    let (a, b) = (l.at.max(s), (l.at + l.seconds).min(e));
                    (b - a > 0.05).then(|| Line { offset: l.offset + (a - l.at), seconds: b - a, at: a - s, pauses: Vec::new(), ..l.clone() })
                })
                .collect();
            let quiet = free
                .iter()
                .map(|&(a, b)| (a.max(s) - s, b.min(e) - s))
                .filter(|(a, b)| b - a >= 1.0)
                .collect();
            Shot { start: s, seconds: e - s, lines: pieces, quiet }
        })
        .collect();
    Ok((total, lines, out))
}

/// Звучащая часть записи и паузы внутри (поиск тишины по энергии: ffmpeg
/// silencedetect, −50 дБ, от 0,25 с): (начало, конец, паузы от начала).
/// Порог низкий нарочно: вздохи, выдохи и смешки Drama 3 — часть игры, а не
/// тишина (при −38 дБ тихий вздох принимался за паузу).
pub fn speech_span(p: &Path) -> Result<(f64, f64, Vec<(f64, f64)>), String> {
    let dur = media_seconds(p)?;
    let o = std::process::Command::new("ffmpeg")
        .args(["-v", "info", "-nostats", "-i"])
        .arg(p)
        .args(["-af", "silencedetect=noise=-50dB:d=0.25", "-f", "null", "-"])
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let log = String::from_utf8_lossy(&o.stderr);
    let mut sil: Vec<(f64, f64)> = Vec::new();
    let mut open: Option<f64> = None;
    for line in log.lines() {
        if let Some(v) = line.split("silence_start: ").nth(1) {
            open = v.trim().parse().ok();
        } else if let Some(v) = line.split("silence_end: ").nth(1) {
            let end: f64 = v.split_whitespace().next().and_then(|x| x.parse().ok()).unwrap_or(dur);
            sil.push((open.take().unwrap_or(0.0).max(0.0), end.min(dur)));
        }
    }
    if let Some(s) = open {
        sil.push((s.max(0.0), dur));
    }
    let mut start = 0.0;
    let mut end = dur;
    if let Some(&(a, b)) = sil.first().filter(|x| x.0 <= 0.01) {
        let _ = a;
        start = b;
    }
    if let Some(&(a, b)) = sil.last().filter(|x| x.1 >= dur - 0.01) {
        let _ = b;
        end = a;
    }
    if end - start < 0.15 {
        return Err(format!("{}: в записи нет речи (одна тишина)", p.display()));
    }
    // запас 0,08 с по краям речи, чтобы не срезать начало и хвост слов
    let start = (start - 0.08).max(0.0);
    let end = (end + 0.08).min(dur);
    let pauses = sil.into_iter().filter(|&(a, b)| a > start + 0.05 && b < end - 0.05).map(|(a, b)| (a - start, b - start)).collect();
    Ok((start, end, pauses))
}

static PLAN: Cmd = Cmd {
    id: "scene.plan",
    category: "Озвучка",
    title: "План съёмки сцены",
    shortcut: "",
    keywords: "scene plan сцена план ролики длина дорожки раскадровка",
    help: "После озвучки сцены, до её съёмки: длина сцены = большее из `action_seconds` (сколько нужно действию по сценарию) и речи (реплики + паузы); сцена длиннее ролика H3 делится на ролики только в паузах между репликами (продолжение по латенту в одном ref-scenes с chain). Делает дорожки говорящих для каждого ролика (H3 вставит их в звук ролика), пишет план в документ «раскадровка». Сцена без речи — `lines` пустой, длина по действию. Ответ — ролики: окно, дорожки (их в refs как <Audio k> fully_copy), реплики.",
    args: &[
        req("scene", Ty::Num, "номер сцены"),
        arg("lines", Ty::Array, "реплики сцены по порядку: [{\"file\": путь из voice.say, \"speaker\": \"<герой>\", \"action_before\": <сек действия без речи перед репликой, по сценарию; можно не указывать>}]; пусто — сцена без речи"),
        arg("action_seconds", Ty::Num, "сколько секунд нужно действию по сценарию (сцены без речи — обязательно)"),
        arg("title", Ty::Str, "название сцены"),
    ],
    danger: false,
    run: |ed, a| {
        let project = ed.path.clone();
        let (text, files, data) = run_plan(a, project.as_deref())?;
        super::voice::import(ed, &files);
        Ok(Out::with(text, data))
    },
};

static MODE: Cmd = Cmd {
    id: "story.mode",
    category: "Озвучка",
    title: "Порядок съёмки фильма",
    shortcut: "",
    keywords: "story mode все сцены разом сцена за сценой порядок",
    help: "Порядок работы над фильмом: по умолчанию сцена за сценой (озвучка, план и съёмка сцены N+1 — только после съёмки сцены N). `all_at_once: true` — все сцены разом; ставить ТОЛЬКО если пользователь прямо попросил.",
    args: &[req("all_at_once", Ty::Bool, "все сцены разом (только по просьбе пользователя)")],
    danger: false,
    run: |ed, a| {
        let p = ed.path.clone().ok_or("проект ещё не сохранён")?;
        let mut s = load(Some(&p));
        s.all_at_once = a.bool("all_at_once")?.unwrap_or(false);
        save(&p, &s);
        Ok(Out::text(if s.all_at_once { "Порядок: все сцены разом (по просьбе пользователя)".to_string() } else { "Порядок: сцена за сценой".to_string() }))
    },
};

static STATUS: Cmd = Cmd {
    id: "story.status",
    category: "Озвучка",
    title: "Ход фильма",
    shortcut: "",
    keywords: "story status сцены сняты следующая",
    help: "Какие сцены сняты, какая следующая, порядок работы.",
    args: &[],
    danger: false,
    run: |ed, _a| {
        let s = load(ed.path.as_deref());
        let next = (1..).find(|n| !s.done.contains(n)).unwrap_or(1);
        let done: Vec<String> = s.done.iter().map(|n| n.to_string()).collect();
        Ok(Out::with(format!("Сняты сцены: {}; следующая — {next}; порядок: {}", if done.is_empty() { "нет".into() } else { done.join(", ") }, if s.all_at_once { "все разом" } else { "сцена за сценой" }), json!({"done": s.done, "next": next, "all_at_once": s.all_at_once})))
    },
};

/// Каркас строки Ref2VA для ролика — только то, что не зависит от сюжета:
/// шесть полей по порядку, дорожки ролика `<Audio base+1…>` (fully_copy),
/// говорящие (S1), (S2)… по порядку первой реплики и правило речи. Постановку
/// (герои, камера, где кто стоит и куда смотрит, действие) пишет агент по
/// сценарию — на месте <…>.
///
/// `continues` — ролик начинается с латентного хвоста предыдущего (второй и
/// следующие ролики сцены и первый ролик сцены, продолжающей прошлую без
/// `[CUT]`): начало кадра обязано совпасть с концом предыдущего ролика, иначе
/// H3 уводит хвост в новый план растворением через эталоны на белом фоне.
pub fn skeleton(_shot: usize, tracks: &[Value], base: usize, continues: bool, timing: &str, windows: &str) -> String {
    // имена — латиницей: в английском тексте вне <d> кириллицу H3 норовит зачитать
    let speakers: Vec<String> = tracks.iter().filter_map(|t| t["latin"].as_str().filter(|s| !s.is_empty()).map(str::to_string).or_else(|| t["speaker"].as_str().map(latin))).collect();
    let mut sd = String::from("subject_definitions: <по строке на каждого героя, место и важный предмет ролика: <Subject N> is <кто или что: приметы теми же словами, что в библии> in <Picture k>.>");
    let mut ra = String::from("retention_analysis: <по строке на каждый <Subject N>: <Subject N> (appears in [Shot 1], …): fully_preserved - <что сохраняется>; место — fully_preserved - the layout, walls, windows and doors stay exactly as in the reference.>");
    for (j, sp) in speakers.iter().enumerate() {
        let k = base + j + 1;
        sd.push_str(&format!(" <Audio {k}> is the exact dialogue track of {sp} (S{}).", j + 1));
        ra.push_str(&format!(" <Audio {k}>: fully_copy - {sp}'s speech, copied exactly and lip-synced."));
    }
    let who: Vec<String> = speakers.iter().enumerate().map(|(j, sp)| format!("{sp} = (S{})", j + 1)).collect();
    format!(
        "{sd} summary: [reference generation{}] <суть ролика одной фразой: кто, где, что происходит>. {ra} detailed_description: <стиль: 1–2 предложения>.{} <камера по сценарию{}: крупность, где стоит, куда смотрит, движение>. [Shot 1] <постановка по сценарию: где каждый герой в кадре (лево/центр/право, передний/задний план) и куда направлены его взгляд и корпус> <действие по битам с таймкодами «At 00:0X.X, …» по сценарию ({}): в окнах без речи — заметное действие (движение, перемещения, работа с предметами, падения, реакции), во время реплик герои тоже заняты делом; без глаголов речи> <по раскадровке — следующие планы этого ролика: [Shot 2] At 00:0X.X, the shot cuts to <другая крупность или ракурс: крупно реакция, через плечо, деталь реквизита> …; иначе убери> <конец ролика: позы, взгляды и предметы на последнем кадре>.{} All speech comes only from the audio tracks; outside them nobody speaks, hums or makes vocal sounds. overall_soundscape: <фон места и звуки действий ролика>. non_diegetic_music: <музыка: инструменты, темп, динамика — или N/A>.",
        if speakers.is_empty() { "" } else { " + audio reuse" },
        if continues { CONTINUES } else { "" },
        if continues { CAMERA_CONT } else { "" },
        timing,
        if speakers.is_empty() { " No one speaks, shouts, hums or makes any vocal sound in this shot; all mouths stay closed or only breathe.".to_string() } else { format!(" Speakers ({}): each line as (Sk): <d>[Russian] <слова из диалогов></d>, in order of the audio; each character moves the mouth only while their own audio is audible.{windows}", who.join(", ")) }
    )
}

/// Латинское имя героя из словаря имён библии: строка «- <Имя> (<Latin>) …»
/// или «<Имя> (<Latin>)»; нет — транслитерация.
pub fn latin_name(bible: &str, name: &str) -> String {
    let low = name.trim().to_lowercase();
    for l in bible.lines() {
        let l = l.trim().trim_start_matches(['-', '*', ' ']).trim_start_matches("**");
        if let Some(i) = l.find('(') {
            let head = l[..i].trim().trim_end_matches("**").trim().to_lowercase();
            if head == low {
                let inner: String = l[i + 1..].chars().take_while(|c| *c != ')').collect();
                let inner = inner.trim();
                if !inner.is_empty() && inner.chars().all(|c| c.is_ascii_alphabetic() || c == ' ' || c == '-' || c == '\'') {
                    return inner.to_string();
                }
            }
        }
    }
    latin(name)
}

/// Имя латиницей (для английского текста промпта): кириллица — по простой
/// транслитерации, остальное как есть.
pub fn latin(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let lower = c.to_lowercase().next().unwrap_or(c);
        let t = match lower {
            'а' => "a", 'б' => "b", 'в' => "v", 'г' => "g", 'д' => "d", 'е' => "e", 'ё' => "yo", 'ж' => "zh", 'з' => "z", 'и' => "i", 'й' => "y",
            'к' => "k", 'л' => "l", 'м' => "m", 'н' => "n", 'о' => "o", 'п' => "p", 'р' => "r", 'с' => "s", 'т' => "t", 'у' => "u", 'ф' => "f",
            'х' => "kh", 'ц' => "ts", 'ч' => "ch", 'ш' => "sh", 'щ' => "shch", 'ъ' => "", 'ы' => "y", 'ь' => "", 'э' => "e", 'ю' => "yu", 'я' => "ya",
            _ => {
                out.push(c);
                continue;
            }
        };
        if c != lower {
            let mut it = t.chars();
            if let Some(f) = it.next() {
                out.extend(f.to_uppercase());
                out.push_str(it.as_str());
            }
        } else {
            out.push_str(t);
        }
    }
    out
}

/// Время ролика словами: когда кто говорит и окна без речи (для битов действия).
fn timing(s: &Shot) -> String {
    let mut parts: Vec<String> = s.lines.iter().map(|l| format!("{} говорит {:.1}–{:.1} с", l.speaker, l.at, l.at + l.seconds)).collect();
    if !s.quiet.is_empty() {
        parts.push(format!("без речи {}", s.quiet.iter().map(|q| format!("{:.1}–{:.1} с", q.0, q.1)).collect::<Vec<_>>().join(", ")));
    }
    format!("ролик {:.1} с: {}", s.seconds, parts.join("; "))
}

/// Время речи и тишины ролика словами для H3 (английский, таймкоды от начала
/// ролика). Звуковой референс H3 не привязан ко времени: тишину дорожки модель
/// сама не соблюдает — заполняет болтовнёй, сдвигает реплику или повторяет её
/// до конца ролика. Окна речи и тишины, названные прямо, держат её на месте.
pub fn speech_windows(s: &Shot) -> String {
    if s.lines.is_empty() {
        return String::new();
    }
    let tc = |x: f64| format!("00:{:04.1}", x.max(0.0));
    let mut order: Vec<&str> = Vec::new();
    for l in &s.lines {
        if !order.contains(&l.speaker.as_str()) {
            order.push(&l.speaker);
        }
    }
    let speech: Vec<String> = s.lines.iter().map(|l| format!("(S{}) at {}–{}", order.iter().position(|x| *x == l.speaker).unwrap_or(0) + 1, tc(l.at), tc(l.at + l.seconds))).collect();
    let mut quiet = Vec::new();
    let mut t = 0.0;
    for l in &s.lines {
        if l.at - t >= 0.3 {
            quiet.push(format!("{}–{}", tc(t), tc(l.at)));
        }
        t = t.max(l.at + l.seconds);
    }
    if s.seconds - t >= 0.3 {
        quiet.push(format!("{}–{}", tc(t), tc(s.seconds)));
    }
    let mut out = format!(" Speech timing: each line is heard exactly once, only at {}.", speech.join(", "));
    if !quiet.is_empty() {
        out.push_str(&format!(" Silence at {}: nobody speaks, mutters, repeats a line or makes any vocal sound, and all mouths stay closed.", quiet.join(" and ")));
    }
    out
}

/// Начало ролика, продолжающего предыдущий по латентному хвосту.
const CONTINUES: &str = " This shot continues seamlessly from the previous shot: it opens on exactly the same camera framing, location, lighting, character positions and poses as the previous shot ended; any change of framing is a continuous camera move, never a dissolve or a cut.";
/// Подсказка к месту камеры в продолжении.
const CAMERA_CONT: &str = " — начало ровно как конец предыдущего ролика (его строка и последний кадр), смена плана — только движением камеры";

/// Строки, которые начинаются с латентного хвоста (вторая и следующие без
/// `[CUT]`; первая — если сцена продолжает прошлую), должны прямо говорить
/// H3, что это продолжение: иначе он уводит хвост в новый план растворением.
pub fn check_continuation(storyboard: &str, first_continues: bool, n: u32) -> Result<(), String> {
    for (i, l) in storyboard.lines().map(str::trim).filter(|l| !l.is_empty()).enumerate() {
        let cont = if i == 0 { first_continues } else { !l.starts_with("[CUT]") };
        if cont && !l.to_lowercase().contains("continues seamlessly") {
            return Err(format!(
                "строка {} storyboard начинается с хвоста {} — нужна фраза из каркаса scene.plan «This shot continues seamlessly from the previous shot…» и начало кадра ровно как конец предыдущего ролика (ракурс, расстановка, позы, свет). Если это другая локация или время или новый план той же сцены (другая крупность, ракурс) — начни строку с [CUT] и убери фразу о продолжении.",
                i + 1,
                if i == 0 { format!("сцены {}", n.saturating_sub(1)) } else { "предыдущего ролика".into() }
            ));
        }
    }
    Ok(())
}

pub fn media_seconds(p: &Path) -> Result<f64, String> {
    let o = std::process::Command::new("ffprobe").args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"]).arg(p).output().map_err(|e| format!("ffprobe: {e}"))?;
    String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().map_err(|_| format!("{}: не звук", p.display()))
}

/// Дорожка: куски записей на своих местах, тишина вокруг, длина `secs`.
/// Кусок — (файл, начало в файле, длина, место в дорожке); края куска —
/// мягкие (10 мс), чтобы разрез в паузе не щёлкал.
pub fn make_track(lines: &[(PathBuf, f64, f64, f64)], secs: f64, out: &Path) -> Result<(), String> {
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y", "-f", "lavfi", "-t", &format!("{secs:.3}"), "-i", "anullsrc=r=44100:cl=mono"]);
    let mut filt = String::new();
    for (k, (f, off, len, at)) in lines.iter().enumerate() {
        cmd.arg("-i").arg(f);
        let n = k + 1;
        let fade = 0.01f64.min(len / 4.0);
        filt.push_str(&format!(
            "[{n}:a]aresample=44100,aformat=channel_layouts=mono,atrim=start={off:.4}:duration={len:.4},asetpts=PTS-STARTPTS,afade=t=in:d={fade:.3},afade=t=out:st={:.4}:d={fade:.3},adelay={}:all=1[a{n}];",
            (len - fade).max(0.0),
            (at.max(0.0) * 1000.0).round() as u64
        ));
    }
    let n = lines.len();
    let ins: String = (0..=n).map(|k| if k == 0 { "[0:a]".to_string() } else { format!("[a{k}]") }).collect();
    filt.push_str(&format!("{ins}amix=inputs={}:normalize=0:duration=first[o]", n + 1));
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let r = cmd.args(["-filter_complex", &filt, "-map", "[o]", "-ar", "44100", "-ac", "1"]).arg(out).output().map_err(|e| format!("ffmpeg: {e}"))?;
    if !r.status.success() {
        return Err(format!("ffmpeg: {}", String::from_utf8_lossy(&r.stderr).trim()));
    }
    Ok(())
}

fn slug(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect::<String>().trim_matches('_').chars().take(40).collect()
}

pub fn run_plan(a: &Args, project: Option<&Path>) -> Result<(String, Vec<PathBuf>, Value), String> {
    let n = a.num("scene")?.ok_or("нужен scene")? as u32;
    gate(project, n, "План")?;
    let action = a.num("action_seconds")?.unwrap_or(0.0).max(0.0);
    let raw = a.value("lines").and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let mut lines = Vec::new();
    for l in &raw {
        let f = l.get("file").and_then(Value::as_str).ok_or("у реплики нужен file (путь из voice.say)")?;
        let file = crate::doc::expand(f);
        let speaker = l.get("speaker").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if speaker.is_empty() {
            return Err(format!("у реплики {} нужен speaker — кто говорит", file.display()));
        }
        let (a0, b0, pauses) = speech_span(&file)?;
        let before = l.get("action_before").and_then(Value::as_f64).unwrap_or(0.0).max(0.0);
        lines.push(Line { offset: a0, seconds: b0 - a0, pauses, speaker, file, at: 0.0, before });
    }
    if lines.is_empty() && action <= 0.0 {
        return Err("сцена без речи — укажи action_seconds (сколько нужно действию по сценарию)".into());
    }
    // запас на округление вверх по сетке H3 — ролик не длиннее предела
    let (total, placed, mut shots) = layout(lines, action, SHOT_LIMIT - 17.0 / 24.0)?;
    for s in &mut shots {
        s.seconds = h3_seconds(s.seconds);
    }
    let base = project.map(crate::store::media_dir).unwrap_or_else(std::env::temp_dir).join("speech");
    // латинские имена героев — из словаря имён библии («- Имя (Latin) — …»)
    let bible = project.map(|p| std::fs::read_to_string(crate::store::media_dir(p).join("01_библия.md")).unwrap_or_default()).unwrap_or_default();
    let mut files = Vec::new();
    let mut shots_json = Vec::new();
    let mut md = format!("\n## Сцена {n}{}\n\nДлина {total:.1} с (действие {action:.1} с, речь {:.1} с), роликов {}.\n\n| Ролик | Окно сцены | Длина | Дорожки | Реплики |\n|---|---|---|---|---|\n", a.str("title")?.map(|t| format!(" — {t}")).unwrap_or_default(), placed.last().map(|l| l.at + l.seconds + TAIL).unwrap_or(0.0), shots.len());
    for (i, s) in shots.iter().enumerate() {
        let mut speakers: Vec<String> = s.lines.iter().map(|l| l.speaker.clone()).collect();
        speakers.dedup();
        let mut uniq: Vec<String> = Vec::new();
        for sp in speakers {
            if !uniq.contains(&sp) {
                uniq.push(sp);
            }
        }
        let mut tracks = Vec::new();
        for sp in &uniq {
            let p = base.join(format!("сцена-{n}-ролик-{}-{}.wav", i + 1, slug(sp)));
            make_track(&s.lines.iter().filter(|l| &l.speaker == sp).map(|l| (l.file.clone(), l.offset, l.seconds, l.at)).collect::<Vec<_>>(), s.seconds, &p)?;
            files.push(p.clone());
            tracks.push(json!({"speaker": sp, "latin": latin_name(&bible, sp), "file": p}));
        }
        let said: Vec<String> = s.lines.iter().map(|l| format!("{} @{:.1}", l.speaker, l.at)).collect();
        md.push_str(&format!(
            "| {} | {:.1}–{:.1} с | {:.1} с | {} | {} |\n",
            i + 1,
            s.start,
            s.start + s.seconds,
            s.seconds,
            tracks.iter().filter_map(|t| t["file"].as_str()).map(|f| Path::new(f).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).collect::<Vec<_>>().join(", "),
            if said.is_empty() { "без речи".into() } else { said.join("; ") }
        ));
        shots_json.push(json!({"shot": i + 1, "start": s.start, "seconds": s.seconds, "tracks": tracks, "lines": s.lines.iter().map(|l| json!({"speaker": l.speaker, "at": l.at, "seconds": l.seconds, "file": l.file})).collect::<Vec<_>>(), "quiet": s.quiet.iter().map(|q| json!([q.0, q.1])).collect::<Vec<_>>(), "timing": timing(s), "windows": speech_windows(s)}));
    }
    if let Some(p) = project {
        // раскадровка: раздел сцены заменяется новым планом
        let doc = crate::store::media_dir(p).join("раскадровка.md");
        let old = std::fs::read_to_string(&doc).unwrap_or_else(|_| "# Раскадровка — план съёмки по сценам\n".into());
        let head = format!("\n## Сцена {n}");
        let kept = match old.find(&head) {
            Some(i) => {
                let rest = &old[i + head.len()..];
                let end = rest.find("\n## Сцена ").map(|j| i + head.len() + j).unwrap_or(old.len());
                format!("{}{}", &old[..i], &old[end..])
            }
            None => old,
        };
        let _ = std::fs::write(&doc, format!("{}{md}", kept.trim_end()));
        let mut s = load(Some(p));
        s.planned.insert(n, shots.len());
        s.shot.remove(&n);
        s.done.remove(&n);
        save(p, &s);
    }
    let summary: Vec<String> = shots
        .iter()
        .enumerate()
        .map(|(i, s)| format!("ролик {}: {:.1} с, реплик {}", i + 1, s.seconds, s.lines.len()))
        .collect();
    // готовая нумерация звука: refs = [картинки…, затем дорожки роликов по порядку]
    let mut k = 0;
    let mut numbering = Vec::new();
    let mut skeletons: Vec<String> = Vec::new();
    // сцена продолжает прошлую по её хвосту (ref-scenes подставит context сам, если первая строка без [CUT])
    let from_prev = n > 1 && tail_of(project, n - 1).is_some();
    for (i, sj) in shots_json.iter().enumerate() {
        let tr = sj["tracks"].as_array().cloned().unwrap_or_default();
        let parts: Vec<String> = tr
            .iter()
            .map(|t| {
                k += 1;
                format!("<Audio {k}> — {} ({})", t["speaker"].as_str().unwrap_or(""), Path::new(t["file"].as_str().unwrap_or("")).file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default())
            })
            .collect();
        numbering.push(format!("ролик {} (строка {} storyboard): {}", i + 1, i + 1, if parts.is_empty() { "без речи — ни одного <Audio>".into() } else { parts.join(", ") }));
        skeletons.push(skeleton(i, &tr, k - tr.len(), i > 0 || from_prev, sj["timing"].as_str().unwrap_or(""), sj["windows"].as_str().unwrap_or("")));
    }
    Ok((
        format!(
            "Каркасы строк storyboard по роликам (заполни <…>, остальное не меняй; герои — <Picture k> по порядку картинок в refs):\n{}\n\nНумерация звука, если в refs после картинок идут дорожки роликов по порядку: {}. В строке ролика упоминай ТОЛЬКО его <Audio k> (не больше 3). Сцена {n}: {total:.1} с, роликов {} ({}). Дорожки — в refs как <Audio k> fully_copy, по ролику; ролики сцены — строками одного ref-scenes с chain: продолжение по хвосту — для непрерывного действия, новый план той же сцены (другая крупность, ракурс) — строка с [CUT] без фразы о продолжении; внутри ролика планы меняй через [Shot 2] At 00:0X.X; звук сцены — звук роликов H3 (речь в нём уже есть), отдельную дорожку речи на таймлайн не клади. План — в документе «раскадровка».{}",
            skeletons.iter().enumerate().map(|(i, s)| format!("ролик {}: {s}", i + 1)).collect::<Vec<_>>().join("\n"),
            numbering.join("; "),
            shots.len(),
            summary.join("; "),
            if from_prev {
                format!("\n\nСцена {n} начинается с латентного хвоста сцены {}: ролик 1 — продолжение, его начало (ракурс, расстановка, позы, свет) опиши ровно как конец последнего ролика сцены {} (перечитай его строку, посмотри последний кадр сцены {} через frame_check). Если сцена в другой локации или времени — начни строку ролика 1 с [CUT] и убери фразу о продолжении.", n - 1, n - 1, n - 1)
            } else {
                String::new()
            },
        ),
        files,
        json!({"scene": n, "seconds": total, "shots": shots_json, "audio_numbering": numbering, "skeletons": skeletons}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(sp: &str, s: f64) -> Line {
        Line { speaker: sp.into(), file: PathBuf::from(format!("/{sp}.wav")), offset: 0.0, seconds: s, pauses: Vec::new(), at: 0.0, before: 0.0 }
    }

    #[test]
    fn long_dialogue_three_shots_cut_in_silence_with_action() {
        // ~40 с диалога: 3 ролика ≤ 18 с, разрезы только между репликами
        let mut lines: Vec<Line> = (0..10).map(|i| l(if i % 2 == 0 { "A" } else { "B" }, 3.5)).collect();
        lines[4].before = 4.0; // действие перед пятой репликой
        let (total, placed, shots) = layout(lines, 0.0, 18.0).unwrap();
        assert_eq!(shots.len(), 3, "{total}");
        assert!(shots.iter().all(|s| s.seconds <= 18.0 + 1e-6));
        // ни одна реплика не разрезана
        let pieces: usize = shots.iter().map(|s| s.lines.len()).sum();
        assert_eq!(pieces, 10);
        assert!((placed[4].at - (placed[3].at + 3.5 + 4.0)).abs() < 1e-9);
        // окно действия видно в плане ролика
        assert!(shots.iter().any(|s| s.quiet.iter().any(|q| q.1 - q.0 >= 3.9)));
        assert!(timing(&shots[0]).contains("A говорит 0.4–3.9 с"));
    }

    #[test]
    fn continuation_needs_phrase() {
        assert!((h3_seconds(4.5) - 124.0 / 24.0).abs() < 1e-9);
        assert!((h3_seconds(124.0 / 24.0) - 124.0 / 24.0).abs() < 1e-9);
        assert_eq!(latin_name("## Словарь\n- Бухле (Buhle) — девочка\n- **Тренер** (Coach) — x", "Бухле"), "Buhle");
        assert_eq!(latin_name("- **Тренер** (Coach) — x", "Тренер"), "Coach");
        assert_eq!(latin_name("", "Бухле"), "Bukhle");
        assert_eq!(latin("Щука-Ёж Bob"), "Shchuka-Yozh Bob");
        // окна речи и тишины ролика словами
        let ln = |sp: &str, at: f64, s: f64| Line { speaker: sp.into(), file: PathBuf::new(), offset: 0.0, seconds: s, pauses: vec![], at, before: 0.0 };
        let shot = Shot { start: 0.0, seconds: 5.0, lines: vec![ln("A", 1.5, 1.2), ln("B", 3.0, 1.0)], quiet: vec![] };
        let w = speech_windows(&shot);
        assert!(w.contains("(S1) at 00:01.5–00:02.7, (S2) at 00:03.0–00:04.0"), "{w}");
        assert!(w.contains("Silence at 00:00.0–00:01.5 and 00:04.0–00:05.0"), "{w}");
        assert!(speech_windows(&Shot { start: 0.0, seconds: 3.0, lines: vec![], quiet: vec![] }).is_empty());
        let sk = skeleton(0, &[], 0, true, "", "");
        assert!(sk.contains("continues seamlessly"));
        assert!(!skeleton(0, &[], 0, false, "", "").contains("continues seamlessly"));
        assert!(check_continuation("a new shot", true, 2).is_err());
        assert!(check_continuation("a new shot", false, 2).is_ok());
        assert!(check_continuation("[CUT] new place", false, 2).is_ok());
        assert!(check_continuation("x\ny continues seamlessly from the previous shot", false, 2).is_ok());
        assert!(check_continuation("x\ny next", false, 2).is_err());
        assert!(check_continuation("x\n[CUT] y", false, 2).is_ok());
    }

    #[test]
    fn short_scene_one_shot_action_wins() {
        let (total, placed, shots) = layout(vec![l("A", 2.0), l("B", 2.0)], 9.0, 18.0).unwrap();
        assert_eq!(total, 9.0);
        assert_eq!(shots.len(), 1);
        assert!((placed[1].at - (0.4 + 2.0 + 0.5)).abs() < 1e-9);
    }

    #[test]
    fn long_scene_splits_between_lines() {
        let lines = vec![l("A", 4.0), l("B", 5.0), l("A", 6.0), l("B", 4.0), l("A", 5.0), l("B", 6.0)];
        let (total, placed, shots) = layout(lines, 0.0, 18.0).unwrap();
        assert!(total > 18.0 && shots.len() >= 2, "{total} {}", shots.len());
        for s in &shots {
            assert!(s.seconds <= 18.0 + 1e-6);
        }
        // ни одна реплика не разрезана: сумма кусков = число реплик
        let n: usize = shots.iter().map(|s| s.lines.len()).sum();
        assert_eq!(n, placed.len());
    }

    #[test]
    fn long_line_is_cut_in_its_pause() {
        let mut long = l("A", 26.0);
        long.offset = 0.3;
        long.pauses = vec![(12.0, 12.4)];
        let (_, _, shots) = layout(vec![long], 0.0, 18.0).unwrap();
        assert_eq!(shots.len(), 2);
        let cut = shots[1].start;
        assert!(cut >= 0.4 + 12.0 && cut <= 0.4 + 12.4, "{cut}");
        // второй кусок продолжает запись с того же места
        let p = &shots[1].lines[0];
        assert!((p.offset - (0.3 + cut - 0.4)).abs() < 1e-6);
        assert!((shots[0].lines[0].seconds + p.seconds - 26.0).abs() < 1e-6);
    }

    #[test]
    fn silent_scene() {
        let (total, _, shots) = layout(vec![], 30.0, 18.0).unwrap();
        assert_eq!(total, 30.0);
        assert_eq!(shots.len(), 2);
    }

    #[test]
    fn scene_done_after_all_planned_shots() {
        let d = tempfile::tempdir().unwrap();
        let proj = d.path().join("Ф").join("Ф.syn");
        std::fs::create_dir_all(proj.parent().unwrap()).unwrap();
        let mut st = Story::default();
        st.planned.insert(1, 2);
        save(&proj, &st);
        add_shots(Some(&proj), 1, 1);
        assert!(!load(Some(&proj)).done.contains(&1));
        assert!(gate(Some(&proj), 2, "Озвучка").is_err());
        add_shots(Some(&proj), 1, 1);
        assert!(load(Some(&proj)).done.contains(&1));
        assert!(gate(Some(&proj), 2, "Озвучка").is_ok());
        assert!(gate(Some(&proj), 3, "Озвучка").is_err());
    }

    #[test]
    fn numbers() {
        assert_eq!(scene_number("Сцена 3 из 8: x"), Some(3));
        assert_eq!(scene_number("12"), Some(12));
        // метка ролика агентом бывает «Сцена-01», «scene_2»
        assert_eq!(scene_number("сцена-4"), Some(4));
        assert_eq!(scene_number("Сцена-01"), Some(1));
        assert_eq!(scene_number("scene_2"), Some(2));
        assert_eq!(scene_number("Кря"), None);
    }
}

#[cfg(test)]
mod facts_tests {
    #[test]
    fn facts_table_gate() {
        let t = "| Сцена | Место | Кто | ✓/✗ |\n|---|---|---|---|\n| 1 | кухня героини | мать, дочь | ✓ |\n| 2 | гостиная | мать | ✗ место |\n";
        assert_eq!(super::facts_rows(t, 1).len(), 1);
        assert!(super::facts_rows(t, 2)[0].contains('✗'));
        assert!(super::facts_rows(t, 3).is_empty());
        let d = tempfile::tempdir().unwrap();
        let proj = d.path().join("p").join("p.syn");
        let dir = crate::store::media_dir(&proj);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("02_сверка.md"), t).unwrap();
        assert!(super::facts_gate(&proj, 1).is_ok());
        assert!(super::facts_gate(&proj, 2).unwrap_err().contains("✗"));
        assert!(super::facts_gate(&proj, 3).unwrap_err().contains("нет строки"));
    }
}
