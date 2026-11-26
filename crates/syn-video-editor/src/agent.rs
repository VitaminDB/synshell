//! Встроенный агент: каждая команда редактора — инструмент модели (схема из
//! таблицы команд). Один ход агента — одна группа отмены. Ядро — syn-assist,
//! модели — служба synaptixd.
//!
//! Всё настраивается на странице «Настройки» ([`crate::settings`]): модель и
//! режим, шаги и параметры генерации, режимы инструментов (вкл / пул / выкл),
//! навыки (инструкции SKILL.md — встроенные заготовки `library/agent-skills`
//! и свои), системный промпт и указания. Агент собирается заново на каждый
//! ход — правка настроек действует со следующего запроса.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use syn_assist::agent::{Agent, AgentEvent, Confirm, DaemonBackend};
use syn_assist::chat::Chat;
use syn_assist::tools::{Danger, Pooled, Tool, ToolOutput, ToolResult};

use crate::cmd::{self, Command};
use crate::editor::Editor;
use crate::settings::{self, Settings, ToolMode};

pub type Shared = Arc<Mutex<Editor>>;

/// Стандартный системный промпт: роль, порядок работы и правила. Подробности
/// по видам задач — в навыках (агент загружает их сам, когда задача подходит).
pub const SYSTEM: &str = "Ты — монтажёр во встроенном ИИ-видеоредакторе syn-video-editor. У тебя полный доступ к редактору через инструменты: каждый инструмент — команда программы (то же, что пользователь делает мышью и в меню). Работай сам до готового результата: импортируй медиа, ставь клипы, режь, добавляй титры, эффекты, переходы, цвет и звук. Не спрашивай разрешения на обычные правки — весь твой ход отменяется пользователем одним шагом.

## Порядок работы
1. Посмотри состояние: project_dump (таймлайн текстом; id: m — медиа, t — дорожки, c — клипы, e — эффекты).
2. Если задача подходит под навык (монтаж ролика, субтитры, цвет и LUT, моушн-графика, шаблоны Fusion, шаблоны проектов, Reels, музыка, видеоистория, граф кадра и блоки, графы сценариев, экспорт, подкаст) — загрузи его через autoskill и следуй ему.
3. Делай правки. Много однотипных (все клипы, каждая сцена) — одним script_run (сценарий Rhai), а не десятками вызовов.
4. Проверь итог: project_dump; если менялось видимое (титры, кадрирование, цвет, шаблоны) — frame_check 1–3 ключевых кадров (модель со зрением ответит, что на кадре и есть ли проблемы) и исправь найденное.
5. Коротко отчитайся: что сделано, что проверено на кадрах, где результат.

## Правила
- Время — секунды числом или «м:сс.ммм».
- Проект сохраняет программа сама. project_save с path, project_new и project_open — только если пользователь прямо просит другой файл или новый проект.
- Видео со звуком clip_add ставит связанной парой: картинка на V-дорожку, звук на A-дорожку (правки идут синхронно); clip_detach_audio разрывает связь.
- Готовые сценарии монтажа — skill_list / skill_run (reels, talking_head, podcast, beat_clip); стиль монтажа — по жанру: промо/Reels/клип → bright_montage, фильм/мультфильм/история → cinematic_montage, инструкция/интервью → calm_montage (неясно — спроси или calm_montage); найти момент в материале — media_search (после media_index).
- Всё, что есть в меню окна (вид, панели, масштаб просмотра, галереи), — ui_list и ui_run.
- Если чего-то сделать нельзя (нет инструмента) — так и скажи, не выдумывай.";

/// Инструмент-обёртка над командой.
struct CmdTool {
    cmd: &'static Command,
    name: &'static str,
    ed: Shared,
}

