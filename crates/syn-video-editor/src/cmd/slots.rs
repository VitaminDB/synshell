//! Шаблоны проектов (G): готовый проект-образец (монтаж, ритм, переходы,
//! титры, музыка, цвет) с местами для вставки — подставляешь свои файлы и
//! тексты, получается ролик (как шаблоны CapCut и After Effects).
//!
//! Место — клип с `slot`: медиа-место (плашка-заливка или клип с медиа-образцом)
//! или текст (титр). Шаблоны: встроенные — сценарии Rhai `library/projects/*.rhai`
//! (строят проект командами `slot.add`, `title.add`, `transition.set`…), свои —
//! проекты `.syn` в `~/.config/syn-video-editor/projects` («Сохранить как шаблон»).

use std::path::PathBuf;

use serde_json::{json, Map, Value};

use super::timeline::{add_track, place, track_free};
use super::{arg, req, Args, Command, Out, Res, Ty};
use crate::doc::{expand, tc, Clip, Effect, Id, Key, MediaKind, Project, Slot, Source, TemplateInfo, TrackKind, PLACEHOLDER_COLOR};
use crate::editor::Editor;
use crate::script::header;

pub static COMMANDS: &[&Command] = &[&SLOT_ADD, &SLOT_SET, &SLOT_CLEAR, &SLOT_LIST, &SLOT_FILL, &TPL_LIST, &TPL_USE, &TPL_SAVE];

const SLOT_KINDS: &[&str] = &["media", "text", "music"];

// ---------- места ----------

static SLOT_ADD: Command = Command {
    id: "slot.add",
    category: "Шаблоны",
    title: "Добавить место для вставки",
    shortcut: "",
    keywords: "место вставки слот плейсхолдер заглушка шаблон проекта drop zone placeholder",
    help: "Добавить место для вставки шаблона проекта: kind=media — плашка для видео/фото на видеодорожке (track: имя дорожки, по умолчанию «Кадры»), text — текстовое место (титр с text и style), music — место для музыки на звуковой дорожке. label — подпись («Фото 1», «Заголовок»), n — порядок заполнения, motion — у фото наезд/панорама, sound — оставить звук видео (по умолчанию без звука: в шаблоне играет музыка), fit — cover (по умолчанию) или contain.",
    args: &[
        req("kind", Ty::Choice(SLOT_KINDS), "вид места"),
        req("at", Ty::Time, "начало"),
        req("duration", Ty::Time, "длительность"),
        arg("label", Ty::Str, "подпись места"),
        arg("n", Ty::Num, "порядок заполнения (по умолчанию — по времени)"),
        arg("track", Ty::Str, "имя дорожки (создаётся)"),
        arg("motion", Ty::Bool, "фото: наезд/панорама"),
        arg("sound", Ty::Bool, "оставить звук видео"),
        arg("fit", Ty::Choice(&["cover", "contain"]), "как ложится в кадр"),
        arg("text", Ty::Str, "текст по умолчанию (kind=text)"),
        arg("style", Ty::Object, "стиль титра (kind=text), как у title.add"),
    ],
    danger: false,
    run: slot_add,
};

fn track_named(p: &mut Project, kind: TrackKind, name: &str) -> usize {
    match p.tracks.iter().position(|t| t.kind == kind && t.name == name) {
        Some(i) => i,
        None => {
            let id = add_track(p, kind, Some(name.to_string()));
            p.tracks.iter().position(|t| t.id == id).unwrap()
        }
    }
}

