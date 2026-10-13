use std::path::Path;

use syngui::prelude::*;
use syngui::widgets::visual::canvas::Canvas;
use syngui::widgets::visual::MarkdownView;
use syngui::GestureDetector;
use syngui::StyleValue;
use synassist::chat::{ChatMsg, Role};

use super::{boxed, icon, icon_button, W};
use crate::icons;
use crate::state::{Live, St};

fn is_image(p: &Path) -> bool {
    synassist::attach::kind(p) == synassist::attach::Kind::Image
}

/// Постер видео или `None`, пока готовится (по готовности лента перерисуется).
fn video_poster(path: &Path) -> Option<std::path::PathBuf> {
    thread_local! {
        static REV: std::cell::Cell<Option<RwSignal<u64>>> = const { std::cell::Cell::new(None) };
    }
    let rev = REV.with(|r| match r.get() {
        Some(v) => v,
        None => {
            let v = use_signal(0u64);
            r.set(Some(v));
            synshell_common::thumbs::set_notify(move || run_on_main_thread(move || v.set(v.get_untracked() + 1)));
            v
        }
    });
    rev.get();
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
    synshell_common::thumbs::get(path, "video/mp4", mtime, 256)
}

/// Вложение плиткой: картинка — превью, видео — постер с ▶, остальное —
/// кнопкой с именем; щелчок открывает файл, `on_remove` — крестик (черновик).
pub fn media_tile(f: &Path, size: f32, on_remove: Option<Box<dyn Fn() + Send + Sync>>) -> W {
    let p = f.to_path_buf();
    let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let kind = synassist::attach::kind(f);
    if !matches!(kind, synassist::attach::Kind::Image | synassist::attach::Kind::Video) || !f.exists() {
        let glyph = match kind {
            synassist::attach::Kind::Audio => "\u{E405}",
            synassist::attach::Kind::Pdf => "\u{E415}",
            _ => icons::FILE,
        };
        let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center);
        row = row.child(Button::new(name).icon(glyph).class("btn att").on_click(move || open_path(&p)));
        if let Some(rm) = on_remove {
            row = row.child(Button::new("").icon("\u{E5CD}").class("btn att-x-inline").on_click(move || rm()));
        }
        return boxed(row);
    }
    let base: W = if kind == synassist::attach::Kind::Image {
        boxed(Image::new(f.to_string_lossy()).fit(ImageFit::Cover).class("att-tile"))
    } else {
        match video_poster(f) {
            Some(poster) => boxed(Image::new(poster.to_string_lossy()).fit(ImageFit::Cover).class("att-tile")),
            None => boxed(DecoratedBox::new().class("att-tile att-tile-empty")),
        }
    };
    let mut stack = syngui::widgets::Stack::new().child(base);
    if kind == synassist::attach::Kind::Video {
        stack = stack.child(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("att-tile")
                .child(DecoratedBox::new().class("att-play").child(icon("\u{E037}").class("att-play-ic"))),
        );
    }
    if let Some(rm) = on_remove {
        stack = stack.child(syngui::widgets::containers::Positioned::new(Button::new("").icon("\u{E5CD}").class("btn att-x").on_click(move || rm())).at(size - 26.0, 4.0));
    }
    let tile = DecoratedBox::new().child(stack).class("att-tile-box").clip(true).style("width", StyleValue::px(size)).style("height", StyleValue::px(size));
    boxed(syngui::widgets::Tooltip::new(GestureDetector::new().on_click(move || open_path(&p)).child(tile), name).delay_ms(500))
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
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                GestureDetector::new().on_click(move || open.set(!open.get_untracked())).child(
                    Row::new()
                        .gap(6.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(icon(icons::PSYCHOLOGY).class("ic think-ic"))
                        .child(Text::new(if live { t!("Размышляет…") } else { t!("Размышления") }).class("think-head grow"))
                        .child(Reactive::new(move || -> Vec<W> { vec![boxed(icon(if open.get() { icons::EXPAND_MORE } else { "\u{E5CC}" }).class("ic think-ic"))] })),
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

pub fn files_row(files: &[std::path::PathBuf]) -> Option<W> {
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
            col = col.child(Button::new(name).icon(icons::FILE).class("btn att").on_click(move || open_path(&p)));
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
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class(if run.ok { "tool-badge" } else { "tool-badge fail" }).child(icon(icons::tool(&run.name)).class("ic tool-ic")))
                    .child(Text::new(if run.summary.is_empty() { run.name.clone() } else { run.summary.clone() }).max_lines(1).elide(Elide::End).class("tool-sum grow"))
                    .child(icon(if run.ok { icons::CHECK } else { icons::ERROR }).class(if run.ok { "ic tool-ok" } else { "ic tool-fail" })),
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

fn summary_card(m: &ChatMsg) -> W {
    let open = use_signal(false);
    let text = m.text.clone();
    boxed(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                GestureDetector::new().on_click(move || open.set(!open.get_untracked())).child(
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(DecoratedBox::new().class("tool-badge").child(icon(icons::SUMMARY).class("ic tool-ic")))
                        .child(Text::new(t!("Начало разговора сжато в сводку ({n} сообщ.)", n = m.covers)).max_lines(1).class("tool-sum grow")),
                ),
            )
            .child(Reactive::new(move || -> Vec<W> {
                if open.get() {
                    vec![markdown(&text, "tool-out")]
                } else {
                    vec![]
                }
            }))
            .class("tool-card"),
    )
}

fn avatar() -> W {
    boxed(DecoratedBox::new().class("avatar").child(icon(icons::SPARKLE).class("ic avatar-ic")))
}

fn answer_actions(st: St, text: String, last: bool) -> W {
    let (t1, t2) = (text.clone(), text);
    let speaking = Reactive::new(move || -> Vec<W> {
        let t = t2.clone();
        if st.voice.speaking.get() {
            vec![icon_button(icons::STOP, t!("Остановить озвучку"), "act", || crate::voice::stop_speaking())]
        } else {
            vec![icon_button(icons::VOLUME_UP, t!("Прочитать вслух"), "act", move || crate::voice::speak(st, crate::voice::strip_markdown(&t)))]
        }
    });
    let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center).class("actions");
    row = row.child(icon_button(icons::COPY, t!("Копировать"), "act", move || st.copy(&t1))).child(speaking);
    if last {
        row = row.child(icon_button(icons::REFRESH, t!("Ответить заново"), "act", move || st.regenerate()));
    }
    boxed(row)
}

