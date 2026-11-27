//! Рецепты ассистента (D4): прогон даёт редактируемый проект.

use serde_json::{json, Value};

use super::{arg, req, Args, Command, Out, Res, Ty};
use crate::editor::Editor;

pub static COMMANDS: &[&Command] = &[&LIST, &RUN];

static LIST: Command = Command {
    id: "recipe.list",
    category: "ИИ",
    title: "Рецепты",
    shortcut: "",
    keywords: "recipe рецепт сценарий генерация видео история клип музыка аватар дубляж",
    help: "Рецепты ассистента (графы synaptix-graph): имя, описание, параметры и как прогон ляжет на таймлайн (узел роликов, звук, имена клипов).",
    args: &[],
    danger: false,
    run: list,
};

fn list(_ed: &mut Editor, _a: &Args) -> Res {
    let all = synaptix_graph::recipes::list(&crate::recipe::dir());
    let lines: Vec<String> = all
        .iter()
        .map(|g| {
            let (v, a, n) = crate::recipe::plan(g);
            format!("{} — {} | параметры: {} | таймлайн: ролики {}, звук {}, имена {}", g.name, g.description, synaptix_graph::recipes::params_line(g), v.unwrap_or_else(|| "—".into()), if a.is_empty() { "—".into() } else { a.join(", ") }, n.unwrap_or_else(|| "—".into()))
        })
        .collect();
    Ok(Out::with(lines.join("\n"), json!(all.iter().map(|g| g.name.clone()).collect::<Vec<_>>())))
}

static RUN: Command = Command {
    id: "recipe.run",
    category: "ИИ",
    title: "Прогнать рецепт в проект",
    shortcut: "",
    keywords: "recipe рецепт прогнать сгенерировать проект таймлайн",
    help: "Прогнать рецепт (службой нейросетей) и собрать из результата редактируемый проект: ролики сцен клипами по порядку (имена — строки сценария), музыка и речь — на звуковых дорожках, если ролики без звука. Проект сохраняется в ~/Videos/syn-video-editor и открывается, если текущий пуст или сохранён. Долго (генерация), нужна видеокарта.",
    args: &[req("name", Ty::Str, "рецепт (recipe.list)"), arg("params", Ty::Object, "параметры рецепта"), arg("open", Ty::Bool, "открыть проект (по умолчанию да)")],
    danger: false,
    run: run,
};

fn run(ed: &mut Editor, a: &Args) -> Res {
    let done = generate(a)?;
    apply(ed, a, done)
}

/// Готовый прогон рецепта: проект на диске и отчёт — до применения к редактору.
pub struct Generated {
    g: synaptix_graph::Graph,
    report: synaptix_graph::Report,
    built: Editor,
    path: std::path::PathBuf,
}

/// Долгая часть `recipe.run` — генерация службой и сборка проекта на диске.
/// Документ редактора не нужен: агент зовёт её без блокировки (иначе окно,
/// ждущее ту же блокировку для отрисовки, висит всю генерацию).
pub fn generate(a: &Args) -> Result<Generated, String> {
    let name = a.need_str("name")?;
    let g = synaptix_graph::recipes::find(&crate::recipe::dir(), &name).ok_or_else(|| format!("рецепта «{name}» нет — recipe.list"))?;
    let params = a.object("params")?.unwrap_or_default();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let run_dir = std::env::var_os("XDG_CACHE_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache"))).unwrap_or_else(std::env::temp_dir).join("syn-video-editor/recipes").join(format!("{}-{stamp}", synaptix_graph::recipes::slug(&g.name)));
    let report = synaptix_graph::run(&g, &params, &run_dir, &mut |s| syn_assist::tools::progress(0, 0, s))?;
    // отчёт прогона рядом с файлами: что дал каждый узел (разбор, если раскладка вышла не той)
    let rep = json!({"recipe": g.name, "params": params, "steps": report.steps, "nodes": report.nodes});
    let _ = std::fs::write(run_dir.join("report.json"), serde_json::to_string_pretty(&rep).unwrap_or_default());
    let built = crate::recipe::build(&g, &report, &g.name)?;
    let path = crate::recipe::project_path(&run_dir, &g.name);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    crate::store::save(&built.project, &path)?;
    Ok(Generated { g, report, built, path })
}

/// Короткая часть: открыть собранный проект в редакторе и ответить агенту.
pub fn apply(ed: &mut Editor, a: &Args, done: Generated) -> Res {
    let Generated { g, report, built, path } = done;
    let clips: usize = built.project.tracks.iter().map(|t| t.clips.len()).sum();
    let open = a.bool("open")?.unwrap_or(true);
    let can = !ed.dirty || ed.project.tracks.iter().all(|t| t.clips.is_empty());
    let opened = open && can;
    if opened {
        ed.replace(built.project, Some(path.clone()));
    }
    let files_str: Vec<String> = report.files.iter().map(|f| f.display().to_string()).collect();
    let refs_note = report
        .nodes
        .get("refs")
        .filter(|o| !o.files.is_empty())
        .map(|o| {
            let list: Vec<String> = o.files.iter().enumerate().map(|(i, p)| format!("<Picture {}> {}", i + 1, p.display())).collect();
            format!(" | референсы: {}", list.join(", "))
        })
        .unwrap_or_default();
    let out_files_note = if files_str.is_empty() { String::new() } else { format!(" | файлы: {}", files_str.join(", ")) };
    Ok(Out::with(
        format!(
            "Рецепт «{}»: проект {} ({clips} клипов){}{out_files_note}{refs_note}",
            g.name,
            path.display(),
            if opened { ", открыт" } else if open { " — текущий проект не сохранён, откройте файл сами" } else { "" }
        ),
        json!({"path": path.display().to_string(), "clips": clips, "files": files_str}),
    ))
}
