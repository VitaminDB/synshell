//! Окно «Теста экрана E Ink»: своя рамка (заголовок, перетаскивание, изменение размера, полный
//! экран), панель режимов драйвера и вкладки с тестами. Без анимаций переходов — каждая лишняя
//! перерисовка на E Ink видна волной; движение есть только в тесте «Движение».

use std::sync::atomic::Ordering;

use syngui::mss::StyleValue;
use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::widgets::{EventHook, KeyReply};
use synshell_common::display::{Scheme, UpdateMode};

use crate::patterns::{self, Pic};
use crate::state::{self, St, Tab, AUTO_MAX, DRAWN, GHOST_EVERY, MOTION_HZ, UI_WAVES};

pub type W = Box<dyn Widget>;

pub fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

mod icons {
    pub const FULLSCREEN: &str = "\u{E5D0}";
    pub const FULLSCREEN_EXIT: &str = "\u{E5D1}";
    pub const REFRESH: &str = "\u{E5D5}";
    pub const PLAY: &str = "\u{E037}";
    pub const STOP: &str = "\u{E047}";
    pub const REMOVE: &str = "\u{E15B}";
    pub const ADD: &str = "\u{E145}";
    pub const SWAP: &str = "\u{E8D4}";
    pub const SCREEN: &str = "\u{E30C}";
}

fn btn(label: impl Into<String>, class: &str, f: impl FnMut() + Send + 'static) -> W {
    boxed(Button::new(label).class(&format!("btn {class}")).on_click(f))
}

fn icon_btn(glyph: &str, label: impl Into<String>, class: &str, f: impl FnMut() + Send + 'static) -> W {
    let label = label.into();
    let b = if label.is_empty() { Button::new("").icon(glyph) } else { Button::new(label).leading_icon(glyph) };
    boxed(b.class(&format!("btn {class}")).on_click(f))
}

/// Кнопка-переключатель: выбранная — чёрная с белым текстом.
fn chip(label: impl Into<String>, on: bool, f: impl FnMut() + Send + 'static) -> W {
    btn(label, if on { "chip on" } else { "chip" }, f)
}


fn px(v: f32) -> StyleValue {
    StyleValue::px(v)
}

fn rgb(c: [u8; 3]) -> Color {
    Color::from_srgb(c[0], c[1], c[2], 1.0)
}

/// Картинка ровно в своём логическом размере: пиксель картинки — пиксель панели (масштаб 2).
fn pic_exact(p: Pic) -> W {
    let (w, h) = p.logical();
    boxed(DecoratedBox::new().class("pic").style("width", px(w)).style("height", px(h)).child(Image::from_rgba_shared(p.key, p.w, p.h, p.rgba).fit(ImageFit::Fill).placeholder(false)))
}

/// Картинка во всю ширину с заданной высотой (растягивается — для плавных переходов).
fn pic_wide(p: Pic, h: f32) -> W {
    boxed(DecoratedBox::new().class("pic").style("height", px(h)).child(Image::from_rgba_shared(p.key, p.w, p.h, p.rgba).fit(ImageFit::Fill).placeholder(false)))
}

fn section(title: impl Into<String>, hint: impl Into<String>) -> W {
    let hint = hint.into();
    let mut c = Column::new().gap(2.0).class("section").child(Text::new(title).class("section-title"));
    if !hint.is_empty() {
        c = c.child(Text::new(hint).class("hint"));
    }
    boxed(c)
}

// ── Заголовок и панель режимов ───────────────────────────────────────────────────────────

