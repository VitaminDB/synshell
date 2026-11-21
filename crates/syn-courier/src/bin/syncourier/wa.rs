//! Канал WhatsApp: привязка как связанное устройство (QR-код), разговор
//! только в чате «Сообщения себе» — всё остальное пропускается.
//!
//! Библиотека — `whatsapp-rust` (протокол WhatsApp Web на Rust), сессия —
//! SQLite в каталоге данных службы (права 600).
//!
//! Петля: ответы Курьера тоже приходят как «свои» сообщения в тот же чат.
//! Номера отправленных запоминаются, эхо по ним пропускается.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use syn_courier::ChannelState;
use whatsapp_rust::prelude::*;
use whatsapp_rust::wacore_binary::JidExt;

use crate::hub::Hub;

const ID: &str = "whatsapp";
/// Сколько номеров своих сообщений помнить для отсева эха.
const SENT_MAX: usize = 200;

#[derive(Default)]
pub struct WhatsApp {
    handle: tokio::sync::Mutex<Option<BotHandle>>,
    client: Mutex<Option<Arc<Client>>>,
    sent: Arc<Mutex<VecDeque<String>>>,
}

fn db_path() -> PathBuf {
    syn_courier::data_dir().join("whatsapp.db")
}

/// Чат «Сообщения себе»: личный чат со своим номером или своим LID.
fn is_self_chat(chat: &Jid, client: &Client) -> bool {
    if chat.is_group() {
        return false;
    }
    let me = [client.pn(), client.lid()];
    me.iter().flatten().any(|j| j.user == chat.user && j.server == chat.server)
}

impl WhatsApp {
    /// Подключиться. `link` — начать привязку (QR) даже без сохранённой сессии.
    pub async fn start(&self, hub: Arc<Hub>, link: bool) {
        let mut h = self.handle.lock().await;
        if h.is_some() {
            return;
        }
        let db = db_path();
        if !link && !db.exists() {
            hub.set_channel(ID, |c| {
                c.state = ChannelState::Unlinked;
                c.qr = None;
            });
            return;
        }
        hub.set_channel(ID, |c| {
            c.state = ChannelState::Connecting;
            c.error = None;
        });
        match self.build(hub.clone(), &db).await {
            Ok(bot) => {
                *self.client.lock().unwrap() = Some(bot.client());
                *h = Some(bot.spawn());
            }
            Err(e) => {
                tracing::warn!("WhatsApp: {e:#}");
                hub.set_channel(ID, |c| {
                    c.state = ChannelState::Error;
                    c.error = Some(format!("{e:#}"));
                });
            }
        }
        // Файлы сессии — только владельцу.
        protect(&db);
    }

    async fn build(&self, hub: Arc<Hub>, db: &std::path::Path) -> anyhow::Result<Bot> {
        let store = SqliteStore::new(db.to_string_lossy().as_ref()).await.map_err(|e| anyhow::anyhow!("{e}")).context("хранилище сессии")?;
        let (h1, h2, h3, h4) = (hub.clone(), hub.clone(), hub.clone(), hub.clone());
        let sent = self.sent.clone();
        let bot = Bot::builder()
            .with_backend(store)
            .on_qr_code(move |code, _timeout| {
                let hub = h1.clone();
                async move {
                    tracing::info!("WhatsApp: новый QR-код для привязки");
                    hub.set_channel(ID, |c| {
                        c.state = ChannelState::Linking;
                        c.qr = Some(code);
                    });
                }
            })
            .on_connected(move |client| {
                let hub = h2.clone();
                async move {
                    let account = client.pn().map(|j| format!("+{}", j.user)).or_else(|| client.lid().map(|j| j.user.to_string()));
                    tracing::info!("WhatsApp подключён: {}", account.clone().unwrap_or_default());
                    hub.set_channel(ID, |c| {
                        c.state = ChannelState::Online;
                        c.qr = None;
                        c.error = None;
                        c.account = account;
                    });
                    protect(&db_path());
                }
            })
            .on_logged_out(move |info| {
                let hub = h3.clone();
                async move {
                    tracing::warn!("WhatsApp: выход из учётной записи ({:?})", info.reason);
                    hub.set_channel(ID, |c| {
                        c.state = ChannelState::Unlinked;
                        c.qr = None;
                        c.account = None;
                    });
                }
            })
            .on_message(move |ctx| {
                let hub = h4.clone();
                let sent = sent.clone();
                async move {
                    let src = &ctx.info.source;
                    if !src.is_from_me || !is_self_chat(&src.chat, &ctx.client) {
                        return;
                    }
                    let id = ctx.info.id.to_string();
                    {
                        let mut s = sent.lock().unwrap();
                        if let Some(i) = s.iter().position(|x| *x == id) {
                            s.remove(i);
                            return;
                        }
                    }
                    let Some(text) = ctx.message.text_content() else {
                        hub.say("Пока понимаю только текст.").await;
                        return;
                    };
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        tracing::info!("WhatsApp: сообщение ({} симв.)", text.chars().count());
                        hub.incoming(ID, text);
                    }
                }
            })
            .build()
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(bot)
    }

    /// Написать в чат «Сообщения себе».
    pub async fn send_self(&self, text: &str) -> anyhow::Result<()> {
        let client = self.client.lock().unwrap().clone().context("WhatsApp не подключён")?;
        let me = client.pn().or_else(|| client.lid()).context("свой номер неизвестен")?.to_non_ad();
        let r = client.send_message(&me, wa::Message::text(text)).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut s = self.sent.lock().unwrap();
        s.push_back(r.message_id);
        while s.len() > SENT_MAX {
            s.pop_front();
        }
        Ok(())
    }

    /// Отвязать: выйти из учётной записи и стереть сессию.
    pub async fn unlink(&self, hub: &Arc<Hub>) {
        let client = self.client.lock().unwrap().take();
        if let Some(c) = client {
            if c.is_logged_in() {
                c.logout().await;
            }
        }
        self.shutdown().await;
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", db_path().display()));
        }
        hub.set_channel(ID, |c| {
            c.state = ChannelState::Unlinked;
            c.account = None;
            c.qr = None;
        });
        tracing::info!("WhatsApp отвязан");
    }

    pub async fn shutdown(&self) {
        if let Some(h) = self.handle.lock().await.take() {
            h.shutdown().await;
        }
        *self.client.lock().unwrap() = None;
    }
}

fn protect(db: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    for ext in ["", "-wal", "-shm"] {
        let p = PathBuf::from(format!("{}{ext}", db.display()));
        if p.exists() {
            let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
        }
    }
}
