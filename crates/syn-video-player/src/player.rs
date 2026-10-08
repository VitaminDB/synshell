//! Экран плеера: виджет плеера syngui (`VideoPlayerView`) на всё окно, сверху — «назад», название,
//! чип «как идёт воспроизведение» (кодек, аппаратно или программно) и настройки — прячутся вместе с
//! панелью. Касания: тап — панель / пауза, двойной тап по краям — ∓шаг перемотки, по центру — во весь
//! экран; мышь и клавиши — как в плеере syngui.

use std::sync::Arc;

use syngui::prelude::*;
use syngui::buttons::Segment;
use syngui::video::DecodeInfo;
use syngui::widgets::*;

use crate::settings::{Settings, SEEK_STEPS};
use crate::ui::{back, icons, reopen, St};

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
        // масштаб и шаг перемотки — свойства виджета: сменились — пересобрать
        let s = st.settings.get();
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
                .fit(if s.fill { ImageFit::Cover } else { ImageFit::Contain })
                .seek_step(s.seek_step as f64)
                .header(move || header(st, title.clone()))
                .build(),
        )]
    });
    let panel = Reactive::new(move || -> Vec<W> {
        if st.settings_open.get() {
            vec![settings_panel(st)]
        } else {
            vec![]
        }
    });
    Box::new(
        Stack::new()
            .fit(StackFit::Expand)
            .child(DecoratedBox::new().class("player-bg"))
            .child(body)
            .child(panel),
    )
}

fn header(st: St, title: String) -> W {
    let chip = Reactive::new(move || -> Vec<W> {
        match (st.settings.get().show_info, st.info.get()) {
            (true, Some(i)) if !i.codec.is_empty() => vec![info_chip(&i)],
            _ => vec![],
        }
    });
    let top = Row::new()
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
        .child(Text::new(title).max_lines(1).class("player-title"))
        .child(DecoratedBox::new().class("grow"))
        .child(
            ToolButton::new(icons::SETTINGS)
                .tooltip("Настройки")
                .on_click(move || st.settings_open.set(true))
                .class("vp-btn"),
        );
    // Чип — под строкой с названием, по центру: на телефоне в портрете рядом с названием ему тесно.
    Box::new(
        Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(top)
            .child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(chip)),
    )
}

/// «H.264 · 1080p | ⚙ Аппаратно · V4L2» — кодек и размер, затем чем декодируется.
fn info_chip(i: &DecodeInfo) -> W {
    let res = resolution_label(i.width, i.height);
    let what = match &i.audio_codec {
        Some(a) => format!("{} · {res} · {a}", i.codec),
        None => format!("{} · {res}", i.codec),
    };
    let (icon, mode, class) = match (&i.hardware, i.known) {
        (_, false) => (icons::HOURGLASS, "Определяю…".to_string(), "chip-mode chip-mode-wait"),
        (Some(hw), true) if i.zero_copy => (icons::MEMORY, format!("Аппаратно · {hw} · без копий"), "chip-mode chip-mode-hw"),
        (Some(hw), true) => (icons::MEMORY, format!("Аппаратно · {hw}"), "chip-mode chip-mode-hw"),
        (None, true) => (icons::CPU, "Программно".to_string(), "chip-mode chip-mode-sw"),
    };
    Box::new(
        DecoratedBox::new()
            .child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(what).max_lines(1).class("chip-what"))
                    .child(
                        DecoratedBox::new()
                            .child(
                                Row::new()
                                    .gap(5.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .child(Icon::new(icon).class("chip-mode-icon"))
                                    .child(Text::new(mode).max_lines(1).class("chip-mode-text")),
                            )
                            .class(class),
                    ),
            )
            .class("info-chip"),
    )
}

fn resolution_label(w: u32, h: u32) -> String {
    let short = w.min(h);
    match short {
        2160.. => "4K".into(),
        1440..=2159 => "1440p".into(),
        1080..=1439 => "1080p".into(),
        720..=1079 => "720p".into(),
        480..=719 => "480p".into(),
        _ => format!("{w}×{h}"),
    }
}

// ─── настройки ──────────────────────────────────────────────────────────────

fn update(st: St, f: impl FnOnce(&mut Settings)) {
    let mut s = st.settings.get_untracked();
    f(&mut s);
    if s != st.settings.get_untracked() {
        s.save();
        st.settings.set(s);
    }
}

