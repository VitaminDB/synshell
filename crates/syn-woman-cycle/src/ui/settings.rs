//! «Настройки»: параметры цикла, масштаб и тема, окно, данные.

use std::sync::atomic::{AtomicU64, Ordering};

use syngui::prelude::*;
use syngui::GestureDetector;
use syngui::StyleValue;

use super::{boxed, icon, page_header, rise, scroll_page, W};
use crate::icons;
use crate::decor::{self, Decor};
use crate::model::{Settings, ThemeMode};
use crate::state::St;

static SCALE_SAVE: AtomicU64 = AtomicU64::new(0);

pub fn section(title: String, rows: Vec<W>) -> W {
    let mut col = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    let n = rows.len();
    for (i, r) in rows.into_iter().enumerate() {
        col = col.child(r);
        if i + 1 < n {
            col = col.child(DecoratedBox::new().class("sep-wrap").child(DecoratedBox::new().class("sep")));
        }
    }
    boxed(Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(title).class("section")).child(DecoratedBox::new().class("group").child(col)))
}

/// Цвет значка строки настроек по смыслу.
fn badge_tone(glyph: &str) -> &'static str {
    match glyph {
        g if g == icons::DROP => "rose",
        g if g == icons::EVENT || g == icons::CALENDAR => "blue",
        g if g == icons::EGG || g == icons::FLOWER => "amber",
        g if g == icons::SPARK || g == icons::PALETTE || g == icons::DARK => "violet",
        g if g == icons::HEART => "pink",
        g if g == icons::ZOOM => "teal",
        _ => "gray",
    }
}

fn label(glyph: &str, title: String, hint: String) -> W {
    let mut texts = Column::new().gap(2.0).child(Text::new(title).max_lines(2).class("row-title"));
    if !hint.is_empty() {
        texts = texts.child(Text::new(hint).max_lines(3).class("hint"));
    }
    boxed(
        DecoratedBox::new()
            .class("grow")
            .child(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class(&format!("set-badge {}", badge_tone(glyph))).child(icon(glyph).class("ic set-ic"))).child(DecoratedBox::new().class("grow").child(texts))),
    )
}

fn toggle_row(st: St, glyph: &str, title: String, hint: String, on: bool, set: impl Fn(&mut Settings, bool) + Send + Sync + 'static) -> W {
    let set = std::sync::Arc::new(set);
    let s2 = set.clone();
    boxed(
        GestureDetector::new()
            .on_click(move || {
                let s = s2.clone();
                st.set_settings(move |x| s(x, !on));
            })
            .child(DecoratedBox::new().class("set-row").child(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(label(glyph, title, hint)).child(Toggle::new().on(on).on_change(move |v| {
                let s = set.clone();
                st.set_settings(move |x| s(x, v));
            })))),
    )
}

pub fn stepper(st: St, glyph: &str, title: String, hint: String, value: i32, min: i32, max: i32, unit: String, set: fn(&mut Settings, i32)) -> W {
    let dec = Button::new("").icon(icons::REMOVE).class("btn round step").disabled(value <= min).on_click(move || st.set_settings(|s| set(s, (value - 1).max(min))));
    let inc = Button::new("").icon(icons::ADD).class("btn round step").disabled(value >= max).on_click(move || st.set_settings(|s| set(s, (value + 1).min(max))));
    let value = DecoratedBox::new().class("step-value").child(Column::new().cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(value.to_string()).class("step-num")).child(Text::new(unit).class("step-unit")));
    let controls = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(dec).child(value).child(inc);
    // на узком окне (или крупном масштабе) кнопки — под подписью
    let row: W = if viewport_size().get_untracked().width < 400.0 {
        boxed(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(label(glyph, title, hint)).child(Row::new().main_axis_alignment(MainAxisAlignment::End).child(controls)))
    } else {
        boxed(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(label(glyph, title, hint)).child(controls))
    };
    boxed(DecoratedBox::new().class("set-row").child(row))
}

