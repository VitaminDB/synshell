//! Окно аргументов команды: любой пункт меню и палитры с обязательными
//! аргументами можно выполнить руками, без агента. Поля строятся по схеме
//! команды (строка, число, время, флаг, выбор, клипы, файлы, объект JSON),
//! значения подставляются из выделения, медиатеки и позиции воспроизведения.

use std::sync::{Arc, Mutex};

use serde_json::{Map, Value};
use syngui::prelude::*;
use syngui::widgets::*;

use super::{ed, St};
use crate::cmd::{self, Ty};

#[derive(Clone)]
pub struct Draft {
    pub id: &'static str,
    /// Значения полей текстом (по порядку аргументов команды).
    pub vals: Arc<Mutex<Vec<String>>>,
    /// Номер перерисовки (выбор файла меняет поле).
    pub redraw: u64,
}

impl PartialEq for Draft {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id && Arc::ptr_eq(&self.vals, &o.vals) && self.redraw == o.redraw
    }
}

fn media_cmd(id: &str) -> bool {
    id.starts_with("media.")
}

/// Значение по умолчанию из контекста.
fn prefill(st: St, cmd_id: &str, name: &str, ty: Ty) -> String {
    let e = ed();
    let sel: Vec<u64> = e.selection.clone();
    let msel = st.media_sel.get_untracked();
    let clips = |v: &[u64]| v.iter().map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
    let medias = |v: &[u64]| v.iter().map(|i| format!("m{i}")).collect::<Vec<_>>().join(", ");
    match (name, ty) {
        ("media", Ty::Id) => msel.first().map(|i| format!("m{i}")).unwrap_or_default(),
        ("media", Ty::Ids) => medias(&msel),
        ("id" | "clip", Ty::Id) if media_cmd(cmd_id) => msel.first().map(|i| format!("m{i}")).unwrap_or_default(),
        ("id" | "clip", Ty::Id) => sel.first().map(|i| format!("c{i}")).unwrap_or_default(),
        ("ids", Ty::Ids) if media_cmd(cmd_id) => medias(&msel),
        ("ids", Ty::Ids) => clips(&sel),
        ("at", Ty::Time) => crate::doc::tc(e.playhead),
        (_, Ty::Bool) => String::new(),
        _ => String::new(),
    }
}

pub fn open(st: St, id: &str) {
    open_with(st, id, &[]);
}

/// Окно с заранее заполненными полями (`(аргумент, значение)`).
pub fn open_with(st: St, id: &str, preset: &[(&str, String)]) {
    let Some(c) = cmd::find(id) else { return };
    let vals: Vec<String> = c.args.iter().map(|a| preset.iter().find(|p| p.0 == a.name).map(|p| p.1.clone()).unwrap_or_else(|| prefill(st, c.id, a.name, a.ty))).collect();
    st.args_draft.set(Some(Draft { id: c.id, vals: Arc::new(Mutex::new(vals)), redraw: 0 }));
}

fn close(st: St) {
    st.args_draft.set(None);
}

fn parse(ty: Ty, s: &str) -> std::result::Result<Value, String> {
    let t = s.trim();
    let list = |sep: &[char]| t.split(sep).map(str::trim).filter(|x| !x.is_empty()).map(|x| Value::from(x.to_string())).collect::<Vec<_>>();
    Ok(match ty {
        Ty::Str | Ty::Choice(_) | Ty::Id | Ty::Time => Value::from(t.to_string()),
        Ty::Num => Value::from(t.replace(',', ".").parse::<f64>().map_err(|_| format!("«{t}» — не число"))?),
        Ty::Bool => Value::Bool(t == "true"),
        Ty::Ids => Value::Array(list(&[',', ' ', ';'])),
        Ty::Paths => Value::Array(list(&['\n', ';'])),
        Ty::Object | Ty::Array => serde_json::from_str(t).map_err(|e| format!("JSON: {e}"))?,
        Ty::Any => t.replace(',', ".").parse::<f64>().map(Value::from).unwrap_or_else(|_| Value::from(t.to_string())),
    })
}

fn submit(st: St) {
    let Some(d) = st.args_draft.get_untracked() else { return };
    let Some(c) = cmd::find(d.id) else { return };
    let vals = d.vals.lock().unwrap().clone();
    let mut args = Map::new();
    for (a, v) in c.args.iter().zip(&vals) {
        let empty = v.trim().is_empty() || (a.ty == Ty::Bool && v != "true" && v != "false");
        if empty {
            if a.required {
                st.toast.set(Some(format!("Заполните «{}» ({})", a.name, a.help)));
                return;
            }
            continue;
        }
        match parse(a.ty, v) {
            Ok(x) => {
                args.insert(a.name.to_string(), x);
            }
            Err(e) => {
                st.toast.set(Some(format!("{}: {e}", a.name)));
                return;
            }
        }
    }
    if let Some(o) = st.exec(d.id, Value::Object(args)) {
        close(st);
        if !o.text.is_empty() {
            st.toast.set(Some(o.text.lines().take(3).collect::<Vec<_>>().join(" · ")));
        }
    }
}

