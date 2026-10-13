mod compose;
mod feed;
mod player;
mod recipes;
mod sidebar;
mod voice;

use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::widgets::{ConfirmDialog, EventHook, KeyReply, MenuItem, PopupMenu, Stack, StackFit, Tooltip};

use crate::icons;
use crate::state::St;

pub type W = Box<dyn Widget>;

pub fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

pub fn icon(g: &str) -> syngui::widget::StyledWidget<Icon> {
    Icon::new(g).class("ic")
}

pub fn tip(w: impl Widget + 'static, text: impl Into<String>) -> W {
    boxed(Tooltip::new(w, text).delay_ms(500))
}

pub fn icon_button(glyph: &str, tooltip: impl Into<String>, class: &str, on_click: impl FnMut() + Send + 'static) -> W {
    tip(Button::new("").icon(glyph).class(&format!("btn round flat {class}")).on_click(on_click), tooltip)
}

fn brand() -> W {
    boxed(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class("brand").child(icon(icons::SPARKLE).class("ic brand-ic")))
            .child(Text::new(t!("Ассистент")).class("brand-title")),
    )
}

fn titlebar(st: St) -> W {
    let left = Reactive::new(move || -> Vec<W> {
        let open = st.sidebar.get();
        let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).class(if open { "tb-left open" } else { "tb-left" });
        row = row.child(icon_button(icons::SIDEBAR, t!("Боковая панель (Ctrl+B)"), "tb-btn", move || st.sidebar.set(!st.sidebar.get_untracked())));
        if !open {
            row = row.child(brand());
        }
        vec![boxed(row)]
    });
    let title = Reactive::new(move || -> Vec<W> {
        let c = st.chat.get();
        let busy = st.busy.get();
        let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).main_axis_alignment(MainAxisAlignment::Center).class("tb-title-row");
        if busy {
            row = row.child(CircularProgress::new().indeterminate().size(14.0).stroke_width(2.0).class("spin"));
        }
        row = row.child(Text::new(c.display_title()).max_lines(1).elide(Elide::End).class("tb-title"));
        vec![boxed(row)]
    });
    let tools = Reactive::new(move || -> Vec<W> {
        let voice_on = st.voice.open.get();
        let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
        row = row.child(tip(
            Button::new(t!("Голосом"))
                .icon(icons::VOICE_CHAT)
                .class(if voice_on { "btn voice-chip on" } else { "btn voice-chip" })
                .on_click(move || {
                    if st.voice.open.get_untracked() {
                        crate::voice::stop_conversation(st);
                    } else {
                        crate::voice::start_conversation(st);
                    }
                }),
            t!("Голосовой разговор (Ctrl+Shift+M)"),
        ));
        row = row.child(icon_button(icons::ADD, t!("Новый чат (Ctrl+N)"), "tb-btn", move || st.new_chat()));
        vec![boxed(row)]
    });
    let controls = Reactive::new(move || -> Vec<W> {
        let w = st.window.get();
        vec![boxed(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    boxed(
        DecoratedBox::new().class("titlebar").child(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(DecoratedBox::new().class("tb-left-wrap").child(left))
                .child(DecoratedBox::new().class("grow drag-wrap").child(WindowDragRegion::new().child(DecoratedBox::new().class("drag-space").child(title))))
                .child(DecoratedBox::new().class("tb-tools").child(tools))
                .child(DecoratedBox::new().class("window-controls").child(controls)),
        ),
    )
}

fn toolbar(st: St, phone: bool) -> W {
    let model = Reactive::new(move || -> Vec<W> {
        let models = st.models.get();
        let cur = st.chat.get().model;
        let service = st.service.get();
        let default_label = match &service.llm {
            Some(t) => t!("По умолчанию · {m}", m = t),
            None => t!("Модель по умолчанию"),
        };
        let mut items = vec![DropdownItem::new("", default_label).icon(icons::MEMORY)];
        for (id, title) in &models {
            items.push(DropdownItem::new(id.clone(), title.clone()));
        }
        let selected = if models.iter().any(|(id, _)| *id == cur) { cur } else { String::new() };
        vec![boxed(
            Dropdown::with_items(items)
                .selected(selected)
                .width(if phone { 170.0 } else { 260.0 })
                .max_height(360.0)
                .on_change(move |v| st.set_model(v))
                .class(if phone { "model-pick phone" } else { "model-pick" }),
        )]
    });
    let chips = Reactive::new(move || -> Vec<W> {
        let thinking = st.chat.get().thinking;
        let speak = st.speak_answers.get();
        let speaking = st.voice.speaking.get();
        let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
        row = row.child(tip(
            Button::new(if phone { String::new() } else { t!("Размышления").to_string() }).icon(icons::PSYCHOLOGY).class(if thinking { "btn chip on" } else { "btn chip" }).on_click(move || st.set_thinking(!thinking)),
            t!("Показывать ход мыслей модели перед ответом"),
        ));
        row = row.child(tip(
            Button::new(if phone { String::new() } else { t!("Озвучка").to_string() }).icon(icons::VOLUME_UP).class(if speak { "btn chip on" } else { "btn chip" }).on_click(move || st.speak_answers.set(!speak)),
            t!("Читать ответы вслух"),
        ));
        if speaking {
            row = row.child(tip(
                Button::new(t!("Стоп")).icon(icons::STOP).class("btn chip speaking").on_click(|| crate::voice::stop_speaking()),
                t!("Остановить озвучку"),
            ));
        }
        vec![boxed(row)]
    });
    let usage = Reactive::new(move || -> Vec<W> {
        let c = st.chat.get();
        if c.context == 0 || c.messages.is_empty() {
            return vec![];
        }
        let pct = (c.prompt_tokens as f32 / c.context as f32).clamp(0.0, 1.0);
        vec![tip(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(ProgressBar::with_value(pct).class(if pct > 0.85 { "usage warn" } else { "usage" }))
                .child(Text::new(format!("{}%", (pct * 100.0).round() as u32)).class("usage-label")),
            t!("Занято окно контекста: {a} из {b} токенов", a = c.prompt_tokens, b = c.context),
        )]
    });
    let more = Button::new("").icon(icons::MORE).class("btn round flat tb-btn").on_click_with_bounds(move |r| {
        st.menu_chat.set(Some(st.chat.get_untracked().id));
        st.menu_pos.set(Point::new(r.max_x() - 220.0, r.max_y() + 4.0));
        st.menu_open.set(true);
    });
    boxed(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class(if phone { "toolbar phone" } else { "toolbar" })
            .child(model)
            .child(chips)
            .child(DecoratedBox::new().class("grow"))
            .child(usage)
            .child(tip(more, t!("Действия с чатом"))),
    )
}

fn confirm_bar(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let Some((tool, summary)) = st.confirm.get() else { return vec![] };
        vec![boxed(
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("confirm")
                .child(DecoratedBox::new().class("confirm-badge").child(icon(icons::tool(&tool)).class("ic confirm-ic")))
                .child(
                    Column::new()
                        .gap(2.0)
                        .class("grow")
                        .child(Text::new(t!("Разрешить действие?")).class("confirm-title"))
                        .child(Text::new(summary).max_lines(4).selectable(true).class("confirm-sum")),
                )
                .child(Button::new(t!("Отклонить")).class("btn ghost").on_click(move || st.answer(false)))
                .child(Button::new(t!("Разрешить")).icon(icons::CHECK).class("btn primary").on_click(move || st.answer(true))),
        )]
    }))
}

