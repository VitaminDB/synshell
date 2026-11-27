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
    // эталон без имени — в медиатеке одинаковые «ref.png»: имя обязательно
    // (или из подписи «Эталон: <имя>»)
    // съёмка сцены N — только после сцены N-1 (сцена за сценой); ссылки
    // строк на референсы — проверить до видеокарты
    if g.name == "ref-scenes" {
        if let Some(n) = asset_of(a)?.and_then(|s| super::story::scene_number(&s)) {
            super::story::gate(project, n, "Съёмка")?;
        }
        let p = a.object("params")?.unwrap_or_default();
        let refs: Vec<String> = p.get("refs").and_then(|v| v.as_array()).map(|v| v.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let sb = p.get("storyboard").and_then(|v| v.as_str()).unwrap_or("");
        check_refs(sb, &refs)?;
    }
    if g.name == "ref-image" && asset_of(a)?.is_none() {
        return Err("ref-image: нужен asset — имя эталона (герой или локация) на верхнем уровне вызова, рядом с name: {name: \"ref-image\", asset: \"<имя героя>\", params: {…}}; файл назовётся по нему".into());
    }
    let mut params = a.object("params")?.unwrap_or_default();
    params.remove("asset");
    params.remove("label");
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
    let asset = asset_of(a)?;
    let mut renamed: std::collections::HashMap<std::path::PathBuf, std::path::PathBuf> = std::collections::HashMap::new();
    let mut shots_seen = 0usize;
    let report = synaptix_graph::run_observed(
        &g,
        &params,
        &run_dir,
        &cancel,
        &mut |s| syn_assist::tools::progress(0, 0, s),
        &mut |node, files| {
            let files: Vec<std::path::PathBuf> = match &asset {
                Some(name) => files.iter().map(|f| asset_name(&mut renamed, f, &part_name(name, node, f))).collect(),
                None => files.to_vec(),
            };
            // склейка из одного ролика — тот же ролик: в медиатеку не класть
            if node == "film" && shots_seen <= 1 {
                return;
            }
            if node == "shots" {
                shots_seen += files.len();
            }
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
        for (node, o) in report.nodes.iter_mut() {
            for f in o.files.iter_mut() {
                *f = asset_name(&mut renamed, f, &part_name(name, node, f));
            }
        }
        for f in report.files.iter_mut() {
            *f = asset_name(&mut renamed, f, name);
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

/// Строки раскадровки ссылаются только на существующие референсы, и у
/// каждой строки (ролика) не больше 3 звуков и 9 картинок — иначе H3 падает
/// посреди съёмки, а снятые ролики пропадают.
pub fn check_refs(storyboard: &str, refs: &[String]) -> Result<(), String> {
    let is_audio = |r: &String| {
        let l = r.to_lowercase();
        [".wav", ".mp3", ".flac", ".ogg", ".opus", ".m4a", ".aac"].iter().any(|e| l.ends_with(e))
    };
    let is_video = |r: &String| {
        let l = r.to_lowercase();
        [".mp4", ".mkv", ".mov", ".webm", ".avi"].iter().any(|e| l.ends_with(e))
    };
    let audios = refs.iter().filter(|r| is_audio(r)).count();
    let images = refs.iter().filter(|r| !is_audio(r) && !is_video(r)).count();
    let tags = |line: &str, word: &str| -> Vec<usize> {
        let mut v: Vec<usize> = Vec::new();
        let pat = format!("<{word} ");
        let mut rest = line;
        while let Some(i) = rest.find(&pat) {
            rest = &rest[i + pat.len()..];
            let d: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(k) = d.parse::<usize>() {
                if !v.contains(&k) {
                    v.push(k);
                }
            }
        }
        v
    };
    for (i, line) in storyboard.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        // формат Ref2VA: шесть полей по порядку (иначе H3 путает роли и постановку)
        const FIELDS: [&str; 6] = ["subject_definitions:", "summary:", "retention_analysis:", "detailed_description:", "overall_soundscape:", "non_diegetic_music:"];
        let mut at = 0usize;
        for f in FIELDS {
            match line[at..].find(f) {
                Some(j) => at += j + f.len(),
                None => {
                    return Err(format!(
                        "строка {} не в формате Ref2VA: нет поля «{f}» на своём месте — нужны все шесть полей по порядку: {} (навык minimax-h3; каркас строки даёт scene.plan)",
                        i + 1,
                        FIELDS.join(" … ")
                    ))
                }
            }
        }
        let au = tags(line, "Audio");
        let pi = tags(line, "Picture");
        if let Some(k) = au.iter().find(|k| **k == 0 || **k > audios) {
            return Err(format!("строка {} ссылается на <Audio {k}>, а звуков в refs {audios}", i + 1));
        }
        if let Some(k) = pi.iter().find(|k| **k == 0 || **k > images) {
            return Err(format!("строка {} ссылается на <Picture {k}>, а картинок в refs {images}", i + 1));
        }
        if au.len() > 3 {
            return Err(format!("строка {} (ролик) ссылается на {} звуков ({}) — у H3 не больше 3 на ролик: в строке ролика упоминай только ЕГО дорожки из scene.plan", i + 1, au.len(), au.iter().map(|k| format!("<Audio {k}>")).collect::<Vec<_>>().join(", ")));
        }
        if pi.len() > 9 {
            return Err(format!("строка {} ссылается на {} картинок — у H3 не больше 9 на ролик", i + 1, pi.len()));
        }
    }
    Ok(())
}

/// Имя файла узла ассета: ролики сцены — «<имя> — ролик N» (по номеру
/// элемента в имени файла узла: shots-000 → 1), склейка — «<имя> — склейка»,
/// остальное — «<имя>».
fn part_name(asset: &str, node: &str, f: &std::path::Path) -> String {
    let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    match node {
        "shots" => {
            let k: usize = stem.rsplit('-').next().and_then(|d| d.parse().ok()).unwrap_or(0);
            format!("{asset} — ролик {}", k + 1)
        }
        "film" => format!("{asset} — склейка"),
        _ => asset.to_string(),
    }
}

/// Имя ассета: `asset`, иначе из подписи (`label` «Эталон: Кот» → «Кот»,
/// «Сцена 3 из 12: …» → «Сцена 3»).
fn asset_of(a: &Args) -> Result<Option<String>, String> {
    if let Some(s) = a.str("asset")?.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        return Ok(Some(s));
    }
    // агент кладёт и внутрь params — тоже принять
    for k in ["asset", "label"] {
        if let Some(s) = a.object("params")?.and_then(|p| p.get(k).and_then(|v| v.as_str()).map(|s| s.trim().to_string())).filter(|s| !s.is_empty()) {
            let s = if k == "label" { s.split_once(':').map(|(_, b)| b.trim().to_string()).unwrap_or(s) } else { s };
            return Ok(Some(s));
        }
    }
    let Some(l) = a.str("label")?.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else { return Ok(None) };
    if let Some(rest) = l.strip_prefix("Сцена ") {
        let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !n.is_empty() {
            return Ok(Some(format!("Сцена {n}")));
        }
    }
    let name = l.split_once(':').map(|(_, b)| b.trim().to_string()).unwrap_or(l);
    Ok(Some(name).filter(|s| !s.is_empty()))
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
    // ролики сцены сняты — когда сняты все ролики её плана, открыта следующая
    if g.name == "ref-scenes" {
        if let Some(n) = asset_of(a)?.and_then(|s| super::story::scene_number(&s)) {
            let shot = report.nodes.get("shots").map(|o| o.files.len()).unwrap_or(1).max(1);
            super::story::add_shots(ed.path.as_deref(), n, shot);
        }
    }
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

    #[test]
    fn part_names_and_refs_check() {
        use std::path::Path;
        assert_eq!(super::part_name("Сцена 1", "shots", Path::new("/x/shots-001.mp4")), "Сцена 1 — ролик 2");
        assert_eq!(super::part_name("Сцена 1", "film", Path::new("/x/film.mp4")), "Сцена 1 — склейка");
        let refs: Vec<String> = ["a.png", "b.png", "c.png", "1.wav", "2.wav", "3.wav", "4.wav"].iter().map(|s| s.to_string()).collect();
        let l = |body: &str| format!("subject_definitions: {body} summary: x retention_analysis: x detailed_description: x overall_soundscape: x non_diegetic_music: N/A");
        assert!(super::check_refs(&format!("{}\n{}", l("<Picture 1> <Audio 1> <Audio 2>"), l("<Picture 2> <Audio 3> <Audio 4>")), &refs).is_ok());
        let e = super::check_refs(&format!("{}\n{}", l("<Picture 1> <Audio 1>"), l("<Audio 1> <Audio 2> <Audio 3> <Audio 4>")), &refs).unwrap_err();
        assert!(e.contains("строка 2") && e.contains("4 звуков"), "{e}");
        assert!(super::check_refs(&l("<Audio 5>"), &refs).is_err());
        assert!(super::check_refs(&l("<Picture 4>"), &refs).is_err());
        // без summary: (как в упавшем прогоне) — отказ с порядком полей
        let e = super::check_refs("[reference generation] subject_definitions: x retention_analysis: x detailed_description: x overall_soundscape: x non_diegetic_music: x", &refs).unwrap_err();
        assert!(e.contains("summary:"), "{e}");
    }
}
