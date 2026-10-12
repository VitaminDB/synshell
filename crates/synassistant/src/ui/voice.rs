use std::sync::{Arc, Mutex};

use syngui::prelude::*;
use syngui::widgets::visual::canvas::Canvas;
use syngui::widgets::visual::MarkdownView;
use syngui::widgets::Stack;
use synassist_voice::{Phase, VadState, VisHandle};

use super::{boxed, icon, tip, W};
use crate::icons;
use crate::state::St;

const AURA: f32 = 260.0;

fn aura(handle: Option<VisHandle>, phase: Phase) -> W {
    let prev_bars: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(vec![0.0; 32]));
    let prev_level: Arc<Mutex<f32>> = Arc::new(Mutex::new(0.0));
    boxed(
        Canvas::new(move |ctx, t| {
            let accent = ctx.mss_accent().unwrap_or_else(|| Color::from_hex("#3B82F6"));
            let second = ctx.mss_color().unwrap_or_else(|| Color::from_hex("#8b5cf6"));
            let (cx, cy) = (ctx.width() * 0.5, ctx.height() * 0.5);
            let inner = 56.0;
            let outer = ctx.width().min(ctx.height()) * 0.5 - 8.0;
            let (target, level) = match (&handle, phase) {
                (Some(h), Phase::Listening) => (h.snapshot_bars(32), h.level()),
                _ => (vec![0.0; 32], 0.0),
            };
            let mut bars = prev_bars.lock().unwrap_or_else(|e| e.into_inner());
            for (i, v) in target.iter().enumerate() {
                bars[i] += (v - bars[i]) * 0.3;
            }
            let mut pl = prev_level.lock().unwrap_or_else(|e| e.into_inner());
            *pl += (level - *pl) * 0.2;
            match phase {
                Phase::Listening => {
                    for i in 0..3 {
                        let p = ((t / 2.4) + i as f32 / 3.0).fract();
                        let r = inner + (outer - inner) * p;
                        ctx.set_stroke_width(1.5 + *pl * 6.0);
                        ctx.set_color(accent.with_alpha((1.0 - p) * 0.45 * (0.5 + *pl)));
                        ctx.stroke_circle(cx, cy, r);
                    }
                    let n = bars.len() as f32;
                    for (i, v) in bars.iter().enumerate() {
                        let a = (i as f32 / n) * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                        let r0 = inner + 10.0;
                        let r1 = r0 + (outer - r0 - 8.0) * v.clamp(0.0, 1.0);
                        ctx.set_stroke_width(4.0);
                        ctx.set_color(accent.with_alpha(0.55 + v.clamp(0.0, 1.0) * 0.45));
                        ctx.draw_line(cx + a.cos() * r0, cy + a.sin() * r0, cx + a.cos() * r1, cy + a.sin() * r1);
                    }
                }
                Phase::Thinking | Phase::Transcribing | Phase::Starting => {
                    let r = inner + 22.0;
                    for i in 0..3 {
                        let start = t * 2.0 + i as f32 * 2.1;
                        ctx.set_stroke_width(4.0);
                        ctx.set_color(if i == 1 { second } else { accent }.with_alpha(0.75));
                        ctx.draw_arc(cx, cy, r + i as f32 * 14.0, start, start + 1.2 + (t * 1.5 + i as f32).sin() * 0.6);
                    }
                }
                Phase::Speaking => {
                    for i in 0..4 {
                        let p = ((t / 1.6) + i as f32 * 0.25).fract();
                        let r = inner + (outer - inner) * p;
                        let wobble = (t * 9.0 + i as f32).sin() * 3.0;
                        ctx.set_stroke_width(3.0 + (1.0 - p) * 4.0);
                        ctx.set_color(if i % 2 == 0 { second } else { accent }.with_alpha((1.0 - p) * 0.6));
                        ctx.stroke_circle(cx, cy, r + wobble);
                    }
                }
                Phase::Off => {
                    ctx.set_stroke_width(2.0);
                    ctx.set_color(accent.with_alpha(0.3));
                    ctx.stroke_circle(cx, cy, inner + 16.0);
                }
            }
            let glow = 0.25 + *pl * 0.5;
            ctx.set_color(accent.with_alpha(glow * 0.5));
            ctx.fill_circle(cx, cy, inner + 6.0);
            ctx.set_color(accent);
            ctx.fill_circle(cx, cy, inner - 4.0);
        })
        .size(AURA, AURA)
        .animated(true)
        .class("aura"),
    )
}

