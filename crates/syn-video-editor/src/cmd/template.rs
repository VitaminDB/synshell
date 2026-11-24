//! Шаблоны из библиотеки (Fusion `.setting` из DaVinci Resolve): поиск и
//! наложение титра с текстами, шрифтами и цветами шаблона.

use serde_json::{json, Map, Value};

use super::{arg, req, Args, Command, Out, Res, Ty};
use crate::doc::{tc, Clip, Source, TrackKind};
use crate::editor::Editor;
use crate::fusion;

pub static COMMANDS: &[&Command] = &[&LIST, &APPLY];

static LIST: Command = Command {
    id: "template.list",
    category: "Шаблоны",
    title: "Найти шаблон",
    shortcut: "",
    keywords: "template шаблон титр заголовок нижняя треть fusion davinci поиск",
    help: "Поиск шаблонов в библиотеке (Fusion .setting из DaVinci Resolve): query — слова в имени или разделе (Заголовки, Нижние трети, Соцсети, YouTube, Переходы…). Показывает имя, раздел и тексты шаблона.",
    args: &[arg("query", Ty::Str, "что искать"), arg("limit", Ty::Num, "сколько показать (20)")],
    danger: false,
    run: list,
};

fn list(_ed: &mut Editor, a: &Args) -> Res {
    let root = fusion::library();
    let all = fusion::scan(&root);
    if all.is_empty() {
        return Err(format!("шаблонов нет в {}", root.display()));
    }
    let q = a.str("query")?.unwrap_or_default().to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    let limit = a.num("limit")?.unwrap_or(20.0) as usize;
    let hits: Vec<_> = all.iter().filter(|(cat, name, _)| {
        let hay = format!("{cat} {name}").to_lowercase().replace('_', " ");
        words.iter().all(|w| hay.contains(w))
    }).collect();
    let mut lines = vec![format!("Найдено {} из {} шаблонов", hits.len(), all.len())];
    for (cat, name, path) in hits.iter().take(limit) {
        let texts = std::fs::read_to_string(path).ok().and_then(|t| fusion::parse(&t).ok()).map(|r| fusion::text_layers(&r).into_iter().map(|l| l.text).collect::<Vec<_>>().join(" / ")).unwrap_or_default();
        lines.push(format!("{name} — {cat}{}", if texts.is_empty() { String::new() } else { format!(" — тексты: {texts}") }));
    }
    Ok(Out::with(lines.join("\n"), json!(hits.iter().take(limit).map(|(c, n, _)| json!({"name": n, "category": c})).collect::<Vec<_>>())))
}

static APPLY: Command = Command {
    id: "template.apply",
    category: "Шаблоны",
    title: "Наложить шаблон",
    shortcut: "",
    keywords: "template apply шаблон титр наложить",
    help: "Титр из шаблона библиотеки: тексты, шрифт, размер и цвет берутся из шаблона, text — свои строки вместо текстов шаблона (через \\n). Анимация шаблона пока приближённая (появление pop), точное исполнение графа Fusion — позже.",
    args: &[req("name", Ty::Str, "имя шаблона (template.list)"), arg("text", Ty::Str, "свой текст (строки через \\n)"), arg("at", Ty::Time, "начало (позиция воспроизведения)"), arg("duration", Ty::Time, "длительность (4 с)"), arg("track", Ty::Id, "видеодорожка")],
    danger: false,
    run: apply,
};

fn apply(ed: &mut Editor, a: &Args) -> Res {
    let name = a.need_str("name")?;
    let all = fusion::scan(&fusion::library());
    let n = name.to_lowercase();
    let (_, tname, path) = all.iter().find(|(_, nm, _)| nm.to_lowercase() == n).or_else(|| all.iter().find(|(_, nm, _)| nm.to_lowercase().contains(&n))).ok_or_else(|| format!("шаблона «{name}» нет — найдите имя через template.list"))?.clone();
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let root = fusion::parse(&text).map_err(|e| format!("{tname}: {e}"))?;
    let layers = fusion::text_layers(&root);
    let main = layers.iter().max_by(|a, b| a.size.total_cmp(&b.size));
    let body = match a.str("text")? {
        Some(t) => t.replace("\\n", "\n"),
        None if !layers.is_empty() => layers.iter().map(|l| l.text.clone()).collect::<Vec<_>>().join("\n"),
        None => tname.replace('_', " "),
    };
    let mut style = Map::new();
    style.insert("template".into(), Value::String(path.display().to_string()));
    style.insert("in".into(), "pop".into());
    style.insert("out".into(), "fade".into());
    if let Some(l) = main {
        let h = ed.project.settings.height.min(ed.project.settings.width) as f64;
        style.insert("size".into(), json!((l.size * h * 1.2).round().clamp(12.0, 400.0)));
        style.insert("color".into(), Value::String(l.color.clone()));
        if !l.font.is_empty() {
            style.insert("font".into(), Value::String(l.font.clone()));
        }
        // центр Fusion: y = 0 внизу, 1 вверху
        let pos = if l.center.1 > 0.62 { "top" } else if l.center.1 < 0.38 { "bottom" } else { "center" };
        style.insert("position".into(), Value::String(pos.into()));
    }
    let at = a.time("at")?.unwrap_or(ed.playhead);
    let duration = a.time("duration")?.unwrap_or(4.0);
    let track = a.id("track")?;
    ed.begin("Шаблон");
    let p = &mut ed.project;
    let ti = match track {
        Some(t) => p.tracks.iter().position(|x| x.id == t && x.kind == TrackKind::Video).ok_or_else(|| format!("нет видеодорожки t{t}"))?,
        None => match p.tracks.iter().position(|t| t.kind == TrackKind::Video && t.name == "Титры") {
            Some(i) => i,
            None => {
                let id = p.alloc();
                p.tracks.push(crate::doc::Track { id, kind: TrackKind::Video, name: "Титры".into(), muted: false, hidden: false, clips: Vec::new() });
                p.tracks.len() - 1
            }
        },
    };
    let id = p.alloc();
    let start = p.snap(at);
    let dur = p.snap(duration);
    // освободить место (перезапись)
    let t = &mut p.tracks[ti];
    t.clips.retain(|c| c.end() <= start + 1e-6 || c.start >= start + dur - 1e-6);
    t.clips.push(Clip { id, source: Source::Title { text: body.clone(), style }, start, duration: dur, offset: 0.0, speed: 1.0, volume: 1.0, name: tname.clone(), effects: Vec::new(), transition: None });
    t.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(Out::with(format!("c{id} шаблон {tname}: «{}» {}–{}", body.replace('\n', " / "), tc(start), tc(start + dur)), json!({"id": id})))
}
