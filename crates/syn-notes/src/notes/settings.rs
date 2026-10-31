//! Экран «Настройки» приложения заметок: вид, чтение (листание), рукопись.
//! Значения — [`super::prefs`]; меняются сразу, без «Сохранить».

use syngui::input::CursorIcon;
use syngui::prelude::*;
use syngui::widgets::{GestureDetector, Toggle};

use crate::icons::*;

use super::library::{self, Library};
use super::prefs::{self, Prefs, SwipeDir, TapMode};
use super::state::NotesCtx;

type W = Box<dyn Widget>;

pub fn view(lib: Library, wide: bool) -> W {
    let ctx = use_context::<NotesCtx>();
    let body = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("lib-main")
        .child(header(ctx, lib, wide))
        .child(DecoratedBox::new().class("grow").child(Stack::new().fit(StackFit::Expand).child(
            ScrollView::new().vertical().child(DecoratedBox::new().class("lib-content").child(content())),
        )));
    library::library_with(ctx, lib, wide, Some(Box::new(body)))
}

fn header(ctx: NotesCtx, lib: Library, wide: bool) -> impl Widget {
    let row = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(library::back_button(ctx, lib))
        .child(
            DecoratedBox::new().class("grow").child(
                Column::new()
                    .gap(2.0)
                    .child(Text::new(tr!("notes.lib.settings")).class(if wide { "lib-h1" } else { "lib-h1 narrow" }))
                    .child(Text::new(tr!("notes.set.sub")).class("lib-h1-sub")),
            ),
        );
    DecoratedBox::new().class(if wide { "lib-header" } else { "lib-header narrow" }).child(row)
}

fn content() -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let _ = prefs::rev().get();
        let p = prefs::get();
        let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);

        col = col.child(section(MI_PALETTE, tr!("notes.set.look")));
        col = col.child(card(vec![switch(
            tr!("notes.set.pure_black"),
            tr!("notes.set.pure_black.hint"),
            p.pure_black,
            |v| prefs::update(|p| p.pure_black = v),
        )]));

        col = col.child(section(MI_AUTO_STORIES, tr!("notes.set.reading")));
        col = col.child(card(vec![
            choice(
                tr!("notes.set.tap"),
                tr!("notes.set.tap.hint"),
                vec![
                    (tr!("notes.set.tap.lr"), p.tap_mode == TapMode::LeftRight),
                    (tr!("notes.set.tap.tb"), p.tap_mode == TapMode::TopBottom),
                    (tr!("notes.set.tap.fwd"), p.tap_mode == TapMode::Forward),
                ],
                |i| prefs::update(|p| p.tap_mode = [TapMode::LeftRight, TapMode::TopBottom, TapMode::Forward][i]),
            ),
            choice(
                tr!("notes.set.swipe"),
                tr!("notes.set.swipe.hint"),
                vec![
                    (tr!("notes.set.swipe.h"), p.swipe == SwipeDir::Horizontal),
                    (tr!("notes.set.swipe.v"), p.swipe == SwipeDir::Vertical),
                    (tr!("notes.set.swipe.both"), p.swipe == SwipeDir::Both),
                    (tr!("notes.set.swipe.off"), p.swipe == SwipeDir::Off),
                ],
                |i| prefs::update(|p| p.swipe = [SwipeDir::Horizontal, SwipeDir::Vertical, SwipeDir::Both, SwipeDir::Off][i]),
            ),
            switch(tr!("notes.set.invert"), tr!("notes.set.invert.hint"), p.tap_invert, |v| prefs::update(|p| p.tap_invert = v)),
            switch(tr!("notes.set.wheel"), tr!("notes.set.wheel.hint"), p.wheel_turns, |v| prefs::update(|p| p.wheel_turns = v)),
        ]));

        col = col.child(section(MI_EDIT_NOTE, tr!("notes.set.ink")));
        col = col.child(card(vec![
            switch(tr!("notes.set.finger"), tr!("notes.set.finger.hint"), p.ink_finger, |v| prefs::update(|p| p.ink_finger = v)),
            switch(tr!("notes.set.crisp"), tr!("notes.set.crisp.hint"), p.ink_crisp, |v| prefs::update(|p| p.ink_crisp = v)),
            switch(tr!("notes.set.hold"), tr!("notes.set.hold.hint"), p.ink_hold_shape, |v| prefs::update(|p| p.ink_hold_shape = v)),
        ]));

        col = col.child(
            DecoratedBox::new().class("set-reset").child(
                GestureDetector::new()
                    .cursor(CursorIcon::Pointer)
                    .on_click(|| prefs::update(|p| *p = Prefs::default()))
                    .child(Text::new(tr!("notes.set.reset")).class("lib-more")),
            ),
        );
        vec![Box::new(col)]
    })
}

fn section(icon: &'static str, title: String) -> W {
    Box::new(
        DecoratedBox::new().class("lib-h2-row").child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(icon).class("set-sec-icon"))
                .child(Text::new(title).class("lib-h2")),
        ),
    )
}

fn card(rows: Vec<W>) -> W {
    Box::new(DecoratedBox::new().class("set-card").child(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).children(rows)))
}

fn label(title: String, hint: String) -> impl Widget {
    let mut col = Column::new().gap(2.0).child(Text::new(title).class("set-title"));
    if !hint.is_empty() {
        col = col.child(Text::new(hint).max_lines(3).class("set-hint"));
    }
    col
}

fn switch(title: String, hint: String, on: bool, f: impl Fn(bool) + Send + Sync + 'static) -> W {
    Box::new(
        DecoratedBox::new().class("set-row").child(
            Row::new()
                .gap(16.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("grow").child(label(title, hint)))
                .child(Toggle::with_state(on).on_change(move |v| f(v))),
        ),
    )
}

fn choice(title: String, hint: String, options: Vec<(String, bool)>, f: impl Fn(usize) + Send + Sync + 'static) -> W {
    let f = std::sync::Arc::new(f);
    let chips: Vec<W> = options
        .into_iter()
        .enumerate()
        .map(|(i, (t, on))| -> W {
            let f = f.clone();
            Box::new(
                GestureDetector::new()
                    .cursor(CursorIcon::Pointer)
                    .on_click(move || f(i))
                    .child(DecoratedBox::new().class(if on { "set-chip active" } else { "set-chip" }).child(Text::new(t).class("set-chip-text"))),
            )
        })
        .collect();
    Box::new(
        DecoratedBox::new().class("set-row").child(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(label(title, hint))
                .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).children(chips)),
        ),
    )
}