fn chat_pane(st: St, phone: bool) -> W {
    let mut col = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("pane grow");
    if phone {
        col = col.child(phone_bar(st));
    }
    col = col.child(toolbar(st, phone)).child(feed::feed(st, phone)).child(confirm_bar(st)).child(compose::compose(st));
    boxed(DecoratedBox::new().class("pane-wrap grow").child(col))
}

fn phone_bar(st: St) -> W {
    let title = Reactive::new(move || -> Vec<W> { vec![boxed(Text::new(st.chat.get().display_title()).max_lines(1).elide(Elide::End).class("tb-title"))] });
    boxed(
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("phone-bar")
            .child(icon_button(icons::MENU, t!("Чаты"), "tb-btn", move || st.list_open.set(true)))
            .child(DecoratedBox::new().class("grow").child(title))
            .child(icon_button(icons::VOICE_CHAT, t!("Голосовой разговор"), "tb-btn", move || crate::voice::start_conversation(st)))
            .child(icon_button(icons::ADD, t!("Новый чат"), "tb-btn", move || st.new_chat())),
    )
}

fn menu_layer(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let current = st.chat.get().id;
        let target = st.menu_chat.get();
        let is_current = target.as_deref() == Some(current.as_str());
        let mut items = vec![MenuItem::new("rename", t!("Переименовать")).icon(icons::EDIT).shortcut("F2")];
        if is_current {
            items.push(MenuItem::new("export", t!("Сохранить в Markdown")).icon(icons::DOWNLOAD).shortcut("Ctrl+E"));
            items.push(MenuItem::new("copy", t!("Копировать последний ответ")).icon(icons::COPY));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::new("delete", t!("Удалить чат")).icon(icons::DELETE));
        vec![boxed(PopupMenu::new().items(items).position(st.menu_pos).is_open(st.menu_open).min_width(220.0).on_select(move |id| {
            st.menu_open.set(false);
            let Some(chat) = st.menu_chat.get_untracked() else { return };
            match id {
                "rename" => {
                    st.sidebar.set(true);
                    st.rename.set(Some(chat));
                }
                "export" => st.export(),
                "copy" => {
                    if let Some(a) = st.last_answer() {
                        st.copy(&a);
                    }
                }
                "delete" => st.ask_delete(&chat),
                _ => {}
            }
        }))]
    }))
}

