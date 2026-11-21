//! Телефонный сеанс Claude Code: долгоживущий `claude -p` с потоковым вводом
//! и выводом (`stream-json`). Сообщения пользователя — во ввод, итог хода
//! (`result`) — пользователю. Сеанс живой и именованный («courier»): другие
//! сеансы пишут ему `SendMessage`, их сообщения тоже дают ход, и итог уходит
//! пользователю. Номер разговора хранится — после перезапуска службы
//! разговор продолжается (`--resume`).

use std::process::Stdio;
use std::sync::{Arc, Mutex};

use syn_courier::{ClaudeConfig, ClaudeStatus};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;

/// Что сеанс знает о своей роли.
const ROLE: &str = "Ты — телефонный сеанс «Курьера» synshell. Пользователь пишет тебе из WhatsApp \
(чат «Сообщения себе»); итоговый текст каждого твоего хода уходит ему туда же. Пиши коротко и по делу, \
обычным текстом: без таблиц и заголовков Markdown, выделение — *звёздочками*, как в WhatsApp, код — в ```. \
Ты такой же сеанс Claude Code, как остальные на этой машине: можешь работать с файлами и системой. \
Другие сеансы Claude видны через ListAgents; сообщение им — SendMessage. Когда пользователь просит передать \
что-то сеансу — передай дословно и потом перескажи пользователю ответ. Сообщения, пришедшие от других \
сеансов (cross-session-message), кратко перескажи пользователю: кто и что сообщил. \
Вложения пользователя приходят строкой «[Пользователь прислал фото/документ/…: ПУТЬ]» — файл уже на диске, открой его \
(картинки читай инструментом Read). Голосовые уже распознаны (службой synaptix) и приходят с пометкой «[голосовое]». \
Чтобы отправить пользователю файл (снимок экрана, картинку, документ, архив) — выполни `syncourier send ПУТЬ [подпись]`. \
Пользователь просил иногда отвечать ему голосом: `syncourier say ТЕКСТ` (синтез речи synaptix). Отвечай голосом прежде всего \
на голосовые и когда ответ короткий и разговорный (пара фраз); код, ссылки, пути, списки и длинное — текстом. Текст для \
голоса пиши для слуха: без разметки, без ссылок и путей, числа словами там, где так понятнее. Если ответил голосом — \
не дублируй его текстом и не сообщай «ответил голосом»: итоговый текст хода оставь пустым, \
кроме того, что голосом не передать (путь, ссылка, код). \
Снимок всего экрана: `synshot --full /tmp/screen.png`, затем `syncourier send /tmp/screen.png`.";

#[derive(Default)]
struct State {
    cfg: ClaudeConfig,
    /// Папка, заданная `/new ПАПКА` (до перезапуска службы).
    cwd_override: Option<String>,
    child: Option<Child>,
    session_id: Option<String>,
    busy: bool,
    error: Option<String>,
    /// Номер запуска: читатель старого процесса не трогает состояние нового.
    gen: u64,
}

pub struct Session {
    st: Arc<Mutex<State>>,
    stdin: Arc<tokio::sync::Mutex<Option<ChildStdin>>>,
    tx: mpsc::UnboundedSender<String>,
    rx: Mutex<Option<mpsc::UnboundedReceiver<String>>>,
}

fn state_file() -> std::path::PathBuf {
    syn_courier::data_dir().join("claude.json")
}

fn load_session_id() -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(state_file()).ok()?).ok()?;
    v.get("session_id")?.as_str().map(str::to_string)
}

fn save_session_id(id: Option<&str>) {
    let v = serde_json::json!({ "session_id": id });
    let _ = std::fs::write(state_file(), v.to_string());
}

/// Программа по имени: PATH службы systemd короче, чем у сеанса, — смотрим и `~/.local/bin`.
pub fn resolve_program(name: &str) -> std::path::PathBuf {
    if name.contains('/') {
        return syn_courier::expand_home(name);
    }
    let path = std::env::var("PATH").unwrap_or_default();
    let extra = [syn_courier::expand_home("~/.local/bin"), syn_courier::expand_home("~/.claude/local"), "/usr/local/bin".into(), "/usr/bin".into()];
    std::env::split_paths(&path).chain(extra).map(|d| d.join(name)).find(|p| p.is_file()).unwrap_or_else(|| name.into())
}

/// Строка ввода `stream-json`: сообщение пользователя.
pub fn user_line(text: &str) -> String {
    let mut s = serde_json::json!({ "type": "user", "message": { "role": "user", "content": text } }).to_string();
    s.push('\n');
    s
}

