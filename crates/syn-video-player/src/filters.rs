//! Панель «Фильтры»: всё, что меняет картинку ролика нейросетями, — карточками в порядке обработки
//! (сейчас — «Улучшение · DLSS 5»), сравнение с исходным кадром (черту можно тянуть по кадру) и
//! нагрузка: сколько из времени кадра ролика уходит на фильтры.

use syngui::prelude::*;
use syngui::buttons::Segment;
use syngui::widgets::*;
use syngui::containers::Positioned;

use crate::dlss::Status;
use crate::settings::Settings;
use crate::ui::{icons, St};

type W = Box<dyn Widget>;

/// Клавиша сравнения с исходным кадром.
pub const COMPARE_KEY: Key = Key::B;

/// Стили модели DLSS 5 (`set_style`).
fn styles() -> Vec<String> {
    vec![t!("Обычный").into(), t!("Естественный").into(), t!("Кино").into()]
}

/// Качество DLSS 5 — разрешение модели (`set_model_resolution`): быстро, баланс, максимум.
const QUALITY: [f32; 3] = [0.5, 0.75, 1.0];

fn quality_index(scale: f32) -> usize {
    QUALITY.iter().enumerate().min_by(|a, b| (a.1 - scale).abs().total_cmp(&(b.1 - scale).abs())).map(|(i, _)| i).unwrap_or(2)
}

/// Кнопка в шапке: светится, пока включён хоть один фильтр.
pub fn button(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let on = st.settings.with(|s| s.dlss || s.sr);
        vec![Box::new(
            ToolButton::new(icons::TUNE)
                .tooltip(t!("Фильтры: DLSS 5 и нейросети"))
                .on_click(move || st.filters_open.set(true))
                .class(if on { "vp-btn flt-btn-on" } else { "vp-btn" }),
        )]
    }))
}

/// Изменить настройки плеера, касающиеся фильтров, и применить к открытому ролику.
fn set(st: St, f: impl FnOnce(&mut Settings)) {
    crate::player::update(st, f);
    crate::dlss::apply(st);
}

/// Сменилась сила или черта: в фильтр сразу, без перестройки плеера.
fn set_live(st: St, f: impl FnOnce(&mut Settings)) {
    crate::player::update(st, f);
    crate::dlss::set_opts(st);
    redraw_paused(st);
}

/// На паузе кадр не меняется сам — показать его заново через фильтр.
fn redraw_paused(st: St) {
    let Some(p) = st.playing.get_untracked() else { return };
    let Ok(mut p) = p.player.lock() else { return };
    if p.is_paused() {
        let pos = p.position_sec();
        let _ = p.seek(pos);
    }
}

/// B — сравнение вкл/выкл.
pub fn on_key(st: St, key: Key, mods: Modifiers) -> bool {
    if key != COMPARE_KEY || mods.ctrl || mods.alt || mods.meta {
        return false;
    }
    if !st.settings.with_untracked(|s| s.dlss) {
        return false;
    }
    let on = !st.settings.with_untracked(|s| s.dlss_compare);
    set_live(st, |s| s.dlss_compare = on);
    st.filters_rev.update(|r| *r += 1);
    true
}

pub fn panel(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        if !st.filters_open.get() {
            return vec![];
        }
        st.filters_rev.get();
        let close = move || st.filters_open.set(false);
        let body = Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(load_card(st))
            .child(dlss_card(st))
            .child(sr_card(st))
            .child(compare_card(st))
            .class("set-body");
        let panel = Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(t!("Фильтры")).class("set-title"))
                    .child(DecoratedBox::new().class("grow"))
                    .child(ToolButton::new(icons::CLOSE).tooltip(t!("Закрыть")).on_click(close).class("set-close"))
                    .class("set-head"),
            )
            .child(ScrollView::new().vertical().child(body).class("grow"))
            .class("set-panel flt-panel");
        // затемнение лёгкое: картинку с фильтрами видно, пока их настраиваешь
        vec![Box::new(
            Stack::new()
                .fit(StackFit::Expand)
                .child(GestureDetector::new().on_click(close).child(DecoratedBox::new().class("sub-scrim")))
                .child(
                    Row::new()
                        .main_axis_alignment(MainAxisAlignment::End)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(GestureDetector::new().on_click(|| {}).child(panel)),
                ),
        )]
    }))
}

