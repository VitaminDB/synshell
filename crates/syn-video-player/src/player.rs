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
use crate::ui::{back, ended, go, icons, reopen, step, Card, St};

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
        let (fill, seek_step) = st.view_key.get();
        let full = st.fullscreen.get();
        let fs = FullscreenCtl {
            active: full,
            toggle: Arc::new(move || {
                let on = !st.fullscreen.get_untracked();
                st.fullscreen.set(on);
                syngui::signal::set_fullscreen(on);
            }),
        };
        let done = p.clone();
        let title = p.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut view = VideoPlayerView::file(p.player.clone())
            .fullscreen(fs)
            .fit(if fill { ImageFit::Cover } else { ImageFit::Contain })
            .seek_step(seek_step as f64)
            .subtitles(st.sub_style)
            .keys(crate::subs::KEYS.iter().copied().chain([crate::filters::COMPARE_KEY]).collect(), move |k, m| {
                crate::subs::on_key(st, k, m) || crate::filters::on_key(st, k, m)
            })
            .header(move || header(st, title.clone()))
            .on_ended(move || ended(st, &done));
        // в каталоге есть другие ролики — листание, как в просмотрщике картинок
        if st.siblings.with(|v| v.len() > 1) {
            let path = p.path.clone();
            view = view.on_prev(move || step(st, -1)).on_next(move || step(st, 1)).footer(move || filmstrip(st, &path));
        }
        vec![Box::new(view.build())]
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
            .child(crate::filters::compare_handle(st))
            .child(panel)
            .child(crate::tracks::panel(st))
            .child(crate::subs::panel(st))
            .child(crate::filters::panel(st))
            .child(crate::vram::dialog(st)),
    )
}

fn header(st: St, title: String) -> W {
    let chip = Reactive::new(move || -> Vec<W> {
        let dlss = dlss_chip(&st.dlss.get());
        match (st.settings.get().show_info, st.info.get()) {
            (true, Some(i)) if !i.codec.is_empty() => vec![info_chip(&i, dlss)],
            // способ воспроизведения скрыт — DLSS 5 всё равно видно, пока он включён
            _ => dlss.map(|d| -> W { Box::new(DecoratedBox::new().child(d).class("info-chip info-chip-dlss")) }).into_iter().collect(),
        }
    });
    let top = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            ToolButton::new(icons::ARROW_BACK)
                .tooltip(t!("Назад"))
                .on_click(move || {
                    back(st);
                })
                .class("vp-btn"),
        )
        .child(Text::new(title).max_lines(1).class("player-title"))
        .child(DecoratedBox::new().class("grow"))
        .child(crate::filters::button(st))
        .child(crate::subs::button(st))
        .child(crate::tracks::button(st))
        .child(
            ToolButton::new(icons::CUT)
                .tooltip(t!("Монтаж: резать и склеивать"))
                .on_click(move || crate::editor::start(st))
                .class("vp-btn"),
        )
        .child(
            ToolButton::new(icons::SETTINGS)
                .tooltip(t!("Настройки"))
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
fn info_chip(i: &DecodeInfo, dlss: Option<W>) -> W {
    let res = resolution_label(i.width, i.height);
    let what = match &i.audio_codec {
        Some(a) => format!("{} · {res} · {a}", i.codec),
        None => format!("{} · {res}", i.codec),
    };
    let (icon, mode, class) = match (&i.hardware, i.known) {
        (_, false) => (icons::HOURGLASS, t!("Определяю…").to_string(), "chip-mode chip-mode-wait"),
        (Some(hw), true) if i.zero_copy => (icons::MEMORY, t!("Аппаратно · {hw} · без копий", hw = hw), "chip-mode chip-mode-hw"),
        (Some(hw), true) => (icons::MEMORY, t!("Аппаратно · {hw}", hw = hw), "chip-mode chip-mode-hw"),
        (None, true) => (icons::CPU, t!("Программно").to_string(), "chip-mode chip-mode-sw"),
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
                    )
                    .children(dlss),
            )
            .class("info-chip"),
    )
}