fn center_button(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let phase = st.voice.phase.get();
        let muted = st.voice.muted.get();
        let (glyph, label): (&str, String) = match phase {
            Phase::Speaking => (icons::STOP, t!("Остановить озвучку").to_string()),
            Phase::Listening if muted => (icons::MIC_OFF, t!("Микрофон выключен — включить").to_string()),
            Phase::Listening => (icons::MIC, t!("Слушаю. Нажмите, чтобы выключить микрофон").to_string()),
            Phase::Thinking => (icons::STOP, t!("Прервать ответ").to_string()),
            _ => (icons::MIC, t!("Подождите…").to_string()),
        };
        let btn = Button::new("").icon(glyph).class("btn voice-center").on_click(move || match phase {
            Phase::Speaking => crate::voice::stop_speaking(),
            Phase::Listening => crate::voice::set_muted(st, !muted),
            Phase::Thinking => st.stop(),
            _ => {}
        });
        vec![boxed(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).class("voice-center-slot").child(tip(btn, label)))]
    }))
}

fn status(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let phase = st.voice.phase.get();
        let muted = st.voice.muted.get();
        let note = st.voice.note.get();
        let s = st.service.get();
        let text = if let Some(n) = note {
            n
        } else if s.online && (s.asr.is_none() || s.tts.is_none()) {
            if s.asr.is_none() {
                t!("Нет модели распознавания речи — добавьте её в Параметры → Нейросети").to_string()
            } else {
                t!("Нет модели озвучки — ответы будут только текстом").to_string()
            }
        } else {
            match phase {
                Phase::Off => t!("Голосовой режим выключен").to_string(),
                Phase::Starting => t!("Включаю микрофон…").to_string(),
                Phase::Listening if muted => t!("Микрофон выключен").to_string(),
                Phase::Listening => t!("Слушаю…").to_string(),
                Phase::Transcribing => t!("Распознаю…").to_string(),
                Phase::Thinking => t!("Думаю…").to_string(),
                Phase::Speaking => t!("Говорю…").to_string(),
            }
        };
        let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(text).max_lines(2).class("voice-status"));
        if phase == Phase::Listening && !muted {
            if let Some(s) = st.voice.vad.get() {
                col = col.child(voice_meter(s));
            }
        }
        vec![boxed(col)]
    }))
}

/// Индикатор детектора речи (как проверка микрофона в synspeak): полоса уровня
/// с отметкой порога тишины, цифры и что детектор слышит.
fn voice_meter(s: VadState) -> impl Widget {
    let (text, class) = if s.in_phrase && s.speaking {
        (t!("Слышу речь"), "voice-meter-state on")
    } else if s.in_phrase {
        (t!("Пауза — здесь фраза закончится"), "voice-meter-state pause")
    } else if s.speaking {
        (t!("Громко — похоже на начало речи"), "voice-meter-state on")
    } else {
        (t!("Тишина"), "voice-meter-state")
    };
    let full = 0.3;
    Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            Stack::new()
                .child(ProgressBar::with_value((s.level / full).clamp(0.0, 1.0)).class(if s.speaking { "voice-meter on" } else { "voice-meter" }))
                .child(
                    Row::new()
                        .class("voice-meter-mark-row")
                        .child(Column::new().width((s.silence / full).clamp(0.0, 1.0) * 300.0))
                        .child(DecoratedBox::new().class("voice-meter-mark")),
                ),
        )
        .child(
            Text::new(t!(
                "уровень {l} % · фон {f} % · тишина ниже {s} % · речь выше {p} %",
                l = format!("{:.1}", s.level * 100.0),
                f = format!("{:.1}", s.floor * 100.0),
                s = format!("{:.1}", s.silence * 100.0),
                p = format!("{:.1}", s.speech * 100.0)
            ))
            .class("voice-meter-text"),
        )
        .child(Text::new(text).class(class))
}

