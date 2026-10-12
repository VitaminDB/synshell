use std::path::Path;

use syngui::prelude::*;
use syngui::GestureDetector;
use syngui::widgets::visual::MarkdownView;
use synassist::chat::{ChatMsg, Role};

use crate::state::{Live, St};

pub type W = Box<dyn Widget>;

fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

fn icon(g: &str) -> syngui::widget::StyledWidget<Icon> {
    Icon::new(g).class("ic")
}

fn tool_icon(name: &str) -> &'static str {
    match name {
        "system" => "\u{E8B8}",
        "windows" => "\u{E069}",
        "files" => "\u{E2C7}",
        "bash" => "\u{EB8E}",
        "web" => "\u{E894}",
        "media" => "\u{E3F4}",
        _ => "\u{E869}",
    }
}

fn is_image(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif"))
}

fn open_path(p: &Path) {
    let _ = std::process::Command::new("xdg-open").arg(p).spawn();
}

fn markdown(text: &str, class: &'static str) -> W {
    boxed(MarkdownView::new(text.to_string()).with_copy_code(true).selectable(true).class(class))
}

fn thinking_block(text: &str, live: bool) -> W {
    let open = use_signal(live);
    let body = text.to_string();
    boxed(
        Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                GestureDetector::new().on_click(move || open.set(!open.get_untracked())).child(
                    Row::new()
                        .gap(6.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(icon("\u{EA4A}").class("ic think-ic"))
                        .child(Text::new(if live { t!("Размышляет…") } else { t!("Размышления") }).class("think-head")),
                ),
            )
            .child(Reactive::new(move || -> Vec<W> {
                if open.get() {
                    vec![markdown(&body, "think-body")]
                } else {
                    vec![]
                }
            }))
            .class("think"),
    )
}

fn files_row(files: &[std::path::PathBuf]) -> Option<W> {
    if files.is_empty() {
        return None;
    }
    let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Start);
    for f in files {
        let p = f.clone();
        if is_image(f) && f.exists() {
            col = col.child(GestureDetector::new().on_click(move || open_path(&p)).child(Image::new(f.to_string_lossy()).fit(ImageFit::Contain).class("att-image")));
        } else {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            col = col.child(Button::new(name).icon("\u{E2C8}").class("btn att").on_click(move || open_path(&p)));
        }
    }
    Some(boxed(col))
}

fn tool_card(m: &ChatMsg) -> W {
    let run = m.tool.clone().unwrap_or(synassist::chat::ToolRun { name: "tool".into(), summary: String::new(), ok: true });
    let open = use_signal(false);
    let text = m.text.clone();
    let mut col = Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(
            GestureDetector::new().on_click(move || open.set(!open.get_untracked())).child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(icon(tool_icon(&run.name)).class(if run.ok { "ic tool-ic" } else { "ic tool-ic fail" }))
                    .child(Text::new(if run.summary.is_empty() { run.name.clone() } else { run.summary.clone() }).max_lines(1).class("tool-sum grow"))
                    .child(icon(if run.ok { "\u{E5CA}" } else { "\u{E000}" }).class(if run.ok { "ic tool-ok" } else { "ic tool-fail" })),
            ),
        )
        .child(Reactive::new(move || -> Vec<W> {
            if open.get() {
                vec![boxed(Text::new(text.clone()).selectable(true).class("tool-out"))]
            } else {
                vec![]
            }
        }));
    if let Some(f) = files_row(&m.files) {
        col = col.child(f);
    }
    boxed(col.class(if run.ok { "tool-card" } else { "tool-card failed" }))
}

fn message(m: &ChatMsg) -> Option<W> {
    match m.role {
        Role::User => Some(boxed(
            Row::new().main_axis_alignment(MainAxisAlignment::End).child(DecoratedBox::new().class("user-bubble").child(Text::new(m.text.clone()).selectable(true).class("user-text"))),
        )),
        Role::Assistant => {
            if m.text.is_empty() && m.thinking.is_empty() {
                return None;
            }
            let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
            if !m.thinking.is_empty() {
                col = col.child(thinking_block(&m.thinking, false));
            }
            if !m.text.is_empty() {
                col = col.child(markdown(&m.text, "answer"));
            }
            Some(boxed(col.class("assistant")))
        }
        Role::Tool => Some(tool_card(m)),
    }
}

