//! Окно «Пульта»: своя рамка (заголовок с кнопками окна, перетаскивание, изменение размера),
//! полноэкранный режим (кнопка, F11), боковая панель «Мои пульты» на широком окне и
//! одностраничная навигация с верхней панелью на узком (телефон).

mod add;
mod database;
mod home;
mod remote;

use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::GestureDetector;
use syngui::widgets::{EventHook, KeyReply};

use crate::icons;
use crate::state::{Page, St};

pub type W = Box<dyn Widget>;

pub fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

pub fn icon(g: &str) -> syngui::widget::StyledWidget<Icon> {
    Icon::new(g).class("ic")
}

pub fn tip(w: impl Widget + 'static, text: impl Into<String>) -> W {
    boxed(Tooltip::new(w, text).delay_ms(500))
}

pub fn icon_button(glyph: &str, tooltip: impl Into<String>, class: &str, on_click: impl FnMut() + Send + 'static) -> W {
    tip(Button::new("").icon(glyph).class(&format!("btn round flat {class}")).on_click(on_click), tooltip)
}

/// Широкое окно — боковая панель и рамка с кнопками окна.
fn wide() -> RwSignal<bool> {
    let narrow = viewport_below(720.0);
    let w = use_signal(!narrow.get_untracked());
    create_effect(move || {
        let v = !narrow.get();
        if w.get_untracked() != v {
            w.set(v);
        }
    });
    w
}

/// Заголовок экрана.
pub fn page_title(st: St, page: &Page) -> String {
    match page {
        Page::Home => t!("Пульт"),
        Page::Remote(id) => st.remote(id).map(|r| r.name).unwrap_or_else(|| t!("Пульт")),
        Page::Categories => t!("Новый пульт"),
        Page::Brands(c) => crate::db::category_info(c).0,
        Page::Models(_, b) => b.clone(),
        Page::Pick => t!("Подбор пульта"),
        Page::Found => t!("Пульт найден"),
        Page::Database => t!("База кодов"),
    }
}

fn brand_mark() -> W {
    boxed(DecoratedBox::new().class("brand").child(icon(icons::REMOTE).class("ic brand-ic")))
}

/// Индикатор передатчика: вспыхивает на каждую посылку.
fn emitter(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let on = st.flash.get().is_some();
        let class = match (st.ir.get().is_some(), on) {
            (_, true) => "led on",
            (true, false) => "led",
            (false, false) => "led off",
        };
        vec![tip(DecoratedBox::new().class(class), if st.ir.get_untracked().is_some() { t!("ИК-передатчик готов") } else { t!("ИК-передатчик не найден") })]
    }))
}

fn fullscreen_button(st: St, class: &'static str) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let fs = st.window.get().fullscreen;
        vec![icon_button(
            if fs { icons::FULLSCREEN_EXIT } else { icons::FULLSCREEN },
            if fs { t!("Выйти из полноэкранного режима (F11)") } else { t!("Во весь экран (F11)") },
            class,
            syngui::signal::toggle_fullscreen,
        )]
    }))
}

