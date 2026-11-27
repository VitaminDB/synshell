//! Рецепты ассистента (D4): прогон даёт редактируемый проект.

use serde_json::json;

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
    args: &[req("name", Ty::Str, "рецепт (recipe.list)"), arg("params", Ty::Object, "параметры рецепта"), arg("open", Ty::Bool, "открыть проект (по умолчанию да)"), arg("label", Ty::Str, "подпись генерации в медиатеке — место в фильме: «Сцена 3 из 12: <название>», «Эталон: <герой>»"), arg("asset", Ty::Str, "имя файлов результата в медиатеке (герой, локация, «Сцена 3»), вместо ref.png / shots-000.mp4")],
    danger: false,
    run: run,
};

fn run(ed: &mut Editor, a: &Args) -> Res {
    let project = ed.path.clone();
    let done = generate(a, project.as_deref(), &mut |_, _| {})?;
    apply(ed, a, done)
}

/// Служебная строка хода с превью генерации: `PREVIEW_TAG узел \t путь.png`.
pub const PREVIEW_TAG: &str = "\u{1}preview\t";

/// Готовый прогон рецепта: проект на диске и отчёт — до применения к редактору.
pub struct Generated {
    g: synaptix_graph::Graph,
    report: synaptix_graph::Report,
    built: Editor,
    path: std::path::PathBuf,
    /// Файлы узлов, показанные в медиатеке по ходу прогона (эталоны героев,
    /// локаций, ролики) — переносятся и в собранный проект.
    live: Vec<std::path::PathBuf>,
}

/// Долгая часть `recipe.run` — генерация службой и сборка проекта на диске.
/// Документ редактора не нужен: агент зовёт её без блокировки (иначе окно,
/// ждущее ту же блокировку для отрисовки, висит всю генерацию).
/// `project` — файл текущего проекта: генерация ложится в его папку
/// (`<имя>.media/generated/…`), без файла — в кэш программы.
pub fn generate(a: &Args, project: Option<&std::path::Path>, on_files: &mut dyn FnMut(&str, &[std::path::PathBuf])) -> Result<Generated, String> {
    let name = a.need_str("name")?;
    let g = synaptix_graph::recipes::find(&crate::recipe::dir(), &name).ok_or_else(|| format!("рецепта «{name}» нет — recipe.list"))?;
    let mut params = a.object("params")?.unwrap_or_default();
    // «ё» в русских репликах: без неё модели речи читают «еще», «идет»
    for v in params.values_mut() {
        crate::yo::yofy_value(v);
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let base = match project {
        Some(p) => crate::store::media_dir(p).join("generated"),
        None => std::env::var_os("XDG_CACHE_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache"))).unwrap_or_else(std::env::temp_dir).join("syn-video-editor/recipes"),
    };
    let run_dir = base.join(format!("{}-{stamp}", synaptix_graph::recipes::slug(&g.name)));
    // «Стоп» в панели ИИ обрывает и сам прогон в службе (картинки, видео), а не
    // только ход агента между шагами: сторож переводит флаг в отмену графа
    let cancel = synaptix_graph::Cancel::new();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let (cancel, done) = (cancel.clone(), done.clone());
        std::thread::spawn(move || {
            while !done.load(std::sync::atomic::Ordering::Relaxed) {
                if crate::ui::AI_CANCEL.load(std::sync::atomic::Ordering::SeqCst) {
                    cancel.cancel();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        });
    }
    let mut live: Vec<std::path::PathBuf> = Vec::new();
    // имя ассета (`asset`: «Грымза», «Сцена 3»): файлы результата — по нему, а
    // не одинаковые «ref.png» / «shots-000.mp4» в медиатеке
    let asset = a.str("asset")?.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let mut renamed: std::collections::HashMap<std::path::PathBuf, std::path::PathBuf> = std::collections::HashMap::new();
    let report = synaptix_graph::run_observed(
        &g,
        &params,
        &run_dir,
        &cancel,
        &mut |s| syn_assist::tools::progress(0, 0, s),
        &mut |node, files| {
            let files: Vec<std::path::PathBuf> = match &asset {
                Some(name) => files.iter().map(|f| asset_name(&mut renamed, f, name)).collect(),
                None => files.to_vec(),
            };
            live.extend(files.iter().cloned());
            on_files(node, &files);
        },
        // превью по ходу — плитке медиатеки служебной строкой хода (в чат не идёт)
        &mut |node, png| syn_assist::tools::progress(0, 0, &format!("{PREVIEW_TAG}{node}\t{}", png.display())),
    );
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut report = report.map_err(|e| if cancel.is_cancelled() { "остановлено пользователем («Стоп»)".to_string() } else { e })?;
    if let Some(name) = &asset {
        // файлы, пришедшие только в итоге (не элементами), — тоже по имени
        for f in report.files.iter_mut() {
            *f = asset_name(&mut renamed, f, name);
        }
        for o in report.nodes.values_mut() {
            for f in o.files.iter_mut() {
                *f = asset_name(&mut renamed, f, name);
            }
        }
    }
    // превью по ходу генерации нужны только во время прогона — не копить в папке проекта
    fn drop_previews(d: &std::path::Path) {
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    drop_previews(&p);
                } else if p.file_name().is_some_and(|n| n.to_string_lossy().contains(".preview-")) {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }
    drop_previews(&run_dir);
    // отчёт прогона рядом с файлами: что дал каждый узел (разбор, если раскладка вышла не той)
    let rep = json!({"recipe": g.name, "params": params, "steps": report.steps, "nodes": report.nodes});
    let _ = std::fs::write(run_dir.join("report.json"), serde_json::to_string_pretty(&rep).unwrap_or_default());
    let built = crate::recipe::build(&g, &report, &g.name)?;
    // у проекта с файлом сборка — в папке прогона (рядом с его файлами), а не
    // отдельным «*.syn» в ~/Videos: в текущий проект результат идёт сам
    let path = if project.is_some() { run_dir.join(format!("{}.syn", synaptix_graph::recipes::slug(&g.name))) } else { crate::recipe::project_path(&run_dir, &g.name) };
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    crate::store::save(&built.project, &path)?;
    Ok(Generated { g, report, built, path, live })
}