/// Масштаб интерфейса: применяется сразу, сохраняется после паузы.
fn set_scale(st: St, v: f32) {
    let v = (v * 20.0).round() / 20.0;
    let v = v.clamp(0.7, 2.0);
    syngui::scale::set_ui_scale(v);
    st.scale.set(v);
    let id = SCALE_SAVE.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        run_on_main_thread(move || {
            if SCALE_SAVE.load(Ordering::Relaxed) == id {
                st.set_settings(|s| s.scale = v);
            }
        });
    });
}

fn scale_row(st: St) -> W {
    let value = Reactive::new(move || -> Vec<W> { vec![boxed(Text::new(format!("{}%", (st.scale.get() * 100.0).round() as i32)).class("scale-value"))] });
    let start = st.scale.get_untracked();
    let presets: [f32; 5] = [0.85, 1.0, 1.15, 1.3, 1.5];
    let chips = Reactive::new(move || -> Vec<W> {
        let cur = st.scale.get();
        let mut row = Flex::new().direction(FlexDirection::Row).wrap().gap(8.0);
        for p in presets {
            let on = (cur - p).abs() < 0.01;
            row = row.child(Button::new(format!("{}%", (p * 100.0).round() as i32)).class(if on { "btn pick plum on" } else { "btn pick plum" }).on_click(move || set_scale(st, p)));
        }
        vec![boxed(row)]
    });
    boxed(
        DecoratedBox::new().class("set-row").child(
            Column::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(label(icons::ZOOM, t!("Масштаб"), t!("Размер текста и элементов"))).child(value))
                .child(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Button::new("").icon(icons::REMOVE).class("btn round step").on_click(move || set_scale(st, st.scale.get_untracked() - 0.05)))
                        .child(DecoratedBox::new().class("grow").child(Slider::new().range(0.7, 2.0).step(0.05).value(start).on_change(move |v| set_scale(st, v))))
                        .child(Button::new("").icon(icons::ADD).class("btn round step").on_click(move || set_scale(st, st.scale.get_untracked() + 0.05))),
                )
                .child(chips),
        ),
    )
}

/// Цвет переменной темы оформления (или запасной).
fn decor_color(d: Decor, dark: bool, key: &str, fallback: &str) -> Color {
    Color::from_hex(d.vars(dark).iter().find(|(k, _)| *k == key).map(|(_, v)| *v).unwrap_or(fallback))
}

/// Карточка стиля: фон-рисунок, капля, кружок «сегодня», сердечко и название.
fn decor_card(st: St, d: Decor, on: bool, w: f32, dark: bool) -> W {
    let h = 78.0;
    let dark = dark || d.always_dark();
    let bg = crate::bg_color(dark, d);
    let art: W = boxed(DecoratedBox::new().style("background-color", StyleValue::from(bg)));
    let ink = d.ink(dark);
    let today = decor_color(d, dark, "today", if dark { "#14d1ba" } else { "#00a896" });
    let rose = decor_color(d, dark, "rose", if dark { "#ff4767" } else { "#e8294f" });
    let shapes = d.shapes();
    let sample = Canvas::new(move |c, _| {
        let y = h * 0.62;
        if shapes {
            decor::paint_drop(c, w * 0.26, y, 30.0, ink.blood, true);
            decor::paint_heart(c, w * 0.76, y - 2.0, 22.0, ink.heart, Some(bg));
        } else {
            c.set_color(rose);
            c.fill_circle(w * 0.26, y, 12.0);
            decor::paint_heart(c, w * 0.76, y - 1.0, 20.0, ink.heart, None);
        }
        c.set_color(today);
        c.fill_circle(w * 0.51, y, 12.0);
    })
    .size(w, h);
    let face = Stack::new().child(DecoratedBox::new().class("decor-art").clip(true).style("width", StyleValue::px(w)).style("height", StyleValue::px(h)).child(art)).child(sample);
    boxed(
        GestureDetector::new()
            .on_click(move || {
                st.set_settings(|s| s.decor = d);
                crate::apply_theme(st);
            })
            .child(
                DecoratedBox::new()
                    .class(if on { "decor-card on" } else { "decor-card" })
                    .clip(true)
                    .style("width", StyleValue::px(w + 4.0))
                    .child(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(face).child(DecoratedBox::new().class("decor-name").child(Text::new(d.name()).max_lines(1).class("decor-name-text")))),
            ),
    )
}

