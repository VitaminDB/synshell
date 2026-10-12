use syngui::prelude::*;
use syngui::GestureDetector;
use synassist::chat::ChatSummary;

use super::{boxed, icon, icon_button, W};
use crate::icons;
use crate::state::St;

fn group_label(updated: i64) -> &'static str {
    let now = synassist::chat::now();
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
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class(if current { "chat-item active" } else { "chat-item" })
                    .child(icon(icons::CHAT).class(if current { "ic chat-ic on" } else { "ic chat-ic" }))
                    .child(
                        Column::new()
                            .gap(1.0)
                            .class("grow")
                            .child(Text::new(c.title.clone()).max_lines(1).elide(Elide::End).class("chat-title"))
                            .child(Text::new(if c.preview.is_empty() { t!("Без ответа").to_string() } else { c.preview.clone() }).max_lines(1).elide(Elide::End).class("chat-preview")),
                    )
                    .child(more),
            ),
    )
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

fn status_row(ok: bool, label: String, value: String) -> W {
    boxed(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class(if ok { "dot ok" } else { "dot off" }))
            .child(Text::new(label).class("status-label"))
            .child(Text::new(value).max_lines(1).elide(Elide::End).class("status-value grow")),
    )
}

fn footer(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let s = st.service.get();
        let none = t!("нет").to_string();
        let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("status");
        if !s.online {
            col = col.child(status_row(false, t!("Нейросети").to_string(), t!("служба не отвечает").to_string()));
        } else {
            col = col
                .child(status_row(s.llm.is_some(), t!("Модель").to_string(), s.llm.clone().unwrap_or_else(|| none.clone())))
                .child(status_row(s.asr.is_some(), t!("Слух").to_string(), s.asr.clone().unwrap_or_else(|| none.clone())))
                .child(status_row(s.tts.is_some(), t!("Голос").to_string(), s.tts.clone().unwrap_or(none)));
        }
        col = col.child(
            Button::new(t!("Настройки ассистента"))
                .icon(icons::TUNE)
                .class("btn ghost small")
                .on_click(|| {
                    let _ = std::process::Command::new("synsettings").arg("assistant").spawn();
                }),
        );
        vec![boxed(col)]
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
    head = head.child(super::brand()).child(DecoratedBox::new().class("grow"));
    if !phone {
        head = head.child(icon_button(icons::SIDEBAR, t!("Скрыть панель (Ctrl+B)"), "tb-btn", move || st.sidebar.set(false)));
    }
    boxed(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class(if phone { "sidebar phone" } else { "sidebar" })
            .child(head)
            .child(search)
            .child(Button::new(t!("Новый чат")).icon(icons::ADD).class("btn new").on_click(move || st.new_chat()))
            .child(ScrollView::new().vertical().class("grow chat-list").child(list(st)))
            .child(footer(st)),
    )
}