/// Переименовать файл результата по имени ассета (`<имя>.ext`, `<имя>-2.ext`…)
/// в том же каталоге; повторный вызов для того же файла — прежнее имя.
fn asset_name(map: &mut std::collections::HashMap<std::path::PathBuf, std::path::PathBuf>, f: &std::path::Path, name: &str) -> std::path::PathBuf {
    if let Some(n) = map.get(f) {
        return n.clone();
    }
    if map.values().any(|v| v == f) || !f.exists() {
        return f.to_path_buf();
    }
    let stem: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | ' ' | '_') { c } else { '_' }).collect::<String>().trim().chars().take(60).collect();
    let ext = f.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let dir = f.parent().unwrap_or(std::path::Path::new("."));
    let mut k = 1;
    let target = loop {
        let n = if k == 1 { format!("{stem}.{ext}") } else { format!("{stem}-{k}.{ext}") };
        let p = dir.join(n);
        if !p.exists() {
            break p;
        }
        k += 1;
    };
    let out = match std::fs::rename(f, &target) {
        Ok(()) => target,
        Err(_) => f.to_path_buf(),
    };
    map.insert(f.to_path_buf(), out.clone());
    out
}

/// Короткая часть: открыть собранный проект в редакторе и ответить агенту.
pub fn apply(ed: &mut Editor, a: &Args, done: Generated) -> Res {
    let Generated { g, report, built, path, live } = done;
    let clips: usize = built.project.tracks.iter().map(|t| t.clips.len()).sum();
    let open = a.bool("open")?.unwrap_or(true);
    let can = !ed.dirty || ed.project.tracks.iter().all(|t| t.clips.is_empty());
    // в непустом проекте с файлом результат добавляется в его медиатеку —
    // проект не подменяется другим файлом
    let into_current = ed.path.is_some() && !ed.project.tracks.iter().all(|t| t.clips.is_empty());
    let opened = open && can && !into_current;
    // пустой проект с файлом (тот, в котором попросили сделать) — собрать в
    // него же: имя, файл и состояние окна проекта (вкладки графов) остаются
    let empty_here = ed.path.is_some() && ed.project.tracks.iter().all(|t| t.clips.is_empty());
    let path = if opened && empty_here { ed.path.clone().unwrap_or(path) } else { path };
    if opened {
        let mut project = built.project;
        if empty_here {
            // имя — своё; медиатека, набранная по ходу прогона, вернётся импортом ниже
            project.name = ed.project.name.clone();
        }
        ed.replace(project, Some(path.clone()));
        if empty_here {
            ed.dirty = true;
        }
    }
    // всё, что появилось по ходу (эталоны героев и локаций и т.п.), — и в
    // медиатеке итогового проекта, а не только ролики на таймлайне
    let have: std::collections::HashSet<std::path::PathBuf> = ed.project.media.iter().map(|m| m.path.clone()).collect();
    let mut live = live;
    if into_current {
        for f in &report.files {
            if !live.contains(f) {
                live.push(f.clone());
            }
        }
    }
    let missing: Vec<String> = live.iter().filter(|f| !have.contains(*f) && f.exists()).map(|f| f.display().to_string()).collect();
    if !missing.is_empty() {
        let _ = crate::cmd::run(ed, "media.import", &json!({"paths": missing}));
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
            if opened { ", открыт" } else if into_current { " — результат добавлен в медиатеку текущего проекта (на таймлайн — clip_add)" } else if open { " — текущий проект не сохранён, откройте файл сами" } else { "" }
        ),
        json!({"path": path.display().to_string(), "clips": clips, "files": files_str}),
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn asset_names() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("ref.png");
        let b = d.path().join("shots-000.mp4");
        let c = d.path().join("film.mp4");
        for f in [&a, &b, &c] {
            std::fs::write(f, b"x").unwrap();
        }
        let mut m = std::collections::HashMap::new();
        let na = super::asset_name(&mut m, &a, "Герой: тролль");
        assert_eq!(na.file_name().unwrap(), "Герой_ тролль.png");
        assert!(na.exists() && !a.exists());
        // тот же файл повторно (итог прогона) — то же имя
        assert_eq!(super::asset_name(&mut m, &a, "Герой: тролль"), na);
        let nb = super::asset_name(&mut m, &b, "Сцена 3");
        let nc = super::asset_name(&mut m, &c, "Сцена 3");
        assert_eq!(nb.file_name().unwrap(), "Сцена 3.mp4");
        assert_eq!(nc.file_name().unwrap(), "Сцена 3-2.mp4");
    }
}
