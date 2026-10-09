//! Настройки проводника: панель выезжает справа поверх окна (на телефоне —
//! почти во всю ширину), под ней затемнение — касание закрывает. Каждая
//! настройка сразу применяется и пишется в `[files]` config.toml.

use syngui::buttons::Segment;
use syngui::CursorIcon;
use syngui::prelude::*;
use syngui::widgets::*;

use super::{boxed, icons, W};
use crate::actions;
use crate::model::SortKey;
use crate::state::{self, ViewMode};

/// Ширина панели на компьютере, px.
const WIDTH: f32 = 384.0;
/// Предел миниатюр на выбор, МБ.
const THUMB_LIMITS: [u64; 4] = [16, 64, 256, 1024];

fn close() {
    state::ctx().settings.set(false);
}

fn section(icon: &'static str, title: &str, rows: Vec<W>) -> W {
    let mut c = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class("set-sec-badge").child(Icon::new(icon).class("icon set-sec-icon")))
            .child(Text::new(title).class("set-sec-title")),
    );
    for r in rows {
        c = c.child(r);
    }
    boxed(DecoratedBox::new().child(c).class("set-card"))
}

fn hint(text: &str) -> W {
    boxed(Text::new(text).max_lines(4).class("set-hint"))
}

fn label(text: &str) -> W {
    boxed(Text::new(text).max_lines(2).class("set-row-title"))
}

fn toggle_row(title: &str, sub: &str, on: bool, f: impl FnMut(bool) + Send + 'static) -> W {
    let mut text = Column::new().gap(2.0).child(Text::new(title).max_lines(2).class("set-row-title"));
    if !sub.is_empty() {
        text = text.child(Text::new(sub).max_lines(4).class("set-hint"));
    }
    boxed(
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class("grow").child(text))
            .child(Toggle::with_state(on).on_change(f)),
    )
}

fn view_section(phone: bool) -> W {
    let p = state::pane();
    let ctx = state::ctx();
    let view = p.view.get_untracked();
    let mut rows: Vec<W> = Vec::new();
    rows.push(label(&t!("Как показывать файлы")));
    if phone {
        let grid = super::items::phone_grid(view);
        rows.push(boxed(
            SegmentedButton::new(vec![
                Segment::with_icon(t!("Список"), icons::VIEW_LIST),
                Segment::with_icon(t!("Сетка"), icons::VIEW_ICONS),
            ])
            .selected(if grid { 1 } else { 0 })
            .on_change(|i| actions::set_view(state::pane(), if i == 1 { ViewMode::Icons } else { ViewMode::Details })),
        ));
    } else {
        const MODES: [ViewMode; 4] = [ViewMode::Details, ViewMode::List, ViewMode::Tiles, ViewMode::Icons];
        rows.push(boxed(
            SegmentedButton::new(vec![
                Segment::icon_only(icons::VIEW_DETAILS),
                Segment::icon_only(icons::VIEW_LIST),
                Segment::icon_only(icons::VIEW_TILES),
                Segment::icon_only(icons::VIEW_ICONS),
            ])
            .selected(MODES.iter().position(|m| *m == view).unwrap_or(0))
            .on_change(|i| actions::set_view(state::pane(), MODES[i.min(3)])),
        ));
        rows.push(hint(&t!("Таблица, список, плитки или значки — для новых вкладок тоже.")));
    }
    let (lo, hi) = if phone { (48.0, 96.0) } else { (32.0, 256.0) };
    let px = p.icon_size.get_untracked() as f32;
    let value = use_signal(px.clamp(lo, hi) as u32);
    rows.push(boxed(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class("grow").child(label(&t!("Размер значков"))))
            .child(Reactive::new(move || -> Vec<W> { vec![boxed(Text::new(format!("{} px", value.get())).class("set-value"))] })),
    ));
    rows.push(boxed(
        Slider::new()
            .range(lo, hi)
            .step(8.0)
            .value(px.clamp(lo, hi))
            .on_change(move |v| {
                let n = v.round() as u32;
                value.set(n);
                actions::set_icon_size(state::pane(), n);
            })
            .class("set-slider"),
    ));
    rows.push(toggle_row(&t!("Скрытые файлы"), &t!("Имена с точки в начале (Ctrl+H)."), ctx.show_hidden.get_untracked(), |on| {
        if state::ctx().show_hidden.get_untracked() != on {
            actions::toggle_hidden();
        }
    }));
    if !phone {
        rows.push(toggle_row(&t!("Панель навигации"), &t!("Места, диски и закреплённые папки слева (F9)."), ctx.sidebar.get_untracked(), |on| {
            if state::ctx().sidebar.get_untracked() != on {
                actions::toggle_sidebar();
            }
        }));
    }
    section(icons::VIEW_ICONS, &t!("Вид"), rows)
}

fn folders_section() -> W {
    let cfg = state::ctx().cfg.get_untracked();
    let p = state::pane();
    section(
        icons::FOLDER,
        &t!("Папки"),
        vec![
            toggle_row(
                &t!("Показывать размеры папок"),
                &t!("Считаются в фоне и видны чипом на значке папки, в таблице — в колонке «Размер». Ссылки и подключённые внутрь диски не учитываются."),
                cfg.files.dir_sizes,
                |on| actions::set_files("dir_sizes", on, |f| f.dir_sizes = on),
            ),
            toggle_row(&t!("Папки сверху"), &t!("Сначала папки, потом файлы."), p.sort.get_untracked().folders_first, |on| {
                let p = state::pane();
                if p.sort.get_untracked().folders_first != on {
                    actions::run(p, "sort:folders");
                }
            }),
        ],
    )
}

