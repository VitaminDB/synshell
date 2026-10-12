use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

use syngui::prelude::*;
use syngui::widgets::visual::MarkdownView;
use synassist::agent::{Agent, AgentEvent, DaemonBackend};
use synassist::chat::{Chat, Role, Store};

use crate::ui::{icon, rx};
use crate::ShellCtx;

#[derive(Clone, Copy)]
struct Panel {
    query: RwSignal<String>,
    answer: RwSignal<String>,
    tool: RwSignal<Option<String>>,
    confirm: RwSignal<Option<String>>,
    busy: RwSignal<bool>,
    chat_id: RwSignal<Option<String>>,
    input_rev: RwSignal<u64>,
}

thread_local! {
    static PANEL: std::cell::Cell<Option<Panel>> = const { std::cell::Cell::new(None) };
}

struct Shared {
    cancel: Arc<AtomicBool>,
    answer: Mutex<Option<Sender<bool>>>,
}

fn shared() -> &'static Shared {
    static S: std::sync::OnceLock<Shared> = std::sync::OnceLock::new();
    S.get_or_init(|| Shared { cancel: Arc::new(AtomicBool::new(false)), answer: Mutex::new(None) })
}

fn panel() -> Panel {
    PANEL.with(|p| match p.get() {
        Some(v) => v,
        None => {
            let v = Panel {
                query: use_signal(String::new()),
                answer: use_signal(String::new()),
                tool: use_signal(None),
                confirm: use_signal(None),
                busy: use_signal(false),
                chat_id: use_signal(None),
                input_rev: use_signal(0),
            };
            p.set(Some(v));
            v
        }
    })
}

fn reply(p: Panel, yes: bool) {
    if let Some(tx) = shared().answer.lock().unwrap().take() {
        let _ = tx.send(yes);
    }
    p.confirm.set(None);
}

fn ask(p: Panel, text: String) {
    let text = text.trim().to_string();
    if text.is_empty() || p.busy.get_untracked() {
        return;
    }
    p.busy.set(true);
    p.answer.set(String::new());
    p.tool.set(None);
    p.query.set(String::new());
    p.input_rev.set(p.input_rev.get_untracked() + 1);
    shared().cancel.store(false, Ordering::Relaxed);
    let store = Store::open();
    let mut chat = p.chat_id.get_untracked().and_then(|id| store.load(&id)).unwrap_or_else(Chat::new);
    std::thread::Builder::new()
        .name("assistant".into())
        .spawn(move || {
            let cfg = synshell_common::Config::load().0.assistant;
            let agent = Agent::configured(Arc::new(DaemonBackend), &cfg.disabled_tools, cfg.auto_approve, cfg.autocompact);
            let confirm = move |_: &str, summary: &str| -> bool {
                let (tx, rx) = channel();
                *shared().answer.lock().unwrap() = Some(tx);
                let s = summary.to_string();
                syngui::async_runtime::run_on_main_thread(move || p.confirm.set(Some(s)));
                rx.recv().unwrap_or(false)
            };
            let cancel = shared().cancel.clone();
            let r = agent.run_turn(&mut chat, &text, &confirm, &cancel, &mut |ev| {
                syngui::async_runtime::run_on_main_thread(move || match ev {
                    AgentEvent::Text(t) => {
                        let mut a = p.answer.get_untracked();
                        a.push_str(&t);
                        p.answer.set(a);
                    }
                    AgentEvent::ToolStart { summary, .. } => p.tool.set(Some(summary)),
                    AgentEvent::Message(m) if m.role == Role::Assistant && !m.tool_calls.is_empty() => p.answer.set(String::new()),
                    _ => {}
                });
            });
            let _ = store.save(&chat);
            let id = chat.id.clone();
            syngui::async_runtime::run_on_main_thread(move || {
                if let Err(e) = r {
                    p.answer.set(format!("**Ошибка:** {e}"));
                }
                p.chat_id.set(Some(id));
                p.tool.set(None);
                p.busy.set(false);
            });
        })
        .ok();
}

fn open_window(p: Panel) {
    let id = p.chat_id.get_untracked();
    let cmd = match id {
        Some(id) => format!("synassistant --chat {id}"),
        None => "synassistant".into(),
    };
    crate::actions::spawn(&cmd);
    ShellCtx::get().close_popup();
}

pub fn view(_ctx: ShellCtx) -> impl Widget {
    let p = panel();
    let field = rx(move || {
        p.input_rev.get();
        let q = p.query;
        Box::new(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(icon("\u{E65F}").class("assist-icon"))
                .child(
                    TextField::new()
                        .placeholder(t!("Спросите ассистента…"))
                        .autofocus(true)
                        .on_change(move |t| q.set(t.to_string()))
                        .on_submit(move |t| ask(p, t.to_string()))
                        .class("search-field grow"),
                ),
        )
    });
    let body = rx(move || {
        let answer = p.answer.get();
        let busy = p.busy.get();
        let tool = p.tool.get();
        let confirm = p.confirm.get();
        let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        if let Some(s) = confirm {
            col = col.child(
                Column::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .class("assist-confirm")
                    .child(Text::new(t!("Разрешить действие?")).class("assist-confirm-title"))
                    .child(Text::new(s).max_lines(4).class("assist-confirm-sum"))
                    .child(
                        Row::new()
                            .gap(8.0)
                            .main_axis_alignment(MainAxisAlignment::End)
                            .child(Button::new(t!("Отклонить")).class("btn").on_click(move || reply(p, false)))
                            .child(Button::new(t!("Разрешить")).class("btn btn-primary").on_click(move || reply(p, true))),
                    ),
            );
        } else if let Some(t) = tool.filter(|_| busy && answer.is_empty()) {
            col = col.child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon("\u{E869}").class("assist-tool-icon")).child(Text::new(t).max_lines(1).class("assist-tool grow")));
        } else if busy && answer.is_empty() {
            col = col.child(Text::new("•••").class("assist-typing"));
        }
        if !answer.is_empty() {
            col = col.child(ScrollView::new().vertical().follow_end(true).class("assist-scroll").child(MarkdownView::new(answer).with_copy_code(true).selectable(true).class("assist-answer")));
        }
        if !busy && p.chat_id.get().is_some() {
            col = col.child(
                Row::new()
                    .gap(8.0)
                    .main_axis_alignment(MainAxisAlignment::End)
                    .child(Button::new(t!("Новый вопрос")).class("btn").on_click(move || {
                        p.chat_id.set(None);
                        p.answer.set(String::new());
                        p.input_rev.set(p.input_rev.get_untracked() + 1);
                    }))
                    .child(Button::new(t!("Открыть чат")).class("btn btn-primary").on_click(move || open_window(p))),
            );
        }
        if col_is_empty(busy, &p) {
            return Box::new(Text::new(t!("Управление системой, файлы, интернет и медиа — на нейросетях этого устройства.")).max_lines(2).class("assist-hint")) as Box<dyn Widget>;
        }
        Box::new(col)
    });
    Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(DecoratedBox::new().child(field).class("search-box"))
        .child(AnimatedSize::new(body).axis(AnimationAxis::Height).spring(420.0, 40.0).class("popup-morph"))
}

fn col_is_empty(busy: bool, p: &Panel) -> bool {
    !busy && p.answer.get_untracked().is_empty() && p.confirm.get_untracked().is_none() && p.chat_id.get_untracked().is_none()
}