fn settings_panel(st: St) -> W {
    let s = st.settings.get_untracked();
    let close = move || st.settings_open.set(false);

    let decoding = section(
        icons::MEMORY,
        "Декодирование",
        vec![
            Box::new(
                SegmentedButton::new(vec![
                    Segment::with_icon("Аппаратно", icons::MEMORY),
                    Segment::with_icon("Программно", icons::CPU),
                ])
                .selected(if s.hardware { 0 } else { 1 })
                .on_change(move |i| {
                    let hw = i == 0;
                    if st.settings.get_untracked().hardware != hw {
                        update(st, |s| s.hardware = hw);
                        // кодек выбирается при открытии — открыть тот же ролик с того же места
                        reopen(st);
                    }
                }),
            ),
            Box::new(Reactive::new(move || -> Vec<W> {
                let text = match st.info.get() {
                    Some(i) if i.known => match &i.hardware {
                        Some(hw) if i.zero_copy => {
                            format!("Сейчас: {} через {hw}, кадры идут на экран прямо из памяти декодера.", i.codec)
                        }
                        Some(hw) => format!("Сейчас: {} через {hw}.", i.codec),
                        None => format!("Сейчас: {} декодирует процессор.", i.codec),
                    },
                    _ => "Аппаратный кодек экономит процессор и батарею; если с ним что-то не так — включите программный.".into(),
                };
                vec![Box::new(Text::new(text).max_lines(4).class("set-hint"))]
            })),
        ],
    );

    let picture = section(
        icons::ASPECT,
        "Изображение",
        vec![
            Box::new(
                SegmentedButton::new(vec![
                    Segment::with_icon("Вписать", icons::FIT),
                    Segment::with_icon("Заполнить", icons::FILL),
                ])
                .selected(if s.fill { 1 } else { 0 })
                .on_change(move |i| update(st, |s| s.fill = i == 1)),
            ),
            hint("«Заполнить» — без чёрных полос, края кадра обрезаются."),
        ],
    );

    let seek = section(
        icons::FAST_FORWARD,
        "Перемотка",
        vec![
            Box::new(
                SegmentedButton::new(SEEK_STEPS.iter().map(|s| format!("{s} с")).collect::<Vec<_>>())
                    .selected(SEEK_STEPS.iter().position(|v| *v == s.seek_step).unwrap_or(1))
                    .on_change(move |i| update(st, |s| s.seek_step = SEEK_STEPS[i.min(SEEK_STEPS.len() - 1)])),
            ),
            hint("Двойной тап по левому или правому краю кадра, кнопки и клавиши J / L."),
        ],
    );

    let playback = section(
        icons::PLAY_CIRCLE,
        "Воспроизведение",
        vec![
            toggle_row("Продолжать с места остановки", "Ролик откроется там, где вы его закрыли.", s.resume, move |on| {
                update(st, |s| s.resume = on)
            }),
            toggle_row("Повторять ролик", "Дошёл до конца — сначала.", s.repeat, move |on| update(st, |s| s.repeat = on)),
        ],
    );

    let view = section(
        icons::INFO,
        "Интерфейс",
        vec![toggle_row(
            "Показывать способ воспроизведения",
            "Чип вверху: кодек, размер, аппаратно или программно.",
            s.show_info,
            move |on| update(st, |s| s.show_info = on),
        )],
    );

    let panel = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new("Настройки").class("set-title"))
                .child(DecoratedBox::new().class("grow"))
                .child(ToolButton::new(icons::CLOSE).tooltip("Закрыть").on_click(close).class("set-close"))
                .class("set-head"),
        )
        .child(
            ScrollView::new()
                .vertical()
                .child(
                    Column::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(decoding)
                        .child(picture)
                        .child(seek)
                        .child(playback)
                        .child(view)
                        .class("set-body"),
                )
                .class("grow"),
        )
        .class("set-panel");

    // Затемнение закрывает по тапу; панель — справа во всю высоту, на узком экране — почти во всю ширину.
    Box::new(
        Stack::new()
            .fit(StackFit::Expand)
            .child(GestureDetector::new().on_click(close).child(DecoratedBox::new().class("set-scrim")))
            .child(
                Row::new()
                    .main_axis_alignment(MainAxisAlignment::End)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(GestureDetector::new().on_click(|| {}).child(panel)),
            ),
    )
}

fn section(icon: &'static str, title: &str, rows: Vec<W>) -> W {
    let mut c = Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(icon).class("set-sec-icon"))
                .child(Text::new(title).class("set-sec-title")),
        );
    for r in rows {
        c = c.child(r);
    }
    Box::new(DecoratedBox::new().child(c).class("set-card"))
}

fn hint(text: &str) -> W {
    Box::new(Text::new(text).max_lines(4).class("set-hint"))
}

fn toggle_row(title: &str, sub: &str, on: bool, f: impl FnMut(bool) + Send + 'static) -> W {
    Box::new(
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(
                Column::new()
                    .gap(2.0)
                    .child(Text::new(title).max_lines(2).class("set-row-title"))
                    .child(Text::new(sub).max_lines(3).class("set-hint"))
                    .class("grow"),
            )
            .child(Toggle::with_state(on).on_change(f)),
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
