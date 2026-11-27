//! Чаты агента в панели ИИ: несколько разговоров (новый, переключение,
//! удаление), у каждого проекта свои (`$XDG_STATE_HOME/syn-video-editor/projects/<ключ>.chats`). Шапка панели — выбор чата, модель, режим «Инструкт /
//! Думать» с глубиной и заполнение окна контекста.

use std::sync::Mutex;

use syn_assist::chat::{Chat, ChatMsg, ChatSummary, Role, Store};
use syngui::prelude::*;
use syngui::widgets::buttons::Segment;


use super::{icons, AiLine, St};

/// Текущий разговор (ход агента берёт его на время хода).
static CHAT: Mutex<Option<Chat>> = Mutex::new(None);

/// Чаты — у каждого проекта свои (`projstate`): новый проект без чатов,
/// открытый — со своими разговорами.
pub fn store_dir() -> std::path::PathBuf {
    crate::projstate::dir().join(format!("{}.chats", crate::projstate::current()))
}

pub fn store() -> Store {
    Store::at(store_dir())
}

/// Взять текущий разговор (последний сохранённый или новый).
pub fn take() -> Chat {
    let mut g = CHAT.lock().unwrap_or_else(|e| e.into_inner());
    g.take().unwrap_or_else(|| store().list().first().and_then(|c| store().load(&c.id)).unwrap_or_default_chat())
}

trait OrNew {
    fn unwrap_or_default_chat(self) -> Chat;
}

impl OrNew for Option<Chat> {
    fn unwrap_or_default_chat(self) -> Chat {
        self.unwrap_or_else(Chat::new)
    }
}

/// Вернуть разговор после хода и сохранить (пустой не пишется).
pub fn put(chat: Chat) {
    if !chat.messages.is_empty() {
        if let Err(e) = store().save(&chat) {
            tracing::warn!("чат не сохранён: {e}");
        }
    }
    *CHAT.lock().unwrap_or_else(|e| e.into_inner()) = Some(chat);
}

/// Лента панели из сообщений разговора.
pub fn lines(chat: &Chat) -> Vec<AiLine> {
    let mut out = Vec::new();
    for m in &chat.messages {
        if m.compacted {
            continue;
        }
        match m.role {
            Role::User => out.push(AiLine::User(m.text.clone())),
            Role::Assistant => {
                if !m.text.trim().is_empty() {
                    out.push(AiLine::Text(m.text.clone()));
                }
            }
            Role::Tool => {
                if let Some(t) = &m.tool {
                    out.push(AiLine::Tool(t.summary.clone()));
                }
                let first = m.text.lines().next().unwrap_or_default().to_string();
                out.push(if m.error.is_some() || m.tool.as_ref().is_some_and(|t| !t.ok) { AiLine::Error(first) } else { AiLine::Result(first) });
            }
            _ => {
                if summary_like(m) {
                    out.push(AiLine::Result(format!("(история сжата) {}", m.text.lines().next().unwrap_or_default())));
                }
            }
        }
    }
    out
}

fn summary_like(m: &ChatMsg) -> bool {
    m.covers > 0
}

/// Показать разговор в окне: лента, список чатов, контекст.
fn show(st: St, chat: &Chat) {
    st.ai_log.set(lines(chat));
    st.ai_chat.set(chat.id.clone());
    st.ai_ctx.set((chat.prompt_tokens, chat.context));
    st.ai_chats.set(store().list());
}

/// Загрузить последний разговор при открытии окна.
pub fn init(st: St) {
    // ключ проекта — до чтения чатов: окно открывается с файлом проекта, а
    // ключ ставили позже (панель графа) — грузились чаты «untitled»
    crate::projstate::set_current(&crate::projstate::key(super::ed().path.as_deref()));
    let chat = take();
    show(st, &chat);
    put_back_quiet(chat);
    load_info(st);
}

fn put_back_quiet(chat: Chat) {
    *CHAT.lock().unwrap_or_else(|e| e.into_inner()) = Some(chat);
}

/// После хода: обновить список, контекст.
pub fn after_turn(st: St, chat: &Chat) {
    st.ai_ctx.set((chat.prompt_tokens, chat.context));
    st.ai_chat.set(chat.id.clone());
    st.ai_chats.set(store().list());
}

/// Проект сменился — его чаты: последний разговор проекта или новый.
pub fn switch_project(st: St) {
    let chat = store().list().first().and_then(|c| store().load(&c.id)).unwrap_or_else(Chat::new);
    show(st, &chat);
    put_back_quiet(chat);
}

pub fn new_chat(st: St) {
    if st.ai_busy.get_untracked() {
        return;
    }
    let chat = Chat::new();
    show(st, &chat);
    put_back_quiet(chat);
}

pub fn open_chat(st: St, id: &str) {
    if st.ai_busy.get_untracked() || st.ai_chat.get_untracked() == id {
        return;
    }
    let Some(chat) = store().load(id) else { return };
    show(st, &chat);
    put_back_quiet(chat);
}

pub fn delete_chat(st: St) {
    if st.ai_busy.get_untracked() {
        return;
    }
    let id = st.ai_chat.get_untracked();
    let _ = store().delete(&id);
    let next = store().list().first().and_then(|c| store().load(&c.id)).unwrap_or_else(Chat::new);
    show(st, &next);
    put_back_quiet(next);
}

