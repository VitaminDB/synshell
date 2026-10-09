//! Экран компаса: курс крупно, картушка (поворачивается, сверху — неподвижная метка
//! направления), уровень, поле и калибровка, координаты; калибровка «восьмёркой» —
//! поверх.

use std::cell::Cell;

use syngui::core::Color;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::containers::Positioned;
use syngui::GestureDetector;
use synsystem::geoclue_client::{self, Status};

use crate::icons;
use crate::sensor::{Cal, Reading, Sensors, State};

thread_local! {
    /// Последний угол поворота картушки без «перескока» 359° → 0°.
    static SHOWN: Cell<f32> = const { Cell::new(0.0) };
}

/// Ближайший к прошлому эквивалент угла: картушка не крутится через всю окружность.
fn unwrap(h: f32) -> f32 {
    SHOWN.with(|s| {
        let prev = s.get();
        let mut d = (h - prev) % 360.0;
        if d > 180.0 {
            d -= 360.0;
        } else if d < -180.0 {
            d += 360.0;
        }
        let v = prev + d;
        s.set(v);
        v
    })
}

/// Сторона света по курсу: 16 румбов → 8 подписей.
pub fn cardinal(h: f32) -> String {
    const NAMES: [&str; 8] = [n_!("С"), n_!("СВ"), n_!("В"), n_!("ЮВ"), n_!("Ю"), n_!("ЮЗ"), n_!("З"), n_!("СЗ")];
    let i = (((h + 22.5) % 360.0) / 45.0) as usize % 8;
    syngui::i18n::t(NAMES[i])
}

/// Широта/долгота в градусах-минутах.
fn dms(v: f64, pos: &str, neg: &str) -> String {
    let a = v.abs();
    let d = a.floor();
    let m = (a - d) * 60.0;
    format!("{}°{}′ {}", d as i64, synshell_common::decimal(m, 1), if v >= 0.0 { pos } else { neg })
}

#[derive(Clone, Copy)]
pub struct St {
    pub sensors: Sensors,
    /// Метка направления (курс, на который ориентироваться).
    pub target: RwSignal<Option<f32>>,
    pub location: RwSignal<Option<geoclue_client::Fix>>,
}

impl St {
    pub fn new(sensors: Sensors) -> Self {
        St { sensors, target: use_signal(None), location: use_signal(None) }
    }
}

/// Слежение за местоположением (GeoClue), пока программа открыта.
pub fn start_location(st: St) {
    if std::env::var_os("SYN_COMPASS_DEMO").is_some() {
        st.location.set(Some(geoclue_client::Fix { lat: 55.7539, lon: 37.6208, accuracy: 12.0, altitude: Some(156.0), heading: None, speed: None }));
        return;
    }
    let loc = st.location;
    let w = geoclue_client::watch("syn-compass", 0, move |s| {
        if let Status::Fix(f) = s {
            run_on_main_thread(move || loc.set(Some(f)));
        }
    });
    // держать до выхода из программы
    std::mem::forget(w);
}

pub fn root(st: St) -> impl Widget {
    let body = move || -> Box<dyn Widget> {
        syngui::i18n::subscribe();
        let vp = viewport_size().get();
        let wide = vp.width > vp.height * 1.15 && vp.width > 640.0;
        let dial_size = if wide { (vp.height - 200.0).clamp(200.0, 520.0) } else { (vp.width - 48.0).clamp(220.0, 440.0) };
        if st.sensors.collect.get().is_some() {
            return Box::new(calibration(st));
        }
        match st.sensors.state.get() {
            State::Starting => Box::new(center(Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(CircularProgress::new().indeterminate().size(32.0)).child(Text::new(t!("Запуск датчиков…")).class("muted")))),
            State::Missing(msg) => Box::new(center(
                Column::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(icons::EXPLORE_OFF).class("big-icon"))
                    .child(Text::new(t!("Компас недоступен")).class("title"))
                    .child(Text::new(msg).max_lines(4).class("muted center")),
            )),
            State::Ok(r) => {
                let dial = dial(st, &r, dial_size);
                let info = info(st, &r);
                if wide {
                    Box::new(center(Row::new().gap(40.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center).child(heading_text(&r)).child(dial)).child(info)))
                } else {
                    Box::new(
                        ScrollView::new().vertical().child(
                            Column::new()
                                .gap(18.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(heading_text(&r))
                                .child(dial)
                                .child(info)
                                .class("page"),
                        ),
                    )
                }
            }
        }
    };
    DecoratedBox::new().child(syngui::widgets::Reactive::new(move || vec![body()])).class("root")
}

