//! Сообщения пользователя: команды (`/help`, `/to` …) или разговор с Claude.

use std::sync::Arc;

use syn_courier::{ChannelState, NotifyMode};

use crate::hub::{Hub, Incoming};

const HELP: &str = "*Курьер* — связь с компьютером.\n\
Просто напишите — ответит Claude Code (сеанс «courier»).\n\
/to ИМЯ ТЕКСТ — передать живому сеансу Claude (ответ придёт сюда)\n\
/sessions — живые сеансы Claude\n\
/new [ПАПКА] — новый разговор (можно в другой папке)\n\
/stop — прервать текущий ответ\n\
/status — состояние\n\
/notify claude|away|all — какие уведомления присылать\n\
/route last|whatsapp|telegram|all — куда отвечать, если подключены оба мессенджера\n\
/help — эта подсказка";

/// Живой сеанс Claude из `claude agents --json`.
#[derive(Debug, Clone, PartialEq)]
pub struct Agent {
    pub name: String,
    pub status: String,
    pub cwd: String,
}

pub fn parse_agents(json: &str) -> Vec<Agent> {
    let Ok(v) = serde_json::from_str::<Vec<serde_json::Value>>(json) else { return Vec::new() };
    v.iter()
        .map(|a| Agent {
            name: a.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            status: a.get("status").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            cwd: a.get("cwd").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        })
        .filter(|a| !a.name.is_empty())
        .collect()
}

async fn agents(program: &str) -> Result<Vec<Agent>, String> {
    let out = tokio::process::Command::new(crate::claude::resolve_program(program)).args(["agents", "--json"]).output().await.map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(parse_agents(&String::from_utf8_lossy(&out.stdout)))
}

fn short_path(p: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    match p.strip_prefix(&home) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => p.to_string(),
    }
}

/// `/to ИМЯ ТЕКСТ` → поручение телефонному сеансу.
pub fn to_prompt(name: &str, text: &str) -> String {
    format!("Передай сеансу «{name}» через SendMessage (to: \"{name}\") сообщение пользователя дословно:\n\n{text}\n\nКогда он ответит — перескажи ответ пользователю.")
}