/// «✦ DLSS 5 · 8 мс» — зелёный, пока кадры идут через модель; серый — запуск; янтарный — кадры как
/// есть (причина — во всплывающей подсказке). Выключен — чипа нет.
fn dlss_chip(s: &crate::dlss::Status) -> Option<W> {
    use crate::dlss::Status;
    let (text, class, tip) = match s {
        Status::Off => return None,
        Status::Starting => (t!("DLSS 5 · запуск…").to_string(), "chip-mode chip-mode-wait", t!("Помощник DLSS 5 готовит модель").to_string()),
        Status::Active { total_ms, fps, size, scale: Some(sc), .. } => (
            t!("DLSS 5 · {ms} мс · {p}%", ms = format!("{total_ms:.0}"), p = format!("{:.0}", sc * 100.0)),
            "chip-mode chip-mode-dlss",
            t!("Кадры {w}×{h} идут через DLSS 5 на {p}% разрешения модели, чтобы успевать: {ms} мс на кадр, {fps} кадр/с", w = size.0, h = size.1, p = format!("{:.0}", sc * 100.0), ms = format!("{total_ms:.1}"), fps = format!("{fps:.0}")),
        ),
        Status::Active { total_ms, fps, size, .. } => (
            t!("DLSS 5 · {ms} мс", ms = format!("{total_ms:.0}")),
            "chip-mode chip-mode-dlss",
            t!("Кадры {w}×{h} идут через DLSS 5: {ms} мс на кадр, {fps} кадр/с", w = size.0, h = size.1, ms = format!("{total_ms:.1}"), fps = format!("{fps:.0}")),
        ),
        Status::Slow { total_ms, budget_ms } => (
            t!("DLSS 5 · не успевает").to_string(),
            "chip-mode chip-mode-sw",
            t!("Модели нужно {a} мс, а кадр ролика — {b} мс: кадры идут как есть, повтор — после перемотки", a = format!("{total_ms:.0}"), b = format!("{budget_ms:.0}")).to_string(),
        ),
        Status::Bypass(e) => (t!("DLSS 5 · как есть").to_string(), "chip-mode chip-mode-sw", crate::dlss::reason(e)),
    };
    let chip = DecoratedBox::new()
        .child(
            Row::new()
                .gap(5.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(icons::DLSS).class("chip-mode-icon"))
                .child(Text::new(text).max_lines(1).class("chip-mode-text")),
        )
        .class(class);
    Some(Box::new(Tooltip::new(chip, &tip)))
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

// ─── лента миниатюр ─────────────────────────────────────────────────────────

/// Ширина миниатюры ленты: на узком окне меньше.
const STRIP_THUMB: f32 = 96.0;
const STRIP_THUMB_NARROW: f32 = 64.0;
const STRIP_GAP: f32 = 6.0;
/// Больше миниатюр разом не показывать, даже на широком экране.
const STRIP_MAX: usize = 11;

/// Окно ленты: `window` миниатюр вокруг текущей, текущая — посередине.
fn strip_range(index: usize, total: usize, window: usize) -> std::ops::Range<usize> {
    if total <= window {
        return 0..total;
    }
    let start = index.saturating_sub(window / 2).min(total - window);
    start..start + window
}

/// Сколько миниатюр шириной `thumb` помещается в `width` (нечётное — текущая по центру).
fn strip_window(width: f32, thumb: f32) -> usize {
    let n = (((width + STRIP_GAP) / (thumb + STRIP_GAP)).floor() as usize).clamp(1, STRIP_MAX);
    if n % 2 == 0 { n - 1 } else { n }
}

/// Лента над шкалой перемотки: соседние ролики каталога, текущий — в рамке акцента.
fn filmstrip(st: St, current: &std::path::Path) -> W {
    let list = st.siblings.get_untracked();
    let Some(index) = list.iter().position(|c| c.item.path == current) else {
        return Box::new(Row::new());
    };
    let width = viewport_size().get().width;
    let narrow = width < 560.0;
    let thumb = if narrow { STRIP_THUMB_NARROW } else { STRIP_THUMB };
    // поля панели `.vp-controls` — по 18 px
    let range = strip_range(index, list.len(), strip_window(width - 36.0, thumb));
    let thumbs: Vec<W> = range.map(|i| strip_thumb(st, &list[i], i == index, narrow)).collect();
    Box::new(
        Row::new()
            .gap(STRIP_GAP)
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(thumbs)
            .class("strip"),
    )
}

fn strip_thumb(st: St, c: &Card, active: bool, narrow: bool) -> W {
    let pic = c.pic;
    let key = format!("svp:{}", c.item.path.display());
    let picture = Reactive::new(move || -> Vec<W> {
        let mut layers = Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().class("strip-bg"));
        layers = match pic.get() {
            Some(p) => layers.child(Image::from_rgba_shared(key.clone(), p.w, p.h, p.rgba.clone()).fit(ImageFit::Cover).class("pic")),
            None => layers.child(
                Column::new()
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(icons::MOVIE).class("strip-icon")),
            ),
        };
        vec![Box::new(layers)]
    });
    let mut class = String::from(if narrow { "strip-thumb strip-thumb-sm" } else { "strip-thumb" });
    if active {
        class.push_str(" strip-thumb-active");
    }
    let path = c.item.path.clone();
    let thumb = DecoratedBox::new().child(picture).class(class);
    let thumb: W = if narrow { Box::new(thumb) } else { Box::new(Tooltip::new(thumb, &c.item.name())) };
    Box::new(
        GestureDetector::new()
            .on_click(move || go(st, path.clone()))
            .cursor(syngui::CursorIcon::Pointer)
            .child(thumb),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_keeps_current_in_window() {
        assert_eq!(strip_range(0, 3, 11), 0..3);
        assert_eq!(strip_range(20, 40, 11), 15..26);
        assert_eq!(strip_range(39, 40, 11), 29..40);
        assert_eq!(strip_range(1, 40, 5), 0..5);
    }

    #[test]
    fn strip_window_is_odd_and_fits() {
        // телефон 360 px: поля по 18 — 324 px, по 64 + 6 — четыре, нечётно — три
        assert_eq!(strip_window(324.0, STRIP_THUMB_NARROW), 3);
        assert_eq!(strip_window(1500.0, STRIP_THUMB), STRIP_MAX);
        assert_eq!(strip_window(10.0, STRIP_THUMB), 1);
    }
}

