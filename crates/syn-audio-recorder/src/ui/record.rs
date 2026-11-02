//! Экран записи: микрофон и шумоподавление, состояние, крупный таймер, живая волна, уровень, кнопки.

use syngui::prelude::*;
use syngui::GestureDetector;

use super::{boxed, icon, icons, tip, wave, MicState, Rec, Sheet, St, W};
use crate::store::{self, Format};

/// Выбор микрофона и шумоподавление.
fn source_card(st: St, wide: bool) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let rec = st.rec.get();
        let _ = st.settings.get();
        let _ = st.mics.get();
        let ns = st.ns.get();
        let mic_state = st.mic.get();
        let locked = rec != Rec::Idle;
        let mic_label = st.mic_label();
        let mic_ic = if matches!(mic_state, MicState::Error(_)) { icons::MIC_OFF } else { icons::MIC };
        let mut mic_row = Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class("src-badge").child(icon(mic_ic).class("ic src-ic")))
            .child(
                DecoratedBox::new().class("grow").child(
                    Column::new()
                        .gap(1.0)
                        .child(Text::new(t!("Микрофон")).class("src-cap"))
                        .child(Text::new(mic_label).max_lines(1).elide(Elide::End).class("src-value")),
                ),
            );
        if !locked {
            mic_row = mic_row.child(icon(icons::EXPAND).class("ic src-chev"));
        }
        let mic_btn: W = if locked {
            tip(DecoratedBox::new().class("src-item locked").child(mic_row), t!("Микрофон меняется, когда запись остановлена"))
        } else {
            boxed(GestureDetector::new().on_click(move || {
                st.refresh_mics();
                st.sheet.set(Some(Sheet::Mics));
            }).child(DecoratedBox::new().class("src-item").child(mic_row)))
        };
        let mut items: Vec<W> = vec![mic_btn];
        if let Some(ns) = ns {
            let applies = st.ns_applies();
            let hint = if !applies {
                t!("Не действует на отдельный микрофон — выберите «Микрофон»")
            } else if ns.on {
                t!("Для речи, во всех программах; музыку — без него")
            } else {
                t!("Системное, для всех программ: гасит постоянный шум")
            };
            let row = Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class(if ns.on && applies { "src-badge on" } else { "src-badge" }).child(icon(icons::NOISE).class("ic src-ic")))
                .child(
                    DecoratedBox::new().class("grow").child(
                        Column::new()
                            .gap(1.0)
                            .child(Text::new(t!("Шумоподавление")).class("src-value"))
                            .child(Text::new(hint).max_lines(2).class(if applies { "src-cap" } else { "src-cap warn" })),
                    ),
                )
                .child(Toggle::with_state(ns.on).on_change(move |on| st.set_ns(on)));
            items.push(boxed(DecoratedBox::new().class("src-item").child(row)));
        }
        let body: W = if wide && items.len() == 2 {
            let mut it = items.into_iter();
            boxed(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Start).child(DecoratedBox::new().class("grow half").child(it.next().unwrap())).child(DecoratedBox::new().class("grow half").child(it.next().unwrap())))
        } else {
            boxed(Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(items))
        };
        vec![body]
    }))
}

/// Состояние, таймер и формат.
fn readout(st: St) -> W {
    let status = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let rec = st.rec.get();
        let mic = st.mic.get();
        let (class, text) = match rec {
            Rec::Recording => ("status rec", t!("Запись")),
            Rec::Paused => ("status paused", t!("Пауза")),
            Rec::Starting => ("status", t!("Открываю микрофон…")),
            Rec::Saving => ("status", t!("Сохраняю…")),
            Rec::Idle => match mic {
                MicState::Error(_) => ("status err", t!("Микрофон недоступен")),
                MicState::Opening => ("status", t!("Открываю микрофон…")),
                _ => ("status", t!("Готов к записи")),
            },
        };
        vec![boxed(
            DecoratedBox::new().class(class).child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("status-dot")).child(Text::new(text).class("status-text"))),
        )]
    });
    let timer = Reactive::new(move || -> Vec<W> {
        let secs = st.tenths.get() as f64 / 10.0;
        let rec = st.rec.get();
        let (hms, frac) = store::timer(secs);
        let cls = if rec.active() { "timer on" } else { "timer" };
        vec![boxed(Row::new().cross_axis_alignment(CrossAxisAlignment::End).child(Text::new(hms).class(cls)).child(Text::new(frac).class(&format!("{cls} frac"))))]
    });
    let info = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let s = st.settings.get();
        let f = match s.format {
            Format::Opus => t!("Opus · {k} кбит/с", k = s.opus_kbps),
            Format::Flac => t!("FLAC · без потерь"),
            Format::Wav => t!("WAV · без сжатия"),
        };
        vec![boxed(Text::new(f).class("fmt"))]
    });
    boxed(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(status).child(timer).child(info))
}

