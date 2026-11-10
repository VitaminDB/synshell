use std::path::Path;

use syngui::prelude::*;
use syngui::widgets::visual::canvas::Canvas;
use syngui::widgets::visual::MarkdownView;
use syngui::GestureDetector;
use syngui::StyleValue;
use syn_assist::chat::{ChatMsg, Role};

use super::{boxed, icon, icon_button, W};
use crate::icons;
use crate::state::{Live, St};

fn is_image(p: &Path) -> bool {
    syn_assist::attach::kind(p) == syn_assist::attach::Kind::Image
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
    let kind = syn_assist::attach::kind(f);
    // Результаты (без крестика черновика): звук и видео играют на месте.
    if on_remove.is_none() && f.exists() && kind == syn_assist::attach::Kind::Audio {
        return super::player::audio(p);
    }
    if on_remove.is_none() && f.exists() && kind == syn_assist::attach::Kind::Video {
        return super::player::video(p, video_poster(f), size.max(360.0));
    }
    if !matches!(kind, syn_assist::attach::Kind::Image | syn_assist::attach::Kind::Video) || !f.exists() {
        let glyph = match kind {
            syn_assist::attach::Kind::Audio => "\u{E405}",
            syn_assist::attach::Kind::Pdf => "\u{E415}",
            _ => icons::FILE,
        };
        let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center);
        row = row.child(Button::new(name).icon(glyph).class("btn att").on_click(move || open_path(&p)));
        if let Some(rm) = on_remove {
            row = row.child(Button::new("").icon("\u{E5CD}").class("btn att-x-inline").on_click(move || rm()));
        }
        return boxed(row);
    }
    let base: W = if kind == syn_assist::attach::Kind::Image {
        boxed(Image::new(f.to_string_lossy()).fit(ImageFit::Cover).class("att-tile"))
    } else {
        match video_poster(f) {
            Some(poster) => boxed(Image::new(poster.to_string_lossy()).fit(ImageFit::Cover).class("att-tile")),
            None => boxed(DecoratedBox::new().class("att-tile att-tile-empty")),
        }
    };
    let mut stack = syngui::widgets::Stack::new().child(base);
    if kind == syn_assist::attach::Kind::Video {
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
        let kind = syn_assist::attach::kind(f);
        if is_image(f) && f.exists() {
            col = col.child(GestureDetector::new().on_click(move || open_path(&p)).child(Image::new(f.to_string_lossy()).fit(ImageFit::Contain).class("att-image")));
        } else if f.exists() && matches!(kind, syn_assist::attach::Kind::Video | syn_assist::attach::Kind::Audio) {
            col = col.child(media_tile(f, 480.0, None));
        } else {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            col = col.child(Button::new(name).icon(icons::FILE).class("btn att").on_click(move || open_path(&p)));
        }
    }
    Some(boxed(col))
}

/// Строк живого вывода в окне.
const TERM_LINES: usize = 24;
/// Строк готового вывода в окне (хвост).
const OUT_LINES: usize = 400;
/// Наибольшая высота окна готового вывода (дальше — прокрутка).
const TERM_MAX_H: f32 = 340.0;
/// Сколько строк вывода помещается в окно без прокрутки.
const TERM_MAX_ROWS: usize = 18;
/// Примерно символов в строке окна (моноширинный 12 px в колонке чата).
const TERM_COLS: usize = 100;

#[derive(Clone, Copy, PartialEq)]
enum ToolState {
    Running,
    Ok,
    Fail,
}

/// Строка шага: значок, что делает, имя инструмента, число строк, итог, стрелка.
fn tool_row(name: &str, summary: &str, state: ToolState, open: bool, lines: usize) -> W {
    let mut row = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class(if open { "step-row open" } else { "step-row" })
        .child(icon(icons::tool(name)).class(if state == ToolState::Fail { "ic step-ic fail" } else { "ic step-ic" }))
        .child(Text::new(if summary.is_empty() { name } else { summary }).max_lines(1).elide(Elide::End).class("step-sum grow"))
        .child(Text::new(name).max_lines(1).class("step-name"));
    if lines > 0 {
        row = row.child(Text::new(t!("{n} стр.", n = lines)).class("step-count"));
    }
    row = match state {
        ToolState::Running => row.child(CircularProgress::new().indeterminate().size(14.0).stroke_width(2.0).class("spin")),
        ToolState::Ok => row.child(icon(icons::CHECK).class("ic step-ok")),
        ToolState::Fail => row.child(icon(icons::ERROR).class("ic step-fail")),
    };
    boxed(row.child(icon(if open { icons::EXPAND_MORE } else { icons::CHEVRON_RIGHT }).class("ic step-chevron")))
}

