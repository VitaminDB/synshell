//! Окно «Цикла»: своя рамка (заголовок с кнопками окна, перетаскивание, изменение размера),
//! полноэкранный режим (по умолчанию, F11), боковая панель на широком окне и нижняя
//! навигация на узком (телефон); экраны перетекают друг в друга.

mod calendar;
mod day;
mod history;
mod intro;
mod settings;
mod today;

use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::widgets::{EventHook, KeyReply};
use syngui::GestureDetector;

use crate::icons;
use crate::state::{St, Tab};

pub type W = Box<dyn Widget>;

pub fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

pub fn icon(g: &str) -> syngui::widget::StyledWidget<Icon> {
    Icon::new(g).class("ic")
}

pub fn icon_button(glyph: &str, class: &str, on_click: impl FnMut() + Send + 'static) -> W {
    boxed(Button::new("").icon(glyph).class(&format!("btn round flat {class}")).on_click(on_click))
}

pub fn tab_title(t: Tab) -> String {
    match t {
        Tab::Today => t!("Сегодня"),
        Tab::Calendar => t!("Календарь"),
        Tab::History => t!("Мой цикл"),
        Tab::Settings => t!("Настройки"),
    }
}

fn tab_icon(t: Tab) -> &'static str {
    match t {
        Tab::Today => icons::FLOWER,
        Tab::Calendar => icons::CALENDAR,
        Tab::History => icons::INSIGHTS,
        Tab::Settings => icons::SETTINGS,
    }
}

/// Широкое окно — боковая панель и рамка с кнопками окна.
fn wide() -> RwSignal<bool> {
    let narrow = viewport_below(760.0);
    let w = use_signal(!narrow.get_untracked());
    create_effect(move || {
        let v = !narrow.get();
        if w.get_untracked() != v {
            w.set(v);
        }
    });
    w
}

/// Ширина колонки содержимого.
pub fn content_width(wide: bool) -> f32 {
    let vp = viewport_size().get();
    let avail = if wide { vp.width - 250.0 - 48.0 } else { vp.width - 32.0 };
    avail.clamp(260.0, 620.0)
}

fn brand_mark() -> W {
    boxed(DecoratedBox::new().class("brand").child(icon(icons::DROP).class("ic brand-ic")))
}

pub fn fullscreen_button(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let fs = st.window.get().fullscreen;
        vec![icon_button(if fs { icons::FULLSCREEN_EXIT } else { icons::FULLSCREEN }, "tb-btn", syngui::signal::toggle_fullscreen)]
    }))
}

/// Заголовок страницы: дата или подпись сверху, крупное название, справа — действия.
pub fn page_header(st: St, wide: bool, over: String, title: String, actions: Vec<W>) -> W {
    let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
    let texts = Column::new().gap(2.0).child(Text::new(over).max_lines(1).class("over")).child(Text::new(title).max_lines(1).class("h1"));
    row = row.child(DecoratedBox::new().class("grow").child(WindowDragRegion::new().child(DecoratedBox::new().class("header-drag").child(texts))));
    for a in actions {
        row = row.child(a);
    }
    let fs = st.window.get_untracked().fullscreen;
    if !wide || fs {
        row = row.child(fullscreen_button(st));
    }
    boxed(DecoratedBox::new().class("page-header").child(row))
}

/// Прокручиваемая страница с колонкой по центру.
pub fn page(wide: bool, children: Vec<W>) -> W {
    let w = content_width(wide);
    let mut col = Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for c in children {
        col = col.child(c);
    }
    boxed(
        ScrollView::new().vertical().class("page-scroll").child(
            Column::new().cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class(if wide { "page wide" } else { "page" }).style("width", StyleValue::px(w + 32.0)).child(col)),
        ),
    )
}

/// Появление блока страницы: подъём и проявление с задержкой по порядку.
pub fn rise(i: u32, w: impl Widget + 'static) -> W {
    boxed(
        Animated::new(w)
            .opacity(Animation::tween(Easing::EaseOutCubic).from(0.0).to(1.0).duration_ms(360).delay_ms(40 + i * 55))
            .translate_y(Animation::tween(Easing::EaseOutCubic).from(18.0).to(0.0).duration_ms(420).delay_ms(40 + i * 55)),
    )
}

fn titlebar(st: St) -> W {
    let controls = Reactive::new(move || -> Vec<W> {
        let w = st.window.get();
        vec![boxed(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    boxed(
        DecoratedBox::new().class("titlebar").child(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(DecoratedBox::new().class("tb-left").child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(brand_mark()).child(Text::new(t!("Цикл")).class("brand-title"))))
                .child(DecoratedBox::new().class("grow drag-wrap").child(WindowDragRegion::new().child(DecoratedBox::new().class("drag-space"))))
                .child(DecoratedBox::new().class("tb-tools").child(fullscreen_button(st)))
                .child(DecoratedBox::new().class("window-controls").child(controls)),
        ),
    )
}

fn sidebar(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let cur = st.tab.get();
        let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for t in Tab::ALL {
            let on = t == cur;
            col = col.child(
                GestureDetector::new().on_click(move || st.go(t)).child(
                    DecoratedBox::new().class(if on { "side-item on" } else { "side-item" }).child(
                        Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(tab_icon(t)).class("ic side-ic")).child(Text::new(tab_title(t)).class("side-title")),
                    ),
                ),
            );
        }
        col = col.child(DecoratedBox::new().class("grow"));
        col = col.child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(icons::LOCK).class("ic tiny-ic")).child(Text::new(t!("Данные хранятся только на устройстве")).max_lines(2).class("hint side-note")));
        vec![boxed(DecoratedBox::new().class("sidebar").child(col))]
    }))
}