/// Сведения о модели агента (окно контекста, зрение, уровни размышлений) и
/// список LLM службы — в фоне.
pub fn load_info(st: St) {
    let model = crate::settings::load().ai_model;
    std::thread::spawn(move || {
        let (info, models) = match synaptixd::Client::connect() {
            Ok(mut c) => {
                let info = c.llm_info(&model).ok();
                let models = super::settings_page::models_of(&mut c, crate::settings::RoleKind::Llm);
                (info, models)
            }
            Err(_) => (None, Vec::new()),
        };
        syngui::async_runtime::run_on_main_thread(move || {
            st.ai_info.set(info);
            if !models.is_empty() {
                st.ai_models.set(models);
            }
        });
    });
}

pub fn level_title(l: &str) -> String {
    match l {
        "" => tr!("ai.level.auto"),
        "low" => tr!("ai.level.low"),
        "medium" => tr!("ai.level.medium"),
        "high" => tr!("ai.level.high"),
        "xhigh" => tr!("ai.level.max"),
        "none" => tr!("ai.level.none"),
        "minimal" => tr!("ai.level.minimal"),
        other => other.to_string(),
    }
}

fn short(n: usize) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

/// Шапка панели ИИ: чаты, модель, режим, контекст.
pub fn head(st: St) -> impl Widget {
    let chats_row = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let list: Vec<ChatSummary> = st.ai_chats.get();
        let cur = st.ai_chat.get();
        let busy = st.ai_busy.get();
        let mut items: Vec<DropdownItem> = list.iter().map(|c| DropdownItem::new(c.id.clone(), c.title.clone())).collect();
        if !list.iter().any(|c| c.id == cur) {
            items.insert(0, DropdownItem::new(cur.clone(), tr!("ai.chat.new_title")));
        }
        let dd = Dropdown::with_items(items).selected(cur).on_change(move |id: &str| open_chat(st, id));
        vec![Box::new(
            Row::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("grow").child(dd))
                .child(ToolButton::new(icons::ADD).tooltip(tr!("ai.chat.new")).disabled(busy).on_click(move || new_chat(st)))
                .child(ToolButton::new(icons::DELETE).tooltip(tr!("ai.chat.delete")).disabled(busy).on_click(move || delete_chat(st)))
                .child(ToolButton::new(icons::SETTINGS).tooltip(tr!("ai.settings")).on_click(move || super::settings_page::open_section(st, super::settings_page::SEC_AGENT))),
        )]
    });
    let mode_row = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.set_rev.get();
        let s = crate::settings::load();
        let info = st.ai_info.get();
        let models = st.ai_models.get();
        let cur = s.ai_model.clone();
        let mut items = vec![DropdownItem::new(String::new(), tr!("settings.model.default"))];
        items.extend(models.iter().map(|(id, title)| DropdownItem::new(id.clone(), if title.is_empty() { id.clone() } else { title.clone() })));
        if !cur.is_empty() && !models.iter().any(|(id, _)| *id == cur) {
            items.push(DropdownItem::new(cur.clone(), cur.clone()));
        }
        let model = Dropdown::with_items(items).selected(cur).on_change(move |id: &str| {
            let id = id.to_string();
            crate::settings::update(move |s| s.ai_model = id);
            st.set_rev.update(|r| *r += 1);
            load_info(st);
        });
        let thinking = s.ai_thinking;
        let mut row = Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(SegmentedButton::new(vec![Segment::new(tr!("ai.mode.instruct")), Segment::new(tr!("ai.mode.think"))]).selected(usize::from(thinking)).on_change(move |i| {
                crate::settings::update(move |s| s.ai_thinking = i == 1);
                st.set_rev.update(|r| *r += 1);
            }));
        let levels = info.as_ref().map(|i| i.reasoning_levels.clone()).unwrap_or_default();
        if thinking && !levels.is_empty() {
            // уровней бывает много (none, minimal, low…) — списком, а не сегментами
            let mut opts = vec![String::new()];
            opts.extend(levels);
            let items: Vec<DropdownItem> = opts.iter().map(|l| DropdownItem::new(l.clone(), level_title(l))).collect();
            row = row.child(Text::new(tr!("ai.depth")).class("ai-status-text")).child(DecoratedBox::new().class("grow").child(Dropdown::with_items(items).selected(s.ai_reasoning.clone()).on_change(move |l: &str| {
                let l = l.to_string();
                crate::settings::update(move |s| s.ai_reasoning = l);
                st.set_rev.update(|r| *r += 1);
            })));
        }
        vec![Box::new(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(model).child(row))]
    });
    let ctx_row = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let (used, window) = st.ai_ctx.get();
        let window = if window > 0 { window } else { st.ai_info.get().map(|i| i.window).unwrap_or(0) };
        let frac = if window > 0 { (used as f32 / window as f32).clamp(0.0, 1.0) } else { 0.0 };
        let text = if window > 0 { tr!("ai.context", used = short(used), window = short(window), pct = format!("{:.0}", frac * 100.0)) } else { tr!("ai.context.unknown") };
        vec![Box::new(
            Column::new()
                .gap(3.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(ProgressBar::with_value(frac).class(if frac > 0.85 { "ai-ctx-bar full" } else { "ai-ctx-bar" }))
                .child(Text::new(text).class("ai-status-text")),
        )]
    });
    DecoratedBox::new().class("ai-head").child(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(chats_row).child(mode_row).child(ctx_row))
}