/// Тёмное окно терминала: вывод инструмента (живой — построчно, со stderr;
/// готовый — одним текстом, его можно выделять и копировать).
fn term_window(st: St, title: &str, lines: &[(String, bool)], state: ToolState) -> W {
    let running = state == ToolState::Running;
    let mut v: Vec<&(String, bool)> = lines.iter().collect();
    // хвостовая пустая строка — недописанная
    while v.last().is_some_and(|(l, _)| l.trim().is_empty()) {
        v.pop();
    }
    let lead = v.iter().take_while(|(l, _)| l.trim().is_empty()).count();
    v.drain(..lead);
    let total = v.len();
    let skip = total.saturating_sub(if running { TERM_LINES } else { OUT_LINES });
    let mut body = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    if skip > 0 {
        body = body.child(Text::new(t!("… выше ещё {n} стр.", n = skip)).class("term-note"));
    }
    if total == 0 {
        body = body.child(Text::new(if running { t!("ожидание вывода…") } else { t!("(без вывода)") }).class("term-note"));
    }
    let all: String = v.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join("\n");
    let shown_lines: Vec<String> = v.iter().skip(skip).map(|(l, _)| l.clone()).collect();
    if running {
        for (line, err) in v.into_iter().skip(skip) {
            body = body.child(Text::new(if line.is_empty() { " " } else { line.as_str() }).max_lines(1).class(if *err { "term-line err" } else { "term-line" }));
        }
        body = body.child(Text::new("▍").class("term-cursor"));
    } else if total > 0 {
        let text: Vec<&str> = v.into_iter().skip(skip).map(|(l, _)| l.as_str()).collect();
        body = body.child(Text::new(text.join("\n")).selectable(true).class("term-line"));
    }
    let (label, class) = match state {
        ToolState::Running => (t!("выполняется"), "term-state run"),
        ToolState::Ok => (t!("готово"), "term-state"),
        ToolState::Fail => (t!("ошибка"), "term-state fail"),
    };
    let mut bar = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("term-dot r"))
        .child(DecoratedBox::new().class("term-dot y"))
        .child(DecoratedBox::new().class("term-dot g"))
        .child(Text::new(title).max_lines(1).elide(Elide::End).class("term-cmd grow"))
        .child(Text::new(label).class(class));
    if total > 0 {
        bar = bar.child(icon_button(icons::COPY, t!("Копировать вывод"), "term-copy", move || st.copy(&all)));
    }
    let body: W = if running {
        boxed(DecoratedBox::new().class("term-body").child(body))
    } else {
        // Короткий вывод — целиком; длинный (с учётом переноса строк) — в окне
        // высотой TERM_MAX_H с прокруткой: ScrollView занимает всю высоту,
        // какую дадут, поэтому её задаёт обёртка.
        let rows: usize = shown_lines.iter().map(|l| l.chars().count().div_ceil(TERM_COLS).max(1)).sum();
        if rows <= TERM_MAX_ROWS {
            boxed(DecoratedBox::new().class("term-body").child(body))
        } else {
            boxed(
                DecoratedBox::new()
                    .style("height", syngui::StyleValue::px(TERM_MAX_H))
                    .child(ScrollView::new().vertical().child(DecoratedBox::new().class("term-body").child(body))),
            )
        }
    };
    boxed(
        DecoratedBox::new().class("term").clip(true).child(
            Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(DecoratedBox::new().class("term-bar").child(bar)).child(body),
        ),
    )
}

/// Карточка вызова инструмента: строка и раскрывающееся окно вывода.
#[allow(clippy::too_many_arguments)]
fn tool_card(st: St, key: String, name: &str, summary: &str, state: ToolState, lines: Vec<(String, bool)>, default_open: bool, extra: Vec<W>) -> W {
    let open = st.tool_open.with(|m| m.get(&key).copied()).unwrap_or(default_open);
    let n = lines.iter().filter(|(l, _)| !l.is_empty()).count();
    let head = GestureDetector::new().on_click(move || st.toggle_tool(&key, default_open)).child(tool_row(name, summary, state, open, n));
    let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head);
    if open {
        let title = if summary.is_empty() || name == "bash" { summary.to_string() } else { format!("{name} · {summary}") };
        col = col.child(DecoratedBox::new().class("term-wrap").child(term_window(st, if title.is_empty() { name } else { &title }, &lines, state)));
    }
    for w in extra {
        col = col.child(w);
    }
    boxed(col.class("step"))
}