fn bottom_nav(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let cur = st.tab.get();
        let mut row = Row::new().main_axis_alignment(MainAxisAlignment::SpaceAround).cross_axis_alignment(CrossAxisAlignment::Center);
        for t in Tab::ALL {
            let on = t == cur;
            row = row.child(
                GestureDetector::new().on_click(move || st.go(t)).child(
                    DecoratedBox::new().class("nav-item").child(
                        Column::new()
                            .gap(4.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(DecoratedBox::new().class(if on { "nav-pill on" } else { "nav-pill" }).child(icon(tab_icon(t)).class(if on { "ic nav-ic on" } else { "ic nav-ic" })))
                            .child(Text::new(tab_title(t)).max_lines(1).class(if on { "nav-label on" } else { "nav-label" })),
                    ),
                ),
            );
        }
        vec![boxed(DecoratedBox::new().class("bottom-nav").child(row))]
    }))
}

/// Текущий экран: смена вкладки — перетекание вбок по направлению.
fn content(st: St, wide: bool) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        if !st.data.get().onboarded {
            return vec![intro::page(st, wide)];
        }
        let tab = st.tab.get();
        vec![boxed(
            AnimatedSwitcher::new(tab.index(), move || -> W {
                syngui::i18n::subscribe();
                match st.tab.get_untracked() {
                    Tab::Today => today::page(st, wide),
                    Tab::Calendar => calendar::page(st, wide),
                    Tab::History => history::page(st, wide),
                    Tab::Settings => settings::page(st, wide),
                }
            })
            .slide(36.0, 0.0)
            .directional(true)
            .duration_ms(320)
            .exit_duration_ms(140)
            .animate_size(false)
            .class("grow"),
        )]
    }))
}

fn toast_layer(st: St) -> W {
    let toast = Reactive::new(move || -> Vec<W> {
        let cur = st.toast.get();
        let shown = cur.is_some();
        let text = cur.map(|(_, t)| t).unwrap_or_default();
        vec![boxed(
            Presence::new(shown, DecoratedBox::new().class("toast").child(Text::new(text).max_lines(3).class("toast-text")))
                .enter(Motion::fade().slide(0.0, 24.0).scale(0.96))
                .exit(Motion::fade().slide(0.0, 12.0))
                .duration_ms(260)
                .initial(false),
        )]
    });
    boxed(Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center).class("toast-wrap").child(toast))
}

/// Клавиатура: F11 — полный экран, Esc — назад или выход из полного экрана,
/// ←/→ — месяцы календаря, 1–4 — вкладки.
fn keys(st: St, inner: impl Widget + 'static) -> W {
    boxed(
        EventHook::new()
            .on_back(move || st.back())
            .on_key_down(move |k, m| {
                if k == Key::F11 {
                    syngui::signal::toggle_fullscreen();
                    return KeyReply::Handled;
                }
                if k == Key::Escape {
                    if st.back() {
                        return KeyReply::Handled;
                    }
                    if st.window.get_untracked().fullscreen {
                        syngui::signal::set_fullscreen(false);
                        return KeyReply::Handled;
                    }
                    return KeyReply::Ignore;
                }
                if m.ctrl || m.alt || !st.data.get_untracked().onboarded {
                    return KeyReply::Ignore;
                }
                match k {
                    Key::Num1 => st.go(Tab::Today),
                    Key::Num2 => st.go(Tab::Calendar),
                    Key::Num3 => st.go(Tab::History),
                    Key::Num4 => st.go(Tab::Settings),
                    Key::Left if st.tab.get_untracked() == Tab::Calendar => st.shift_month(-1),
                    Key::Right if st.tab.get_untracked() == Tab::Calendar => st.shift_month(1),
                    _ => return KeyReply::Ignore,
                }
                KeyReply::Handled
            })
            .child(inner),
    )
}

pub fn root(st: St) -> W {
    let wide = wide();
    // окно перестраивается только при смене полноэкранного режима, а не фокуса
    let full = use_signal(false);
    create_effect(move || {
        let v = st.window.get().fullscreen;
        if full.get_untracked() != v {
            full.set(v);
        }
    });
    boxed(Reactive::new(move || -> Vec<W> {
        let w = wide.get();
        let fs = full.get();
        let onboarded = st.data.get().onboarded;
        let mut main = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class(if w { "window" } else { "window phone" });
        if w && !fs {
            main = main.child(titlebar(st));
        }
        let body: W = if w && onboarded {
            boxed(Row::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow").child(sidebar(st)).child(DecoratedBox::new().class("grow body").child(content(st, w))))
        } else {
            let mut col = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow").child(DecoratedBox::new().class("grow body").child(content(st, w)));
            if onboarded {
                col = col.child(bottom_nav(st));
            }
            boxed(col)
        };
        main = main.child(body);
        let stack = Stack::new().fit(StackFit::Expand).child(main).child(day::layer(st, w)).child(settings::reset_dialog(st)).child(toast_layer(st));
        let frame = DecoratedBox::new().class(if w && !fs { "window-frame" } else { "window-frame flat" }).clip(true).child(keys(st, stack));
        vec![boxed(WindowResizeRegion::new().over_content(true).inset(5.0).enabled(w && !fs).child(frame))]
    }))
}
