//! Живой предпросмотр: настоящие классы оболочки и «Параметров системы» в
//! теме-черновике (области `.te-scope-shell`/`.te-scope-settings`, см.
//! `preview_style`). В режиме пипетки щелчок по элементу показывает его
//! токены и правила в инспекторе.

use syngui::prelude::*;
use syngui::widgets::buttons::Segment;
use syngui::GestureDetector;
use synshell_common::t;

use super::{boxed, icon, tip, W};
use crate::icons;
use crate::state::{Mode, Pick, Scene, St};

fn ic(g: &str, class: &str) -> W {
    boxed(Icon::new(g).class(class))
}

fn txt(s: impl Into<String>, class: &str) -> W {
    boxed(Text::new(s.into()).max_lines(1).class(class))
}

// ── Рабочий стол ────────────────────────────────────────────────────────

fn ws(n: &str, active: bool) -> W {
    boxed(
        DecoratedBox::new().class(if active { "ws ws-active" } else { "ws" }).child(
            Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(n).class("ws-label")).child(DecoratedBox::new().class("ws-dot")),
        ),
    )
}

fn task(g: &str, title: String, active: bool) -> W {
    boxed(DecoratedBox::new().class(if active { "task task-active" } else { "task" }).child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(ic(g, "icon task-glyph")).child(txt(title, "task-title"))))
}

fn panel_row(st: St) -> W {
    boxed(
        Row::new()
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("panel-content")
            .child(pick(st, t!("Кнопка меню"), &["fg", "hover"], &[".applet", ".applet-launcher .icon"], || DecoratedBox::new().class("applet applet-launcher").child(Icon::new(icons::APPS).class("icon"))))
            .child(pick(st, t!("Рабочие столы"), &["accent-soft", "fg", "muted", "accent"], &[".ws", ".ws-active", ".ws-label", ".ws-dot", ".ws-active .ws-dot"], || {
                DecoratedBox::new().class("applet applet-workspaces").child(Row::new().gap(2.0).child(ws("1", true)).child(ws("2", false)).child(ws("3", false)))
            }))
            .child(pick(st, t!("Задачи"), &["accent-soft", "hover", "fg", "muted"], &[".task", ".task-active", ".task-title"], || {
                DecoratedBox::new()
                    .class("applet-slot-taskbar")
                    .child(Row::new().gap(4.0).child(task(icons::TERMINAL, t!("Терминал"), true)).child(task(icons::FOLDER, t!("Проводник"), false)).child(task(icons::WEB, t!("Браузер"), false)))
            }))
            .child(DecoratedBox::new().class("applet-spacer"))
            .child(pick(st, t!("Значки в трее"), &["fg", "hover"], &[".tray-item", ".icon"], || {
                DecoratedBox::new().class("applet applet-tray").child(
                    Row::new()
                        .gap(2.0)
                        .child(DecoratedBox::new().class("tray-item").child(Icon::new(icons::WIFI).class("icon")))
                        .child(DecoratedBox::new().class("tray-item").child(Icon::new(icons::VOLUME).class("icon")))
                        .child(DecoratedBox::new().class("tray-item").child(Icon::new(icons::BATTERY).class("icon"))),
                )
            }))
            .child(pick(st, t!("Часы"), &["fg", "muted"], &[".clock-time", ".clock-date"], || {
                DecoratedBox::new()
                    .class("applet applet-clock")
                    .child(Column::new().cross_axis_alignment(CrossAxisAlignment::End).child(Text::new("14:32").class("clock-time")).child(Text::new(t!("чт, 22 окт")).class("clock-date")))
            })),
    )
}

fn panel(st: St) -> W {
    pick(st, t!("Панель"), &["panel-bg", "border", "radius"], &[".panel", ".panel-floating"], move || DecoratedBox::new().class("panel panel-floating").child(panel_row(st)))
}

fn launcher_item(g: &str, name: String, sub: String, sel: bool) -> W {
    boxed(
        DecoratedBox::new().class(if sel { "launcher-row launcher-row-selected" } else { "launcher-row" }).child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("launcher-icon").child(Icon::new(g).class("launcher-icon-glyph")))
                .child(Column::new().child(txt(name, "launcher-name")).child(txt(sub, "launcher-sub"))),
        ),
    )
}

