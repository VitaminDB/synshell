//! Настройки ИИ видеоредактора: агент (модель, режим, шаги, параметры
//! генерации), инструменты (вкл / пул / выкл), навыки, системный промпт и
//! модели по ролям — кто смотрит кадр, распознаёт речь, пишет музыку и т. д.
//! Файл — `~/.config/syn-video-editor/settings.toml` (учитывает
//! `XDG_CONFIG_HOME`); пишется сразу при каждой правке на странице «Настройки».
//!
//! Пустая строка модели везде значит «по умолчанию»: решает служба synaptix
//! (её основная модель нужного вида) или сама нода графа.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Режим инструмента агента.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolMode {
    /// Схема инструмента сразу в запросе.
    On,
    /// В пуле `autotools`: модель видит строку каталога и подгружает схему по нужде.
    Pool,
    /// Модель инструмента не видит.
    Off,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Модель агента (LLM службы); пусто — основная LLM службы.
    pub ai_model: String,
    /// Режим «Думать» (рассуждения перед ответом).
    pub ai_thinking: bool,
    /// Глубина рассуждений (`low`, `medium`, `high`…); пусто — авто.
    pub ai_reasoning: String,
    /// Шагов (вызовов модели) за один запрос.
    pub ai_max_steps: usize,
    /// Пересказывать старую историю, когда контекст заполнен.
    pub ai_autocompact: bool,
    /// Порог сжатия истории, % окна контекста.
    pub ai_compact_percent: u32,
    /// Инструмент `subagent` — узкие подзадачи отдельным агентом.
    pub ai_subagent: bool,
    /// Температура генерации; 0 — как настроено у модели в службе.
    pub ai_temperature: f32,
    /// top_p; 0 — как у модели.
    pub ai_top_p: f32,
    /// Предел длины одного ответа модели, токенов; 0 — как у модели.
    pub ai_max_tokens: u32,
    /// Режимы инструментов, отличные от умолчания (см. [`default_mode`]).
    pub tool_modes: BTreeMap<String, ToolMode>,
    /// Отключённые навыки (по имени): инструкции SKILL.md и сценарии Rhai.
    pub disabled_skills: Vec<String>,
    /// Дополнительные каталоги навыков (папка на навык с SKILL.md).
    pub skill_dirs: Vec<PathBuf>,
    /// Свой системный промпт; пусто — стандартный ([`crate::agent::SYSTEM`]).
    pub prompt: String,
    /// Указания пользователя — дописываются в конец промпта.
    pub instructions: String,
    /// Модели по ролям ([`ROLES`]): роль → id модели службы; пусто — по умолчанию.
    pub models: BTreeMap<String, String>,
    /// Модели нод графов по видам (`music`, `image_model`…), поверх ролей.
    pub node_models: BTreeMap<String, String>,
    /// Ширина кадра для модели зрения, px.
    pub vision_width: u32,
    /// Длина ответа модели зрения, токенов.
    pub vision_max_tokens: u32,
    /// Температура модели зрения.
    pub vision_temperature: f32,
    /// Язык распознавания речи (`ru`, `en`…); пусто — определить самому.
    pub asr_language: String,
    /// При запуске без файла — открыть последний проект; по умолчанию новый.
    pub open_last_project: bool,
    /// LoRA видео MiniMax-H3 для съёмки (ref-scenes, story-video): пусто или «off» —
    /// без LoRA (полные шаги, по умолчанию), иначе файл (Turbo — только явным выбором).
    pub video_lora: String,
    /// Съёмочная группа: свой системный промпт роли (id роли → текст); нет — по умолчанию.
    pub crew_prompts: BTreeMap<String, String>,
    /// Съёмочная группа: модель роли (id роли → модель службы); нет — модель агента.
    pub crew_models: BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ai_model: String::new(),
            ai_thinking: false,
            ai_reasoning: String::new(),
            ai_max_steps: 60,
            ai_autocompact: true,
            ai_compact_percent: 85,
            ai_subagent: false,
            ai_temperature: 0.0,
            ai_top_p: 0.0,
            ai_max_tokens: 0,
            tool_modes: BTreeMap::new(),
            disabled_skills: Vec::new(),
            skill_dirs: Vec::new(),
            prompt: String::new(),
            instructions: String::new(),
            models: BTreeMap::new(),
            node_models: BTreeMap::new(),
            video_lora: String::new(),
            crew_prompts: BTreeMap::new(),
            crew_models: BTreeMap::new(),
            vision_width: 768,
            vision_max_tokens: 700,
            vision_temperature: 0.2,
            asr_language: String::new(),
            open_last_project: false,
        }
    }
}