fn slot_add(ed: &mut Editor, a: &Args) -> Res {
    let kind = a.need_str("kind")?;
    let at = a.need_time("at")?.max(0.0);
    let dur = a.need_time("duration")?;
    if dur <= 0.0 {
        return Err("duration > 0".into());
    }
    let n = a.num("n")?.map(|v| v.max(0.0) as u32);
    let label = a.str("label")?.unwrap_or_default();
    ed.begin("Место для вставки");
    let p = &mut ed.project;
    let fps = p.settings.fps;
    // по кадрам — начало и конец (тогда места встык не налезают друг на друга)
    let snap = |t: f64| (t * fps).round() / fps;
    let (at, dur) = (snap(at), (snap(at + dur) - snap(at)).max(1.0 / fps));
    let (tk, default_track) = match kind.as_str() {
        "music" => (TrackKind::Audio, "Музыка"),
        "text" => (TrackKind::Video, "Тексты"),
        _ => (TrackKind::Video, "Кадры"),
    };
    let tname = a.str("track")?.unwrap_or_else(|| default_track.to_string());
    let ti = track_named(p, tk, &tname);
    if !track_free(p, ti, at, dur, None) {
        return Err(format!("на дорожке «{tname}» {}–{} занято", tc(at), tc(at + dur)));
    }
    let n = n.unwrap_or_else(|| p.tracks.iter().flat_map(|t| &t.clips).filter(|c| c.slot.is_some()).count() as u32 + 1);
    let source = match kind.as_str() {
        "text" => Source::Title { text: a.str("text")?.unwrap_or_else(|| if label.is_empty() { "Ваш текст".into() } else { label.clone() }), style: a.object("style")?.unwrap_or_default() },
        _ => Source::Color { color: PLACEHOLDER_COLOR.into() },
    };
    let id = p.alloc();
    let keep_sound = a.bool("sound")?.unwrap_or(kind == "music");
    let fit = match a.str("fit")?.as_deref() {
        Some("contain") => String::new(),
        _ if kind == "media" => "cover".into(),
        _ => String::new(),
    };
    let clip = Clip {
        id,
        source,
        start: at,
        duration: dur,
        offset: 0.0,
        speed: 1.0,
        volume: if kind == "media" && !keep_sound { 0.0 } else { 1.0 },
        name: label.clone(),
        effects: Vec::new(),
        transition: None,
        keys: Default::default(),
        blend: String::new(),
        parent: None,
        link: None,
        graph: None,
        fit,
        slot: Some(Slot { label: label.clone(), n, motion: a.bool("motion")?.unwrap_or(false), filled: kind == "text" }),
    };
    place(p, ti, clip, false);
    Ok(Out::with(format!("c{id} место «{label}» ({kind}) {}–{}", tc(at), tc(at + dur)), json!({"id": id})))
}

static SLOT_SET: Command = Command {
    id: "slot.set",
    category: "Шаблоны",
    title: "Сделать местом для вставки",
    shortcut: "",
    keywords: "место вставки слот шаблон пометить плейсхолдер",
    help: "Пометить клип местом для вставки шаблона (или изменить место): label, n — порядок, motion — наезд у фото. Клип с медиа остаётся образцом и заменяется при заполнении; титр — текстовое место.",
    args: &[req("id", Ty::Id, "клип"), arg("label", Ty::Str, "подпись"), arg("n", Ty::Num, "порядок заполнения"), arg("motion", Ty::Bool, "фото: наезд/панорама")],
    danger: false,
    run: slot_set,
};

fn slot_set(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let c = ed.project.clip(id).ok_or_else(|| format!("нет клипа c{id}"))?;
    if matches!(c.source, Source::Shape { .. }) {
        return Err(format!("c{id} — фигура: местом может быть видео/фото, заливка или титр"));
    }
    let count = ed.project.tracks.iter().flat_map(|t| &t.clips).filter(|c| c.slot.is_some()).count() as u32;
    let (label, n, motion) = (a.str("label")?, a.num("n")?, a.bool("motion")?);
    ed.begin("Место для вставки");
    let c = ed.project.clip_mut(id).unwrap();
    let filled = !matches!(c.source, Source::Color { .. });
    let s = c.slot.get_or_insert_with(|| Slot { label: c.name.clone(), n: count + 1, motion: false, filled });
    if let Some(l) = label {
        s.label = l;
    }
    if let Some(n) = n {
        s.n = n.max(0.0) as u32;
    }
    if let Some(m) = motion {
        s.motion = m;
    }
    let s = s.clone();
    Ok(Out::text(format!("c{id} — место «{}» №{}", s.label, s.n)))
}

static SLOT_CLEAR: Command = Command {
    id: "slot.clear",
    category: "Шаблоны",
    title: "Убрать место для вставки",
    shortcut: "",
    keywords: "место вставки слот снять пометку очистить",
    help: "Снять с клипа пометку места для вставки (содержимое остаётся); empty=true — вернуть место к пустой плашке (файл убирается).",
    args: &[req("id", Ty::Id, "клип"), arg("empty", Ty::Bool, "очистить содержимое места")],
    danger: false,
    run: slot_clear,
};

