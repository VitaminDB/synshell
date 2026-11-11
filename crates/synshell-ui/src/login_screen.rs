//! Экран входа и блокировки — один компонент, как тема SDDM = экран блокировки в KDE: им рисуют
//! блокировку сеанса (телефон), её предпросмотр (`shell lock-preview`) и экран входа `synlogin greeter`.
//!
//! Обложка: часы, дата, уведомления (у блокировки), подсказка «Проведите вверх»; свайп вверх (или
//! тап) — цифровая панель PIN или пароль с клавиатурой — та же, что демон synkeyboard
//! ([`syn_keyboard::embedded`]). Состояние (ступень, набранное) — своё у каждого экрана: предпросмотр
//! не делит его с настоящей блокировкой. Вид — `styles/login.mss` ([`crate::theme::LOGIN_STYLE`]) и
//! переменные темы пользователя (у экрана входа — из [`synshell_common::login_theme`]).

use std::sync::Arc;

use syngui::prelude::*;

use crate::notifications::Notification;
use crate::ui::{icon, mi};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    /// Часы, уведомления, «Проведите вверх».
    Cover,
    /// Цифровая панель.
    Pin,
    /// Пароль с клавиатурой.
    Text,
}

/// Что показать и что делать — у блокировки, предпросмотра и экрана входа своё.
#[derive(Clone)]
pub struct Spec {
    /// Имя над полем ввода.
    pub name: String,
    /// Часы (секунды Unix).
    pub now: RwSignal<i64>,
    /// Уведомления на обложке (у блокировки сеанса).
    pub notes: Option<RwSignal<Vec<Notification>>>,
    /// Идёт проверка пароля.
    pub checking: RwSignal<bool>,
    pub error: RwSignal<String>,
    /// Счётчик неудачных попыток — набранное стирается.
    pub attempt: RwSignal<u32>,
    /// Проверить пароль (блокировка — PAM и снятие, экран входа — вход).
    pub submit: Arc<dyn Fn(String) + Send + Sync>,
    /// Свайп вверх или тап по обложке: `true` — экран снят без пароля (метод «свайп», нет пароля,
    /// предпросмотр), иначе открывается ввод.
    pub swipe: Arc<dyn Fn() -> bool + Send + Sync>,
    /// Дополнительная кнопка под вводом (экран входа: «Другой пользователь»).
    pub extra: Option<(String, Arc<dyn Fn() + Send + Sync>)>,
    /// Низ обложки под подсказкой (экран входа: выключение, перезагрузка, масштаб).
    pub footer: Option<Arc<dyn Fn() -> Box<dyn Widget> + Send + Sync>>,
    /// Полоса над часами и над вводом (блокировка: идущий разговор).
    pub banner: Option<Arc<dyn Fn() -> Box<dyn Widget> + Send + Sync>>,
    /// Фон: обои.
    pub background: Arc<dyn Fn() -> Box<dyn Widget> + Send + Sync>,
    pub initial: Stage,
    /// Длительность смены ступени, мс.
    pub anim_ms: u32,
}

/// Экран входа или блокировки во весь экран.
pub fn view(spec: Spec) -> impl Widget {
    use syngui::widgets::{PanAxis, SwipeDirection};
    use syngui::GestureDetector;
    let stage = use_signal(spec.initial);
    let pin = use_signal(String::new());
    let kb = password_keyboard(pin, spec.submit.clone());
    let dur = spec.anim_ms;
    // Раскладка — снаружи реактивных узлов: Reactive/AnimatedSwitcher отдают детям свободные
    // ограничения (натуральный размер, прижаты влево-вверх), и `grow`/выравнивание внутри них не
    // работают. Поэтому слои на весь экран — здесь: верх (часы, уведомления) и низ (подсказка /
    // цифры / клавиатура) раздельно, оба по центру.
    let part = {
        let spec = spec.clone();
        move |top: bool| {
            let spec = spec.clone();
            crate::ui::rx(move || {
                let s = stage.get();
                let key = match s {
                    Stage::Cover => 1u64,
                    Stage::Pin => 2,
                    Stage::Text => 3,
                };
                let spec = spec.clone();
                Box::new(
                    AnimatedSwitcher::new(key, move || -> Box<dyn Widget> {
                        match (s, top) {
                            (Stage::Cover, true) => Box::new(cover_top(&spec)),
                            (Stage::Cover, false) => Box::new(cover_bottom(&spec)),
                            (_, true) => Box::new(Column::new()),
                            (Stage::Pin, false) => Box::new(pin_pad(&spec, stage, pin)),
                            (Stage::Text, false) => Box::new(text_entry(&spec, stage, pin, kb)),
                        }
                    })
                    .directional(true)
                    .slide(0.0, 60.0)
                    .duration_ms(dur)
                    .exit_duration_ms(dur * 2 / 3)
                    .animate_size(false),
                )
            })
        }
    };
    // Ошибка пароля — снова пустой ввод.
    let attempt = spec.attempt;
    create_effect(move || {
        let _ = attempt.get();
        pin.set(String::new());
    });
    let (swipe, swipe2) = (spec.swipe.clone(), spec.swipe.clone());
    let gestures = GestureDetector::new()
        .pan_axis(PanAxis::Vertical)
        .on_swipe(move |dir, _| match dir {
            SwipeDirection::Up if stage.get_untracked() == Stage::Cover => {
                if !swipe() {
                    stage.set(Stage::Pin);
                }
            }
            SwipeDirection::Down => stage.set(Stage::Cover),
            _ => {}
        })
        .on_click(move || {
            if stage.get_untracked() == Stage::Cover && !swipe2() {
                stage.set(Stage::Pin);
            }
        })
        .child(
            // Два слоя, а не одна колонка SpaceBetween: у цифровой панели верх пустой, и
            // единственный блок встал бы в начало.
            Stack::new()
                .fit(StackFit::Expand)
                .child({
                    let mut top = Column::new().cross_axis_alignment(CrossAxisAlignment::Center);
                    if let Some(b) = &spec.banner {
                        top = top.child(b());
                    }
                    top.child(part(true)).class("lock-phone")
                })
                .child(
                    Column::new()
                        .main_axis_alignment(MainAxisAlignment::End)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(part(false))
                        .class("lock-phone"),
                ),
        );
    Stack::new()
        .fit(StackFit::Expand)
        .child((spec.background)())
        .child(DecoratedBox::new().class("lock-scrim"))
        .child(gestures)
        // Буква над пальцем и варианты долгого нажатия клавиатуры пароля — во весь экран.
        .child(syn_keyboard::overlay(kb))
}