fn titlebar(st: St, fs: bool) -> W {
    let mut row = Row::new().cross_axis_alignment(CrossAxisAlignment::Center).class("titlebar");
    let title = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Icon::new(icons::SCREEN).class("brand-ic"))
        .child(Text::new(t!("Тест экрана E Ink")).class("app-title"));
    row = row.child(DecoratedBox::new().class("grow").child(WindowDragRegion::new().child(DecoratedBox::new().class("drag").child(title))));
    row = row.child(icon_btn(
        if fs { icons::FULLSCREEN_EXIT } else { icons::FULLSCREEN },
        "",
        "flat",
        move || state::toggle_fullscreen(st),
    ));
    if !fs {
        let controls = Reactive::new(move || -> Vec<W> {
            let w = st.window.get();
            vec![boxed(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
        });
        row = row.child(DecoratedBox::new().class("window-controls").child(controls));
    }
    boxed(row)
}

/// Режимы `update_mode` + «Эксперт», цвет, очистка, текущие значения драйвера.
fn modes_panel(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let d = st.drv.get();
        let expert = st.expert.get();
        let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("panel");

        let mut row = Flex::row().wrap().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
        row = row.child(Text::new(t!("Режим:")).class("label"));
        let modes = if d.modes.is_empty() { UpdateMode::KNOWN.iter().map(|s| s.to_string()).collect() } else { d.modes.clone() };
        for m in modes {
            let on = !expert && d.mode.as_deref() == Some(m.as_str());
            let title = UpdateMode::from_name(&m).map(|u| u.title()).unwrap_or_else(|| m.clone());
            row = row.child(chip(title, on, move || {
                st.expert.set(false);
                st.set_mode(m.clone());
            }));
        }
        row = row.child(chip(t!("Эксперт"), expert, move || st.expert.set(!st.expert.get_untracked())));
        row = row.child(DecoratedBox::new().class("sep"));
        if let Some(c) = d.color {
            row = row.child(chip(t!("Цвет"), c, move || st.set_color(true)));
            row = row.child(chip(t!("Монохром"), !c, move || st.set_color(false)));
            row = row.child(DecoratedBox::new().class("sep"));
        }
        row = row.child(icon_btn(icons::REFRESH, t!("Очистить"), "", move || st.refresh()));
        col = col.child(row);

        if expert {
            let mut ex = Flex::row().wrap().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
            ex = ex.child(Text::new(t!("Волна ui:")).class("label"));
            for v in UI_WAVES {
                let on = d.scheme == Some(Scheme::Ui(v));
                ex = ex.child(chip(format!("0x{v:02x}"), on, move || st.set_scheme(Scheme::Ui(v))));
            }
            ex = ex.child(DecoratedBox::new().class("sep"));
            let n = st.auto_n.get();
            let auto_on = d.scheme == Some(Scheme::Auto(n));
            ex = ex.child(Text::new(t!("Авто FPGA:")).class("label"));
            ex = ex.child(icon_btn(icons::REMOVE, "", "", move || {
                let n = st.auto_n.get_untracked().saturating_sub(1);
                st.set_scheme(Scheme::Auto(n));
            }));
            ex = ex.child(chip(format!("auto {n}"), auto_on, move || st.set_scheme(Scheme::Auto(st.auto_n.get_untracked()))));
            ex = ex.child(icon_btn(icons::ADD, "", "", move || {
                let n = (st.auto_n.get_untracked() + 1).min(AUTO_MAX);
                st.set_scheme(Scheme::Auto(n));
            }));
            ex = ex.child(DecoratedBox::new().class("sep"));
            ex = ex.child(
                TextField::new()
                    .placeholder(t!("ui 0x15 / auto 4"))
                    .width(170.0)
                    .on_submit(move |s| match Scheme::parse(s) {
                        Some(sc) => st.set_scheme(sc),
                        None => st.error.set(Some(t!("Не разобрать «{s}»: нужно ui <hex> или auto <n>", s = s))),
                    })
                    .class("field"),
            );
            col = col.child(ex);
        }

        let status = if d.found {
            format!("update_mode: {}    scheme: {}", or_dash(&d.mode_raw), or_dash(&d.scheme_raw))
        } else {
            t!("Драйвер E Ink не найден — режимы не применяются (проверка: SYNSHELL_SYSFS)")
        };
        col = col.child(Text::new(status).class("status"));
        if let Some(e) = st.error.get() {
            col = col.child(Text::new(t!("Ошибка записи: {e}", e = e)).class("error"));
        }
        vec![boxed(col)]
    }))
}

fn or_dash(s: &str) -> &str {
    if s.is_empty() {
        "—"
    } else {
        s
    }
}

fn tabs(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let cur = st.tab.get();
        let mut row = Flex::row().wrap().gap(4.0).class("tabs");
        for t in Tab::ALL {
            row = row.child(btn(t.title(), if t == cur { "tab on" } else { "tab" }, move || st.go(t)));
        }
        vec![boxed(row)]
    }))
}

// ── Страницы ─────────────────────────────────────────────────────────────────────────────

