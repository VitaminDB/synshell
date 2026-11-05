use std::sync::Arc;

use crate::chat::{ChatMsg, Role};
use crate::tools::Tool;

fn today() -> String {
    let out = std::process::Command::new("date").arg("+%Y-%m-%d, %A").output();
    out.ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_default()
}

fn user_name() -> String {
    std::env::var("USER").unwrap_or_default()
}

pub const VOICE_NOTE: &str = "\nРежим голосового общения: твой ответ будет озвучен вслух. Отвечай коротко и разговорно, одной-тремя фразами; без markdown, списков, таблиц, кода и ссылок; числа и единицы — словами, как в живой речи. Перед долгим действием (видео, музыка, картинка, поиск в интернете) сначала скажи одну короткую фразу, что начинаешь (например «Сейчас сделаю, это займёт пару минут»), и в том же ответе вызови инструмент — это важнее правила «перед вызовом инструмента ничего не пиши».\n";

/// Фраза, которую голосовой режим говорит сразу, пока идут инструменты:
/// реплика модели перед их вызовом, а если её нет и инструмент долгий
/// (генерация медиа) — `fallback`.
pub fn before_tools(m: &ChatMsg, fallback: &str) -> Option<String> {
    if m.role != Role::Assistant || m.tool_calls.is_empty() {
        return None;
    }
    let text = m.text.trim();
    if !text.is_empty() {
        return Some(text.to_string());
    }
    let long = m.tool_calls.iter().any(|c| {
        c.name == "media" && serde_json::from_str::<serde_json::Value>(&c.arguments).ok().and_then(|a| a.get("action")?.as_str().map(|s| s != "speak")).unwrap_or(false)
    });
    long.then(|| fallback.to_string())
}

/// Итоговый ответ хода для озвучки: реплики модели без тех, что шли перед
/// вызовами инструментов (они уже сказаны по ходу).
pub fn final_answer(messages: &[ChatMsg]) -> String {
    messages
        .iter()
        .filter(|m| m.role == Role::Assistant && m.tool_calls.is_empty() && !m.text.trim().is_empty())
        .map(|m| m.text.trim().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn build_voice(tools: &[Arc<dyn Tool>]) -> String {
    let mut s = build(tools);
    s.push_str(VOICE_NOTE);
    s
}

pub fn build(tools: &[Arc<dyn Tool>]) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut groups: Vec<&str> = tools.iter().map(|t| t.group()).collect();
    groups.dedup();
    let mut s = String::new();
    s.push_str("Ты — ассистент, встроенный в операционную систему synshell (Linux). Ты помогаешь пользователю на его компьютере или телефоне: отвечаешь на вопросы, управляешь системой, работаешь с файлами, ищешь в интернете и создаёшь медиа.\n\n");
    s.push_str("Правила:\n");
    s.push_str("- Отвечай на языке пользователя, по делу и кратко; markdown можно.\n");
    s.push_str("- Если задачу можно решить инструментом — вызывай инструмент, а не объясняй, как сделать вручную.\n");
    s.push_str("- Перед вызовом инструмента ничего не пиши; после результата коротко скажи, что сделано.\n");
    s.push_str("- Не выдумывай факты о системе и файлах — сначала посмотри инструментом.\n");
    s.push_str("- Опасные действия (удаление, запись файлов, команды терминала, выключение) пользователь подтверждает сам — просто вызывай инструмент.\n");
    s.push_str("- Свежие факты, новости, цены и погоду ищи в интернете.\n");
    s.push_str("- Отвечай пользователю обычным текстом, никогда не JSON. Созданные файлы (картинки, звук, видео) пользователь видит сам под ответом — не пиши их пути и ссылки, просто коротко скажи, что получилось.\n\n");
    s.push_str(&format!("Пользователь: {}. Домашняя папка: {home}. Сегодня: {}.\n", user_name(), today()));
    if !groups.is_empty() {
        s.push_str(&format!("Доступные группы инструментов: {}.\n", groups.join(", ")));
    }
    s
}