/// По центру окна. Высота — явно: Reactive отдаёт детям свободные ограничения, и колонка
/// без неё сжалась бы по содержимому.
fn center<M>(w: impl syngui::IntoWidget<M>) -> impl Widget {
    let vp = viewport_size().get_untracked();
    Column::new()
        .main_axis_alignment(MainAxisAlignment::Center)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(w)
        .style("width", StyleValue::px(vp.width))
        .style("height", StyleValue::px(vp.height))
        .class("center-page")
}

fn heading_text(r: &Reading) -> impl Widget {
    let deg = (r.heading.round() as i32).rem_euclid(360);
    let sub = if r.upright { t!("магнитный · по задней камере") } else { t!("магнитный север") };
    Column::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::End)
                .child(Text::new(format!("{deg}°")).class("heading"))
                .child(Text::new(cardinal(r.heading)).class("cardinal")),
        )
        .child(Text::new(sub).class("muted"))
}

/// Картушка: шкала и подписи поворачиваются против курса, метка направления сверху
/// неподвижна; касание — поставить или снять метку направления.
fn dial(st: St, r: &Reading, size: f32) -> impl Widget {
    let rot = unwrap(r.heading);
    let accent = Color::from_hex("#ef5350");
    let rose = Canvas::new(move |c, _| {
        let (cx, cy) = (size / 2.0, size / 2.0);
        let rad = size / 2.0 - 4.0;
        let fg = Color::from_hex("#d8dee9");
        c.set_color(fg.with_alpha(0.08));
        c.fill_circle(cx, cy, rad);
        for i in 0..72 {
            let a = (i as f32 * 5.0).to_radians() - std::f32::consts::FRAC_PI_2;
            let major = i % 6 == 0;
            let (r0, w) = if major { (rad - 16.0, 2.5) } else if i % 2 == 0 { (rad - 10.0, 1.5) } else { (rad - 7.0, 1.0) };
            c.set_stroke_width(w);
            c.set_color(if i == 0 { accent } else { fg.with_alpha(if major { 0.9 } else { 0.5 }) });
            c.draw_line(cx + a.cos() * r0, cy + a.sin() * r0, cx + a.cos() * rad, cy + a.sin() * rad);
        }
        // стрелка: север — красная, юг — светлая
        let (w, l) = (size * 0.06, size * 0.27);
        c.set_color(accent);
        c.fill_polygon(&[(cx, cy - l), (cx + w, cy), (cx - w, cy)]);
        c.set_color(fg.with_alpha(0.8));
        c.fill_polygon(&[(cx, cy + l), (cx - w, cy), (cx + w, cy)]);
    })
    .size(size, size);
    let mut labels = Stack::new().child(rose);
    let label_r = size / 2.0 - 40.0;
    for (deg, text, class) in [
        (0, syngui::i18n::t(n_!("С")), "rose-n"),
        (90, syngui::i18n::t(n_!("В")), "rose-main"),
        (180, syngui::i18n::t(n_!("Ю")), "rose-main"),
        (270, syngui::i18n::t(n_!("З")), "rose-main"),
    ]
    .into_iter()
    .chain([30, 60, 120, 150, 210, 240, 300, 330].into_iter().map(|d| (d, d.to_string(), "rose-num")))
    {
        let a = (deg as f32).to_radians() - std::f32::consts::FRAC_PI_2;
        let (bw, bh) = (44.0, 28.0);
        let (x, y) = (size / 2.0 + a.cos() * label_r - bw / 2.0, size / 2.0 + a.sin() * label_r - bh / 2.0);
        // подпись повёрнута вместе с картушкой: читается «по радиусу», как на настоящем компасе
        labels = labels.child(
            Positioned::new(
                DecoratedBox::new()
                    .child(Text::new(text).class(class))
                    .style("rotate", StyleValue::Number(deg as f32))
                    .class("rose-label"),
            )
            .at(x, y)
            .dimensions(bw, bh),
        );
    }
    let rotating = DecoratedBox::new().child(labels).style("rotate", StyleValue::Number(-rot)).class("rose");
    // неподвижное: метка курса сверху, метка направления, центр
    let target = st.target.get();
    let heading = r.heading;
    let fixed = Canvas::new(move |c, _| {
        // картушка ниже на 20 px — место для метки курса над ней
        let (cx, cy) = (size / 2.0, size / 2.0 + 20.0);
        let rad = size / 2.0 - 4.0;
        c.set_color(Color::from_hex("#eceff4"));
        c.fill_polygon(&[(cx, cy - rad - 2.0), (cx - 9.0, cy - rad - 18.0), (cx + 9.0, cy - rad - 18.0)]);
        if let Some(t) = target {
            let a = (t - heading).to_radians() - std::f32::consts::FRAC_PI_2;
            c.set_color(Color::from_hex("#ffb74d"));
            c.set_stroke_width(4.0);
            c.draw_line(cx + a.cos() * (rad - 22.0), cy + a.sin() * (rad - 22.0), cx + a.cos() * (rad + 2.0), cy + a.sin() * (rad + 2.0));
            c.fill_circle(cx + a.cos() * (rad - 28.0), cy + a.sin() * (rad - 28.0), 6.0);
        }
        c.set_color(Color::from_hex("#2e3440"));
        c.fill_circle(cx, cy, 9.0);
        c.set_color(Color::from_hex("#eceff4"));
        c.set_stroke_width(2.0);
        c.stroke_circle(cx, cy, 9.0);
    })
    .size(size, size + 20.0);
    let tap = GestureDetector::new().on_click(move || {
        // метка направления: поставить на текущий курс или снять
        let cur = match st.sensors.state.get_untracked() {
            State::Ok(r) => Some(r.heading),
            _ => None,
        };
        st.target.set(if st.target.get_untracked().is_some() { None } else { cur });
    });
    tap.child(
        Stack::new()
            .child(Positioned::new(rotating).at(0.0, 20.0).dimensions(size, size))
            .child(Positioned::new(fixed).at(0.0, 0.0).dimensions(size, size + 20.0))
            .child(DecoratedBox::new().style("width", StyleValue::px(size)).style("height", StyleValue::px(size + 20.0))),
    )
}

