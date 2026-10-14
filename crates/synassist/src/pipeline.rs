//! Инструмент `pipeline`: многошаговые задачи графом-сценарием или рецептом
//! (см. `synaptix_graph`) — шаги исполняет служба нейросетей и ffmpeg.

use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::tools::{str_arg, Danger, Tool, ToolOutput, ToolResult};

/// Каталог рецептов из настройки; пусто — по умолчанию.
pub fn resolve(setting: &str) -> PathBuf {
    if setting.trim().is_empty() {
        return crate::chat::data_dir().join("recipes");
    }
    crate::builtin::expand(setting)
}

/// Раздел системного промпта: рецепты с параметрами.
pub fn prompt_section(dir: &std::path::Path) -> String {
    let list = synaptix_graph::recipes::list(dir);
    let mut s = String::from("\n\n## Рецепты (инструмент pipeline)\nМногошаговые задачи с медиа (клип, говорящий аватар, видеоистория, оживить фото, переозвучка) выполняй одним вызовом pipeline: рецептом по имени с params или своим графом. Рецепты:\n");
    for g in list {
        s.push_str(&format!("- {}: {} Параметры: {}.\n", g.name, g.description, synaptix_graph::recipes::params_line(&g)));
    }
    s
}

pub struct PipelineTool {
    pub dir: PathBuf,
}