fn slot_clear(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let c = ed.project.clip(id).ok_or_else(|| format!("нет клипа c{id}"))?;
    if c.slot.is_none() {
        return Err(format!("c{id} — не место для вставки"));
    }
    let empty = a.bool("empty")?.unwrap_or(false);
    ed.begin("Место для вставки");
    let c = ed.project.clip_mut(id).unwrap();
    if empty {
        if !matches!(c.source, Source::Title { .. }) {
            c.source = Source::Color { color: PLACEHOLDER_COLOR.into() };
            c.offset = 0.0;
            c.speed = 1.0;
            strip_motion(c);
            if let Some(s) = &mut c.slot {
                s.filled = false;
            }
        }
        return Ok(Out::text(format!("c{id}: место пустое")));
    }
    c.slot = None;
    Ok(Out::text(format!("c{id}: больше не место для вставки")))
}

/// Места проекта по порядку заполнения: (клип, дорожка, вид).
pub fn slots(p: &Project) -> Vec<(&Clip, &crate::doc::Track, &'static str)> {
    let mut v: Vec<(&Clip, &crate::doc::Track, &'static str)> = p
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter().filter(|c| c.slot.is_some()).map(move |c| (c, t)))
        .map(|(c, t)| {
            let k = match (&c.source, t.kind) {
                (Source::Title { .. }, _) => "text",
                (_, TrackKind::Audio) => "music",
                _ => "media",
            };
            (c, t, k)
        })
        .collect();
    v.sort_by(|a, b| a.0.slot.as_ref().unwrap().n.cmp(&b.0.slot.as_ref().unwrap().n).then(a.0.start.total_cmp(&b.0.start)));
    v
}

fn slot_line(p: &Project, c: &Clip, kind: &str) -> String {
    let s = c.slot.as_ref().unwrap();
    let what = match &c.source {
        Source::Title { text, .. } => format!("«{text}»"),
        Source::Media { media } => p.media(*media).map(|m| m.name()).unwrap_or_default(),
        _ => "пусто".into(),
    };
    format!("№{} c{} «{}» {kind} {}–{} ({:.1} с): {what}{}", s.n, c.id, s.label, tc(c.start), tc(c.end()), c.duration, if s.motion { ", наезд" } else { "" })
}

static SLOT_LIST: Command = Command {
    id: "slot.list",
    category: "Шаблоны",
    title: "Места для вставки",
    shortcut: "",
    keywords: "места вставки слоты шаблон что заполнить",
    help: "Места для вставки текущего проекта по порядку заполнения: вид (media — видео/фото, text — текст, music — музыка), подпись, время, длина, чем заполнено.",
    args: &[],
    danger: false,
    run: slot_list,
};

fn slot_list(ed: &mut Editor, _a: &Args) -> Res {
    let p = &ed.project;
    let v = slots(p);
    if v.is_empty() {
        return Ok(Out::with("мест для вставки нет", json!([])));
    }
    let lines: Vec<String> = v.iter().map(|(c, _, k)| slot_line(p, c, k)).collect();
    let data: Vec<Value> = v
        .iter()
        .map(|(c, t, k)| {
            let s = c.slot.as_ref().unwrap();
            json!({"id": c.id, "n": s.n, "label": s.label, "kind": k, "track": t.name, "start": c.start, "duration": c.duration, "filled": s.filled, "motion": s.motion})
        })
        .collect();
    Ok(Out::with(lines.join("\n"), Value::Array(data)))
}

static SLOT_FILL: Command = Command {
    id: "slot.fill",
    category: "Шаблоны",
    title: "Заполнить место",
    shortcut: "",
    keywords: "заполнить место вставить свой файл фото видео текст шаблон заменить",
    help: "Заполнить место для вставки (id или label): file — путь к видео/фото/музыке (импортируется) или media — m… из медиатеки; text — текст текстового места; offset — с какого момента источника (иначе лучший отрезок: сцена из индекса или середина). Видео короче места замедляется (до 0.5×), фото с motion — плавный наезд.",
    args: &[
        arg("id", Ty::Id, "место (c…)"),
        arg("label", Ty::Str, "подпись места (вместо id)"),
        arg("file", Ty::Str, "файл"),
        arg("media", Ty::Id, "медиа m…"),
        arg("text", Ty::Str, "текст"),
        arg("offset", Ty::Time, "начало в источнике"),
    ],
    danger: false,
    run: slot_fill,
};