/// Что вынуть из строки вывода `stream-json`.
#[derive(Debug, PartialEq)]
pub enum Out {
    Init(String),
    /// Claude в этом ходе ответил голосом (`syncourier say`).
    SaidByVoice,
    Result { text: String, error: bool },
    Other,
}

pub fn parse_line(line: &str) -> Out {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { return Out::Other };
    let t = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match t {
        "system" if v.get("subtype").and_then(|s| s.as_str()) == Some("init") => {
            v.get("session_id").and_then(|s| s.as_str()).map(|s| Out::Init(s.to_string())).unwrap_or(Out::Other)
        }
        "assistant" => {
            let said = v.pointer("/message/content").and_then(|c| c.as_array()).is_some_and(|parts| {
                parts.iter().any(|p| {
                    p.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                        && p.pointer("/input/command").and_then(|c| c.as_str()).is_some_and(|c| c.contains("syncourier say"))
                })
            });
            if said {
                Out::SaidByVoice
            } else {
                Out::Other
            }
        }
        "result" => Out::Result {
            text: v.get("result").and_then(|r| r.as_str()).unwrap_or("").to_string(),
            error: v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false),
        },
        _ => Out::Other,
    }
}

/// Итог хода после голосового ответа — лишний, если в нём нет того, что голосом не передать.
pub fn redundant_after_voice(text: &str) -> bool {
    let t = text.trim();
    t.chars().count() <= 200 && !t.contains("```") && !t.contains("http") && !t.contains('/') && !t.contains('~')
}