fn user_bubble(m: &ChatMsg) -> W {
    let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::End);
    col = col.child(DecoratedBox::new().class("user-bubble").child(Text::new(m.text.clone()).selectable(true).class("user-text")));
    if !m.files.is_empty() {
        let mut files = Flex::new().direction(FlexDirection::Row).wrap().gap(6.0);
        for f in &m.files {
            files = files.child(media_tile(f, 160.0, None));
        }
        col = col.child(files);
    }
    boxed(Row::new().main_axis_alignment(MainAxisAlignment::End).child(col))
}

fn message(st: St, m: &ChatMsg, last_answer: bool) -> Option<W> {
    if m.compacted {
        return None;
    }
    match m.role {
        Role::Summary => Some(summary_card(m)),
        Role::User => Some(user_bubble(m)),
        Role::Assistant => {
            if m.text.is_empty() && m.thinking.is_empty() {
                return None;
            }
            let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow");
            if !m.thinking.is_empty() {
                col = col.child(thinking_block(&m.thinking, false));
            }
            if !m.text.is_empty() {
                col = col.child(markdown(&m.text, "answer"));
                col = col.child(answer_actions(st, m.text.clone(), last_answer));
            }
            Some(boxed(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Start).class("assistant").child(avatar()).child(col)))
        }
        Role::Tool => Some(boxed(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Start).class("assistant").child(DecoratedBox::new().class("avatar-gap")).child(tool_card(m)))),
    }
}

fn typing() -> W {
    boxed(
        Canvas::new(|ctx, t| {
            let c = ctx.mss_accent().or_else(|| ctx.mss_color()).unwrap_or_else(|| Color::from_hex("#3B82F6"));
            let cy = ctx.height() * 0.5;
            for i in 0..3 {
                let phase = (t * 2.2 - i as f32 * 0.35).sin() * 0.5 + 0.5;
                let r = 3.0 + phase * 2.0;
                ctx.set_color(c.with_alpha(0.35 + phase * 0.65));
                ctx.fill_circle(10.0 + i as f32 * 14.0, cy, r);
            }
        })
        .size(48.0, 20.0)
        .animated(true)
        .class("typing"),
    )
}

fn live_view(l: &Live) -> W {
    let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow");
    if !l.thinking.is_empty() {
        col = col.child(thinking_block(&l.thinking, l.text.is_empty()));
    }
    if !l.text.is_empty() {
        col = col.child(markdown(&l.text, "answer"));
    } else if let Some((name, summary)) = &l.tool {
        col = col.child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("tool-badge").child(icon(icons::tool(name)).class("ic tool-ic")))
                .child(Text::new(summary.clone()).max_lines(1).elide(Elide::End).class("tool-sum grow"))
                .child(CircularProgress::new().indeterminate().size(16.0).stroke_width(2.0).class("spin"))
                .class("tool-card"),
        );
    } else if !l.tool_text.is_empty() {
        col = col.child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(typing()).child(Text::new(t!("Готовит действие…")).class("hint")));
    } else if l.thinking.is_empty() {
        col = col.child(typing());
    }
    boxed(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Start).class("assistant").child(avatar()).child(col))
}

