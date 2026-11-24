//! Таймлайн: линейка, дорожки, клипы, позиция воспроизведения. Щелчок по
//! клипу выделяет его, по линейке или пустому месту — ставит позицию.

use serde_json::json;
use syngui::prelude::*;
use syngui::widgets::*;
use syngui::containers::Positioned;

use super::{ed, St};
use crate::doc::{Source, TrackKind};

const HEAD: f32 = 96.0;
const RULER: f32 = 24.0;
const ROW_V: f32 = 52.0;
const ROW_A: f32 = 38.0;

fn row_h(k: TrackKind) -> f32 {
    if k == TrackKind::Video {
        ROW_V
    } else {
        ROW_A
    }
}

/// Шаг делений линейки, с: чтобы подписи не слипались.
fn tick_step(zoom: f32) -> f64 {
    for s in [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0] {
        if s as f32 * zoom >= 70.0 {
            return s;
        }
    }
    1200.0
}

fn label(t: f64) -> String {
    let s = t.round() as i64;
    if t.fract().abs() > 1e-6 && t < 60.0 {
        format!("{t:.1}")
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn view(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let zoom = st.zoom.get();
        let e = ed();
        let p = &e.project;
        // видеодорожки сверху вниз: последняя — верхний слой
        let mut order: Vec<usize> = (0..p.tracks.len()).filter(|i| p.tracks[*i].kind == TrackKind::Video).rev().collect();
        order.extend((0..p.tracks.len()).filter(|i| p.tracks[*i].kind == TrackKind::Audio));
        let width = ((p.duration() as f32 + 30.0) * zoom).max(1200.0);
        let height = RULER + order.iter().map(|i| row_h(p.tracks[*i].kind)).sum::<f32>() + 8.0;

        // заголовки дорожек
        let mut heads = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(DecoratedBox::new().class("tl-corner").child(Text::new(crate::doc::tc(e.playhead)).class("tl-time")));
        for &i in &order {
            let t = &p.tracks[i];
            let mut flags = String::new();
            if t.muted {
                flags.push_str(" 🔇");
            }
            if t.hidden {
                flags.push_str(" ◌");
            }
            heads = heads.child(DecoratedBox::new().class(if t.kind == TrackKind::Video { "tl-head tl-head-v" } else { "tl-head tl-head-a" }).child(Text::new(format!("{}{flags}", t.name)).max_lines(1).class("tl-head-text")));
        }

        // поле: линейка, дорожки, клипы
        let mut field = Stack::new();
        field = field.child(Positioned::new(DecoratedBox::new().class("tl-ruler")).at(0.0, 0.0).dimensions(width, RULER));
        let step = tick_step(zoom);
        let mut t = 0.0;
        while (t as f32) * zoom < width {
            let x = t as f32 * zoom;
            field = field.child(Positioned::new(DecoratedBox::new().class("tl-tick")).at(x, RULER - 8.0).dimensions(1.0, 8.0));
            field = field.child(Positioned::new(Text::new(label(t)).class("tl-tick-text")).at(x + 3.0, 3.0));
            t += step;
        }
        let mut y = RULER;
        for &i in &order {
            let tr = &p.tracks[i];
            let h = row_h(tr.kind);
            field = field.child(Positioned::new(DecoratedBox::new().class("tl-row")).at(0.0, y).dimensions(width, h));
            for c in &tr.clips {
                let (class, name) = match &c.source {
                    Source::Media { media } => (if tr.kind == TrackKind::Audio { "clip clip-audio" } else { "clip clip-video" }, p.media(*media).map(|m| m.name()).unwrap_or_default()),
                    Source::Title { text, .. } => ("clip clip-title", format!("T  {text}")),
                    Source::Color { color } => ("clip clip-color", color.clone()),
                };
                let selected = e.selection.contains(&c.id);
                let fx = if c.effects.is_empty() { String::new() } else { format!("  ✦{}", c.effects.len()) };
                let w = (c.duration as f32 * zoom - 2.0).max(3.0);
                let id = c.id;
                let clip = DecoratedBox::new().class(class).class(if selected { "clip-selected" } else { "" }).child(Text::new(format!("{name}{fx}")).max_lines(1).elide(Elide::End).class("clip-text"));
                field = field.child(
                    Positioned::new(GestureDetector::new().on_click(move || {
                        st.exec("selection.set", json!({"ids": [id]}));
                    }).child(clip))
                    .at(c.start as f32 * zoom + 1.0, y + 3.0)
                    .dimensions(w, h - 6.0),
                );
            }
            y += h;
        }
        // позиция воспроизведения
        let px = e.playhead as f32 * zoom;
        field = field.child(Positioned::new(DecoratedBox::new().class("tl-playhead")).at(px, 0.0).dimensions(2.0, height));
        field = field.child(Positioned::new(DecoratedBox::new().class("tl-playhead-cap")).at(px - 5.0, 0.0).dimensions(12.0, 10.0));
        drop(e);

        let surface = GestureDetector::new()
            .on_click_at(move |pt: Point| {
                // щелчок мимо клипов: позиция и снятие выделения
                let t = (pt.x / st.zoom.get_untracked()).max(0.0) as f64;
                st.exec("playhead.set", json!({"at": t}));
                if pt.y > RULER {
                    st.exec("selection.set", json!({"ids": []}));
                }
                st.toast.set(None);
            })
            .child(DecoratedBox::new().child(field).style("width", width).style("height", height));

        vec![Box::new(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .class("timeline")
                .child(DecoratedBox::new().child(heads).class("tl-heads").style("width", HEAD))
                .child(DecoratedBox::new().class("grow").child(ScrollView::new().horizontal().vertical().child(surface))),
        )]
    })
}