// ─── настройки ──────────────────────────────────────────────────────────────

pub fn update(st: St, f: impl FnOnce(&mut Settings)) {
    let mut s = st.settings.get_untracked();
    f(&mut s);
    if s != st.settings.get_untracked() {
        s.save();
        let key = (s.fill, s.seek_step);
        st.settings.set(s);
        if st.view_key.get_untracked() != key {
            st.view_key.set(key);
        }
    }
}

fn settings_panel(st: St) -> W {
    let s = st.settings.get_untracked();
    let close = move || st.settings_open.set(false);

    let decoding = section(
        icons::MEMORY,
        &t!("Декодирование"),
        vec![
            Box::new(
                SegmentedButton::new(vec![
                    Segment::with_icon(t!("Аппаратно"), icons::MEMORY),
                    Segment::with_icon(t!("Программно"), icons::CPU),
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
                            t!("Сейчас: {codec} через {hw}, кадры идут на экран прямо из памяти декодера.", codec = i.codec, hw = hw)
                        }
                        Some(hw) => t!("Сейчас: {codec} через {hw}.", codec = i.codec, hw = hw),
                        None => t!("Сейчас: {codec} декодирует процессор.", codec = i.codec),
                    },
                    _ => t!("Аппаратный кодек экономит процессор и батарею; если с ним что-то не так — включите программный.").into(),
                };
                vec![Box::new(Text::new(text).max_lines(4).class("set-hint"))]
            })),
        ],
    );

    let dlss = filters_section(st);

    let picture = section(
        icons::ASPECT,
        &t!("Изображение"),
        vec![
            Box::new(
                SegmentedButton::new(vec![
                    Segment::with_icon(t!("Вписать"), icons::FIT),
                    Segment::with_icon(t!("Заполнить"), icons::FILL),
                ])
                .selected(if s.fill { 1 } else { 0 })
                .on_change(move |i| update(st, |s| s.fill = i == 1)),
            ),
            hint(&t!("«Заполнить» — без чёрных полос, края кадра обрезаются.")),
        ],
    );

    let seek = section(
        icons::FAST_FORWARD,
        &t!("Перемотка"),
        vec![
            Box::new(
                SegmentedButton::new(SEEK_STEPS.iter().map(|s| t!("{s} с", s = s)).collect::<Vec<_>>())
                    .selected(SEEK_STEPS.iter().position(|v| *v == s.seek_step).unwrap_or(1))
                    .on_change(move |i| update(st, |s| s.seek_step = SEEK_STEPS[i.min(SEEK_STEPS.len() - 1)])),
            ),
            hint(&t!("Двойной тап по левому или правому краю кадра, кнопки и клавиши J / L.")),
        ],
    );

    let playback = section(
        icons::PLAY_CIRCLE,
        &t!("Воспроизведение"),
        vec![
            toggle_row(&t!("Продолжать с места остановки"), &t!("Ролик откроется там, где вы его закрыли."), s.resume, move |on| {
                update(st, |s| s.resume = on)
            }),
            toggle_row(&t!("Повторять ролик"), &t!("Дошёл до конца — сначала."), s.repeat, move |on| update(st, |s| s.repeat = on)),
            toggle_row(
                &t!("Воспроизводить следующий"),
                &t!("Дошёл до конца — следующий ролик из той же папки. Если включён повтор — повтор."),
                s.autoplay_next,
                move |on| update(st, |s| s.autoplay_next = on),
            ),
        ],
    );

    let subtitles = section(
        icons::SUBTITLES,
        &t!("Субтитры"),
        vec![
            Box::new(
                Button::new(t!("Дорожки и оформление…"))
                    .icon(icons::PALETTE)
                    .on_click(move || {
                        st.settings_open.set(false);
                        st.subs_open.set(true);
                    }),
            ),
            hint(&t!("Шрифт, размер, цвет, контур, тень, анимация; язык и автовключение. Клавиши: C — вкл/выкл, Z/X — раньше/позже.")),
        ],
    );

    let view = section(
        icons::INFO,
        &t!("Интерфейс"),
        vec![toggle_row(
            &t!("Показывать способ воспроизведения"),
            &t!("Чип вверху: кодек, размер, аппаратно или программно."),
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
                .child(Text::new(t!("Настройки")).class("set-title"))
                .child(DecoratedBox::new().class("grow"))
                .child(ToolButton::new(icons::CLOSE).tooltip(t!("Закрыть")).on_click(close).class("set-close"))
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
                        .child(dlss)
                        .child(picture)
                        .child(seek)
                        .child(playback)
                        .child(subtitles)
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

/// DLSS 5 и нейросети — в панели «Фильтры»; здесь ссылка на неё.
fn filters_section(st: St) -> W {
    section(
        icons::TUNE,
        &t!("Фильтры"),
        vec![
            Box::new(Button::new(t!("DLSS 5 и нейросети…")).icon(icons::DLSS).on_click(move || {
                st.settings_open.set(false);
                st.filters_open.set(true);
            })),
            hint(&t!("Улучшение кадров DLSS 5, сравнение с исходным и нагрузка — в панели фильтров (кнопка в шапке плеера).")),
        ],
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
