use serde_json::json;

use super::{arg, req, Args, Command, Out, Res, Ty};
use crate::doc::{expand, Project};
use crate::editor::Editor;
use crate::store;

pub static COMMANDS: &[&Command] = &[&NEW, &OPEN, &SAVE, &SETTINGS, &DUMP, &EXPORT, &FRAME, &CHECK];

static NEW: Command = Command {
    id: "project.new",
    category: "Проект",
    title: "Новый проект",
    shortcut: "Ctrl+N",
    keywords: "new создать проект",
    help: "Создать пустой проект (дорожки V1 и A1). Несохранённые изменения текущего теряются.",
    args: &[
        arg("name", Ty::Str, "название"),
        arg("width", Ty::Num, "ширина кадра, пикс."),
        arg("height", Ty::Num, "высота кадра, пикс."),
        arg("fps", Ty::Num, "кадров в секунду"),
        arg("bg", Ty::Str, "цвет фона кадра (#rrggbb или transparent)"),
    ],
    danger: false,
    run: new,
};

fn new(ed: &mut Editor, a: &Args) -> Res {
    let mut p = Project::new(&a.str("name")?.unwrap_or_else(|| "Без названия".into()));
    apply_settings(&mut p, a)?;
    ed.replace(p, None);
    Ok(Out::text(format!("Создан проект «{}»", ed.project.name)))
}

static OPEN: Command = Command {
    id: "project.open",
    category: "Проект",
    title: "Открыть проект…",
    shortcut: "Ctrl+O",
    keywords: "open открыть загрузить",
    help: "Открыть проект из файла .syn.",
    args: &[req("path", Ty::Str, "путь к файлу проекта .syn")],
    danger: false,
    run: open,
};

fn open(ed: &mut Editor, a: &Args) -> Res {
    let path = expand(&a.need_str("path")?);
    let p = store::load(&path)?;
    ed.replace(p, Some(path.clone()));
    Ok(Out::text(format!("Открыт проект «{}» ({})", ed.project.name, path.display())))
}

static SAVE: Command = Command {
    id: "project.save",
    category: "Проект",
    title: "Сохранить",
    shortcut: "Ctrl+S",
    keywords: "save сохранить записать",
    help: "Сохранить проект; path — сохранить в другой файл (.syn) и дальше работать с ним.",
    args: &[arg("path", Ty::Str, "новый путь файла .syn")],
    danger: false,
    run: save,
};

fn save(ed: &mut Editor, a: &Args) -> Res {
    let path = match a.str("path")? {
        Some(p) => {
            let mut p = expand(&p);
            if p.extension().is_none() {
                p.set_extension(store::EXT);
            }
            p
        }
        None => ed.path.clone().ok_or("проект ещё не сохранялся — укажите path")?,
    };
    store::save(&ed.project, &path)?;
    ed.path = Some(path.clone());
    ed.dirty = false;
    Ok(Out::with(format!("Сохранено: {}", path.display()), json!({"path": path})))
}

static SETTINGS: Command = Command {
    id: "project.settings",
    category: "Проект",
    title: "Параметры проекта…",
    shortcut: "",
    keywords: "settings размер кадра fps частота разрешение 4k 60 ширина высота фон цвет",
    help: "Размер кадра (произвольные чётные width и height 16–16384), частота и цвет фона кадра (bg: #rrggbb или transparent).",
    args: &[
        arg("name", Ty::Str, "название"),
        arg("width", Ty::Num, "ширина кадра, пикс."),
        arg("height", Ty::Num, "высота кадра, пикс."),
        arg("fps", Ty::Num, "кадров в секунду"),
        arg("bg", Ty::Str, "цвет фона кадра (#rrggbb или transparent)"),
    ],
    danger: false,
    run: settings,
};

fn settings(ed: &mut Editor, a: &Args) -> Res {
    ed.begin("Параметры проекта");
    if let Some(n) = a.str("name")? {
        ed.project.name = n;
    }
    apply_settings(&mut ed.project, a)?;
    let s = &ed.project.settings;
    Ok(Out::text(format!("Проект: {}x{} @ {} fps, фон {}", s.width, s.height, s.fps, s.bg)))
}