/// Заголовок широкого окна: знак, название экрана, индикатор, полный экран, кнопки окна.
fn titlebar(st: St) -> W {
    let title = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let page = st.page.get();
        let _ = st.remotes.get();
        let mut row = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).class("tb-title-row");
        row = row.child(Text::new(page_title(st, &page)).max_lines(1).elide(Elide::End).class("tb-title"));
        vec![boxed(row)]
    });
    let controls = Reactive::new(move || -> Vec<W> {
        let w = st.window.get();
        vec![boxed(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    boxed(
        DecoratedBox::new().class("titlebar").child(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(DecoratedBox::new().class("tb-left").child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(brand_mark()).child(Text::new(t!("Пульт")).class("brand-title"))))
                .child(DecoratedBox::new().class("grow drag-wrap").child(WindowDragRegion::new().child(DecoratedBox::new().class("drag-space").child(title))))
                .child(DecoratedBox::new().class("tb-tools").child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(emitter(st)).child(fullscreen_button(st, "tb-btn"))))
                .child(DecoratedBox::new().class("window-controls").child(controls)),
        ),
    )
}

/// Верхняя панель узкого окна: назад, название, индикатор, действия экрана, полный экран.
fn appbar(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let page = st.page.get();
        let _ = st.remotes.get();
        let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if page == Page::Home {
            row = row.child(DecoratedBox::new().class("appbar-mark").child(brand_mark()));
        } else {
            row = row.child(icon_button(icons::BACK, t!("Назад"), "tb-btn", move || {
                st.back();
            }));
        }
        let title = Text::new(page_title(st, &page)).max_lines(1).elide(Elide::End).class("appbar-title");
        row = row.child(DecoratedBox::new().class("grow").child(WindowDragRegion::new().child(DecoratedBox::new().class("appbar-drag").child(title))));
        row = row.child(emitter(st));
        if page == Page::Home {
            row = row.child(icon_button(icons::STORAGE, t!("База кодов"), "tb-btn", move || st.go(Page::Database)));
        }
        row = row.child(fullscreen_button(st, "tb-btn"));
        vec![boxed(DecoratedBox::new().class("appbar").child(row))]
    }))
}

/// Боковая панель широкого окна: мои пульты, добавить, база кодов.
fn sidebar(st: St) -> W {
    let list = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let page = st.page.get();
        let remotes = st.remotes.get();
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        if remotes.is_empty() {
            col = col.child(Text::new(t!("Пультов пока нет")).class("hint side-empty"));
        }
        for r in remotes {
            let on = page == Page::Remote(r.id.clone());
            let id = r.id.clone();
            let (_, glyph, _) = crate::db::category_info(&r.category);
            col = col.child(
                GestureDetector::new().on_click(move || st.go(Page::Remote(id.clone()))).child(
                    DecoratedBox::new().class(if on { "side-item on" } else { "side-item" }).child(
                        Row::new()
                            .gap(10.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(icon(glyph).class("ic side-ic"))
                            .child(Column::new().gap(1.0).child(Text::new(r.name.clone()).max_lines(1).elide(Elide::End).class("side-title")).child(Text::new(r.model.clone()).max_lines(1).elide(Elide::End).class("hint"))),
                    ),
                ),
            );
        }
        vec![boxed(col)]
    });
    let foot = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let page = st.page.get();
        let db_on = page == Page::Database;
        vec![boxed(
            GestureDetector::new().on_click(move || st.go(Page::Database)).child(
                DecoratedBox::new().class(if db_on { "side-item on" } else { "side-item" }).child(
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(icon(icons::STORAGE).class("ic side-ic"))
                        .child(Text::new(t!("База кодов")).class("side-title")),
                ),
            ),
        )]
    });
    boxed(
        DecoratedBox::new().class("sidebar").child(
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(Button::new(t!("Добавить пульт")).icon(icons::ADD).class("btn new").on_click(move || st.go(Page::Categories)))
                .child(Text::new(t!("Мои пульты")).class("section side-section"))
                .child(DecoratedBox::new().class("grow").child(ScrollView::new().vertical().child(list)))
                .child(foot),
        ),
    )
}

/// Содержимое текущего экрана.
fn content(st: St, wide: bool) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let page = st.page.get();
        vec![match page {
            Page::Home => home::page(st, wide),
            Page::Remote(id) => remote::page(st, id),
            Page::Categories => add::categories(st),
            Page::Brands(c) => add::brands(st, c),
            Page::Models(c, b) => add::models(st, c, b),
            Page::Pick => add::pick(st),
            Page::Found => add::found(st),
            Page::Database => database::page(st),
        }]
    }))
}

fn toast_layer(st: St) -> W {
    let toast = Reactive::new(move || -> Vec<W> {
        let Some((_, text)) = st.toast.get() else { return vec![] };
        vec![boxed(DecoratedBox::new().class("toast").child(Text::new(text).max_lines(3).class("toast-text")))]
    });
    boxed(Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center).class("toast-wrap").child(toast))
}