fn live_view(l: &Live) -> W {
    let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    if !l.thinking.is_empty() {
        col = col.child(thinking_block(&l.thinking, l.text.is_empty()));
    }
    if !l.text.is_empty() {
        col = col.child(markdown(&l.text, "answer"));
    } else if let Some((name, summary)) = &l.tool {
        col = col.child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(icon(tool_icon(name)).class("ic tool-ic"))
                .child(Text::new(summary.clone()).max_lines(1).class("tool-sum grow"))
                .child(CircularProgress::new().indeterminate().class("spin"))
                .class("tool-card"),
        );
    } else if !l.tool_text.is_empty() {
        col = col.child(Text::new(t!("Готовит действие…")).class("hint"));
    } else if l.thinking.is_empty() {
        col = col.child(Text::new("•••").class("typing"));
    }
    boxed(col.class("assistant"))
}

fn empty_state(st: St) -> W {
    let examples = [
        t!("Сделай громкость 30% и яркость 60%"),
        t!("Что занимает место в папке Загрузки?"),
        t!("Найди в интернете курс доллара"),
        t!("Озвучь: доброе утро!"),
    ];
    let mut col = Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("hero").child(icon("\u{E65F}").class("ic hero-ic")))
        .child(Text::new(t!("Чем помочь?")).class("hero-title"))
        .child(Text::new(t!("Ассистент работает на нейросетях этого устройства: управляет системой, файлами, ищет в интернете и создаёт медиа.")).max_lines(3).class("hero-sub"));
    for e in examples {
        let text = e.clone();
        col = col.child(Button::new(e).class("btn example").on_click(move || st.send(text.clone())));
    }
    boxed(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(col).class("empty grow"))
}

fn feed_list(st: St) -> W {
    let list = Reactive::new(move || -> Vec<W> {
        let chat = st.chat.get();
        let mut col = Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("feed");
        for m in &chat.messages {
            if let Some(w) = message(m) {
                col = col.child(w);
            }
        }
        vec![boxed(col)]
    });
    let live = Reactive::new(move || -> Vec<W> { st.live.get().map(|l| live_view(&l)).into_iter().collect() });
    let error = Reactive::new(move || -> Vec<W> {
        st.error.get().map(|e| boxed(Text::new(t!("Ошибка: {e}", e = e)).max_lines(6).class("error"))).into_iter().collect()
    });
    boxed(
        ScrollView::new()
            .vertical()
            .follow_end(true)
            .class("grow")
            .child(Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("feed-pad").child(list).child(live).child(error)),
    )
}

fn feed(st: St) -> W {
    boxed(DecoratedBox::new().class("grow host").child(Reactive::new(move || -> Vec<W> { if st.empty.get() { vec![empty_state(st)] } else { vec![feed_list(st)] } })))
}

fn confirm_bar(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let Some((tool, summary)) = st.confirm.get() else { return vec![boxed(Text::new(""))] };
        vec![boxed(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("confirm")
                .child(icon(tool_icon(&tool)).class("ic confirm-ic"))
                .child(
                    Column::new()
                        .gap(2.0)
                        .class("grow")
                        .child(Text::new(t!("Разрешить действие?")).class("confirm-title"))
                        .child(Text::new(summary).max_lines(4).selectable(true).class("confirm-sum")),
                )
                .child(Button::new(t!("Отклонить")).class("btn").on_click(move || st.answer(false)))
                .child(Button::new(t!("Разрешить")).class("btn primary").on_click(move || st.answer(true))),
        )]
    }))
}