fn page(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let tab = st.tab.get();
        let body = match tab {
            Tab::Gray => gray_page(),
            Tab::Color => color_page(),
            Tab::Text => text_page(),
            Tab::Motion => motion_page(st),
            Tab::Ghost => ghost_page(st),
            Tab::Scroll => return vec![scroll_page()],
        };
        vec![boxed(ScrollView::new().vertical().class("grow").child(DecoratedBox::new().class("page").child(body)))]
    }))
}

fn gray_page() -> W {
    let mut steps = Row::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("steps");
    for i in 0..16u8 {
        let v = i * 17;
        steps = steps.child(
            DecoratedBox::new()
                .class(if i < 8 { "step dark" } else { "step" })
                .style("background-color", StyleValue::from(rgb([v, v, v])))
                .child(Text::new(i.to_string()).class("step-label")),
        );
    }
    let mut labels = Column::new().class("band-labels");
    for l in patterns::DITHER_BANDS {
        labels = labels.child(DecoratedBox::new().class("band-label").child(Text::new(t!(l)).class("hint")));
    }
    boxed(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(section(t!("16 ступеней серого"), t!("Соседние ступени должны различаться; 0 — чёрный, 15 — белый")))
            .child(steps)
            .child(section(t!("Плавный градиент"), t!("Сверху — плавно, снизу — те же 16 ступеней: полосы и пятна в плавном — недостаток волны")))
            .child(pic_wide(patterns::gray_ramp(), 90.0))
            .child(section(t!("Мелкий дизеринг"), t!("Пиксель в пиксель панели: шахматка 1 px должна выглядеть ровным серым, без муара")))
            .child(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Start).child(labels).child(pic_exact(patterns::dither()))),
    )
}

/// Цвета Kaleido: название и sRGB.
const COLORS: [(&str, [u8; 3]); 9] = [
    (n_!("Красный"), [0xe5, 0x1e, 0x25]),
    (n_!("Зелёный"), [0x1f, 0xa2, 0x3c]),
    (n_!("Синий"), [0x1e, 0x4f, 0xd8]),
    (n_!("Голубой"), [0x00, 0xb4, 0xe6]),
    (n_!("Пурпурный"), [0xd6, 0x1f, 0x9b]),
    (n_!("Жёлтый"), [0xff, 0xdd, 0x00]),
    (n_!("Оранжевый"), [0xf5, 0x82, 0x1f]),
    (n_!("Коричневый"), [0x7a, 0x48, 0x22]),
    (n_!("Кожа"), [0xf1, 0xc2, 0x9e]),
];

fn mix(c: [u8; 3], to: [u8; 3], k: f32) -> [u8; 3] {
    let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k).round() as u8;
    [m(c[0], to[0]), m(c[1], to[1]), m(c[2], to[2])]
}

fn patches(f: impl Fn([u8; 3]) -> [u8; 3]) -> W {
    let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for (name, c) in COLORS {
        let c = f(c);
        let light = (c[0] as u32 * 299 + c[1] as u32 * 587 + c[2] as u32 * 114) / 1000 > 140;
        row = row.child(
            DecoratedBox::new()
                .class(if light { "patch" } else { "patch dark" })
                .style("background-color", StyleValue::from(rgb(c)))
                .child(Text::new(t!(name)).max_lines(1).elide(Elide::End).class("patch-label")),
        );
    }
    boxed(row)
}

fn color_page() -> W {
    boxed(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(section(t!("Насыщенные"), t!("Основные R, G, B, смешанные C, M, Y, оранжевый, коричневый, кожа")))
            .child(patches(|c| c))
            .child(section(t!("Пастельные"), t!("Те же цвета, наполовину с белым — на Kaleido самые трудные")))
            .child(patches(|c| mix(c, [255, 255, 255], 0.55)))
            .child(section(t!("Тёмные"), ""))
            .child(patches(|c| mix(c, [0, 0, 0], 0.45)))
            .child(section(t!("Переходы"), t!("Небо → закат, коричневый → кожа, хвоя → зелень, красный → синий, серое")))
            .child(pic_wide(patterns::color_ramps(), 180.0))
            .child(section(t!("Оттенок и яркость"), t!("По горизонтали — оттенок, сверху — к белому, снизу — к чёрному")))
            .child(pic_wide(patterns::hue_field(), 180.0))
            .child(section(t!("Картинка"), t!("Пейзаж без файла: мягкие переходы неба, горы, луг с цветами")))
            .child(pic_exact(patterns::landscape())),
    )
}