/// Вид модели службы для роли (что показывать в списке выбора).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoleKind {
    Llm,
    /// LLM со зрением (картинки на входе).
    Vision,
    Asr,
    Tts,
    Music,
    Image,
    Video,
    Embed,
    Diarize,
    /// Покадровое нейроувеличение (vfx-пакеты .syn).
    Upscale,
    /// Межкадровая интерполяция RIFE (vfx-пакеты .syn).
    Rife,
}

/// Роль модели: ключ в настройках, ключи перевода названия и пояснения,
/// вид модели, виды нод графов, которые берут эту модель, если у них пусто.
pub struct Role {
    pub key: &'static str,
    pub title: &'static str,
    pub hint: &'static str,
    pub kind: RoleKind,
    pub nodes: &'static [&'static str],
}

pub const ROLES: &[Role] = &[
    Role { key: "vision", title: "settings.role.vision", hint: "settings.role.vision.hint", kind: RoleKind::Vision, nodes: &[] },
    Role { key: "llm_nodes", title: "settings.role.llm_nodes", hint: "settings.role.llm_nodes.hint", kind: RoleKind::Llm, nodes: &["llm"] },
    Role { key: "asr", title: "settings.role.asr", hint: "settings.role.asr.hint", kind: RoleKind::Asr, nodes: &["transcribe"] },
    Role { key: "diarize", title: "settings.role.diarize", hint: "settings.role.diarize.hint", kind: RoleKind::Diarize, nodes: &["diarize"] },
    Role { key: "tts", title: "settings.role.tts", hint: "settings.role.tts.hint", kind: RoleKind::Tts, nodes: &["speak"] },
    Role { key: "music", title: "settings.role.music", hint: "settings.role.music.hint", kind: RoleKind::Music, nodes: &["music"] },
    Role { key: "image", title: "settings.role.image", hint: "settings.role.image.hint", kind: RoleKind::Image, nodes: &["image"] },
    Role { key: "video", title: "settings.role.video", hint: "settings.role.video.hint", kind: RoleKind::Video, nodes: &["video"] },
    Role { key: "embed", title: "settings.role.embed", hint: "settings.role.embed.hint", kind: RoleKind::Embed, nodes: &[] },
    Role { key: "upscale", title: "settings.role.upscale", hint: "settings.role.upscale.hint", kind: RoleKind::Upscale, nodes: &["vfx_upscale"] },
    Role { key: "rife", title: "settings.role.rife", hint: "settings.role.rife.hint", kind: RoleKind::Rife, nodes: &[] },
];

impl Settings {
    /// Модель роли; пусто — по умолчанию.
    pub fn model(&self, role: &str) -> String {
        self.models.get(role).map(|m| m.trim().to_string()).unwrap_or_default()
    }

    /// То же, `None` вместо пустого.
    pub fn model_opt(&self, role: &str) -> Option<String> {
        Some(self.model(role)).filter(|m| !m.is_empty())
    }

    pub fn tool_mode(&self, tool: &str) -> ToolMode {
        self.tool_modes.get(tool).copied().unwrap_or_else(|| default_mode(tool))
    }

    pub fn skill_on(&self, name: &str) -> bool {
        !self.disabled_skills.iter().any(|d| d == name)
    }

    /// Модели нод графов по видам: сначала роли, поверх — явные по видам.
    pub fn flow_models(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        for r in ROLES {
            let id = self.model(r.key);
            if !id.is_empty() {
                for k in r.nodes {
                    m.insert(k.to_string(), id.clone());
                }
            }
        }
        for (k, v) in &self.node_models {
            if !v.trim().is_empty() {
                m.insert(k.clone(), v.trim().to_string());
            }
        }
        m
    }

    /// Параметры генерации агента поверх настроек модели в службе.
    pub fn gen_params(&self) -> synaptixd::Params {
        let mut p = synaptixd::Params::new();
        if self.ai_temperature > 0.0 {
            p = p.with("temperature", self.ai_temperature as f64);
        }
        if self.ai_top_p > 0.0 {
            p = p.with("top_p", self.ai_top_p as f64);
        }
        if self.ai_max_tokens > 0 {
            p = p.with("max_tokens", self.ai_max_tokens as i64);
        }
        p
    }
}

