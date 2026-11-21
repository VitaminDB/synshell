//! Узел службы: настройки, состояние для клиентов, сокет, разводка
//! «канал ↔ Claude ↔ уведомления».

use std::sync::{Arc, Mutex, RwLock};

use anyhow::Context;
use syn_courier::{ChannelState, ChannelStatus, Config, NotifyMode, Recent, Request, Response, Status};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

use crate::{claude, notify, router, wa};

/// Сколько последних сообщений помнить для апплета.
const RECENT_MAX: usize = 20;
/// Предел длины одного сообщения в мессенджер; длиннее — частями.
pub const CHUNK: usize = 3500;

/// Входящее сообщение пользователя из канала.
#[derive(Debug, Clone)]
pub struct Incoming {
    pub channel: String,
    pub text: String,
}

pub struct Hub {
    pub cfg: RwLock<Config>,
    pub status: Mutex<Status>,
    pub wa: wa::WhatsApp,
    pub claude: claude::Session,
    incoming_tx: mpsc::UnboundedSender<Incoming>,
    incoming_rx: Mutex<Option<mpsc::UnboundedReceiver<Incoming>>>,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl Hub {
    pub fn new() -> anyhow::Result<Self> {
        let dir = syn_courier::data_dir();
        std::fs::create_dir_all(&dir).with_context(|| dir.display().to_string())?;
        // Сессия мессенджера — ключи учётной записи: только владельцу.
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        let cfg = Config::load();
        let (tx, rx) = mpsc::unbounded_channel();
        let status = Status {
            channels: vec![ChannelStatus { id: "whatsapp".into(), title: "WhatsApp".into(), ..Default::default() }],
            notify: cfg.notify.mode,
            ..Default::default()
        };
        Ok(Self {
            claude: claude::Session::new(&cfg.claude),
            cfg: RwLock::new(cfg),
            status: Mutex::new(status),
            wa: wa::WhatsApp::default(),
            incoming_tx: tx,
            incoming_rx: Mutex::new(Some(rx)),
        })
    }

    pub fn config(&self) -> Config {
        self.cfg.read().unwrap().clone()
    }

    /// Обновить состояние канала.
    pub fn set_channel(&self, id: &str, f: impl FnOnce(&mut ChannelStatus)) {
        let mut st = self.status.lock().unwrap();
        if let Some(c) = st.channels.iter_mut().find(|c| c.id == id) {
            f(c);
        }
    }

    pub fn channel_state(&self, id: &str) -> ChannelState {
        self.status.lock().unwrap().channel(id).map(|c| c.state).unwrap_or_default()
    }

    fn remember(&self, dir: &str, text: &str) {
        let mut st = self.status.lock().unwrap();
        st.recent.push(Recent { dir: dir.into(), text: text.chars().take(300).collect(), at: now_ms() });
        let n = st.recent.len();
        if n > RECENT_MAX {
            st.recent.drain(..n - RECENT_MAX);
        }
    }

    /// Сообщение пользователя из канала → разводка.
    pub fn incoming(&self, channel: &str, text: String) {
        self.remember("in", &text);
        let _ = self.incoming_tx.send(Incoming { channel: channel.into(), text });
    }

    /// Отправить пользователю во все подключённые каналы (длинное — частями).
    pub async fn say(self: &Arc<Self>, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.remember("out", text);
        for part in split_chunks(text, CHUNK) {
            if self.channel_state("whatsapp") == ChannelState::Online {
                if let Err(e) = self.wa.send_self(&part).await {
                    tracing::warn!("WhatsApp: не отправлено: {e:#}");
                }
            } else {
                tracing::info!("канал не подключён, сообщение не отправлено: {}", part.chars().take(80).collect::<String>());
            }
        }
    }

    /// Уведомление: от Claude — всегда, остальные — по режиму.
    pub async fn notify(self: &Arc<Self>, title: &str, body: &str, source: &str, app: &str) {
        let cfg = self.config();
        let from_claude = source == "claude";
        if from_claude {
            match cfg.notify.claude {
                syn_courier::ClaudeEvents::Never => return,
                syn_courier::ClaudeEvents::Always => {}
                syn_courier::ClaudeEvents::Away => {
                    if !notify::away(cfg.notify.away_minutes) {
                        return;
                    }
                }
            }
        } else {
            if cfg.notify.exclude.iter().any(|x| x.eq_ignore_ascii_case(app)) {
                return;
            }
            match cfg.notify.mode {
                NotifyMode::Claude => return,
                NotifyMode::All => {}
                NotifyMode::Away => {
                    if !notify::away(cfg.notify.away_minutes) {
                        return;
                    }
                }
            }
        }
        let head = if from_claude { "🤖" } else { "🔔" };
        let mut text = String::new();
        match (title.trim(), app.trim()) {
            ("", "") => text.push_str(head),
            (t, "") => text.push_str(&format!("{head} *{t}*")),
            ("", a) => text.push_str(&format!("{head} *{a}*")),
            (t, a) if t.eq_ignore_ascii_case(a) => text.push_str(&format!("{head} *{t}*")),
            (t, a) => text.push_str(&format!("{head} *{t}* · {a}")),
        }
        if !body.trim().is_empty() {
            text.push('\n');
            text.push_str(body.trim());
        }
        self.say(&text).await;
    }

    pub fn snapshot(&self) -> Status {
        let mut st = self.status.lock().unwrap().clone();
        st.claude = self.claude.status();
        st.notify = self.cfg.read().unwrap().notify.mode;
        st
    }

    pub async fn run(self: Arc<Self>) -> anyhow::Result<()> {
        // Сокет первым: клиенты (апплет, хуки) видят службу сразу.
        let path = syn_courier::socket_path();
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).with_context(|| path.display().to_string())?;
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        let hub = self.clone();
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((conn, _)) => {
                        let hub = hub.clone();
                        tokio::spawn(async move {
                            if let Err(e) = serve(hub, conn).await {
                                tracing::debug!("клиент: {e:#}");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!("сокет: {e}");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }
            }
        });

        // Ответы Claude → пользователю.
        let mut replies = self.claude.take_replies();
        let hub = self.clone();
        tokio::spawn(async move {
            while let Some(text) = replies.recv().await {
                hub.say(&text).await;
            }
        });

        // Уведомления системы (D-Bus).
        notify::start(self.clone());

        // WhatsApp: если привязан — подключиться.
        if self.config().whatsapp.enabled {
            self.wa.start(self.clone(), false).await;
        } else {
            self.set_channel("whatsapp", |c| c.state = ChannelState::Off);
        }

        tracing::info!("syncourier запущен");
        let mut rx = self.incoming_rx.lock().unwrap().take().context("разводка уже запущена")?;
        let hub = self.clone();
        let router = tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                router::handle(&hub, m).await;
            }
        });

        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
            _ = router => {}
        }
        tracing::info!("остановка");
        self.claude.stop().await;
        self.wa.shutdown().await;
        let _ = std::fs::remove_file(&path);
        Ok(())
    }
}

