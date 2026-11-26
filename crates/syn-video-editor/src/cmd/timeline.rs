//! Таймлайн: дорожки, клипы, титры, эффекты. Размещение клипа — с
//! перезаписью (наложенное обрезается) или со вставкой (сдвиг вправо).

use serde_json::{json, Map, Value};

use super::{arg, req, Args, Command, Out, Res, Ty};
use crate::doc::{tc, Clip, Effect, Id, MediaKind, Project, Source, Track, TrackKind};
use crate::editor::Editor;

pub static COMMANDS: &[&Command] = &[&TRACK_ADD, &TRACK_SET, &TRACK_REMOVE, &TRACK_MOVE, &CLIP_ADD, &TITLE_ADD, &SHAPE_ADD, &CLIP_MOVE, &CLIP_TRIM, &CLIP_SPLIT, &CLIP_REMOVE, &CLIP_SET, &CLIP_SHIFT, &CLIP_COPY, &CLIP_PASTE, &CLIP_DETACH_AUDIO, &EFFECT_ADD, &EFFECT_SET, &EFFECT_REMOVE, &TRANSITION_SET, &TRANSITION_REMOVE, &TRANSITION_LIST];

const MODES: &[&str] = &["overwrite", "insert"];
const EPS: f64 = 1e-6;
/// Длительность картинки, цвета и титра по умолчанию, с.
const STILL: f64 = 5.0;
const TITLE: f64 = 3.0;

// ── дорожки ─────────────────────────────────────────────────────────────

static TRACK_ADD: Command = Command {
    id: "track.add",
    category: "Таймлайн",
    title: "Добавить дорожку",
    shortcut: "",
    keywords: "track add дорожка слой",
    help: "Новая дорожка: video (слой поверх прежних) или audio.",
    args: &[req("kind", Ty::Choice(&["video", "audio"]), "вид дорожки"), arg("name", Ty::Str, "название")],
    danger: false,
    run: track_add,
};

fn track_add(ed: &mut Editor, a: &Args) -> Res {
    let kind = track_kind(&a.need_str("kind")?)?;
    ed.begin("Добавить дорожку");
    let t = add_track(&mut ed.project, kind, a.str("name")?);
    let tr = ed.project.track(t).unwrap();
    Ok(Out::with(format!("Дорожка t{t} {}", tr.name), json!({"id": t})))
}

fn track_kind(s: &str) -> Result<TrackKind, String> {
    match s {
        "video" | "v" => Ok(TrackKind::Video),
        "audio" | "a" => Ok(TrackKind::Audio),
        _ => Err(format!("вид дорожки «{s}»: video или audio")),
    }
}

pub(crate) fn add_track(p: &mut Project, kind: TrackKind, name: Option<String>) -> Id {
    let n = p.tracks.iter().filter(|t| t.kind == kind).count() + 1;
    let name = name.unwrap_or_else(|| format!("{}{n}", if kind == TrackKind::Video { "V" } else { "A" }));
    let id = p.alloc();
    p.tracks.push(Track { id, kind, name, muted: false, hidden: false, clips: Vec::new() });
    id
}

static TRACK_SET: Command = Command {
    id: "track.set",
    category: "Таймлайн",
    title: "Свойства дорожки",
    shortcut: "",
    keywords: "track mute hide звук скрыть переименовать",
    help: "Переименовать дорожку, выключить звук (muted) или скрыть (hidden).",
    args: &[req("id", Ty::Id, "дорожка (t…)"), arg("name", Ty::Str, "название"), arg("muted", Ty::Bool, "без звука"), arg("hidden", Ty::Bool, "скрыта")],
    danger: false,
    run: track_set,
};

fn track_set(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    ed.project.track(id).ok_or_else(|| format!("нет дорожки t{id}"))?;
    let (name, muted, hidden) = (a.str("name")?, a.bool("muted")?, a.bool("hidden")?);
    ed.begin("Свойства дорожки");
    let t = ed.project.track_mut(id).unwrap();
    if let Some(n) = name {
        t.name = n;
    }
    if let Some(m) = muted {
        t.muted = m;
    }
    if let Some(h) = hidden {
        t.hidden = h;
    }
    Ok(Out::text(format!("t{id} {}{}{}", t.name, if t.muted { ", без звука" } else { "" }, if t.hidden { ", скрыта" } else { "" })))
}

static TRACK_REMOVE: Command = Command {
    id: "track.remove",
    category: "Таймлайн",
    title: "Удалить дорожку",
    shortcut: "",
    keywords: "track remove удалить дорожку",
    help: "Удалить дорожку вместе с её клипами.",
    args: &[req("id", Ty::Id, "дорожка (t…)")],
    danger: false,
    run: track_remove,
};

fn track_remove(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let removed_ids: Vec<Id> = ed.project.track(id).ok_or_else(|| format!("нет дорожки t{id}"))?.clips.iter().map(|c| c.id).collect();
    let n = removed_ids.len();
    ed.begin("Удалить дорожку");
    ed.project.tracks.retain(|t| t.id != id);
    for cid in removed_ids {
        ed.project.clear_parent(cid);
        ed.project.clear_link(cid);
    }
    Ok(Out::text(format!("Удалена дорожка t{id} и клипов: {n}")))
}

// ── размещение ──────────────────────────────────────────────────────────

