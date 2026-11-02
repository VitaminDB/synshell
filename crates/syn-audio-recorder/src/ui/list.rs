//! Записи: список (название, дата, длительность, размер, миниатюра волны) и плеер выбранной записи
//! (волна с перемоткой касанием, ±10 с, переименовать, удалить).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use syngui::prelude::*;
use syngui::GestureDetector;
use syngui::widgets::PanAxis;

use super::{boxed, icon, icon_button, icons, tip, wave, St, W};
use crate::store::{self, Recording};

fn row(st: St, r: Recording, selected: bool, playing: bool, now: i64) -> W {
    let path = r.path.clone();
    let meta = if r.pending {
        format!("{} · {}", store::when(r.created, now), store::size(r.size))
    } else {
        format!("{} · {} · {}", store::when(r.created, now), store::duration(r.duration), store::size(r.size))
    };
    let play_ic = if selected && playing { icons::PAUSE } else { icons::PLAY };
    let badge = DecoratedBox::new().class(if selected { "row-play on" } else { "row-play" }).child(icon(play_ic).class("ic row-play-ic"));
    let thumb = wave::peaks(r.peaks.clone(), None, 3.0).class(if selected { "thumb on" } else { "thumb" });
    let body = Row::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(badge)
        .child(
            DecoratedBox::new().class("grow").child(
                Column::new()
                    .gap(3.0)
                    .child(Text::new(r.title.clone()).max_lines(1).elide(Elide::End).class(if selected { "row-title on" } else { "row-title" }))
                    .child(Text::new(meta).max_lines(1).elide(Elide::End).class("row-meta")),
            ),
        )
        .child(thumb);
    boxed(GestureDetector::new().on_click(move || st.play(path.clone())).child(DecoratedBox::new().class(if selected { "row on" } else { "row" }).child(body)))
}

fn rows(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let list = st.recordings.get();
        let sel = st.selected.get();
        let playing = st.playing.get();
        if list.is_empty() {
            let text = if st.scanning.get() { t!("Читаю папку…") } else { t!("Записей пока нет.\nНажмите красную кнопку, чтобы начать.") };
            return vec![boxed(
                Column::new()
                    .gap(12.0)
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class("empty")
                    .child(DecoratedBox::new().class("empty-badge").child(icon(icons::WAVE).class("ic empty-ic")))
                    .child(Text::new(text).max_lines(4).class("empty-text")),
            )];
        }
        let now = store::now();
        let col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("rows").children(list.into_iter().map(|r| {
            let on = sel.as_ref() == Some(&r.path);
            row(st, r, on, on && playing, now)
        }));
        vec![boxed(ScrollView::new().vertical().child(col))]
    }))
}

/// Волна плеера: касание и протяжка — перемотка.
fn seek_wave(st: St, peaks: Vec<u8>) -> W {
    let width = Arc::new(AtomicU32::new(0));
    let w1 = width.clone();
    let w2 = width.clone();
    let canvas = Reactive::new(move || -> Vec<W> {
        let d = st.dur.get();
        let p = st.pos.get();
        let prog = if d > 0.0 { (p / d) as f32 } else { 0.0 };
        vec![boxed(wave::peaks(peaks.clone(), Some(prog), 4.0).class("seek-wave"))]
    });
    let to_sec = move |x: f32, w: f32| -> f64 {
        let d = st.dur.get_untracked();
        if w <= 0.0 {
            return st.pos.get_untracked();
        }
        (x / w).clamp(0.0, 1.0) as f64 * d
    };
    boxed(
        GestureDetector::new()
            .pan_axis(PanAxis::Horizontal)
            .on_press_with_bounds(move |p, r| {
                w1.store(r.size.width.to_bits(), Ordering::Relaxed);
                st.seek(to_sec(p.x - r.origin.x, r.size.width));
            })
            .on_pan_update(move |u| {
                let w = f32::from_bits(w2.load(Ordering::Relaxed));
                st.seek(to_sec(u.position.x, w));
            })
            .child(canvas),
    )
}

/// Плеер выбранной записи.
pub fn player(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let Some(path) = st.selected.get() else { return vec![] };
        let Some(r) = st.recordings.get().into_iter().find(|r| r.path == path) else { return vec![] };
        let head = Row::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class("grow").child(Text::new(r.title.clone()).max_lines(1).elide(Elide::End).class("pl-title")))
            .child({
                let p = path.clone();
                icon_button(icons::EDIT, t!("Переименовать"), "pl-act", move || st.rename.set(Some(p.clone())))
            })
            .child({
                let p = path.clone();
                icon_button(icons::DELETE, t!("Удалить"), "pl-act danger", move || st.delete(p.clone()))
            })
            .child(icon_button(icons::CLOSE, t!("Закрыть"), "pl-act", move || st.close_player()));
        let times = Reactive::new(move || -> Vec<W> {
            let p = st.pos.get();
            let d = st.dur.get();
            vec![boxed(
                Row::new()
                    .child(Text::new(store::duration(p)).class("pl-time"))
                    .child(DecoratedBox::new().class("grow"))
                    .child(Text::new(format!("−{}", store::duration((d - p).max(0.0)))).class("pl-time")),
            )]
        });
        let ctl = Reactive::new(move || -> Vec<W> {
            let playing = st.playing.get();
            vec![boxed(
                Row::new()
                    .gap(18.0)
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(icon_button(icons::BACK10, t!("Назад на 10 с"), "pl-skip", move || st.skip(-10.0)))
                    .child(tip(Button::new("").icon(if playing { icons::PAUSE } else { icons::PLAY }).class("btn pl-main").on_click(move || st.toggle_play()), t!("Играть / пауза (K)")))
                    .child(icon_button(icons::FWD10, t!("Вперёд на 10 с"), "pl-skip", move || st.skip(10.0))),
            )]
        });
        vec![boxed(DecoratedBox::new().class("player-wrap").child(
            DecoratedBox::new().class("player").child(
                Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head).child(seek_wave(st, r.peaks.clone())).child(times).child(ctl),
            ),
        ))]
    }))
}

fn header(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let list = st.recordings.get();
        let total: f64 = list.iter().map(|r| r.duration).sum();
        let sub = if list.is_empty() { String::new() } else { format!("{} · {}", tn!(list.len(), "{n} запись", "{n} записи", "{n} записей"), store::duration(total)) };
        vec![boxed(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("list-head")
                .child(DecoratedBox::new().class("grow").child(Column::new().gap(2.0).child(Text::new(t!("Записи")).class("list-title")).child(Text::new(sub).class("row-meta"))))
                .child(icon_button(icons::FOLDER, t!("Открыть папку"), "tb-btn", move || st.open_folder())),
        )]
    }))
}

/// Боковая панель широкого окна.
pub fn sidebar(st: St) -> W {
    boxed(
        DecoratedBox::new().class("sidebar").child(
            Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(header(st)).child(DecoratedBox::new().class("grow").child(rows(st))).child(player(st)),
        ),
    )
}

/// Экран списка на узком окне.
pub fn page(st: St) -> W {
    boxed(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("list-page").child(DecoratedBox::new().class("grow").child(rows(st))).child(player(st)))
}
