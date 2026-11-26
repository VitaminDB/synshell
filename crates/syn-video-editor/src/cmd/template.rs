//! Шаблоны из библиотеки (Fusion `.setting` из DaVinci Resolve): поиск и
//! наложение титра с текстами, шрифтами и цветами шаблона.

use serde_json::{json, Map, Value};

use super::{arg, req, Args, Command, Out, Res, Ty};
use crate::doc::{tc, Clip, Source, TrackKind};
use crate::editor::Editor;
use crate::fusion;

pub static COMMANDS: &[&Command] = &[&LIST, &INSPECT, &COVERAGE, &APPLY, &PARAMS, &SET, &FONTS];

static LIST: Command = Command {
    id: "template.list",
    category: "Шаблоны",
    title: "Найти шаблон",
    shortcut: "",
    keywords: "template шаблон титр заголовок нижняя треть fusion davinci поиск lottie анимация стикер",
    help: "Поиск шаблонов в библиотеке (Fusion .setting из DaVinci Resolve и анимации Lottie из «Анимации Lottie» — раздел Lottie): query — слова в имени или разделе (Заголовки, Нижние трети, Соцсети, YouTube, Переходы…). Показывает имя, раздел и тексты шаблона.",
    args: &[arg("query", Ty::Str, "что искать"), arg("limit", Ty::Num, "сколько показать (20)")],
    danger: false,
    run: list,
};

fn list(_ed: &mut Editor, a: &Args) -> Res {
    let root = fusion::library();
    let mut all = fusion::scan(&root);
    all.extend(crate::lottie::scan());
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
        if crate::lottie::is_lottie_ext(path) {
            let info = crate::lottie::probe(path).map(|(w, h, fps, d)| format!("анимация {w}×{h}, {d:.1} с, {fps:.0} fps")).unwrap_or_else(|e| e);
            lines.push(format!("{name} — {cat} — {info}"));
            continue;
        }
        let texts = std::fs::read_to_string(path).ok().and_then(|t| fusion::parse(&t).ok()).map(|r| fusion::text_layers(&r).into_iter().map(|l| l.text).collect::<Vec<_>>().join(" / ")).unwrap_or_default();
        lines.push(format!("{name} — {cat}{}", if texts.is_empty() { String::new() } else { format!(" — тексты: {texts}") }));
    }
    Ok(Out::with(lines.join("\n"), json!(hits.iter().take(limit).map(|(c, n, _)| json!({"name": n, "category": c})).collect::<Vec<_>>())))
}

static INSPECT: Command = Command {
    id: "template.inspect",
    category: "Шаблоны",
    title: "Состав шаблона Fusion",
    shortcut: "",
    keywords: "template inspect fusion граф инструменты ноды параметры шаблон",
    help: "Разобрать шаблон Fusion (.setting): текстовые слои, список инструментов графа, неподдерживаемые ноды (если есть), исходная длительность и вынесенные входы MacroOperator.",
    args: &[req("name", Ty::Str, "имя шаблона (template.list)")],
    danger: false,
    run: inspect,
};