/// Полноэкранный режим: вместо заголовка — тонкая строка с «назад», индикатором и «выйти».
fn fullscreen_bar(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let home = st.page.get() == Page::Home;
        let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).class("fs-bar");
        if !home {
            row = row.child(icon_button(icons::BACK, t!("Назад"), "fs-btn", move || {
                st.back();
            }));
        }
        row = row.child(DecoratedBox::new().class("grow"));
        row = row.child(emitter(st));
        row = row.child(icon_button(icons::FULLSCREEN_EXIT, t!("Выйти из полноэкранного режима (F11)"), "fs-btn", syngui::signal::toggle_fullscreen));
        vec![boxed(row)]
    }))
}

/// Клавиатура: F11 — полный экран, Esc — назад или выход из полного экрана; на пульте — стрелки,
/// Enter, +/−, PageUp/PageDown, цифры, M — без звука.
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
                    if st.menu_open.get_untracked() {
                        st.menu_open.set(false);
                        return KeyReply::Handled;
                    }
                    if st.window.get_untracked().fullscreen {
                        syngui::signal::set_fullscreen(false);
                        return KeyReply::Handled;
                    }
                    return if st.back() { KeyReply::Handled } else { KeyReply::Ignore };
                }
                if m.ctrl || m.alt {
                    return KeyReply::Ignore;
                }
                if let Page::Remote(id) = st.page.get_untracked() {
                    let key = match k {
                        Key::Up => "up",
                        Key::Down => "down",
                        Key::Left => "left",
                        Key::Right => "right",
                        Key::Enter => "ok",
                        Key::Backspace => "back",
                        Key::PageUp => "ch_up",
                        Key::PageDown => "ch_dn",
                        Key::M => "mute",
                        Key::P => "power",
                        Key::H => "home",
                        Key::I => "source",
                        Key::Space | Key::MediaPlayPause => "play_pause",
                        Key::Num0 => "0",
                        Key::Num1 => "1",
                        Key::Num2 => "2",
                        Key::Num3 => "3",
                        Key::Num4 => "4",
                        Key::Num5 => "5",
                        Key::Num6 => "6",
                        Key::Num7 => "7",
                        Key::Num8 => "8",
                        Key::Num9 => "9",
                        _ => return KeyReply::Ignore,
                    };
                    if let Some(r) = st.remote(&id) {
                        let sig = crate::keys::find(&r.signals, key).or_else(|| if key == "play_pause" { crate::keys::find(&r.signals, "play") } else { None });
                        if let Some(sig) = sig {
                            st.send(sig);
                            return KeyReply::Handled;
                        }
                    }
                }
                KeyReply::Ignore
            })
            .on_char(move |c| {
                let Page::Remote(id) = st.page.get_untracked() else { return false };
                let key = match c {
                    '+' | '=' => "vol_up",
                    '-' | '_' => "vol_dn",
                    _ => return false,
                };
                if let Some(sig) = st.remote(&id).and_then(|r| crate::keys::find(&r.signals, key).cloned()) {
                    st.send(&sig);
                    return true;
                }
                false
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
        let body: W = if fs {
            boxed(DecoratedBox::new().class("grow body fs").child(content(st, w)))
        } else if w {
            boxed(Row::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow").child(sidebar(st)).child(DecoratedBox::new().class("grow body").child(content(st, w))))
        } else {
            boxed(DecoratedBox::new().class("grow body").child(content(st, w)))
        };
        let mut main = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class(if w { "window" } else { "window phone" });
        main = main.child(if fs { fullscreen_bar(st) } else if w { titlebar(st) } else { appbar(st) });
        main = main.child(body);
        let stack = Stack::new().fit(StackFit::Expand).child(main).child(remote::menu_layer(st)).child(toast_layer(st)).child(remote::dialogs(st));
        let frame = DecoratedBox::new().class(if w { "window-frame" } else { "window-frame flat" }).clip(true).child(keys(st, stack));
        vec![boxed(WindowResizeRegion::new().over_content(true).inset(5.0).enabled(w && !fs).child(frame))]
    }))
}
