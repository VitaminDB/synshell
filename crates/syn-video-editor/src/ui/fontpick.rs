//! Выбор шрифта титра: окно поверх приложения со списком шрифтов библиотеки,
//! каждый показывает текст титра своим начертанием (как в CapCut), поиск по
//! названию, «По умолчанию» сверху. Щелчок — шрифт клипа, окно закрывается.

use serde_json::{json, Value};
use syngui::prelude::*;
use syngui::widgets::*;
use syngui::StyleValue;

use super::{ed, St};
use crate::doc::Source;

fn close(st: St) {
    st.font_pick.set(None);
    st.font_query.set(String::new());
}

fn pick(st: St, cid: u64, param: Option<String>, font: Option<String>) {
    match param {
        Some(pk) => {
            st.exec("template.set", json!({"id": cid, "param": pk, "value": font}));
        }
        None => {
            st.exec("clip.set", json!({"id": cid, "style": {"font": font}}));
        }
    }
    close(st);
}

pub fn view(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some((cid, param)) = st.font_pick.get() else { return Vec::new() };
        let (sample, cur) = {
            let e = ed();
            match e.project.clip(cid).map(|c| &c.source) {
                Some(Source::Title { text, style }) => {
                    let cur = match &param {
                        Some(pk) => {
                            let sk = format!("{}{pk}", crate::fusion::STYLE_PREFIX);
                            style
                                .get(&sk)
                                .and_then(Value::as_str)
                                .filter(|s| !s.is_empty())
                                .map(str::to_string)
                        }
                        None => style.get("font").and_then(Value::as_str).map(str::to_string),
                    };
                    let sample = match &param {
                        Some(pk) => pk
                            .split_once('.')
                            .and_then(|(op, _)| style.get(&format!("{}{op}.StyledText", crate::fusion::STYLE_PREFIX)).and_then(Value::as_str))
                            .unwrap_or(text.as_str())
                            .lines()
                            .next()
                            .unwrap_or("")
                            .chars()
                            .take(24)
                            .collect::<String>(),
                        None => text.lines().next().unwrap_or("").chars().take(24).collect::<String>(),
                    };
                    (sample, cur)
                }
                _ => return Vec::new(),
            }
        };
        let sample = if sample.trim().is_empty() { "Заголовок Title".to_string() } else { sample };
        let list = Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let q = st.font_query.get().to_lowercase();
            let cur = cur.clone();
            let param = param.clone();
            let mut col = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).gap(2.0);
            if q.is_empty() {
                col = col.child(row(st, cid, param.clone(), None, &sample, cur.is_none()));
            }
            for f in crate::fonts::families().into_iter().filter(|f| f.to_lowercase().contains(&q)) {
                crate::fonts::ensure(&f);
                let sel = cur.as_deref() == Some(f.as_str());
                col = col.child(row(st, cid, param.clone(), Some(f), &sample, sel));
            }
            vec![Box::new(col)]
        });
        let head = Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new("Шрифт титра").class("fp-title"))
            .child(DecoratedBox::new().class("grow"))
            .child(Text::new(format!("{} в библиотеке", crate::fonts::families().len())).class("fp-count"))
            .child(ToolButton::new("\u{E5CD}").tooltip("Закрыть (Esc)").on_click(move || close(st)));
        let search = TextField::new().placeholder("Поиск по названию").autofocus(true).on_change(move |s: &str| st.font_query.set(s.to_string())).on_escape(move || close(st));
        let card = DecoratedBox::new()
            .class("fp-card")
            .child(Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head).child(search).child(DecoratedBox::new().class("grow").child(ScrollView::new().vertical().child(list))))
            .style("width", 460.0f32)
            .style("height", 600.0f32);
        vec![Box::new(
            Stack::new()
                .fit(StackFit::Expand)
                .child(GestureDetector::new().on_click(move || close(st)).child(DecoratedBox::new().class("fp-scrim")))
                .child(Center::new().child(EventHook::new().on_key_down(move |k, _| {
                    if k == Key::Escape {
                        close(st);
                        return KeyReply::Handled;
                    }
                    KeyReply::Ignore
                }).child(card))),
        )]
    })
}

/// Строка шрифта: текст титра этим шрифтом и название.
fn row(st: St, cid: u64, param: Option<String>, font: Option<String>, sample: &str, selected: bool) -> Box<dyn Widget> {
    // букв текста нет в шрифте — латиница и пометка, а не запасной шрифт
    let covered = font.as_deref().is_none_or(|f| syngui::text::family_covers(f, sample));
    let mut name = font.clone().unwrap_or_else(|| "По умолчанию".into());
    let sample = if covered { sample.to_string() } else {
        name.push_str(" · нет этих букв");
        "Aa Bb Cc 123".to_string()
    };
    let mut preview = Text::new(sample).max_lines(1).elide(Elide::End).class("fp-sample");
    if let Some(f) = &font {
        preview = preview.style("font-family", StyleValue::String(f.clone()));
    }
    let body = DecoratedBox::new()
        .class("fp-row")
        .class(if selected { "fp-row-on" } else { "" })
        .child(Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Start).child(preview).child(Text::new(name).max_lines(1).class("fp-name")));
    Box::new(GestureDetector::new().cursor(syngui::input::CursorIcon::Pointer).on_click(move || pick(st, cid, param.clone(), font.clone())).child(body))
}