fn card(icon: &'static str, title: &str, toggle: Option<W>, rows: Vec<W>) -> W {
    let mut head = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Icon::new(icon).class("set-sec-icon"))
        .child(Text::new(title).class("set-sec-title grow"));
    if let Some(t) = toggle {
        head = head.child(t);
    }
    let mut c = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head);
    for r in rows {
        c = c.child(r);
    }
    Box::new(DecoratedBox::new().child(c).class("set-card"))
}

fn hint(text: &str) -> W {
    Box::new(Text::new(text).max_lines(5).class("set-hint"))
}

fn label(text: &str) -> W {
    Box::new(Text::new(text).class("set-row-title"))
}

fn ms(v: f32) -> String {
    format!("{v:.1}").replace('.', ",")
}

/// Нагрузка: время фильтров против времени кадра ролика.
fn load_card(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let (text, frac, warn) = match st.dlss.get() {
            Status::Off if st.sr.get() == crate::neural::Status::Off => (t!("Фильтры выключены — кадры идут как есть.").to_string(), None, false),
            Status::Off => (String::new(), None, false),
            Status::Starting => (t!("Готовлю модели…").to_string(), None, false),
            Status::Active { total_ms, budget_ms, scale, .. } if budget_ms > 0.0 => {
                let mut t = t!("Обработка кадра: {a} мс из {b} мс.", a = ms(total_ms), b = ms(budget_ms)).to_string();
                if let Some(s) = scale {
                    t.push(' ');
                    t.push_str(&t!("Чтобы успевать, DLSS 5 работает на {p}% разрешения.", p = format!("{:.0}", s * 100.0)));
                }
                (t, Some(total_ms / budget_ms), scale.is_some())
            }
            Status::Active { total_ms, .. } => (t!("Обработка кадра: {a} мс.", a = ms(total_ms)).to_string(), None, false),
            Status::Slow { total_ms, budget_ms } => (
                t!("Не успеваем: {a} мс при кадре {b} мс. Кадры идут как есть, повтор — после перемотки.", a = ms(total_ms), b = ms(budget_ms)).to_string(),
                Some(1.0),
                true,
            ),
            Status::Bypass(e) => (crate::dlss::reason(&e), None, true),
        };
        let (text, frac, warn) = match st.sr.get() {
            crate::neural::Status::Active { ms: sr_ms, budget_ms, .. } if budget_ms > 0.0 => {
                let dl = match st.dlss.get() {
                    Status::Active { total_ms, .. } => total_ms,
                    _ => 0.0,
                };
                let total = dl + sr_ms;
                let t = if dl > 0.0 {
                    t!("Обработка кадра: {a} мс из {b} мс (DLSS 5 — {d}, увеличение — {s}).", a = ms(total), b = ms(budget_ms), d = ms(dl), s = ms(sr_ms)).to_string()
                } else {
                    t!("Обработка кадра: {a} мс из {b} мс.", a = ms(sr_ms), b = ms(budget_ms)).to_string()
                };
                (t, Some(total / budget_ms), warn || total > budget_ms * 0.9)
            }
            _ => (text, frac, warn),
        };
        let mut rows: Vec<W> = Vec::new();
        if let Some(f) = frac {
            rows.push(Box::new(ProgressBar::new().value(f.clamp(0.0, 1.0)).class(if warn { "flt-load flt-load-warn" } else { "flt-load" })));
        }
        if !text.is_empty() {
            rows.push(Box::new(Text::new(text).max_lines(4).class(if warn { "set-hint flt-warn" } else { "set-hint" })));
        }
        vec![card(icons::SPEED, &t!("Нагрузка"), None, rows)]
    }))
}

