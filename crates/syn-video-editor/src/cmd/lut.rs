//! LUT из библиотеки (C7): поиск и наложение на клипы.

use serde_json::{json, Map, Value};

use super::{arg, Args, Command, Out, Res, Ty};
use crate::doc::{Effect, TrackKind};
use crate::editor::Editor;
use crate::render::lut;

pub static COMMANDS: &[&Command] = &[&LIST, &APPLY];

static LIST: Command = Command {
    id: "lut.list",
    category: "Цвет",
    title: "Найти LUT",
    shortcut: "",
    keywords: "lut цвет цветокоррекция грейдинг look cube кино винтаж log rec709 поиск",
    help: "Поиск LUT (.cube) в библиотеке «LUT»: темы — Кино, Винтаж и ретро, Город, Природа и путешествия, Свадьба и романтика, Блог и соцсети, Чёрно-белые, Температура, Конвертация Log в Rec.709 (по камерам: Sony, Canon, DJI…). query — слова в теме, наборе или имени; theme — тема точно. Показывает «тема / набор / имя» и путь.",
    args: &[arg("query", Ty::Str, "что искать"), arg("theme", Ty::Str, "тема (папка верхнего уровня)"), arg("limit", Ty::Num, "сколько показать (20)")],
    danger: false,
    run: list,
};

fn list(_ed: &mut Editor, a: &Args) -> Res {
    let all = lut::catalog();
    if all.is_empty() {
        return Err(format!("LUT нет в {}", lut::library_dir().display()));
    }
    let q = a.str("query")?.unwrap_or_default();
    let theme = a.str("theme")?.unwrap_or_default();
    let limit = a.num("limit")?.unwrap_or(20.0).max(1.0) as usize;
    let hits = lut::filter(all, &q, &theme, "");
    let mut themes: Vec<&str> = all.iter().map(|e| e.theme.as_str()).collect();
    themes.dedup();
    let mut lines = vec![format!("Найдено {} из {} LUT; темы: {}", hits.len(), all.len(), themes.join(", "))];
    for e in hits.iter().take(limit) {
        lines.push(format!("{} / {} / {} — {}", e.theme, e.set, e.name, e.path.display()));
    }
    let data: Vec<Value> = hits.iter().take(limit).map(|e| json!({"theme": e.theme, "set": e.set, "name": e.name, "path": e.path.display().to_string()})).collect();
    Ok(Out::with(lines.join("\n"), json!(data)))
}

static APPLY: Command = Command {
    id: "lut.apply",
    category: "Цвет",
    title: "Наложить LUT",
    shortcut: "",
    keywords: "lut цвет цветокоррекция грейдинг look наложить применить",
    help: "Наложить LUT на клипы (ids, по умолчанию выделенные): path — файл .cube или name — слова для поиска в библиотеке (берётся первый найденный). Если у клипа уже есть LUT — он заменяется (сила сохраняется, если не задана strength 0–1). Связанный звук пропускается. Один шаг отмены.",
    args: &[arg("ids", Ty::Ids, "клипы (c…), по умолчанию выделенные"), arg("path", Ty::Str, "файл .cube"), arg("name", Ty::Str, "поиск по библиотеке"), arg("strength", Ty::Num, "сила 0–1 (1)")],
    danger: false,
    run: apply,
};

fn apply(ed: &mut Editor, a: &Args) -> Res {
    let path = match (a.str("path")?.filter(|s| !s.trim().is_empty()), a.str("name")?.filter(|s| !s.trim().is_empty())) {
        (Some(p), _) => crate::doc::expand(&p),
        (None, Some(n)) => {
            // точное имя → имя начинается с запроса → набор → любые слова
            let hits = lut::filter(lut::catalog(), &n, "", "");
            let k = n.trim().to_lowercase().replace(['_', '-'], " ");
            let norm = |s: &str| s.to_lowercase().replace(['_', '-'], " ");
            hits.iter()
                .find(|e| norm(&e.name) == k)
                .or_else(|| hits.iter().find(|e| norm(&e.name).starts_with(&k)))
                .or_else(|| hits.iter().find(|e| norm(&e.set) == k))
                .or(hits.first())
                .map(|e| e.path.clone())
                .ok_or_else(|| format!("LUT «{n}» не найден — lut.list"))?
        }
        (None, None) => return Err("lut.apply: нужен path или name".into()),
    };
    lut::load(&path.display().to_string()).map_err(|e| format!("{}: {e}", path.display()))?;
    let strength = a.num("strength")?;
    if strength.is_some_and(|s| !(0.0..=1.0).contains(&s)) {
        return Err("strength вне 0–1".into());
    }
    let ids = a.ids("ids")?.unwrap_or_else(|| ed.selection.clone());
    let mut targets = Vec::new();
    for id in &ids {
        let (ti, _) = ed.project.find_clip(*id).ok_or_else(|| format!("нет клипа c{id}"))?;
        if ed.project.tracks[ti].kind == TrackKind::Video && !targets.contains(id) {
            targets.push(*id);
        }
    }
    if targets.is_empty() {
        return Err("нет видеоклипов: выделите клип или укажите ids".into());
    }
    ed.begin("LUT");
    let ps = path.display().to_string();
    let mut made = Vec::new();
    for id in targets {
        let eid = ed.project.alloc();
        let c = ed.project.clip_mut(id).unwrap();
        match c.effects.iter_mut().find(|e| e.kind == "lut") {
            Some(e) => {
                e.params.insert("path".into(), Value::String(ps.clone()));
                if let Some(s) = strength {
                    e.params.insert("strength".into(), json!(s));
                }
                made.push(format!("c{id}: e{} заменён", e.id));
            }
            None => {
                let mut params = Map::new();
                params.insert("path".into(), Value::String(ps.clone()));
                params.insert("strength".into(), json!(strength.unwrap_or(1.0)));
                c.effects.push(Effect { id: eid, kind: "lut".into(), params });
                made.push(format!("c{id}: e{eid}"));
            }
        }
    }
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Out::text(format!("LUT {name}: {}", made.join(", "))))
}
