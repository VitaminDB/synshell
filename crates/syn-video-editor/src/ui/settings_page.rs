//! Страница «Настройки» (вкладка вверху окна, как в Hexspire): разделы слева,
//! содержимое карточками справа. Всё пишется сразу в
//! `~/.config/syn-video-editor/settings.toml` ([`crate::settings`]) и действует
//! со следующего хода агента.

use std::collections::BTreeMap;
use std::sync::Mutex;

use syngui::prelude::*;
use syngui::widgets::buttons::Segment;
use syngui::widgets::*;

use super::{icons, St};
use crate::settings::{self, RoleKind, Settings, ToolMode, ROLES};

/// Разделы: значок и ключ названия.
const SECTIONS: [(&str, &str); 8] = [
    (icons::AI, "settings.sec.agent"),
    (icons::DESCRIPTION, "settings.sec.prompt"),
    (icons::PSYCHOLOGY, "settings.sec.skills"),
    (icons::BUILD, "settings.sec.tools"),
    (icons::HUB, "settings.sec.models"),
    (icons::ACCOUNT_TREE, "settings.sec.nodes"),
    (icons::MOVIE, "settings.sec.crew"),
    (icons::TUNE, "settings.sec.advanced"),
];

pub const SEC_AGENT: usize = 0;
pub const SEC_PROMPT: usize = 1;
pub const SEC_SKILLS: usize = 2;
pub const SEC_TOOLS: usize = 3;
pub const SEC_MODELS: usize = 4;
pub const SEC_CREW: usize = 6;

thread_local! {
    static SECTION: std::cell::RefCell<Option<RwSignal<usize>>> = const { std::cell::RefCell::new(None) };
    static SKILL_Q: std::cell::RefCell<Option<(RwSignal<String>, RwSignal<usize>)>> = const { std::cell::RefCell::new(None) };
    static EDIT_SKILL: std::cell::RefCell<Option<RwSignal<Option<Edit>>>> = const { std::cell::RefCell::new(None) };
    static PREVIEW: std::cell::RefCell<Option<RwSignal<u64>>> = const { std::cell::RefCell::new(None) };
}

fn section() -> RwSignal<usize> {
    SECTION.with(|s| *s.borrow_mut().get_or_insert_with(|| use_signal(0)))
}

fn skill_filter() -> (RwSignal<String>, RwSignal<usize>) {
    SKILL_Q.with(|s| *s.borrow_mut().get_or_insert_with(|| (use_signal(String::new()), use_signal(0))))
}

/// Скил или сценарий, открытый в правке.
fn edit_skill() -> RwSignal<Option<Edit>> {
    EDIT_SKILL.with(|s| *s.borrow_mut().get_or_insert_with(|| use_signal(None)))
}

fn preview_rev() -> RwSignal<u64> {
    PREVIEW.with(|s| *s.borrow_mut().get_or_insert_with(|| use_signal(0)))
}

/// Открыть страницу настроек на разделе.
pub fn open_section(st: St, i: usize) {
    section().set(i);
    st.page.set(true);
    if i == SEC_MODELS || i == SEC_AGENT || i == 5 {
        load_models(st);
    }
}

/// Записать настройки и перерисовать страницу.
fn change(st: St, f: impl FnOnce(&mut Settings)) {
    settings::update(f);
    st.set_rev.update(|r| *r += 1);
}

// ── списки моделей службы ─────────────────────────────────────────────────

/// Модели по видам ролей (id, название) — из службы в фоне.
static MODELS: Mutex<Vec<(RoleKind, Vec<(String, String)>)>> = Mutex::new(Vec::new());

fn cached(kind: RoleKind) -> Vec<(String, String)> {
    MODELS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(k, _)| *k == kind).map(|(_, v)| v.clone()).unwrap_or_default()
}

fn service_kind(k: RoleKind) -> Option<synaptixd::ModelKind> {
    use synaptixd::ModelKind as M;
    Some(match k {
        RoleKind::Llm | RoleKind::Vision => M::Llm,
        RoleKind::Asr => M::Asr,
        RoleKind::Tts => M::Tts,
        RoleKind::Music => M::Music,
        RoleKind::Image => M::Image,
        RoleKind::Video => M::Video,
        RoleKind::Embed => M::Embed,
        RoleKind::Diarize => M::Diarize,
        RoleKind::Upscale | RoleKind::Rife => return None,
    })
}

/// Модели вида из службы (для зрения — LLM, которые принимают картинки);
/// нейрофильтры — из пакетов vfx библиотеки.
pub fn models_of(c: &mut synaptixd::Client, kind: RoleKind) -> Vec<(String, String)> {
    match kind {
        RoleKind::Upscale => return crate::render::neural::available().into_iter().filter(|(_, a)| a != "rife").map(|(id, a)| (id.clone(), format!("{id} ({a})"))).collect(),
        RoleKind::Rife => return crate::render::neural::available().into_iter().filter(|(_, a)| a == "rife").map(|(id, _)| (id.clone(), id)).collect(),
        _ => {}
    }
    let Some(sk) = service_kind(kind) else { return Vec::new() };
    let list = c.list().unwrap_or_default();
    let mut out = Vec::new();
    for m in list.into_iter().filter(|m| m.kind == sk) {
        if kind == RoleKind::Vision && !c.llm_info(&m.id).map(|i| i.media).unwrap_or(false) {
            continue;
        }
        let title = if m.title.trim().is_empty() { m.id.clone() } else { format!("{} · {}", m.title, m.id) };
        out.push((m.id, title));
    }
    out
}

const ALL_KINDS: [RoleKind; 11] = [RoleKind::Llm, RoleKind::Vision, RoleKind::Asr, RoleKind::Tts, RoleKind::Music, RoleKind::Image, RoleKind::Video, RoleKind::Embed, RoleKind::Diarize, RoleKind::Upscale, RoleKind::Rife];

