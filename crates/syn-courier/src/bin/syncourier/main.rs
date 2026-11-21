//! `syncourier` — служба Курьера и её командная строка.
//!
//! ```text
//! syncourier                 служба (systemd --user: syncourier.service)
//! syncourier status          состояние каналов и сеанса Claude
//! syncourier link            привязать WhatsApp: QR-код в терминале
//! syncourier unlink          отвязать WhatsApp
//! syncourier hook           хук Claude Code (Stop, Notification): JSON события на входе
//! syncourier ask ТЕКСТ      сообщение как от пользователя (проверка без телефона)
//! syncourier notify ТЕКСТ [--title З] [--source claude]
//!                            уведомление (хуки Claude Code, claude-call-user, скрипты)
//! ```
//!
//! Переменные для проверки: `SYNCOURIER_SOCKET`, `SYNCOURIER_DATA_DIR`,
//! `SYNSHELL_CONFIG_DIR`.

mod claude;
mod hub;
mod notify;
mod router;
mod wa;

use std::sync::Arc;

use syn_courier::{ChannelState, Request};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        None | Some("daemon") => run_daemon(),
        Some("status") => cli_status(),
        Some("link") => cli_link(),
        Some("unlink") => simple(Request::Unlink { channel: "whatsapp".into() }),
        Some("reload") => simple(Request::Reload),
        Some("notify") => cli_notify(&args[1..]),
        Some("hook") => cli_hook(),
        Some("ask") => simple(Request::Ask { text: args[1..].join(" ") }),
        Some("-h" | "--help" | "help") => {
            println!("syncourier [status | link | unlink | reload | notify ТЕКСТ [--title З] [--source claude]]");
            0
        }
        Some(other) => {
            eprintln!("неизвестная команда «{other}» (syncourier --help)");
            2
        }
    };
    std::process::exit(code);
}

fn simple(req: Request) -> i32 {
    match syn_courier::request(&req) {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cli_status() -> i32 {
    match syn_courier::status() {
        Ok(s) => {
            println!("{}", serde_json::to_string_pretty(&s).unwrap_or_default());
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cli_notify(args: &[String]) -> i32 {
    let mut title = String::new();
    let mut source = String::new();
    let mut body = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--title" => title = it.next().cloned().unwrap_or_default(),
            "--source" => source = it.next().cloned().unwrap_or_default(),
            _ => body.push(a.clone()),
        }
    }
    let mut body = body.join(" ");
    if body.is_empty() {
        use std::io::Read;
        let _ = std::io::stdin().read_to_string(&mut body);
    }
    simple(Request::Notify { title, body: body.trim().to_string(), source })
}

/// Хук Claude Code: событие (JSON на входе) → уведомление. Никогда не мешает Claude: код 0.
fn cli_hook() -> i32 {
    use std::io::Read;
    let mut input = String::new();
    let _ = std::io::stdin().take(1 << 20).read_to_string(&mut input);
    let Ok(ev) = serde_json::from_str::<serde_json::Value>(&input) else { return 0 };
    let get = |k: &str| ev.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let Ok(st) = syn_courier::status() else { return 0 };
    let session = get("session_id");
    // Телефонный сеанс сам отвечает в мессенджер.
    if st.claude.session_id.as_deref() == Some(session.as_str()) {
        return 0;
    }
    let name = session_name(&session).unwrap_or_else(|| {
        std::path::Path::new(&get("cwd")).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Claude".into())
    });
    let body = match get("hook_event_name").as_str() {
        "Stop" | "SubagentStop" => {
            let last = if ev.get("last_assistant_message").is_some() { get("last_assistant_message") } else { last_assistant_text(&get("transcript_path")) };
            let t = if last.trim().is_empty() { "Готово.".to_string() } else { last };
            format!("✅ {}", trim_chars(&t, 700))
        }
        "Notification" => format!("❓ {}", get("message")),
        _ => return 0,
    };
    let _ = syn_courier::request(&Request::Notify { title: name, body, source: "claude".into() });
    0
}

/// Имя сеанса по его номеру (`claude agents --json`).
fn session_name(id: &str) -> Option<String> {
    let out = std::process::Command::new("claude").args(["agents", "--json"]).output().ok()?;
    let v: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).ok()?;
    v.iter().find(|a| a.get("sessionId").and_then(|s| s.as_str()) == Some(id)).and_then(|a| a.get("name")?.as_str().map(str::to_string))
}

/// Последний текст Claude из транскрипта (JSONL).
pub fn last_assistant_text(path: &str) -> String {
    let Ok(text) = std::fs::read_to_string(path) else { return String::new() };
    for line in text.lines().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let Some(parts) = v.pointer("/message/content").and_then(|c| c.as_array()) else { continue };
        let t: Vec<&str> = parts.iter().filter(|p| p.get("type").and_then(|x| x.as_str()) == Some("text")).filter_map(|p| p.get("text")?.as_str()).collect();
        if !t.is_empty() {
            return t.join("\n");
        }
    }
    String::new()
}

fn trim_chars(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n {
        return s.to_string();
    }
    format!("{}…", s.chars().take(n).collect::<String>().trim_end())
}

/// Привязка из терминала: QR-код символами, ждём сканирования.
fn cli_link() -> i32 {
    if let Err(e) = syn_courier::request(&Request::Link { channel: "whatsapp".into() }) {
        eprintln!("{e}");
        return 1;
    }
    let mut shown = String::new();
    for _ in 0..600 {
        let Ok(s) = syn_courier::status() else { break };
        let Some(c) = s.channel("whatsapp") else { break };
        match c.state {
            ChannelState::Online => {
                println!("WhatsApp подключён: {}", c.account.clone().unwrap_or_default());
                return 0;
            }
            ChannelState::Linking => {
                if let Some(q) = &c.qr {
                    if *q != shown {
                        shown = q.clone();
                        println!("\nWhatsApp → Связанные устройства → Привязка устройства, отсканируйте:\n");
                        println!("{}", qr_text(q));
                    }
                }
            }
            ChannelState::Error => {
                eprintln!("ошибка: {}", c.error.clone().unwrap_or_default());
                return 1;
            }
            _ => {}
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    eprintln!("не дождался привязки");
    1
}

/// QR-код полублоками (две строки модулей на строку терминала).
fn qr_text(data: &str) -> String {
    let Ok(code) = qrcode::QrCode::new(data.as_bytes()) else { return data.to_string() };
    let w = code.width();
    let dark = |x: i64, y: i64| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < w && code[(x as usize, y as usize)] == qrcode::Color::Dark;
    let mut out = String::new();
    // Светлая рамка 2 модуля: тёмные модули — пробелы на светлом фоне терминала не видны,
    // поэтому рисуем светлое блоками, тёмное — пробелами (QR читается на тёмной теме).
    let q = 2i64;
    let mut y = -q;
    while y < w as i64 + q {
        for x in -q..w as i64 + q {
            let (top, bot) = (!dark(x, y), !dark(x, y + 1));
            out.push(match (top, bot) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        out.push('\n');
        y += 2;
    }
    out
}

fn run_daemon() -> i32 {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,whatsapp_rust=warn,wacore=warn".into())).init();
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("tokio: {e}");
            return 1;
        }
    };
    rt.block_on(async {
        let hub = match hub::Hub::new() {
            Ok(h) => Arc::new(h),
            Err(e) => {
                tracing::error!("{e:#}");
                return 1;
            }
        };
        if let Err(e) = hub.clone().run().await {
            tracing::error!("{e:#}");
            return 1;
        }
        0
    })
}
