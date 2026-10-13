//! Вкладка «Рецепты»: список рецептов (встроенные и свои), шаги графа,
//! параметры с выбором файлов, запуск с ходом выполнения и результатами,
//! копия в свои, открытие в редакторе узлов Synthos.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use syngui::prelude::*;
use syngui::GestureDetector;
use synaptix_graph::Graph;

use super::{boxed, icon, W};
use crate::icons;
use crate::state::St;

/// Состояние вкладки (живёт, пока открыто окно).
#[derive(Clone, Copy)]
pub struct RecipesState {
    pub selected: RwSignal<Option<String>>,
    /// Значения параметров выбранного рецепта (имя → текст; списки — через перевод строки).
    pub values: RwSignal<Map<String, Value>>,
    /// Ход прогона: строки шагов; `None` — не идёт.
    pub progress: RwSignal<Option<Vec<String>>>,
    pub result: RwSignal<Option<std::result::Result<synaptix_graph::Report, String>>>,
    pub rev: RwSignal<u64>,
}

thread_local! {
    static STATE: std::cell::Cell<Option<RecipesState>> = const { std::cell::Cell::new(None) };
}

pub fn state() -> RecipesState {
    STATE.with(|s| match s.get() {
        Some(v) => v,
        None => {
            let v = RecipesState {
                selected: use_signal(None),
                values: use_signal(Map::new()),
                progress: use_signal(None),
                result: use_signal(None),
                rev: use_signal(0),
            };
            s.set(Some(v));
            v
        }
    })
}

pub fn dir() -> PathBuf {
    synassist::pipeline::resolve(&synshell_common::Config::load().0.assistant.recipes_dir)
}

fn is_user(name: &str) -> bool {
    synaptix_graph::recipes::user(&dir()).iter().any(|(g, ok)| ok.is_ok() && g.name == name)
}

/// Откуда узел берёт данные: «← plan.lines», «← photo».
fn sources(n: &synaptix_graph::Node, g: &Graph) -> String {
    let mut refs: Vec<String> = Vec::new();
    let mut walk = |v: &Value| {
        let s = v.to_string();
        let mut rest = s.as_str();
        while let Some(a) = rest.find("{{") {
            let Some(b) = rest[a..].find("}}") else { break };
            let r = rest[a + 2..a + b].trim().to_string();
            if !refs.contains(&r) {
                refs.push(r);
            }
            rest = &rest[a + b + 2..];
        }
    };
    for v in n.args.values() {
        walk(v);
    }
    if let Some(e) = &n.each {
        walk(&Value::String(e.clone()));
    }
    let refs: Vec<String> = refs.into_iter().filter(|r| r != "item" && (r.contains('.') || g.params.contains_key(r))).collect();
    if refs.is_empty() {
        String::new()
    } else {
        format!("← {}", refs.join(", "))
    }
}

fn op_title(op: &str) -> String {
    match op {
        "llm" => t!("Текст (LLM)"),
        "text" => t!("Текст"),
        "image" => t!("Картинка"),
        "video" => t!("Видео"),
        "music" => t!("Музыка"),
        "speak" => t!("Озвучка"),
        "transcribe" => t!("Распознавание речи"),
        "concat" => t!("Склейка"),
        "mux" => t!("Звук к видео"),
        "frame" => t!("Кадр из видео"),
        "trim" => t!("Обрезка"),
        "still" => t!("Картинка в ролик"),
        other => other.to_string(),
    }
    .to_string()
}

fn op_icon(op: &str) -> &'static str {
    match op {
        "llm" | "text" => icons::PSYCHOLOGY,
        "image" | "frame" => icons::IMAGE,
        "video" | "still" | "concat" | "trim" => "\u{E04B}",
        "music" | "mux" => "\u{E405}",
        "speak" => icons::VOLUME_UP,
        "transcribe" => icons::MIC,
        _ => icons::BOLT,
    }
}

fn list_view(st: St, rs: RecipesState) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        rs.rev.get();
        let sel = rs.selected.get();
        let user: Vec<String> = synaptix_graph::recipes::user(&dir()).into_iter().filter(|(_, ok)| ok.is_ok()).map(|(g, _)| g.name).collect();
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for g in synaptix_graph::recipes::list(&dir()) {
            let name = g.name.clone();
            let active = sel.as_deref() == Some(name.as_str());
            let mine = user.contains(&name);
            col = col.child(
                GestureDetector::new()
                    .on_click(move || {
                        rs.selected.set(Some(name.clone()));
                        rs.values.set(Map::new());
                        rs.result.set(None);
                        st.recipes_open.set(true);
                    })
                    .child(
                        Row::new()
                            .gap(8.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .class(if active { "chat-item active" } else { "chat-item" })
                            .child(icon(icons::BOLT).class(if active { "ic chat-ic on" } else { "ic chat-ic" }))
                            .child(
                                Column::new()
                                    .gap(1.0)
                                    .class("grow")
                                    .child(Text::new(g.name.clone()).max_lines(1).elide(Elide::End).class("chat-title"))
                                    .child(Text::new(if mine { t!("свой").to_string() } else { t!("встроенный").to_string() }).max_lines(1).class("chat-preview")),
                            ),
                    ),
            );
        }
        vec![boxed(col)]
    }))
}

