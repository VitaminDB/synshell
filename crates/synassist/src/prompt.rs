use std::sync::Arc;

use crate::tools::Tool;

fn today() -> String {
    let out = std::process::Command::new("date").arg("+%Y-%m-%d, %A").output();
    out.ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_default()
}

fn user_name() -> String {
    std::env::var("USER").unwrap_or_default()
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
    s.push_str("- Свежие факты, новости, цены и погоду ищи в интернете.\n\n");
    s.push_str(&format!("Пользователь: {}. Домашняя папка: {home}. Сегодня: {}.\n", user_name(), today()));
    if !groups.is_empty() {
        s.push_str(&format!("Доступные группы инструментов: {}.\n", groups.join(", ")));
    }
    s
}