impl Tool for CmdTool {
    fn name(&self) -> &'static str {
        self.name
    }
    fn group(&self) -> &'static str {
        "editor"
    }
    fn description(&self) -> String {
        self.cmd.help.to_string()
    }
    fn parameters(&self) -> Value {
        self.cmd.schema()
    }
    fn danger(&self, _args: &Value) -> Danger {
        if self.cmd.danger {
            Danger::Confirm
        } else {
            Danger::Safe
        }
    }
    fn summary(&self, args: &Value) -> String {
        let a = args.as_object().filter(|o| !o.is_empty()).map(|o| Value::Object(o.clone()).to_string()).unwrap_or_default();
        format!("{} {a}", self.cmd.title)
    }
    fn when(&self) -> String {
        format!("{}: {}", self.cmd.category, self.cmd.title)
    }
    fn run(&self, args: &Value) -> ToolResult {
        // команды окна (графы панели «Граф», действия окна) идут в главный поток и
        // ждут его — без блокировки документа, иначе главный поток, ждущий её для
        // отрисовки, и агент ждали бы друг друга
        if self.cmd.id.starts_with("nodes.") || self.cmd.id.starts_with("ui.") {
            let mut scratch = crate::editor::Editor::new();
            return cmd::run_cmd(&mut scratch, self.cmd, args).map(|o| ToolOutput::text(o.text));
        }
        let mut ed = self.ed.lock().unwrap_or_else(|e| e.into_inner());
        cmd::run_cmd(&mut ed, self.cmd, args).map(|o| ToolOutput::text(o.text))
    }
}

/// Инструмент агента — для страницы настроек.
#[derive(Clone, Debug)]
pub struct ToolInfo {
    pub name: String,
    pub category: &'static str,
    pub title: &'static str,
    pub help: &'static str,
}

/// Все инструменты-команды по разделам (порядок таблицы команд).
pub fn catalog() -> Vec<ToolInfo> {
    cmd::all().into_iter().map(|c| ToolInfo { name: c.tool_name(), category: c.category, title: c.title, help: c.help }).collect()
}

/// Инструменты с учётом режимов: выключенные убраны, «пул» — через autotools.
pub fn tools(ed: &Shared, s: &Settings) -> Vec<Arc<dyn Tool>> {
    cmd::all()
        .into_iter()
        .filter_map(|c| {
            let name = c.tool_name();
            let mode = s.tool_mode(&name);
            if mode == ToolMode::Off {
                return None;
            }
            let name: &'static str = Box::leak(name.into_boxed_str());
            let t = Arc::new(CmdTool { cmd: c, name, ed: ed.clone() }) as Arc<dyn Tool>;
            Some(if mode == ToolMode::Pool { Arc::new(Pooled(t)) as Arc<dyn Tool> } else { t })
        })
        .collect()
}

/// Встроенные навыки-заготовки (`library/agent-skills/<имя>/SKILL.md`).
pub const BUILTIN_SKILLS: &[(&str, &str)] = &[
    ("montage", include_str!("../library/agent-skills/montage/SKILL.md")),
    ("subtitles", include_str!("../library/agent-skills/subtitles/SKILL.md")),
    ("color-grade", include_str!("../library/agent-skills/color-grade/SKILL.md")),
    ("motion-graphics", include_str!("../library/agent-skills/motion-graphics/SKILL.md")),
    ("fusion-templates", include_str!("../library/agent-skills/fusion-templates/SKILL.md")),
    ("project-templates", include_str!("../library/agent-skills/project-templates/SKILL.md")),
    ("reels", include_str!("../library/agent-skills/reels/SKILL.md")),
    ("music-for-video", include_str!("../library/agent-skills/music-for-video/SKILL.md")),
    ("video-story", include_str!("../library/agent-skills/video-story/SKILL.md")),
    ("frame-graph", include_str!("../library/agent-skills/frame-graph/SKILL.md")),
    ("flow-graphs", include_str!("../library/agent-skills/flow-graphs/SKILL.md")),
    ("export", include_str!("../library/agent-skills/export/SKILL.md")),
    ("podcast", include_str!("../library/agent-skills/podcast/SKILL.md")),
];

/// Каталог встроенных навыков на диске: заготовки из программы выкладываются
/// в кэш (syn-assist читает навыки из папок) и обновляются с программой.
pub fn builtin_skills_dir() -> PathBuf {
    let dir = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("syn-video-editor")
        .join("agent-skills");
    for (name, text) in BUILTIN_SKILLS {
        let d = dir.join(name);
        let f = d.join("SKILL.md");
        if std::fs::read_to_string(&f).ok().as_deref() != Some(*text) {
            let _ = std::fs::create_dir_all(&d);
            let _ = std::fs::write(&f, text);
        }
    }
    dir
}

/// Каталоги навыков по порядку: встроенные, свои (`~/.config/…/skills`),
/// добавленные в настройках; одноимённый поздний заменяет ранний.
pub fn skill_dirs(s: &Settings) -> Vec<PathBuf> {
    let mut v = vec![builtin_skills_dir(), settings::skills_dir()];
    v.extend(s.skill_dirs.iter().cloned());
    v
}

