//! Листы поверх окна: выбор микрофона, настройки, переименование. На узком окне — снизу,
//! на широком — карточкой по центру.

use syngui::prelude::*;
use syngui::GestureDetector;

use super::{boxed, icon, icon_button, icons, Sheet, St, W};
use crate::store::{self, Format, OPUS_BITRATES};

fn shell(st: St, wide: bool, title: String, body: W, close: impl Fn() + Send + Sync + 'static) -> W {
    let close = std::sync::Arc::new(close);
    let c1 = close.clone();
    let scrim = GestureDetector::new().on_click(move || c1()).child(DecoratedBox::new().class("scrim"));
    let head = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("grow").child(Text::new(title).class("sheet-title")))
        .child(icon_button(icons::CLOSE, t!("Закрыть"), "tb-btn", move || close()));
    let mut col = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    if !wide {
        col = col.child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(DecoratedBox::new().class("grabber")));
    }
    let card = DecoratedBox::new().class(if wide { "sheet card" } else { "sheet bottom" }).child(col.child(head).child(body));
    let _ = st;
    let place = if wide {
        Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).class("sheet-wrap").child(card)
    } else {
        Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Stretch).class("sheet-wrap bottom").child(card)
    };
    boxed(Stack::new().fit(StackFit::Expand).child(scrim).child(place))
}

fn option_row(label: String, sub: Option<String>, on: bool, glyph: &'static str, f: impl FnMut() + Send + 'static) -> W {
    let mut text = Column::new().gap(2.0).child(Text::new(label).max_lines(1).elide(Elide::End).class(if on { "opt-title on" } else { "opt-title" }));
    if let Some(s) = sub {
        text = text.child(Text::new(s).max_lines(2).class("opt-sub"));
    }
    boxed(
        GestureDetector::new().on_click(f).child(
            DecoratedBox::new().class(if on { "opt on" } else { "opt" }).child(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(icon(glyph).class(if on { "ic opt-ic on" } else { "ic opt-ic" }))
                    .child(DecoratedBox::new().class("grow").child(text))
                    .child(icon(if on { icons::RADIO_ON } else { icons::RADIO_OFF }).class(if on { "ic opt-radio on" } else { "ic opt-radio" })),
            ),
        ),
    )
}

fn mics(st: St, wide: bool) -> W {
    let list = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let s = st.settings.get();
        let mics = st.mics.get();
        let has_ns = st.ns.get().is_some();
        let cur = s.mic_node();
        let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        let def = mics.iter().find(|m| m.default).map(|m| m.label.clone());
        col = col.child(option_row(t!("По умолчанию"), def.map(|d| t!("Системный: {d}", d = d)), cur.is_none(), icons::MIC, move || {
            st.set_settings(|s| s.mic.clear());
            st.sheet.set(None);
        }));
        for m in mics {
            let on = cur.as_deref() == Some(m.name.as_str());
            let sub = if has_ns && m.name.starts_with("palaudio.mic.") {
                Some(t!("Без обработки: шумоподавление к нему не применяется"))
            } else if has_ns && m.name == "palaudio.mic" {
                Some(t!("Оба микрофона, с шумоподавлением"))
            } else {
                None
            };
            let name = m.name.clone();
            col = col.child(option_row(m.label.clone(), sub, on, icons::MIC, move || {
                let n = name.clone();
                st.set_settings(|s| s.mic = n);
                st.sheet.set(None);
            }));
        }
        if st.mics.get_untracked().is_empty() {
            col = col.child(Text::new(t!("Список микрофонов недоступен (PipeWire не отвечает) — запись пойдёт с микрофона по умолчанию")).max_lines(3).class("opt-sub pad"));
        }
        vec![boxed(col)]
    });
    shell(st, wide, t!("Микрофон"), boxed(list), move || st.sheet.set(None))
}