fn find_slot(p: &Project, a: &Args) -> Result<Id, String> {
    if let Some(id) = a.id("id")? {
        let c = p.clip(id).ok_or_else(|| format!("нет клипа c{id}"))?;
        return c.slot.as_ref().map(|_| id).ok_or_else(|| format!("c{id} — не место для вставки"));
    }
    let l = a.str("label")?.ok_or("укажите id или label места")?;
    let l = l.trim().to_lowercase();
    slots(p).iter().find(|(c, _, _)| c.slot.as_ref().unwrap().label.to_lowercase() == l).map(|(c, _, _)| c.id).ok_or_else(|| format!("нет места «{l}» — slot.list"))
}

/// Медиа по пути: уже в медиатеке или импорт.
fn media_for(p: &mut Project, path: &str) -> Result<Id, String> {
    let f = expand(path);
    let f = std::fs::canonicalize(&f).unwrap_or(f);
    if let Some(m) = p.media.iter().find(|m| m.path == f) {
        return Ok(m.id);
    }
    let mut m = super::media::probe(&f)?;
    m.id = p.alloc();
    let id = m.id;
    p.media.push(m);
    Ok(id)
}

fn strip_motion(c: &mut Clip) {
    if let Some(e) = c.effects.iter().find(|e| e.kind == "transform" && e.params.get("motion").is_some()) {
        let pre = format!("e{}.", e.id);
        c.keys.retain(|k, _| !k.starts_with(&pre));
        let id = e.id;
        c.effects.retain(|e| e.id != id);
    }
}

/// Отрезок источника под место: сцена индекса, в которую место влезает (самая
/// длинная), иначе середина файла.
fn best_offset(m: &crate::doc::Media, need: f64) -> f64 {
    let dur = m.duration;
    if dur <= need {
        return 0.0;
    }
    if let Some(ix) = &m.index {
        if let Some(s) = ix.scenes.iter().filter(|s| s.end - s.start >= need).max_by(|a, b| (a.end - a.start).total_cmp(&(b.end - b.start))) {
            return (s.start + (s.end - s.start - need) / 2.0).clamp(0.0, dur - need);
        }
    }
    (dur - need) / 2.0
}

/// Подставить медиа в место. Возвращает строку отчёта.
pub fn fill_media(p: &mut Project, cid: Id, mid: Id, offset: Option<f64>) -> Result<String, String> {
    let m = p.media(mid).ok_or_else(|| format!("нет медиа m{mid}"))?.clone();
    let (ti, _) = p.find_clip(cid).ok_or_else(|| format!("нет клипа c{cid}"))?;
    let audio_track = p.tracks[ti].kind == TrackKind::Audio;
    if audio_track && !m.has_audio {
        return Err(format!("место c{cid} — для звука, а в {} звука нет", m.name()));
    }
    if !audio_track && m.kind == MediaKind::Audio {
        return Err(format!("место c{cid} — для видео/фото, а {} — звук", m.name()));
    }
    let eff_id = p.alloc();
    let c = p.clip_mut(cid).unwrap();
    if matches!(c.source, Source::Title { .. }) {
        return Err(format!("c{cid} — текстовое место: заполняется text"));
    }
    strip_motion(c);
    c.source = Source::Media { media: mid };
    let mut note = String::new();
    c.speed = 1.0;
    c.offset = 0.0;
    match m.kind {
        MediaKind::Video | MediaKind::Audio => {
            let need = c.duration;
            if m.duration > 0.0 && m.duration < need {
                c.speed = (m.duration / need).clamp(0.5, 1.0);
                if m.duration / need < 0.5 {
                    note = format!(", файл короче места ({:.1} из {:.1} с)", m.duration, need);
                } else if c.speed < 0.999 {
                    note = format!(", замедлено до {:.2}×", c.speed);
                }
            } else {
                c.offset = offset.unwrap_or_else(|| if audio_track { 0.0 } else { best_offset(&m, need) });
            }
        }
        MediaKind::Image if c.slot.as_ref().is_some_and(|s| s.motion) => {
            // наезд или панорама — чередуются по номеру места
            let n = c.slot.as_ref().unwrap().n;
            let mut params = Map::new();
            params.insert("motion".into(), Value::Bool(true));
            c.effects.insert(0, Effect { id: eff_id, kind: "transform".into(), params });
            let d = c.duration;
            let (k0, k1, x0, x1) = match n % 3 {
                0 => (1.15, 1.02, 0.0, 0.0),
                1 => (1.02, 1.15, 0.0, 0.0),
                _ => (1.1, 1.1, -0.03, 0.03),
            };
            let key = |t: f64, v: f64| Key { t, v: json!(v), ease: "ease".into() };
            c.keys.insert(format!("e{eff_id}.scale"), vec![key(0.0, k0), key(d, k1)]);
            if x0 != x1 {
                c.keys.insert(format!("e{eff_id}.x"), vec![key(0.0, x0), key(d, x1)]);
            }
            note = ", наезд".into();
        }
        _ => {}
    }
    if let Some(s) = &mut c.slot {
        s.filled = true;
    }
    let label = c.slot.as_ref().map(|s| s.label.clone()).unwrap_or_default();
    Ok(format!("c{cid} «{label}» ← {}{note}", m.name()))
}