/// Инструменты, которые по умолчанию в пуле: редкие, служебные или узкие —
/// их схемы не занимают контекст, агент подгружает их через `autotools`.
const POOLED: &[&str] = &[
    "template_inspect",
    "template_coverage",
    "template_params",
    "font_list",
    "key_move",
    "key_list",
    "track_move",
    "clip_copy",
    "clip_paste",
    "clip_shift",
    "graph_link",
    "graph_unlink",
    "graph_set",
    "block_save",
    "block_set",
    "slot_add",
    "slot_set",
    "slot_clear",
    "template_project_save",
    "media_transcript",
    "media_silences",
    "frame_export",
    "playhead_set",
    "selection_set",
    "edit_redo",
];

/// Режим инструмента по умолчанию.
pub fn default_mode(tool: &str) -> ToolMode {
    if POOLED.contains(&tool) {
        ToolMode::Pool
    } else {
        ToolMode::On
    }
}

pub fn path() -> PathBuf {
    crate::layout::config_dir().join("settings.toml")
}

/// Каталог своих навыков (SKILL.md в подпапках и сценарии `*.rhai`).
pub fn skills_dir() -> PathBuf {
    crate::layout::config_dir().join("skills")
}

static CACHE: Mutex<Option<Settings>> = Mutex::new(None);
static REV: AtomicU64 = AtomicU64::new(0);

/// Номер правки настроек — растёт при каждом сохранении (агент и окно
/// перечитывают по нему).
pub fn rev() -> u64 {
    REV.load(Ordering::Relaxed)
}

/// Текущие настройки (с диска при первом обращении; битый файл — умолчания).
/// Модель агента только для этого процесса (CLI `--model`, проверки): в
/// настройки на диске не пишется, действует и на роли группы без своей модели.
pub fn override_model(model: &str) {
    let mut s = load();
    s.ai_model = model.trim().to_string();
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
}

pub fn load() -> Settings {
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = c.as_ref() {
        return s.clone();
    }
    let s = match std::fs::read_to_string(path()) {
        Ok(t) => toml::from_str(&t).unwrap_or_else(|e| {
            tracing::warn!("{}: {e} — настройки по умолчанию", path().display());
            Settings::default()
        }),
        Err(_) => Settings::default(),
    };
    *c = Some(s.clone());
    s
}

/// Изменить и сохранить (только если что-то поменялось); применить к графам.
pub fn update(f: impl FnOnce(&mut Settings)) {
    let mut s = load();
    let before = s.clone();
    f(&mut s);
    if s == before {
        return;
    }
    match toml::to_string_pretty(&s) {
        Ok(text) => {
            if let Err(e) = crate::layout::write_atomic(&path(), &text) {
                tracing::warn!("настройки не записаны: {e}");
            }
        }
        Err(e) => tracing::warn!("настройки не записаны: {e}"),
    }
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
    REV.fetch_add(1, Ordering::Relaxed);
    apply(&s);
}

/// Применить то, что живёт вне агента: модели нод графов.
pub fn apply(s: &Settings) {
    synaptix_flow::defaults::set_models(s.flow_models());
}

/// Перечитать файл (правка снаружи, проверки).
pub fn reload() -> Settings {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let s = load();
    REV.fetch_add(1, Ordering::Relaxed);
    apply(&s);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_feed_flow_nodes_and_node_overrides_win() {
        let mut s = Settings::default();
        s.models.insert("music".into(), "ace-step".into());
        s.models.insert("tts".into(), " ".into());
        s.node_models.insert("speak".into(), "vibevoice".into());
        s.node_models.insert("music".into(), "yue2".into());
        let m = s.flow_models();
        assert_eq!(m.get("music").map(String::as_str), Some("yue2"));
        assert_eq!(m.get("speak").map(String::as_str), Some("vibevoice"));
        assert!(!m.contains_key("image"));
    }

    #[test]
    fn toml_roundtrip_keeps_defaults_for_missing_keys() {
        let s: Settings = toml::from_str("ai_model = \"qwen\"\n[tool_modes]\nclip_add = \"off\"\n").unwrap();
        assert_eq!(s.ai_model, "qwen");
        assert_eq!(s.ai_max_steps, 60);
        assert_eq!(s.tool_mode("clip_add"), ToolMode::Off);
        assert_eq!(s.tool_mode("font_list"), ToolMode::Pool);
        assert_eq!(s.tool_mode("title_add"), ToolMode::On);
        let back: Settings = toml::from_str(&toml::to_string_pretty(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