const PANGRAM: &str = n_!("Съешь же ещё этих мягких французских булок, да выпей чаю. 0123456789");
const TEXT_PT: [u32; 11] = [8, 9, 10, 11, 12, 14, 16, 18, 20, 24, 32];

fn text_column(dark: bool) -> W {
    let mut col = Column::new().gap(6.0).class(if dark { "text-col dark" } else { "text-col" });
    for pt in TEXT_PT {
        let size = pt as f32 * 4.0 / 3.0;
        col = col.child(Text::new(format!("{pt} pt  {}", t!(PANGRAM))).max_lines(1).elide(Elide::End).style("font-size", px(size)).class("sample"));
    }
    boxed(col)
}

fn lines_canvas() -> W {
    boxed(
        Canvas::new(|c, _| {
            let w = c.width();
            c.clear(Color::white());
            c.set_color(Color::black());
            let widths = [0.5f32, 1.0, 1.5, 2.0, 3.0];
            for (i, lw) in widths.iter().enumerate() {
                let y = 16.0 + i as f32 * 22.0;
                c.set_stroke_width(*lw);
                c.draw_line(10.0, y, w * 0.45, y);
                // наклонные — видны лесенки и рваные края
                let x0 = w * 0.52 + i as f32 * 40.0;
                c.draw_line(x0, 6.0, x0 + 90.0, 112.0);
            }
            c.set_stroke_width(1.0);
            for k in 0..6 {
                c.stroke_circle(w * 0.9, 60.0, 8.0 + k as f32 * 9.0);
            }
        })
        .height(124.0)
        .class("canvas"),
    )
}

fn text_page() -> W {
    boxed(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(section(t!("Текст 8–32 pt"), t!("Чёрное на белом и белое на чёрном: тонкие засечки, ореолы вокруг букв")))
            .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(DecoratedBox::new().class("grow half").child(text_column(false))).child(DecoratedBox::new().class("grow half").child(text_column(true))))
            .child(section(t!("Тонкие линии"), t!("Толщина 0,5–3: горизонтальные, наклонные, окружности")))
            .child(lines_canvas())
            .child(section(t!("Линии в пикселях панели"), t!("1, 2, 3, 4 px через промежуток: горизонтальные, вертикальные, наклонная 1 px")))
            .child(pic_exact(patterns::hairlines())),
    )
}

/// Позиций на круге «Движения»: квадрат делает шаг на кадр.
const ORBIT: u64 = 24;

fn motion_canvas(tick: u64) -> W {
    boxed(
        Canvas::new(move |c, _| {
            DRAWN.fetch_add(1, Ordering::Relaxed);
            let (w, h) = (c.width(), c.height());
            c.clear(Color::white());
            // круг: метки позиций и квадрат на текущей
            let (cx, cy, r) = (h / 2.0, h / 2.0, h / 2.0 - 40.0);
            c.set_color(Color::from_srgb(160, 160, 160, 1.0));
            c.set_stroke_width(1.0);
            c.stroke_circle(cx, cy, r);
            for k in 0..ORBIT {
                let a = k as f32 / ORBIT as f32 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                c.fill_circle(cx + r * a.cos(), cy + r * a.sin(), 3.0);
            }
            let a = (tick % ORBIT) as f32 / ORBIT as f32 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
            c.set_color(Color::black());
            c.set_stroke_width(3.0);
            c.draw_line(cx, cy, x, y);
            c.fill_rect(x - 22.0, y - 22.0, 44.0, 44.0);
            // полоса: бегунок по дорожке, слева направо
            let (x0, x1) = (h + 20.0, w - 20.0);
            if x1 - x0 > 80.0 {
                let track = (x1 - x0) - 40.0;
                c.set_color(Color::from_srgb(160, 160, 160, 1.0));
                c.set_stroke_width(1.0);
                c.draw_rect(x0, 40.0, x1 - x0, 120.0);
                for k in 0..=ORBIT {
                    let tx = x0 + 20.0 + track * k as f32 / ORBIT as f32;
                    c.draw_line(tx, 160.0, tx, 172.0);
                }
                c.set_color(Color::black());
                let bx = x0 + track * (tick % ORBIT) as f32 / ORBIT as f32;
                c.fill_rect(bx, 40.0, 40.0, 120.0);
                // шкала кадров двоичным кодом: какой кадр на самом деле на панели
                let bits = 10;
                let bw = ((x1 - x0) / bits as f32).min(60.0);
                for b in 0..bits {
                    let on = (tick >> (bits - 1 - b)) & 1 == 1;
                    let bxx = x0 + b as f32 * bw;
                    c.set_color(Color::black());
                    c.set_stroke_width(1.0);
                    c.draw_rect(bxx + 2.0, 220.0, bw - 4.0, 50.0);
                    if on {
                        c.fill_rect(bxx + 2.0, 220.0, bw - 4.0, 50.0);
                    }
                }
            }
        })
        .height(420.0)
        .class("canvas"),
    )
}