fn launcher_body(st: St) -> W {
    let search = pick(st, t!("Поиск"), &["surface-alt", "border", "muted", "fg"], &[".search-box", ".search-icon", ".search-field"], || {
        DecoratedBox::new()
            .class("search-box")
            .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icons::SEARCH).class("search-icon")).child(Text::new(t!("Поиск программ")).class("launcher-sub")))
    });
    let list = Column::new()
        .gap(2.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(pick(st, t!("Выбранная строка"), &["accent-soft", "accent", "fg", "muted"], &[".launcher-row-selected", ".launcher-icon-glyph", ".launcher-name", ".launcher-sub"], || {
            DecoratedBox::new().child(launcher_item(icons::TERMINAL, t!("Терминал"), t!("Командная строка"), true))
        }))
        .child(launcher_item(icons::FOLDER, t!("Проводник"), t!("Файлы и папки"), false))
        .child(launcher_item(icons::SETTINGS, t!("Параметры системы"), t!("Оформление, сеть, звук"), false))
        .child(launcher_item(icons::MUSIC, t!("Музыка"), t!("Плеер"), false));
    boxed(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(search).child(list))
}

fn launcher(st: St) -> W {
    pick(st, t!("Меню запуска"), &["menu-bg", "border", "shadow", "radius"], &[".popup-card"], move || DecoratedBox::new().class("popup-card te-pv-launcher").child(launcher_body(st)))
}

fn notification(st: St) -> W {
    pick(st, t!("Уведомление"), &["surface", "border", "shadow", "fg", "muted", "accent-soft", "accent"], &[".notif", ".notif-plate", ".notif-glyph", ".notif-app", ".notif-summary", ".notif-body", ".notif-action"], || {
        DecoratedBox::new().class("notif te-pv-notif").child(
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .child(DecoratedBox::new().class("notif-plate").child(Icon::new(icons::NOTIFY).class("notif-glyph")))
                .child(
                    DecoratedBox::new().class("grow").child(
                        Column::new()
                            .gap(3.0)
                            .cross_axis_alignment(CrossAxisAlignment::Stretch)
                            .child(Row::new().gap(6.0).child(txt(t!("Почта"), "notif-app")).child(txt(t!("сейчас"), "notif-time")))
                            .child(txt(t!("Новое письмо"), "notif-summary"))
                            .child(Text::new(t!("Тема оформления готова — посмотрите, как она смотрится.")).max_lines(2).class("notif-body"))
                            .child(Row::new().gap(6.0).child(DecoratedBox::new().class("notif-action").child(Text::new(t!("Открыть")).class("notif-action-label")))),
                    ),
                ),
        )
    })
}

fn osd(st: St) -> W {
    pick(st, t!("Громкость (OSD)"), &["menu-bg", "border", "accent", "pressed", "fg"], &[".osd", ".osd-icon", ".meter-fill", ".meter-rest", ".osd-value"], || {
        DecoratedBox::new().class("osd").child(
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(icons::VOLUME).class("osd-icon"))
                .child(Row::new().class("osd-meter").child(DecoratedBox::new().class("meter-fill te-pv-meter-fill")).child(DecoratedBox::new().class("meter-rest grow")))
                .child(Text::new("65").class("osd-value")),
        )
    })
}

fn win_title(st: St, inactive: bool, text: String) -> W {
    let label = if inactive { t!("Заголовок неактивного окна") } else { t!("Заголовок окна") };
    pick(st, label, if inactive { &["titlebar_inactive"] } else { &["titlebar"] }, &[], move || {
        DecoratedBox::new().class(if inactive { "te-win-title te-win-title-inactive" } else { "te-win-title" }).child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("grow").child(Text::new(text.clone()).max_lines(1).class("te-win-title-text")))
                .child(DecoratedBox::new().class("te-win-dot"))
                .child(DecoratedBox::new().class("te-win-dot"))
                .child(DecoratedBox::new().class("te-win-dot te-win-dot-close")),
        )
    })
}

/// Окно программы: заголовок композитора и «Параметры системы» внутри.
fn app_window(st: St, inactive: bool) -> W {
    let title = win_title(st, inactive, if inactive { t!("Проводник") } else { t!("Параметры системы") });
    let body: W = if inactive { boxed(DecoratedBox::new().class("te-scope-settings te-win-body").child(DecoratedBox::new().class("root grow"))) } else { settings_body(st, true) };
    boxed(DecoratedBox::new().class(if inactive { "te-win te-win-back" } else { "te-win" }).clip(true).child(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(title).child(DecoratedBox::new().class("grow").child(body))))
}