fn dlss_card(st: St) -> W {
    let s = st.settings.get_untracked();
    let toggle = Toggle::with_state(s.dlss).on_change(move |on| {
        set(st, |s| s.dlss = on);
        st.filters_rev.update(|r| *r += 1);
    });
    let mut rows: Vec<W> = vec![hint(&t!("Нейрорендеринг NVIDIA (RTX 50): свет, тон и детали каждого кадра. Обучен на играх — для фильмов лучше стиль «Естественный»."))];
    if s.dlss {
        let style = crate::dlss::profile_value("set_style").map(|v| v as usize).unwrap_or(1).min(2);
        rows.push(label(&t!("Стиль")));
        rows.push(Box::new(SegmentedButton::new(styles()).selected(style).on_change(move |i| {
            crate::dlss::set_profile(&[("set_style", i.min(2).to_string())], true);
            redraw_paused(st);
        })));

        let q = quality_index(crate::dlss::profile_value("set_model_resolution").unwrap_or(1.0));
        rows.push(label(&t!("Качество")));
        rows.push(Box::new(
            SegmentedButton::new(vec![Segment::new(t!("Быстро")), Segment::new(t!("Баланс")), Segment::new(t!("Максимум"))])
                .selected(q)
                .on_change(move |i| {
                    crate::dlss::set_profile(&[("set_model_resolution", QUALITY[i.min(2)].to_string())], false);
                    redraw_paused(st);
                }),
        ));
        rows.push(hint(&t!("Разрешение модели: 50, 75 или 100% кадра. Не успевает за роликом — плеер сам снижает его на время, без мигания.")));

        let strength = use_signal(s.dlss_strength.min(100));
        rows.push(Box::new(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(t!("Сила")).class("set-row-title grow"))
                .child(Reactive::new(move || -> Vec<W> { vec![Box::new(Text::new(format!("{} %", strength.get())).class("sub-value"))] })),
        ));
        rows.push(Box::new(
            Slider::new()
                .range(0.0, 100.0)
                .step(5.0)
                .value(s.dlss_strength.min(100) as f32)
                .on_change(move |v| {
                    let v = v.round().clamp(0.0, 100.0) as u32;
                    strength.set(v);
                    set_live(st, |s| s.dlss_strength = v);
                })
                .class("sub-slider"),
        ));
        rows.push(Box::new(Reactive::new(move || -> Vec<W> {
            let text = match st.dlss.get() {
                Status::Active { total_ms, helper_ms, size, fps, .. } => t!(
                    "Работает: {w}×{h}, {ms} мс на кадр (модель и оптический поток — {hms} мс), {fps} кадр/с.",
                    w = size.0,
                    h = size.1,
                    ms = ms(total_ms),
                    hms = ms(helper_ms),
                    fps = format!("{fps:.0}")
                )
                .to_string(),
                Status::Starting => t!("Помощник DLSS 5 готовит модель…").to_string(),
                _ => return vec![],
            };
            vec![Box::new(Text::new(text).max_lines(4).class("set-hint set-dlss-state"))]
        })));
        rows.push(Box::new(Row::new().child(
            Button::new(t!("Тонкая настройка…"))
                .icon(icons::OPEN_IN_NEW)
                .on_click(|| {
                    if let Err(e) = std::process::Command::new("syn-dlss").spawn() {
                        tracing::warn!("syn-dlss: {e}");
                    }
                })
                .class("set-dlss-open"),
        )));
        rows.push(hint(&t!("У видео свой профиль DLSS 5: что меняете здесь и в окне DLSS 5, пока идёт ролик, не трогает настройки игр.")));
    }
    card(icons::DLSS, &t!("Улучшение · DLSS 5"), Some(Box::new(toggle)), rows)
}