fn set_val(st: St, i: usize, v: String) {
    if let Some(d) = st.args_draft.get_untracked() {
        if let Some(x) = d.vals.lock().unwrap().get_mut(i) {
            *x = v;
        }
    }
}

fn pick_files(st: St, i: usize, many: bool) {
    std::thread::spawn(move || {
        let d = rfd::FileDialog::new().set_title(if many { "Файлы" } else { "Файл" });
        let files = if many { d.pick_files() } else { d.pick_file().map(|f| vec![f]) };
        let Some(files) = files else { return };
        let v = files.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
        syngui::async_runtime::run_on_main_thread(move || {
            set_val(st, i, v);
            st.args_draft.update(|d| {
                if let Some(d) = d {
                    d.redraw += 1;
                }
            });
        });
    });
}

pub fn view(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(d) = st.args_draft.get() else { return Vec::new() };
        let Some(c) = cmd::find(d.id) else { return Vec::new() };
        vec![Box::new(
            Stack::new()
                .fit(StackFit::Expand)
                .child(GestureDetector::new().on_click(move || close(st)).child(DecoratedBox::new().class("fp-scrim")))
                .child(Center::new().child(
                    EventHook::new()
                        .on_key_down(move |k, m| {
                            if k == Key::Escape {
                                close(st);
                                return KeyReply::Handled;
                            }
                            if k == Key::Enter && m.ctrl {
                                submit(st);
                                return KeyReply::Handled;
                            }
                            KeyReply::Ignore
                        })
                        .child(card(st, c, &d)),
                )),
        )]
    })
}

fn card(st: St, c: &'static cmd::Command, d: &Draft) -> impl Widget {
    let vals = d.vals.lock().unwrap().clone();
    let head = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(c.title.trim_end_matches('…').to_string()).class("fp-title"))
        .child(DecoratedBox::new().class("grow"))
        .child(Text::new(c.id).class("row-sub"))
        .child(ToolButton::new("\u{E5CD}").tooltip("Закрыть (Esc)").on_click(move || close(st)));
    let mut form = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for (i, a) in c.args.iter().enumerate() {
        let v = vals.get(i).cloned().unwrap_or_default();
        let label = Column::new()
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Text::new(format!("{}{}", a.name, if a.required { " *" } else { "" })).class("lp-name"))
            .child(Text::new(a.help).max_lines(3).class("row-sub"));
        let field: Box<dyn Widget> = match a.ty {
            Ty::Bool => Box::new(Checkbox::checked(v == "true").label("да").on_change(move |b| set_val(st, i, b.to_string()))),
            Ty::Choice(items) => {
                let mut list = Vec::new();
                if !a.required {
                    list.push(DropdownItem::new("", "— по умолчанию —"));
                }
                list.extend(items.iter().map(|x| DropdownItem::new(*x, *x)));
                let cur = if v.is_empty() && a.required { items.first().map(|s| s.to_string()).unwrap_or_default() } else { v.clone() };
                if a.required && v.is_empty() {
                    set_val(st, i, cur.clone());
                }
                Box::new(Dropdown::new().items(list).selected(cur).on_change(move |s: &str| set_val(st, i, s.to_string())))
            }
            Ty::Paths => Box::new(
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class("grow").child(TextField::new().text(v.replace('\n', "; ")).placeholder("пути через «;»").on_change(move |s: &str| set_val(st, i, s.to_string()))))
                    .child(Button::new("Выбрать…").class("head-btn").on_click(move || pick_files(st, i, true))),
            ),
            Ty::Str if matches!(a.name, "file" | "path" | "music" | "lut") => Box::new(
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class("grow").child(TextField::new().text(v).placeholder("путь к файлу").on_change(move |s: &str| set_val(st, i, s.to_string()))))
                    .child(Button::new("Выбрать…").class("head-btn").on_click(move || pick_files(st, i, false))),
            ),
            _ => {
                let ph = match a.ty {
                    Ty::Time => "секунды или м:сс.ммм",
                    Ty::Num => "число",
                    Ty::Id => "c12 / m3 / t1",
                    Ty::Ids => "c1, c2",
                    Ty::Object => "{\"ключ\": значение}",
                    Ty::Array => "[…]",
                    _ => "",
                };
                Box::new(TextField::new().text(v).placeholder(ph).on_change(move |s: &str| set_val(st, i, s.to_string())))
            }
        };
        form = form.child(
            DecoratedBox::new().class("lp-item").child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().style("width", 260.0f32).child(label))
                    .child(DecoratedBox::new().class("grow").child(field)),
            ),
        );
    }
    let foot = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new("* — обязательно · Ctrl+Enter — выполнить").class("row-sub"))
        .child(DecoratedBox::new().class("grow"))
        .child(Button::new("Отмена").class("head-btn").on_click(move || close(st)))
        .child(Button::new("Выполнить").class("primary-btn").on_click(move || submit(st)));
    DecoratedBox::new().class("fp-card").style("width", 760.0f32).style("max-height", 760.0f32).child(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(head)
            .child(Text::new(c.help).max_lines(8).class("row-sub"))
            .child(DecoratedBox::new().class("grow").child(ScrollView::new().vertical().child(form)))
            .child(foot),
    )
}