/// Фон из файла обоев (`mode` — как `[wallpaper] mode`) поверх цвета темы (`.wallpaper-color`).
pub fn wallpaper_background(path: String, mode: String) -> Box<dyn Widget> {
    let fit = match mode.as_str() {
        "fit" => ImageFit::Contain,
        "stretch" => ImageFit::Fill,
        "center" | "tile" => ImageFit::None,
        _ => ImageFit::Cover,
    };
    let mut s = Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().class("wallpaper-color"));
    if !path.is_empty() && std::path::Path::new(&path).is_file() {
        s = s.child(Image::new(path).fit(fit));
    }
    Box::new(s)
}

/// Клавиатура ввода пароля — та же, что у демона synkeyboard: ряды, слои, долгие нажатия;
/// набранное — в `pin`, Enter — `submit`.
fn password_keyboard(pin: RwSignal<String>, submit: Arc<dyn Fn(String) + Send + Sync>) -> syn_keyboard::ui::Keyboard {
    use syn_keyboard::Typed;
    syn_keyboard::embedded(move |t| match t {
        Typed::Text(s) => pin.update(|p| p.push_str(&s)),
        Typed::Key("space") => pin.update(|p| p.push(' ')),
        Typed::Key("backspace") => pin.update(|p| {
            p.pop();
        }),
        Typed::Key("enter") => submit(pin.get_untracked()),
        Typed::Key(_) => {}
    })
}

fn cover_top(spec: &Spec) -> impl Widget {
    let now = spec.now;
    let clock = crate::ui::rx(move || {
        let now = now.get();
        syngui::i18n::subscribe();
        Box::new(
            Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(crate::clock::format(now - now % 60, "%H:%M")).class("lock-phone-time"))
                .child(Text::new(crate::clock::format(now, "%A, %e %B")).class("lock-date")),
        )
    });
    let mut col = Column::new().gap(28.0).cross_axis_alignment(CrossAxisAlignment::Center).child(clock);
    if let Some(history) = spec.notes {
        col = col.child(crate::ui::rx(move || {
            let list = history.get();
            // Ширина экрана без полей .lock-phone: в свободных ограничениях карточки иначе ужались
            // бы до своего текста.
            let w = (syngui::viewport::viewport_size().get().width - 48.0).max(0.0);
            let mut col = Column::new().gap(8.0);
            for n in list.iter().rev().take(4) {
                col = col.child(
                    DecoratedBox::new()
                        .child(
                            Column::new()
                                .gap(2.0)
                                .child(Text::new(n.summary.clone()).max_lines(1).class("lock-note-title"))
                                .child(Text::new(n.body.clone()).max_lines(2).class("lock-note-body")),
                        )
                        .class("lock-note")
                        .style("width", w),
                );
            }
            Box::new(col)
        }));
    }
    col.class("lock-cover")
}

fn cover_bottom(spec: &Spec) -> impl Widget {
    let mut col = Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(icon(mi::LOCK).class("lock-phone-lock"))
        .child(Text::new(t!("Проведите вверх, чтобы разблокировать")).class("lock-hint"));
    if let Some(f) = &spec.footer {
        col = col.child(DecoratedBox::new().class("lock-footer-gap")).child(f());
    }
    col
}