/// Уровень (пузырёк), поле, калибровка, метка направления, координаты, кнопки.
fn info(st: St, r: &Reading) -> impl Widget {
    let (pitch, roll) = (r.pitch, r.roll);
    let level = Canvas::new(move |c, _| {
        let s = 64.0;
        let (cx, cy) = (s / 2.0, s / 2.0);
        c.set_color(Color::from_hex("#d8dee9").with_alpha(0.12));
        c.fill_circle(cx, cy, s / 2.0 - 1.0);
        c.set_color(Color::from_hex("#d8dee9").with_alpha(0.4));
        c.set_stroke_width(1.0);
        c.stroke_circle(cx, cy, 10.0);
        // пузырёк уходит в сторону, обратную наклону; 30° — к краю
        let k = (s / 2.0 - 9.0) / 30.0;
        let (bx, by) = ((-roll).clamp(-30.0, 30.0) * k, (pitch).clamp(-30.0, 30.0) * k);
        let level = roll.abs() < 2.0 && pitch.abs() < 2.0;
        c.set_color(if level { Color::from_hex("#66bb6a") } else { Color::from_hex("#ffb74d") });
        c.fill_circle(cx + bx, cy + by, 8.0);
    })
    .size(64.0, 64.0);
    let tilt = t!("Наклон {p}° · {r}°", p = pitch.round() as i32, r = roll.round() as i32);
    let field_ok = (20.0..=75.0).contains(&r.field);
    let field = t!("Поле {v} мкТл", v = synshell_common::decimal(r.field as f64, 0));
    let (cal_text, cal_class) = match r.cal {
        Cal::Slpi => (t!("Откалиброван датчиком"), "chip ok"),
        Cal::Own => (t!("Своя калибровка"), "chip ok"),
        Cal::None => (t!("Нужна калибровка"), "chip warn"),
    };
    let mut col = Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(
            Row::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(level)
                .child(Column::new().gap(4.0).child(Text::new(tilt).class("value")).child(Text::new(field).class(if field_ok { "muted" } else { "warn-text" })).class("grow"))
                .child(DecoratedBox::new().child(Text::new(cal_text).class("chip-text")).class(cal_class)),
        );
    if !field_ok {
        col = col.child(Text::new(t!("Поле необычной силы: рядом магнит, металл или электроника — отойдите или откалибруйте компас.")).max_lines(3).class("warn-text"));
    }
    if let Some(t) = st.target.get() {
        let mut d = (t - r.heading + 540.0) % 360.0 - 180.0;
        if d.abs() < 0.5 {
            d = 0.0;
        }
        let dir = if d.abs() < 2.0 {
            t!("по курсу")
        } else if d > 0.0 {
            t!("правее на {d}°", d = d.round() as i32)
        } else {
            t!("левее на {d}°", d = (-d).round() as i32)
        };
        col = col.child(Text::new(t!("Метка {t}° {c} — {dir}", t = (t.round() as i32).rem_euclid(360), c = cardinal(t), dir = dir)).class("target-text"));
    }
    if let Some(f) = st.location.get() {
        let mut s = format!("{}  {}", dms(f.lat, &t!("с. ш."), &t!("ю. ш.")), dms(f.lon, &t!("в. д."), &t!("з. д.")));
        if let Some(a) = f.altitude {
            s.push_str(&t!(" · высота {a} м", a = a.round() as i64));
        }
        col = col.child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icons::PLACE).class("small-icon")).child(Text::new(s).selectable(true).class("muted")));
    }
    let target_on = st.target.get().is_some();
    col = col.child(
        Row::new()
            .gap(10.0)
            .main_axis_alignment(MainAxisAlignment::Center)
            .child(
                Button::new(if target_on { t!("Снять метку") } else { t!("Метка направления") })
                    .icon(icons::FLAG)
                    .on_click(move || {
                        let cur = match st.sensors.state.get_untracked() {
                            State::Ok(r) => Some(r.heading),
                            _ => None,
                        };
                        st.target.set(if st.target.get_untracked().is_some() { None } else { cur });
                    }),
            )
            .child(Button::new(t!("Калибровать")).icon(icons::CALIBRATE).on_click(move || st.sensors.start_calibration())),
    );
    DecoratedBox::new().child(col).class("info")
}