fn inspect(_ed: &mut Editor, a: &Args) -> Res {
    let name = a.need_str("name")?;
    let all = fusion::scan(&fusion::library());
    let n = name.to_lowercase();
    let (cat, tname, path) = all
        .iter()
        .find(|(_, nm, _)| nm.to_lowercase() == n)
        .or_else(|| all.iter().find(|(_, nm, _)| nm.to_lowercase().contains(&n)))
        .ok_or_else(|| format!("шаблона «{name}» нет — найдите имя через template.list"))?
        .clone();
    let comp = fusion::load_cached(&path)?;
    let root = fusion::parse(&std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?)?;
    let texts = fusion::text_layers(&root);
    let types = comp.tool_types();
    let unsup = comp.unsupported_tools();
    let params = comp.all_template_params();
    let mut lines = vec![format!("{tname} ({cat}) — узлов: {}, кадров: {:.0}, параметров MacroOperator: {}", comp.tools.len(), comp.source_frames, params.len())];
    if !texts.is_empty() {
        let ts: Vec<String> = texts.iter().map(|l| format!("{} = «{}»", l.name, l.text)).collect();
        lines.push(format!("Тексты: {}", ts.join("; ")));
    }
    if !params.is_empty() {
        lines.push("Параметры:".into());
        for p in &params {
            lines.push(format!("  {} [{}] — {}: {} (по умолч. {})", p.key, p.kind_name(), p.group, p.label, p.default_json()));
        }
    }
    lines.push(format!("Инструменты: {}", types.join(", ")));
    if !unsup.is_empty() {
        lines.push(format!("Неподдерживаемые: {}", unsup.join(", ")));
    }
    Ok(Out::with(
        lines.join("\n"),
        json!({
            "name": tname,
            "category": cat,
            "nodes": comp.tools.len(),
            "source_frames": comp.source_frames,
            "texts": texts.iter().map(|l| json!({"name": l.name, "text": l.text, "font": l.font, "size": l.size, "color": l.color})).collect::<Vec<_>>(),
            "tools": types,
            "unsupported": unsup,
            "exposed": comp.exposed.len(),
            "params": params.iter().map(|p| json!({
                "key": p.key,
                "group": p.group,
                "label": p.label,
                "kind": p.kind_name(),
                "default": p.default_json(),
            })).collect::<Vec<_>>(),
        }),
    ))
}

static COVERAGE: Command = Command {
    id: "template.coverage",
    category: "Шаблоны",
    title: "Покрытие шаблонов Fusion",
    shortcut: "",
    keywords: "template coverage report отчёт покрытие библиотека fusion статистика инструменты",
    help: "Отчёт покрытия библиотеки шаблонов Fusion (.setting): сколько разобрано, сколько полностью поддерживаются по разделам и рейтинг недостающих инструментов (с числом шаблонов, которые станут 100% после добавления инструмента).",
    args: &[arg("query", Ty::Str, "фильтр по разделу или имени"), arg("top", Ty::Num, "сколько недостающих инструментов показать (20)")],
    danger: false,
    run: coverage,
};

fn coverage(_ed: &mut Editor, a: &Args) -> Res {
    let root = fusion::library();
    let q = a.str("query")?;
    let top = a.num("top")?.unwrap_or(20.0).max(0.0) as usize;
    let rep = fusion::report::analyze(&root, q.as_deref());
    if rep.total == 0 {
        return Err(format!("шаблонов нет в {}", root.display()));
    }
    Ok(Out::with(rep.format(top), rep.to_json(top)))
}

static APPLY: Command = Command {
    id: "template.apply",
    category: "Шаблоны",
    title: "Наложить шаблон",
    shortcut: "",
    keywords: "template apply шаблон титр наложить fusion",
    help: "Наложить шаблон из библиотеки. Анимация Lottie (раздел Lottie, .json/.lottie) встаёт клипом медиа поверх (импорт в медиатеку сам; duration по умолчанию — длина анимации, длиннее — по кругу; положение и размер — эффект transform). Шаблон Fusion (исполняется полный граф: Transform, Merge, Background, градиенты, маски, TextPlus, BezierSpline, KeyStretcher, Dissolve, Blur, Glow, Displace, Duplicate). Можно указать name или точный path к .setting; text — свои строки вместо текстов шаблона (через \\n).",
    args: &[
        req("name", Ty::Str, "имя шаблона (template.list)"),
        arg("path", Ty::Str, "точный путь к .setting (необязательно)"),
        arg("text", Ty::Str, "свой текст (строки через \\n)"),
        arg("at", Ty::Time, "начало (позиция воспроизведения)"),
        arg("duration", Ty::Time, "длительность (4 с)"),
        arg("track", Ty::Id, "видеодорожка"),
    ],
    danger: false,
    run: apply,
};

