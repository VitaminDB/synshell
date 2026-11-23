//! Узел службы: настройки, состояние для клиентов, сокет, разводка
//! «канал ↔ Claude ↔ уведомления».

use std::sync::{Arc, Mutex, RwLock};

use anyhow::Context;
use syn_courier::{ChannelState, ChannelStatus, Config, NotifyMode, Recent, Request, Response, Status};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

use crate::{claude, notify, router, tg, wa};

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
    pub tg: tg::Telegram,
    /// Откуда пользователь писал последним — туда ответы и уведомления.
    last_channel: Mutex<String>,
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
            channels: vec![
                ChannelStatus { id: "whatsapp".into(), title: "WhatsApp".into(), ..Default::default() },
                ChannelStatus { id: "telegram".into(), title: "Telegram".into(), ..Default::default() },
            ],
            notify: cfg.notify.mode,
            ..Default::default()
        };
        Ok(Self {
            claude: claude::Session::new(&cfg.claude),
            cfg: RwLock::new(cfg),
            status: Mutex::new(status),
            wa: wa::WhatsApp::default(),
            tg: tg::Telegram::default(),
            last_channel: Mutex::new(std::fs::read_to_string(syn_courier::data_dir().join("last_channel")).unwrap_or_default().trim().to_string()),
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

    pub fn remember_pub(&self, dir: &str, text: &str) {
        self.remember(dir, text);
    }

    /// Сообщение пользователя из канала → разводка.
    pub fn incoming(&self, channel: &str, text: String) {
        self.remember("in", &text);
        if channel != "local" {
            self.set_last_channel(channel);
        }
        let _ = self.incoming_tx.send(Incoming { channel: channel.into(), text });
    }

    /// Отправить пользователю во все подключённые каналы (длинное — частями).
    pub async fn say(self: &Arc<Self>, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.remember("out", text);
        let targets = self.targets();
        if targets.is_empty() {
            tracing::info!("ни один канал не подключён, сообщение не отправлено: {}", text.chars().take(80).collect::<String>());
            return;
        }
        for ch in targets {
            for part in split_chunks(text, CHUNK) {
                let r = match ch.as_str() {
                    "telegram" => self.tg.send_text(&part).await,
                    _ => self.wa.send_self(&part).await,
                };
                if let Err(e) = r {
                    tracing::warn!("{ch}: не отправлено: {e:#}");
                }
            }
        }
    }

    /// Запомнить канал, откуда пользователь написал.
    pub fn set_last_channel(&self, ch: &str) {
        let mut l = self.last_channel.lock().unwrap();
        if *l != ch {
            *l = ch.to_string();
            let _ = std::fs::write(syn_courier::data_dir().join("last_channel"), ch);
        }
    }

    /// Куда отправлять — по настройке `delivery` среди подключённых каналов.
    pub fn targets(&self) -> Vec<String> {
        let last = self.last_channel.lock().unwrap().clone();
        let delivery = self.cfg.read().unwrap().delivery;
        let online: Vec<String> = {
            let st = self.status.lock().unwrap();
            st.channels.iter().filter(|c| c.state == ChannelState::Online).map(|c| c.id.clone()).collect()
        };
        pick_targets(delivery, &online, &last)
    }

    /// Первый адресат (для состояния и проверок «есть ли куда отправить»).
    pub fn target(&self) -> Option<String> {
        self.targets().into_iter().next()
    }

    /// Файл пользователю — в каналы по настройке.
    pub async fn send_file(self: &Arc<Self>, path: &std::path::Path, caption: &str, voice: bool, as_file: bool) -> anyhow::Result<()> {
        // Картинка с прозрачностью или слишком большая — документом: фото пережимается в JPEG.
        let as_file = as_file || (!voice && crate::media::needs_document(path));
        let targets = self.targets();
        anyhow::ensure!(!targets.is_empty(), "ни один канал не подключён");
        let mut last_err = None;
        let mut sent = 0;
        for ch in &targets {
            let r = match ch.as_str() {
                "telegram" => self.tg.send_file(path, caption, voice, as_file).await,
                _ => self.wa.send_file_self(path, caption, voice, as_file).await,
            };
            match r {
                Ok(()) => sent += 1,
                Err(e) => {
                    tracing::warn!("{ch}: файл не отправлен: {e:#}");
                    last_err = Some(e);
                }
            }
        }
        match (sent, last_err) {
            (0, Some(e)) => Err(e),
            _ => Ok(()),
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
        let t = self.targets();
        st.active = if t.len() > 1 { "all".into() } else { t.into_iter().next().unwrap_or_default() };
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

        // Пульт — мини-приложение Telegram (страница + API на localhost, quick tunnel Cloudflare).
        crate::panel::start(self.clone());

        // WhatsApp: если привязан — подключиться.
        if self.config().whatsapp.enabled {
            self.wa.start(self.clone(), false).await;
        } else {
            self.set_channel("whatsapp", |c| c.state = ChannelState::Off);
        }

        if self.config().telegram.enabled {
            self.tg.start(self.clone()).await;
        } else {
            self.set_channel("telegram", |c| c.state = ChannelState::Off);
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
        Ok(Request::Link { channel }) if channel == "telegram" => {
            hub.tg.start(hub.clone()).await;
            Response { ok: true, status: Some(hub.snapshot()), ..Default::default() }
        }
        Ok(Request::Unlink { channel }) if channel == "telegram" => {
            hub.tg.unlink(&hub).await;
            Response { ok: true, status: Some(hub.snapshot()), ..Default::default() }
        }
        Ok(Request::SetToken { channel, token }) if channel == "telegram" => match hub.tg.set_token(hub.clone(), &token).await {
            Ok(()) => Response { ok: true, status: Some(hub.snapshot()), ..Default::default() },
            Err(e) => Response { ok: false, error: Some(format!("{e:#}")), ..Default::default() },
        },
        Ok(Request::SetToken { channel, .. }) => Response { ok: false, error: Some(format!("у канала «{channel}» нет токена")), ..Default::default() },
        Ok(Request::Link { channel } | Request::Unlink { channel }) => Response { ok: false, error: Some(format!("канал «{channel}» пока не поддерживается")), ..Default::default() },
        Ok(Request::Notify { title, body, source }) => {
            let hub2 = hub.clone();
            // Не держать клиента (хук Claude) на время отправки.
            tokio::spawn(async move { hub2.notify(&title, &body, &source, "").await });
            Response { ok: true, ..Default::default() }
        }
        Ok(Request::Hook { event }) => {
            let hub2 = hub.clone();
            tokio::spawn(async move { crate::hook::handle(hub2, event).await });
            Response { ok: true, ..Default::default() }
        }
        Ok(Request::SendFile { path, caption, voice, file }) => {
            let p = std::path::PathBuf::from(&path);
            if !p.is_file() {
                Response { ok: false, error: Some(format!("нет файла {path}")), ..Default::default() }
            } else if hub.target().is_none() {
                Response { ok: false, error: Some("ни один канал не подключён".into()), ..Default::default() }
            } else {
                hub.remember_pub("out", &format!("📎 {}", p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
                match hub.send_file(&p, &caption, voice, file).await {
                    Ok(()) => Response { ok: true, ..Default::default() },
                    Err(e) => Response { ok: false, error: Some(format!("{e:#}")), ..Default::default() },
                }
            }
        }
        Ok(Request::Say { text }) => match crate::media::synthesize(&text).await {
            Ok((ogg, _)) => {
                hub.remember_pub("out", &format!("🔊 {text}"));
                let r = hub.send_file(&ogg, "", true, false).await;
                let _ = std::fs::remove_file(&ogg);
                match r {
                    Ok(()) => Response { ok: true, ..Default::default() },
                    Err(e) => Response { ok: false, error: Some(format!("{e:#}")), ..Default::default() },
                }
            }
            Err(e) => Response { ok: false, error: Some(format!("{e:#}")), ..Default::default() },
        },
        Ok(Request::Ask { text }) => {
            hub.incoming("local", text);
            Response { ok: true, ..Default::default() }
        }
        Ok(Request::Reload) => {
            let c = Config::load();
            hub.claude.reconfigure(&c.claude);
            let wa_on = c.whatsapp.enabled;
            let tg_on = c.telegram.enabled;
            *hub.cfg.write().unwrap() = c;
            if !wa_on {
                hub.wa.shutdown().await;
                hub.set_channel("whatsapp", |c| c.state = ChannelState::Off);
            } else if hub.channel_state("whatsapp") == ChannelState::Off {
                hub.wa.start(hub.clone(), false).await;
            }
            if !tg_on {
                hub.tg.stop();
                hub.set_channel("telegram", |c| c.state = ChannelState::Off);
            } else if hub.channel_state("telegram") == ChannelState::Off {
                hub.tg.start(hub.clone()).await;
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

/// Адресаты: `online` — подключённые каналы по порядку, `last` — откуда писали последним.
pub fn pick_targets(delivery: syn_courier::Delivery, online: &[String], last: &str) -> Vec<String> {
    use syn_courier::Delivery;
    let has = |c: &str| online.iter().any(|o| o == c);
    let prefer = |c: &str| -> Vec<String> {
        if has(c) {
            vec![c.to_string()]
        } else {
            online.first().cloned().into_iter().collect()
        }
    };
    match delivery {
        Delivery::All => online.to_vec(),
        Delivery::Whatsapp => prefer("whatsapp"),
        Delivery::Telegram => prefer("telegram"),
        Delivery::Last => prefer(last),
    }
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
    fn delivery() {
        use syn_courier::Delivery;
        let both = vec!["whatsapp".to_string(), "telegram".to_string()];
        assert_eq!(pick_targets(Delivery::Last, &both, "telegram"), vec!["telegram"]);
        assert_eq!(pick_targets(Delivery::Whatsapp, &both, "telegram"), vec!["whatsapp"]);
        assert_eq!(pick_targets(Delivery::All, &both, ""), both);
        let wa = vec!["whatsapp".to_string()];
        assert_eq!(pick_targets(Delivery::Telegram, &wa, ""), vec!["whatsapp"], "основной не подключён — второй");
        assert!(pick_targets(Delivery::All, &[], "").is_empty());
    }

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