fn desk(st: St) -> W {
    // Окна — в фоне на всю область, меню и уведомление — поверх, как на столе.
    let windows = Stack::new()
        .fit(StackFit::Expand)
        .child(DecoratedBox::new().class("te-pv-win-back-pos").child(app_window(st, true)))
        .child(DecoratedBox::new().class("te-pv-win-pos").child(app_window(st, false)));
    let popups = Row::new()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .child(launcher(st))
        .child(DecoratedBox::new().class("grow"))
        .child(Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("te-pv-right").child(notification(st)));
    let body = DecoratedBox::new().class("te-pv-body").child(Stack::new().fit(StackFit::Expand).child(windows).child(popups));
    let bottom = Row::new().main_axis_alignment(MainAxisAlignment::Center).class("te-pv-bottom").child(osd(st));
    boxed(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("te-pv-desk")
            .child(DecoratedBox::new().class("te-pv-panel-pos").child(panel(st)))
            .child(DecoratedBox::new().class("grow").child(body))
            .child(bottom),
    )
}

fn desktop(st: St) -> W {
    let wall = pick(st, t!("Обои"), &["wallpaper"], &[".wallpaper-color"], move || DecoratedBox::new().class("wallpaper-color te-pv-wall").child(desk(st)));
    boxed(DecoratedBox::new().class("te-scope-shell te-pv-frame").clip(true).child(wall))
}

// ── «Параметры системы» ────────────────────────────────────────────────

fn nav(g: &str, label: String, active: bool) -> W {
    boxed(DecoratedBox::new().class(if active { "nav-item active" } else { "nav-item" }).child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(g).class("nav-icon")).child(Text::new(label).class("nav-label"))))
}

fn setting(label: String, sub: String, control: W) -> W {
    boxed(
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("setting-row")
            .child(DecoratedBox::new().class("row-text").child(Column::new().gap(2.0).child(Text::new(label).class("row-label")).child(Text::new(sub).max_lines(1).class("row-sub"))))
            .child(control),
    )
}

fn settings_body(st: St, compact: bool) -> W {
    let sidebar = pick(st, t!("Боковая панель настроек"), &["sidebar-bg", "accent-soft", "accent", "fg", "muted"], &[".sidebar", ".nav-item", ".nav-item.active", ".nav-icon", ".nav-label"], || {
        DecoratedBox::new().class("sidebar te-pv-set-side").child(
            Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(DecoratedBox::new().class("brand").child(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(DecoratedBox::new().class("brand-logo").child(Icon::new(icons::SETTINGS).class("brand-icon")))
                        .child(Text::new("synshell").class("brand-title")),
                ))
                .child(nav(icons::BRUSH, t!("Оформление"), true))
                .child(nav(icons::WIFI, t!("Сеть"), false))
                .child(nav(icons::VOLUME, t!("Звук"), false))
                .child(nav(icons::KEYBOARD, t!("Клавиатура"), false)),
        )
    });
    let card = pick(st, t!("Карточка настроек"), &["card-bg", "border", "radius", "shadow", "fg", "muted"], &[".group-card", ".setting-row", ".row-label", ".row-sub", ".row-sep"], move || {
        let mut col = Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(setting(t!("Тёмная тема"), t!("Тёмные фоны во всех программах"), boxed(Toggle::with_state(true))));
        if !compact {
            col = col
                .child(DecoratedBox::new().class("row-sep"))
                .child(setting(t!("Прозрачность панели"), t!("Сквозь панель видно обои"), boxed(Slider::new().range(0.0, 1.0).value(0.8).width(140.0))))
                .child(DecoratedBox::new().class("row-sep"))
                .child(setting(t!("Имя устройства"), t!("Видно в сети"), boxed(TextField::new().text("synshell").width(150.0))));
        }
        DecoratedBox::new().class("group-card").child(col)
    });
    let buttons = pick(st, t!("Кнопки"), &["accent", "accent-fg", "surface-alt", "border", "danger"], &["Button", "Button.primary", "Button.danger"], || {
        Row::new().gap(8.0).child(Button::new(t!("Применить")).class("primary")).child(Button::new(t!("Отмена"))).child(Button::new(t!("Удалить")).class("danger"))
    });
    let title = pick(st, t!("Заголовок страницы"), &["fg", "muted"], &[".page-title", ".page-subtitle"], || {
        Column::new().gap(4.0).child(Text::new(t!("Оформление")).class("page-title")).child(Text::new(t!("Тема, цвета и шрифты")).class("page-subtitle"))
    });
    let page = Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("page-body").child(title).child(Text::new(t!("Вид")).class("group-title")).child(card).child(buttons);
    let mut row = Row::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("root grow");
    if !compact {
        row = row.child(sidebar);
    }
    let row = row.child(DecoratedBox::new().class(if compact { "main grow te-pv-compact" } else { "main grow" }).child(page));
    boxed(DecoratedBox::new().class("te-scope-settings te-win-body").child(row))
}