fn fill_text(p: &mut Project, cid: Id, text: &str) -> Result<String, String> {
    let c = p.clip_mut(cid).ok_or_else(|| format!("нет клипа c{cid}"))?;
    match &mut c.source {
        Source::Title { text: t, .. } => {
            *t = text.to_string();
            Ok(format!("c{cid} «{}» ← «{text}»", c.slot.as_ref().map(|s| s.label.as_str()).unwrap_or("")))
        }
        _ => Err(format!("c{cid} — не текстовое место")),
    }
}

fn slot_fill(ed: &mut Editor, a: &Args) -> Res {
    let cid = find_slot(&ed.project, a)?;
    let (file, media, text, offset) = (a.str("file")?, a.id("media")?, a.str("text")?, a.time("offset")?);
    if file.is_none() && media.is_none() && text.is_none() {
        return Err("укажите file, media или text".into());
    }
    let mut tmp = ed.project.clone();
    let line = match text {
        Some(t) => fill_text(&mut tmp, cid, &t)?,
        None => {
            let mid = match media {
                Some(m) => m,
                None => media_for(&mut tmp, file.as_deref().unwrap())?,
            };
            fill_media(&mut tmp, cid, mid, offset)?
        }
    };
    ed.begin("Заполнить место");
    ed.project = tmp;
    Ok(Out::text(line))
}

// ---------- шаблоны проектов ----------

const BUILTIN: &[(&str, &str)] = &[
    ("reels_slideshow", include_str!("../../library/projects/reels_slideshow.rhai")),
    ("beat_cuts", include_str!("../../library/projects/beat_cuts.rhai")),
    ("intro", include_str!("../../library/projects/intro.rhai")),
    ("promo", include_str!("../../library/projects/promo.rhai")),
    ("before_after", include_str!("../../library/projects/before_after.rhai")),
    ("travel", include_str!("../../library/projects/travel.rhai")),
    ("story", include_str!("../../library/projects/story.rhai")),
];

pub struct ProjectTemplate {
    pub name: String,
    pub title: String,
    pub description: String,
    pub category: String,
    /// Сценарий встроенного шаблона или путь своего `.syn`.
    pub code: Option<String>,
    pub path: Option<PathBuf>,
}

pub fn dir() -> PathBuf {
    crate::layout::config_dir().join("projects")
}

fn parse_builtin(name: &str, code: &str) -> ProjectTemplate {
    ProjectTemplate {
        name: name.to_string(),
        title: header(code, "title").unwrap_or_else(|| name.to_string()),
        description: header(code, "description").unwrap_or_default(),
        category: header(code, "category").unwrap_or_default(),
        code: Some(code.to_string()),
        path: None,
    }
}

pub fn all() -> Vec<ProjectTemplate> {
    let mut v: Vec<ProjectTemplate> = BUILTIN.iter().map(|(n, c)| parse_builtin(n, c)).collect();
    if let Ok(rd) = std::fs::read_dir(dir()) {
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == crate::store::EXT || x == "rhai")).collect();
        files.sort();
        for f in files {
            let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let t = if f.extension().is_some_and(|x| x == "rhai") {
                let Ok(code) = std::fs::read_to_string(&f) else { continue };
                ProjectTemplate { path: Some(f), ..parse_builtin(&stem, &code) }
            } else {
                let Ok(p) = crate::store::load(&f) else { continue };
                let info = p.template.clone().unwrap_or_default();
                ProjectTemplate { name: stem, title: p.name.clone(), description: info.description, category: info.category, code: None, path: Some(f) }
            };
            v.retain(|x| x.name != t.name);
            v.push(t);
        }
    }
    v
}