/// Встроенный ли навык (у своего с тем же именем — нет).
pub fn is_builtin_skill(k: &syn_assist::skills::Skill) -> bool {
    k.dir.starts_with(builtin_skills_dir())
}

/// Системный промпт из настроек: свой или стандартный.
pub fn base_prompt(s: &Settings) -> String {
    if s.prompt.trim().is_empty() {
        SYSTEM.to_string()
    } else {
        s.prompt.clone()
    }
}

pub fn agent(ed: &Shared) -> Agent {
    agent_with(ed, &settings::load())
}

/// Агент по настройкам.
pub fn agent_with(ed: &Shared, s: &Settings) -> Agent {
    let mut a = Agent::new(Arc::new(DaemonBackend), tools(ed, s));
    a.system = Some(base_prompt(s));
    a.subagent = s.ai_subagent;
    a.max_steps = s.ai_max_steps.max(1);
    a.compact_percent = if s.ai_autocompact { s.ai_compact_percent.clamp(50, 98) } else { 0 };
    a.params = s.gen_params();
    let mut a = a.with_skill_dirs(skill_dirs(s), s.disabled_skills.clone()).with_instructions(&s.instructions);
    // все команды правят проект с отменой; «спросить» без окна — отказ, поэтому
    // включённые инструменты разрешены
    let approved: Vec<String> = a.tools.iter().map(|t| t.name().to_string()).collect();
    a.approved = approved;
    a
}

/// Итоговый системный промпт и число инструментов (сразу / в пуле) — для предпросмотра.
pub fn full_prompt(s: &Settings) -> (String, usize, usize) {
    let ed: Shared = Arc::new(Mutex::new(Editor::new()));
    let a = agent_with(&ed, s);
    let pooled = a.tools.iter().filter(|t| t.pooled()).count();
    (a.system_prompt(), a.tools.len() - pooled, pooled)
}

/// Модель и режим размышлений из настроек — в разговор перед ходом.
pub fn prepare_chat(chat: &mut Chat, s: &Settings) {
    chat.model = s.ai_model.trim().to_string();
    chat.thinking = s.ai_thinking;
    chat.reasoning = s.ai_reasoning.clone();
}

/// Подтверждение необратимого без окна: CLI — отказ, если не разрешено явно.
pub struct AutoConfirm(pub bool);

impl Confirm for AutoConfirm {
    fn confirm(&self, _tool: &str, _summary: &str) -> bool {
        self.0
    }
}

/// Ход агента: весь ход — одна группа отмены «ИИ: …».
pub fn run_turn(agent: &Agent, ed: &Shared, chat: &mut Chat, text: &str, confirm: &dyn Confirm, cancel: &AtomicBool, on: &mut dyn FnMut(AgentEvent)) -> Result<(), String> {
    let label: String = text.chars().take(40).collect();
    ed.lock().unwrap_or_else(|e| e.into_inner()).begin_group(&format!("ИИ: {label}"));
    let r = agent.run_turn(chat, text, confirm, cancel, on);
    ed.lock().unwrap_or_else(|e| e.into_inner()).end_group();
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_skills_parse_and_modes_apply() {
        for (name, text) in BUILTIN_SKILLS {
            assert_eq!(syn_assist::skills::front(text, "name").as_deref(), Some(*name), "имя навыка {name}");
            assert!(syn_assist::skills::front(text, "description").is_some_and(|d| d.len() > 40), "описание {name}");
        }
        let mut s = Settings::default();
        s.tool_modes.insert("clip_add".into(), ToolMode::Off);
        s.disabled_skills.push("reels".into());
        let ed: Shared = Arc::new(Mutex::new(Editor::new()));
        let a = agent_with(&ed, &s);
        assert!(!a.tools.iter().any(|t| t.name() == "clip_add"));
        assert!(a.tools.iter().any(|t| t.name() == "font_list" && t.pooled()));
        assert!(a.tools.iter().any(|t| t.name() == "autoskill"));
        assert!(a.skills.iter().any(|k| k.name == "montage"));
        assert!(!a.skills.iter().any(|k| k.name == "reels"));
        let p = a.system_prompt();
        assert!(p.contains("монтажёр") && !p.contains("навыки models"));
    }
}