/// Боковая панель в режиме рецептов.
pub fn sidebar(st: St, rs: RecipesState) -> W {
    boxed(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                Row::new()
                    .gap(6.0)
                    .child(Button::new(t!("К чатам")).icon(icons::ARROW_BACK).class("btn new grow").on_click(move || st.recipes_open.set(false)))
                    .child(super::icon_button(icons::FOLDER, t!("Папка своих рецептов"), "tb-btn", move || {
                        let d = dir();
                        let _ = std::fs::create_dir_all(&d);
                        let _ = std::process::Command::new("xdg-open").arg(&d).spawn();
                    }))
                    .child(super::icon_button(icons::REFRESH, t!("Обновить список"), "tb-btn", move || rs.rev.set(rs.rev.get_untracked() + 1))),
            )
            .child(ScrollView::new().vertical().class("grow chat-list").child(list_view(st, rs))),
    )
}

fn value_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join("\n"),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn param_row(rs: RecipesState, name: String, p: &synaptix_graph::Param) -> W {
    let current = rs.values.get_untracked().get(&name).cloned().or(p.default.clone());
    let kind = p.kind.clone();
    let placeholder = if p.default.is_some() { t!("по умолчанию").to_string() } else { t!("обязательно").to_string() };
    let n1 = name.clone();
    let field = TextField::new()
        .text(value_text(current.as_ref()))
        .placeholder(placeholder)
        .on_change(move |t| {
            let mut m = rs.values.get_untracked();
            let v = match kind.as_str() {
                "files" => Value::Array(t.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| Value::String(l.into())).collect()),
                "number" => t.trim().parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(Value::Number).unwrap_or(Value::String(t.to_string())),
                _ => Value::String(t.to_string()),
            };
            m.insert(n1.clone(), v);
            rs.values.set(m);
        })
        .class("recipe-field grow");
    let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(field);
    if p.kind == "file" || p.kind == "files" {
        let n2 = name.clone();
        let many = p.kind == "files";
        row = row.child(Button::new(t!("Выбрать…")).icon(icons::FOLDER).class("btn").on_click(move || {
            let n3 = n2.clone();
            std::thread::spawn(move || {
                let home = synassist::builtin::home();
                if let Ok(Some(path)) = synsystem::portal_files::pick(&t!("Файл для рецепта"), false, &home, &[]) {
                    run_on_main_thread(move || {
                        let mut m = rs.values.get_untracked();
                        let s = path.display().to_string();
                        let v = if many {
                            let mut a = m.get(&n3).and_then(Value::as_array).cloned().unwrap_or_default();
                            a.push(Value::String(s));
                            Value::Array(a)
                        } else {
                            Value::String(s)
                        };
                        m.insert(n3, v);
                        rs.values.set(m);
                        rs.rev.set(rs.rev.get_untracked() + 1);
                    });
                }
            });
        }));
    }
    let kind_label = match p.kind.as_str() {
        "file" => t!("файл"),
        "files" => t!("файлы, по строке"),
        "number" => t!("число"),
        _ => t!("текст"),
    };
    boxed(
        Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Text::new(format!("{name} · {kind_label}")).class("recipe-param"))
            .child(Text::new(p.description.clone()).max_lines(3).class("recipe-hint"))
            .child(row),
    )
}

fn run_recipe(rs: RecipesState, g: Graph) {
    let params = rs.values.get_untracked();
    rs.progress.set(Some(Vec::new()));
    rs.result.set(None);
    std::thread::spawn(move || {
        let out = synassist::builtin::media::media_dir().join(format!("recipe-{}-{:x}", g.name, synassist::chat::now()));
        let r = synaptix_graph::run(&g, &params, &out, &mut |s| {
            let s = s.to_string();
            run_on_main_thread(move || {
                if let Some(mut v) = rs.progress.get_untracked() {
                    v.push(s);
                    rs.progress.set(Some(v));
                }
            });
        });
        run_on_main_thread(move || {
            rs.progress.set(None);
            rs.result.set(Some(r));
        });
    });
}