/// «Чёткость»: увеличение разрешения нейросетью synaptix (модели в папке vfx библиотеки).
fn sr_card(st: St) -> W {
    use crate::neural::Status as Sr;
    let s = st.settings.get_untracked();
    let toggle = Toggle::with_state(s.sr).on_change(move |on| {
        crate::player::update(st, |s| s.sr = on);
        crate::neural::apply(st);
        st.filters_rev.update(|r| *r += 1);
    });
    let mut rows: Vec<W> = vec![hint(&t!("Нейросеть дорисовывает детали маленьким роликам: 480p становится чётким на большом экране. Свои ядра synaptix на видеокарте NVIDIA."))];
    if s.sr {
        let list = crate::neural::models();
        if list.is_empty() {
            rows.push(hint(&t!("Модели не установлены: положите файлы .syn в папку vfx библиотеки нейросетей.")));
        } else {
            let cur = crate::neural::pick(&s.sr_model).map(|m| m.id).unwrap_or_default();
            rows.push(label(&t!("Модель")));
            for m in list {
                let active = m.id == cur;
                let id = m.id.clone();
                let sub = model_hint(&m);
                rows.push(Box::new(
                    GestureDetector::new()
                        .on_click(move || {
                            let id = id.clone();
                            crate::player::update(st, |s| s.sr_model = id);
                            crate::neural::apply(st);
                            st.filters_rev.update(|r| *r += 1);
                        })
                        .cursor(syngui::CursorIcon::Pointer)
                        .child(
                            DecoratedBox::new()
                                .child(
                                    Row::new()
                                        .gap(12.0)
                                        .cross_axis_alignment(CrossAxisAlignment::Center)
                                        .child(Icon::new(if active { icons::RADIO_ON } else { icons::RADIO_OFF }).class("track-radio"))
                                        .child(Column::new().gap(2.0).child(Text::new(m.title()).class("set-row-title")).child(Text::new(sub).max_lines(2).class("set-hint")).class("grow")),
                                )
                                .class(if active { "track-row track-row-active" } else { "track-row" }),
                        ),
                ));
            }
        }
        rows.push(Box::new(Reactive::new(move || -> Vec<W> {
            let (text, warn) = match st.sr.get() {
                Sr::Off => return vec![],
                Sr::Loading => (t!("Готовлю модель…").to_string(), false),
                Sr::Active { ms: m, from, to, .. } => (t!("Работает: {a}×{b} → {c}×{d}, {ms} мс на кадр.", a = from.0, b = from.1, c = to.0, d = to.1, ms = ms(m)).to_string(), false),
                Sr::Skipped(h) => (t!("Ролик {h}p — увеличивать некуда: вышло бы больше 4K.", h = h).to_string(), false),
                Sr::Slow { ms: m, budget_ms } => (t!("Не успевает: {a} мс при кадре {b} мс — кадры как есть до перемотки. Выберите модель полегче.", a = ms(m), b = ms(budget_ms)).to_string(), true),
                Sr::Failed(e) => (e, true),
            };
            vec![Box::new(Text::new(text).max_lines(4).class(if warn { "set-hint flt-warn" } else { "set-hint set-dlss-state" }))]
        })));
    }
    card(icons::SR, &t!("Чёткость · увеличение"), Some(Box::new(toggle)), rows)
}

/// Подпись модели: для чего и насколько тяжёлая.
fn model_hint(m: &crate::neural::Model) -> String {
    match (m.arch.as_str(), m.rank) {
        ("nanovsr", 0) => t!("Видео, самая лёгкая: для слабых карт").into(),
        ("nanovsr", 1) => t!("Видео, баланс скорости и качества").into(),
        ("nanovsr", 2) => t!("Видео, качественнее, нужна мощная карта").into(),
        ("nanovsr", _) => t!("Видео, максимум качества, только для маленьких роликов").into(),
        _ => String::new(),
    }
}

