//! Канал Telegram: личный бот пользователя (Bot API). Токен — от @BotFather,
//! привязка — QR-кодом со ссылкой `t.me/<бот>?start=<код>`: пользователь
//! сканирует, нажимает «Старт», и бот запоминает его чат. Сообщения других
//! людей бот не принимает.
//!
//! Токен и привязка хранятся в `telegram.json` каталога данных (права 600).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use syn_courier::ChannelState;

use crate::hub::Hub;

const ID: &str = "telegram";
const API: &str = "https://api.telegram.org";
/// Предел скачивания файлов у Bot API.
const DOWNLOAD_MAX: u64 = 20 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Stored {
    token: String,
    /// Чат владельца; `None` — бот ещё не привязан.
    owner: Option<i64>,
    owner_name: String,
    bot: String,
}

fn store_path() -> std::path::PathBuf {
    syn_courier::data_dir().join("telegram.json")
}

fn load() -> Stored {
    std::fs::read_to_string(store_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(s: &Stored) {
    use std::os::unix::fs::PermissionsExt;
    let p = store_path();
    if std::fs::write(&p, serde_json::to_string(s).unwrap_or_default()).is_ok() {
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
}

/// Случайный код привязки (12 знаков из /dev/urandom).
fn pair_code() -> String {
    use std::io::Read;
    let mut b = [0u8; 9];
    let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    b.iter().map(|x| format!("{x:02x}")).collect::<String>()[..12].to_string()
}

pub struct Telegram {
    http: reqwest::Client,
    st: Mutex<Stored>,
    code: Mutex<String>,
    /// Номер цикла опроса: старый цикл видит, что его сменили, и выходит.
    epoch: AtomicU64,
}

impl Default for Telegram {
    fn default() -> Self {
        Self {
            http: reqwest::Client::builder().timeout(Duration::from_secs(70)).build().unwrap_or_default(),
            st: Mutex::new(load()),
            code: Mutex::new(String::new()),
            epoch: AtomicU64::new(0),
        }
    }
}

#[derive(Deserialize)]
struct ApiResp<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

impl Telegram {
    fn token(&self) -> String {
        self.st.lock().unwrap().token.clone()
    }

    fn owner(&self) -> Option<i64> {
        self.st.lock().unwrap().owner
    }

    async fn call<T: for<'de> Deserialize<'de>>(&self, method: &str, body: serde_json::Value) -> anyhow::Result<T> {
        let url = format!("{API}/bot{}/{method}", self.token());
        let r: ApiResp<T> = self.http.post(&url).json(&body).send().await?.json().await?;
        if !r.ok {
            anyhow::bail!("{}", r.description.unwrap_or_else(|| "ошибка Telegram".into()));
        }
        r.result.context("пустой ответ Telegram")
    }

    /// Сохранить токен (проверив его) и подключиться.
    pub async fn set_token(&self, hub: Arc<Hub>, token: &str) -> anyhow::Result<()> {
        let token = token.trim().to_string();
        anyhow::ensure!(token.contains(':'), "это не похоже на токен бота (вида 123456:ABC…)");
        let url = format!("{API}/bot{token}/getMe");
        let r: ApiResp<serde_json::Value> = self.http.get(&url).send().await?.json().await?;
        anyhow::ensure!(r.ok, "Telegram не принял токен: {}", r.description.unwrap_or_default());
        let bot = r.result.and_then(|v| v.get("username")?.as_str().map(str::to_string)).unwrap_or_default();
        {
            let mut st = self.st.lock().unwrap();
            if st.token != token {
                st.owner = None;
                st.owner_name.clear();
            }
            st.token = token;
            st.bot = bot;
            save(&st);
        }
        self.stop();
        self.start(hub).await;
        Ok(())
    }

    pub fn stop(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    /// Подключиться: без токена — «не привязан», без владельца — QR-код привязки, иначе — опрос.
    pub async fn start(&self, hub: Arc<Hub>) {
        let st = self.st.lock().unwrap().clone();
        if st.token.is_empty() {
            hub.set_channel(ID, |c| {
                c.state = ChannelState::Unlinked;
                c.qr = None;
                c.account = None;
            });
            return;
        }
        let me = hub.clone();
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        if st.owner.is_none() {
            let code = pair_code();
            *self.code.lock().unwrap() = code.clone();
            let link = format!("https://t.me/{}?start={code}", st.bot);
            hub.set_channel(ID, |c| {
                c.state = ChannelState::Linking;
                c.qr = Some(link);
                c.account = Some(format!("@{}", st.bot));
                c.error = None;
            });
        } else {
            hub.set_channel(ID, |c| {
                c.state = ChannelState::Online;
                c.qr = None;
                c.account = Some(format!("@{}", st.bot));
                c.error = None;
            });
        }
        // Меню команд бота.
        let _ = self
            .call::<serde_json::Value>(
                "setMyCommands",
                serde_json::json!({ "commands": [
                    { "command": "help", "description": "Команды Курьера" },
                    { "command": "sessions", "description": "Живые сеансы Claude" },
                    { "command": "status", "description": "Состояние" },
                    { "command": "new", "description": "Новый разговор" },
                    { "command": "stop", "description": "Прервать ответ" },
                ]}),
            )
            .await;
        tokio::spawn(async move { me.tg.poll(me.clone(), epoch).await });
    }

    async fn poll(&self, hub: Arc<Hub>, epoch: u64) {
        let mut offset: i64 = 0;
        let mut first = true;
        tracing::info!("Telegram: опрос бота");
        while self.epoch.load(Ordering::SeqCst) == epoch {
            let r: anyhow::Result<Vec<serde_json::Value>> = self
                .call("getUpdates", serde_json::json!({ "offset": offset, "timeout": 50, "allowed_updates": ["message"] }))
                .await;
            if self.epoch.load(Ordering::SeqCst) != epoch {
                break;
            }
            match r {
                Ok(updates) => {
                    for u in updates {
                        offset = offset.max(u.get("update_id").and_then(|x| x.as_i64()).unwrap_or(0) + 1);
                        // Накопленное за время простоя (старше 10 минут) — мимо.
                        let date = u.pointer("/message/date").and_then(|d| d.as_u64()).unwrap_or(0) * 1000;
                        if first && date + 10 * 60 * 1000 < crate::hub::now_ms() {
                            continue;
                        }
                        if let Some(m) = u.get("message") {
                            self.handle(&hub, m).await;
                        }
                    }
                    first = false;
                    if self.owner().is_some() && hub.channel_state(ID) != ChannelState::Online {
                        hub.set_channel(ID, |c| {
                            c.state = ChannelState::Online;
                            c.error = None;
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!("Telegram: {e:#}");
                    let msg = format!("{e:#}");
                    if msg.contains("Unauthorized") || msg.contains("Not Found") {
                        hub.set_channel(ID, |c| {
                            c.state = ChannelState::Error;
                            c.error = Some("Telegram не принимает токен бота — укажите новый в Параметрах".into());
                        });
                        return;
                    }
                    hub.set_channel(ID, |c| c.error = Some(msg));
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn handle(&self, hub: &Arc<Hub>, m: &serde_json::Value) {
        let chat = m.pointer("/chat/id").and_then(|c| c.as_i64()).unwrap_or(0);
        let text = m.get("text").and_then(|t| t.as_str()).unwrap_or("").trim().to_string();
        let name = m.pointer("/from/username").and_then(|u| u.as_str()).map(|u| format!("@{u}")).or_else(|| m.pointer("/from/first_name").and_then(|u| u.as_str()).map(str::to_string)).unwrap_or_default();
        match self.owner() {
            None => {
                let code = self.code.lock().unwrap().clone();
                let given = text.strip_prefix("/start").map(str::trim).unwrap_or("");
                if !code.is_empty() && given == code {
                    {
                        let mut st = self.st.lock().unwrap();
                        st.owner = Some(chat);
                        st.owner_name = name.clone();
                        save(&st);
                    }
                    tracing::info!("Telegram: бот привязан ({name})");
                    hub.set_channel(ID, |c| {
                        c.state = ChannelState::Online;
                        c.qr = None;
                    });
                    hub.set_last_channel(ID);
                    let _ = self.send_text_to(chat, "✅ Курьер подключён. Пишите — ответит Claude Code; /help — команды.").await;
                } else {
                    // Чужим — молча: не выдавать, что бот кому-то служит.
                    tracing::info!("Telegram: сообщение не от владельца (до привязки) — пропущено");
                }
                return;
            }
            Some(owner) if owner != chat => {
                tracing::info!("Telegram: сообщение не от владельца — пропущено");
                return;
            }
            Some(_) => {}
        }
        hub.set_last_channel(ID);
        if !text.is_empty() {
            // «/start» владельца — подсказка; «/cmd@бот» — просто «/cmd».
            if text == "/start" || text.starts_with("/start ") {
                hub.incoming(ID, "/help".into());
                return;
            }
            let text = strip_bot_mention(&text);
            hub.incoming(ID, text);
            return;
        }
        let caption = m.get("caption").and_then(|t| t.as_str()).unwrap_or("").to_string();
        // (file_id, имя, что это, голосовое, размер)
        let file: Option<(String, String, &str, bool, u64)> = if let Some(v) = m.get("voice") {
            Some((str_of(v, "file_id"), "голосовое.ogg".into(), "голосовое", true, u64_of(v, "file_size")))
        } else if let Some(p) = m.get("photo").and_then(|p| p.as_array()).and_then(|a| a.last()) {
            Some((str_of(p, "file_id"), "фото.jpg".into(), "фото", false, u64_of(p, "file_size")))
        } else if let Some(d) = m.get("document") {
            let n = d.get("file_name").and_then(|x| x.as_str()).unwrap_or("документ").to_string();
            Some((str_of(d, "file_id"), n, "документ", false, u64_of(d, "file_size")))
        } else if let Some(v) = m.get("video") {
            Some((str_of(v, "file_id"), "видео.mp4".into(), "видео", false, u64_of(v, "file_size")))
        } else if let Some(a) = m.get("audio") {
            let n = a.get("file_name").and_then(|x| x.as_str()).unwrap_or("аудио.mp3").to_string();
            Some((str_of(a, "file_id"), n, "аудио", false, u64_of(a, "file_size")))
        } else if let Some(v) = m.get("video_note") {
            Some((str_of(v, "file_id"), "кружок.mp4".into(), "видеосообщение", false, u64_of(v, "file_size")))
        } else {
            None
        };
        let Some((id, name, what, voice, size)) = file else { return };
        if size > DOWNLOAD_MAX {
            hub.say(&format!("⚠️ {what} больше 20 МБ — Telegram не отдаёт ботам такие файлы.")).await;
            return;
        }
        match self.download(&id).await {
            Ok(bytes) => crate::media::deliver(hub, ID, &bytes, &name, &caption, what, voice).await,
            Err(e) => hub.say(&format!("⚠️ Не скачал {what}: {e:#}")).await,
        }
    }

    async fn download(&self, file_id: &str) -> anyhow::Result<Vec<u8>> {
        let f: serde_json::Value = self.call("getFile", serde_json::json!({ "file_id": file_id })).await?;
        let path = f.get("file_path").and_then(|p| p.as_str()).context("нет пути файла")?;
        let url = format!("{API}/file/bot{}/{path}", self.token());
        let r = self.http.get(&url).send().await?;
        anyhow::ensure!(r.status().is_success(), "HTTP {}", r.status());
        Ok(r.bytes().await?.to_vec())
    }

    async fn send_text_to(&self, chat: i64, text: &str) -> anyhow::Result<()> {
        let html: anyhow::Result<serde_json::Value> = self.call("sendMessage", serde_json::json!({ "chat_id": chat, "text": to_html(text), "parse_mode": "HTML" })).await;
        if html.is_err() {
            let _: serde_json::Value = self.call("sendMessage", serde_json::json!({ "chat_id": chat, "text": text })).await?;
        }
        Ok(())
    }

    pub async fn send_text(&self, text: &str) -> anyhow::Result<()> {
        let chat = self.owner().context("бот Telegram не привязан")?;
        self.send_text_to(chat, text).await
    }

    /// Файл владельцу: фото, видео, аудио, голосовое или документ.
    pub async fn send_file(&self, path: &std::path::Path, caption: &str, voice: bool, as_file: bool) -> anyhow::Result<()> {
        let chat = self.owner().context("бот Telegram не привязан")?;
        let data = tokio::fs::read(path).await.with_context(|| path.display().to_string())?;
        anyhow::ensure!(data.len() <= 50 * 1024 * 1024, "файл больше 50 МБ — предел Telegram для ботов");
        let mime = crate::media::mime_for(path);
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "файл".into());
        let (method, field) = if voice {
            ("sendVoice", "voice")
        } else if as_file {
            ("sendDocument", "document")
        } else if mime.starts_with("image/") && mime != "image/gif" {
            ("sendPhoto", "photo")
        } else if mime.starts_with("video/") {
            ("sendVideo", "video")
        } else if mime.starts_with("audio/") {
            ("sendAudio", "audio")
        } else {
            ("sendDocument", "document")
        };
        let r = self.upload(chat, method, field, data.clone(), &name, mime, caption).await;
        match r {
            // Фото/видео не прошло (IMAGE_PROCESS_FAILED, размеры) — тем же файлом документом.
            Err(e) if method != "sendDocument" && method != "sendVoice" => {
                tracing::info!("Telegram: {method} не прошёл ({e:#}) — отправляю документом");
                self.upload(chat, "sendDocument", "document", data, &name, mime, caption).await
            }
            r => r,
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn upload(&self, chat: i64, method: &str, field: &str, data: Vec<u8>, name: &str, mime: &str, caption: &str) -> anyhow::Result<()> {
        let part = reqwest::multipart::Part::bytes(data).file_name(name.to_string()).mime_str(mime.split(';').next().unwrap_or("application/octet-stream"))?;
        let mut form = reqwest::multipart::Form::new().text("chat_id", chat.to_string()).part(field.to_string(), part);
        if !caption.trim().is_empty() {
            form = form.text("caption", caption.trim().to_string());
        }
        let url = format!("{API}/bot{}/{method}", self.token());
        let r: ApiResp<serde_json::Value> = self.http.post(&url).multipart(form).send().await?.json().await?;
        anyhow::ensure!(r.ok, "{}", r.description.unwrap_or_else(|| "Telegram не принял файл".into()));
        Ok(())
    }

    /// Отвязать: забыть владельца и токен.
    pub async fn unlink(&self, hub: &Arc<Hub>) {
        self.stop();
        *self.st.lock().unwrap() = Stored::default();
        let _ = std::fs::remove_file(store_path());
        hub.set_channel(ID, |c| {
            c.state = ChannelState::Unlinked;
            c.qr = None;
            c.account = None;
            c.error = None;
        });
        tracing::info!("Telegram отвязан");
    }
}

fn str_of(v: &serde_json::Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn u64_of(v: &serde_json::Value, k: &str) -> u64 {
    v.get(k).and_then(|x| x.as_u64()).unwrap_or(0)
}

/// `/to@my_bot ИМЯ …` → `/to ИМЯ …`.
pub fn strip_bot_mention(text: &str) -> String {
    if let Some(rest) = text.strip_prefix('/') {
        let (cmd, tail) = rest.split_once(char::is_whitespace).map(|(c, t)| (c, Some(t))).unwrap_or((rest, None));
        if let Some((c, _)) = cmd.split_once('@') {
            return match tail {
                Some(t) => format!("/{c} {t}"),
                None => format!("/{c}"),
            };
        }
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn mention() {
        assert_eq!(super::strip_bot_mention("/to@my_bot hex привет"), "/to hex привет");
        assert_eq!(super::strip_bot_mention("/help@my_bot"), "/help");
        assert_eq!(super::strip_bot_mention("обычный текст"), "обычный текст");
        assert_eq!(super::pair_code().len(), 12);
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn inline_html(line: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            if let Some(j) = (i + 1..chars.len()).find(|&j| chars[j] == '`') {
                if j > i + 1 {
                    let inner: String = chars[i + 1..j].iter().collect();
                    out.push_str(&format!("<code>{}</code>", escape_html(&inner)));
                    i = j + 1;
                    continue;
                }
            }
        }
        if c == '*' && i + 1 < chars.len() && !chars[i + 1].is_whitespace() && (i == 0 || !chars[i - 1].is_alphanumeric()) {
            let close = (i + 1..chars.len()).find(|&j| chars[j] == '*' && !chars[j - 1].is_whitespace() && (j + 1 == chars.len() || !chars[j + 1].is_alphanumeric()));
            if let Some(j) = close {
                if j > i + 1 {
                    let inner: String = chars[i + 1..j].iter().collect();
                    out.push_str(&format!("<b>{}</b>", inline_html(&inner)));
                    i = j + 1;
                    continue;
                }
            }
        }
        out.push_str(&escape_html(&c.to_string()));
        i += 1;
    }
    out
}

pub fn to_html(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut code: Option<Vec<String>> = None;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            match code.take() {
                Some(block) => out.push(format!("<pre>{}</pre>", escape_html(&block.join("\n")))),
                None => code = Some(Vec::new()),
            }
            continue;
        }
        match code.as_mut() {
            Some(block) => block.push(line.to_string()),
            None => out.push(inline_html(line)),
        }
    }
    if let Some(block) = code {
        out.push(format!("<pre>{}</pre>", escape_html(&block.join("\n"))));
    }
    out.join("\n")
}

#[cfg(test)]
mod html_tests {
    use super::to_html;

    #[test]
    fn table_and_bold() {
        let t = "*отчёт 1*\n```\nls  58/315 → 72/308\n<x> & y\n```\n* скелет — новое, `a<b`";
        assert_eq!(to_html(t), "<b>отчёт 1</b>\n<pre>ls  58/315 → 72/308\n&lt;x&gt; &amp; y</pre>\n* скелет — новое, <code>a&lt;b</code>");
    }

    #[test]
    fn unclosed() {
        assert_eq!(to_html("2*3 = 6, a*b"), "2*3 = 6, a*b");
        assert_eq!(to_html("```\nкод"), "<pre>код</pre>");
    }
}
