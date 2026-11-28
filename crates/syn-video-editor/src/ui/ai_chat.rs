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
            Role::User => out.push(AiLine::User(with_files(&m.text, &m.files))),
            Role::Assistant => {
                if !m.thinking.trim().is_empty() {
                    out.push(AiLine::Thinking { text: m.thinking.clone(), done: true, secs: 0.0, started: 0 });
                }
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
/// Подпись модели в списке: название службы без повтора id («имя · имя» → «имя»).
fn model_title(id: &str, title: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for p in title.split(" · ").map(str::trim).filter(|p| !p.is_empty()) {
        if !parts.contains(&p) {
            parts.push(p);
        }
    }
    if parts.is_empty() {
        id.to_string()
    } else {
        parts.join(" · ")
    }
}

/// Шкала занятого контекста открытого чата (режиссёра или роли) — внизу панели ИИ, над вводом.
pub fn context_bar(st: St) -> impl Widget {
    DecoratedBox::new().class("ai-ctx").child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        // контекст того чата, что открыт: режиссёра или роли
        let (used, window) = match st.ai_view.get().filter(|v| v != "board" && v != "notes") {
            Some(r) => st.role_ctx.get().iter().find(|(id, _)| *id == r).map(|x| x.1).unwrap_or((0, 0)),
            None => st.ai_ctx.get(),
        };
        let window = if window > 0 { window } else { st.ai_info.get().map(|i| i.window).unwrap_or(0) };
        let frac = if window > 0 { (used as f32 / window as f32).clamp(0.0, 1.0) } else { 0.0 };
        let text = if window > 0 { tr!("ai.context", used = short(used), window = short(window), pct = format!("{:.0}", frac * 100.0)) } else { tr!("ai.context.unknown") };
        vec![Box::new(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("grow").child(ProgressBar::with_value(frac).class(if frac > 0.85 { "ai-ctx-bar full" } else { "ai-ctx-bar" })))
                .child(Text::new(text).max_lines(1).class("ai-status-text")),
        )]
    }))
}

pub fn head(st: St) -> impl Widget {
    let restart_open = use_signal(false);
    let chats_row = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let list: Vec<ChatSummary> = st.ai_chats.get();
        let cur = st.ai_chat.get();
        let busy = st.ai_busy.get();
        let mut items: Vec<DropdownItem> = list.iter().map(|c| DropdownItem::new(c.id.clone(), c.title.clone())).collect();
        if !list.iter().any(|c| c.id == cur) {
            items.insert(0, DropdownItem::new(cur.clone(), tr!("ai.chat.new_title")));
        }
        let dd = Dropdown::with_items(items).selected(cur).on_change(move |id: &str| open_chat(st, id)).class("ai-ctl");
        vec![Box::new(
            Row::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("grow").child(dd))
                .child(ToolButton::new(icons::ADD).tooltip(tr!("ai.chat.new")).disabled(busy).on_click(move || new_chat(st)))
                .child(ToolButton::new(icons::DELETE).tooltip(tr!("ai.chat.delete")).disabled(busy).on_click(move || delete_chat(st)))
                .child(ToolButton::new(icons::REFRESH).tooltip("Начать заново: удалить всё сделанное в проекте и заново отдать агенту первую задачу чата").disabled(busy || first_task(st).is_none()).on_click(move || restart_open.set(true)))
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
        items.extend(models.iter().map(|(id, title)| DropdownItem::new(id.clone(), model_title(id, title))));
        if !cur.is_empty() && !models.iter().any(|(id, _)| *id == cur) {
            items.push(DropdownItem::new(cur.clone(), cur.clone()));
        }
        let model = Dropdown::with_items(items).selected(cur).on_change(move |id: &str| {
            let id = id.to_string();
            crate::settings::update(move |s| s.ai_model = id);
            st.set_rev.update(|r| *r += 1);
            load_info(st);
        }).class("ai-ctl");
        let thinking = s.ai_thinking;
        let mut row = Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(SegmentedButton::new(vec![Segment::new(tr!("ai.mode.instruct")), Segment::new(tr!("ai.mode.think"))]).selected(usize::from(thinking)).on_change(move |i| {
                crate::settings::update(move |s| s.ai_thinking = i == 1);
                st.set_rev.update(|r| *r += 1);
            }).class("ai-ctl"));
        let levels = info.as_ref().map(|i| i.reasoning_levels.clone()).unwrap_or_default();
        if thinking && !levels.is_empty() {
            // уровней бывает много (none, minimal, low…) — списком, а не сегментами
            let mut opts = vec![String::new()];
            opts.extend(levels);
            let items: Vec<DropdownItem> = opts.iter().map(|l| DropdownItem::new(l.clone(), level_title(l))).collect();
            row = row.child(Dropdown::with_items(items).selected(s.ai_reasoning.clone()).on_change(move |l: &str| {
                let l = l.to_string();
                crate::settings::update(move |s| s.ai_reasoning = l);
                st.set_rev.update(|r| *r += 1);
            }).class("ai-ctl"));
        }
        // модель, режим и глубина — одной строкой (шапка не съедает ленту)
        vec![Box::new(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("grow").child(model)).child(row))]
    });
    DecoratedBox::new().class("ai-head").child(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(chats_row)
            .child(mode_row)
            .child(ConfirmDialog::new(
                "Начать заново",
                "Удалить всё, что сделано в проекте — сгенерированные картинки и ролики, голоса, реплики, документы, медиатеку и таймлайн — и заново отдать агенту первую задачу этого чата?",
                restart_open,
                move |ok| {
                    if ok {
                        restart(st);
                    }
                },
            )),
    )
}

