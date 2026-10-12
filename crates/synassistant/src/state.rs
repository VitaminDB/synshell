use std::path::PathBuf;
use std::result::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

use syngui::prelude::*;
use synassist_voice::VisHandle;
pub use synassist_voice::Phase;
use synassist::agent::{Agent, AgentEvent, DaemonBackend};
use synassist::chat::{Chat, ChatMsg, ChatSummary, Role, Store};
use synshell_common::config::Assistant;
use synshell_common::Config;

#[derive(Clone, PartialEq, Default)]
pub struct Live {
    pub text: String,
    pub thinking: String,
    pub tool_text: String,
    pub tool: Option<(String, String)>,
}

#[derive(Clone, Copy)]
pub struct Voice {
    pub open: RwSignal<bool>,
    pub phase: RwSignal<Phase>,
    pub vis: RwSignal<Option<VisHandle>>,
    pub heard: RwSignal<String>,
    pub muted: RwSignal<bool>,
    pub dictating: RwSignal<bool>,
    pub speaking: RwSignal<bool>,
    pub note: RwSignal<Option<String>>,
}

#[derive(Clone, PartialEq, Default)]
pub struct Service {
    pub online: bool,
    pub llm: Option<String>,
    pub asr: Option<String>,
    pub tts: Option<String>,
}

#[derive(Clone, Copy)]
pub struct St {
    pub chats: RwSignal<Vec<ChatSummary>>,
    pub chat: RwSignal<Chat>,
    pub live: RwSignal<Option<Live>>,
    pub busy: RwSignal<bool>,
    pub confirm: RwSignal<Option<(String, String)>>,
    pub error: RwSignal<Option<String>>,
    pub input_rev: RwSignal<u64>,
    pub list_open: RwSignal<bool>,
    pub empty: RwSignal<bool>,
    pub sidebar: RwSignal<bool>,
    pub window: RwSignal<syngui::window::WindowState>,
    pub theme: RwSignal<String>,
    pub search: RwSignal<String>,
    pub models: RwSignal<Vec<(String, String)>>,
    pub service: RwSignal<Service>,
    pub menu_open: RwSignal<bool>,
    pub menu_pos: RwSignal<Point>,
    pub menu_chat: RwSignal<Option<String>>,
    pub rename: RwSignal<Option<String>>,
    pub delete_ask: RwSignal<Option<String>>,
    pub delete_open: RwSignal<bool>,
    pub toast: RwSignal<Option<(u64, String)>>,
    pub attachments: RwSignal<Vec<PathBuf>>,
    pub speak_answers: RwSignal<bool>,
    pub cfg: RwSignal<Assistant>,
    pub voice: Voice,
}

pub struct Shared {
    pub cancel: Arc<AtomicBool>,
    pub answer: Mutex<Option<Sender<bool>>>,
    pub insert: Arc<Mutex<Vec<String>>>,
    pub turn: Mutex<Option<Sender<Result<String, String>>>>,
}

pub fn shared() -> &'static Shared {
    static S: std::sync::OnceLock<Shared> = std::sync::OnceLock::new();
    S.get_or_init(|| Shared {
        cancel: Arc::new(AtomicBool::new(false)),
        answer: Mutex::new(None),
        insert: Arc::new(Mutex::new(Vec::new())),
        turn: Mutex::new(None),
    })
}

pub fn store() -> Store {
    Store::open()
}

fn plain(text: &str) -> String {
    crate::voice::strip_markdown(text)
}