fn apply_settings(p: &mut Project, a: &Args) -> Result<(), String> {
    if let Some(w) = a.num("width")? {
        p.settings.width = dim(w)?;
    }
    if let Some(h) = a.num("height")? {
        p.settings.height = dim(h)?;
    }
    if let Some(f) = a.num("fps")? {
        if !(1.0..=480.0).contains(&f) {
            return Err(format!("частота {f} вне 1–480"));
        }
        p.settings.fps = f;
    }
    if let Some(bg) = a.str("bg")? {
        let b = bg.trim();
        if b.eq_ignore_ascii_case("transparent") || b.eq_ignore_ascii_case("none") {
            p.settings.bg = "transparent".into();
        } else if syngui::mss::MssColor::parse(b).is_some() {
            p.settings.bg = b.to_string();
        } else {
            return Err(format!("цвет фона «{b}»: укажите #rrggbb или transparent"));
        }
    }
    Ok(())
}

fn dim(v: f64) -> Result<u32, String> {
    let v = v.round() as i64;
    if !(16..=16384).contains(&v) {
        return Err(format!("размер {v} вне 16–16384"));
    }
    // чётный: так требуют кодеки 4:2:0
    Ok((v as u32) & !1)
}

static DUMP: Command = Command {
    id: "project.dump",
    category: "Проект",
    title: "Таймлайн текстом",
    shortcut: "",
    keywords: "dump info таймлайн состав клипы",
    help: "Весь проект текстом: медиатека, дорожки, клипы с таймкодами и эффектами. Смотреть перед правкой.",
    args: &[],
    danger: false,
    run: dump,
};