fn done_tool_card(st: St, index: usize, m: &ChatMsg) -> W {
    let run = m.tool.clone().unwrap_or(syn_assist::chat::ToolRun { name: "tool".into(), summary: String::new(), ok: true });
    let lines = crate::state::strip_ansi(&m.text).lines().map(|l| (l.to_string(), false)).collect();
    tool_card(st, st.tool_key(index), &run.name, &run.summary, if run.ok { ToolState::Ok } else { ToolState::Fail }, lines, false, files_row(&m.files).map(|w| boxed(DecoratedBox::new().class("step-files").child(w))).into_iter().collect())
}

/// Идущий вызов: тот же ключ, что получит карточка результата, — раскрытое окно
/// не схлопывается, когда инструмент закончил.
fn live_tool_card(st: St, l: &Live, name: &str, summary: &str) -> W {
    let mut extra: Vec<W> = Vec::new();
    if let Some((frac, label)) = &l.progress {
        let bar = match frac {
            Some(f) => ProgressBar::new().value(f.clamp(0.0, 1.0)),
            None => ProgressBar::new().indeterminate(),
        }
        .class("tool-progress");
        let pct = frac.map(|f| format!("{:.0}%", f * 100.0)).unwrap_or_default();
        extra.push(boxed(
            Column::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .class("tool-progress-box")
                .child(Row::new().gap(8.0).child(Text::new(label.clone()).max_lines(1).class("hint grow")).child(Text::new(pct).class("hint")))
                .child(bar),
        ));
    }
    tool_card(st, st.live_tool_key(), name, summary, ToolState::Running, l.term.clone(), !l.term.is_empty(), extra)
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
    boxed(Row::new().main_axis_alignment(MainAxisAlignment::End).class("user-row").child(col))
}

fn message(st: St, index: usize, m: &ChatMsg, last_answer: bool) -> Option<W> {
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
            Some(boxed(col.class("assistant")))
        }
        Role::Tool => Some(done_tool_card(st, index, m)),
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

fn live_view(st: St, l: &Live) -> W {
    let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow");
    if !l.thinking.is_empty() {
        col = col.child(thinking_block(&l.thinking, l.text.is_empty()));
    }
    if !l.text.is_empty() {
        col = col.child(markdown(&l.text, "answer"));
    } else if let Some((name, summary)) = &l.tool {
        col = col.child(DecoratedBox::new().class("steps").child(live_tool_card(st, l, name, summary)));
    } else if !l.tool_text.is_empty() {
        col = col.child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(typing()).child(Text::new(t!("Готовит действие…")).class("hint")));
    } else if l.thinking.is_empty() {
        col = col.child(typing());
    }
    boxed(col.class("assistant"))
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
    boxed(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(super::centered(col.class("hero"))).class("empty grow"))
}

fn feed_list(st: St) -> W {
    let list = Reactive::new(move || -> Vec<W> {
        let chat = st.chat.get();
        let busy = st.busy.get();
        let last_idx = chat.messages.iter().rposition(|m| m.role == Role::Assistant && !m.text.is_empty() && !m.compacted);
        let mut col = Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("feed");
        // Подряд идущие вызовы инструментов — одним блоком шагов.
        let mut steps: Vec<W> = Vec::new();
        let flush = |col: Column, steps: &mut Vec<W>| -> Column {
            if steps.is_empty() {
                return col;
            }
            let mut g = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("steps");
            for w in steps.drain(..) {
                g = g.child(w);
            }
            col.child(g)
        };
        for (i, m) in chat.messages.iter().enumerate() {
            let Some(w) = message(st, i, m, !busy && Some(i) == last_idx) else { continue };
            if m.role == Role::Tool {
                steps.push(w);
            } else {
                col = flush(col, &mut steps);
                col = col.child(w);
            }
        }
        col = flush(col, &mut steps);
        vec![boxed(col)]
    });
    let live = Reactive::new(move || -> Vec<W> {
        st.live.get().map(|l| live_view(st, &l)).into_iter().collect()
    });
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
            .child(super::centered(Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("feed-pad column").child(list).child(live).child(error))),
    )
}

pub fn feed(st: St, phone: bool) -> W {
    boxed(DecoratedBox::new().class("grow host").child(Reactive::new(move || -> Vec<W> { if st.empty.get() { vec![empty_state(st, phone)] } else { vec![feed_list(st)] } })))
}
