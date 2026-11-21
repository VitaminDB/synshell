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
                    // Только новые: история и очередь за время простоя службы — мимо.
                    let ts = ctx.info.timestamp.timestamp_millis().max(0) as u64;
                    if !fresh(ts) {
                        return;
                    }
                    let Some(text) = ctx.message.text_content() else {
                        // Вложения; служебные (ключи, синхронизация, реакции) — молча.
                        incoming_media(&hub, &ctx).await;
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

    /// Отправить файл в «Сообщения себе»: картинка, видео, аудио (голосовое — `voice`), иначе документ.
    pub async fn send_file_self(&self, path: &std::path::Path, caption: &str, voice: bool) -> anyhow::Result<()> {
        use whatsapp_rust::download::MediaType;
        let client = self.client.lock().unwrap().clone().context("WhatsApp не подключён")?;
        let me = client.pn().or_else(|| client.lid()).context("свой номер неизвестен")?.to_non_ad();
        let data = std::fs::read(path).with_context(|| path.display().to_string())?;
        anyhow::ensure!(data.len() <= 100 * 1024 * 1024, "файл больше 100 МБ");
        let mime = crate::media::mime_for(path).to_string();
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "файл".into());
        let caption = (!caption.trim().is_empty()).then(|| caption.trim().to_string());
        let kind = if voice || mime.starts_with("audio/") {
            MediaType::Audio
        } else if mime.starts_with("image/") && mime != "image/gif" {
            MediaType::Image
        } else if mime.starts_with("video/") {
            MediaType::Video
        } else {
            MediaType::Document
        };
        let seconds = if matches!(kind, MediaType::Audio | MediaType::Video) { Some(crate::media::duration_secs(path).await) } else { None };
        let up = client.upload(data, kind, Default::default()).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut msg = wa::Message::default();
        match kind {
            MediaType::Image => {
                msg.image_message = MessageField::some(wa::message::ImageMessage {
                    url: Some(up.url),
                    direct_path: Some(up.direct_path),
                    media_key: Some(up.media_key.to_vec()),
                    file_enc_sha256: Some(up.file_enc_sha256.to_vec()),
                    file_sha256: Some(up.file_sha256.to_vec()),
                    file_length: Some(up.file_length),
                    media_key_timestamp: Some(up.media_key_timestamp),
                    mimetype: Some(mime),
                    caption,
                    ..Default::default()
                })
            }
            MediaType::Video => {
                msg.video_message = MessageField::some(wa::message::VideoMessage {
                    url: Some(up.url),
                    direct_path: Some(up.direct_path),
                    media_key: Some(up.media_key.to_vec()),
                    file_enc_sha256: Some(up.file_enc_sha256.to_vec()),
                    file_sha256: Some(up.file_sha256.to_vec()),
                    file_length: Some(up.file_length),
                    media_key_timestamp: Some(up.media_key_timestamp),
                    mimetype: Some(mime),
                    streaming_sidecar: up.streaming_sidecar,
                    seconds,
                    caption,
                    ..Default::default()
                })
            }
            MediaType::Audio => {
                msg.audio_message = MessageField::some(wa::message::AudioMessage {
                    url: Some(up.url),
                    direct_path: Some(up.direct_path),
                    media_key: Some(up.media_key.to_vec()),
                    file_enc_sha256: Some(up.file_enc_sha256.to_vec()),
                    file_sha256: Some(up.file_sha256.to_vec()),
                    file_length: Some(up.file_length),
                    media_key_timestamp: Some(up.media_key_timestamp),
                    mimetype: Some(if voice { "audio/ogg; codecs=opus".into() } else { mime }),
                    streaming_sidecar: up.streaming_sidecar,
                    seconds,
                    ptt: Some(voice),
                    ..Default::default()
                })
            }
            _ => {
                msg.document_message = MessageField::some(wa::message::DocumentMessage {
                    url: Some(up.url),
                    direct_path: Some(up.direct_path),
                    media_key: Some(up.media_key.to_vec()),
                    file_enc_sha256: Some(up.file_enc_sha256.to_vec()),
                    file_sha256: Some(up.file_sha256.to_vec()),
                    file_length: Some(up.file_length),
                    media_key_timestamp: Some(up.media_key_timestamp),
                    mimetype: Some(mime),
                    title: Some(name.clone()),
                    file_name: Some(name),
                    caption,
                    ..Default::default()
                })
            }
        }
        let r = client.send_message(&me, msg).await.map_err(|e| anyhow::anyhow!("{e}"))?;
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

/// Вложение из чата: сохранить; голосовое — распознать; Claude — путь и подпись.
async fn incoming_media(hub: &Arc<Hub>, ctx: &MessageContext) {
    let base = ctx.message.get_base_message();
    let client = &ctx.client;
    // (байты, имя файла, подпись, что это, голосовое)
    let got: Option<(anyhow::Result<Vec<u8>>, String, String, &str, bool)> = if let Some(m) = base.image_message.as_option() {
        Some((client.download(m).await.map_err(|e| anyhow::anyhow!("{e}")), format!("фото.{}", crate::media::ext_for(m.mimetype.as_deref().unwrap_or(""))), m.caption.clone().unwrap_or_default(), "фото", false))
    } else if let Some(m) = base.video_message.as_option() {
        Some((client.download(m).await.map_err(|e| anyhow::anyhow!("{e}")), format!("видео.{}", crate::media::ext_for(m.mimetype.as_deref().unwrap_or(""))), m.caption.clone().unwrap_or_default(), "видео", false))
    } else if let Some(m) = base.document_message.as_option() {
        let name = m.file_name.clone().unwrap_or_else(|| format!("документ.{}", crate::media::ext_for(m.mimetype.as_deref().unwrap_or(""))));
        Some((client.download(m).await.map_err(|e| anyhow::anyhow!("{e}")), name, m.caption.clone().unwrap_or_default(), "документ", false))
    } else if let Some(m) = base.audio_message.as_option() {
        let voice = m.ptt.unwrap_or(false);
        Some((client.download(m).await.map_err(|e| anyhow::anyhow!("{e}")), format!("{}.{}", if voice { "голосовое" } else { "аудио" }, crate::media::ext_for(m.mimetype.as_deref().unwrap_or(""))), String::new(), if voice { "голосовое" } else { "аудио" }, voice))
    } else if let Some(m) = base.sticker_message.as_option() {
        Some((client.download(m).await.map_err(|e| anyhow::anyhow!("{e}")), "стикер.webp".into(), String::new(), "стикер", false))
    } else {
        None
    };
    let Some((bytes, name, caption, what, voice)) = got else { return };
    let bytes = match bytes {
        Ok(b) => b,
        Err(e) => {
            hub.say(&format!("⚠️ Не скачал {what}: {e:#}")).await;
            return;
        }
    };
    let path = match crate::media::save(&bytes, &name) {
        Ok(p) => p,
        Err(e) => {
            hub.say(&format!("⚠️ Не сохранил {what}: {e}")).await;
            return;
        }
    };
    tracing::info!("WhatsApp: {what} → {}", path.display());
    if voice {
        match crate::media::transcribe(&path).await {
            Ok(t) if !t.is_empty() => {
                hub.say(&format!("🎤 {t}")).await;
                hub.incoming(ID, format!("[голосовое] {t}"));
                return;
            }
            Ok(_) => hub.say("🎤 Речь не распознана — передаю файл Claude.").await,
            Err(e) => hub.say(&format!("🎤 Не распознал ({e:#}) — передаю файл Claude.")).await,
        }
    }
    let mut msg = format!("[Пользователь прислал {what}: {}]", path.display());
    if !caption.trim().is_empty() {
        msg.push('\n');
        msg.push_str(caption.trim());
    }
    hub.incoming(ID, msg);
}

/// Предел возраста сообщения: старше — не разбирать (часть истории).
const MAX_AGE_MS: u64 = 10 * 60 * 1000;

fn last_file() -> PathBuf {
    syn_courier::data_dir().join("whatsapp.last")
}

/// Новое ли сообщение: не старше [`MAX_AGE_MS`] и позже последнего разобранного (оно запоминается).
fn fresh(ts: u64) -> bool {
    static LAST: Mutex<Option<u64>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap();
    let prev = *last.get_or_insert_with(|| std::fs::read_to_string(last_file()).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0));
    if !is_fresh(ts, prev, crate::hub::now_ms()) {
        return false;
    }
    *last = Some(ts);
    let _ = std::fs::write(last_file(), ts.to_string());
    true
}

pub fn is_fresh(ts: u64, last: u64, now: u64) -> bool {
    ts > last && ts + MAX_AGE_MS >= now
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

#[cfg(test)]
mod tests {
    #[test]
    fn freshness() {
        let now = 1_000_000_000u64;
        assert!(super::is_fresh(now - 1000, 0, now));
        assert!(!super::is_fresh(now - 3_600_000, 0, now), "история — мимо");
        assert!(!super::is_fresh(now - 1000, now - 500, now), "уже разобранное — мимо");
    }
}