fn motion_page(st: St) -> W {
    let controls = Reactive::new(move || -> Vec<W> {
        let on = st.motion_on.get();
        let hz = st.motion_hz.get();
        let mut row = Flex::row().wrap().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
        row = row.child(if on {
            icon_btn(icons::STOP, t!("Стоп"), "primary", move || st.motion_stop())
        } else {
            icon_btn(icons::PLAY, t!("Пуск"), "primary", move || st.motion_start())
        });
        row = row.child(DecoratedBox::new().class("sep"));
        row = row.child(Text::new(t!("Частота:")).class("label"));
        for f in MOTION_HZ {
            row = row.child(chip(t!("{n} Гц", n = f), hz == f, move || st.motion_hz(f)));
        }
        row = row.child(DecoratedBox::new().class("sep"));
        row = row.child(btn(t!("Сброс"), "", move || {
            st.tick.set(0);
            st.stamp_ms.set(0);
            if st.motion_on.get_untracked() {
                st.motion_start();
            }
        }));
        vec![boxed(row)]
    });
    let stats = Reactive::new(move || -> Vec<W> {
        let tick = st.tick.get();
        let ms = st.stamp_ms.get();
        let r = st.rate.get();
        let hz = st.motion_hz.get();
        vec![boxed(
            Flex::row()
                .wrap()
                .gap(24.0)
                .cross_axis_alignment(CrossAxisAlignment::End)
                .child(Column::new().child(Text::new(t!("кадр")).class("hint")).child(Text::new(format!("{tick}")).class("big")))
                .child(Column::new().child(Text::new(t!("время, мс")).class("hint")).child(Text::new(group(ms)).class("big")))
                .child(
                    Column::new()
                        .gap(2.0)
                        .child(Text::new(t!("Заявлено: {hz} кадров/с", hz = hz)).class("stat"))
                        .child(Text::new(t!("Таймер: {v} кадров/с", v = format!("{:.1}", r.ticks))).class("stat"))
                        .child(Text::new(t!("Отрисовано приложением: {v} кадров/с", v = format!("{:.1}", r.drawn))).class("stat")),
                ),
        )]
    });
    let canvas = Reactive::new(move || -> Vec<W> { vec![motion_canvas(st.tick.get())] });
    boxed(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(section(t!("Движение"), t!("Квадрат шагает по кругу, полоса — по дорожке, внизу — номер кадра двоичным кодом. Снимите экран: видно, сколько кадров панель успевает показать и сколько следов остаётся")))
            .child(controls)
            .child(stats)
            .child(canvas),
    )
}

/// 12345678 → «12 345 678».
fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('\u{2009}');
        }
        out.push(ch);
    }
    out
}

fn checker(inv: bool) -> W {
    boxed(
        Canvas::new(move |c, _| {
            let (w, h) = (c.width(), c.height());
            let cell = 35.0;
            c.clear(if inv { Color::black() } else { Color::white() });
            c.set_color(if inv { Color::white() } else { Color::black() });
            let (nx, ny) = ((w / cell) as i32, (h / cell) as i32);
            for y in 0..ny {
                for x in 0..nx {
                    if (x + y) % 2 == 0 {
                        c.fill_rect(x as f32 * cell, y as f32 * cell, cell, cell);
                    }
                }
            }
        })
        .size(420.0, 420.0)
        .class("canvas"),
    )
}