fn orb() -> W {
    boxed(
        Canvas::new(|ctx, t| {
            let a = ctx.mss_accent().unwrap_or_else(|| Color::from_hex("#3B82F6"));
            let b = ctx.mss_color().unwrap_or_else(|| Color::from_hex("#8b5cf6"));
            let (cx, cy) = (ctx.width() * 0.5, ctx.height() * 0.5);
            let base = ctx.width().min(ctx.height()) * 0.5;
            for i in 0..4 {
                let p = ((t / 3.2) + i as f32 * 0.25).fract();
                let r = base * (0.45 + 0.55 * p);
                let col = if i % 2 == 0 { a } else { b };
                ctx.set_stroke_width(1.5 + (1.0 - p) * 2.5);
                ctx.set_color(col.with_alpha((1.0 - p) * 0.5));
                ctx.stroke_circle(cx, cy, r);
            }
            let breath = (t * 1.1).sin() * 0.06 + 1.0;
            ctx.set_color(b.with_alpha(0.35));
            ctx.fill_circle(cx + 3.0, cy + 3.0, base * 0.42 * breath);
            ctx.set_color(a.with_alpha(0.9));
            ctx.fill_circle(cx, cy, base * 0.4 * breath);
            ctx.set_color(Color::from_srgb(255, 255, 255, 0.35));
            ctx.fill_circle(cx - base * 0.12, cy - base * 0.14, base * 0.12 * breath);
        })
        .size(150.0, 150.0)
        .animated(true)
        .class("orb"),
    )
}

fn empty_state(st: St, phone: bool) -> W {
    let examples: [(&str, String, String); 4] = [
        (icons::SETTINGS, t!("Громкость 30%, яркость 60%").to_string(), t!("Сделай громкость 30% и яркость 60%").to_string()),
        (icons::FOLDER, t!("Что занимает место в Загрузках?").to_string(), t!("Что занимает место в папке Загрузки?").to_string()),
        (icons::LANGUAGE, t!("Курс доллара сегодня").to_string(), t!("Найди в интернете курс доллара").to_string()),
        (icons::VOLUME_UP, t!("Озвучь «доброе утро»").to_string(), t!("Озвучь: доброе утро!").to_string()),
    ];
    let mut grid = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("examples");
    for pair in examples.chunks(if phone { 1 } else { 2 }) {
        let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for (glyph, label, prompt) in pair {
            let p = prompt.clone();
            row = row.child(Button::new(label.clone()).icon(*glyph).class("btn example grow").on_click(move || st.send(p.clone())));
        }
        grid = grid.child(row);
    }
    let col = Column::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(orb())
        .child(Text::new(t!("Чем помочь?")).class("hero-title"))
        .child(Text::new(t!("Ассистент работает на нейросетях этого устройства: управляет системой, файлами, ищет в интернете и создаёт медиа. Говорите голосом или пишите.")).max_lines(3).class("hero-sub"))
        .child(grid)
        .child(
            Button::new(t!("Начать голосовой разговор"))
                .icon(icons::VOICE_CHAT)
                .class("btn voice-start")
                .on_click(move || crate::voice::start_conversation(st)),
        );
    boxed(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(col).class("empty grow"))
}

fn feed_list(st: St) -> W {
    let list = Reactive::new(move || -> Vec<W> {
        let chat = st.chat.get();
        let busy = st.busy.get();
        let last_idx = chat.messages.iter().rposition(|m| m.role == Role::Assistant && !m.text.is_empty() && !m.compacted);
        let mut col = Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("feed");
        for (i, m) in chat.messages.iter().enumerate() {
            if let Some(w) = message(st, m, !busy && Some(i) == last_idx) {
                col = col.child(w);
            }
        }
        vec![boxed(col)]
    });
    let live = Reactive::new(move || -> Vec<W> { st.live.get().map(|l| live_view(&l)).into_iter().collect() });
    let error = Reactive::new(move || -> Vec<W> {
        st.error
            .get()
            .map(|e| {
                boxed(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .class("error-card")
                        .child(icon(icons::ERROR).class("ic error-ic"))
                        .child(Text::new(e).max_lines(6).selectable(true).class("error grow"))
                        .child(Button::new(t!("Повторить")).icon(icons::REFRESH).class("btn ghost small").on_click(move || st.regenerate())),
                )
            })
            .into_iter()
            .collect()
    });
    boxed(
        ScrollView::new()
            .vertical()
            .follow_end(true)
            .class("grow")
            .child(Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("feed-pad").child(list).child(live).child(error)),
    )
}

pub fn feed(st: St, phone: bool) -> W {
    boxed(DecoratedBox::new().class("grow host").child(Reactive::new(move || -> Vec<W> { if st.empty.get() { vec![empty_state(st, phone)] } else { vec![feed_list(st)] } })))
}