fn transcript(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let heard = st.voice.heard.get();
        let live = st.live.get();
        let answer = if let Some(l) = live.as_ref().filter(|l| !l.text.is_empty()) {
            l.text.clone()
        } else if st.voice.phase.get() == Phase::Speaking || !st.busy.get() {
            st.last_answer().unwrap_or_default()
        } else {
            String::new()
        };
        let tool = live.as_ref().and_then(|l| l.tool.clone());
        let mut col = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("voice-transcript");
        if !heard.is_empty() {
            col = col.child(Row::new().main_axis_alignment(MainAxisAlignment::End).child(DecoratedBox::new().class("user-bubble").child(Text::new(heard.clone()).selectable(true).class("user-text"))));
        }
        if let Some((name, summary)) = tool {
            col = col.child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class("tool-badge").child(icon(icons::tool(&name)).class("ic tool-ic")))
                    .child(Text::new(summary).max_lines(1).elide(Elide::End).class("tool-sum grow"))
                    .child(CircularProgress::new().indeterminate().size(16.0).stroke_width(2.0).class("spin"))
                    .class("tool-card"),
            );
        }
        if !answer.is_empty() && !heard.is_empty() {
            col = col.child(ScrollView::new().vertical().follow_end(true).class("voice-answer-scroll").child(MarkdownView::new(answer).selectable(true).class("answer voice-answer")));
        }
        vec![boxed(col)]
    }))
}

fn confirm(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let Some((tool, summary)) = st.confirm.get() else { return vec![] };
        vec![boxed(
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .class("confirm voice")
                .child(
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(DecoratedBox::new().class("confirm-badge").child(icon(icons::tool(&tool)).class("ic confirm-ic")))
                        .child(Column::new().gap(2.0).class("grow").child(Text::new(t!("Разрешить действие?")).class("confirm-title")).child(Text::new(summary).max_lines(4).class("confirm-sum"))),
                )
                .child(
                    Row::new()
                        .gap(8.0)
                        .main_axis_alignment(MainAxisAlignment::End)
                        .child(Button::new(t!("Отклонить")).class("btn ghost").on_click(move || st.answer(false)))
                        .child(Button::new(t!("Разрешить")).icon(icons::CHECK).class("btn primary").on_click(move || st.answer(true))),
                ),
        )]
    }))
}

fn actions(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let muted = st.voice.muted.get();
        let speaking = st.voice.speaking.get();
        let mut row = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).main_axis_alignment(MainAxisAlignment::Center);
        row = row.child(tip(
            Button::new("").icon(if muted { icons::MIC_OFF } else { icons::MIC }).class(if muted { "btn round voice-act off" } else { "btn round voice-act" }).on_click(move || crate::voice::set_muted(st, !muted)),
            if muted { t!("Включить микрофон") } else { t!("Выключить микрофон") },
        ));
        if speaking {
            row = row.child(tip(Button::new("").icon(icons::VOLUME_OFF).class("btn round voice-act").on_click(|| crate::voice::stop_speaking()), t!("Замолчать")));
        }
        row = row.child(tip(Button::new("").icon(icons::KEYBOARD).class("btn round voice-act").on_click(move || crate::voice::stop_conversation(st)), t!("Вернуться к тексту (Esc)")));
        row = row.child(tip(Button::new("").icon(icons::CALL_END).class("btn round voice-act end").on_click(move || crate::voice::stop_conversation(st)), t!("Завершить голосовой разговор")));
        vec![boxed(row)]
    }))
}

pub fn overlay(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        if !st.voice.open.get() {
            return vec![];
        }
        let stage = Reactive::new(move || -> Vec<W> {
            let handle = st.voice.vis.get();
            let phase = st.voice.phase.get();
            vec![boxed(DecoratedBox::new().class("aura-wrap").child(Stack::new().clip(false).child(aura(handle, phase)).child(center_button(st))))]
        });
        let card = Column::new()
            .gap(14.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("voice-card")
            .child(stage)
            .child(status(st))
            .child(transcript(st))
            .child(confirm(st))
            .child(actions(st));
        vec![boxed(
            DecoratedBox::new()
                .class("voice-scrim")
                .child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).class("voice-host").child(card)),
        )]
    }))
}