impl St {
    pub fn new(cfg: &Config) -> Self {
        let chats = store().list();
        let chat = chats.first().and_then(|c| store().load(&c.id)).filter(|c| !c.messages.is_empty()).unwrap_or_else(Chat::new);
        let empty = chat.messages.is_empty();
        let st = Self {
            chats: use_signal(chats),
            chat: use_signal(chat),
            live: use_signal(None),
            busy: use_signal(false),
            confirm: use_signal(None),
            error: use_signal(None),
            input_rev: use_signal(0),
            list_open: use_signal(false),
            empty: use_signal(true),
            sidebar: use_signal(true),
            window: use_signal(syngui::window::WindowState::default()),
            theme: use_signal(String::new()),
            search: use_signal(String::new()),
            models: use_signal(Vec::new()),
            service: use_signal(Service::default()),
            menu_open: use_signal(false),
            menu_pos: use_signal(Point::zero()),
            menu_chat: use_signal(None),
            rename: use_signal(None),
            delete_ask: use_signal(None),
            delete_open: use_signal(false),
            toast: use_signal(None),
            attachments: use_signal(Vec::new()),
            speak_answers: use_signal(cfg.assistant.speak_answers),
            cfg: use_signal(cfg.assistant.clone()),
            voice: Voice {
                open: use_signal(false),
                phase: use_signal(Phase::Off),
                vis: use_signal(None),
                heard: use_signal(String::new()),
                muted: use_signal(false),
                dictating: use_signal(false),
                speaking: use_signal(false),
                note: use_signal(None),
            },
        };
        st.empty.set(empty);
        st
    }

    pub fn config_changed(self, cfg: Config) {
        let was = self.cfg.get_untracked();
        if was.speak_answers != cfg.assistant.speak_answers {
            self.speak_answers.set(cfg.assistant.speak_answers);
        }
        self.cfg.set(cfg.assistant);
    }

    pub fn start_background(self) {
        std::thread::Builder::new()
            .name("assistant-status".into())
            .spawn(move || loop {
                let (service, models) = probe_service();
                run_on_main_thread(move || {
                    if self.service.get_untracked() != service {
                        self.service.set(service);
                    }
                    if self.models.get_untracked() != models {
                        self.models.set(models);
                    }
                });
                std::thread::sleep(std::time::Duration::from_secs(10));
            })
            .ok();
    }

    pub fn refresh_list(self) {
        self.chats.set(store().list());
    }

    pub fn new_chat(self) {
        if self.busy.get_untracked() {
            return;
        }
        let mut c = Chat::new();
        c.thinking = self.chat.get_untracked().thinking;
        c.model = self.chat.get_untracked().model;
        self.chat.set(c);
        self.empty.set(true);
        self.error.set(None);
        self.attachments.set(Vec::new());
        self.list_open.set(false);
        self.input_rev.set(self.input_rev.get_untracked() + 1);
    }

    pub fn open(self, id: &str) {
        if self.busy.get_untracked() {
            return;
        }
        if let Some(c) = store().load(id) {
            self.empty.set(c.messages.is_empty());
            self.chat.set(c);
            self.error.set(None);
        }
        self.list_open.set(false);
    }

    pub fn ask_delete(self, id: &str) {
        self.delete_ask.set(Some(id.to_string()));
        self.delete_open.set(true);
    }

    pub fn delete(self, id: &str) {
        let _ = store().delete(id);
        if self.chat.get_untracked().id == id {
            self.chat.set(Chat::new());
            self.empty.set(true);
        }
        self.refresh_list();
    }

    pub fn rename_chat(self, id: &str, title: &str) {
        let title = title.trim();
        if let Some(mut c) = store().load(id) {
            c.title = title.to_string();
            let _ = store().save(&c);
        }
        if self.chat.get_untracked().id == id {
            let mut c = self.chat.get_untracked();
            c.title = title.to_string();
            self.chat.set(c);
        }
        self.rename.set(None);
        self.refresh_list();
    }

    fn save_chat(self, f: impl FnOnce(&mut Chat)) {
        let mut c = self.chat.get_untracked();
        f(&mut c);
        if !c.messages.is_empty() {
            let _ = store().save(&c);
        }
        self.chat.set(c);
    }

    pub fn set_thinking(self, on: bool) {
        self.save_chat(|c| c.thinking = on);
    }

    pub fn set_model(self, id: &str) {
        let id = id.to_string();
        self.save_chat(|c| c.model = id);
    }

    pub fn answer(self, yes: bool) {
        if let Some(tx) = shared().answer.lock().unwrap().take() {
            let _ = tx.send(yes);
        }
        self.confirm.set(None);
    }