/// Перечитать модели службы (в фоне), затем перерисовать страницу.
pub fn load_models(st: St) {
    std::thread::spawn(move || {
        let mut all = Vec::new();
        match synaptixd::Client::connect() {
            Ok(mut c) => {
                for k in ALL_KINDS {
                    all.push((k, models_of(&mut c, k)));
                }
            }
            Err(_) => {
                // служба недоступна — нейрофильтры всё равно видны (пакеты на диске)
                for k in [RoleKind::Upscale, RoleKind::Rife] {
                    all.push((k, crate::render::neural::available().into_iter().filter(|(_, a)| (a == "rife") == (k == RoleKind::Rife)).map(|(id, _)| (id.clone(), id)).collect()));
                }
            }
        }
        *MODELS.lock().unwrap_or_else(|e| e.into_inner()) = all;
        syngui::async_runtime::run_on_main_thread(move || {
            st.ai_models.set(cached(RoleKind::Llm));
            st.set_rev.update(|r| *r += 1);
        });
    });
}

/// Выбор модели: «по умолчанию» + модели вида (+ текущая, если её нет в списке).
fn model_dropdown(kind: RoleKind, cur: String, default_label: String, on: impl FnMut(&str) + Send + 'static) -> Dropdown {
    let models = cached(kind);
    let mut items = vec![DropdownItem::new(String::new(), default_label)];
    items.extend(models.iter().map(|(id, t)| DropdownItem::new(id.clone(), t.clone())));
    if !cur.is_empty() && !models.iter().any(|(id, _)| *id == cur) {
        items.push(DropdownItem::new(cur.clone(), cur.clone()));
    }
    Dropdown::with_items(items).selected(cur).max_height(420.0).on_change(on)
}

// ── общие строки ─────────────────────────────────────────────────────────

fn cards(min: f32, items: Vec<Box<dyn Widget>>) -> Grid {
    Grid::new(1).min_column_width(min).gap(16.0).masonry(true).children(items)
}

fn card(title: String, hint: String, body: impl Widget + 'static) -> impl Widget {
    DecoratedBox::new().class("set-card").child(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Text::new(title).class("set-card-title"))
            .child(Text::new(hint).class("note"))
            .child(body),
    )
}

fn label(title: String, hint: String) -> impl Widget {
    DecoratedBox::new().class("grow").child(Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(title).class("set-label")).child(Text::new(hint).class("tree-meta")))
}

fn toggle_row(title: String, hint: String, on: bool, f: impl FnMut(bool) + Send + 'static) -> impl Widget {
    Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(label(title, hint)).child(Toggle::with_state(on).on_change(f))
}

fn control_row(title: String, hint: String, control: impl Widget + 'static) -> impl Widget {
    Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(label(title, hint)).child(DecoratedBox::new().class("set-wide").child(control))
}

fn num_row(title: String, hint: String, value: f64, min: f64, max: f64, step: f64, places: u8, f: impl FnMut(f64) + Send + 'static) -> impl Widget {
    control_row(title, hint, SpinBox::new().range(min, max).step(step).decimal_places(places).value(value).on_change(f))
}

// ── ИИ: модель и агент ───────────────────────────────────────────────────

fn agent_model(st: St, s: &Settings) -> impl Widget {
    let cur = s.ai_model.clone();
    let thinking = s.ai_thinking;
    let reasoning = s.ai_reasoning.clone();
    let model = model_dropdown(RoleKind::Llm, cur, tr!("settings.model.default"), move |id: &str| {
        let id = id.to_string();
        change(st, move |s| s.ai_model = id);
        super::ai_chat::load_info(st);
    });
    let info = st.ai_info.get();
    let mut mode = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(tr!("ai.mode")).class("set-label"))
        .child(SegmentedButton::new(vec![Segment::new(tr!("ai.mode.instruct")), Segment::new(tr!("ai.mode.think"))]).selected(usize::from(thinking)).on_change(move |i| change(st, move |s| s.ai_thinking = i == 1)));
    let levels = info.as_ref().map(|i| i.reasoning_levels.clone()).unwrap_or_default();
    if thinking && !levels.is_empty() {
        let mut opts = vec![String::new()];
        opts.extend(levels);
        let sel = opts.iter().position(|l| *l == reasoning).unwrap_or(0);
        let o2 = opts.clone();
        let items: Vec<DropdownItem> = opts.iter().map(|l| DropdownItem::new(l.clone(), super::ai_chat::level_title(l))).collect();
        let _ = sel;
        mode = mode.child(Text::new(tr!("ai.depth")).class("set-label")).child(DecoratedBox::new().class("set-wide").child(Dropdown::with_items(items).selected(reasoning.clone()).on_change(move |l: &str| {
            let l = l.to_string();
            change(st, move |s| s.ai_reasoning = l)
        })));
        let _ = o2;
    }
    let facts = match &info {
        Some(i) => tr!(
            "settings.agent.facts",
            window = i.window,
            media = if i.media { tr!("common.yes") } else { tr!("common.no") },
            levels = if i.reasoning_levels.is_empty() { tr!("settings.agent.levels_none") } else { i.reasoning_levels.join(", ") }
        ),
        None => tr!("settings.agent.facts_none"),
    };
    card(
        tr!("settings.agent.model"),
        tr!("settings.agent.model.hint"),
        Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class("grow").child(model))
                    .child(ToolButton::new(icons::REFRESH).tooltip(tr!("settings.models.refresh")).on_click(move || {
                        load_models(st);
                        super::ai_chat::load_info(st);
                    })),
            )
            .child(mode)
            .child(Text::new(facts).class("tree-meta")),
    )
}