fn settings(st: St, wide: bool) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let s = st.settings.get();
        let fmt_idx = Format::ALL.iter().position(|f| *f == s.format).unwrap_or(0);
        let mut col = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        col = col.child(Text::new(t!("Формат")).class("section"));
        col = col.child(
            SegmentedButton::new(Format::ALL.iter().map(|f| f.label().to_string()).collect::<Vec<_>>())
                .selected(fmt_idx)
                .on_change(move |i| {
                    if let Some(f) = Format::ALL.get(i).copied() {
                        st.set_settings(|s| s.format = f);
                    }
                })
                .class("seg"),
        );
        let hint = match s.format {
            Format::Opus => t!("Маленькие файлы, отличный звук для речи. Файл .ogg"),
            Format::Flac => t!("Без потерь, в 2–3 раза меньше WAV. Файл .flac"),
            Format::Wav => t!("Без сжатия: ~5,5 МБ в минуту. Файл .wav"),
        };
        col = col.child(Text::new(hint).max_lines(2).class("opt-sub"));
        if s.format == Format::Opus {
            let idx = OPUS_BITRATES.iter().position(|b| *b == s.opus_kbps).unwrap_or(2);
            col = col.child(Text::new(t!("Качество Opus, кбит/с")).class("section"));
            col = col.child(
                SegmentedButton::new(OPUS_BITRATES.iter().map(|b| b.to_string()).collect::<Vec<_>>())
                    .selected(idx)
                    .on_change(move |i| {
                        if let Some(b) = OPUS_BITRATES.get(i).copied() {
                            st.set_settings(|s| s.opus_kbps = b);
                        }
                    })
                    .class("seg"),
            );
            let mb = s.opus_kbps as f64 * 60.0 / 8.0 / 1024.0;
            col = col.child(Text::new(t!("≈ {mb} МБ в минуту; 32 — речь, 64 — с запасом, 128 — музыка", mb = format!("{mb:.1}").replace('.', ","))).max_lines(2).class("opt-sub"));
        }
        col = col.child(Text::new(t!("Папка записей")).class("section"));
        let folder = use_signal(s.dir().display().to_string());
        col = col.child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(
                    DecoratedBox::new().class("grow").child(
                        TextField::new()
                            .text(s.dir().display().to_string())
                            .on_change(move |t| folder.set(t.to_string()))
                            .on_submit(move |t| {
                                let t = t.to_string();
                                st.set_settings(|s| s.folder = if std::path::Path::new(&t) == store::default_dir() { String::new() } else { t });
                            })
                            .submit_on_focus_lost(true)
                            .class("field"),
                    ),
                )
                .child(icon_button(icons::FOLDER, t!("Открыть папку"), "tb-btn", move || st.open_folder())),
        );
        if !s.folder.is_empty() {
            col = col.child(Button::new(t!("По умолчанию: {d}", d = store::default_dir().display())).class("btn flat small-btn").on_click(move || st.set_settings(|s| s.folder.clear())));
        }
        col = col.child(Text::new(t!("Микрофон")).class("section"));
        col = col.child(
            DecoratedBox::new().class("opt").child(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(
                        DecoratedBox::new().class("grow").child(
                            Column::new()
                                .gap(2.0)
                                .child(Text::new(t!("Уровень до записи")).class("opt-title"))
                                .child(Text::new(t!("Волна реагирует на голос, пока окно активно; через 2 минуты без фокуса микрофон закрывается")).max_lines(3).class("opt-sub")),
                        ),
                    )
                    .child(Toggle::with_state(s.monitor).on_change(move |on| st.set_settings(|s| s.monitor = on))),
            ),
        );
        vec![boxed(col)]
    });
    shell(st, wide, t!("Настройки"), boxed(body), move || st.sheet.set(None))
}

fn rename(st: St, wide: bool) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let Some(path) = st.rename.get() else { return vec![] };
        let Some(r) = st.recordings.get_untracked().into_iter().find(|r| r.path == path) else { return vec![] };
        let text = use_signal(r.title.clone());
        let (p1, p2) = (path.clone(), path.clone());
        let n = r.title.len();
        let body = Column::new()
            .gap(14.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                TextField::new()
                    .text(r.title.clone())
                    .autofocus(true)
                    .select_range(0, n)
                    .on_change(move |t| text.set(t.to_string()))
                    .on_submit(move |t| {
                        st.do_rename(p1.clone(), t.to_string());
                        st.rename.set(None);
                    })
                    .on_escape(move || st.rename.set(None))
                    .class("field"),
            )
            .child(
                Row::new()
                    .gap(8.0)
                    .main_axis_alignment(MainAxisAlignment::End)
                    .child(Button::new(t!("Отмена")).class("btn flat").on_click(move || st.rename.set(None)))
                    .child(Button::new(t!("Сохранить")).icon(icons::CHECK).class("btn primary").on_click(move || {
                        st.do_rename(p2.clone(), text.get_untracked());
                        st.rename.set(None);
                    })),
            );
        vec![shell(st, wide, t!("Название записи"), boxed(body), move || st.rename.set(None))]
    }))
}

pub fn layer(st: St, wide: bool) -> W {
    let sheet = Reactive::new(move || -> Vec<W> {
        match st.sheet.get() {
            None => vec![],
            Some(Sheet::Mics) => vec![mics(st, wide)],
            Some(Sheet::Settings) => vec![settings(st, wide)],
        }
    });
    boxed(Stack::new().fit(StackFit::Expand).child(sheet).child(rename(st, wide)))
}
