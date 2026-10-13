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
        "Выполнить многошаговый сценарий с нейросетями: recipe — имя рецепта из системного промпта и params — его параметры (пути файлов — абсолютные); или graph — свой граф: {\"params\":{...}, \"nodes\":[{\"id\":\"…\",\"op\":\"…\", поля}], \"output\":[\"id\"]}. Операции: llm (prompt, system), text (text), image (prompt, width, height, images — референсы), video (prompt, seconds, first_frame, last_frame, refs, audio, source_video, extend, control_video, restyle), music (prompt, lyrics, seconds), speak (text, voice), transcribe (file), concat (files), mux (video, audio, mode: replace|mix), frame (video, at: first|last|секунда), trim (file, start, end), still (image, audio). Поля ссылаются на параметры и результаты: {{param}}, {{id.text}}, {{id.file}}, {{id.files}}, {{id.lines}}; each: \"{{список}}\" — выполнить узел для каждого элемента ({{item}}). Результат покажется пользователю. Генерация видео и музыки идёт минуты.".into()
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "recipe": {"type": "string", "description": "имя рецепта"},
                "params": {"type": "object", "description": "параметры рецепта"},
                "graph": {"type": "object", "description": "свой граф вместо рецепта"}
            }
        })
    }
    fn danger(&self, _: &Value) -> Danger {
        Danger::Safe
    }
    fn summary(&self, args: &Value) -> String {
        match str_arg(args, "recipe") {
            Some(r) => format!("Рецепт: {r}"),
            None => "Сценарий из нескольких шагов".into(),
        }
    }
    fn run(&self, args: &Value) -> ToolResult {
        let graph = match (str_arg(args, "recipe"), args.get("graph")) {
            (Some(name), _) => synaptix_graph::recipes::find(&self.dir, name).ok_or_else(|| {
                let names: Vec<String> = synaptix_graph::recipes::list(&self.dir).into_iter().map(|g| g.name).collect();
                format!("рецепта «{name}» нет; есть: {}", names.join(", "))
            })?,
            (None, Some(g)) => serde_json::from_value(g.clone()).map_err(|e| format!("граф: {e}"))?,
            _ => return Err("нужен recipe или graph".into()),
        };
        let params: Map<String, Value> = args.get("params").and_then(Value::as_object).cloned().unwrap_or_default();
        let dir = crate::builtin::media::media_dir().join(format!("pipeline-{:x}", crate::chat::now()));
        let report = synaptix_graph::run(&graph, &params, &dir, &mut |_| {})?;
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
