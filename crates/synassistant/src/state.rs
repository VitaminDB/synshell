use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

use syngui::prelude::*;
use synassist::agent::{Agent, AgentEvent, DaemonBackend};
use synassist::chat::{Chat, ChatMsg, ChatSummary, Store};

#[derive(Clone, PartialEq, Default)]
pub struct Live {
    pub text: String,
    pub thinking: String,
    pub tool_text: String,
    pub tool: Option<(String, String)>,
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
}

struct Shared {
    cancel: Arc<AtomicBool>,
    answer: Mutex<Option<Sender<bool>>>,
}

fn shared() -> &'static Shared {
    static S: std::sync::OnceLock<Shared> = std::sync::OnceLock::new();
    S.get_or_init(|| Shared { cancel: Arc::new(AtomicBool::new(false)), answer: Mutex::new(None) })
}

pub fn store() -> Store {
    Store::open()
}

impl St {
    pub fn new() -> Self {
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
        };
        st.empty.set(empty);
        st
    }

    pub fn refresh_list(self) {
        self.chats.set(store().list());
    }

    pub fn new_chat(self) {
        if self.busy.get_untracked() {
            return;
        }
        self.chat.set(Chat::new());
        self.empty.set(true);
        self.error.set(None);
        self.list_open.set(false);
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

    pub fn delete(self, id: &str) {
        let _ = store().delete(id);
        if self.chat.get_untracked().id == id {
            self.chat.set(Chat::new());
            self.empty.set(true);
        }
        self.refresh_list();
    }

    pub fn set_thinking(self, on: bool) {
        let mut c = self.chat.get_untracked();
        c.thinking = on;
        if !c.messages.is_empty() {
            let _ = store().save(&c);
        }
        self.chat.set(c);
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

    pub fn send(self, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() || self.busy.get_untracked() {
            return;
        }
        self.busy.set(true);
        self.empty.set(false);
        self.error.set(None);
        self.live.set(Some(Live::default()));
        self.input_rev.set(self.input_rev.get_untracked() + 1);
        shared().cancel.store(false, Ordering::Relaxed);
        let mut chat = self.chat.get_untracked();
        std::thread::Builder::new()
            .name("assistant".into())
            .spawn(move || {
                let cfg = synshell_common::Config::load().0.assistant;
                let mut agent = Agent::new(Arc::new(DaemonBackend), synassist::builtin::enabled(&cfg.disabled_tools));
                agent.auto_approve = cfg.auto_approve;
                let confirm = move |tool: &str, summary: &str| -> bool {
                    let (tx, rx) = channel();
                    *shared().answer.lock().unwrap() = Some(tx);
                    let (t, s) = (tool.to_string(), summary.to_string());
                    syngui::async_runtime::run_on_main_thread(move || self.confirm.set(Some((t, s))));
                    rx.recv().unwrap_or(false)
                };
                let cancel = shared().cancel.clone();
                let r = agent.run_turn(&mut chat, &text, &confirm, &cancel, &mut |ev| {
                    syngui::async_runtime::run_on_main_thread(move || self.apply(ev));
                });
                let _ = store().save(&chat);
                syngui::async_runtime::run_on_main_thread(move || {
                    if let Err(e) = r {
                        self.error.set(Some(e));
                    }
                    self.chat.set(chat);
                    self.live.set(None);
                    self.busy.set(false);
                    self.confirm.set(None);
                    self.refresh_list();
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
