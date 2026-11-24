//! Таймлайн: дорожки, клипы, титры, эффекты. Размещение клипа — с
//! перезаписью (наложенное обрезается) или со вставкой (сдвиг вправо).

use serde_json::{json, Map, Value};

use super::{arg, req, Args, Command, Out, Res, Ty};
use crate::doc::{tc, Clip, Effect, Id, MediaKind, Project, Source, Track, TrackKind};
use crate::editor::Editor;

pub static COMMANDS: &[&Command] = &[&TRACK_ADD, &TRACK_SET, &TRACK_REMOVE, &TRACK_MOVE, &CLIP_ADD, &TITLE_ADD, &CLIP_MOVE, &CLIP_TRIM, &CLIP_SPLIT, &CLIP_REMOVE, &CLIP_SET, &CLIP_SHIFT, &CLIP_COPY, &CLIP_PASTE, &CLIP_DETACH_AUDIO, &EFFECT_ADD, &EFFECT_SET, &EFFECT_REMOVE, &TRANSITION_SET, &TRANSITION_REMOVE];

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

fn add_track(p: &mut Project, kind: TrackKind, name: Option<String>) -> Id {
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
    let n = ed.project.track(id).ok_or_else(|| format!("нет дорожки t{id}"))?.clips.len();
    ed.begin("Удалить дорожку");
    ed.project.tracks.retain(|t| t.id != id);
    Ok(Out::text(format!("Удалена дорожка t{id} и клипов: {n}")))
}

// ── размещение ──────────────────────────────────────────────────────────