fn detail(st: St, rs: RecipesState) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        rs.rev.get();
        let Some(name) = rs.selected.get() else {
            return vec![boxed(
                Column::new()
                    .gap(8.0)
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class("grow recipe-empty")
                    .child(icon(icons::BOLT).class("ic recipe-empty-ic"))
                    .child(Text::new(t!("Рецепты — готовые сценарии из нескольких шагов")).class("recipe-title"))
                    .child(Text::new(t!("Выберите рецепт слева: видны шаги, параметры, можно запустить. Ассистент запускает их сам по просьбе в чате и может составить свой граф под задачу.")).max_lines(3).class("recipe-hint")),
            )];
        };
        let Some(g) = synaptix_graph::recipes::find(&dir(), &name) else {
            return vec![boxed(Text::new(t!("Рецепт не найден")).class("recipe-hint"))];
        };
        let mine = is_user(&g.name);
        let mut col = Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("recipe-detail");
        // Заголовок и действия.
        let g_open = g.clone();
        let g_copy = g.clone();
        let mut actions = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
        actions = actions.child(Button::new(t!("Открыть в Synthos")).icon("\u{E89E}").class("btn").on_click(move || {
            match synaptix_graph::synthos::export_and_open(&g_open, &rs.values.get_untracked()) {
                Ok(note) => st.toast(note),
                Err(e) => st.toast(format!("{}: {e}", t!("Synthos"))),
            }
        }));
        if mine {
            let n = g.name.clone();
            actions = actions.child(Button::new(t!("Удалить")).icon(icons::DELETE).class("btn").on_click(move || {
                let _ = std::fs::remove_file(dir().join(format!("{n}.json")));
                rs.selected.set(None);
                rs.rev.set(rs.rev.get_untracked() + 1);
            }));
        } else {
            actions = actions.child(Button::new(t!("Копировать в свои")).icon(icons::COPY).class("btn").on_click(move || {
                let d = dir();
                let _ = std::fs::create_dir_all(&d);
                let path = d.join(format!("{}.json", g_copy.name));
                if let Ok(json) = serde_json::to_string_pretty(&g_copy) {
                    let _ = std::fs::write(&path, json);
                    let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
                }
                rs.rev.set(rs.rev.get_untracked() + 1);
            }));
        }
        col = col.child(
            Column::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(g.name.clone()).class("recipe-title grow")).child(actions))
                .child(Text::new(g.description.clone()).max_lines(4).class("recipe-desc")),
        );
        // Шаги.
        let mut steps = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        let order = g.order().unwrap_or_default();
        for (k, &i) in order.iter().enumerate() {
            let n = &g.nodes[i];
            let each = n.each.as_ref().map(|e| format!(" · {} {e}", t!("для каждого из"))).unwrap_or_default();
            steps = steps.child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class("recipe-step")
                    .child(Text::new(format!("{}", k + 1)).class("recipe-step-n"))
                    .child(icon(op_icon(&n.op)).class("ic recipe-step-ic"))
                    .child(
                        Column::new()
                            .gap(1.0)
                            .class("grow")
                            .child(Text::new(format!("{} · {}{each}", n.id, op_title(&n.op))).max_lines(1).elide(Elide::End).class("recipe-step-title"))
                            .child(Text::new(sources(n, &g)).max_lines(1).elide(Elide::End).class("recipe-hint")),
                    ),
            );
        }
        col = col.child(Text::new(t!("Шаги")).class("recipe-section")).child(steps);
        // Параметры.
        if !g.params.is_empty() {
            let mut ps = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
            for (k, p) in &g.params {
                ps = ps.child(param_row(rs, k.clone(), p));
            }
            col = col.child(Text::new(t!("Параметры")).class("recipe-section")).child(ps);
        }
        // Запуск, ход и результат.
        let busy = rs.progress.get().is_some();
        let g_run = g.clone();
        col = col.child(
            Row::new().gap(8.0).child(
                Button::new(if busy { t!("Выполняется…") } else { t!("Запустить") })
                    .icon(if busy { icons::STOP } else { icons::BOLT })
                    .class("btn primary")
                    .on_click(move || {
                        if rs.progress.get_untracked().is_none() {
                            run_recipe(rs, g_run.clone());
                        }
                    }),
            ),
        );
        if let Some(lines) = rs.progress.get() {
            let mut c = Column::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("recipe-log");
            for l in lines.iter().rev().take(6).rev() {
                c = c.child(Text::new(l.clone()).max_lines(1).class("recipe-hint"));
            }
            col = col.child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(CircularProgress::new().indeterminate().size(16.0).stroke_width(2.0)).child(c));
        }
        match rs.result.get() {
            Some(Ok(r)) => {
                let mut c = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
                for s in &r.steps {
                    c = c.child(Text::new(format!("✓ {s}")).max_lines(2).class("recipe-hint"));
                }
                if !r.text.is_empty() {
                    c = c.child(Text::new(r.text.clone()).selectable(true).class("recipe-desc"));
                }
                let mut tiles = Flex::new().direction(FlexDirection::Row).wrap().gap(8.0);
                for f in &r.files {
                    tiles = tiles.child(super::feed::media_tile(f, 220.0, None));
                }
                let d = r.dir.clone();
                c = c.child(tiles).child(Button::new(t!("Открыть папку прогона")).icon(icons::FOLDER).class("btn").on_click(move || {
                    let _ = std::process::Command::new("xdg-open").arg(&d).spawn();
                }));
                col = col.child(Text::new(t!("Результат")).class("recipe-section")).child(c);
            }
            Some(Err(e)) => {
                col = col.child(Text::new(format!("{}: {e}", t!("Ошибка"))).max_lines(6).class("recipe-error"));
            }
            None => {}
        }
        vec![boxed(ScrollView::new().vertical().class("grow").child(col))]
    }))
}

pub fn pane(st: St, rs: RecipesState) -> W {
    boxed(DecoratedBox::new().class("pane-wrap grow").child(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("pane grow").child(detail(st, rs))))
}

#[allow(dead_code)]
fn _path_used(_: &Path) {}