/// Освободить `[s, e)` на дорожке: наложенные клипы обрезать или разрезать.
/// Возвращает `(удалённые id, разрезанные пары (левый id, новый правый id))`.
fn clear_range(p: &mut Project, ti: usize, s: f64, e: f64) -> (Vec<Id>, Vec<(Id, Id)>) {
    let mut out = Vec::new();
    let mut removed = Vec::new();
    let mut splits = Vec::new();
    let clips = std::mem::take(&mut p.tracks[ti].clips);
    for c in clips {
        if c.end() <= s + EPS || c.start >= e - EPS {
            out.push(c);
            continue;
        }
        let has_left = c.start < s - EPS;
        let has_right = c.end() > e + EPS;
        if !has_left && !has_right {
            removed.push(c.id);
            continue;
        }
        if has_left {
            let mut left = c.clone();
            left.duration = s - c.start;
            out.push(left);
        }
        if has_right {
            let mut right = c.clone();
            if has_left {
                let rid = p.alloc();
                right.id = rid;
                right.transition = None;
                right.link = None;
                right.renew_effect_ids(|| p.alloc());
                splits.push((c.id, rid));
            }
            let cut = e - c.start;
            right.start = e;
            right.duration = c.end() - e;
            right.offset = c.offset + cut * c.speed;
            out.push(right);
        }
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    p.tracks[ti].clips = out;
    for id in &removed {
        p.clear_parent(*id);
        p.clear_link(*id);
    }
    (removed, splits)
}

/// Вставка: клипы правее `s` сдвинуть на `d` (пересекающий `s` — разрезать).
fn open_gap(p: &mut Project, ti: usize, s: f64, d: f64) -> Vec<(Id, Id)> {
    let mut splits = Vec::new();
    if let Some(c) = p.tracks[ti].clips.iter().find(|c| c.start < s - EPS && c.end() > s + EPS).map(|c| c.id) {
        if let Some(rid) = split_clip(p, c, s) {
            if let Some(rc) = p.clip_mut(rid) {
                rc.link = None;
            }
            splits.push((c, rid));
        }
    }
    for c in &mut p.tracks[ti].clips {
        if c.start >= s - EPS {
            c.start += d;
        }
    }
    splits
}

fn place_with_splits(p: &mut Project, ti: usize, mut clip: Clip, insert: bool) -> (Id, Vec<(Id, Id)>) {
    clip.start = p.snap(clip.start);
    clip.duration = p.snap(clip.duration).max(1.0 / p.settings.fps.max(1.0));
    let splits = if insert {
        open_gap(p, ti, clip.start, clip.duration)
    } else {
        clear_range(p, ti, clip.start, clip.end()).1
    };
    let id = clip.id;
    let t = &mut p.tracks[ti];
    t.clips.push(clip);
    t.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    (id, splits)
}

pub(crate) fn place(p: &mut Project, ti: usize, clip: Clip, insert: bool) -> Id {
    place_with_splits(p, ti, clip, insert).0
}

/// После парного разрезания (видео + звук) связать правые половины парных клипов.
fn relink_splits(p: &mut Project, v_splits: &[(Id, Id)], a_splits: &[(Id, Id)]) {
    for &(v_old, v_right) in v_splits {
        let Some(a_old) = p.linked_peer(v_old) else { continue };
        if let Some(&(_, a_right)) = a_splits.iter().find(|&&(ao, _)| ao == a_old) {
            if let Some(vc) = p.clip_mut(v_right) {
                vc.link = Some(a_right);
            }
            if let Some(ac) = p.clip_mut(a_right) {
                ac.link = Some(v_right);
            }
        }
    }
}

/// Расширить список клипов связанными парами (без повторов).
fn expand_linked(p: &Project, ids: &[Id]) -> Vec<Id> {
    let mut out = Vec::with_capacity(ids.len() * 2);
    for &id in ids {
        if !out.contains(&id) {
            out.push(id);
        }
        if let Some(peer) = p.linked_peer(id) {
            if !out.contains(&peer) {
                out.push(peer);
            }
        }
    }
    out
}

/// Свободен ли интервал `[s, s+d)` на дорожке `ti`.
pub(crate) fn track_free(p: &Project, ti: usize, s: f64, d: f64, except: Option<Id>) -> bool {
    p.tracks[ti].clips.iter().all(|o| Some(o.id) == except || o.end() <= s + EPS || o.start >= s + d - EPS)
}

/// Звуковая дорожка для видеоклипа с видеодорожки `v_ti` на `[start, start+dur)`.
fn audio_track_for_video(p: &mut Project, v_ti: usize, start: f64, dur: f64, insert: bool) -> usize {
    let v_rank = p.tracks[..v_ti].iter().filter(|t| t.kind == TrackKind::Video).count();
    let audio_tracks: Vec<usize> = (0..p.tracks.len()).filter(|i| p.tracks[*i].kind == TrackKind::Audio).collect();
    if insert {
        if let Some(&ati) = audio_tracks.get(v_rank) {
            return ati;
        }
        let mut last = audio_tracks.last().copied();
        for _ in audio_tracks.len()..=v_rank {
            let id = add_track(p, TrackKind::Audio, None);
            last = p.tracks.iter().position(|t| t.id == id);
        }
        return last.unwrap();
    }
    // перезапись: если на видеодорожке под этим интервалом лежат клипы со связанным звуком — идём на их звуковую дорожку
    for c in &p.tracks[v_ti].clips {
        if c.end() > start + EPS && c.start < start + dur - EPS {
            if let Some(peer) = p.linked_peer(c.id) {
                if let Some((ati, _)) = p.find_clip(peer) {
                    if p.tracks[ati].kind == TrackKind::Audio {
                        return ati;
                    }
                }
            }
        }
    }
    if let Some(&ati) = audio_tracks.get(v_rank) {
        if track_free(p, ati, start, dur, None) {
            return ati;
        }
    }
    if let Some(&ati) = audio_tracks.iter().find(|&&ati| track_free(p, ati, start, dur, None)) {
        return ati;
    }
    let id = add_track(p, TrackKind::Audio, None);
    p.tracks.iter().position(|t| t.id == id).unwrap()
}

/// Разрезать клип в момент `at` (время таймлайна); id правой части.
fn split_clip(p: &mut Project, id: Id, at: f64) -> Option<Id> {
    let (ti, ci) = p.find_clip(id)?;
    let c = p.tracks[ti].clips[ci].clone();
    if at <= c.start + EPS || at >= c.end() - EPS {
        return None;
    }
    let nid = p.alloc();
    let mut right = c.clone();
    right.id = nid;
    right.start = at;
    right.duration = c.end() - at;
    right.offset = c.offset + (at - c.start) * c.speed;
    // переход «в начале» остаётся у левой части: на разрезе склейки нет
    right.transition = None;
    right.renew_effect_ids(|| p.alloc());
    right.shift_keys(-(at - c.start));
    p.tracks[ti].clips[ci].duration = at - c.start;
    p.tracks[ti].clips.insert(ci + 1, right);
    Some(nid)
}

/// Индекс дорожки: указанная или первая подходящего вида (нет — создаётся).
pub(crate) fn target_track(p: &mut Project, track: Option<Id>, kind: TrackKind) -> Result<usize, String> {
    if let Some(t) = track {
        let i = p.tracks.iter().position(|x| x.id == t).ok_or_else(|| format!("нет дорожки t{t}"))?;
        if p.tracks[i].kind != kind {
            return Err(format!("дорожка t{t} — {}, а нужна {}", p.tracks[i].kind.as_str(), kind.as_str()));
        }
        return Ok(i);
    }
    if let Some(i) = p.tracks.iter().position(|x| x.kind == kind) {
        return Ok(i);
    }
    let id = add_track(p, kind, None);
    Ok(p.tracks.iter().position(|x| x.id == id).unwrap())
}

fn track_end(p: &Project, ti: usize) -> f64 {
    p.tracks[ti].clips.last().map(Clip::end).unwrap_or(0.0)
}

fn mode_insert(a: &Args) -> Result<bool, String> {
    match a.str("mode")?.as_deref() {
        None | Some("overwrite") => Ok(false),
        Some("insert") => Ok(true),
        Some(m) => Err(format!("режим «{m}»: overwrite или insert")),
    }
}

// ── клипы ───────────────────────────────────────────────────────────────

static CLIP_ADD: Command = Command {
    id: "clip.add",
    category: "Таймлайн",
    title: "Добавить клип",
    shortcut: "",
    keywords: "clip add вставить клип на таймлайн поставить",
    help: "Поставить медиа на таймлайн. Видео со звуком автоматически ставится парой: картинка на видеодорожку и связанный звук на аудиодорожку (audio: false — только видео со встроенным звуком). Без at — в конец дорожки; без track — первая дорожка подходящего вида. offset/duration — какой кусок источника взять. mode: overwrite (наложенное обрезается, по умолчанию) или insert (остальное сдвигается вправо).",
    args: &[
        req("media", Ty::Id, "медиа (m…)"),
        arg("track", Ty::Id, "дорожка (t…)"),
        arg("at", Ty::Time, "начало на таймлайне"),
        arg("offset", Ty::Time, "с какого места источника"),
        arg("duration", Ty::Time, "длительность на таймлайне"),
        arg("mode", Ty::Choice(MODES), "overwrite или insert"),
        arg("audio", Ty::Bool, "вынести звук видео на звуковую дорожку (по умолчанию true)"),
    ],
    danger: false,
    run: clip_add,
};

fn clip_add(ed: &mut Editor, a: &Args) -> Res {
    let mid = a.need_id("media")?;
    let m = ed.project.media(mid).ok_or_else(|| format!("нет медиа m{mid}"))?.clone();
    let kind = if m.kind == MediaKind::Audio { TrackKind::Audio } else { TrackKind::Video };
    let offset = a.time("offset")?.unwrap_or(0.0);
    if m.src_limit().is_some() && offset >= m.duration {
        return Err(format!("offset {} за концом источника ({})", tc(offset), tc(m.duration)));
    }
    let avail = if m.duration > 0.0 { (m.duration - offset).max(1.0 / ed.project.settings.fps.max(1.0)) } else { STILL };
    let duration = a.time("duration")?.unwrap_or(avail);
    if m.src_limit().is_some() && duration > avail + 1e-3 {
        return Err(format!("в источнике после {} только {}", tc(offset), tc(avail)));
    }
    let insert = mode_insert(a)?;
    let with_audio = a.bool("audio")?.unwrap_or(true);
    let (track, at) = (a.id("track")?, a.time("at")?);
    ed.begin("Добавить клип");
    let p = &mut ed.project;
    let ti = target_track(p, track, kind)?;
    let start = at.unwrap_or_else(|| track_end(p, ti));
    if m.kind == MediaKind::Video && m.has_audio && with_audio {
        let vid = p.alloc();
        let aid = p.alloc();
        let ati = audio_track_for_video(p, ti, start, duration, insert);
        let v_clip = Clip { id: vid, source: Source::Media { media: mid }, start, duration, offset, speed: 1.0, volume: 0.0, name: String::new(), effects: Vec::new(), transition: None, keys: Default::default(), blend: String::new(), parent: None, link: Some(aid), graph: None, fit: String::new(), slot: None };
        let a_clip = Clip { id: aid, source: Source::Media { media: mid }, start, duration, offset, speed: 1.0, volume: 1.0, name: String::new(), effects: Vec::new(), transition: None, keys: Default::default(), blend: String::new(), parent: None, link: Some(vid), graph: None, fit: String::new(), slot: None };
        let (_, v_splits) = place_with_splits(p, ti, v_clip, insert);
        let (_, a_splits) = place_with_splits(p, ati, a_clip, insert);
        relink_splits(p, &v_splits, &a_splits);
        let c = p.clip(vid).unwrap();
        let vtid = p.tracks[ti].id;
        let atid = p.tracks[ati].id;
        return Ok(Out::with(
            format!("c{vid} + звук c{aid} {} на t{vtid}/t{atid} {}–{}", m.name(), tc(c.start), tc(c.end())),
            json!({"id": vid, "audio_id": aid}),
        ));
    }
    let id = p.alloc();
    let clip = Clip { id, source: Source::Media { media: mid }, start, duration, offset, speed: 1.0, volume: 1.0, name: String::new(), effects: Vec::new(), transition: None, keys: Default::default(), blend: String::new(), parent: None, link: None, graph: None, fit: String::new(), slot: None };
    place(p, ti, clip, insert);
    let c = p.clip(id).unwrap();
    Ok(Out::with(format!("c{id} {} на t{} {}–{}", m.name(), p.tracks[ti].id, tc(c.start), tc(c.end())), json!({"id": id})))
}

static TITLE_ADD: Command = Command {
    id: "title.add",
    category: "Таймлайн",
    title: "Добавить титр",
    shortcut: "Ctrl+T",
    keywords: "title text титр надпись текст подпись анимация печать разброс символы слова строки",
    help: "Текстовый титр поверх видео. Без track — на верхнюю видеодорожку «Титры» (создаётся). style: {size, color, font, position: top|center|bottom, box: bool, by: all|char|word|line (целиком, по символам, словам, строкам), in/out: fade|slide|pop|scatter|type|none (появление, сдвиг, выпрыгивание, разброс, печать), anim: с (0.4), stagger: 0–1 (разбег элементов, 0.65), ease: out|ease|linear|in|back|elastic|bounce}.",
    args: &[req("text", Ty::Str, "текст"), arg("at", Ty::Time, "начало (по умолчанию — позиция воспроизведения)"), arg("duration", Ty::Time, "длительность (3 с)"), arg("track", Ty::Id, "видеодорожка (t…)"), arg("style", Ty::Object, "оформление")],
    danger: false,
    run: title_add,
};

fn title_add(ed: &mut Editor, a: &Args) -> Res {
    let text = a.need_str("text")?;
    let at = a.time("at")?.unwrap_or(ed.playhead);
    let duration = a.time("duration")?.unwrap_or(TITLE);
    let style = a.object("style")?.unwrap_or_default();
    let track = a.id("track")?;
    ed.begin("Добавить титр");
    let p = &mut ed.project;
    let ti = match track {
        Some(t) => target_track(p, Some(t), TrackKind::Video)?,
        None => match p.tracks.iter().position(|t| t.kind == TrackKind::Video && t.name == "Титры") {
            Some(i) => i,
            None => {
                let id = add_track(p, TrackKind::Video, Some("Титры".into()));
                p.tracks.iter().position(|t| t.id == id).unwrap()
            }
        },
    };
    let id = p.alloc();
    let clip = Clip { id, source: Source::Title { text: text.clone(), style }, start: at, duration, offset: 0.0, speed: 1.0, volume: 1.0, name: String::new(), effects: Vec::new(), transition: None, keys: Default::default(), blend: String::new(), parent: None, link: None, graph: None, fit: String::new(), slot: None };
    place(p, ti, clip, false);
    let c = p.clip(id).unwrap();
    Ok(Out::with(format!("c{id} титр «{text}» {}–{}", tc(c.start), tc(c.end())), json!({"id": id})))
}

static SHAPE_ADD: Command = Command {
    id: "shape.add",
    category: "Таймлайн",
    title: "Добавить фигуру",
    shortcut: "",
    keywords: "shape фигура прямоугольник круг овал треугольник звезда линия стрелка слой",
    help: "Добавить векторную фигуру поверх видео. kind: rect (прямоугольник), ellipse (круг/овал), triangle (треугольник), star (звезда), line (линия), arrow (стрелка). style: {fill: #цвет|none, stroke: #цвет|none, stroke_width, width (доля кадра), height, radius (0–0.5), points (3–12 для звезды)}.",
    args: &[req("kind", Ty::Choice(&["rect", "ellipse", "triangle", "star", "line", "arrow"]), "вид фигуры"), arg("at", Ty::Time, "начало (позиция воспроизведения)"), arg("duration", Ty::Time, "длительность (3 с)"), arg("track", Ty::Id, "видеодорожка (t…)"), arg("style", Ty::Object, "стиль фигуры")],
    danger: false,
    run: shape_add,
};

fn shape_add(ed: &mut Editor, a: &Args) -> Res {
    let kind = a.need_str("kind")?;
    if !crate::doc::SHAPE_KINDS.iter().any(|(k, _)| *k == kind) {
        return Err(format!("нет фигуры «{kind}»; есть: rect, ellipse, triangle, star, line, arrow"));
    }
    let at = a.time("at")?.unwrap_or(ed.playhead);
    let duration = a.time("duration")?.unwrap_or(TITLE);
    let style = a.object("style")?.unwrap_or_default();
    let track = a.id("track")?;
    ed.begin("Добавить фигуру");
    let p = &mut ed.project;
    let ti = match track {
        Some(t) => target_track(p, Some(t), TrackKind::Video)?,
        None => match p.tracks.iter().position(|t| t.kind == TrackKind::Video && t.name == "Фигуры") {
            Some(i) => i,
            None => {
                let id = add_track(p, TrackKind::Video, Some("Фигуры".into()));
                p.tracks.iter().position(|t| t.id == id).unwrap()
            }
        },
    };
    let id = p.alloc();
    let clip = Clip { id, source: Source::Shape { kind: kind.clone(), style }, start: at, duration, offset: 0.0, speed: 1.0, volume: 1.0, name: String::new(), effects: Vec::new(), transition: None, keys: Default::default(), blend: String::new(), parent: None, link: None, graph: None, fit: String::new(), slot: None };
    place(p, ti, clip, false);
    let c = p.clip(id).unwrap();
    Ok(Out::with(format!("c{id} фигура {} {}–{}", crate::doc::shape_title(&kind), tc(c.start), tc(c.end())), json!({"id": id})))
}

static CLIP_MOVE: Command = Command {
    id: "clip.move",
    category: "Таймлайн",
    title: "Переместить клип",
    shortcut: "",
    keywords: "move сдвинуть переместить перенести",
    help: "Перенести клип на новое время и/или дорожку того же вида.",
    args: &[req("id", Ty::Id, "клип (c…)"), arg("at", Ty::Time, "новое начало"), arg("track", Ty::Id, "дорожка (t…)"), arg("mode", Ty::Choice(MODES), "overwrite или insert")],
    danger: false,
    run: clip_move,
};

fn clip_move(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let (ti, ci) = ed.project.find_clip(id).ok_or_else(|| format!("нет клипа c{id}"))?;
    let kind = ed.project.tracks[ti].kind;
    let old_start = ed.project.tracks[ti].clips[ci].start;
    let peer = ed.project.linked_peer(id);
    let (at, track, insert) = (a.time("at")?, a.id("track")?, mode_insert(a)?);
    if let Some(t) = track {
        let tr = ed.project.track(t).ok_or_else(|| format!("нет дорожки t{t}"))?;
        if tr.kind != kind {
            return Err(format!("клип c{id} нельзя перенести на дорожку {}", tr.kind.as_str()));
        }
    }
    ed.begin("Переместить клип");
    let p = &mut ed.project;
    let mut clip = p.tracks[ti].clips.remove(ci);
    if let Some(at) = at {
        clip.start = at;
    }
    let to = match track {
        Some(t) => p.tracks.iter().position(|x| x.id == t).unwrap(),
        None => ti,
    };
    let (_, v_splits) = place_with_splits(p, to, clip, insert);
    let new_start = p.clip(id).unwrap().start;
    let dt = new_start - old_start;
    if let Some(pid) = peer {
        if dt.abs() > EPS {
            if let Some((pti, pci)) = p.find_clip(pid) {
                let pkind = p.tracks[pti].kind;
                let mut pc = p.tracks[pti].clips.remove(pci);
                pc.start = p.snap((pc.start + dt).max(0.0));
                let dest_ti = if track_free(p, pti, pc.start, pc.duration, None) {
                    pti
                } else if !insert {
                    (0..p.tracks.len())
                        .find(|&i| p.tracks[i].kind == pkind && track_free(p, i, pc.start, pc.duration, None))
                        .unwrap_or(pti)
                } else {
                    pti
                };
                let (_, a_splits) = place_with_splits(p, dest_ti, pc, insert);
                relink_splits(p, &v_splits, &a_splits);
            }
        }
    }
    let c = p.clip(id).unwrap();
    Ok(Out::text(format!("c{id} → t{} {}–{}", p.tracks[to].id, tc(c.start), tc(c.end()))))
}

static CLIP_TRIM: Command = Command {
    id: "clip.trim",
    category: "Таймлайн",
    title: "Обрезать клип",
    shortcut: "",
    keywords: "trim обрезать край начало конец укоротить",
    help: "Сдвинуть края клипа: start — новое начало, end — новый конец (время таймлайна). Не дальше границ источника.",
    args: &[req("id", Ty::Id, "клип (c…)"), arg("start", Ty::Time, "новое начало"), arg("end", Ty::Time, "новый конец")],
    danger: false,
    run: clip_trim,
};

fn clip_trim(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let c = ed.project.clip(id).ok_or_else(|| format!("нет клипа c{id}"))?.clone();
    let peer = ed.project.linked_peer(id).and_then(|pid| ed.project.clip(pid).cloned());
    let src = c.media().and_then(|m| ed.project.media(m)).and_then(|m| m.src_limit());
    let mut start = a.time("start")?.map(|t| ed.project.snap(t)).unwrap_or(c.start);
    let mut end = a.time("end")?.map(|t| ed.project.snap(t)).unwrap_or(c.end());
    // в источнике до начала клипа есть только offset/speed секунд, после — остаток
    let min_start = c.start - c.offset / c.speed;
    start = start.max(min_start.max(0.0));
    if let Some(d) = src {
        end = end.min(c.start + (d - c.offset) / c.speed);
    }
    if end - start < 1.0 / ed.project.settings.fps {
        return Err(format!("c{id}: после обрезки клип пустой"));
    }
    let (ti, _) = ed.project.find_clip(id).unwrap();
    if ed.project.tracks[ti].clips.iter().any(|o| o.id != id && o.start < end - EPS && o.end() > start + EPS) {
        return Err(format!("c{id}: новые края налезают на соседний клип"));
    }
    let peer_trim = if let Some(pc) = &peer {
        let ds = start - c.start;
        let de = end - c.end();
        let psrc = pc.media().and_then(|m| ed.project.media(m)).and_then(|m| m.src_limit());
        let mut pstart = ed.project.snap(pc.start + ds).max((pc.start - pc.offset / pc.speed).max(0.0));
        let mut pend = ed.project.snap(pc.end() + de);
        if let Some(d) = psrc {
            pend = pend.min(pc.start + (d - pc.offset) / pc.speed);
        }
        let (pti, _) = ed.project.find_clip(pc.id).unwrap();
        let collides = ed.project.tracks[pti].clips.iter().any(|o| o.id != pc.id && o.start < pend - EPS && o.end() > pstart + EPS);
        if collides {
            return Err(format!("c{}: новые края связанного звука налезают на соседний клип", pc.id));
        }
        if pend - pstart >= 1.0 / ed.project.settings.fps {
            // если край поджался источником — выровняем и основной, когда совпадали
            if (pc.start - c.start).abs() < EPS {
                start = pstart;
                pstart = start;
            }
            if (pc.end() - c.end()).abs() < EPS {
                end = pend;
                pend = end;
            }
            Some((pc.id, pstart, pend, pc.clone()))
        } else {
            None
        }
    } else {
        None
    };
    ed.begin("Обрезать клип");
    let cm = ed.project.clip_mut(id).unwrap();
    cm.offset = (c.offset + (start - c.start) * c.speed).max(0.0);
    cm.start = start;
    cm.duration = end - start;
    if let Some((pid, ps, pe, pc)) = peer_trim {
        if let Some(pm) = ed.project.clip_mut(pid) {
            pm.offset = (pc.offset + (ps - pc.start) * pc.speed).max(0.0);
            pm.start = ps;
            pm.duration = pe - ps;
        }
    }
    Ok(Out::text(format!("c{id} {}–{}", tc(start), tc(end))))
}

static CLIP_SPLIT: Command = Command {
    id: "clip.split",
    category: "Таймлайн",
    title: "Разрезать",
    shortcut: "S",
    keywords: "split cut разрезать разделить лезвие",
    help: "Разрезать клипы в момент at (по умолчанию — позиция воспроизведения). ids — какие клипы; без ids — выделенные клипы под этим моментом, а если таких нет — все клипы под ним.",
    args: &[arg("at", Ty::Time, "момент разреза"), arg("ids", Ty::Ids, "клипы (c…)")],
    danger: false,
    run: clip_split,
};

fn clip_split(ed: &mut Editor, a: &Args) -> Res {
    let at = ed.project.snap(a.time("at")?.unwrap_or(ed.playhead));
    let raw_ids: Vec<Id> = match a.ids("ids")? {
        Some(v) => v,
        None => {
            let under: Vec<Id> = ed.project.tracks.iter().flat_map(|t| t.clips.iter()).filter(|c| c.start < at - EPS && c.end() > at + EPS).map(|c| c.id).collect();
            let sel: Vec<Id> = under.iter().copied().filter(|id| ed.selection.contains(id)).collect();
            if sel.is_empty() {
                under
            } else {
                sel
            }
        }
    };
    if raw_ids.is_empty() {
        return Err(format!("в {} нечего резать", tc(at)));
    }
    for id in &raw_ids {
        let c = ed.project.clip(*id).ok_or_else(|| format!("нет клипа c{id}"))?;
        if at <= c.start + EPS || at >= c.end() - EPS {
            return Err(format!("c{id} ({}–{}) не накрывает {}", tc(c.start), tc(c.end()), tc(at)));
        }
    }
    // связанные пары режем вместе, если парный клип тоже накрывает момент разреза
    let mut ids = Vec::with_capacity(raw_ids.len() * 2);
    for &id in &raw_ids {
        if !ids.contains(&id) {
            ids.push(id);
        }
        if let Some(peer) = ed.project.linked_peer(id) {
            if !ids.contains(&peer) {
                if let Some(pc) = ed.project.clip(peer) {
                    if at > pc.start + EPS && at < pc.end() - EPS {
                        ids.push(peer);
                    }
                }
            }
        }
    }
    ed.begin("Разрезать");
    let mut splits: std::collections::HashMap<Id, Id> = std::collections::HashMap::new();
    let mut parts = Vec::new();
    for &id in &ids {
        if let Some(n) = split_clip(&mut ed.project, id, at) {
            splits.insert(id, n);
            parts.push(format!("c{id} | c{n}"));
        }
    }
    for (&old_id, &right_id) in &splits {
        let peer_right = ed.project.clip(old_id).and_then(|c| c.link).and_then(|peer| splits.get(&peer).copied());
        if let Some(rc) = ed.project.clip_mut(right_id) {
            rc.link = peer_right;
        }
    }
    Ok(Out::text(format!("Разрез в {}: {}", tc(at), parts.join(", "))))
}

static CLIP_REMOVE: Command = Command {
    id: "clip.remove",
    category: "Таймлайн",
    title: "Удалить клип",
    shortcut: "Delete",
    keywords: "remove delete удалить вырезать",
    help: "Удалить клипы (без ids — выделенные). ripple: true — сдвинуть следующие клипы дорожки влево, закрыв дыру.",
    args: &[arg("ids", Ty::Ids, "клипы (c…), по умолчанию выделенные"), arg("ripple", Ty::Bool, "закрыть дыру")],
    danger: false,
    run: clip_remove,
};

fn clip_remove(ed: &mut Editor, a: &Args) -> Res {
    let raw_ids = a.ids("ids")?.unwrap_or_else(|| ed.selection.clone());
    if raw_ids.is_empty() {
        return Err("нечего удалять: выделите клипы".into());
    }
    let ripple = a.bool("ripple")?.unwrap_or(false);
    for id in &raw_ids {
        ed.project.find_clip(*id).ok_or_else(|| format!("нет клипа c{id}"))?;
    }
    let ids = expand_linked(&ed.project, &raw_ids);
    ed.begin("Удалить клип");
    // справа налево: сдвиги не мешают следующим удалениям
    let mut doomed: Vec<(Id, usize, f64, f64)> = ids.iter().filter_map(|id| ed.project.find_clip(*id).map(|(t, c)| (*id, t, ed.project.tracks[t].clips[c].start, ed.project.tracks[t].clips[c].duration))).collect();
    doomed.sort_by(|a, b| b.2.total_cmp(&a.2));
    for (id, ti, start, dur) in doomed {
        let t = &mut ed.project.tracks[ti];
        t.clips.retain(|c| c.id != id);
        if ripple {
            for c in &mut t.clips {
                if c.start >= start - EPS {
                    c.start = (c.start - dur).max(0.0);
                }
            }
        }
        ed.project.clear_parent(id);
        ed.project.clear_link(id);
    }
    ed.selection.retain(|s| !ids.contains(s));
    Ok(Out::text(format!("Удалено клипов: {}{}", ids.len(), if ripple { " (со сдвигом)" } else { "" })))
}

static CLIP_SET: Command = Command {
    id: "clip.set",
    category: "Таймлайн",
    title: "Свойства клипа",
    shortcut: "",
    keywords: "clip speed volume скорость громкость текст имя фигура наложение blend родитель parent",
    help: "Один клип (id) или несколько сразу (ids: [c1, c2] — одним шагом отмены, при ошибке у любого не меняется ни один). Скорость (speed: 2 — вдвое быстрее, длина клипа меняется), громкость (volume: 0–4), имя, текст титра, вид фигуры (shape), стиль титра или фигуры (style), режим наложения слоя (blend: normal|multiply|screen|overlay|add|darken|lighten|soft_light|hard_light|difference), родительский клип (parent: c… или 0/\"\" чтобы отвязать), fit: contain|cover — вписать видео/фото целиком или заполнить кадр.",
    args: &[
        arg("id", Ty::Id, "клип (c…)"),
        arg("ids", Ty::Ids, "несколько клипов сразу (вместо id)"),
        arg("speed", Ty::Num, "скорость"),
        arg("volume", Ty::Num, "громкость"),
        arg("name", Ty::Str, "имя"),
        arg("text", Ty::Str, "текст титра"),
        arg("shape", Ty::Str, "вид фигуры (rect|ellipse|triangle|star|line|arrow)"),
        arg("style", Ty::Object, "стиль титра или фигуры (поля сливаются)"),
        arg("blend", Ty::Str, "режим наложения слоя"),
        arg("parent", Ty::Any, "родительский клип (c… или 0/\"\" чтобы отвязать)"),
        arg("fit", Ty::Choice(&["contain", "cover"]), "как видео/фото ложится в кадр: contain — целиком (поля), cover — заполнить кадр (края обрезаются)"),
    ],
    danger: false,
    run: clip_set,
};

fn clip_set(ed: &mut Editor, a: &Args) -> Res {
    let Some(ids) = a.ids("ids")? else {
        return clip_set_one(ed, a);
    };
    if a.has("id") {
        return Err("clip.set: укажите id или ids, не оба".into());
    }
    if ids.is_empty() {
        return Err("clip.set: ids пуст".into());
    }
    // всё на копии проекта: ошибка у любого клипа — не меняется ни один
    let mut tmp = Editor::new();
    tmp.project = ed.project.clone();
    let mut lines = Vec::new();
    for id in &ids {
        let mut v = a.v.clone();
        v.remove("ids");
        v.insert("id".into(), Value::from(*id));
        lines.push(clip_set_one(&mut tmp, &Args { cmd: a.cmd, v })?.text);
    }
    ed.begin("Свойства клипа");
    ed.project = tmp.project;
    Ok(Out::text(lines.join("\n")))
}

fn clip_set_one(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let c = ed.project.clip(id).ok_or_else(|| format!("нет клипа c{id}"))?.clone();
    let speed = a.num("speed")?;
    if let Some(s) = speed {
        if !(0.05..=20.0).contains(&s) {
            return Err(format!("скорость {s} вне 0.05–20"));
        }
    }
    let volume = a.num("volume")?;
    if volume.is_some_and(|v| !(0.0..=4.0).contains(&v)) {
        return Err("громкость вне 0–4".into());
    }
    let (text, shape, style) = (a.str("text")?, a.str("shape")?, a.object("style")?);
    if text.is_some() && !matches!(c.source, Source::Title { .. }) {
        return Err(format!("c{id} не титр"));
    }
    if let Some(sk) = &shape {
        if !matches!(c.source, Source::Shape { .. }) {
            return Err(format!("c{id} не фигура"));
        }
        if !crate::doc::SHAPE_KINDS.iter().any(|(k, _)| k == sk) {
            return Err(format!("нет фигуры «{sk}»"));
        }
    }
    if style.is_some() && !matches!(c.source, Source::Title { .. } | Source::Shape { .. }) {
        return Err(format!("c{id} не титр и не фигура"));
    }
    let blend = a.str("blend")?;
    if let Some(b) = &blend {
        if !crate::doc::BLEND_MODES.iter().any(|(k, _)| k == b) {
            return Err(format!("нет режима наложения «{b}»; есть: {}", crate::doc::BLEND_MODES.iter().map(|m| m.0).collect::<Vec<_>>().join(", ")));
        }
    }
    let parent_opt: Option<Option<Id>> = if a.has("parent") {
        match a.raw("parent") {
            Some(Value::Number(n)) if n.as_u64() == Some(0) => Some(None),
            Some(Value::String(s)) if s.trim().is_empty() || s.trim() == "0" || s.trim() == "none" => Some(None),
            Some(v) => {
                let pid = super::parse_id(v).ok_or_else(|| "parent: укажите id клипа (c…) или 0".to_string())?;
                if ed.project.clip(pid).is_none() {
                    return Err(format!("нет родительского клипа c{pid}"));
                }
                if ed.project.is_ancestor(id, pid) {
                    return Err(format!("нельзя привязать c{id} к c{pid}: получится цикл"));
                }
                Some(Some(pid))
            }
            None => Some(None),
        }
    } else {
        None
    };
    let peer_id = ed.project.linked_peer(id);
    let is_video_track = ed.project.find_clip(id).is_some_and(|(ti, _)| ed.project.tracks[ti].kind == TrackKind::Video);
    let new_dur = speed.map(|s| c.duration * c.speed / s);
    let peer_new_dur = match (speed, peer_id.and_then(|pid| ed.project.clip(pid))) {
        (Some(s), Some(pc)) => Some((pc.id, pc.duration * pc.speed / s, pc.start)),
        _ => None,
    };
    if let Some(d) = new_dur {
        let (ti, _) = ed.project.find_clip(id).unwrap();
        if ed.project.tracks[ti].clips.iter().any(|o| o.id != id && o.start < c.start + d - EPS && o.end() > c.start + EPS) {
            return Err(format!("c{id}: при скорости {} клип налезет на соседний (длина {})", speed.unwrap(), tc(d)));
        }
    }
    if let Some((pid, pd, pstart)) = peer_new_dur {
        let (pti, _) = ed.project.find_clip(pid).unwrap();
        if ed.project.tracks[pti].clips.iter().any(|o| o.id != pid && o.start < pstart + pd - EPS && o.end() > pstart + EPS) {
            return Err(format!("c{pid}: при скорости {} связанный звук налезет на соседний", speed.unwrap()));
        }
    }
    ed.begin("Свойства клипа");
    let fps = ed.project.settings.fps;
    let route_vol_to_peer = volume.is_some()
        && is_video_track
        && peer_id.and_then(|pid| ed.project.find_clip(pid)).is_some_and(|(pti, _)| ed.project.tracks[pti].kind == TrackKind::Audio);
    let cm = ed.project.clip_mut(id).unwrap();
    if let (Some(s), Some(d)) = (speed, new_dur) {
        cm.speed = s;
        cm.duration = ((d * fps).round() / fps).max(1.0 / fps);
    }
    if let Some(v) = volume {
        if !route_vol_to_peer {
            cm.volume = v;
        }
    }
    if let Some(n) = a.str("name")? {
        cm.name = n;
    }
    if let Some(b) = blend {
        cm.blend = if b == "normal" { String::new() } else { b };
    }
    if let Some(f) = a.str("fit")? {
        cm.fit = if f == "cover" { f } else { String::new() };
    }
    if let Some(par) = parent_opt {
        cm.parent = par;
    }
    match &mut cm.source {
        Source::Title { text: t, style: st } => {
            if let Some(x) = text {
                *t = x;
            }
            if let Some(s) = style {
                st.extend(s);
            }
        }
        Source::Shape { kind: k, style: st } => {
            if let Some(sk) = shape {
                *k = sk;
            }
            if let Some(s) = style {
                st.extend(s);
            }
        }
        _ => {}
    }
    if let (Some(s), Some((pid, pd, _))) = (speed, peer_new_dur) {
        if let Some(pm) = ed.project.clip_mut(pid) {
            pm.speed = s;
            pm.duration = ((pd * fps).round() / fps).max(1.0 / fps);
        }
    }
    if route_vol_to_peer {
        if let (Some(v), Some(pid)) = (volume, peer_id) {
            if let Some(pm) = ed.project.clip_mut(pid) {
                pm.volume = v;
            }
        }
    }
    let cm = ed.project.clip(id).unwrap();
    let eff_vol = if route_vol_to_peer { peer_id.and_then(|pid| ed.project.clip(pid)).map(|p| p.volume).unwrap_or(cm.volume) } else { cm.volume };
    Ok(Out::text(format!("c{id} {}–{} скорость {} громкость {}", tc(cm.start), tc(cm.end()), cm.speed, eff_vol)))
}

// ── эффекты ─────────────────────────────────────────────────────────────

/// Известные эффекты: вид и описание параметров (для модели и инспектора).
pub const EFFECTS: &[(&str, &str)] = &[
    ("fade_in", "проявление из чёрного: duration (с)"),
    ("fade_out", "затухание в чёрный: duration (с)"),
    ("audio_fade_in", "нарастание звука: duration (с)"),
    ("audio_fade_out", "затухание звука: duration (с)"),
    ("lut", "цветокоррекция LUT: path (.cube), strength 0–1"),
    ("grade", "первичная коррекция (колёса и кривые, как в Resolve): lift/gamma/gain — тени/средние/света: яркость −1…1 и оттенок точкой в круге *_x, *_y (длина 0–1 — сила). Направления (x, y): красный (1, 0), тёплый оранжевый (0.87, 0.5), жёлтый (0.5, 0.87), зелёный (−0.5, 0.87), голубой/холодный (−1, 0), синий (−0.5, −0.87), пурпурный (0.5, −0.87); умеренно — длина 0.2–0.5. curve, curve_r, curve_g, curve_b — точки [[x, y], …] 0–1 (S-кривая контраста: [[0.25, 0.2], [0.75, 0.8]])"),
    ("color", "яркость/контраст/насыщенность/температура: brightness, contrast, saturation, temperature (−1…1)"),
    ("blur", "размытие: radius (пикс.)"),
    ("sharpen", "резкость: amount 0–2"),
    ("vignette", "виньетка: amount 0–1"),
    ("transform", "положение и масштаб: x, y (доли кадра), scale, rotation (°), opacity 0–1"),
    ("mask", "маска слоя: shape (rect|ellipse), x, y, width, height, radius (0–0.5), feather (0–0.5), invert (bool)"),
    ("glass", "матовое стекло (glassmorphism для фигур rect/ellipse): blur (0–80 пикс.), tint (#цвет), tint_opacity (0–1), border_opacity (0–1), highlight (0–1), shadow (0–1), saturation (0–2.5), brightness (−0.5…0.5)"),
    ("upscale", "нейроувеличение synaptix-vfx: model (plksr|span|compact|nanovsr), scale 2|4"),
    ("interpolate", "нейроинтерполяция RIFE до частоты проекта (плавность, замедление)"),
];

static EFFECT_ADD: Command = Command {
    id: "effect.add",
    category: "Эффекты",
    title: "Добавить эффект",
    shortcut: "",
    keywords: "effect фильтр эффект lut цвет грейдинг колёса кривые lift gamma gain тени света размытие затухание fade маска mask стекло glass glassmorphism upscale rife увеличение плавность",
    help: "Добавить эффект клипам. Виды: fade_in, fade_out, audio_fade_in, audio_fade_out, lut {path, strength}, grade {lift, gamma, gain (−1…1), lift_x/lift_y, gamma_x/gamma_y, gain_x/gain_y (оттенок теней/средних/светов точкой в круге: тёплый (0.87, 0.5), холодный (−1, 0), синий (−0.5, −0.87), зелёный (−0.5, 0.87), пурпурный (0.5, −0.87); сила — длина 0.2–0.5), curve, curve_r, curve_g, curve_b ([[x,y],…] 0–1)} (колёса и кривые; до LUT), color {brightness, contrast, saturation, temperature}, blur {radius}, sharpen {amount}, vignette {amount}, transform {x, y, scale, rotation, opacity}, mask {shape: rect|ellipse, x, y, width, height, radius, feather, invert}, glass {blur, tint, tint_opacity, border_opacity, highlight, shadow, saturation, brightness} (матовое стекло с размытием фона под скруглённой фигурой), upscale {model, scale} (нейроувеличение), interpolate (RIFE до fps проекта).",
    args: &[req("ids", Ty::Ids, "клипы (c…)"), req("kind", Ty::Str, "вид эффекта"), arg("params", Ty::Object, "параметры")],
    danger: false,
    run: effect_add,
};

fn effect_add(ed: &mut Editor, a: &Args) -> Res {
    let ids = a.ids("ids")?.unwrap_or_default();
    let kind = a.need_str("kind")?;
    if !EFFECTS.iter().any(|(k, _)| *k == kind) {
        return Err(format!("нет эффекта «{kind}»; есть: {}", EFFECTS.iter().map(|e| e.0).collect::<Vec<_>>().join(", ")));
    }
    let params = a.object("params")?.unwrap_or_default();
    if kind == "lut" {
        let path = params.get("path").and_then(Value::as_str).ok_or("lut: нужен params.path к файлу .cube")?;
        let p = crate::doc::expand(path);
        if !p.is_file() {
            return Err(format!("lut: файла нет: {}", p.display()));
        }
    }
    for id in &ids {
        ed.project.find_clip(*id).ok_or_else(|| format!("нет клипа c{id}"))?;
    }
    // звуковой эффект на видеоклип со связанным звуком → перенаправляем на аудиоклип;
    // видеоэффект на выделенную пару [видео, звук] → не вешаем на аудиоклип
    let is_audio_fx = kind.starts_with("audio_");
    let mut targets = Vec::with_capacity(ids.len());
    for &id in &ids {
        let (ti, _) = ed.project.find_clip(id).unwrap();
        let tr_kind = ed.project.tracks[ti].kind;
        let target = if is_audio_fx && tr_kind == TrackKind::Video {
            let c = ed.project.clip(id).unwrap();
            if c.volume == 0.0 {
                ed.project.linked_peer(id).unwrap_or(id)
            } else {
                id
            }
        } else if !is_audio_fx && tr_kind == TrackKind::Audio {
            if let Some(peer) = ed.project.linked_peer(id) {
                if ids.contains(&peer) {
                    continue;
                }
            }
            id
        } else {
            id
        };
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
    ed.begin("Добавить эффект");
    let mut made = Vec::new();
    for id in &targets {
        // у клипа граф кадра — эффект изображения встаёт нодой перед выходом
        if crate::framegraph::IMAGE_FX.contains(&kind.as_str()) {
            if let Some(g) = ed.project.clip_mut(*id).unwrap().graph.as_mut() {
                let nid = g.next_id();
                let out = g.nodes.iter().find(|n| n.kind == "output").map(|n| (n.id, n.x, n.y));
                if let Some((oid, ox, oy)) = out {
                    let prev = g.input(oid, "in");
                    g.links.retain(|l| !(l.to == oid && l.port == "in"));
                    if let Some(p) = prev {
                        g.links.push(crate::framegraph::GLink { from: p, to: nid, port: "in".into() });
                    }
                    g.links.push(crate::framegraph::GLink { from: nid, to: oid, port: "in".into() });
                    g.nodes.push(crate::framegraph::GNode { id: nid, kind: kind.clone(), params: params.clone(), x: ox, y: oy + 120.0 });
                    made.push(format!("c{id}: нода {nid} графа кадра"));
                    continue;
                }
            }
        }
        let eid = ed.project.alloc();
        let mut params: Map<String, Value> = params.clone();
        if kind == "lut" {
            if let Some(Value::String(p)) = params.get("path").cloned() {
                params.insert("path".into(), Value::String(crate::doc::expand(&p).display().to_string()));
            }
        }
        ed.project.clip_mut(*id).unwrap().effects.push(Effect { id: eid, kind: kind.clone(), params });
        made.push(format!("c{id}: e{eid}"));
    }
    Ok(Out::text(format!("{kind}: {}", made.join(", "))))
}

static EFFECT_SET: Command = Command {
    id: "effect.set",
    category: "Эффекты",
    title: "Параметры эффекта",
    shortcut: "",
    keywords: "effect params параметры эффекта",
    help: "Изменить параметры эффекта (поля сливаются с прежними).",
    args: &[req("id", Ty::Id, "эффект (e…)"), req("params", Ty::Object, "параметры")],
    danger: false,
    run: effect_set,
};

fn find_effect(p: &Project, eid: Id) -> Option<(Id, usize)> {
    p.tracks.iter().flat_map(|t| &t.clips).find_map(|c| c.effects.iter().position(|e| e.id == eid).map(|i| (c.id, i)))
}

fn effect_set(ed: &mut Editor, a: &Args) -> Res {
    let eid = a.need_id("id")?;
    let (cid, i) = find_effect(&ed.project, eid).ok_or_else(|| format!("нет эффекта e{eid}"))?;
    let params = a.object("params")?.unwrap_or_default();
    ed.begin("Параметры эффекта");
    let e = &mut ed.project.clip_mut(cid).unwrap().effects[i];
    e.params.extend(params);
    Ok(Out::text(format!("e{eid} {} {}", e.kind, Value::Object(e.params.clone()))))
}

static EFFECT_REMOVE: Command = Command {
    id: "effect.remove",
    category: "Эффекты",
    title: "Убрать эффект",
    shortcut: "",
    keywords: "effect remove убрать эффект",
    help: "Убрать эффект с клипа.",
    args: &[req("id", Ty::Id, "эффект (e…)")],
    danger: false,
    run: effect_remove,
};

fn effect_remove(ed: &mut Editor, a: &Args) -> Res {
    let eid = a.need_id("id")?;
    let (cid, i) = find_effect(&ed.project, eid).ok_or_else(|| format!("нет эффекта e{eid}"))?;
    ed.begin("Убрать эффект");
    let c = ed.project.clip_mut(cid).unwrap();
    let e = c.effects.remove(i);
    c.keys.retain(|k, _| !k.starts_with(&format!("e{eid}.")));
    Ok(Out::text(format!("Убран e{eid} {} с c{cid}", e.kind)))
}

// ── переходы ────────────────────────────────────────────────────────────

static TRANSITION_SET: Command = Command {
    id: "transition.set",
    category: "Эффекты",
    title: "Переход",
    shortcut: "",
    keywords: "transition переход наплыв dissolve сдвиг шторка зум склейка fusion шаблон",
    help: "Переход в начале клипа от предыдущего на той же дорожке (клипы должны стоять встык; переход по центру склейки). kind: dissolve (наплыв), dip_black (через чёрное), slide_left, slide_right, slide_up, slide_down, wipe_left, wipe_right, zoom, blur, либо fusion (с параметром template — имя из transition.list или путь к .setting; также допускается сокращение fusion:<имя>). duration — с (0.6 для встроенных, 0.8 для Fusion). params — переопределения входов MacroOperator ({ключ: значение, null — сбросить}); reset: true — сбросить все параметры.",
    args: &[
        req("id", Ty::Id, "клип (c…), к началу которого переход"),
        req("kind", Ty::Str, "вид перехода (встроенный или fusion)"),
        arg("duration", Ty::Time, "длительность, с"),
        arg("template", Ty::Str, "имя или путь .setting перехода Fusion (когда kind=fusion)"),
        arg("params", Ty::Object, "параметры MacroOperator перехода Fusion ({ключ: значение})"),
        arg("reset", Ty::Bool, "сбросить параметры перехода Fusion (true)"),
    ],
    danger: false,
    run: transition_set,
};

fn resolve_fusion_transition(query: &str) -> Result<std::path::PathBuf, String> {
    let q = query.trim();
    if q.is_empty() {
        return Err("укажите template (имя из transition.list или путь к .setting)".into());
    }
    let direct = std::path::Path::new(q);
    if direct.is_file() {
        return Ok(direct.to_path_buf());
    }
    let all = crate::fusion::scan(&crate::fusion::library());
    let needle = q.to_lowercase();
    let needle_sp = needle.replace('_', " ");
    let is_tr = |cat: &str| cat.starts_with("Переходы") || cat.to_lowercase().contains("transition");
    // Сначала ищем в разделе «Переходы» (точное имя, затем подстрока), затем по всей библиотеке.
    if let Some((_, _, p)) = all.iter().find(|(c, n, _)| is_tr(c) && (n.to_lowercase() == needle || n.to_lowercase().replace('_', " ") == needle_sp)) {
        return Ok(p.clone());
    }
    if let Some((_, _, p)) = all.iter().find(|(c, n, _)| is_tr(c) && (n.to_lowercase().contains(&needle) || n.to_lowercase().replace('_', " ").contains(&needle_sp))) {
        return Ok(p.clone());
    }
    if let Some((_, _, p)) = all.iter().find(|(_, n, _)| n.to_lowercase() == needle || n.to_lowercase().replace('_', " ") == needle_sp) {
        return Ok(p.clone());
    }
    if let Some((_, _, p)) = all.iter().find(|(_, n, _)| n.to_lowercase().contains(&needle) || n.to_lowercase().replace('_', " ").contains(&needle_sp)) {
        return Ok(p.clone());
    }
    Err(format!("переход Fusion «{q}» не найден — посмотрите список через transition.list"))
}

fn transition_set(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let raw_kind = a.need_str("kind")?;
    let (kind, inline_tpl) = match raw_kind.strip_prefix("fusion:") {
        Some(rest) => ("fusion".to_string(), Some(rest.to_string())),
        None => (raw_kind, None),
    };
    let is_fusion = kind == "fusion";
    if !is_fusion && !crate::render::compose::TRANSITIONS.iter().any(|(k, _)| *k == kind) {
        return Err(format!("нет перехода «{kind}»; есть: {}, fusion", crate::render::compose::TRANSITIONS.iter().map(|t| t.0).collect::<Vec<_>>().join(", ")));
    }
    let (ti, ci) = ed.project.find_clip(id).ok_or_else(|| format!("нет клипа c{id}"))?;
    let c = &ed.project.tracks[ti].clips[ci];
    let prev = ci.checked_sub(1).map(|j| &ed.project.tracks[ti].clips[j]);
    let Some(prev) = prev.filter(|p| (p.end() - c.start).abs() < 1e-3) else {
        return Err(format!("c{id}: перед ним на дорожке нет клипа встык — переходу не из чего"));
    };
    let existing = c.transition.clone();
    let max = prev.duration.min(c.duration);
    let fps = ed.project.settings.fps.max(1.0);

    if is_fusion {
        let tpl_arg = a.str("template")?.or(inline_tpl);
        let (tpl_path, mut params) = match tpl_arg {
            Some(q) => {
                let p = resolve_fusion_transition(&q)?;
                // проверяем, что .setting разбирается
                crate::fusion::load_cached(&p)?;
                let ps = p.display().to_string();
                let keep = existing.as_ref().filter(|t| t.template.as_deref() == Some(ps.as_str())).map(|t| t.params.clone()).unwrap_or_default();
                (ps, keep)
            }
            None => match existing.as_ref().and_then(|t| t.template.clone()).filter(|s| !s.is_empty()) {
                Some(p) => (p, existing.as_ref().map(|t| t.params.clone()).unwrap_or_default()),
                None => return Err("для kind=fusion укажите template (имя из transition.list или путь к .setting)".into()),
            },
        };
        if a.bool("reset")?.unwrap_or(false) {
            params.clear();
        }
        if let Some(obj) = a.object("params")? {
            for (raw_k, val) in obj {
                let k = raw_k.strip_prefix(crate::fusion::STYLE_PREFIX).unwrap_or(&raw_k);
                let sk = format!("{}{k}", crate::fusion::STYLE_PREFIX);
                if val.is_null() {
                    params.remove(&sk);
                    params.remove(&format!("{sk}.x"));
                    params.remove(&format!("{sk}.y"));
                } else if let Some(arr) = val.as_array().filter(|arr| arr.len() == 2 && arr[0].is_f64() && arr[1].is_f64()) {
                    params.insert(format!("{sk}.x"), arr[0].clone());
                    params.insert(format!("{sk}.y"), arr[1].clone());
                } else {
                    params.insert(sk, val);
                }
            }
        }
        let def_d = existing.as_ref().filter(|t| t.is_fusion()).map(|t| t.duration).unwrap_or(0.8);
        let d = a.time("duration")?.unwrap_or(def_d).clamp(1.0 / fps, max);
        let d_snap = (d * fps).round() / fps;
        let mut tr = crate::doc::Transition::fusion(tpl_path, d_snap);
        tr.params = params;
        let title = tr.title();
        let data = json!({"id": id, "kind": tr.kind, "duration": tr.duration, "template": tr.template});
        ed.begin("Переход Fusion");
        ed.project.tracks[ti].clips[ci].transition = Some(tr);
        return Ok(Out::with(format!("c{id}: переход Fusion «{title}» {d_snap:.2} с"), data));
    }

    let d = a.time("duration")?.unwrap_or(0.6).clamp(1.0 / fps, max);
    let d_snap = (d * fps).round() / fps;
    let tr = crate::doc::Transition::builtin(kind.clone(), d_snap);
    let data = json!({"id": id, "kind": tr.kind, "duration": tr.duration});
    ed.begin("Переход");
    ed.project.tracks[ti].clips[ci].transition = Some(tr);
    Ok(Out::with(format!("c{id}: переход {kind} {d_snap:.2} с"), data))
}

static TRANSITION_LIST: Command = Command {
    id: "transition.list",
    category: "Эффекты",
    title: "Список переходов",
    shortcut: "",
    keywords: "transition list переходы fusion библиотека поиск",
    help: "Список встроенных переходов и переходов Fusion (.setting из раздела «Переходы» библиотеки). query — фильтр по имени или подразделу (Zoom, Spin, Glitch, Pan, Whip…).",
    args: &[arg("query", Ty::Str, "фильтр по имени или подразделу"), arg("limit", Ty::Num, "сколько переходов Fusion показать (30)")],
    danger: false,
    run: transition_list,
};

fn transition_list(_ed: &mut Editor, a: &Args) -> Res {
    let q = a.str("query")?.unwrap_or_default().to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    let limit = a.num("limit")?.unwrap_or(30.0).max(1.0) as usize;
    let builtins: Vec<(&str, &str)> = crate::render::compose::TRANSITIONS
        .iter()
        .copied()
        .filter(|(k, l)| {
            let hay = format!("{k} {l}").to_lowercase();
            words.iter().all(|w| hay.contains(w))
        })
        .collect();
    let all = crate::fusion::scan(&crate::fusion::library());
    let fusion_hits: Vec<_> = all
        .into_iter()
        .filter(|(cat, name, _)| {
            let is_tr = cat.starts_with("Переходы") || cat.to_lowercase().contains("transition");
            if !is_tr {
                return false;
            }
            let hay = format!("{cat} {name}").to_lowercase().replace('_', " ");
            words.iter().all(|w| hay.contains(w))
        })
        .collect();
    let mut lines = Vec::new();
    if !builtins.is_empty() {
        lines.push(format!("Встроенные ({}): {}", builtins.len(), builtins.iter().map(|(k, l)| format!("{k} ({l})")).collect::<Vec<_>>().join(", ")));
    }
    lines.push(format!("Fusion-переходы: найдено {}", fusion_hits.len()));
    for (cat, name, _) in fusion_hits.iter().take(limit) {
        lines.push(format!("  {name} — {cat}"));
    }
    Ok(Out::with(
        lines.join("\n"),
        json!({
            "builtin": builtins.iter().map(|(k, l)| json!({"kind": k, "title": l})).collect::<Vec<_>>(),
            "fusion_total": fusion_hits.len(),
            "fusion": fusion_hits.iter().take(limit).map(|(c, n, p)| json!({"name": n, "category": c, "path": p.display().to_string()})).collect::<Vec<_>>(),
        }),
    ))
}

static TRANSITION_REMOVE: Command = Command {
    id: "transition.remove",
    category: "Эффекты",
    title: "Убрать переход",
    shortcut: "",
    keywords: "transition remove убрать переход",
    help: "Убрать переход в начале клипа.",
    args: &[req("id", Ty::Id, "клип (c…)")],
    danger: false,
    run: transition_remove,
};

fn transition_remove(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    ed.project.clip(id).ok_or_else(|| format!("нет клипа c{id}"))?;
    ed.begin("Убрать переход");
    ed.project.clip_mut(id).unwrap().transition = None;
    Ok(Out::text(format!("c{id}: переход убран")))
}

// ── сдвиг, копирование, вставка ─────────────────────────────────────────

static CLIP_SHIFT: Command = Command {
    id: "clip.shift",
    category: "Таймлайн",
    title: "Сдвинуть клипы",
    shortcut: "",
    keywords: "shift nudge сдвинуть подвинуть кадр несколько",
    help: "Сдвинуть клипы по времени на by секунд (минус — влево), каждый на своей дорожке; без ids — выделенные. Не налезает на другие клипы и не уходит левее нуля.",
    args: &[req("by", Ty::Num, "сдвиг, с"), arg("ids", Ty::Ids, "клипы (c…), по умолчанию выделенные")],
    danger: false,
    run: clip_shift,
};

fn clip_shift(ed: &mut Editor, a: &Args) -> Res {
    let raw_ids = a.ids("ids")?.unwrap_or_else(|| ed.selection.clone());
    if raw_ids.is_empty() {
        return Err("нечего сдвигать: выделите клипы".into());
    }
    for id in &raw_ids {
        ed.project.find_clip(*id).ok_or_else(|| format!("нет клипа c{id}"))?;
    }
    let ids = expand_linked(&ed.project, &raw_ids);
    let raw = a.num("by")?.unwrap_or(0.0);
    let by = raw.signum() * ed.project.snap(raw.abs());
    let p = &ed.project;
    for id in &ids {
        let (ti, _) = p.find_clip(*id).ok_or_else(|| format!("нет клипа c{id}"))?;
        let c = p.clip(*id).unwrap();
        let s = c.start + by;
        if s < -EPS {
            return Err(format!("c{id} уйдёт левее нуля"));
        }
        if let Some(o) = p.tracks[ti].clips.iter().find(|o| !ids.contains(&o.id) && o.start < s + c.duration - EPS && o.end() > s + EPS) {
            return Err(format!("c{id} налезет на c{}", o.id));
        }
    }
    ed.begin("Сдвинуть клипы");
    for id in &ids {
        let (ti, _) = ed.project.find_clip(*id).unwrap();
        let c = ed.project.clip_mut(*id).unwrap();
        c.start = (c.start + by).max(0.0);
        ed.project.tracks[ti].clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    }
    Ok(Out::text(format!("Сдвинуто клипов: {} на {by:+.3} с", ids.len())))
}

static CLIP_COPY: Command = Command {
    id: "clip.copy",
    category: "Таймлайн",
    title: "Копировать клипы",
    shortcut: "Ctrl+C",
    keywords: "copy копировать скопировать",
    help: "Запомнить клипы для вставки (без ids — выделенные).",
    args: &[arg("ids", Ty::Ids, "клипы (c…), по умолчанию выделенные")],
    danger: false,
    run: clip_copy,
};

fn clip_copy(ed: &mut Editor, a: &Args) -> Res {
    let raw_ids = a.ids("ids")?.unwrap_or_else(|| ed.selection.clone());
    if raw_ids.is_empty() {
        return Err("нечего копировать: выделите клипы".into());
    }
    for id in &raw_ids {
        ed.project.find_clip(*id).ok_or_else(|| format!("нет клипа c{id}"))?;
    }
    let ids = expand_linked(&ed.project, &raw_ids);
    let mut buf = Vec::new();
    for id in &ids {
        let (ti, _) = ed.project.find_clip(*id).unwrap();
        buf.push((ed.project.tracks[ti].id, ed.project.clip(*id).unwrap().clone()));
    }
    ed.clipboard = buf;
    Ok(Out::text(format!("Скопировано клипов: {}", ids.len())))
}

static CLIP_PASTE: Command = Command {
    id: "clip.paste",
    category: "Таймлайн",
    title: "Вставить клипы",
    shortcut: "Ctrl+V",
    keywords: "paste вставить",
    help: "Вставить скопированные клипы с момента at (по умолчанию — позиция воспроизведения), сохраняя их взаимное расположение. На прежние дорожки, если там свободно; иначе — на свободную дорожку того же вида выше или новую. Вставленные клипы выделяются.",
    args: &[arg("at", Ty::Time, "куда вставить")],
    danger: false,
    run: clip_paste,
};

fn clip_paste(ed: &mut Editor, a: &Args) -> Res {
    if ed.clipboard.is_empty() {
        return Err("буфер пуст: сначала скопируйте клипы".into());
    }
    let at = ed.project.snap(a.time("at")?.unwrap_or(ed.playhead));
    let base = ed.clipboard.iter().map(|(_, c)| c.start).fold(f64::MAX, f64::min);
    let buf = ed.clipboard.clone();
    ed.begin("Вставить клипы");
    let mut new_ids = Vec::new();
    let mut id_map: std::collections::HashMap<Id, Id> = std::collections::HashMap::new();
    // по дорожкам: клипы одной дорожки едут вместе (уникальные дорожки без повторов)
    let mut tracks: Vec<Id> = Vec::new();
    for (t, _) in &buf {
        if !tracks.contains(t) {
            tracks.push(*t);
        }
    }
    for tid in tracks {
        let group: Vec<Clip> = buf.iter().filter(|(t, _)| *t == tid).map(|(_, c)| c.clone()).collect();
        let kind = match ed.project.track(tid) {
            Some(t) => t.kind,
            None => match group[0].media().and_then(|m| ed.project.media(m)) {
                Some(m) if m.kind == MediaKind::Audio => TrackKind::Audio,
                _ => TrackKind::Video,
            },
        };
        let fits = |p: &Project, ti: usize| group.iter().all(|c| {
            let s = at + c.start - base;
            p.tracks[ti].clips.iter().all(|o| o.end() <= s + EPS || o.start >= s + c.duration - EPS)
        });
        let same: Vec<usize> = (0..ed.project.tracks.len()).filter(|i| ed.project.tracks[*i].kind == kind).collect();
        let from = same.iter().position(|i| ed.project.tracks[*i].id == tid).unwrap_or(0);
        let ti = match same.iter().skip(from).copied().find(|ti| fits(&ed.project, *ti)) {
            Some(ti) => ti,
            None => {
                let n = ed.project.tracks.iter().filter(|t| t.kind == kind).count() + 1;
                let id = ed.project.alloc();
                let name = if kind == TrackKind::Video { format!("V{n}") } else { format!("A{n}") };
                ed.project.tracks.push(Track { id, kind, name, muted: false, hidden: false, clips: Vec::new() });
                ed.project.tracks.len() - 1
            }
        };
        for mut c in group {
            let old_id = c.id;
            c.start = at + c.start - base;
            c.id = ed.project.alloc();
            c.renew_effect_ids(|| ed.project.alloc());
            id_map.insert(old_id, c.id);
            new_ids.push(c.id);
            place(&mut ed.project, ti, c, false);
        }
    }
    for &nid in &new_ids {
        if let Some(c) = ed.project.clip_mut(nid) {
            c.link = c.link.and_then(|old| id_map.get(&old).copied());
        }
    }
    ed.selection = new_ids.clone();
    let list: Vec<String> = new_ids.iter().map(|i| format!("c{i}")).collect();
    Ok(Out::text(format!("Вставлено в {}: {}", tc(at), list.join(", "))))
}

static CLIP_DETACH_AUDIO: Command = Command {
    id: "clip.detach_audio",
    category: "Таймлайн",
    title: "Отделить / отвязать звук",
    shortcut: "",
    keywords: "detach unlink audio отделить отвязать звук аудио разделить дорожка",
    help: "Если видео и звук на аудиодорожке связаны — разрывает связь (их можно двигать и резать порознь). Если у видеоклипа встроенный звук — выносит его отдельным клипом на звуковую дорожку (у видеоклипа громкость становится 0). Без ids — выделенные.",
    args: &[arg("ids", Ty::Ids, "клипы (c…), по умолчанию выделенные")],
    danger: false,
    run: clip_detach_audio,
};

fn clip_detach_audio(ed: &mut Editor, a: &Args) -> Res {
    let raw_ids = a.ids("ids")?.unwrap_or_else(|| ed.selection.clone());
    if raw_ids.is_empty() {
        return Err("выделите видеоклип".into());
    }
    // если выделены оба клипа пары — обрабатываем пару один раз
    let mut ids = Vec::new();
    for &id in &raw_ids {
        let peer_already = ed.project.linked_peer(id).is_some_and(|p| ids.contains(&p));
        if !ids.contains(&id) && !peer_already {
            ids.push(id);
        }
    }
    for &id in &ids {
        let c = ed.project.clip(id).ok_or_else(|| format!("нет клипа c{id}"))?;
        if ed.project.linked_peer(id).is_some() {
            continue;
        }
        let m = c.media().and_then(|m| ed.project.media(m)).ok_or_else(|| format!("c{id} не из медиатеки"))?;
        if m.kind != MediaKind::Video || !m.has_audio {
            return Err(format!("у c{id} нет звука для отделения"));
        }
        if c.volume == 0.0 {
            return Err(format!("звук c{id} уже отделён или выключен"));
        }
    }
    ed.begin("Отделить звук");
    let mut out = Vec::new();
    let mut unlinked_peers = Vec::new();
    for &id in &ids {
        if let Some(peer) = ed.project.linked_peer(id) {
            ed.project.unlink(id);
            unlinked_peers.push(peer);
            out.push(format!("c{id} ✕ c{peer}"));
            continue;
        }
        let c = ed.project.clip(id).unwrap().clone();
        let fits = |p: &Project, ti: usize| p.tracks[ti].clips.iter().all(|o| o.end() <= c.start + EPS || o.start >= c.end() - EPS);
        let ti = match (0..ed.project.tracks.len()).find(|ti| ed.project.tracks[*ti].kind == TrackKind::Audio && fits(&ed.project, *ti)) {
            Some(ti) => ti,
            None => {
                let n = ed.project.tracks.iter().filter(|t| t.kind == TrackKind::Audio).count() + 1;
                let tid = ed.project.alloc();
                ed.project.tracks.push(Track { id: tid, kind: TrackKind::Audio, name: format!("A{n}"), muted: false, hidden: false, clips: Vec::new() });
                ed.project.tracks.len() - 1
            }
        };
        let mut au = c.clone();
        au.id = ed.project.alloc();
        au.transition = None;
        au.link = None;
        // звуковые эффекты едут со звуком, видео — остаются
        au.effects = c.effects.iter().filter(|e| e.kind.starts_with("audio_")).cloned().collect();
        au.renew_effect_ids(|| ed.project.alloc());
        let nid = au.id;
        let tid = ed.project.tracks[ti].id;
        place(&mut ed.project, ti, au, false);
        let v = ed.project.clip_mut(id).unwrap();
        v.volume = 0.0;
        v.effects.retain(|e| !e.kind.starts_with("audio_"));
        out.push(format!("c{id} → c{nid} (t{tid})"));
    }
    if !unlinked_peers.is_empty() {
        ed.selection.retain(|s| !unlinked_peers.contains(s));
    }
    Ok(Out::text(format!("Звук отделён: {}", out.join(", "))))
}

static TRACK_MOVE: Command = Command {
    id: "track.move",
    category: "Таймлайн",
    title: "Переставить дорожку",
    shortcut: "",
    keywords: "track move order порядок выше ниже слой дорожка",
    help: "Переставить дорожку среди дорожек того же вида. to — место сверху вниз, как на таймлайне (0 — верхняя строка; у видео верхняя — верхний слой); или by: −1 — выше, +1 — ниже.",
    args: &[req("id", Ty::Id, "дорожка (t…)"), arg("to", Ty::Num, "место сверху (0…)"), arg("by", Ty::Num, "сдвиг: −1 выше, +1 ниже")],
    danger: false,
    run: track_move,
};

/// Дорожки вида `kind` в порядке строк таймлайна сверху вниз (индексы).
fn display_order(p: &Project, kind: TrackKind) -> Vec<usize> {
    let mut v: Vec<usize> = (0..p.tracks.len()).filter(|i| p.tracks[*i].kind == kind).collect();
    if kind == TrackKind::Video {
        v.reverse();
    }
    v
}

fn track_move(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let ti = ed.project.tracks.iter().position(|t| t.id == id).ok_or_else(|| format!("нет дорожки t{id}"))?;
    let kind = ed.project.tracks[ti].kind;
    let order = display_order(&ed.project, kind);
    let cur = order.iter().position(|i| *i == ti).unwrap();
    let to = match (a.num("to")?, a.num("by")?) {
        (Some(t), _) => t.round().max(0.0) as usize,
        (None, Some(b)) => (cur as i64 + b.round() as i64).max(0) as usize,
        _ => return Err("укажите to или by".into()),
    }
    .min(order.len() - 1);
    if to == cur {
        return Ok(Out::text(format!("t{id} уже на месте")));
    }
    ed.begin("Переставить дорожку");
    // новый порядок строк этого вида и перекладка в общем списке на те же места
    let mut rows: Vec<Track> = order.iter().map(|i| ed.project.tracks[*i].clone()).collect();
    let t = rows.remove(cur);
    rows.insert(to, t);
    if kind == TrackKind::Video {
        rows.reverse();
    }
    let mut slots: Vec<usize> = order.clone();
    slots.sort();
    for (slot, t) in slots.into_iter().zip(rows) {
        ed.project.tracks[slot] = t;
    }
    Ok(Out::text(format!("t{id} → место {to}")))
}