fn settings_scene(st: St) -> W {
    let win = boxed(
        DecoratedBox::new()
            .class("te-win te-win-full")
            .clip(true)
            .child(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(win_title(st, false, t!("Параметры системы"))).child(DecoratedBox::new().class("grow").child(settings_body(st, false)))),
    );
    boxed(DecoratedBox::new().class("te-scope-shell te-pv-frame te-pv-frame-settings").clip(true).child(DecoratedBox::new().class("wallpaper-color te-pv-wall").child(DecoratedBox::new().class("te-pv-settings-pos").child(win))))
}

/// Элемент, который можно выбрать пипеткой: щелчок в режиме пипетки
/// показывает его токены и правила в инспекторе; выбранный подсвечен.
/// Без `Reactive` (он ослабляет ограничения раскладки): подсветку меняет
/// пересборка всей сцены при смене выбора.
fn pick<F, R>(st: St, label: String, tokens: &'static [&'static str], selectors: &'static [&'static str], build: F) -> W
where
    F: Fn() -> R + Send + Sync + 'static,
    R: Widget + 'static,
{
    let armed = st.pipette.get_untracked();
    let hit = st.pick.get_untracked().is_some_and(|p| p.label == label);
    let class = match (armed, hit) {
        (true, _) => "te-pick te-pick-armed",
        (false, true) => "te-pick te-pick-hit",
        _ => "te-pick",
    };
    boxed(
        GestureDetector::new()
            .on_click(move || {
                if st.pipette.get_untracked() {
                    st.pick.set(Some(Pick { label: label.clone(), tokens: tokens.to_vec(), selectors: selectors.to_vec() }));
                    st.pipette.set(false);
                    if !matches!(st.mode.get_untracked(), Mode::Palette | Mode::Code) {
                        st.mode.set(Mode::Palette);
                    }
                }
            })
            .child(DecoratedBox::new().class(class).child(build())),
    )
}

fn toolbar(st: St) -> W {
    let scene = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let s = st.scene.get();
        vec![boxed(
            SegmentedButton::new(vec![Segment::with_icon(t!("Рабочий стол"), icons::DESKTOP), Segment::with_icon(t!("Окно настроек"), icons::SETTINGS)])
                .selected(if s == Scene::Desktop { 0 } else { 1 })
                .on_change(move |i| st.scene.set(if i == 0 { Scene::Desktop } else { Scene::Settings }))
                .class("te-seg"),
        )]
    });
    let pipette = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let on = st.pipette.get();
        vec![tip(
            Button::new(if on { t!("Щёлкните по элементу…") } else { t!("Пипетка") }).icon(icons::PIPETTE).class(if on { "te-btn te-btn-on" } else { "te-btn" }).on_click(move || st.pipette.set(!st.pipette.get_untracked())),
            t!("Щёлкните по любому элементу предпросмотра — в инспекторе появятся его цвета и правила"),
        )]
    });
    let err = Reactive::new(move || -> Vec<W> {
        match st.error.get() {
            Some(e) => vec![boxed(DecoratedBox::new().class("te-error").child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(icons::WARNING).class("te-ic te-error-ic")).child(Text::new(e).max_lines(2).class("te-error-text"))))],
            None => vec![],
        }
    });
    boxed(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).class("te-pv-toolbar").child(scene).child(DecoratedBox::new().class("grow")).child(pipette))
            .child(err),
    )
}

pub fn preview(st: St) -> W {
    let scene = Reactive::new(move || -> Vec<W> {
        // Подсветка выбранного пипеткой — пересборкой сцены.
        let _ = (st.pick.get(), st.pipette.get());
        match st.scene.get() {
            Scene::Desktop => vec![desktop(st)],
            Scene::Settings => vec![settings_scene(st)],
        }
    });
    boxed(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("te-preview").child(toolbar(st)).child(DecoratedBox::new().class("grow").child(scene)))
}