fn dump(ed: &mut Editor, _a: &Args) -> Res {
    let mut s = ed.project.dump();
    if let Some(p) = &ed.path {
        s.push_str(&format!("Файл: {}{}\n", p.display(), if ed.dirty { " (не сохранён)" } else { "" }));
    }
    s.push_str(&format!("Позиция: {}", crate::doc::tc(ed.playhead)));
    if !ed.selection.is_empty() {
        s.push_str(&format!("; выделено: {}", ed.selection.iter().map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ")));
    }
    Ok(Out::text(s))
}

static EXPORT: Command = Command {
    id: "project.export",
    category: "Проект",
    title: "Экспорт видео…",
    shortcut: "Ctrl+E",
    keywords: "export render рендер экспорт mp4 сохранить видео вывести",
    help: "Отрендерить таймлайн в видеофайл (mp4, mov, mkv, webm) с титрами, эффектами и звуком; кадры — тот же композитор, что в просмотре. start/end — только часть. Перезаписывает файл, если он есть.",
    args: &[req("path", Ty::Str, "файл результата (.mp4)"), arg("crf", Ty::Num, "качество x264: 18 — высокое (по умолчанию), 23 — меньше файл"), arg("start", Ty::Time, "с какого момента"), arg("end", Ty::Time, "по какой момент"), arg("encoder", Ty::Choice(&["auto", "nvenc", "x264"]), "кодер: auto — NVENC на карте NVIDIA, иначе x264")],
    danger: false,
    run: export,
};

fn export(ed: &mut Editor, a: &Args) -> Res {
    let mut path = expand(&a.need_str("path")?);
    if path.extension().is_none() {
        path.set_extension("mp4");
    }
    let mut args = vec!["export".to_string(), path.display().to_string(), a.num("crf")?.unwrap_or(18.0).round().to_string()];
    if a.has("start") || a.has("end") {
        args.push(a.time("start")?.unwrap_or(0.0).to_string());
        args.push(a.time("end")?.unwrap_or(ed.project.duration()).to_string());
    }
    if let Some(e) = a.str("encoder")? {
        args.push(format!("encoder={e}"));
    }
    let r = crate::render::run_child(&ed.project, &args, |i, n| syn_assist::tools::progress(i, n, "кадры"))?;
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Ok(Out::with(format!("Экспорт готов: {r}, {:.1} МБ", size as f64 / 1048576.0), json!({"path": path})))
}

static FRAME: Command = Command {
    id: "frame.export",
    category: "Проект",
    title: "Кадр в картинку",
    shortcut: "",
    keywords: "frame snapshot кадр снимок картинка png проверить посмотреть",
    help: "Сохранить кадр таймлайна в момент at в PNG (тот же композитор, что экспорт) — посмотреть результат: титры, эффекты, кадрирование. width — ширина картинки (по умолчанию 960).",
    args: &[req("at", Ty::Time, "момент"), arg("path", Ty::Str, "файл .png (по умолчанию во временной папке)"), arg("width", Ty::Num, "ширина, пикс.")],
    danger: false,
    run: frame,
};

fn frame(ed: &mut Editor, a: &Args) -> Res {
    let t = a.need_time("at")?;
    let path = match a.str("path")? {
        Some(p) => expand(&p),
        None => std::env::temp_dir().join(format!("syn-video-editor-frame-{}.png", (t * 1000.0).round() as i64)),
    };
    let width = a.num("width")?.unwrap_or(960.0).round().max(16.0) as u32;
    let r = crate::render::run_child(&ed.project, &["frame".into(), t.to_string(), path.display().to_string(), width.to_string()], |_, _| {})?;
    Ok(Out::with(format!("Кадр {} → {r}", crate::doc::tc(t)), json!({"path": path})))
}

static CHECK: Command = Command {
    id: "frame.check",
    category: "Проект",
    title: "Проверить кадр глазами",
    shortcut: "",
    keywords: "проверить кадр посмотреть зрение vision самопроверка титр обрезан читается цвет",
    help: "Самопроверка по кадру: кадр таймлайна в момент at (тот же композитор, что экспорт) показывается модели со зрением, ответ — что она видит. question — что проверить (по умолчанию: читаются ли титры, не обрезан ли текст или объект, нет ли пустого/чёрного кадра, пересвета, странного цвета). Использовать после монтажа титров, кадрирования, цветокоррекции: проверить 1–3 ключевых момента и исправить найденное.",
    args: &[req("at", Ty::Time, "момент"), arg("question", Ty::Str, "что проверить на кадре"), arg("width", Ty::Num, "ширина кадра для модели (768)"), arg("model", Ty::Str, "модель со зрением (по умолчанию — из настроек или найдётся сама)")],
    danger: false,
    run: check,
};

/// Модель со зрением: явно заданная; из настроек («Модели → Зрение»); модель
/// агента, если видит картинки; уже загруженная со зрением (без лишней
/// загрузки); иначе первая со зрением.
pub(crate) fn vision_model(c: &mut synaptixd::Client, want: Option<String>) -> Result<String, String> {
    if let Some(m) = want.filter(|m| !m.trim().is_empty()).or_else(|| crate::settings::load().model_opt("vision")) {
        return Ok(m);
    }
    if c.llm_info("").map(|i| i.media).unwrap_or(false) {
        return Ok(String::new());
    }
    let list = c.list().map_err(|e| format!("служба нейросетей: {e}"))?;
    let llms: Vec<&synaptixd::ModelEntry> = list.iter().filter(|m| m.kind == synaptixd::ModelKind::Llm).collect();
    let (mut order, rest): (Vec<&synaptixd::ModelEntry>, Vec<&synaptixd::ModelEntry>) = llms.into_iter().partition(|m| m.state == synaptixd::LoadState::Loaded);
    order.extend(rest);
    for m in order {
        if c.llm_info(&m.id).map(|i| i.media).unwrap_or(false) {
            return Ok(m.id.clone());
        }
    }
    Err("нет языковой модели со зрением (картинки) — загрузите модель со зрением (например, Qwen3.6) или укажите model".into())
}

fn check(ed: &mut Editor, a: &Args) -> Res {
    let t = a.need_time("at")?;
    let q = a.str("question")?.filter(|q| !q.trim().is_empty()).unwrap_or_else(|| "Это кадр смонтированного видео. Кратко и по делу: что на кадре; читаются ли титры и текст целиком (не обрезаны ли краями, не сливаются ли с фоном); не обрезан ли главный объект; нет ли пустого или чёрного кадра, пересвета, странного цвета. Перечисли найденные проблемы или скажи, что их нет.".into());
    let set = crate::settings::load();
    let width = a.num("width")?.unwrap_or(set.vision_width as f64).round().clamp(128.0, 1920.0) as u32;
    let path = std::env::temp_dir().join(format!("syn-video-editor-check-{}-{}.png", std::process::id(), (t * 1000.0).round() as i64));
    crate::render::run_child(&ed.project, &["frame".into(), t.to_string(), path.display().to_string(), width.to_string()], |_, _| {})?;
    syn_assist::tools::progress(0, 0, "смотрю кадр");
    let mut c = synaptixd::Client::connect().map_err(|e| format!("служба нейросетей: {e}"))?;
    let model = vision_model(&mut c, a.str("model")?)?;
    let mut m = synaptixd::Message::user(format!("[file 1: кадр {}]\n{}\n{q}", crate::doc::tc(t), synaptixd::MEDIA_MARK));
    m.media = vec![synaptixd::Media { kind: synaptixd::MediaKind::Image, path: path.display().to_string() }];
    let r = c.generate(&model, vec![m], set.vision_max_tokens.clamp(64, 8192) as usize, Some(set.vision_temperature.clamp(0.0, 2.0)), |_| {}).map_err(|e| format!("модель зрения: {e}"))?;
    let text = r.text.trim().to_string();
    Ok(Out::with(format!("Кадр {} ({}): {text}", crate::doc::tc(t), path.display()), json!({"path": path, "model": model, "answer": text})))
}