fn agent_run(st: St, s: &Settings) -> impl Widget {
    let steps = s.ai_max_steps;
    card(
        tr!("settings.agent.run"),
        tr!("settings.agent.run.hint"),
        Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(label(tr!("settings.agent.steps"), tr!("settings.agent.steps.hint"))).child({
                    const STEPS: [usize; 6] = [16, 30, 45, 60, 90, 150];
                    let sel = STEPS.iter().position(|&x| x >= steps).unwrap_or(STEPS.len() - 1);
                    SegmentedButton::new(STEPS.iter().map(|x| Segment::new(x.to_string())).collect()).selected(sel).on_change(move |i| change(st, move |s| s.ai_max_steps = STEPS[i]))
                }),
            )
            .child(toggle_row(tr!("settings.agent.compact"), tr!("settings.agent.compact.hint"), s.ai_autocompact, move |on| change(st, move |s| s.ai_autocompact = on)))
            .child(toggle_row(tr!("settings.agent.subagent"), tr!("settings.agent.subagent.hint"), s.ai_subagent, move |on| change(st, move |s| s.ai_subagent = on))),
    )
}

fn agent_chats(st: St) -> impl Widget {
    let n = st.ai_chats.get().len();
    card(
        tr!("settings.agent.chats"),
        tr!("settings.agent.chats.hint", n = n),
        Row::new()
            .gap(8.0)
            .child(Button::new(tr!("ai.chat.new")).icon(icons::ADD).on_click(move || {
                super::ai_chat::new_chat(st);
                st.page.set(false);
                st.open_panel("ai");
                st.dock.activate("ai");
            }).class("btn-small"))
            .child(Button::new(tr!("settings.agent.chats.folder")).icon(icons::FOLDER_OPEN).on_click(|| {
                let d = super::ai_chat::store_dir();
                let _ = std::fs::create_dir_all(&d);
                let _ = std::process::Command::new("xdg-open").arg(d).spawn();
            }).class("btn-small")),
    )
}

// ── системный промпт ─────────────────────────────────────────────────────

/// Съёмочная группа: у каждой роли — системный промпт (свой или по умолчанию),
/// её навыки и документы, модель роли (пусто — модель агента).
fn crew_page(st: St, s: &Settings) -> impl Widget {
    let mut col = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    col = col.child(Text::new("Фильм делает группа ролей: режиссёр ведёт конвейер сцена за сценой и поручает шаги ролям. У каждой роли свой системный промпт (роль и обязательные правила) и свои навыки (знания). Правила проекта из задания пользователя (язык, мат, стиль, длина) добавляются к промпту каждой роли сами.").class("set-hint"));
    for r in crate::crew::ROLES {
        let id = r.id;
        let custom = s.crew_prompts.get(id).is_some_and(|p| !p.trim().is_empty());
        let prompt = MultilineTextEdit::new()
            .text(crate::crew::role_system(r, s))
            .soft_wrap(true)
            .rows(6)
            .on_change(move |t: &str| {
                let t = t.to_string();
                settings::update(move |s| {
                    let def = crate::crew::role(id).map(|r| r.prompt.trim()).unwrap_or("");
                    if t.trim().is_empty() || t.trim() == def {
                        s.crew_prompts.remove(id);
                    } else {
                        s.crew_prompts.insert(id.to_string(), t);
                    }
                });
            })
            .class("set-instr-editor");
        let model = TextField::new()
            .text(s.crew_models.get(id).cloned().unwrap_or_default())
            .placeholder("модель агента (по умолчанию)")
            .on_change(move |t: &str| {
                let t = t.trim().to_string();
                settings::update(move |s| {
                    if t.is_empty() {
                        s.crew_models.remove(id);
                    } else {
                        s.crew_models.insert(id.to_string(), t);
                    }
                });
            });
        let reset = Button::new(tr!("settings.prompt.reset")).icon(icons::RESTORE).disabled(!custom).on_click(move || change(st, move |s| {
            s.crew_prompts.remove(id);
        })).class("btn-small");
        let hint = format!("Навыки: {}. Документы: {}.{}", r.skills.join(", "), r.owns.join(", "), if custom { " Промпт изменён." } else { "" });
        col = col.child(card(
            r.title.to_string(),
            hint,
            Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(prompt).child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new("Модель роли").class("set-label")).child(DecoratedBox::new().class("grow").child(model)).child(reset)),
        ));
    }
    col
}

fn prompt_page(st: St, s: &Settings) -> impl Widget {
    let custom = !s.prompt.trim().is_empty();
    let text = crate::agent::base_prompt(s);
    let editor = {
        use syngui::widgets::input::code_editor::syntax::Language;
        use syngui::widgets::input::code_editor::CodeEditorChange;
        CodeEditor::new()
            .text(text)
            .language(Language::Markdown)
            .soft_wrap(true)
            .tab_width(4)
            .on_change(move |ch: CodeEditorChange| {
                let t = ch.full_text.to_string();
                let t = if t.trim() == crate::agent::SYSTEM.trim() { String::new() } else { t };
                settings::update(move |s| s.prompt = t);
                preview_rev().update(|r| *r += 1);
            })
            .class("set-code-prompt")
    };
    let instr = MultilineTextEdit::new()
        .text(s.instructions.clone())
        .soft_wrap(true)
        .placeholder(tr!("settings.prompt.instructions.placeholder"))
        .rows(5)
        .on_change(move |t: &str| {
            let t = t.to_string();
            settings::update(move |s| s.instructions = t);
            preview_rev().update(|r| *r += 1);
        })
        .class("set-instr-editor");
    let base = card(
        if custom { tr!("settings.prompt.custom") } else { tr!("settings.prompt.standard") },
        tr!("settings.prompt.hint"),
        Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(editor).child(
            Row::new().gap(8.0).child(Button::new(tr!("settings.prompt.reset")).icon(icons::RESTORE).disabled(!custom).on_click(move || {
                change(st, |s| s.prompt.clear());
                preview_rev().update(|r| *r += 1);
            }).class("btn-small")),
        ),
    );
    let instructions = card(tr!("settings.prompt.instructions"), tr!("settings.prompt.instructions.hint"), instr);
    let preview = card(
        tr!("settings.prompt.preview"),
        tr!("settings.prompt.preview.hint"),
        Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let _ = preview_rev().get();
            let (text, on, pooled) = crate::agent::full_prompt(&settings::load());
            let copy = text.clone();
            vec![Box::new(
                Column::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(
                        Row::new()
                            .gap(8.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(DecoratedBox::new().class("grow").child(Text::new(tr!("settings.prompt.stats", chars = text.chars().count(), tools = on, pooled = pooled)).class("tree-meta")))
                            .child(Button::new(tr!("common.copy")).icon(icons::COPY).on_click(move || syngui::clipboard::copy(&copy)).class("btn-small"))
                            .child(Button::new(tr!("common.refresh")).icon(icons::REFRESH).on_click(|| preview_rev().update(|r| *r += 1)).class("btn-small")),
                    )
                    .child(DecoratedBox::new().class("set-preview").child(Text::new(text).selectable(true).class("set-preview-text"))),
            )]
        }),
    );
    Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(base).child(instructions).child(preview)
}