fn apply(ed: &mut Editor, a: &Args) -> Res {
    let name = a.need_str("name")?;
    let (tname, path) = if let Some(p) = a.str("path")?.filter(|s| !s.trim().is_empty()) {
        let pb = std::path::PathBuf::from(&p);
        if !pb.is_file() {
            return Err(format!("файл шаблона не найден: {}", pb.display()));
        }
        let stem = pb.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| name.clone());
        (stem, pb)
    } else {
        let mut all = fusion::scan(&fusion::library());
        all.extend(crate::lottie::scan());
        let n = name.to_lowercase();
        let (_, tname, path) = all
            .iter()
            .find(|(_, nm, _)| nm.to_lowercase() == n)
            .or_else(|| all.iter().find(|(_, nm, _)| nm.to_lowercase().contains(&n)))
            .ok_or_else(|| format!("шаблона «{name}» нет — найдите имя через template.list"))?
            .clone();
        (tname, path)
    };
    if crate::lottie::is_lottie_ext(&path) {
        return apply_lottie(ed, a, &tname, &path);
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let root = fusion::parse(&text).map_err(|e| format!("{tname}: {e}"))?;
    let layers = fusion::text_layers(&root);
    let main = layers.iter().max_by(|a, b| a.size.total_cmp(&b.size));
    let body = match a.str("text")?.map(|t| t.replace("\\n", "\n")) {
        Some(t) => t.replace("\\n", "\n"),
        None if !layers.is_empty() => layers.iter().map(|l| l.text.clone()).collect::<Vec<_>>().join("\n"),
        None => tname.replace('_', " "),
    };
    let mut style = Map::new();
    style.insert("template".into(), Value::String(path.display().to_string()));
    style.insert("in".into(), "none".into());
    style.insert("out".into(), "none".into());
    if let Some(l) = main {
        let h = ed.project.settings.height.min(ed.project.settings.width) as f64;
        style.insert("size".into(), json!((l.size * h * 1.2).round().clamp(12.0, 400.0)));
        style.insert("color".into(), Value::String(l.color.clone()));
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
    t.clips.push(Clip { id, source: Source::Title { text: body.clone(), style }, start, duration: dur, offset: 0.0, speed: 1.0, volume: 1.0, name: tname.clone(), effects: Vec::new(), transition: None, keys: Default::default(), blend: String::new(), parent: None, link: None, graph: None, fit: String::new(), slot: None });
    t.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(Out::with(format!("c{id} шаблон {tname}: «{}» {}–{}", body.replace('\n', " / "), tc(start), tc(start + dur)), json!({"id": id})))
}

/// Анимация Lottie поверх: в медиатеку (если её там нет) и клипом на
/// верхнюю видеодорожку «Графика» (перезапись места) или на `track`.
fn apply_lottie(ed: &mut Editor, a: &Args, tname: &str, path: &std::path::Path) -> Res {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let probed = match ed.project.media.iter().find(|m| m.path == path) {
        Some(_) => None,
        None => Some(super::media::probe(&path)?),
    };
    let anim = match &probed {
        Some(m) => m.duration,
        None => ed.project.media.iter().find(|m| m.path == path).map(|m| m.duration).unwrap_or(3.0),
    };
    let at = a.time("at")?.unwrap_or(ed.playhead);
    let duration = a.time("duration")?.unwrap_or(anim).max(0.1);
    let track = a.id("track")?;
    ed.begin("Шаблон");
    let p = &mut ed.project;
    let mid = match probed {
        Some(mut m) => {
            m.id = p.alloc();
            let id = m.id;
            p.media.push(m);
            id
        }
        None => p.media.iter().find(|m| m.path == path).map(|m| m.id).unwrap_or_default(),
    };
    let ti = match track {
        Some(t) => p.tracks.iter().position(|x| x.id == t && x.kind == TrackKind::Video).ok_or_else(|| format!("нет видеодорожки t{t}"))?,
        // свободная дорожка «Графика…» на это время, иначе новая поверх — чужое не затираем
        None => {
            let (s0, e0) = (p.snap(at), p.snap(at) + p.snap(duration));
            match p.tracks.iter().position(|t| t.kind == TrackKind::Video && t.name.starts_with("Графика") && t.clips.iter().all(|c| c.end() <= s0 + 1e-6 || c.start >= e0 - 1e-6)) {
                Some(i) => i,
                None => {
                    let n = p.tracks.iter().filter(|t| t.name.starts_with("Графика")).count();
                    let id = p.alloc();
                    let name = if n == 0 { "Графика".to_string() } else { format!("Графика {}", n + 1) };
                    p.tracks.push(crate::doc::Track { id, kind: TrackKind::Video, name, muted: false, hidden: false, clips: Vec::new() });
                    p.tracks.len() - 1
                }
            }
        }
    };
    let id = p.alloc();
    let start = p.snap(at);
    let dur = p.snap(duration).max(1.0 / p.settings.fps.max(1.0));
    let t = &mut p.tracks[ti];
    t.clips.retain(|c| c.end() <= start + 1e-6 || c.start >= start + dur - 1e-6);
    t.clips.push(Clip { id, source: Source::Media { media: mid }, start, duration: dur, offset: 0.0, speed: 1.0, volume: 1.0, name: tname.to_string(), effects: Vec::new(), transition: None, keys: Default::default(), blend: String::new(), parent: None, link: None, graph: None, fit: String::new(), slot: None });
    t.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(Out::with(format!("c{id} Lottie {tname} (m{mid}) {}–{}", tc(start), tc(start + dur)), json!({"id": id, "media": mid})))
}

static PARAMS: Command = Command {
    id: "template.params",
    category: "Шаблоны",
    title: "Параметры шаблона клипа",
    shortcut: "",
    keywords: "template params параметры шаблон macrooperator входы fusion",
    help: "Список вынесенных параметров (MacroOperator.Inputs) наложенного шаблона Fusion у клипа id: ключ, группа, подпись, тип, текущее значение и значение по умолчанию. Править через template.set или анимировать через key.set с param=style.p:<ключ>.",
    args: &[req("id", Ty::Id, "клип с шаблоном (c…)")],
    danger: false,
    run: params_cmd,
};

fn params_cmd(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let c = ed.project.clip(id).ok_or_else(|| format!("нет клипа c{id}"))?;
    let Source::Title { style, .. } = &c.source else {
        return Err(format!("c{id} не шаблон Fusion"));
    };
    let tpl_path = style.get("template").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| format!("у клипа c{id} нет шаблона Fusion"))?;
    let comp = fusion::load_cached(std::path::Path::new(tpl_path))?;
    let all = comp.visible_template_params(style);
    let mut lines = vec![format!("c{id} шаблон «{}» — параметров: {}", comp.name, all.len())];
    let mut items = Vec::with_capacity(all.len());
    for p in &all {
        let def = p.default_json();
        let cur = match &p.kind {
            fusion::ParamKind::Point { default, .. } => {
                let x = style.get(&p.style_key_sub("x")).and_then(Value::as_f64).unwrap_or(default.0);
                let y = style.get(&p.style_key_sub("y")).and_then(Value::as_f64).unwrap_or(default.1);
                json!([(x * 10_000.0).round() / 10_000.0, (y * 10_000.0).round() / 10_000.0])
            }
            _ => style.get(&p.style_key()).cloned().unwrap_or_else(|| def.clone()),
        };
        let modified = cur != def;
        let mark = if modified { " *" } else { "" };
        lines.push(format!("  {} [{}] ({} / {}) = {}{mark}", p.key, p.kind_name(), p.group, p.label, cur));
        items.push(json!({
            "key": p.key,
            "group": p.group,
            "label": p.label,
            "kind": p.kind_name(),
            "value": cur,
            "default": def,
            "modified": modified,
        }));
    }
    Ok(Out::with(lines.join("\n"), json!({"id": id, "template": comp.name, "params": items})))
}