/// Строка вложения в сообщении чата.
const ATTACHED: &str = "📎 ";

/// Текст сообщения с вложениями (по строке на файл) — так они видны в ленте.
pub fn with_files(text: &str, files: &[std::path::PathBuf]) -> String {
    let mut s = text.to_string();
    for f in files {
        s.push_str(&format!("\n{ATTACHED}{}", f.display()));
    }
    s
}

/// Обратно: текст и файлы из строки ленты.
pub fn split_files(line: &str) -> (String, Vec<String>) {
    let mut text = Vec::new();
    let mut files = Vec::new();
    for l in line.lines() {
        match l.strip_prefix(ATTACHED) {
            Some(f) => files.push(f.to_string()),
            None => text.push(l),
        }
    }
    (text.join("\n").trim_end().to_string(), files)
}

/// Первая задача пользователя в текущем чате (текст и вложения).
fn first_task(st: St) -> Option<(String, Vec<String>)> {
    st.ai_log.get_untracked().iter().find_map(|l| match l {
        AiLine::User(t) if !t.trim().is_empty() => Some(split_files(t)),
        _ => None,
    })
}

/// «Начать заново»: стереть сделанное в проекте (папки generated, voices,
/// speech, документы .md, состояние фильма story.json), очистить медиатеку и
/// таймлайн (настройки кадра остаются), открыть новый чат и снова отдать
/// агенту первую задачу прежнего.
pub fn restart(st: St) {
    if st.ai_busy.get_untracked() {
        return;
    }
    let Some((task, files)) = first_task(st) else { return };
    {
        let mut e = super::ed();
        let path = e.path.clone();
        if let Some(path) = &path {
            wipe_work(&crate::store::media_dir(path));
        }
        let mut p = crate::doc::Project::new(&e.project.name);
        p.settings = e.project.settings.clone();
        e.replace(p, path.clone());
        if let Some(path) = &path {
            let _ = crate::store::save(&e.project, path);
        }
        st.rev.set(e.revision);
        st.ph.set(e.playhead);
    }
    new_chat(st);
    st.ai_files.set(files);
    st.ask_ai(task);
}

/// Удалить из папки проекта то, что делает работа над фильмом.
pub fn wipe_work(dir: &std::path::Path) {
    for d in ["generated", "voices", "speech"] {
        let _ = std::fs::remove_dir_all(dir.join(d));
    }
    let _ = std::fs::remove_file(dir.join("story.json"));
    for f in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = f.path();
        if p.is_file() && p.extension().is_some_and(|x| x == "md") {
            let _ = std::fs::remove_file(p);
        }
    }
}

#[cfg(test)]
mod restart_tests {
    #[test]
    fn attachments_round_trip_in_chat_line() {
        let line = super::with_files("сними фильм по сценарию", &[std::path::PathBuf::from("/a/b c.pdf")]);
        assert!(line.contains("📎 /a/b c.pdf"));
        let (t, f) = super::split_files(&line);
        assert_eq!(t, "сними фильм по сценарию");
        assert_eq!(f, vec!["/a/b c.pdf".to_string()]);
    }

    #[test]
    fn wipe_keeps_project_file() {
        let d = tempfile::tempdir().unwrap();
        for sub in ["generated/a", "voices", "speech", "imported"] {
            std::fs::create_dir_all(d.path().join(sub)).unwrap();
        }
        for f in ["p.syn", "story.json", "сценарий.md", "imported/x.mp4", "voices/v.wav"] {
            std::fs::write(d.path().join(f), b"x").unwrap();
        }
        super::wipe_work(d.path());
        assert!(d.path().join("p.syn").exists());
        assert!(d.path().join("imported/x.mp4").exists());
        for f in ["story.json", "сценарий.md", "generated", "voices", "speech"] {
            assert!(!d.path().join(f).exists(), "{f}");
        }
    }
}