fn sort_section() -> W {
    const KEYS: [SortKey; 4] = [SortKey::Name, SortKey::Modified, SortKey::Type, SortKey::Size];
    let s = state::pane().sort.get_untracked();
    section(
        icons::SORT,
        &t!("Сортировка"),
        vec![
            boxed(
                SegmentedButton::new(vec![t!("Имя"), t!("Дата"), t!("Тип"), t!("Размер")])
                    .selected(KEYS.iter().position(|k| *k == s.key).unwrap_or(0))
                    .on_change(|i| actions::set_sort_key(state::pane(), KEYS[i.min(3)])),
            ),
            toggle_row(&t!("По убыванию"), "", s.descending, |on| actions::run(state::pane(), if on { "sort:desc" } else { "sort:asc" })),
        ],
    )
}

fn thumbs_section() -> W {
    let cfg = state::ctx().cfg.get_untracked();
    let limit = THUMB_LIMITS.iter().position(|v| *v >= cfg.files.thumbnail_max_mb).unwrap_or(THUMB_LIMITS.len() - 1);
    section(
        icons::IMAGE,
        &t!("Миниатюры"),
        vec![
            toggle_row(
                &t!("Миниатюры картинок и видео"),
                &t!("Вместо значка типа — уменьшенное содержимое файла."),
                cfg.files.thumbnails,
                |on| actions::set_files("thumbnails", on, |f| f.thumbnails = on),
            ),
            label(&t!("Не делать для файлов больше")),
            boxed(
                SegmentedButton::new(THUMB_LIMITS.iter().map(|m| if *m >= 1024 { t!("{v} ГБ", v = m / 1024) } else { t!("{v} МБ", v = m) }).collect::<Vec<_>>())
                    .selected(limit)
                    .on_change(|i| {
                        let mb = THUMB_LIMITS[i.min(THUMB_LIMITS.len() - 1)];
                        actions::set_files("thumbnail_max_mb", mb as i64, |f| f.thumbnail_max_mb = mb);
                    }),
            ),
        ],
    )
}

fn behaviour_section(phone: bool) -> W {
    let cfg = state::ctx().cfg.get_untracked();
    let mut rows: Vec<W> = Vec::new();
    if !phone {
        rows.push(toggle_row(
            &t!("Открывать одним щелчком"),
            &t!("Как в KDE: щелчок открывает, выделение — с Ctrl или рамкой."),
            cfg.files.single_click,
            |on| actions::set_files("single_click", on, |f| f.single_click = on),
        ));
    }
    rows.push(toggle_row(
        &t!("Спрашивать перед удалением"),
        &t!("Подтверждение перед отправкой в корзину. Удаление насовсем спрашивается всегда."),
        cfg.files.confirm_trash,
        |on| actions::set_files("confirm_trash", on, |f| f.confirm_trash = on),
    ));
    rows.push(toggle_row(
        &t!("Восстанавливать вкладки"),
        &t!("При запуске открыть папки, где остановились."),
        cfg.files.restore_tabs,
        |on| actions::set_files("restore_tabs", on, |f| f.restore_tabs = on),
    ));
    section(icons::TUNE, &t!("Поведение"), rows)
}

fn panel(phone: bool) -> W {
    let head = Row::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("set-logo").child(Icon::new(icons::SETTINGS).class("icon")))
        .child(
            Column::new()
                .gap(1.0)
                .class("grow")
                .child(Text::new(t!("Настройки")).class("set-title"))
                .child(Text::new(t!("Проводник")).class("set-hint")),
        )
        .child(super::icon_button(icons::CLOSE, &t!("Закрыть (Esc)"), "set-close", true, close))
        .class("set-head");
    let body = Column::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(view_section(phone))
        .child(folders_section())
        .child(sort_section())
        .child(thumbs_section())
        .child(behaviour_section(phone))
        .child(section(icons::HISTORY, &t!("Журнал изменений"), super::activity::settings_rows()))
        .child(hint(&t!("Настройки хранятся в ~/.config/synshell/config.toml, раздел [files].")))
        .class("set-body");
    let col = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("set-panel-inner")
        .child(head)
        .child(ScrollView::new().vertical().class("grow").child(body));
    let vw = syngui::viewport::viewport_size().get_untracked().width;
    let w = if phone { (vw - 40.0).clamp(260.0, 420.0) } else { WIDTH.min((vw - 48.0).max(280.0)) };
    // Касания по панели не доходят до затемнения под ней.
    boxed(
        GestureDetector::new()
            .cursor(CursorIcon::Default)
            .on_click(|| {})
            .child(DecoratedBox::new().style("width", w).class(if phone { "set-panel phone" } else { "set-panel" }).child(col)),
    )
}

fn scrim(open: RwSignal<bool>) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let shown = open.get();
        let vp = syngui::viewport::viewport_size().get();
        vec![boxed(
            Presence::new(
                shown,
                GestureDetector::new()
                    .cursor(CursorIcon::Default)
                    .on_click(move || open.set(false))
                    .child(DecoratedBox::new().class("set-scrim").style("width", vp.width).style("height", vp.height)),
            )
            .enter(Motion::fade())
            .exit(Motion::fade())
            .duration_ms(200)
            .initial(false),
        )]
    }))
}

/// Слой настроек поверх окна.
pub fn layer(phone: bool) -> W {
    let open = state::ctx().settings;
    boxed(
        Stack::new().fit(StackFit::Expand).child(scrim(open)).child(
            // Прижать вправо — снаружи `Presence` (он меряется по содержимому).
            Row::new().main_axis_alignment(MainAxisAlignment::End).child(
                Presence::signal(open, move || panel(phone))
                    .enter(Motion::fade().slide(WIDTH, 0.0))
                    .exit(Motion::fade().slide(WIDTH, 0.0))
                    .duration_ms(260)
                    .exit_duration_ms(180),
            ),
        ),
    )
}