pub async fn handle(hub: &Arc<Hub>, m: Incoming) {
    tracing::debug!("сообщение из «{}»", m.channel);
    let text = m.text.trim();
    let (cmd, rest) = match text.strip_prefix('/') {
        Some(t) => {
            let mut it = t.splitn(2, char::is_whitespace);
            (it.next().unwrap_or("").to_lowercase(), it.next().unwrap_or("").trim().to_string())
        }
        None => (String::new(), String::new()),
    };
    let program = hub.config().claude.program;
    match cmd.as_str() {
        "" => ask_claude(hub, text).await,
        "help" | "start" | "помощь" => hub.say(HELP).await,
        "status" => {
            let s = hub.snapshot();
            let mut lines = vec!["*Курьер*".to_string()];
            for c in &s.channels {
                let st = match c.state {
                    ChannelState::Online => "подключён",
                    ChannelState::Linking => "ждёт QR",
                    ChannelState::Connecting => "подключается",
                    ChannelState::Unlinked => "не привязан",
                    ChannelState::Off => "выключен",
                    ChannelState::Error => "ошибка",
                };
                lines.push(format!("{}: {st}{}", c.title, c.account.as_ref().map(|a| format!(" · {a}")).unwrap_or_default()));
            }
            let c = &s.claude;
            lines.push(format!(
                "Claude «{}»: {} · {}",
                c.name,
                if c.busy { "думает" } else if c.running { "ждёт" } else { "не запущен" },
                short_path(&syn_courier::expand_home(&c.cwd).to_string_lossy())
            ));
            lines.push(format!(
                "Уведомления: {}",
                match s.notify {
                    NotifyMode::Claude => "только Claude",
                    NotifyMode::Away => "все, когда меня нет",
                    NotifyMode::All => "все",
                }
            ));
            hub.say(&lines.join("\n")).await;
        }
        "sessions" | "agents" => match agents(&program).await {
            Ok(list) if list.is_empty() => hub.say("Живых сеансов Claude нет.").await,
            Ok(list) => {
                let mut lines = vec!["*Сеансы Claude*".to_string()];
                for a in list {
                    let st = match a.status.as_str() {
                        "busy" => "занят",
                        "idle" => "свободен",
                        s => s,
                    };
                    lines.push(format!("• {} — {st} — {}", a.name, short_path(&a.cwd)));
                }
                lines.push("Написать: /to ИМЯ текст".into());
                hub.say(&lines.join("\n")).await;
            }
            Err(e) => hub.say(&format!("Не получил список сеансов: {e}")).await,
        },
        "to" => {
            let mut it = rest.splitn(2, char::is_whitespace);
            let (name, body) = (it.next().unwrap_or("").trim(), it.next().unwrap_or("").trim());
            if name.is_empty() || body.is_empty() {
                hub.say("Формат: /to ИМЯ текст (имена — /sessions)").await;
                return;
            }
            // Имя проверить заранее: опечатка — подсказка, а не ход Claude впустую.
            if let Ok(list) = agents(&program).await {
                if !list.iter().any(|a| a.name == name) {
                    let near: Vec<String> = list.iter().filter(|a| a.name.contains(name)).map(|a| a.name.clone()).collect();
                    let hint = if near.is_empty() { "Список: /sessions".to_string() } else { format!("Может быть: {}", near.join(", ")) };
                    hub.say(&format!("Сеанса «{name}» нет. {hint}")).await;
                    return;
                }
            }
            ask_claude(hub, &to_prompt(name, body)).await;
        }
        "new" => {
            let dir = (!rest.is_empty()).then(|| rest.clone());
            if let Some(d) = &dir {
                if !syn_courier::expand_home(d).is_dir() {
                    hub.say(&format!("Нет такой папки: {d}")).await;
                    return;
                }
            }
            hub.claude.reset(dir).await;
            let cwd = hub.claude.status().cwd;
            hub.say(&format!("Новый разговор · {}", short_path(&syn_courier::expand_home(&cwd).to_string_lossy()))).await;
        }
        "stop" => {
            let was = hub.claude.status().busy;
            hub.claude.stop().await;
            hub.say(if was { "Остановлено. Разговор сохранён." } else { "Claude сейчас ничего не делает." }).await;
        }
        "route" => {
            use syn_courier::Delivery;
            let d = match rest.as_str() {
                "last" => Some(Delivery::Last),
                "whatsapp" | "wa" => Some(Delivery::Whatsapp),
                "telegram" | "tg" => Some(Delivery::Telegram),
                "all" | "both" => Some(Delivery::All),
                _ => None,
            };
            match d {
                Some(d) => {
                    let mut c = hub.config();
                    c.delivery = d;
                    let r = c.save();
                    *hub.cfg.write().unwrap() = c;
                    hub.say(match (r, d) {
                        (Err(e), _) => format!("Не сохранилось: {e}"),
                        (_, Delivery::Last) => "Отвечаю туда, откуда вы написали.".into(),
                        (_, Delivery::Whatsapp) => "Основной — WhatsApp.".into(),
                        (_, Delivery::Telegram) => "Основной — Telegram.".into(),
                        (_, Delivery::All) => "Отправляю в оба мессенджера.".into(),
                    }
                    .as_str())
                    .await;
                }
                None => hub.say("Формат: /route last | whatsapp | telegram | all").await,
            }
        }
        "notify" => {
            let mode = match rest.as_str() {
                "claude" => Some(NotifyMode::Claude),
                "away" => Some(NotifyMode::Away),
                "all" => Some(NotifyMode::All),
                _ => None,
            };
            match mode {
                Some(mode) => {
                    let mut c = hub.config();
                    c.notify.mode = mode;
                    let r = c.save();
                    *hub.cfg.write().unwrap() = c;
                    hub.say(match (r, mode) {
                        (Err(e), _) => format!("Не сохранилось: {e}"),
                        (_, NotifyMode::Claude) => "Буду присылать только события Claude.".into(),
                        (_, NotifyMode::Away) => "Буду присылать все уведомления, когда вас нет за компьютером.".into(),
                        (_, NotifyMode::All) => "Буду присылать все уведомления.".into(),
                    }
                    .as_str())
                    .await;
                }
                None => hub.say("Формат: /notify claude | away | all").await,
            }
        }
        _ => hub.say(&format!("Не знаю команды /{cmd}. /help — список.")).await,
    }
}

async fn ask_claude(hub: &Arc<Hub>, text: &str) {
    if !hub.claude.enabled() {
        hub.say("Разговор с Claude выключен в настройках Курьера.").await;
        return;
    }
    if let Err(e) = hub.claude.send(text).await {
        hub.say(&format!("⚠️ Claude: {e:#}")).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_json() {
        let j = r#"[{"pid":1,"cwd":"/home/u/p","kind":"interactive","name":"hexspire-b0","status":"idle"},{"pid":2}]"#;
        let a = parse_agents(j);
        assert_eq!(a, vec![Agent { name: "hexspire-b0".into(), status: "idle".into(), cwd: "/home/u/p".into() }]);
        assert!(parse_agents("не json").is_empty());
        assert!(to_prompt("x", "привет").contains("(to: \"x\")"));
    }
}