// ── навыки ───────────────────────────────────────────────────────────────

fn skill_tile(st: St, k: &syn_assist::skills::Skill, on: bool) -> impl Widget {
    let name = k.name.clone();
    let file = k.dir.join("SKILL.md");
    let builtin = crate::agent::is_builtin_skill(k);
    let badge = if builtin { (tr!("settings.skills.builtin"), "set-badge read") } else { (tr!("settings.skills.own"), "set-badge write") };
    DecoratedBox::new().class(if on { "set-skill" } else { "set-skill off" }).child(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .child(Toggle::with_state(on).on_change(move |v| {
                let n = name.clone();
                change(st, move |s| {
                    s.disabled_skills.retain(|x| x != &n);
                    if !v {
                        s.disabled_skills.push(n);
                    }
                })
            }))
            .child(DecoratedBox::new().class("grow").child(
                Column::new()
                    .gap(3.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(k.name.clone()).max_lines(1).class(if on { "tree-text mono" } else { "tree-text mono dim" })).child(DecoratedBox::new().class(badge.1).child(Text::new(badge.0).class("set-badge-text"))))
                    .child(Text::new(k.description.clone()).max_lines(3).class("tree-meta")),
            ))
            .child(ToolButton::new(icons::EDIT).tooltip(tr!("settings.skills.edit")).on_click(move || open_edit(Edit::Skill(file.clone())))),
    )
}

/// Своя копия встроенного навыка (правка встроенного — в своём каталоге).
fn own_copy(path: &std::path::Path) -> std::path::PathBuf {
    let builtin = crate::agent::builtin_skills_dir();
    match path.strip_prefix(&builtin) {
        Ok(rel) => settings::skills_dir().join(rel),
        Err(_) => path.to_path_buf(),
    }
}

/// Что открыто в редакторе: скил (SKILL.md) или сценарий Rhai (по имени).
#[derive(Clone, PartialEq, Debug)]
pub enum Edit {
    Skill(std::path::PathBuf),
    Script(String),
}

thread_local! {
    static STATUS: std::cell::RefCell<Option<RwSignal<(bool, String)>>> = const { std::cell::RefCell::new(None) };
    static BUF: std::cell::RefCell<Option<RwSignal<String>>> = const { std::cell::RefCell::new(None) };
}

/// Строка состояния редактора: (ошибка?, текст).
fn status() -> RwSignal<(bool, String)> {
    STATUS.with(|s| *s.borrow_mut().get_or_insert_with(|| use_signal((false, String::new()))))
}

/// Текст в редакторе (для «Проверить»).
fn buf() -> RwSignal<String> {
    BUF.with(|s| *s.borrow_mut().get_or_insert_with(|| use_signal(String::new())))
}

fn open_edit(e: Edit) {
    if edit_skill().get_untracked().as_ref() != Some(&e) {
        status().set((false, String::new()));
    }
    edit_skill().set(Some(e));
}

/// Файл, куда пишется правка, исходный текст и встроенный ли.
fn edit_source(e: &Edit) -> (std::path::PathBuf, String, bool) {
    match e {
        Edit::Skill(path) => {
            let target = own_copy(path);
            let src = if target.exists() { &target } else { path };
            (target.clone(), std::fs::read_to_string(src).unwrap_or_default(), &target != path)
        }
        Edit::Script(name) => {
            let k = crate::cmd::skill::all().into_iter().find(|k| &k.name == name);
            let target = k.as_ref().and_then(|k| k.path.clone()).unwrap_or_else(|| crate::cmd::skill::dir().join(format!("{name}.rhai")));
            let builtin = crate::cmd::skill::is_builtin(name);
            (target, k.map(|k| k.code).unwrap_or_default(), builtin)
        }
    }
}

/// Проверка: Rhai — компиляция без запуска; SKILL.md — поля name и description.
fn check(e: &Edit, text: &str) -> (bool, String) {
    match e {
        Edit::Script(_) => match crate::script::check(text) {
            Ok(()) => (false, tr!("settings.editor.ok")),
            Err(err) => (true, err),
        },
        Edit::Skill(_) => {
            let name = syn_assist::skills::front(text, "name");
            let desc = syn_assist::skills::front(text, "description");
            if name.as_deref().is_some_and(|n| !n.trim().is_empty()) && desc.as_deref().is_some_and(|d| !d.trim().is_empty()) {
                (false, tr!("settings.editor.ok"))
            } else {
                (true, tr!("settings.editor.md_missing"))
            }
        }
    }
}