impl Session {
    pub fn new(cfg: &ClaudeConfig) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let st = State { cfg: cfg.clone(), session_id: load_session_id(), ..Default::default() };
        Self { st: Arc::new(Mutex::new(st)), stdin: Arc::new(tokio::sync::Mutex::new(None)), tx, rx: Mutex::new(Some(rx)) }
    }

    /// Ответы сеанса (итоги ходов) — забирает узел.
    pub fn take_replies(&self) -> mpsc::UnboundedReceiver<String> {
        self.rx.lock().unwrap().take().expect("ответы уже забраны")
    }

    pub fn reconfigure(&self, cfg: &ClaudeConfig) {
        self.st.lock().unwrap().cfg = cfg.clone();
    }

    pub fn status(&self) -> ClaudeStatus {
        let st = self.st.lock().unwrap();
        ClaudeStatus {
            enabled: st.cfg.enabled,
            name: st.cfg.name.clone(),
            cwd: st.cwd_override.clone().unwrap_or_else(|| st.cfg.cwd.clone()),
            running: st.child.is_some(),
            busy: st.busy,
            session_id: st.session_id.clone(),
            error: st.error.clone(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.st.lock().unwrap().cfg.enabled
    }

    /// Запустить процесс, если не запущен.
    async fn ensure(&self) -> anyhow::Result<()> {
        let mut stdin = self.stdin.lock().await;
        if stdin.is_some() && self.st.lock().unwrap().child.is_some() {
            return Ok(());
        }
        let (cfg, cwd, resume, gen) = {
            let mut st = self.st.lock().unwrap();
            st.gen += 1;
            (st.cfg.clone(), st.cwd_override.clone().unwrap_or_else(|| st.cfg.cwd.clone()), st.session_id.clone(), st.gen)
        };
        let mut cmd = Command::new(resolve_program(&cfg.program));
        cmd.args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"]);
        cmd.args(["--name", &cfg.name]);
        cmd.args(["--append-system-prompt", ROLE]);
        if !cfg.model.is_empty() {
            cmd.args(["--model", &cfg.model]);
        }
        match cfg.permission_mode.as_str() {
            "" => {}
            "bypassPermissions" => {
                cmd.arg("--dangerously-skip-permissions");
            }
            m => {
                cmd.args(["--permission-mode", m]);
            }
        }
        if let Some(id) = &resume {
            cmd.args(["--resume", id]);
        }
        let dir = syn_courier::expand_home(&cwd);
        cmd.current_dir(if dir.is_dir() { dir } else { syn_courier::expand_home("~") });
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| anyhow::anyhow!("{}: {e}", cfg.program))?;
        tracing::info!("Claude: сеанс «{}» запущен{}", cfg.name, resume.as_ref().map(|r| format!(" (продолжение {r})")).unwrap_or_default());
        *stdin = child.stdin.take();
        let out = child.stdout.take();
        let err = child.stderr.take();
        {
            let mut st = self.st.lock().unwrap();
            st.child = Some(child);
            st.error = None;
        }
        // Вывод: номер разговора, итоги ходов.
        if let Some(out) = out {
            let st = self.st.clone();
            let tx = self.tx.clone();
            let stdin_slot = self.stdin.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(out).lines();
                let mut voiced = false;
                while let Ok(Some(line)) = lines.next_line().await {
                    match parse_line(&line) {
                        Out::Init(id) => {
                            let mut s = st.lock().unwrap();
                            if s.gen == gen && s.session_id.as_deref() != Some(&id) {
                                s.session_id = Some(id.clone());
                                save_session_id(Some(&id));
                            }
                        }
                        Out::SaidByVoice => voiced = true,
                        Out::Result { text, error } => {
                            st.lock().unwrap().busy = false;
                            let text = if std::mem::take(&mut voiced) && !error && redundant_after_voice(&text) { String::new() } else { text };
                            let text = if error { format!("⚠️ {text}") } else { text };
                            // Пустой итог — Claude уже ответил голосом или файлом.
                            if !text.trim().is_empty() {
                                let _ = tx.send(text);
                            }
                        }
                        Out::Other => {}
                    }
                }
                // Процесс кончился.
                let ours = {
                    let mut s = st.lock().unwrap();
                    if s.gen == gen {
                        if let Some(mut c) = s.child.take() {
                            let _ = c.start_kill();
                        }
                        s.busy = false;
                        true
                    } else {
                        false
                    }
                };
                if ours {
                    *stdin_slot.lock().await = None;
                }
                tracing::info!("Claude: сеанс завершился");
            });
        }
        if let Some(err) = err {
            let st = self.st.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(err).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!("claude: {line}");
                    let mut s = st.lock().unwrap();
                    if s.gen == gen && !line.trim().is_empty() {
                        s.error = Some(line.chars().take(300).collect());
                    }
                }
            });
        }
        Ok(())
    }

    /// Сообщение пользователя → сеанс.
    pub async fn send(&self, text: &str) -> anyhow::Result<()> {
        // Разговор, которого больше нет (стёрт), — начать новый, а не падать раз за разом.
        for attempt in 0..2 {
            self.ensure().await?;
            let mut stdin = self.stdin.lock().await;
            let Some(w) = stdin.as_mut() else { continue };
            match w.write_all(user_line(text).as_bytes()).await {
                Ok(()) => {
                    let _ = w.flush().await;
                    self.st.lock().unwrap().busy = true;
                    return Ok(());
                }
                Err(e) if attempt == 0 => {
                    tracing::warn!("Claude: ввод закрыт ({e}), перезапуск");
                    *stdin = None;
                    drop(stdin);
                    self.stop().await;
                }
                Err(e) => return Err(e.into()),
            }
        }
        anyhow::bail!("сеанс Claude не запускается: {}", self.st.lock().unwrap().error.clone().unwrap_or_default())
    }

    /// Остановить процесс (разговор сохраняется — следующее сообщение его продолжит).
    pub async fn stop(&self) {
        *self.stdin.lock().await = None;
        let child = {
            let mut st = self.st.lock().unwrap();
            st.gen += 1;
            st.busy = false;
            st.child.take()
        };
        if let Some(mut c) = child {
            let _ = c.kill().await;
        }
    }

    /// Новый разговор (`/new [папка]`).
    pub async fn reset(&self, cwd: Option<String>) {
        self.stop().await;
        let mut st = self.st.lock().unwrap();
        st.session_id = None;
        if cwd.is_some() {
            st.cwd_override = cwd;
        }
        save_session_id(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_json() {
        assert_eq!(parse_line(r#"{"type":"system","subtype":"init","session_id":"abc"}"#), Out::Init("abc".into()));
        assert_eq!(parse_line(r#"{"type":"result","result":"Готов.","is_error":false}"#), Out::Result { text: "Готов.".into(), error: false });
        assert_eq!(parse_line(r#"{"type":"assistant"}"#), Out::Other);
        assert_eq!(
            parse_line(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"syncourier say 'Привет'"}}]}}"#),
            Out::SaidByVoice
        );
        assert!(redundant_after_voice("Ответил голосом. Сейчас 13:23."));
        assert!(!redundant_after_voice("Файл: ~/Загрузки/отчёт.pdf"));
        assert_eq!(parse_line("мусор"), Out::Other);
        let l = user_line("привет \"мир\"");
        assert!(l.ends_with('\n'));
        let v: serde_json::Value = serde_json::from_str(l.trim()).unwrap();
        assert_eq!(v["message"]["content"], "привет \"мир\"");
    }
}