fn ghost_page(st: St) -> W {
    let controls = Reactive::new(move || -> Vec<W> {
        let every = st.ghost_every.get();
        let mut row = Flex::row().wrap().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
        row = row.child(icon_btn(icons::SWAP, t!("Инвертировать"), "primary", move || st.ghost_flip()));
        row = row.child(DecoratedBox::new().class("sep"));
        row = row.child(Text::new(t!("Авто:")).class("label"));
        for s in GHOST_EVERY {
            let label = if s == 0 { t!("выкл") } else { t!("{n} с", n = s) };
            row = row.child(chip(label, every == s, move || st.ghost_every(s)));
        }
        vec![boxed(row)]
    });
    let body = Reactive::new(move || -> Vec<W> {
        let inv = st.ghost_inv.get();
        let n = st.ghost_count.get();
        let mut steps = Row::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("steps small");
        for i in 0..16u8 {
            let v = if inv { 255 - i * 17 } else { i * 17 };
            steps = steps.child(DecoratedBox::new().class("step").style("background-color", StyleValue::from(rgb([v, v, v]))));
        }
        let word = if inv { t!("ПОСЛЕ") } else { t!("ДО") };
        vec![boxed(
            Flex::row()
                .wrap()
                .gap(16.0)
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .child(checker(inv))
                .child(
                    Column::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .class("ghost-side")
                        .child(DecoratedBox::new().class(if inv { "ghost-word dark" } else { "ghost-word" }).child(Text::new(word).class("ghost-text")))
                        .child(steps)
                        .child(Text::new(t!("Смен: {n}", n = n)).class("stat")),
                ),
        )]
    });
    boxed(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(section(t!("Следы"), t!("Шахматка и надпись меняются на обратные: в хорошем режиме прежняя картинка не просвечивает")))
            .child(controls)
            .child(body),
    )
}

const SCROLL_TEXT: [&str; 4] = [
    n_!("Прокрутите список пальцем: строки, подписи и картинки должны оставаться читаемыми, а следы — исчезать после остановки."),
    n_!("Быстрые режимы показывают движение раньше, качественные — чище, но с задержкой."),
    n_!("Сравните, как одна и та же строка выглядит в движении и после него."),
    n_!("Маленький текст и цветные миниатюры — самое трудное для E Ink при прокрутке."),
];

fn scroll_page() -> W {
    let mut col = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("page");
    col = col.child(section(t!("Прокрутка"), t!("120 строк с текстом и картинками")));
    for i in 0..120usize {
        col = col.child(
            Row::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("list-row")
                .child(pic_exact(patterns::thumb(i)))
                .child(
                    Column::new()
                        .gap(4.0)
                        .class("grow")
                        .child(Text::new(t!("Строка {n}", n = i + 1)).class("row-title"))
                        .child(Text::new(t!(SCROLL_TEXT[i % SCROLL_TEXT.len()])).max_lines(2).elide(Elide::End).class("row-text")),
                ),
        );
    }
    boxed(ScrollView::new().vertical().class("grow").child(col))
}

// ── Окно ─────────────────────────────────────────────────────────────────────────────────

/// F11 — полный экран, Esc — выход из него.
fn keys(st: St, inner: impl Widget + 'static) -> W {
    boxed(
        EventHook::new()
            .on_key_down(move |k, _| {
                if k == Key::F11 {
                    state::toggle_fullscreen(st);
                    return KeyReply::Handled;
                }
                if k == Key::Escape && st.window.get_untracked().fullscreen {
                    state::set_fullscreen(st, false);
                    return KeyReply::Handled;
                }
                KeyReply::Ignore
            })
            .child(inner),
    )
}

pub fn root(st: St) -> W {
    // перестройка только при смене полного экрана, не фокуса
    let full = use_signal(st.window.get_untracked().fullscreen);
    create_effect(move || {
        let v = st.window.get().fullscreen;
        if full.get_untracked() != v {
            full.set(v);
        }
    });
    boxed(Reactive::new(move || -> Vec<W> {
        let fs = full.get();
        let main = Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("window")
            .child(titlebar(st, fs))
            .child(modes_panel(st))
            .child(tabs(st))
            .child(DecoratedBox::new().class("grow body").child(page(st)));
        let frame = DecoratedBox::new().class(if fs { "window-frame full" } else { "window-frame" }).clip(true).child(keys(st, main));
        vec![boxed(WindowResizeRegion::new().over_content(true).inset(5.0).enabled(!fs).child(frame))]
    }))
}