impl Tool for PipelineTool {
    fn name(&self) -> &'static str {
        "pipeline"
    }
    fn group(&self) -> &'static str {
        "медиа"
    }
    fn description(&self) -> String {
        "action: run (по умолчанию) — выполнить; save — сохранить graph своим рецептом (в graph обязательны name — короткое имя латиницей, description — когда применять, params — то, что меняется от раза к разу: тема, файлы, текст, с type text|file|files|number, description и default; в узлах вместо конкретных значений — {{параметр}}); delete — удалить свой рецепт recipe. Пример графа: {\"name\":\"poster\",\"description\":\"постер со слоганом по теме\",\"params\":{\"topic\":{\"type\":\"text\",\"description\":\"тема\"}},\"nodes\":[{\"id\":\"slogan\",\"op\":\"llm\",\"prompt\":\"Слоган на тему {{topic}}, только текст\"},{\"id\":\"art\",\"op\":\"image\",\"prompt\":\"vertical poster about {{topic}}, bold composition\",\"width\":768,\"height\":1024}],\"output\":[\"art\",\"slogan\"]} — output перечисляет id узлов, размеры задавай оба. Сохраняй, когда пользователь просит, а после удачного прогона своего графа коротко предложи сохранить его рецептом. Выполнить многошаговый сценарий с нейросетями: recipe — имя рецепта из системного промпта и params — его параметры (пути файлов — абсолютные); или graph — свой граф: {\"params\":{...}, \"nodes\":[{\"id\":\"…\",\"op\":\"…\", поля}], \"output\":[\"id\"]}. Операции: llm (prompt, system), text (text), image (prompt, width, height, images — референсы), video (prompt, seconds, first_frame, last_frame, refs, audio, source_video, extend, control_video, restyle), music (prompt, lyrics, seconds — только если пользователь назвал длительность, иначе не указывай: песня около 3 минут), speak (text, voice), transcribe (file), concat (files), mux (video, audio, mode: replace|mix), frame (video, at: first|last|секунда), trim (file, start, end), still (image, audio). Поля ссылаются на параметры и результаты: {{param}}, {{id.text}}, {{id.file}}, {{id.files}}, {{id.lines}}; each: \"{{список}}\" — выполнить узел для каждого элемента ({{item}}). Результат покажется пользователю. Генерация видео и музыки идёт минуты.".into()
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["run", "save", "delete"]},
                "recipe": {"type": "string", "description": "имя рецепта"},
                "params": {"type": "object", "description": "параметры рецепта"},
                "graph": {"type": "object", "description": "свой граф вместо рецепта"}
            }
        })
    }
    fn danger(&self, args: &Value) -> Danger {
        // Удаление своего рецепта пользователь подтверждает.
        if str_arg(args, "action") == Some("delete") {
            Danger::Confirm
        } else {
            Danger::Safe
        }
    }
    fn summary(&self, args: &Value) -> String {
        if str_arg(args, "action") == Some("save") {
            let name = args.get("graph").and_then(|g| g.get("name")).and_then(Value::as_str).unwrap_or("");
            return format!("Сохраняю рецепт: {name}");
        }
        if str_arg(args, "action") == Some("delete") {
            return format!("Удаляю рецепт: {}", str_arg(args, "recipe").unwrap_or(""));
        }
        match str_arg(args, "recipe") {
            Some(r) => format!("Рецепт: {r}"),
            None => "Сценарий из нескольких шагов".into(),
        }
    }
    fn run(&self, args: &Value) -> ToolResult {
        self.run_ctx(args, &std::sync::atomic::AtomicBool::new(false))
    }

    fn run_ctx(&self, args: &Value, cancel: &std::sync::atomic::AtomicBool) -> ToolResult {
        match str_arg(args, "action") {
            Some("save") => {
                let g: synaptix_graph::Graph = serde_json::from_value(args.get("graph").cloned().ok_or("нужен graph")?).map_err(|e| format!("граф: {e}"))?;
                let (path, existed) = synaptix_graph::recipes::save(&self.dir, &g)?;
                let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                return Ok(ToolOutput::text(format!(
                    "Рецепт «{name}» {} ({}). Он виден в окне ассистента → ⚡ Рецепты и доступен по имени в следующих разговорах.",
                    if existed { "обновлён" } else { "сохранён" },
                    path.display()
                )));
            }
            Some("delete") => {
                let name = synaptix_graph::recipes::slug(crate::tools::need_str(args, "recipe")?);
                let path = self.dir.join(format!("{name}.json"));
                if !path.exists() {
                    return Err(format!("своего рецепта «{name}» нет (встроенные не удаляются)"));
                }
                std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                return Ok(ToolOutput::text(format!("Рецепт «{name}» удалён.")));
            }
            _ => {}
        }
        let graph = match (str_arg(args, "recipe"), args.get("graph")) {
            (Some(name), _) => synaptix_graph::recipes::find(&self.dir, name).ok_or_else(|| {
                let names: Vec<String> = synaptix_graph::recipes::list(&self.dir).into_iter().map(|g| g.name).collect();
                format!("рецепта «{name}» нет; есть: {}", names.join(", "))
            })?,
            (None, Some(g)) => {
                // Длительность музыки в своём графе — только названная пользователем
                // (модель ставит 30 с по привычке); иначе — песня целиком.
                let mut g = g.clone();
                if !crate::tools::asked_duration() {
                    for n in g.get_mut("nodes").and_then(Value::as_array_mut).into_iter().flatten() {
                        if n.get("op").and_then(Value::as_str) == Some("music") {
                            if let Some(o) = n.as_object_mut() {
                                o.remove("seconds");
                            }
                        }
                    }
                }
                serde_json::from_value(g).map_err(|e| format!("граф: {e}"))?
            }
            _ => return Err("нужен recipe или graph".into()),
        };
        let params: Map<String, Value> = args.get("params").and_then(Value::as_object).cloned().unwrap_or_default();
        let dir = crate::builtin::media::media_dir().join(format!("pipeline-{:x}", crate::chat::now()));
        // Ход: «узел (операция) · шаг/всего» → шаг и подпись для панели.
        let stop = synaptix_graph::Cancel::new();
        let report = crate::tools::watch_cancel(cancel, || stop.cancel(), || {
            synaptix_graph::run_cancellable(&graph, &params, &dir, &stop, &mut |s| {
                match s.rsplit_once(" · ").and_then(|(head, frac)| frac.split_once('/').map(|(a, b)| (head, a.parse::<usize>(), b.parse::<usize>()))) {
                    Some((head, Ok(i), Ok(n))) => crate::tools::progress(i, n, head),
                    _ => crate::tools::progress(0, 0, s),
                }
            })
        })?;
        let mut text = format!("Сценарий{} выполнен; результат уже показан пользователю под ответом (пути — для инструментов, пользователю не называй).\nШаги:\n", if graph.name.is_empty() { String::new() } else { format!(" «{}»", graph.name) });
        for s in &report.steps {
            text.push_str(&format!("- {s}\n"));
        }
        if !report.text.is_empty() {
            text.push_str(&format!("\nТекст результата:\n{}\n", report.text));
        }
        text.push_str(&format!("Файлы: {}", report.files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>().join(", ")));
        Ok(ToolOutput { text, files: report.files })
    }
}