fn compare_card(st: St) -> W {
    let s = st.settings.get_untracked();
    if !s.dlss {
        return Box::new(Row::new());
    }
    let toggle = Toggle::with_state(s.dlss_compare).on_change(move |on| {
        set_live(st, |s| s.dlss_compare = on);
        st.filters_rev.update(|r| *r += 1);
    });
    let mut rows = vec![hint(&t!("Слева от черты — исходный кадр, справа — с фильтрами. Черту можно тянуть прямо по кадру. Клавиша B."))];
    if s.dlss_compare {
        rows.push(Box::new(
            Slider::new()
                .range(0.0, 100.0)
                .step(1.0)
                .value(s.dlss_split * 100.0)
                .on_change(move |v| {
                    st.split.set(v / 100.0);
                    set_live(st, |s| s.dlss_split = (v / 100.0).clamp(0.0, 1.0));
                })
                .class("sub-slider"),
        ));
    }
    card(icons::COMPARE, &t!("Сравнение"), Some(Box::new(toggle)), rows)
}

// ─── черта сравнения поверх кадра ───────────────────────────────────────────

/// Ширина ручки черты (область захвата), px.
const HANDLE: f32 = 36.0;

/// Прямоугольник кадра `fw×fh` в окне `vw×vh` (вписан или заполняет): x, ширина.
fn frame_x(vw: f32, vh: f32, fw: f32, fh: f32, fill: bool) -> (f32, f32) {
    if fw <= 0.0 || fh <= 0.0 || vw <= 0.0 || vh <= 0.0 {
        return (0.0, vw);
    }
    let k = if fill { (vw / fw).max(vh / fh) } else { (vw / fw).min(vh / fh) };
    let w = fw * k;
    ((vw - w) / 2.0, w)
}

/// Ручка черты сравнения: узкая полоса во всю высоту там, где черта на кадре; тянется мышью и
/// пальцем. Пока сравнение выключено или кадры не через модель — её нет.
pub fn compare_handle(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let (on, fill) = st.settings.with(|s| (s.dlss && s.dlss_compare, s.fill));
        if !on || !matches!(st.dlss.get(), Status::Active { .. }) {
            return vec![];
        }
        let Some(info) = st.info.get() else { return vec![] };
        let vp = viewport_size().get();
        let (x0, w) = frame_x(vp.width, vp.height, info.width as f32, info.height as f32, fill);
        let split = st.split.get_untracked();
        let offset = use_signal(Point::new(x0 + split * w - HANDLE / 2.0, 0.0));
        let start = use_signal(split);
        let grip = GestureDetector::new()
            .cursor(syngui::CursorIcon::ColResize)
            .on_pan_start(move |_| start.set(st.split.get_untracked()))
            .on_pan_update(move |p| {
                if w <= 0.0 {
                    return;
                }
                let v = (start.get_untracked() + p.total.x / w).clamp(0.0, 1.0);
                st.split.set(v);
                offset.set(Point::new(x0 + v * w - HANDLE / 2.0, 0.0));
                // в фильтр сразу; в настройки — по отпусканию (их смена перестроила бы ручку посреди жеста)
                crate::dlss::set_opts(st);
                redraw_paused(st);
            })
            .on_pan_end(move |_| {
                let v = st.split.get_untracked();
                crate::player::update(st, |s| s.dlss_split = v);
            })
            .child(
                Row::new()
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().child(Icon::new(icons::COMPARE).class("flt-grip-icon")).class("flt-grip"))
                    .class("flt-grip-area"),
            );
        vec![Box::new(Positioned::new(grip).offset_signal(offset).dimensions(HANDLE, vp.height))]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_rect() {
        // 16:9 в окне 4:3 — по ширине
        assert_eq!(frame_x(1200.0, 900.0, 1920.0, 1080.0, false), (0.0, 1200.0));
        // 4:3 в окне 16:9 — поля по бокам
        let (x, w) = frame_x(1600.0, 900.0, 1440.0, 1080.0, false);
        assert_eq!((x, w), (200.0, 1200.0));
        // заполнить — шире окна
        let (x, w) = frame_x(1600.0, 900.0, 1440.0, 1080.0, true);
        assert!(x < 0.0 && w > 1600.0);
    }

    #[test]
    fn quality_nearest() {
        assert_eq!(quality_index(1.0), 2);
        assert_eq!(quality_index(0.8), 1);
        assert_eq!(quality_index(0.3), 0);
    }
}