/// Проект из шаблона (без заполнения).
pub fn build(t: &ProjectTemplate) -> Result<Project, String> {
    match (&t.code, &t.path) {
        (Some(code), _) => {
            let mut ed = Editor::new();
            ed.project = Project::new(&t.title);
            crate::script::run(&mut ed, code).map_err(|e| format!("шаблон «{}»: {e}", t.title))?;
            let mut p = ed.project;
            // пустые дорожки нового проекта (V1, A1) шаблону не нужны
            p.tracks.retain(|t| !t.clips.is_empty());
            p.name = t.title.clone();
            p.template = Some(TemplateInfo { description: t.description.clone(), category: t.category.clone() });
            Ok(p)
        }
        (None, Some(path)) => crate::store::load(path),
        _ => Err("пустой шаблон".into()),
    }
}

pub fn find(name: &str) -> Result<ProjectTemplate, String> {
    let q = name.trim().to_lowercase();
    let v = all();
    let names = v.iter().map(|t| t.name.clone()).collect::<Vec<_>>().join(", ");
    v.into_iter().find(|t| t.name.to_lowercase() == q || t.title.to_lowercase() == q).ok_or_else(|| format!("нет шаблона проекта «{name}»; есть: {names}"))
}

static TPL_LIST: Command = Command {
    id: "template.project.list",
    category: "Шаблоны",
    title: "Шаблоны проектов",
    shortcut: "",
    keywords: "шаблоны проектов готовый ролик слайд-шоу reels интро промо capcut after effects места вставки",
    help: "Готовые шаблоны проектов (как в CapCut/After Effects): монтаж, ритм, переходы, титры, музыка — с местами для своих файлов и текстов. Для каждого: имя, описание, места (сколько видео/фото, тексты, музыка). Использовать — template.project.use.",
    args: &[],
    danger: false,
    run: tpl_list,
};

pub fn summary(p: &Project) -> String {
    let v = slots(p);
    let (m, t, a) = (v.iter().filter(|x| x.2 == "media").count(), v.iter().filter(|x| x.2 == "text").count(), v.iter().filter(|x| x.2 == "music").count());
    let texts: Vec<String> = v.iter().filter(|x| x.2 == "text").map(|x| format!("«{}»", x.0.slot.as_ref().unwrap().label)).collect();
    let s = &p.settings;
    format!("{}×{}, {:.0} с; видео/фото: {m}, тексты: {t}{}, музыка: {a}", s.width, s.height, p.duration(), if texts.is_empty() { String::new() } else { format!(" ({})", texts.join(", ")) })
}

fn tpl_list(_ed: &mut Editor, _a: &Args) -> Res {
    let mut lines = Vec::new();
    let mut data = Vec::new();
    for t in all() {
        let info = build(&t).map(|p| summary(&p)).unwrap_or_else(|e| format!("ошибка: {e}"));
        lines.push(format!("{} — {}: {} | {info}", t.name, t.title, t.description));
        data.push(json!({"name": t.name, "title": t.title, "description": t.description, "category": t.category, "slots": info, "path": t.path}));
    }
    Ok(Out::with(lines.join("\n"), Value::Array(data)))
}