/// Список слева: скилы и сценарии, щелчок — открыть в редакторе.
fn edit_list(st: St, s: &Settings, cur: &Edit) -> impl Widget {
    let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    col = col.child(Text::new(tr!("settings.sec.skills")).class("section-title"));
    for k in syn_assist::skills::list_all(&crate::agent::skill_dirs(s), &[]) {
        let e = Edit::Skill(k.dir.join("SKILL.md"));
        let on = matches!(cur, Edit::Skill(p) if own_copy(p) == own_copy(&k.dir.join("SKILL.md")));
        let e2 = e.clone();
        col = col.child(GestureDetector::new().on_click(move || open_edit(e2.clone())).child(
            DecoratedBox::new().class(if on { "set-nav-item on" } else { "set-nav-item" }).child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icons::DESCRIPTION).class("set-nav-icon")).child(Text::new(k.name.clone()).max_lines(1).class("set-nav-text"))),
        ));
    }
    col = col.child(Text::new(tr!("settings.skills.scripts")).class("section-title"));
    for k in crate::cmd::skill::all() {
        let on = matches!(cur, Edit::Script(n) if *n == k.name);
        let e = Edit::Script(k.name.clone());
        col = col.child(GestureDetector::new().on_click(move || open_edit(e.clone())).child(
            DecoratedBox::new().class(if on { "set-nav-item on" } else { "set-nav-item" }).child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icons::CODE).class("set-nav-icon")).child(Text::new(format!("{}.rhai", k.name)).max_lines(1).class("set-nav-text"))),
        ));
    }
    col = col.child(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Button::new(tr!("settings.skills.new")).icon(icons::ADD).on_click(move || new_skill(st)).class("btn-small"))
            .child(Button::new(tr!("settings.scripts.new")).icon(icons::ADD).on_click(move || new_script(st)).class("btn-small")),
    );
    DecoratedBox::new().class("set-edit-list").child(ScrollView::new().vertical().child(col))
}

fn skill_editor(st: St, s: &Settings, e: Edit) -> impl Widget {
    use syngui::widgets::input::code_editor::syntax::Language;
    use syngui::widgets::input::code_editor::CodeEditorChange;
    let (target, text, builtin) = edit_source(&e);
    let rhai = matches!(e, Edit::Script(_));
    buf().set(text.clone());
    let save_to = target.clone();
    let editor = CodeEditor::new()
        .text(text)
        .language(if rhai { Language::Rust } else { Language::Markdown })
        .soft_wrap(!rhai)
        .tab_width(4)
        .on_change(move |ch: CodeEditorChange| {
            let t = ch.full_text.to_string();
            if let Some(d) = save_to.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            match crate::layout::write_atomic(&save_to, &t) {
                Ok(()) => status().set((false, tr!("settings.editor.saved"))),
                Err(err) => status().set((true, err)),
            }
            buf().set(t);
        })
        .class("set-code");
    let name = match &e {
        Edit::Skill(_) => target.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        Edit::Script(n) => format!("{n}.rhai"),
    };
    let hint = match (rhai, builtin) {
        (false, true) => tr!("settings.skills.editing.builtin_hint", path = target.display()),
        (false, false) => tr!("settings.skills.editing.hint", path = target.display()),
        (true, true) => tr!("settings.scripts.editing.builtin_hint", path = target.display()),
        (true, false) => tr!("settings.scripts.editing.hint", path = target.display()),
    };
    let (del, e_check) = (target.clone(), e.clone());
    let head = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("grow").child(Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(name).class("set-card-title mono")).child(Text::new(hint).class("tree-meta"))))
        .child(Button::new(tr!("settings.editor.check")).icon(icons::CHECK).on_click(move || status().set(check(&e_check, &buf().get_untracked()))).class("btn-small"))
        .child(Button::new(match (builtin, rhai) {
            (true, _) => tr!("settings.skills.reset_builtin"),
            (false, false) => tr!("settings.skills.delete"),
            (false, true) => tr!("settings.scripts.delete"),
        }).icon(if builtin { icons::RESTORE } else { icons::DELETE }).on_click(move || {
            // наличие своей копии — в момент нажатия (правка создаёт её на лету)
            if !del.exists() {
                status().set((false, tr!("settings.editor.nothing_to_reset")));
                return;
            }
            if rhai {
                let _ = std::fs::remove_file(&del);
            } else if let Some(d) = del.parent() {
                let _ = std::fs::remove_dir_all(d);
            }
            if builtin {
                status().set((false, tr!("settings.editor.restored")));
                // перечитать редактор со встроенным текстом (страница строится заново)
                st.set_rev.update(|r| *r += 1);
            } else {
                edit_skill().set(None);
            }
        }).class("btn-small"))
        .child(Button::new(tr!("common.done")).icon(icons::CHECK).on_click(move || {
            edit_skill().set(None);
            st.set_rev.update(|r| *r += 1);
        }).class("btn-primary btn-small"));
    let status_row = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let (bad, t) = status().get();
        if t.is_empty() {
            return Vec::new();
        }
        vec![Box::new(Text::new(t).class(if bad { "set-status bad" } else { "set-status" }))]
    });
    Row::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(edit_list(st, s, &e))
        .child(DecoratedBox::new().class("grow").child(DecoratedBox::new().class("set-card").child(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head).child(status_row).child(editor))))
}

/// Новый свой навык-заготовка и сразу в правку.
fn new_skill(st: St) {
    let base = settings::skills_dir();
    let mut n = 1;
    let mut dir = base.join("my-skill");
    while dir.exists() {
        n += 1;
        dir = base.join(format!("my-skill-{n}"));
    }
    let name = dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("SKILL.md"), tr!("settings.skills.template", name = name));
    open_edit(Edit::Skill(dir.join("SKILL.md")));
    st.set_rev.update(|r| *r += 1);
}

/// Новый свой сценарий Rhai-заготовка и сразу в правку.
fn new_script(st: St) {
    let base = crate::cmd::skill::dir();
    let _ = std::fs::create_dir_all(&base);
    let mut n = 1;
    let mut name = "my_script".to_string();
    while base.join(format!("{name}.rhai")).exists() || crate::cmd::skill::all().iter().any(|k| k.name == name) {
        n += 1;
        name = format!("my_script_{n}");
    }
    let _ = std::fs::write(base.join(format!("{name}.rhai")), tr!("settings.scripts.template", name = name));
    open_edit(Edit::Script(name));
    st.set_rev.update(|r| *r += 1);
}

