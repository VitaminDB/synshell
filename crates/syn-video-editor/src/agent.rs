//! Встроенный агент: каждая команда редактора — инструмент модели (схема из
//! таблицы команд). Один ход агента — одна группа отмены. Ядро — syn-assist,
//! модели — служба synaptixd.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use syn_assist::agent::{Agent, AgentEvent, Confirm, DaemonBackend};
use syn_assist::chat::Chat;
use syn_assist::tools::{Danger, Tool, ToolOutput, ToolResult};

use crate::cmd::{self, Command};
use crate::editor::Editor;

pub type Shared = Arc<Mutex<Editor>>;

const SYSTEM: &str = "Ты — монтажёр во встроенном ИИ-видеоредакторе syn-video-editor. У тебя полный доступ к редактору через инструменты: каждый инструмент — команда программы (то же, что делает пользователь мышью). Работай сам до готового результата: импортируй медиа, ставь клипы, режь, добавляй титры, фигуры, анимацию и эффекты. Пользователь потом посмотрит и поправит — не спрашивай разрешения на обычные правки, всё отменяется одним шагом.\n\nВозможности моушн-графики:\n- Ключи и кривые: key_set / key_move / key_remove / key_clear (transform, mask, color, blur, vignette, sharpen, lut, style титра и фигуры; сглаживание linear/ease/in/out/back/elastic/bounce/hold/bezier).\n- Пресеты движения: motion_list и motion_apply (вход, движение по клипу, выход — ставят ключи transform).\n- Анимация титров: title_add / clip_set со style {by: all|char|word|line, in/out: fade|slide|pop|scatter|type|none, anim, stagger, ease, font}.\n- Слои: shape_add (rect, ellipse, triangle, star, line, arrow), effect_add kind=mask (rect/ellipse, feather, invert), effect_add kind=glass (матовое стекло / glassmorphism для rect/ellipse: blur, tint, tint_opacity, border_opacity, highlight, shadow, saturation, brightness), clip_set с blend (normal/multiply/screen/overlay/add/darken/lighten/soft_light/hard_light/difference) и parent (привязка потомка к родителю).\n- Шаблоны Fusion (.setting): панель «Шаблоны» (галерея с поиском, фильтрами по разделам, живым превью при наведении и переносом на таймлайн), template_list (поиск), template_inspect (состав графа, тексты, параметры MacroOperator), template_coverage (отчёт покрытия библиотеки), template_apply (нативный исполнитель графа Fusion; имя или точный path к .setting, свои строки через text с \\n), template_params (список параметров MacroOperator наложенного клипа с текущими значениями), template_set (правка параметров шаблона: тексты, шрифты, цвета, градиенты, положение, масштаб, reset: true; числовые и цветовые параметры шаблона также анимируются через key_set с param=tpl.<ключ> или style.p:<ключ>).\n- Переходы Fusion (.setting из раздела «Переходы»): transition_list (список встроенных и Fusion-переходов по запросу), transition_set с kind=fusion и template (имя или путь к .setting, либо kind=fusion:<имя>), duration, params ({ключ: значение} для MacroOperator) и reset: true — нативный рендер A→B между стыкующимися клипами в просмотре и экспорте; переходы также перетаскиваются из галереи «Шаблоны → Переходы» прямо на склейку таймлайна.\n- Настройки проекта: project_settings поддерживает произвольные чётные width и height (16–16384), fps и цвет фона кадра bg (#rrggbb или transparent).
- Звук видео: clip_add для видео со звуком автоматически ставит связанную пару — видео на V-дорожку и звук с волной на A-дорожку (правки move/trim/split/remove/speed/cut_silence/cut_ranges идут синхронно); clip_detach_audio разрывает связь (или отделяет встроенный звук).\n\nПравила:\n- Перед правкой смотри состояние: project_dump (таймлайн текстом, id: m — медиа, t — дорожки, c — клипы, e — эффекты).\n- Время — секунды числом или «м:сс.ммм».\n- Проект сохраняет программа сама. project_save с path и project_new/project_open — только если пользователь прямо просит другой файл или новый проект.\n- После серии правок проверь итог через project_dump. Если менялось видимое (титры, кадрирование, положение, цвет, шаблоны) — посмотри 1–3 ключевых кадра через frame_check (модель со зрением ответит, что на кадре и есть ли проблемы) и исправь найденное. Коротко отчитайся: что сделано, что проверено на кадрах, где результат.
- Много однотипных правок (все клипы, каждая сцена) — одним script_run (сценарий Rhai), а не десятками вызовов.
- Типовые задачи монтажа — готовыми навыками: skill_run name=reels (Reels/Shorts 9:16 с субтитрами), talking_head (говорящая голова), podcast (подкаст с подписями дикторов), beat_clip (клип под музыку по битам); список и параметры — skill_list. Найти момент в материале — media_search (после media_index).\n- Если чего-то сделать нельзя (нет инструмента) — так и скажи, не выдумывай.";

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
    fn run(&self, args: &Value) -> ToolResult {
        let mut ed = self.ed.lock().unwrap_or_else(|e| e.into_inner());
        cmd::run_cmd(&mut ed, self.cmd, args).map(|o| ToolOutput::text(o.text))
    }
}

pub fn tools(ed: &Shared) -> Vec<Arc<dyn Tool>> {
    cmd::all()
        .into_iter()
        .map(|c| {
            let name: &'static str = Box::leak(c.tool_name().into_boxed_str());
            Arc::new(CmdTool { cmd: c, name, ed: ed.clone() }) as Arc<dyn Tool>
        })
        .collect()
}

pub fn agent(ed: &Shared) -> Agent {
    let mut a = Agent::new(Arc::new(DaemonBackend), tools(ed));
    a.system = Some(SYSTEM.to_string());
    a.subagent = false;
    a.max_steps = 60;
    a
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
