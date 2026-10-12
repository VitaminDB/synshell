use syngui::prelude::*;
use syngui::widgets::input::MultilineTextEdit;
use syngui::widgets::visual::canvas::Canvas;
use synassist_voice::{Phase, VisHandle};

use super::{boxed, icon, tip, W};
use crate::icons;
use crate::state::{shared, St};

fn attachments(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let files = st.attachments.get();
        if files.is_empty() {
            return vec![];
        }
        let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).class("att-row");
        for (i, f) in files.iter().enumerate() {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            row = row.child(Chip::new(name).icon(icons::ATTACH).deletable().on_delete(move || st.remove_attachment(i)).class("att-chip"));
        }
        vec![boxed(row)]
    }))
}

fn wave(handle: Option<VisHandle>) -> W {
    boxed(
        Canvas::new(move |ctx, t| {
            let c = ctx.mss_accent().unwrap_or_else(|| Color::from_hex("#3B82F6"));
            let bars = match &handle {
                Some(h) => h.snapshot_bars(18),
                None => (0..18).map(|i| ((t * 3.0 + i as f32 * 0.6).sin() * 0.5 + 0.5) * 0.3).collect(),
            };
            let n = bars.len().max(1) as f32;
            let w = ctx.width() / n;
            let h = ctx.height();
            for (i, v) in bars.iter().enumerate() {
                let amp = (v.clamp(0.0, 1.0) * h * 0.9).max(3.0);
                let x = i as f32 * w + w * 0.5;
                ctx.set_stroke_width((w * 0.55).max(2.0));
                ctx.set_color(c.with_alpha(0.55 + v.clamp(0.0, 1.0) * 0.45));
                ctx.draw_line(x, h * 0.5 - amp * 0.5, x, h * 0.5 + amp * 0.5);
            }
        })
        .size(120.0, 26.0)
        .animated(true)
        .class("wave"),
    )
}

fn dictation_hint(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        if !st.voice.dictating.get() {
            return vec![];
        }
        let phase = st.voice.phase.get();
        let label = match phase {
            Phase::Transcribing => t!("Распознаю…"),
            Phase::Starting => t!("Включаю микрофон…"),
            _ => t!("Говорите — пауза завершит фразу, или нажмите микрофон ещё раз"),
        };
        vec![boxed(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("dict-row")
                .child(wave(st.voice.vis.get()))
                .child(Text::new(label).max_lines(1).elide(Elide::End).class("dict-text grow")),
        )]
    }))
}

pub fn compose(st: St) -> W {
    let field = Reactive::new(move || -> Vec<W> {
        st.input_rev.get();
        let draft = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let d2 = draft.clone();
        let edit = MultilineTextEdit::new()
            .placeholder(t!("Спросите что-нибудь…"))
            .rows(1)
            .max_rows(10)
            .auto_height(true)
            .soft_wrap(true)
            .submit_on_enter(true)
            .insert_queue(shared().insert.clone())
            .on_change(move |s: &str| *d2.lock().unwrap() = s.to_string())
            .on_submit(move |s: &str| st.send(s.to_string()))
            .class("input grow");
        let mic = Reactive::new(move || -> Vec<W> {
            let on = st.voice.dictating.get();
            let asr = st.service.get().asr.is_some();
            let class = if on { "btn round mic on" } else if asr { "btn round mic" } else { "btn round mic off" };
            let label = if on { t!("Закончить диктовку") } else if asr { t!("Надиктовать (Ctrl+M)") } else { t!("Модель распознавания речи не установлена") };
            vec![tip(Button::new("").icon(if on { icons::STOP } else { icons::MIC }).class(class).on_click(move || crate::voice::toggle_dictation(st)), label)]
        });
        let send = Reactive::new(move || -> Vec<W> {
            let draft = draft.clone();
            if st.busy.get() {
                vec![tip(Button::new("").icon(icons::STOP).class("btn round stop").on_click(move || st.stop()), t!("Остановить (Esc)"))]
            } else {
                vec![tip(Button::new("").icon(icons::SEND).class("btn round send").on_click(move || st.send(draft.lock().unwrap().clone())), t!("Отправить (Enter)"))]
            }
        });
        let attach = tip(Button::new("").icon(icons::ATTACH).class("btn round flat attach").on_click(move || st.attach()), t!("Прикрепить файл"));
        vec![boxed(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::End).child(attach).child(edit).child(mic).child(send))]
    });
    let hint = Reactive::new(move || -> Vec<W> {
        let s = st.service.get();
        if s.online && s.llm.is_some() {
            return vec![];
        }
        let text = if !s.online { t!("Служба нейросетей не отвечает — ответы недоступны") } else { t!("Языковая модель не установлена: Параметры → Нейросети") };
        vec![boxed(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(icons::CLOUD).class("ic warn-ic")).child(Text::new(text).max_lines(2).class("warn-text")))]
    });
    boxed(
        DecoratedBox::new().class("compose-wrap").child(
            Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(hint)
                .child(DecoratedBox::new().class("composer").child(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(attachments(st)).child(dictation_hint(st)).child(field))),
        ),
    )
}