/// Калибровка «восьмёркой»: покрытие направлений, готово — когда покрыто большинство.
fn calibration(st: St) -> impl Widget {
    let c = st.sensors.collect.get().unwrap_or_default();
    let cov = c.coverage();
    let done = cov >= 0.8;
    let ring = {
        let v = cov.clamp(0.0, 1.0);
        Canvas::new(move |cv, _| {
            let s = 180.0;
            let (cx, cy) = (s / 2.0, s / 2.0);
            cv.set_stroke_width(12.0);
            cv.set_color(Color::from_hex("#d8dee9").with_alpha(0.15));
            cv.stroke_circle(cx, cy, s / 2.0 - 10.0);
            if v > 0.0 {
                cv.set_color(Color::from_hex(if v >= 0.8 { "#66bb6a" } else { "#4fc3f7" }));
                let a0 = -std::f32::consts::FRAC_PI_2;
                cv.draw_arc(cx, cy, s / 2.0 - 10.0, a0, a0 + v * std::f32::consts::TAU);
            }
        })
        .size(180.0, 180.0)
    };
    center(
        Column::new()
            .gap(16.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new(t!("Калибровка компаса")).class("title"))
            .child(Text::new(t!("Медленно поворачивайте телефон «восьмёркой» во всех направлениях — экраном вверх, вниз и на ребро, подальше от магнитов и металла.")).max_lines(4).class("muted center"))
            .child(Stack::new().child(ring).child(
                Column::new()
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(format!("{}%", (cov * 100.0).round() as i32)).class("heading-small"))
                    .child(Text::new(tn!(c.points.len(), "{n} точка", "{n} точки", "{n} точек")).class("muted"))
                    .style("width", StyleValue::px(180.0))
                    .style("height", StyleValue::px(180.0)),
            ))
            .child(
                Row::new()
                    .gap(10.0)
                    .child(Button::new(t!("Отмена")).on_click(move || st.sensors.collect.set(None)))
                    .child(Button::new(t!("Готово")).class("primary").disabled(!done).on_click(move || {
                        if !st.sensors.finish_calibration() {
                            tracing::warn!("калибровка: точек мало или они в одной плоскости");
                        }
                    })),
            )
            .class("calib"),
    )
}