fn dialogs(st: St) -> W {
    syngui::widgets::set_dialog_labels(t!("Удалить"), t!("Отмена"));
    boxed(
        ConfirmDialog::new(t!("Удалить чат?"), t!("Сообщения этого чата будут удалены безвозвратно."), st.delete_open, move |ok| {
            if ok {
                if let Some(id) = st.delete_ask.get_untracked() {
                    st.delete(&id);
                }
            }
            st.delete_ask.set(None);
        })
        .icon(icons::DELETE)
        .width(380.0),
    )
}

fn toast_layer(st: St) -> W {
    let toast = Reactive::new(move || -> Vec<W> {
        let Some((_, text)) = st.toast.get() else { return vec![] };
        vec![boxed(
            DecoratedBox::new().class("toast").child(
                Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(icons::DONE).class("ic toast-ic")).child(Text::new(text).max_lines(2).class("toast-text")),
            ),
        )]
    });
    boxed(Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center).class("toast-wrap").child(toast))
}

fn keys(st: St, inner: impl Widget + 'static) -> W {
    boxed(
        EventHook::new()
            .on_key_down(move |k, m| {
                if k == Key::Escape {
                    if st.menu_open.get_untracked() {
                        st.menu_open.set(false);
                        return KeyReply::Handled;
                    }
                    if st.voice.open.get_untracked() {
                        crate::voice::stop_conversation(st);
                        return KeyReply::Handled;
                    }
                    if st.rename.get_untracked().is_some() {
                        st.rename.set(None);
                        return KeyReply::Handled;
                    }
                    if st.busy.get_untracked() {
                        st.stop();
                        return KeyReply::Handled;
                    }
                    if st.voice.speaking.get_untracked() {
                        crate::voice::stop_speaking();
                        return KeyReply::Handled;
                    }
                    return KeyReply::Ignore;
                }
                if k == Key::F2 {
                    st.sidebar.set(true);
                    st.rename.set(Some(st.chat.get_untracked().id));
                    return KeyReply::Handled;
                }
                if !m.ctrl {
                    return KeyReply::Ignore;
                }
                match k {
                    Key::N => st.new_chat(),
                    Key::B => st.sidebar.set(!st.sidebar.get_untracked()),
                    Key::E => st.export(),
                    Key::M if m.shift => {
                        if st.voice.open.get_untracked() {
                            crate::voice::stop_conversation(st);
                        } else {
                            crate::voice::start_conversation(st);
                        }
                    }
                    Key::M => crate::voice::toggle_dictation(st),
                    _ => return KeyReply::Ignore,
                }
                KeyReply::Handled
            })
            .child(inner),
    )
}

fn desktop_root(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let mut row = Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("body grow");
        if st.recipes_open.get() {
            let rs = recipes::state();
            if st.sidebar.get() {
                row = row.child(DecoratedBox::new().class("sidebar").child(recipes::sidebar(st, rs)));
            }
            row = row.child(recipes::pane(st, rs));
            return vec![boxed(row)];
        }
        if st.sidebar.get() {
            row = row.child(sidebar::sidebar(st, false));
        }
        row = row.child(chat_pane(st, false));
        vec![boxed(row)]
    });
    let main = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("window").child(titlebar(st)).child(DecoratedBox::new().class("grow body-wrap").child(body));
    let stack = Stack::new().fit(StackFit::Expand).child(main).child(voice::overlay(st)).child(toast_layer(st)).child(dialogs(st)).child(menu_layer(st));
    let frame = DecoratedBox::new().class("window-frame").clip(true).child(keys(st, stack));
    boxed(WindowResizeRegion::new().over_content(true).inset(5.0).child(frame))
}

fn phone_root(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        if st.list_open.get() {
            vec![sidebar::sidebar(st, true)]
        } else {
            vec![chat_pane(st, true)]
        }
    });
    let main = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("window phone").child(DecoratedBox::new().class("grow body-wrap").child(body));
    keys(st, Stack::new().fit(StackFit::Expand).child(main).child(voice::overlay(st)).child(toast_layer(st)).child(dialogs(st)).child(menu_layer(st)))
}

pub fn root(st: St) -> W {
    let narrow = syngui::viewport::viewport_below(720.0);
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let phone = narrow.get();
        run_on_main_thread(move || syngui::signal::set_decorations(phone));
        vec![if phone { phone_root(st) } else { desktop_root(st) }]
    }))
}