    pub fn stop(self) {
        shared().cancel.store(true, Ordering::Relaxed);
        self.answer(false);
    }

    pub fn toast(self, text: impl Into<String>) {
        let id = synassist::chat::now() as u64;
        self.toast.set(Some((id, text.into())));
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(2600));
            run_on_main_thread(move || {
                if self.toast.get_untracked().map(|t| t.0) == Some(id) {
                    self.toast.set(None);
                }
            });
        });
    }

    pub fn copy(self, text: &str) {
        syngui::clipboard::copy(text);
        self.toast(t!("Скопировано"));
    }

    pub fn attach(self) {
        let home = synshell_common::paths::home();
        std::thread::spawn(move || {
            let r = synsystem::portal_files::pick(&t!("Прикрепить файл"), false, &home, &[]);
            run_on_main_thread(move || match r {
                Ok(Some(p)) => {
                    let mut v = self.attachments.get_untracked();
                    if !v.contains(&p) {
                        v.push(p);
                    }
                    self.attachments.set(v);
                }
                Ok(None) => {}
                Err(e) => self.toast(e),
            });
        });
    }

    pub fn remove_attachment(self, i: usize) {
        let mut v = self.attachments.get_untracked();
        if i < v.len() {
            v.remove(i);
        }
        self.attachments.set(v);
    }

    pub fn export(self) {
        let c = self.chat.get_untracked();
        if c.messages.is_empty() {
            return;
        }
        let mut md = format!("# {}\n\n", c.display_title());
        for m in c.messages.iter().filter(|m| !m.compacted) {
            match m.role {
                Role::User => md.push_str(&format!("**Вы:** {}\n\n", m.text.trim())),
                Role::Assistant if !m.text.trim().is_empty() => md.push_str(&format!("**Ассистент:** {}\n\n", m.text.trim())),
                Role::Tool => {
                    let name = m.tool.as_ref().map(|t| t.summary.clone()).unwrap_or_default();
                    md.push_str(&format!("> {} \n\n", name));
                }
                _ => {}
            }
        }
        let dir = documents_dir().join(t!("Ассистент").to_string());
        let safe: String = c.display_title().chars().map(|ch| if ch.is_alphanumeric() || ch == ' ' || ch == '-' { ch } else { '_' }).collect();
        let path = dir.join(format!("{}-{}.md", safe.trim(), &c.id));
        match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, md)) {
            Ok(()) => self.toast(t!("Сохранено: {p}", p = path.display())),
            Err(e) => self.toast(t!("Не удалось сохранить: {e}", e = e)),
        }
    }

    pub fn last_answer(self) -> Option<String> {
        let c = self.chat.get_untracked();
        c.messages.iter().rev().find(|m| m.role == Role::Assistant && !m.text.trim().is_empty() && !m.compacted).map(|m| m.text.clone())
    }

    pub fn regenerate(self) {
        if self.busy.get_untracked() {
            return;
        }
        let mut c = self.chat.get_untracked();
        let Some(i) = c.messages.iter().rposition(|m| m.role == Role::User && !m.compacted) else { return };
        let user = c.messages[i].clone();
        c.messages.truncate(i);
        self.chat.set(c);
        self.attachments.set(user.files.clone());
        self.send_with(user.text, false);
    }

    pub fn send(self, text: String) {
        self.send_with(text, false);
    }

    pub fn send_with(self, text: String, voice: bool) {
        let text = text.trim().to_string();
        if text.is_empty() || self.busy.get_untracked() {
            if voice {
                if let Some(tx) = shared().turn.lock().unwrap().take() {
                    let _ = tx.send(Err(t!("Ассистент занят").to_string()));
                }
            }
            return;
        }
        let files = self.attachments.get_untracked();
        self.attachments.set(Vec::new());
        self.busy.set(true);
        self.empty.set(false);
        self.error.set(None);
        self.live.set(Some(Live::default()));
        self.input_rev.set(self.input_rev.get_untracked() + 1);
        shared().cancel.store(false, Ordering::Relaxed);
        let speak_after = !voice && self.speak_answers.get_untracked();
        let mut chat = self.chat.get_untracked();
        let from = chat.messages.len();
        std::thread::Builder::new()
            .name("assistant".into())
            .spawn(move || {
                let cfg = Config::load().0.assistant;
                let mut agent = Agent::configured(Arc::new(DaemonBackend), &cfg.disabled_tools, cfg.auto_approve, cfg.autocompact);
                if voice {
                    agent.system = Some(synassist::prompt::build_voice(&agent.tools));
                }
                let confirm = move |tool: &str, summary: &str| -> bool {
                    let (tx, rx) = channel();
                    *shared().answer.lock().unwrap() = Some(tx);
                    let (t, s) = (tool.to_string(), summary.to_string());
                    run_on_main_thread(move || self.confirm.set(Some((t, s))));
                    rx.recv().unwrap_or(false)
                };
                let cancel = shared().cancel.clone();
                let r = agent.run_turn_files(&mut chat, &text, files, &confirm, &cancel, &mut |ev| {
                    run_on_main_thread(move || self.apply(ev));
                });
                let _ = store().save(&chat);
                let answer: String = chat
                    .messages
                    .iter()
                    .skip(from)
                    .filter(|m| m.role == Role::Assistant && !m.text.trim().is_empty())
                    .map(|m| m.text.trim().to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                let result = match &r {
                    Ok(()) => Ok(answer.clone()),
                    Err(e) => Err(e.clone()),
                };
                if let Some(tx) = shared().turn.lock().unwrap().take() {
                    let _ = tx.send(result);
                }
                run_on_main_thread(move || {
                    if let Err(e) = r {
                        self.error.set(Some(e));
                    }
                    self.chat.set(chat);
                    self.live.set(None);
                    self.busy.set(false);
                    self.confirm.set(None);
                    self.refresh_list();
                    if speak_after && !answer.is_empty() {
                        crate::voice::speak(self, plain(&answer));
                    }
                });
            })
            .ok();
    }

    fn apply(self, ev: AgentEvent) {
        match ev {
            AgentEvent::Text(t) => self.live_update(|l| l.text.push_str(&t)),
            AgentEvent::Thinking(t) => self.live_update(|l| l.thinking.push_str(&t)),
            AgentEvent::ToolText(t) => self.live_update(|l| l.tool_text.push_str(&t)),
            AgentEvent::ToolStart { name, summary } => self.live_update(|l| l.tool = Some((name, summary))),
            AgentEvent::Message(m) => self.push(m),
            AgentEvent::Compacted(msgs) => {
                let mut c = self.chat.get_untracked();
                c.messages = msgs;
                self.chat.set(c);
            }
            AgentEvent::Done => {}
        }
    }

    fn live_update(self, f: impl FnOnce(&mut Live)) {
        let mut l = self.live.get_untracked().unwrap_or_default();
        f(&mut l);
        self.live.set(Some(l));
    }

    fn push(self, m: ChatMsg) {
        let mut c = self.chat.get_untracked();
        c.messages.push(m);
        self.chat.set(c);
        self.live.set(Some(Live::default()));
    }
}

fn probe_service() -> (Service, Vec<(String, String)>) {
    let Ok(mut c) = synaptixd::Client::connect() else { return (Service::default(), Vec::new()) };
    let title = |r: synaptixd::Result<synaptixd::ModelEntry>| r.ok().map(|m| m.title);
    let llm = title(c.resolve(synaptixd::ModelKind::Llm));
    let asr = title(c.resolve(synaptixd::ModelKind::Asr));
    let tts = title(c.resolve(synaptixd::ModelKind::Tts));
    let models = c
        .list()
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.kind == synaptixd::ModelKind::Llm && m.runnable)
        .map(|m| (m.id, m.title))
        .collect();
    (Service { online: true, llm, asr, tts }, models)
}

fn documents_dir() -> PathBuf {
    let out = std::process::Command::new("xdg-user-dir").arg("DOCUMENTS").output().ok();
    let s = out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    if !s.is_empty() && std::path::Path::new(&s).is_dir() {
        return PathBuf::from(s);
    }
    synshell_common::paths::home()
}