fn skills_page(st: St, s: &Settings) -> Box<dyn Widget> {
    if let Some(e) = edit_skill().get() {
        return Box::new(skill_editor(st, s, e));
    }
    let all = syn_assist::skills::list_all(&crate::agent::skill_dirs(s), &[]);
    let disabled = s.disabled_skills.clone();
    let (q, mode) = skill_filter();
    let total = all.len();
    let n_off = all.iter().filter(|k| disabled.contains(&k.name)).count();
    let toolbar = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("grow").child(TextField::with_text(q.get_untracked()).placeholder(tr!("settings.skills.search")).prefix_icon(icons::SEARCH).on_change(move |v: &str| q.set(v.to_string())).class("filter-field")))
        .child(
            SegmentedButton::new(vec![Segment::new(tr!("settings.skills.all", n = total)), Segment::new(tr!("settings.skills.on", n = total - n_off)), Segment::new(tr!("settings.skills.off", n = n_off))])
                .selected(mode.get_untracked())
                .on_change(move |i| mode.set(i)),
        )
        .child(Button::new(tr!("settings.skills.new")).icon(icons::ADD).on_click(move || new_skill(st)).class("btn-small"));
    let list = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ql = q.get().to_lowercase();
        let m = mode.get();
        let rows: Vec<(syn_assist::skills::Skill, bool)> = all
            .iter()
            .map(|k| (k.clone(), !disabled.contains(&k.name)))
            .filter(|(_, on)| match m {
                1 => *on,
                2 => !*on,
                _ => true,
            })
            .filter(|(k, _)| ql.is_empty() || k.name.to_lowercase().contains(&ql) || k.description.to_lowercase().contains(&ql))
            .collect();
        if rows.is_empty() {
            return vec![Box::new(Text::new(tr!("settings.skills.none")).class("note"))];
        }
        let tiles: Vec<Box<dyn Widget>> = rows.iter().map(|(k, on)| Box::new(skill_tile(st, k, *on)) as Box<dyn Widget>).collect();
        vec![Box::new(Grid::new(1).min_column_width(420.0).gap(8.0).children(tiles))]
    });
    // сценарии Rhai (skill_run): вкл/выкл
    let mut scripts = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for k in crate::cmd::skill::all() {
        let on = s.skill_on(&k.name);
        let name = k.name.clone();
        scripts = scripts.child(DecoratedBox::new().class(if on { "set-dir" } else { "set-dir off" }).child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(label(format!("{} · {}", k.name, k.title), k.description.clone()))
                .child({
                    let n = k.name.clone();
                    ToolButton::new(icons::EDIT).tooltip(tr!("settings.scripts.edit")).on_click(move || open_edit(Edit::Script(n.clone())))
                })
                .child(Toggle::with_state(on).on_change(move |v| {
                    let n = name.clone();
                    change(st, move |s| {
                        s.disabled_skills.retain(|x| x != &n);
                        if !v {
                            s.disabled_skills.push(n);
                        }
                    })
                })),
        ));
    }
    let side = DecoratedBox::new().class("set-side").child(
        Column::new()
            .gap(16.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(card(
                tr!("settings.skills.about"),
                tr!("settings.skills.about.hint"),
                Row::new().gap(8.0).child(Button::new(tr!("settings.skills.folder")).icon(icons::FOLDER_OPEN).on_click(|| {
                    let d = settings::skills_dir();
                    let _ = std::fs::create_dir_all(&d);
                    let _ = std::process::Command::new("xdg-open").arg(d).spawn();
                }).class("btn-small")),
            ))
            .child(card(tr!("settings.skills.scripts"), tr!("settings.skills.scripts.hint"), scripts.child(Row::new().child(Button::new(tr!("settings.scripts.new")).icon(icons::ADD).on_click(move || new_script(st)).class("btn-small"))))),
    );
    Box::new(
        Row::new()
            .gap(16.0)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .child(DecoratedBox::new().class("grow").child(Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(toolbar).child(list)))
            .child(side),
    )
}

// ── инструменты ──────────────────────────────────────────────────────────

fn tool_row(st: St, t: &crate::agent::ToolInfo, mode: ToolMode) -> impl Widget {
    let name = t.name.clone();
    let first = t.help.split(['.', '\n']).next().unwrap_or("").trim().to_string();
    const MODES: [ToolMode; 3] = [ToolMode::On, ToolMode::Pool, ToolMode::Off];
    let sel = MODES.iter().position(|m| *m == mode).unwrap_or(0);
    DecoratedBox::new().class(if mode == ToolMode::Off { "set-dir off" } else { "set-dir" }).child(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class("grow").child(
                Column::new()
                    .gap(1.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(t.name.clone()).class("tree-text mono")).child(Text::new(t.title).max_lines(1).class("tree-meta")))
                    .child(Text::new(first).max_lines(1).class("tree-meta")),
            ))
            .child(SegmentedButton::new(vec![Segment::new(tr!("settings.tools.on")), Segment::new(tr!("settings.tools.pool")), Segment::new(tr!("settings.tools.off"))]).selected(sel).on_change(move |i| {
                let m = MODES[i];
                let n = name.clone();
                change(st, move |s| {
                    s.tool_modes.remove(&n);
                    if settings::default_mode(&n) != m {
                        s.tool_modes.insert(n, m);
                    }
                })
            })),
    )
}