/// Волна, уровень и перегрузка.
fn visual(st: St, wide: bool) -> W {
    let canvas = Reactive::new(move || -> Vec<W> {
        let rec = st.rec.get();
        let mic = st.mic.get();
        let animated = mic == MicState::Open || rec.active();
        vec![boxed(wave::live(animated).class(if wide { "vis" } else { "vis phone" }))]
    });
    let clip = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let on = st.clip.get();
        vec![boxed(DecoratedBox::new().class(if on { "clip on" } else { "clip" }).child(Text::new(t!("Перегрузка")).class("clip-text")))]
    });
    let meter_row = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("grow").child(wave::meter().class("meter"))).child(clip);
    boxed(DecoratedBox::new().class("vis-card").child(Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(canvas).child(meter_row)))
}

/// Отменить — Запись/Стоп — Пауза.
fn controls(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let rec = st.rec.get();
        let active = rec.active();
        let side = |glyph: &'static str, label: String, enabled: bool, f: Box<dyn FnMut() + Send>| -> W {
            let f = std::sync::Mutex::new(f);
            let b = Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Button::new("").icon(glyph).disabled(!enabled).class("btn side").on_click(move || (f.lock().unwrap())()))
                .child(Text::new(label).class(if enabled { "side-label" } else { "side-label dim" }));
            boxed(DecoratedBox::new().class(if enabled { "side-wrap" } else { "side-wrap off" }).child(b))
        };
        let cancel = side(icons::CLOSE, t!("Отменить"), active, Box::new(move || st.cancel()));
        let pause = if rec == Rec::Paused {
            side(icons::MIC, t!("Продолжить"), true, Box::new(move || st.toggle_pause()))
        } else {
            side(icons::PAUSE, t!("Пауза"), active, Box::new(move || st.toggle_pause()))
        };
        let busy = matches!(rec, Rec::Starting | Rec::Saving);
        let core_class = if active { "rec-core stop" } else if busy { "rec-core busy" } else { "rec-core" };
        let ring_class = if rec == Rec::Recording { "rec-ring live" } else { "rec-ring" };
        let main = GestureDetector::new()
            .on_click(move || match st.rec.get_untracked() {
                Rec::Idle => st.record(),
                Rec::Recording | Rec::Paused => st.stop(),
                _ => {}
            })
            .child(DecoratedBox::new().class(ring_class).child(DecoratedBox::new().class(core_class)));
        let main = tip(main, if active { t!("Остановить и сохранить (Enter)") } else { t!("Начать запись (Пробел)") });
        vec![boxed(Row::new().gap(28.0).main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(cancel).child(main).child(pause))]
    }))
}

pub fn page(st: St, wide: bool) -> W {
    let col = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(source_card(st, wide))
        .child(DecoratedBox::new().class("grow min-gap"))
        .child(readout(st))
        .child(DecoratedBox::new().class("gap-24"))
        .child(visual(st, wide))
        .child(DecoratedBox::new().class("grow min-gap"))
        .child(controls(st));
    let col = if wide { col.child(Text::new(t!("Пробел — запись и пауза · Enter — сохранить")).class("keys-hint")) } else { col };
    let inner = DecoratedBox::new().class(if wide { "rec-page" } else { "rec-page phone" }).child(col);
    boxed(Row::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow").child(DecoratedBox::new().class("grow rec-max").child(inner)))
}