static SET: Command = Command {
    id: "template.set",
    category: "Шаблоны",
    title: "Параметр шаблона",
    shortcut: "",
    keywords: "template set параметр шаблон fusion macrooperator цвет текст размер сброс",
    help: "Изменить параметры (входы MacroOperator) шаблона Fusion у клипа id: либо param + value (для точки: param=Master.Center с value=[x,y] или param=Master.Center.x с числом; value=null — сбросить параметр), либо params: {ключ: значение}, либо reset: true (сбросить все параметры к значениям шаблона).",
    args: &[
        req("id", Ty::Id, "клип с шаблоном (c…)"),
        arg("param", Ty::Str, "ключ параметра из template.params"),
        arg("value", Ty::Any, "значение (строка, число, #цвет, bool, [x,y] или null для сброса)"),
        arg("params", Ty::Object, "несколько параметров сразу: {ключ: значение}"),
        arg("reset", Ty::Bool, "сбросить все параметры шаблона (true)"),
    ],
    danger: false,
    run: set_cmd,
};

fn set_cmd(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let c = ed.project.clip(id).ok_or_else(|| format!("нет клипа c{id}"))?.clone();
    let Source::Title { style, .. } = &c.source else {
        return Err(format!("c{id} не шаблон Fusion"));
    };
    let tpl_path = style.get("template").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| format!("у клипа c{id} нет шаблона Fusion"))?;
    let comp = fusion::load_cached(std::path::Path::new(tpl_path))?;
    let reset = a.bool("reset")?.unwrap_or(false);

    let mut pairs: Vec<(String, Value)> = Vec::new();
    if let Some(k) = a.str("param")? {
        let v = a.raw("value").cloned().unwrap_or(Value::Null);
        pairs.push((k, v));
    }
    if let Some(obj) = a.object("params")? {
        for (k, v) in obj {
            pairs.push((k, v));
        }
    }
    if !reset && pairs.is_empty() {
        return Err("укажите param + value, params или reset: true".into());
    }

    let known = comp.all_template_params();
    let mut updates: Vec<(String, Option<Value>)> = Vec::new();
    let mut summary: Vec<String> = Vec::new();

    for (raw_key, val) in pairs {
        let k = raw_key.strip_prefix(fusion::STYLE_PREFIX).unwrap_or(&raw_key);
        // Точечный подпараметр `.x` / `.y`
        if let Some(base) = k.strip_suffix(".x").or_else(|| k.strip_suffix(".y")) {
            let axis = if k.ends_with(".x") { "x" } else { "y" };
            if let Some(p) = known.iter().find(|p| p.key == base && matches!(p.kind, fusion::ParamKind::Point { .. })) {
                if val.is_null() {
                    updates.push((p.style_key_sub(axis), None));
                    summary.push(format!("{k}=default"));
                } else {
                    let n = val.as_f64().ok_or_else(|| format!("{k}: нужно число"))?;
                    updates.push((p.style_key_sub(axis), Some(json!(n))));
                    summary.push(format!("{k}={n}"));
                }
                continue;
            }
        }
        let Some(p) = known.iter().find(|p| p.key == k) else {
            // Допускаем прямой ключ `Tool.Input`, если такой узел есть в графе
            if let Some((op, _)) = k.split_once('.') {
                if comp.tools.contains_key(op) {
                    let sk = format!("{}{k}", fusion::STYLE_PREFIX);
                    if val.is_null() {
                        updates.push((sk, None));
                        summary.push(format!("{k}=default"));
                    } else {
                        summary.push(format!("{k}={val}"));
                        updates.push((sk, Some(val)));
                    }
                    continue;
                }
            }
            return Err(format!("у шаблона «{}» нет параметра «{k}» — посмотрите template.params", comp.name));
        };
        if val.is_null() {
            match p.kind {
                fusion::ParamKind::Point { .. } => {
                    updates.push((p.style_key_sub("x"), None));
                    updates.push((p.style_key_sub("y"), None));
                    updates.push((p.style_key(), None));
                }
                _ => updates.push((p.style_key(), None)),
            }
            summary.push(format!("{k}=default"));
            continue;
        }
        match &p.kind {
            fusion::ParamKind::Point { .. } => {
                let (x, y) = val
                    .as_array()
                    .and_then(|arr| Some((arr.first()?.as_f64()?, arr.get(1)?.as_f64()?)))
                    .or_else(|| {
                        let o = val.as_object()?;
                        Some((o.get("x")?.as_f64()?, o.get("y")?.as_f64()?))
                    })
                    .ok_or_else(|| format!("{k}: нужна точка [x, y] или {{\"x\": …, \"y\": …}}"))?;
                updates.push((p.style_key_sub("x"), Some(json!(x))));
                updates.push((p.style_key_sub("y"), Some(json!(y))));
                summary.push(format!("{k}=[{x}, {y}]"));
            }
            fusion::ParamKind::Number { .. } => {
                let n = val.as_f64().ok_or_else(|| format!("{k}: нужно число"))?;
                updates.push((p.style_key(), Some(json!(n))));
                summary.push(format!("{k}={n}"));
            }
            fusion::ParamKind::Bool { .. } => {
                let b = val.as_bool().or_else(|| val.as_f64().map(|n| n > 0.5)).ok_or_else(|| format!("{k}: нужно true/false"))?;
                updates.push((p.style_key(), Some(Value::Bool(b))));
                summary.push(format!("{k}={b}"));
            }
            fusion::ParamKind::Color { .. } | fusion::ParamKind::GradStop { .. } => {
                let s = val.as_str().ok_or_else(|| format!("{k}: нужен цвет #rrggbb"))?;
                if fusion::params::parse_hex_rgba(s).is_none() {
                    return Err(format!("{k}: цвет должен быть #rrggbb или #rrggbbaa, а не «{s}»"));
                }
                updates.push((p.style_key(), Some(Value::String(s.to_string()))));
                summary.push(format!("{k}={s}"));
            }
            fusion::ParamKind::Text { .. } | fusion::ParamKind::Font { .. } | fusion::ParamKind::Choice { .. } => {
                let s = val.as_str().ok_or_else(|| format!("{k}: нужна строка"))?;
                updates.push((p.style_key(), Some(Value::String(s.to_string()))));
                summary.push(format!("{k}=«{s}»"));
            }
        }
    }

    ed.begin("Параметры шаблона");
    if let Some(cm) = ed.project.clip_mut(id) {
        if let Source::Title { style, .. } = &mut cm.source {
            if reset {
                style.retain(|k, _| !k.starts_with(fusion::STYLE_PREFIX));
                cm.keys.retain(|k, _| !k.starts_with("style.p:"));
            }
            for (sk, ov) in updates {
                match ov {
                    Some(v) => {
                        style.insert(sk, v);
                    }
                    None => {
                        style.remove(&sk);
                    }
                }
            }
        }
    }
    if reset && summary.is_empty() {
        Ok(Out::text(format!("c{id}: параметры шаблона «{}» сброшены", comp.name)))
    } else {
        Ok(Out::text(format!("c{id} шаблон: {}", summary.join(", "))))
    }
}

static FONTS: Command = Command {
    id: "font.list",
    category: "Шаблоны",
    title: "Шрифты титров",
    shortcut: "",
    keywords: "font шрифт гарнитура титр текст",
    help: "Шрифты из папки «Шрифты» библиотеки: query — часть названия. Имя ставится в стиль титра: style {font: \"<имя>\"}. Системные шрифты тоже работают по имени.",
    args: &[arg("query", Ty::Str, "часть названия"), arg("limit", Ty::Num, "сколько показать (50)")],
    danger: false,
    run: fonts,
};

fn fonts(_ed: &mut Editor, a: &Args) -> Res {
    let q = a.str("query")?.unwrap_or_default().to_lowercase();
    let limit = a.num("limit")?.unwrap_or(50.0) as usize;
    let all = crate::fonts::families();
    if all.is_empty() {
        return Err(format!("шрифтов нет в {}", crate::fonts::dir().display()));
    }
    let hits: Vec<&String> = all.iter().filter(|f| f.to_lowercase().contains(&q)).collect();
    let shown: Vec<&String> = hits.iter().take(limit).copied().collect();
    Ok(Out::with(format!("Шрифтов: {} из {}\n{}", hits.len(), all.len(), shown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")), json!(shown)))
}