fn tools_page(st: St, s: &Settings) -> impl Widget {
    let cat = crate::agent::catalog();
    let mut groups: BTreeMap<usize, (&'static str, Vec<crate::agent::ToolInfo>)> = BTreeMap::new();
    let mut order: Vec<&'static str> = Vec::new();
    for t in cat {
        let i = match order.iter().position(|c| *c == t.category) {
            Some(i) => i,
            None => {
                order.push(t.category);
                order.len() - 1
            }
        };
        groups.entry(i).or_insert_with(|| (t.category, Vec::new())).1.push(t);
    }
    let total: usize = groups.values().map(|(_, v)| v.len()).sum();
    let n_pool = groups.values().flat_map(|(_, v)| v).filter(|t| s.tool_mode(&t.name) == ToolMode::Pool).count();
    let n_off = groups.values().flat_map(|(_, v)| v).filter(|t| s.tool_mode(&t.name) == ToolMode::Off).count();
    let mut items: Vec<Box<dyn Widget>> = vec![Box::new(card(
        tr!("settings.tools.modes"),
        tr!("settings.tools.modes.hint", total = total, on = total - n_pool - n_off, pool = n_pool, off = n_off),
        Row::new().child(Button::new(tr!("settings.tools.reset")).icon(icons::REFRESH).on_click(move || change(st, |s| s.tool_modes.clear())).class("btn-small")),
    ))];
    for (_, (category, tools)) in groups {
        let mut list = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for t in &tools {
            list = list.child(tool_row(st, t, s.tool_mode(&t.name)));
        }
        items.push(Box::new(card(t!(category), tr!("settings.tools.count", n = tools.len()), list)));
    }
    cards(620.0, items)
}

// ── модели ───────────────────────────────────────────────────────────────

fn models_page(st: St, s: &Settings) -> impl Widget {
    let mut list = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    let agent_model = if s.ai_model.is_empty() { tr!("settings.model.default") } else { s.ai_model.clone() };
    list = list.child(control_row(tr!("settings.role.agent"), tr!("settings.role.agent.hint", model = agent_model), Button::new(tr!("settings.role.agent.open")).on_click(move || section().set(SEC_AGENT)).class("btn-small")));
    for r in ROLES {
        let key = r.key;
        let dd = model_dropdown(r.kind, s.model(key), tr!("settings.model.default"), move |id: &str| {
            let id = id.to_string();
            change(st, move |s| {
                if id.is_empty() {
                    s.models.remove(key);
                } else {
                    s.models.insert(key.to_string(), id);
                }
            })
        });
        list = list.child(control_row(tr!(r.title), tr!(r.hint), dd));
    }
    let loaded = !MODELS.lock().unwrap_or_else(|e| e.into_inner()).is_empty();
    cards(
        640.0,
        vec![Box::new(card(
            tr!("settings.models.roles"),
            if loaded { tr!("settings.models.roles.hint") } else { tr!("settings.models.loading") },
            Column::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(Row::new().gap(8.0).child(Button::new(tr!("settings.models.refresh")).icon(icons::REFRESH).on_click(move || load_models(st)).class("btn-small")).child(Button::new(tr!("settings.models.reset")).icon(icons::RESTORE).on_click(move || change(st, |s| s.models.clear())).class("btn-small")))
                .child(list),
        ))],
    )
}

/// Вид модели для ноды графа (что предлагать в списке).
fn node_kind(kind: &str) -> RoleKind {
    match kind {
        "llm" => RoleKind::Llm,
        "image" | "image_model" => RoleKind::Image,
        "video" | "h3_model" | "ltx_model" => RoleKind::Video,
        "music" | "acestep_model" | "yue2_model" => RoleKind::Music,
        "speak" | "omnivoice" | "voxcpm" | "vibevoice" => RoleKind::Tts,
        "transcribe" => RoleKind::Asr,
        "diarize" => RoleKind::Diarize,
        "vfx_upscale" => RoleKind::Upscale,
        _ => RoleKind::Llm,
    }
}

fn nodes_page(st: St, s: &Settings) -> impl Widget {
    let roles = s.flow_models();
    let mut list = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for kind in synaptix_flow::defaults::kinds() {
        let Some(spec) = synaptix_flow::ops::spec(kind) else { continue };
        let cur = s.node_models.get(kind).cloned().unwrap_or_default();
        // что возьмёт нода, если здесь пусто: модель роли или умолчание ноды
        let role = s.models.iter().find(|(k, _)| ROLES.iter().any(|r| r.key == k.as_str() && r.nodes.contains(&kind))).map(|(_, v)| v.clone()).filter(|v| !v.is_empty());
        let schema_default = spec.field("model").map(|f| f.default.to_string()).unwrap_or_default();
        let default_label = match (role, schema_default.is_empty()) {
            (Some(m), _) => tr!("settings.nodes.from_role", model = m),
            (None, false) => tr!("settings.nodes.node_default", model = schema_default),
            (None, true) => tr!("settings.model.default"),
        };
        let _ = &roles;
        let k = kind.to_string();
        let dd = model_dropdown(node_kind(kind), cur, default_label, move |id: &str| {
            let (k, id) = (k.clone(), id.to_string());
            change(st, move |s| {
                if id.is_empty() {
                    s.node_models.remove(&k);
                } else {
                    s.node_models.insert(k, id);
                }
            })
        });
        list = list.child(control_row(syngui::i18n::t(spec.title), format!("{kind} · {}", syngui::i18n::t(spec.description)), dd));
    }
    cards(
        640.0,
        vec![Box::new(card(
            tr!("settings.nodes.title"),
            tr!("settings.nodes.hint"),
            Column::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(Row::new().gap(8.0).child(Button::new(tr!("settings.models.refresh")).icon(icons::REFRESH).on_click(move || load_models(st)).class("btn-small")).child(Button::new(tr!("settings.models.reset")).icon(icons::RESTORE).on_click(move || change(st, |s| s.node_models.clear())).class("btn-small")))
                .child(list),
        ))],
    )
}

// ── расширенные ──────────────────────────────────────────────────────────

fn advanced_page(st: St, s: &Settings) -> impl Widget {
    let generation = card(
        tr!("settings.adv.generation"),
        tr!("settings.adv.generation.hint"),
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(num_row(tr!("settings.adv.temperature"), tr!("settings.adv.zero_model"), s.ai_temperature as f64, 0.0, 2.0, 0.05, 2, move |v| change(st, move |s| s.ai_temperature = v as f32)))
            .child(num_row(tr!("settings.adv.top_p"), tr!("settings.adv.zero_model"), s.ai_top_p as f64, 0.0, 1.0, 0.05, 2, move |v| change(st, move |s| s.ai_top_p = v as f32)))
            .child(num_row(tr!("settings.adv.max_tokens"), tr!("settings.adv.max_tokens.hint"), s.ai_max_tokens as f64, 0.0, 65536.0, 256.0, 0, move |v| change(st, move |s| s.ai_max_tokens = v as u32)))
            .child(num_row(tr!("settings.adv.compact_percent"), tr!("settings.adv.compact_percent.hint"), s.ai_compact_percent as f64, 50.0, 98.0, 1.0, 0, move |v| change(st, move |s| s.ai_compact_percent = v as u32))),
    );
    let vision = card(
        tr!("settings.adv.vision"),
        tr!("settings.adv.vision.hint"),
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(num_row(tr!("settings.adv.vision_width"), tr!("settings.adv.vision_width.hint"), s.vision_width as f64, 128.0, 1920.0, 64.0, 0, move |v| change(st, move |s| s.vision_width = v as u32)))
            .child(num_row(tr!("settings.adv.vision_tokens"), tr!("settings.adv.vision_tokens.hint"), s.vision_max_tokens as f64, 64.0, 8192.0, 64.0, 0, move |v| change(st, move |s| s.vision_max_tokens = v as u32)))
            .child(num_row(tr!("settings.adv.vision_temperature"), tr!("settings.adv.vision_temperature.hint"), s.vision_temperature as f64, 0.0, 2.0, 0.05, 2, move |v| change(st, move |s| s.vision_temperature = v as f32))),
    );
    let speech = card(
        tr!("settings.adv.speech"),
        tr!("settings.adv.speech.hint"),
        control_row(tr!("settings.adv.asr_language"), tr!("settings.adv.asr_language.hint"), {
            let langs = ["", "ru", "en", "kk", "uk", "de", "fr", "es", "it", "pt", "pl", "tr", "ja", "ko", "zh"];
            let items: Vec<DropdownItem> = langs.iter().map(|l| DropdownItem::new(*l, if l.is_empty() { tr!("settings.adv.asr_auto") } else { l.to_string() })).collect();
            Dropdown::with_items(items).selected(s.asr_language.clone()).on_change(move |l: &str| {
                let l = l.to_string();
                change(st, move |s| s.asr_language = l)
            })
        }),
    );
    let reset = card(
        tr!("settings.adv.reset"),
        tr!("settings.adv.reset.hint", path = settings::path().display()),
        Row::new()
            .gap(8.0)
            .child(Button::new(tr!("settings.adv.reset.button")).icon(icons::RESTORE).on_click(move || change(st, |s| *s = Settings::default())).class("btn-small"))
            .child(Button::new(tr!("settings.adv.open_file")).icon(icons::FOLDER_OPEN).on_click(|| {
                let _ = std::process::Command::new("xdg-open").arg(crate::layout::config_dir()).spawn();
            }).class("btn-small")),
    );
    let startup = card(
        tr!("settings.adv.startup"),
        tr!("settings.adv.startup.hint"),
        Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(toggle_row(tr!("settings.adv.open_last"), tr!("settings.adv.open_last.hint"), s.open_last_project, move |on| change(st, move |s| s.open_last_project = on))),
    );
    cards(560.0, vec![Box::new(startup), Box::new(generation), Box::new(vision), Box::new(speech), Box::new(reset)])
}

// ── страница ─────────────────────────────────────────────────────────────

pub fn view(st: St) -> impl Widget {
    let sec = section();
    let nav = DecoratedBox::new().class("set-nav").child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let cur = sec.get();
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(DecoratedBox::new().class("set-nav-title").child(Text::new(tr!("settings.title")).class("set-title")));
        for (i, (icon, title)) in SECTIONS.iter().enumerate() {
            col = col.child(GestureDetector::new().on_click(move || {
                edit_skill().set(None);
                sec.set(i);
                if i == SEC_MODELS || i == 5 || i == SEC_AGENT {
                    load_models(st);
                }
            }).child(
                DecoratedBox::new().class(if i == cur { "set-nav-item on" } else { "set-nav-item" }).child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(*icon).class("set-nav-icon")).child(Text::new(tr!(*title)).class("set-nav-text"))),
            ));
        }
        vec![Box::new(col)]
    }));
    let body = ScrollView::new().vertical().child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.set_rev.get();
        let _ = st.ai_info.get();
        let _ = edit_skill().get();
        let s = settings::load();
        let i = sec.get();
        let content: Box<dyn Widget> = match i {
            SEC_AGENT => Box::new(cards(520.0, vec![Box::new(agent_model(st, &s)), Box::new(agent_run(st, &s)), Box::new(agent_chats(st))])),
            SEC_PROMPT => Box::new(prompt_page(st, &s)),
            SEC_SKILLS => skills_page(st, &s),
            SEC_TOOLS => Box::new(tools_page(st, &s)),
            SEC_MODELS => Box::new(models_page(st, &s)),
            5 => Box::new(nodes_page(st, &s)),
            SEC_CREW => Box::new(crew_page(st, &s)),
            _ => Box::new(advanced_page(st, &s)),
        };
        vec![Box::new(DecoratedBox::new().class("set-body").child(Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(tr!(SECTIONS[i.min(SECTIONS.len() - 1)].1)).class("set-title")).child(content)))]
    }));
    DecoratedBox::new().class("settings-page").child(Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(nav).child(DecoratedBox::new().class("grow").class("set-main").child(body)))
}