fn input(st: St) -> W {
    let field = Reactive::new(move || -> Vec<W> {
        st.input_rev.get();
        let draft = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let d2 = draft.clone();
        let edit = syngui::widgets::input::MultilineTextEdit::new()
            .placeholder(t!("Спросите что-нибудь…"))
            .rows(1)
            .max_rows(8)
            .auto_height(true)
            .soft_wrap(true)
            .submit_on_enter(true)
            .on_change(move |s: &str| *d2.lock().unwrap() = s.to_string())
            .on_submit(move |s: &str| st.send(s.to_string()))
            .class("input grow");
        let send = Reactive::new(move || -> Vec<W> {
            let draft = draft.clone();
            if st.busy.get() {
                vec![boxed(Button::new("").icon("\u{E047}").class("btn round stop").on_click(move || st.stop()))]
            } else {
                vec![boxed(Button::new("").icon("\u{E163}").class("btn round send").on_click(move || st.send(draft.lock().unwrap().clone())))]
            }
        });
        vec![boxed(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::End).child(edit).child(send))]
    });
    boxed(DecoratedBox::new().class("input-bar").child(field))
}

fn header(st: St, narrow: bool) -> W {
    let title = DecoratedBox::new().class("grow").child(Reactive::new(move || -> Vec<W> { vec![boxed(Text::new(st.chat.get().display_title()).max_lines(1).class("title"))] }));
    let think = Reactive::new(move || -> Vec<W> {
        let on = st.chat.get().thinking;
        vec![boxed(
            Button::new(t!("Размышления"))
                .icon("\u{EA4A}")
                .class(if on { "btn chip on" } else { "btn chip" })
                .on_click(move || st.set_thinking(!on)),
        )]
    });
    let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).class("header");
    if narrow {
        row = row.child(Button::new("").icon("\u{E5D2}").class("btn round flat").on_click(move || st.list_open.set(true)));
    }
    row = row.child(title).child(think).child(Button::new("").icon("\u{E145}").class("btn round flat").on_click(move || st.new_chat()));
    boxed(row)
}

fn chat_list(st: St) -> W {
    let list = Reactive::new(move || -> Vec<W> {
        let chats = st.chats.get();
        let cur = st.chat.get().id;
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        if chats.is_empty() {
            col = col.child(Text::new(t!("Чатов пока нет")).class("hint pad"));
        }
        for c in chats {
            let (id, id2) = (c.id.clone(), c.id.clone());
            col = col.child(
                GestureDetector::new().on_click(move || st.open(&id)).child(
                    Row::new()
                        .gap(6.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .class(if c.id == cur { "chat-item active" } else { "chat-item" })
                        .child(
                            Column::new()
                                .gap(2.0)
                                .class("grow")
                                .child(Text::new(c.title.clone()).max_lines(1).class("chat-title"))
                                .child(Text::new(c.preview.clone()).max_lines(1).class("chat-preview")),
                        )
                        .child(Button::new("").icon("\u{E872}").class("btn round flat small").on_click(move || st.delete(&id2))),
                ),
            );
        }
        vec![boxed(col)]
    });
    boxed(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("sidebar")
            .child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class("brand").child(icon("\u{E65F}").class("ic brand-ic")))
                    .child(Text::new(t!("Ассистент")).class("brand-title grow")),
            )
            .child(Button::new(t!("Новый чат")).icon("\u{E145}").class("btn new").on_click(move || st.new_chat()))
            .child(ScrollView::new().vertical().class("grow").child(list)),
    )
}

fn chat_pane(st: St, narrow: bool) -> W {
    boxed(Column::new().gap(0.0).class("pane grow").child(header(st, narrow)).child(feed(st)).child(confirm_bar(st)).child(input(st)))
}

pub fn root(st: St) -> W {
    let narrow = syngui::viewport::viewport_below(720.0);
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        if narrow.get() {
            if st.list_open.get() {
                vec![boxed(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("root").child(chat_list(st)))]
            } else {
                vec![boxed(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("root").child(chat_pane(st, true)))]
            }
        } else {
            vec![boxed(Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("root").child(chat_list(st)).child(chat_pane(st, false)))]
        }
    }))
}