static TPL_USE: Command = Command {
    id: "template.project.use",
    category: "Шаблоны",
    title: "Ролик по шаблону…",
    shortcut: "",
    keywords: "использовать шаблон проекта ролик из шаблона вставить свои файлы фото видео слайд-шоу reels capcut",
    help: "Новый проект из шаблона (template.project.list) с вашими файлами: files — видео/фото по местам по порядку (папка — все файлы в ней, пустая строка — пропустить место), music — музыка в место для музыки, texts — тексты: объект {подпись: текст} или список по порядку. Файлов меньше, чем мест, — повторяются по кругу (у видео другой отрезок), ролик остаётся целым; repeat=false — места остаются плашками (slot.list, slot.fill). Пустые места не удалять — сдвинутся тексты и музыка. Текущий проект заменяется (отмена вернёт его); сохранить — project.save path.",
    args: &[req("name", Ty::Str, "шаблон"), arg("files", Ty::Paths, "видео/фото"), arg("music", Ty::Str, "музыка"), arg("texts", Ty::Any, "тексты: {подпись: текст} или [..]"), arg("media", Ty::Ids, "медиа m… текущего проекта вместо files"), arg("repeat", Ty::Bool, "файлов меньше, чем мест, — повторить по кругу (true); false — оставить места пустыми")],
    danger: false,
    run: tpl_use,
};