fn decor_row(st: St, cur: Decor, width: f32) -> W {
    let dark = st.dark.get_untracked();
    let gap = 10.0;
    let per = if width > 520.0 { 4.0 } else { 3.0 };
    // место в строке: рамка группы (2) и поля строки (32); карточка = рисунок + 4 на рамку
    let avail = width - 2.0 - 32.0;
    let w = ((avail - gap * (per - 1.0)) / per).floor() - 4.0;
    let mut f = Flex::new().direction(FlexDirection::Row).wrap().gap(gap);
    for d in Decor::ALL {
        f = f.child(decor_card(st, d, d == cur, w, dark));
    }
    let hint = if cur.always_dark() { t!("«Звёздная ночь» всегда тёмная") } else { t!("Цвета, капли и сердечки в календаре") };
    boxed(DecoratedBox::new().class("set-row").child(Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(label(icons::FLOWER, t!("Оформление"), hint)).child(f)))
}

fn theme_row(st: St, cur: ThemeMode, width: f32) -> W {
    // три равные кнопки: рамка группы, поля строки, поля дорожки и промежутки
    let seg_w = ((width - 2.0 - 32.0 - 6.0 - 6.0 * 2.0) / 3.0).floor();
    let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for (m, g, l) in [(ThemeMode::System, icons::AUTO, t!("Как в системе")), (ThemeMode::Light, icons::LIGHT, t!("Светлая")), (ThemeMode::Dark, icons::DARK, t!("Тёмная"))] {
        let on = m == cur;
        row = row.child(
            GestureDetector::new()
                .on_click(move || {
                    st.set_settings(|s| s.theme = m);
                    crate::apply_theme(st);
                })
                .child(DecoratedBox::new().class(if on { "seg on" } else { "seg" }).style("width", StyleValue::px(seg_w)).child(
                    Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(g).class(if on { "ic seg-ic on" } else { "ic seg-ic" })).child(Text::new(l).max_lines(1).class(if on { "seg-text on center" } else { "seg-text center" })),
                )),
        );
    }
    boxed(DecoratedBox::new().class("set-row").child(Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(label(icons::PALETTE, t!("Тема"), String::new())).child(DecoratedBox::new().class("segs").child(row))))
}

/// «Язык»: «Как в системе» (язык из «Параметров») по умолчанию или свой из списка.
fn language_row() -> W {
    let r = synshell_common::i18n::language_row();
    boxed(
        DecoratedBox::new().class("set-row").child(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(label(icons::TRANSLATE, r.title, r.hint))
                .child(synshell_common::i18n::language_dropdown().class("set-dropdown")),
        ),
    )
}