async fn serve(hub: Arc<Hub>, conn: tokio::net::UnixStream) -> anyhow::Result<()> {
    let (r, mut w) = conn.into_split();
    let mut line = String::new();
    BufReader::new(r).read_line(&mut line).await?;
    let resp = match serde_json::from_str::<Request>(&line) {
        Ok(Request::Status) => Response { ok: true, status: Some(hub.snapshot()), ..Default::default() },
        Ok(Request::Link { channel }) if channel == "whatsapp" => {
            if !hub.config().whatsapp.enabled {
                let mut c = hub.config();
                c.whatsapp.enabled = true;
                let _ = c.save();
                *hub.cfg.write().unwrap() = c;
            }
            hub.wa.start(hub.clone(), true).await;
            Response { ok: true, status: Some(hub.snapshot()), ..Default::default() }
        }
        Ok(Request::Unlink { channel }) if channel == "whatsapp" => {
            hub.wa.unlink(&hub).await;
            Response { ok: true, status: Some(hub.snapshot()), ..Default::default() }
        }
        Ok(Request::Link { channel } | Request::Unlink { channel }) => Response { ok: false, error: Some(format!("канал «{channel}» пока не поддерживается")), ..Default::default() },
        Ok(Request::Notify { title, body, source }) => {
            let hub2 = hub.clone();
            // Не держать клиента (хук Claude) на время отправки.
            tokio::spawn(async move { hub2.notify(&title, &body, &source, "").await });
            Response { ok: true, ..Default::default() }
        }
        Ok(Request::Ask { text }) => {
            hub.incoming("local", text);
            Response { ok: true, ..Default::default() }
        }
        Ok(Request::Reload) => {
            let c = Config::load();
            hub.claude.reconfigure(&c.claude);
            let wa_on = c.whatsapp.enabled;
            *hub.cfg.write().unwrap() = c;
            if !wa_on {
                hub.wa.shutdown().await;
                hub.set_channel("whatsapp", |c| c.state = ChannelState::Off);
            } else if hub.channel_state("whatsapp") == ChannelState::Off {
                hub.wa.start(hub.clone(), false).await;
            }
            Response { ok: true, status: Some(hub.snapshot()), ..Default::default() }
        }
        Err(e) => Response { ok: false, error: Some(format!("запрос: {e}")), ..Default::default() },
    };
    let mut out = serde_json::to_string(&resp)?;
    out.push('\n');
    w.write_all(out.as_bytes()).await?;
    Ok(())
}

/// Разбить текст на части не длиннее `max` символов — по абзацам, строкам, словам.
pub fn split_chunks(text: &str, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_len = 0usize;
    for line in text.split_inclusive('\n') {
        let n = line.chars().count();
        if cur_len + n > max && !cur.is_empty() {
            out.push(std::mem::take(&mut cur).trim_end().to_string());
            cur_len = 0;
        }
        if n > max {
            // Очень длинная строка — по словам, слово длиннее предела — по символам.
            let mut piece = String::new();
            let mut plen = 0;
            for word in line.split_inclusive(' ') {
                let wl = word.chars().count();
                if plen + wl > max && !piece.is_empty() {
                    out.push(std::mem::take(&mut piece).trim_end().to_string());
                    plen = 0;
                }
                if wl > max {
                    let chars: Vec<char> = word.chars().collect();
                    for c in chars.chunks(max) {
                        out.push(c.iter().collect());
                    }
                    continue;
                }
                piece.push_str(word);
                plen += wl;
            }
            cur = piece;
            cur_len = plen;
            continue;
        }
        cur.push_str(line);
        cur_len += n;
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim_end().to_string());
    }
    out
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks() {
        assert_eq!(split_chunks("abc", 10), vec!["abc"]);
        let t = "раз два\nтри четыре\nпять";
        let v = split_chunks(t, 12);
        assert!(v.iter().all(|p| p.chars().count() <= 12), "{v:?}");
        assert_eq!(v.join("\n"), t);
        let long = "x".repeat(25);
        let v = split_chunks(&long, 10);
        assert_eq!(v.len(), 3);
        assert_eq!(v.concat(), long);
    }
}