/// Разложить файлы, музыку и тексты по местам проекта.
pub fn fill_all(p: &mut Project, files: &[String], music: Option<&str>, texts: Option<&Value>, repeat: bool) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    // файлы: папки раскрываются, звук — в место музыки, если не задано отдельно
    let mut visual = Vec::new();
    let mut audio: Vec<String> = music.map(|m| vec![m.to_string()]).unwrap_or_default();
    for f in files {
        if f.trim().is_empty() {
            visual.push(String::new());
            continue;
        }
        let pth = expand(f);
        let list: Vec<PathBuf> = if pth.is_dir() {
            let mut v: Vec<PathBuf> = std::fs::read_dir(&pth).map_err(|e| format!("{}: {e}", pth.display()))?.flatten().map(|e| e.path()).filter(|x| super::media::probe(x).is_ok()).collect();
            v.sort();
            v
        } else {
            vec![pth]
        };
        for x in list {
            let ext = x.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if super::media::AUDIO.contains(&ext.as_str()) {
                if music.is_none() {
                    audio.push(x.display().to_string());
                }
            } else {
                visual.push(x.display().to_string());
            }
        }
    }
    let order: Vec<(Id, &'static str, String)> = slots(p).iter().map(|(c, _, k)| (c.id, *k, c.slot.as_ref().unwrap().label.clone())).collect();
    let media_slots: Vec<Id> = order.iter().filter(|x| x.1 == "media").map(|x| x.0).collect();
    let music_slots: Vec<Id> = order.iter().filter(|x| x.1 == "music").map(|x| x.0).collect();
    for (i, f) in visual.iter().enumerate() {
        match media_slots.get(i) {
            // пустая строка — место пропускается (окно шаблона передаёт файлы по местам)
            Some(_) if f.is_empty() => {}
            Some(cid) => {
                let mid = media_for(p, f)?;
                out.push(fill_media(p, *cid, mid, None)?);
            }
            None => {
                out.push(format!("лишних файлов: {} (мест для видео/фото {})", visual.len() - media_slots.len(), media_slots.len()));
                break;
            }
        }
    }
    // файлов меньше, чем мест (и окно не задавало места поштучно) — по кругу, как в CapCut:
    // ролик остаётся целым (длина, тексты, музыка), у видео при повторе — другой отрезок
    let given: Vec<&String> = visual.iter().filter(|f| !f.is_empty()).collect();
    if repeat && !given.is_empty() && given.len() == visual.len() && visual.len() < media_slots.len() {
        let mut uses: std::collections::HashMap<Id, usize> = Default::default();
        for (i, cid) in media_slots.iter().enumerate().skip(visual.len()) {
            let f = given[i % given.len()];
            let mid = media_for(p, f)?;
            let k = uses.entry(mid).or_insert(0);
            *k += 1;
            let offset = p.media(mid).filter(|m| m.kind == MediaKind::Video).and_then(|m| {
                let need = p.clip(*cid)?.duration;
                let room = m.duration - need;
                (room > 0.05).then(|| (best_offset(m, need) + need * *k as f64) % (room + 1e-9))
            });
            out.push(format!("{} (повтор)", fill_media(p, *cid, mid, offset)?));
        }
    }
    for (i, f) in audio.iter().enumerate() {
        if let Some(cid) = music_slots.get(i) {
            let mid = media_for(p, f)?;
            out.push(fill_media(p, *cid, mid, Some(0.0))?);
        }
    }
    if let Some(t) = texts {
        let text_slots: Vec<&(Id, &'static str, String)> = order.iter().filter(|x| x.1 == "text").collect();
        match t {
            Value::Object(m) => {
                for (k, v) in m {
                    let s = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                    let kl = k.trim().to_lowercase();
                    let hit = text_slots.iter().find(|x| x.2.to_lowercase() == kl).or_else(|| text_slots.iter().find(|x| x.2.to_lowercase().contains(&kl)));
                    match hit {
                        Some(x) => out.push(fill_text(p, x.0, &s)?),
                        None => out.push(format!("нет текстового места «{k}»")),
                    }
                }
            }
            Value::Array(a) => {
                for (x, v) in text_slots.iter().zip(a) {
                    out.push(fill_text(p, x.0, v.as_str().unwrap_or(&v.to_string()))?);
                }
            }
            Value::String(s) => {
                if let Some(x) = text_slots.first() {
                    out.push(fill_text(p, x.0, s)?);
                }
            }
            _ => return Err("texts: объект {подпись: текст} или список".into()),
        }
    }
    let empty: Vec<String> = slots(p).iter().filter(|(c, _, k)| *k != "text" && !c.slot.as_ref().unwrap().filled).map(|(c, _, _)| format!("«{}»", c.slot.as_ref().unwrap().label)).collect();
    if !empty.is_empty() {
        out.push(format!("пустые места: {}", empty.join(", ")));
    }
    Ok(out)
}

fn tpl_use(ed: &mut Editor, a: &Args) -> Res {
    let t = find(&a.need_str("name")?)?;
    let mut p = build(&t)?;
    let mut files = a.strs("files")?.unwrap_or_default();
    if let Some(ids) = a.ids("media")? {
        for id in ids {
            let m = ed.project.media(id).ok_or_else(|| format!("нет медиа m{id}"))?;
            files.push(m.path.display().to_string());
        }
    }
    let music = a.str("music")?;
    let lines = fill_all(&mut p, &files, music.as_deref(), a.raw("texts"), a.bool("repeat")?.unwrap_or(true))?;
    // файл текущего проекта остаётся, только если тот был пуст (иначе «Сохранить» затёрло бы его)
    let was_empty = ed.project.media.is_empty() && ed.project.tracks.iter().all(|t| t.clips.is_empty());
    ed.begin(&format!("Шаблон «{}»", t.title));
    ed.project = p;
    if !was_empty {
        ed.path = None;
    }
    ed.selection.clear();
    ed.playhead = 0.0;
    let mut text = format!("Проект по шаблону «{}» ({})", t.title, summary(&ed.project));
    for l in lines {
        text.push_str("\n  ");
        text.push_str(&l);
    }
    Ok(Out::text(text))
}

static TPL_SAVE: Command = Command {
    id: "template.project.save",
    category: "Шаблоны",
    title: "Сохранить как шаблон проекта…",
    shortcut: "",
    keywords: "сохранить шаблон проекта свой шаблон места вставки",
    help: "Сохранить текущий проект шаблоном (~/.config/syn-video-editor/projects/<name>.syn): места для вставки — клипы с пометкой (slot.add / slot.set); медиа-образцы в местах остаются примером и заменяются при использовании.",
    args: &[req("name", Ty::Str, "имя шаблона (файла)"), arg("title", Ty::Str, "название"), arg("description", Ty::Str, "описание"), arg("category", Ty::Str, "раздел")],
    danger: false,
    run: tpl_save,
};

fn tpl_save(ed: &mut Editor, a: &Args) -> Res {
    let name = a.need_str("name")?;
    let safe: String = name.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if safe.is_empty() {
        return Err("пустое имя".into());
    }
    if slots(&ed.project).is_empty() {
        return Err("в проекте нет мест для вставки — пометьте клипы (slot.set) или добавьте места (slot.add)".into());
    }
    let mut p = ed.project.clone();
    if let Some(t) = a.str("title")? {
        p.name = t;
    }
    let mut info = p.template.clone().unwrap_or_default();
    if let Some(d) = a.str("description")? {
        info.description = d;
    }
    if let Some(c) = a.str("category")? {
        info.category = c;
    }
    p.template = Some(info);
    let path = dir().join(format!("{safe}.{}", crate::store::EXT));
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    crate::store::save(&p, &path)?;
    Ok(Out::with(format!("Шаблон «{}» сохранён: {} ({})", p.name, path.display(), summary(&p)), json!({"path": path})))
}
