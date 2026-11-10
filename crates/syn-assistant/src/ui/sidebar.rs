use syngui::prelude::*;
use syngui::GestureDetector;
use syngui::overlay::WindowDragRegion;
use syn_assist::chat::ChatSummary;

use super::{boxed, icon_button, tip, W};
use crate::icons;
use crate::state::St;

fn group_label(updated: i64) -> &'static str {
    let now = syn_assist::chat::now();
    let age = (now - updated).max(0);
    let day = 24 * 3600 * 1000;
    if age < day {
        "today"
    } else if age < 2 * day {
        "yesterday"
    } else if age < 7 * day {
        "week"
    } else {
        "older"
    }
}

fn group_title(key: &str) -> String {
    match key {
        "today" => t!("Сегодня"),
        "yesterday" => t!("Вчера"),
        "week" => t!("На этой неделе"),
        _ => t!("Ранее"),
    }
    .to_string()
}

fn chat_item(st: St, c: &ChatSummary, current: bool, renaming: bool) -> W {
    let id = c.id.clone();
    if renaming {
        let id2 = id.clone();
        return boxed(
            DecoratedBox::new().class("chat-item renaming").child(
                TextField::with_text(c.title.clone())
                    .autofocus(true)
                    .submit_on_focus_lost(true)
                    .on_submit(move |t| st.rename_chat(&id2, t))
                    .on_escape(move || st.rename.set(None))
                    .class("rename-field"),
            ),
        );
    }
    let (id_open, id_menu, id_menu2, id_rename) = (id.clone(), id.clone(), id.clone(), id.clone());
    let more = Button::new("").icon(icons::MORE).class("btn round flat tiny").on_click_with_bounds(move |r| {
        st.menu_chat.set(Some(id_menu.clone()));
        st.menu_pos.set(Point::new(r.max_x() - 220.0, r.max_y() + 2.0));
        st.menu_open.set(true);
    });
    boxed(
        GestureDetector::new()
            .on_click(move || st.open(&id_open))
            .on_double_click(move || st.rename.set(Some(id_rename.clone())))
            .on_secondary_click(move |p| {
                st.menu_chat.set(Some(id_menu2.clone()));
                st.menu_pos.set(p);
                st.menu_open.set(true);
            })
            .child(
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class(if current { "chat-item active" } else { "chat-item" })
                    .child(Text::new(super::chat_title(c.title.clone())).max_lines(1).elide(Elide::End).class(if current { "chat-title on grow" } else { "chat-title grow" }))
                    .child(if current { boxed(more) } else { boxed(Text::new(when(c.updated)).class("chat-time")) }),
            ),
    )
}

/// Давность чата: «сейчас», минуты, часы, дни, недели.
fn when(updated: i64) -> String {
    let min = ((syn_assist::chat::now() - updated).max(0) / 60_000) as u64;
    match min {
        0 => t!("сейчас").to_string(),
        1..=59 => t!("{n} мин", n = min).to_string(),
        60..=1439 => t!("{n} ч", n = min / 60).to_string(),
        1440..=10079 => t!("{n} д", n = min / 1440).to_string(),
        _ => t!("{n} нед", n = min / 10080).to_string(),
    }
}

fn list(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let chats = st.chats.get();
        let cur = st.chat.get().id;
        let q = st.search.get().trim().to_lowercase();
        let renaming = st.rename.get();
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        let filtered: Vec<&ChatSummary> = chats.iter().filter(|c| q.is_empty() || c.title.to_lowercase().contains(&q) || c.preview.to_lowercase().contains(&q)).collect();
        if filtered.is_empty() {
            let hint = if q.is_empty() { t!("Чатов пока нет — задайте первый вопрос") } else { t!("Ничего не найдено") };
            col = col.child(Text::new(hint).max_lines(2).class("hint pad"));
        }
        let mut last_group = "";
        for c in filtered {
            let g = group_label(c.updated);
            if g != last_group {
                col = col.child(Text::new(group_title(g)).class("group-title"));
                last_group = g;
            }
            col = col.child(chat_item(st, c, c.id == cur, renaming.as_deref() == Some(c.id.as_str())));
        }
        vec![boxed(col)]
    }))
}

fn status_dot(ok: bool, glyph: &'static str, tip_text: String) -> W {
    tip(
        Row::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class(if ok { "svc ok" } else { "svc off" })
            .child(DecoratedBox::new().class(if ok { "dot ok" } else { "dot off" }))
            .child(Icon::new(glyph).class("ic svc-ic")),
        tip_text,
    )
}

/// Нижняя строка: состояние нейросетей значками (подробности — в подсказках) и настройки.
fn footer(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let s = st.service.get();
        let none = t!("нет").to_string();
        let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).class("status");
        if !s.online {
            row = row.child(status_dot(false, icons::CLOUD, t!("Нейросети: служба не отвечает").to_string())).child(Text::new(t!("служба не отвечает")).max_lines(1).class("status-label"));
        } else {
            row = row
                .child(status_dot(s.llm.is_some(), icons::MEMORY, format!("{}: {}", t!("Модель"), s.llm.clone().unwrap_or_else(|| none.clone()))))
                .child(status_dot(s.asr.is_some(), icons::MIC, format!("{}: {}", t!("Слух"), s.asr.clone().unwrap_or_else(|| none.clone()))))
                .child(status_dot(s.tts.is_some(), icons::VOLUME_UP, format!("{}: {}", t!("Голос"), s.tts.clone().unwrap_or(none))));
        }
        row = row.child(DecoratedBox::new().class("grow")).child(icon_button(icons::TUNE, t!("Настройки ассистента"), "tb-btn", || {
            let _ = std::process::Command::new("synsettings").arg("assistant").spawn();
        }));
        vec![boxed(row)]
    }))
}

pub fn sidebar(st: St, phone: bool) -> W {
    let search = TextField::new()
        .placeholder(t!("Поиск по чатам"))
        .prefix_icon(icons::SEARCH)
        .on_change(move |t| st.search.set(t.to_string()))
        .class("search");
    let mut head = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).class("sidebar-head");
    if phone {
        head = head.child(icon_button(icons::ARROW_BACK, t!("К чату"), "tb-btn", move || st.list_open.set(false)));
    }
    head = head.child(super::brand());
    // Узкая раскладка — шапка служит заголовком окна: за пустое место окно перетаскивают.
    // Шапка служит и заголовком окна: за пустое место окно перетаскивают.
    head = head.child(boxed(DecoratedBox::new().class("grow").child(WindowDragRegion::new().child(DecoratedBox::new().class("head-drag")))));
    if !phone {
        head = head.child(icon_button(icons::SIDEBAR, t!("Скрыть панель (Ctrl+B)"), "tb-btn", move || st.sidebar.set(false)));
    } else {
        head = head.child(super::narrow_window_controls(st));
    }
    boxed(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class(if phone { "sidebar phone" } else { "sidebar" })
            .child(head)
            .child(search)
            .child(
                Row::new()
                    .gap(6.0)
                    .child(DecoratedBox::new().class("grow").child(tip(Button::new(t!("Новый чат")).icon(icons::EDIT_SQUARE).class("btn new").on_click(move || st.new_chat()), t!("Новый чат (Ctrl+N)"))))
                    .child(super::icon_button(icons::BOLT, t!("Рецепты — готовые сценарии"), "tb-btn", move || st.recipes_open.set(true))),
            )
            .child(ScrollView::new().vertical().class("grow chat-list").child(list(st)))
            .child(footer(st)),
    )
}