/// Освободить `[s, e)` на дорожке: наложенные клипы обрезать или разрезать.
fn clear_range(p: &mut Project, ti: usize, s: f64, e: f64) {
    let mut out = Vec::new();
    let clips = std::mem::take(&mut p.tracks[ti].clips);
    for c in clips {
        if c.end() <= s + EPS || c.start >= e - EPS {
            out.push(c);
            continue;
        }
        if c.start < s - EPS {
            let mut left = c.clone();
            left.duration = s - c.start;
            out.push(left);
        }
        if c.end() > e + EPS {
            let mut right = c.clone();
            if c.start < s - EPS {
                right.id = p.alloc();
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
}

/// Вставка: клипы правее `s` сдвинуть на `d` (пересекающий `s` — разрезать).
fn open_gap(p: &mut Project, ti: usize, s: f64, d: f64) {
    if let Some(c) = p.tracks[ti].clips.iter().find(|c| c.start < s - EPS && c.end() > s + EPS).map(|c| c.id) {
        split_clip(p, c, s);
    }
    for c in &mut p.tracks[ti].clips {
        if c.start >= s - EPS {
            c.start += d;
        }
    }
}

fn place(p: &mut Project, ti: usize, mut clip: Clip, insert: bool) -> Id {
    clip.start = p.snap(clip.start);
    clip.duration = p.snap(clip.duration).max(1.0 / p.settings.fps.max(1.0));
    if insert {
        open_gap(p, ti, clip.start, clip.duration);
    } else {
        clear_range(p, ti, clip.start, clip.end());
    }
    let id = clip.id;
    let t = &mut p.tracks[ti];
    t.clips.push(clip);
    t.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    id
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
fn target_track(p: &mut Project, track: Option<Id>, kind: TrackKind) -> Result<usize, String> {
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
    help: "Поставить медиа на таймлайн. Без at — в конец дорожки; без track — первая дорожка подходящего вида (видео и картинки — video, звук — audio). offset/duration — какой кусок источника взять. mode: overwrite (наложенное обрезается, по умолчанию) или insert (остальное сдвигается вправо).",
    args: &[
        req("media", Ty::Id, "медиа (m…)"),
        arg("track", Ty::Id, "дорожка (t…)"),
        arg("at", Ty::Time, "начало на таймлайне"),
        arg("offset", Ty::Time, "с какого места источника"),
        arg("duration", Ty::Time, "длительность на таймлайне"),
        arg("mode", Ty::Choice(MODES), "overwrite или insert"),
    ],
    danger: false,
    run: clip_add,
};

fn clip_add(ed: &mut Editor, a: &Args) -> Res {
    let mid = a.need_id("media")?;
    let m = ed.project.media(mid).ok_or_else(|| format!("нет медиа m{mid}"))?.clone();
    let kind = if m.kind == MediaKind::Audio { TrackKind::Audio } else { TrackKind::Video };
    let offset = a.time("offset")?.unwrap_or(0.0);
    if m.duration > 0.0 && offset >= m.duration {
        return Err(format!("offset {} за концом источника ({})", tc(offset), tc(m.duration)));
    }
    let avail = if m.duration > 0.0 { m.duration - offset } else { STILL };
    let duration = a.time("duration")?.unwrap_or(avail);
    if m.duration > 0.0 && duration > avail + 1e-3 {
        return Err(format!("в источнике после {} только {}", tc(offset), tc(avail)));
    }
    let insert = mode_insert(a)?;
    let (track, at) = (a.id("track")?, a.time("at")?);
    ed.begin("Добавить клип");
    let p = &mut ed.project;
    let ti = target_track(p, track, kind)?;
    let start = at.unwrap_or_else(|| track_end(p, ti));
    let id = p.alloc();
    let clip = Clip { id, source: Source::Media { media: mid }, start, duration, offset, speed: 1.0, volume: 1.0, name: String::new(), effects: Vec::new(), transition: None, keys: Default::default() };
    place(p, ti, clip, insert);
    let c = p.clip(id).unwrap();
    Ok(Out::with(format!("c{id} {} на t{} {}–{}", m.name(), p.tracks[ti].id, tc(c.start), tc(c.end())), json!({"id": id})))
}

static TITLE_ADD: Command = Command {
    id: "title.add",
    category: "Таймлайн",
    title: "Добавить титр",
    shortcut: "Ctrl+T",
    keywords: "title text титр надпись текст подпись",
    help: "Текстовый титр поверх видео. Без track — на верхнюю видеодорожку «Титры» (создаётся). style: {size, color, position: top|center|bottom, font}.",
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
    let clip = Clip { id, source: Source::Title { text: text.clone(), style }, start: at, duration, offset: 0.0, speed: 1.0, volume: 1.0, name: String::new(), effects: Vec::new(), transition: None, keys: Default::default() };
    place(p, ti, clip, false);
    let c = p.clip(id).unwrap();
    Ok(Out::with(format!("c{id} титр «{text}» {}–{}", tc(c.start), tc(c.end())), json!({"id": id})))
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
    place(p, to, clip, insert);
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
    let src = c.media().and_then(|m| ed.project.media(m)).map(|m| m.duration).filter(|d| *d > 0.0);
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
    ed.begin("Обрезать клип");
    let cm = ed.project.clip_mut(id).unwrap();
    cm.offset = (c.offset + (start - c.start) * c.speed).max(0.0);
    cm.start = start;
    cm.duration = end - start;
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
    let ids: Vec<Id> = match a.ids("ids")? {
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
    if ids.is_empty() {
        return Err(format!("в {} нечего резать", tc(at)));
    }
    for id in &ids {
        let c = ed.project.clip(*id).ok_or_else(|| format!("нет клипа c{id}"))?;
        if at <= c.start + EPS || at >= c.end() - EPS {
            return Err(format!("c{id} ({}–{}) не накрывает {}", tc(c.start), tc(c.end()), tc(at)));
        }
    }
    ed.begin("Разрезать");
    let parts: Vec<String> = ids.iter().filter_map(|id| split_clip(&mut ed.project, *id, at).map(|n| format!("c{id} | c{n}"))).collect();
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
    let ids = a.ids("ids")?.unwrap_or_else(|| ed.selection.clone());
    if ids.is_empty() {
        return Err("нечего удалять: выделите клипы".into());
    }
    let ripple = a.bool("ripple")?.unwrap_or(false);
    for id in &ids {
        ed.project.find_clip(*id).ok_or_else(|| format!("нет клипа c{id}"))?;
    }
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
    }
    ed.selection.retain(|s| !ids.contains(s));
    Ok(Out::text(format!("Удалено клипов: {}{}", ids.len(), if ripple { " (со сдвигом)" } else { "" })))
}

static CLIP_SET: Command = Command {
    id: "clip.set",
    category: "Таймлайн",
    title: "Свойства клипа",
    shortcut: "",
    keywords: "clip speed volume скорость громкость текст имя",
    help: "Скорость (speed: 2 — вдвое быстрее, длина клипа меняется), громкость (volume: 0–4), имя, текст и стиль титра.",
    args: &[req("id", Ty::Id, "клип (c…)"), arg("speed", Ty::Num, "скорость"), arg("volume", Ty::Num, "громкость"), arg("name", Ty::Str, "имя"), arg("text", Ty::Str, "текст титра"), arg("style", Ty::Object, "стиль титра (поля сливаются)")],
    danger: false,
    run: clip_set,
};

fn clip_set(ed: &mut Editor, a: &Args) -> Res {
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
    let (text, style) = (a.str("text")?, a.object("style")?);
    if (text.is_some() || style.is_some()) && !matches!(c.source, Source::Title { .. }) {
        return Err(format!("c{id} не титр"));
    }
    let new_dur = speed.map(|s| c.duration * c.speed / s);
    if let Some(d) = new_dur {
        let (ti, _) = ed.project.find_clip(id).unwrap();
        if ed.project.tracks[ti].clips.iter().any(|o| o.id != id && o.start < c.start + d - EPS && o.end() > c.start + EPS) {
            return Err(format!("c{id}: при скорости {} клип налезет на соседний (длина {})", speed.unwrap(), tc(d)));
        }
    }
    ed.begin("Свойства клипа");
    let fps = ed.project.settings.fps;
    let cm = ed.project.clip_mut(id).unwrap();
    if let (Some(s), Some(d)) = (speed, new_dur) {
        cm.speed = s;
        cm.duration = ((d * fps).round() / fps).max(1.0 / fps);
    }
    if let Some(v) = volume {
        cm.volume = v;
    }
    if let Some(n) = a.str("name")? {
        cm.name = n;
    }
    if let Source::Title { text: t, style: st } = &mut cm.source {
        if let Some(x) = text {
            *t = x;
        }
        if let Some(s) = style {
            st.extend(s);
        }
    }
    Ok(Out::text(format!("c{id} {}–{} скорость {} громкость {}", tc(cm.start), tc(cm.end()), cm.speed, cm.volume)))
}

// ── эффекты ─────────────────────────────────────────────────────────────

/// Известные эффекты: вид и описание параметров (для модели и инспектора).
pub const EFFECTS: &[(&str, &str)] = &[
    ("fade_in", "проявление из чёрного: duration (с)"),
    ("fade_out", "затухание в чёрный: duration (с)"),
    ("audio_fade_in", "нарастание звука: duration (с)"),
    ("audio_fade_out", "затухание звука: duration (с)"),
    ("lut", "цветокоррекция LUT: path (.cube), strength 0–1"),
    ("color", "яркость/контраст/насыщенность/температура: brightness, contrast, saturation, temperature (−1…1)"),
    ("blur", "размытие: radius (пикс.)"),
    ("sharpen", "резкость: amount 0–2"),
    ("vignette", "виньетка: amount 0–1"),
    ("transform", "положение и масштаб: x, y (доли кадра), scale, rotation (°), opacity 0–1"),
    ("upscale", "нейроувеличение synaptix-vfx: model (plksr|span|compact|nanovsr), scale 2|4"),
    ("interpolate", "нейроинтерполяция RIFE до частоты проекта (плавность, замедление)"),
];

static EFFECT_ADD: Command = Command {
    id: "effect.add",
    category: "Эффекты",
    title: "Добавить эффект",
    shortcut: "",
    keywords: "effect фильтр эффект lut цвет размытие затухание fade upscale rife увеличение плавность",
    help: "Добавить эффект клипам. Виды: fade_in, fade_out, audio_fade_in, audio_fade_out, lut {path, strength}, color {brightness, contrast, saturation, temperature}, blur {radius}, sharpen {amount}, vignette {amount}, transform {x, y, scale, rotation, opacity}, upscale {model, scale} (нейроувеличение), interpolate (RIFE до fps проекта).",
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
    ed.begin("Добавить эффект");
    let mut made = Vec::new();
    for id in &ids {
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
    keywords: "transition переход наплыв dissolve сдвиг шторка зум склейка",
    help: "Переход в начале клипа от предыдущего на той же дорожке (клипы должны стоять встык; переход по центру склейки). kind: dissolve (наплыв), dip_black (через чёрное), slide_left, slide_right, slide_up, slide_down, wipe_left, wipe_right, zoom, blur. duration — с (0.6).",
    args: &[req("id", Ty::Id, "клип (c…), к началу которого переход"), req("kind", Ty::Str, "вид перехода"), arg("duration", Ty::Time, "длительность, с")],
    danger: false,
    run: transition_set,
};

fn transition_set(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let kind = a.need_str("kind")?;
    if !crate::render::compose::TRANSITIONS.iter().any(|(k, _)| *k == kind) {
        return Err(format!("нет перехода «{kind}»; есть: {}", crate::render::compose::TRANSITIONS.iter().map(|t| t.0).collect::<Vec<_>>().join(", ")));
    }
    let (ti, ci) = ed.project.find_clip(id).ok_or_else(|| format!("нет клипа c{id}"))?;
    let c = &ed.project.tracks[ti].clips[ci];
    let prev = ci.checked_sub(1).map(|j| &ed.project.tracks[ti].clips[j]);
    let Some(prev) = prev.filter(|p| (p.end() - c.start).abs() < 1e-3) else {
        return Err(format!("c{id}: перед ним на дорожке нет клипа встык — переходу не из чего"));
    };
    let max = prev.duration.min(c.duration);
    let d = a.time("duration")?.unwrap_or(0.6).clamp(1.0 / ed.project.settings.fps, max);
    ed.begin("Переход");
    let fps = ed.project.settings.fps;
    ed.project.tracks[ti].clips[ci].transition = Some(crate::doc::Transition { kind: kind.clone(), duration: (d * fps).round() / fps });
    Ok(Out::text(format!("c{id}: переход {kind} {:.2} с", d)))
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
    let ids = a.ids("ids")?.unwrap_or_else(|| ed.selection.clone());
    if ids.is_empty() {
        return Err("нечего сдвигать: выделите клипы".into());
    }
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
    let ids = a.ids("ids")?.unwrap_or_else(|| ed.selection.clone());
    if ids.is_empty() {
        return Err("нечего копировать: выделите клипы".into());
    }
    let mut buf = Vec::new();
    for id in &ids {
        let (ti, _) = ed.project.find_clip(*id).ok_or_else(|| format!("нет клипа c{id}"))?;
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
    // по дорожкам: клипы одной дорожки едут вместе
    let mut tracks: Vec<Id> = buf.iter().map(|(t, _)| *t).collect();
    tracks.dedup();
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
            c.start = at + c.start - base;
            c.id = ed.project.alloc();
            c.renew_effect_ids(|| ed.project.alloc());
            new_ids.push(c.id);
            place(&mut ed.project, ti, c, false);
        }
    }
    ed.selection = new_ids.clone();
    let list: Vec<String> = new_ids.iter().map(|i| format!("c{i}")).collect();
    Ok(Out::text(format!("Вставлено в {}: {}", tc(at), list.join(", "))))
}

static CLIP_DETACH_AUDIO: Command = Command {
    id: "clip.detach_audio",
    category: "Таймлайн",
    title: "Отделить звук",
    shortcut: "",
    keywords: "detach audio отделить звук аудио разделить дорожка",
    help: "Вынести звук видеоклипа отдельным клипом на звуковую дорожку (та же обрезка, скорость и громкость); у видеоклипа звук выключается (громкость 0). Без ids — выделенные.",
    args: &[arg("ids", Ty::Ids, "видеоклипы (c…), по умолчанию выделенные")],
    danger: false,
    run: clip_detach_audio,
};

fn clip_detach_audio(ed: &mut Editor, a: &Args) -> Res {
    let ids = a.ids("ids")?.unwrap_or_else(|| ed.selection.clone());
    if ids.is_empty() {
        return Err("выделите видеоклип".into());
    }
    for id in &ids {
        let c = ed.project.clip(*id).ok_or_else(|| format!("нет клипа c{id}"))?;
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
    for id in &ids {
        let c = ed.project.clip(*id).unwrap().clone();
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
        // звуковые эффекты едут со звуком, видео — остаются
        au.effects = c.effects.iter().filter(|e| e.kind.starts_with("audio_")).cloned().collect();
        au.renew_effect_ids(|| ed.project.alloc());
        let nid = au.id;
        let tid = ed.project.tracks[ti].id;
        place(&mut ed.project, ti, au, false);
        let v = ed.project.clip_mut(*id).unwrap();
        v.volume = 0.0;
        v.effects.retain(|e| !e.kind.starts_with("audio_"));
        out.push(format!("c{id} → c{nid} (t{tid})"));
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