pub fn page(st: St, wide: bool) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let data = st.data.get();
        let fc = st.fc.get_untracked();
        let s = data.settings.clone();
        let mut blocks: Vec<W> = vec![page_header(st, wide, t!("Цикл, вид и данные"), t!("Настройки"), vec![])];
        let auto_hint = if s.auto && fc.cycles.iter().filter(|c| c.len.is_some()).count() >= 1 {
            t!("Сейчас по истории: цикл {c} дн., месячные {p} дн.", c = fc.cycle, p = fc.period)
        } else {
            t!("Пока истории мало — берутся значения ниже")
        };
        blocks.push(rise(
            0,
            section(
                t!("Цикл"),
                vec![
                    toggle_row(st, icons::SPARK, t!("Считать по истории"), auto_hint, s.auto, |x, v| x.auto = v),
                    stepper(st, icons::EVENT, t!("Длина цикла"), t!("Обычно 21–35 дней"), s.cycle_len, 18, 45, t!("дн."), |x, v| x.cycle_len = v),
                    stepper(st, icons::DROP, t!("Длительность месячных"), t!("Обычно 3–7 дней"), s.period_len, 1, 10, t!("дн."), |x, v| x.period_len = v),
                    stepper(st, icons::DARK, t!("Лютеиновая фаза"), t!("От овуляции до месячных, обычно 14"), s.luteal, 10, 16, t!("дн."), |x, v| x.luteal = v),
                ],
            ),
        ));
        blocks.push(rise(
            1,
            section(
                t!("Вид"),
                {
                    let mut rows: Vec<W> = Vec::new();
                    // На телефоне и компьютере synshell — общий язык в «Параметрах»; в APK/iOS их нет.
                    if !cfg!(any(target_os = "android", target_os = "ios")) {
                        rows.push(language_row());
                    }
                    rows.extend([
                    scale_row(st),
                    decor_row(st, s.decor, super::content_width(wide)),
                    theme_row(st, s.theme, super::content_width(wide)),
                    toggle_row(st, icons::CALENDAR, t!("Неделя с понедельника"), String::new(), s.monday, |x, v| x.monday = v),
                    toggle_row(st, icons::FLOWER, t!("Показывать фертильное окно"), String::new(), s.show_fertile, |x, v| x.show_fertile = v),
                    toggle_row(st, icons::EGG, t!("Показывать овуляцию"), String::new(), s.show_ovulation, |x, v| x.show_ovulation = v),
                    toggle_row(st, icons::HEART, t!("Показывать близость"), t!("Отметки в календаре и статистике; можно скрыть от посторонних глаз"), s.show_intimacy, |x, v| x.show_intimacy = v),
                    ]);
                    rows
                },
            ),
        ));
        // iPhone: окно всегда на весь экран — раздела «Окно» нет.
        if !cfg!(target_os = "ios") {
        blocks.push(rise(
            2,
            section(
                t!("Окно"),
                vec![toggle_row(st, icons::FULLSCREEN, t!("Открывать во весь экран"), t!("Выйти из полноэкранного режима — кнопкой вверху или F11"), s.fullscreen, |x, v| {
                    x.fullscreen = v;
                    syngui::signal::set_fullscreen(v);
                })],
            ),
        ));
        }
        let n = data.days.len();
        blocks.push(rise(
            3,
            section(
                t!("Данные"),
                vec![
                    boxed(DecoratedBox::new().class("set-row").child(label(icons::LOCK, t!("Только на этом устройстве"), t!("Отметки никуда не отправляются: {n} дней в файле ~/.local/share/synshell/woman-cycle", n = n)))),
                    boxed(DecoratedBox::new().class("set-row").child(Button::new(t!("Удалить все отметки")).icon(icons::DELETE).class("btn danger").on_click(move || st.confirm_reset.set(true)))),
                ],
            ),
        ));
        blocks.push(Box::new(Text::new(format!("{} {}", t!("Цикл"), env!("CARGO_PKG_VERSION"))).class("hint center disclaimer")));
        vec![scroll_page(wide, blocks)]
    }))
}

pub fn reset_dialog(st: St) -> W {
    let scrim = Reactive::new(move || -> Vec<W> {
        let shown = st.confirm_reset.get();
        let vp = viewport_size().get();
        vec![boxed(
            Presence::new(shown, GestureDetector::new().on_click(move || st.confirm_reset.set(false)).child(DecoratedBox::new().class("scrim").style("width", vp.width).style("height", vp.height)))
                .enter(Motion::fade())
                .exit(Motion::fade())
                .duration_ms(200)
                .initial(false),
        )]
    });
    let card = move || -> W {
        boxed(
            GestureDetector::new().on_click(|| {}).child(
                DecoratedBox::new().class("dialog").child(
                    Column::new()
                        .gap(14.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(DecoratedBox::new().class("hero-badge danger").child(icon(icons::DELETE).class("ic hero-ic"))))
                        .child(Text::new(t!("Удалить все отметки?")).class("h2 center"))
                        .child(Text::new(t!("История циклов, симптомы и настроение будут удалены без возможности восстановления. Настройки сохранятся.")).max_lines(5).class("hint center"))
                        .child(Row::new().gap(10.0).child(Button::new(t!("Отмена")).class("btn ghost grow").on_click(move || st.confirm_reset.set(false))).child(Button::new(t!("Удалить")).class("btn danger-solid grow").on_click(move || {
                            st.reset();
                            st.toast(t!("Все отметки удалены"));
                        }))),
                ),
            ),
        )
    };
    boxed(
        Stack::new().fit(StackFit::Expand).child(scrim).child(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Presence::signal(st.confirm_reset, card).enter(Motion::fade().scale(0.9)).exit(Motion::fade().scale(0.95)).duration_ms(260).exit_duration_ms(160)),
        ),
    )
}