fn pill(label: String, f: impl FnMut() + Send + 'static) -> impl Widget {
    syngui::GestureDetector::new().on_click(f).child(DecoratedBox::new().child(Text::new(label).class("lock-pill-text")).class("lock-pill"))
}

fn pin_pad(spec: &Spec, stage: RwSignal<Stage>, pin: RwSignal<String>) -> impl Widget {
    use syngui::GestureDetector;
    let (checking, error) = (spec.checking, spec.error);
    let dots = crate::ui::rx(move || {
        let n = pin.get().chars().count();
        let checking = checking.get();
        let mut row = Row::new().gap(14.0).main_axis_alignment(MainAxisAlignment::Center);
        for i in 0..n.max(4) {
            let class = if i < n { "lock-dot lock-dot-on" } else { "lock-dot" };
            row = row.child(DecoratedBox::new().class(if checking { "lock-dot lock-dot-checking" } else { class }));
        }
        Box::new(row)
    });
    let error = crate::ui::rx(move || Box::new(Text::new(error.get()).class("lock-error")));
    let key = move |label: &'static str, sub: &'static str| {
        GestureDetector::new()
            .on_press(move |_| {
                if checking.get_untracked() {
                    return;
                }
                let mut p = pin.get_untracked();
                if p.len() < 32 {
                    p.push_str(label);
                    pin.set(p);
                }
            })
            .child(
                DecoratedBox::new()
                    .child(
                        Column::new()
                            .gap(0.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(Text::new(label).class("lock-key-digit"))
                            // Пустая подпись («1») — неразрывный пробел: цифра на одной высоте с остальными.
                            .child(Text::new(if sub.is_empty() { "\u{a0}" } else { sub }).class("lock-key-sub")),
                    )
                    .class("lock-key"),
            )
    };
    let action = move |glyph: &'static str, f: Box<dyn Fn() + Send + Sync>| {
        GestureDetector::new().on_press(move |_| f()).child(
            DecoratedBox::new()
                .child(crate::ui::vcenter(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(icon(glyph).class("lock-key-icon"))))
                .class("lock-key lock-key-action"),
        )
    };
    let mut grid = Grid::new(3).gap(14.0);
    for (d, sub) in [("1", ""), ("2", "ABC"), ("3", "DEF"), ("4", "GHI"), ("5", "JKL"), ("6", "MNO"), ("7", "PQRS"), ("8", "TUV"), ("9", "WXYZ")] {
        grid = grid.child(key(d, sub));
    }
    let submit = spec.submit.clone();
    grid = grid
        .child(action("\u{E14A}", Box::new(move || {
            let mut p = pin.get_untracked();
            p.pop();
            pin.set(p);
        })))
        .child(key("0", "+"))
        .child(action("\u{E5CA}", Box::new(move || submit(pin.get_untracked()))));
    let mut pills = Row::new().gap(12.0).child(pill("ABC".into(), move || stage.set(Stage::Text))).child(pill(t!("Отмена").into(), move || stage.set(Stage::Cover)));
    if let Some((label, f)) = spec.extra.clone() {
        pills = pills.child(pill(label, move || f()));
    }
    Column::new()
        .gap(22.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .main_axis_alignment(MainAxisAlignment::End)
        .child(Text::new(spec.name.clone()).class("lock-user"))
        .child(Text::new(t!("Введите PIN-код или пароль")).class("lock-hint"))
        .child(dots)
        .child(error)
        .child(grid.class("lock-pad"))
        .child(pills)
        .class("lock-pin")
}

fn text_entry(spec: &Spec, stage: RwSignal<Stage>, pin: RwSignal<String>, kb: syn_keyboard::ui::Keyboard) -> impl Widget {
    let (checking, error) = (spec.checking, spec.error);
    let dots = crate::ui::rx(move || {
        let n = pin.get().chars().count();
        Box::new(if checking.get() {
            Text::new(t!("Проверка…")).class("lock-text-hint")
        } else if n == 0 {
            Text::new(t!("Пароль")).class("lock-text-hint")
        } else {
            Text::new("•".repeat(n)).class("lock-text-dots")
        })
    });
    let error = crate::ui::rx(move || Box::new(Text::new(error.get()).class("lock-error")));
    let mut pills = Row::new().gap(12.0).child(pill("123".into(), move || stage.set(Stage::Pin))).child(pill(t!("Отмена").into(), move || stage.set(Stage::Cover)));
    if let Some((label, f)) = spec.extra.clone() {
        pills = pills.child(pill(label, move || f()));
    }
    Column::new()
        .gap(16.0)
        .main_axis_alignment(MainAxisAlignment::End)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(spec.name.clone()).class("lock-user"))
        .child(DecoratedBox::new().child(dots).class("lock-text-box"))
        .child(error)
        .child(pills)
        .child(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(syn_keyboard::keys(kb)).style("width", (syngui::viewport::viewport_size().get_untracked().width - 48.0).max(280.0)))
        .class("lock-pin lock-pin-text")
}
