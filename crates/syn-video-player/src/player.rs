//! Экран плеера: виджет плеера syngui (`VideoPlayerView`) на всё окно, сверху — «назад» и название
//! (прячутся вместе с панелью). Касания: тап — панель / пауза, двойной тап по краям — ∓10 с, по
//! центру — во весь экран; мышь и клавиши — как в плеере syngui.

use std::sync::Arc;

use syngui::prelude::*;
use syngui::widgets::*;

use crate::ui::{back, icons, St};

type W = Box<dyn Widget>;

pub fn view(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let Some(p) = st.playing.get() else {
            return vec![if let Some(e) = st.error.get() {
                centered(Box::new(Text::new(e).max_lines(6).class("player-error-text")))
            } else {
                centered(Box::new(CircularProgress::new().indeterminate().size(48.0).stroke_width(4.0)))
            }];
        };
        let full = st.fullscreen.get();
        let fs = FullscreenCtl {
            active: full,
            toggle: Arc::new(move || {
                let on = !st.fullscreen.get_untracked();
                st.fullscreen.set(on);
                syngui::signal::set_fullscreen(on);
            }),
        };
        let title = p.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        vec![Box::new(
            VideoPlayerView::file(p.player.clone())
                .fullscreen(fs)
                .header(move || header(st, title.clone()))
                .build(),
        )]
    });
    Box::new(Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().class("player-bg")).child(body))
}

fn header(st: St, title: String) -> W {
    Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(
                ToolButton::new(icons::ARROW_BACK)
                    .tooltip("Назад")
                    .on_click(move || {
                        back(st);
                    })
                    .class("vp-btn"),
            )
            .child(Text::new(title).max_lines(1).class("player-title")),
    )
}

fn centered(w: W) -> W {
    Box::new(
        Column::new()
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(w)
            .class("player-center"),
    )
}
