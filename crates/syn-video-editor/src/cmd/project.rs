use serde_json::json;

use super::{arg, req, Args, Command, Out, Res, Ty};
use crate::doc::{expand, Project};
use crate::editor::Editor;
use crate::store;

pub static COMMANDS: &[&Command] = &[&NEW, &OPEN, &SAVE, &SETTINGS, &DUMP];

static NEW: Command = Command {
    id: "project.new",
    category: "Проект",
    title: "Новый проект",
    shortcut: "Ctrl+N",
    keywords: "new создать проект",
    help: "Создать пустой проект (дорожки V1 и A1). Несохранённые изменения текущего теряются.",
    args: &[arg("name", Ty::Str, "название"), arg("width", Ty::Num, "ширина кадра, пикс."), arg("height", Ty::Num, "высота кадра, пикс."), arg("fps", Ty::Num, "кадров в секунду")],
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
    keywords: "settings размер кадра fps частота разрешение 4k 60",
    help: "Размер кадра и частота проекта (например, 3840x2160 и 60 fps для экспорта в 4K 60).",
    args: &[arg("name", Ty::Str, "название"), arg("width", Ty::Num, "ширина кадра"), arg("height", Ty::Num, "высота кадра"), arg("fps", Ty::Num, "кадров в секунду")],
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
    Ok(Out::text(format!("Проект: {}x{} @ {} fps", s.width, s.height, s.fps)))
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
