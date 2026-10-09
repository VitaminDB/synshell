//! Раскладка рабочего стола: своё окно — заголовок (название, «назад»,
//! поиск, кнопки окна), сайдбар как в магазинах (обзор и категории сверху,
//! AUR, установленные, обновления, задачи внизу), справа — витрина, списки
//! или страница программы на всю ширину (снимки крупно, описание и сведения
//! в две колонки).

use synsystem::packages::{CatalogApp, Pkg, Source};
use syngui::mss::StyleValue;
use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion};
use syngui::prelude::*;
use syngui::GestureDetector;

use crate::ui::*;
use crate::Tab;
use crate::*;

/// Ширина сайдбара (и области названия в заголовке над ним).
const SIDEBAR_W: f32 = 248.0;
/// Высота заголовка окна (как `.titlebar` в MSS).
const TITLEBAR_H: f32 = 52.0;
/// Поле поиска в заголовке.
const SEARCH_W: f32 = 520.0;
/// Снимок экрана на странице программы.
const PAGE_SHOT_W: f32 = 480.0;
const PAGE_SHOT_H: f32 = 270.0;

pub fn desktop(st: St) -> W {
    let main = Reactive::new(move || -> Vec<W> {
        let sel = st.selected.get();
        let key = sel.as_ref().map(|_| 2u64).unwrap_or(1);
        vec![Box::new(
            AnimatedSwitcher::new(key, move || -> Box<dyn Widget> {
                match st.selected.get_untracked() {
                    Some(p) => detail_page(st, p),
                    None => tab_content(st),
                }
            })
            .directional(true)
            .slide(40.0, 0.0)
            .duration_ms(220)
            .animate_size(false)
            .class("grow"),
        )]
    });
    Box::new(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(titlebar(st))
            .child(
                Row::new()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(sidebar(st))
                    .child(DecoratedBox::new().child(Column::new().child(DecoratedBox::new().child(main).class("grow")).child(queue_bar(st))).class("grow content"))
                    .class("grow"),
            )
            .class("window"),
    )
}

/// Сбросить запрос поиска и пересоздать поле в заголовке.
fn clear_search(st: St) {
    if !st.query.get_untracked().is_empty() {
        st.query.set(String::new());
        search(st, String::new());
        st.search_rev.set(st.search_rev.get_untracked() + 1);
    }
}

fn titlebar(st: St) -> W {
    // Что ищет поле: программы (обзор, обновления, задачи), AUR или установленные. Поле
    // пересоздаётся только при смене области — набор не теряет фокус при переходе в «Обзор».
    let domain = use_signal(0u8);
    create_effect(move || {
        domain.set(match st.tab.get() {
            Tab::Aur => 1,
            Tab::Installed => 2,
            _ => 0,
        })
    });
    let field = Reactive::new(move || -> Vec<W> {
        let d = domain.get();
        let _ = st.search_rev.get();
        let w: W = match d {
            1 => Box::new(search_box(st.aur_query.get_untracked(), &t!("Искать в AUR"), move |t| {
                st.selected.set(None);
                st.aur_query.set(t.clone());
                search_aur(st, t);
            })),
            2 => Box::new(search_box(st.filter.get_untracked(), &t!("Фильтр установленных"), move |t| {
                st.selected.set(None);
                st.filter.set(t);
            })),
            _ => Box::new(search_box(st.query.get_untracked(), &t!("Найти программу в репозиториях и AUR"), move |t| {
                st.selected.set(None);
                if st.tab.get_untracked() != Tab::Explore {
                    st.tab.set(Tab::Explore);
                }
                st.query.set(t.clone());
                search(st, t);
            })),
        };
        vec![w]
    });
    let back = Reactive::new(move || -> Vec<W> {
        let can = st.selected.get().is_some() || (st.tab.get() == Tab::Explore && st.category.get().is_some());
        if !can {
            return vec![Box::new(DecoratedBox::new().class("tb-back-off"))];
        }
        vec![Box::new(GestureDetector::new().on_click(move || {
            if st.selected.get_untracked().is_some() {
                st.selected.set(None);
            } else {
                st.category.set(None);
            }
        }).child(DecoratedBox::new().child(Icon::new("\u{E5C4}").class("tb-back-icon")).class("tb-back")))]
    });
    let controls = Reactive::new(move || -> Vec<W> {
        let w = st.window.get();
        vec![Box::new(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    // Колонка высотой с заголовок, ряд — по её центру: значок и название ровно посередине.
    let brand = WindowDragRegion::new().child(
        Column::new()
            .main_axis_alignment(MainAxisAlignment::Center)
            .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().child(Icon::new("\u{EA12}").class("brand-icon")).class("brand-icon-box")).child(Text::new(t!("Программы")).class("brand")))
            .class("tb-brand")
            .style("width", StyleValue::px(SIDEBAR_W))
            .style("height", StyleValue::px(TITLEBAR_H)),
    );
    Box::new(
        DecoratedBox::new()
            .child(
                Row::new()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(brand)
                    .child(back)
                    .child(DecoratedBox::new().child(field).class("tb-search").style("width", StyleValue::px(SEARCH_W)))
                    .child(DecoratedBox::new().class("grow tb-drag").child(WindowDragRegion::new().child(DecoratedBox::new().class("tb-drag-space"))))
                    .child(DecoratedBox::new().child(controls).class("window-controls")),
            )
            .class("titlebar"),
    )
}

fn nav_item(icon: &str, label: String, badge: Option<String>, on: bool, f: impl Fn() + Send + Sync + 'static) -> W {
    let mut row = Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icon).class("nav-icon")).child(Text::new(label).max_lines(1).class("nav-label grow"));
    if let Some(b) = badge {
        row = row.child(DecoratedBox::new().child(Text::new(b).class("badge-text")).class("badge"));
    }
    Box::new(GestureDetector::new().on_click(f).child(DecoratedBox::new().child(row).class(if on { "nav-item active" } else { "nav-item" })))
}

fn sidebar(st: St) -> W {
    let top = Reactive::new(move || -> Vec<W> {
        let tab = st.tab.get();
        let cat = st.category.get();
        let explore = tab == Tab::Explore;
        let mut col = Column::new().gap(2.0).child(nav_item("\u{E88A}", t!("Обзор").into(), None, explore && cat.is_none(), move || {
            st.selected.set(None);
            st.tab.set(Tab::Explore);
            st.category.set(None);
            clear_search(st);
        }));
        let apps = st.catalog.get().unwrap_or_default();
        if !apps.is_empty() {
            col = col.child(Text::new(t!("Категории")).class("nav-section"));
            for c in CATEGORIES {
                let n = apps.iter().filter(|a| c.contains(a)).count();
                if n == 0 {
                    continue;
                }
                let key = c.key;
                col = col.child(nav_item(c.icon, c.title(), Some(n.to_string()).filter(|_| false), explore && cat == Some(key), move || {
                    st.selected.set(None);
                    st.tab.set(Tab::Explore);
                    st.category.set(Some(key));
                    clear_search(st);
                }));
            }
        }
        col = col.child(Text::new(t!("Наборы")).class("nav-section")).child(nav_item(META_ICON, t!("Метапакеты и группы").into(), None, explore && cat == Some(META_KEY), move || {
            st.selected.set(None);
            st.tab.set(Tab::Explore);
            st.category.set(Some(META_KEY));
            clear_search(st);
        }));
        vec![Box::new(col)]
    });
    let bottom = Reactive::new(move || -> Vec<W> {
        let tab = st.tab.get();
        let mut col = Column::new().gap(2.0);
        for t in [Tab::Aur, Tab::Installed, Tab::Updates, Tab::Jobs] {
            col = col.child(nav_item(t.icon(), t.label().into(), tab_badge(st, t), tab == t, move || {
                st.selected.set(None);
                open_tab(st, t);
            }));
        }
        vec![Box::new(col)]
    });
    Box::new(
        DecoratedBox::new()
            .child(Column::new().child(ScrollView::new().vertical().child(top).class("grow")).child(DecoratedBox::new().class("nav-sep")).child(bottom))
            .class("sidebar")
            .style("width", StyleValue::px(SIDEBAR_W)),
    )
}

/// Колонок карточек шириной ~`tile` в области содержимого.
pub fn cols(tile: f32) -> usize {
    let w = syngui::viewport::viewport_size().get().width;
    let avail = w - SIDEBAR_W - 56.0;
    ((avail + 14.0) / (tile + 14.0)).floor().max(1.0) as usize
}

/// Чем выше — тем раньше на витрине: известные, со значком и снимками.
fn showcase_rank(a: &CatalogApp) -> (usize, bool, bool) {
    let featured = FEATURED.iter().position(|n| *n == a.pkg.name).unwrap_or(usize::MAX);
    let has_icon = a.icon.as_deref().and_then(synshell_common::xdg::lookup_icon).is_some() || !a.cached_icons.is_empty();
    (featured, !has_icon, a.screenshots.is_empty())
}

fn card_grid(st: St, apps: Vec<CatalogApp>, n: usize) -> W {
    let mut grid = Grid::new(n).gap(14.0);
    for a in apps {
        grid = grid.child(app_card(st, a));
    }
    Box::new(grid)
}

fn shelf_head(title: &str, more: Option<(String, Box<dyn Fn() + Send + Sync>)>) -> W {
    let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(title.to_string()).class("h2 grow"));
    if let Some((label, f)) = more {
        row = row.child(GestureDetector::new().on_click(move || f()).child(DecoratedBox::new().child(Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(label).class("more-text")).child(Icon::new("\u{E5CC}").class("more-icon"))).class("more")));
    }
    Box::new(row.class("shelf-head"))
}

/// Обзор: популярные программы и полка каждой категории.
pub fn home(st: St, apps: Vec<CatalogApp>) -> W {
    let n = cols(240.0);
    let mut col = Column::new().gap(14.0).child(
        DecoratedBox::new()
            .child(
                Column::new()
                    .gap(6.0)
                    .child(Text::new(t!("Программы для вашего компьютера")).class("hero-title"))
                    .child(Text::new(t!("{v} из репозиториев Arch Linux, поиск — ещё и по AUR. Отмечайте программы — установка и удаление выполнятся одной очередью.", v = programs(apps.len()))).class("hero-sub")),
            )
            .class("hero"),
    );
    let featured: Vec<CatalogApp> = FEATURED.iter().filter_map(|f| apps.iter().find(|a| a.pkg.name == *f)).filter(|a| a.pkg.installed.is_none()).take(n * 2).cloned().collect();
    let shown: Vec<String> = featured.iter().map(|a| a.pkg.name.clone()).collect();
    if !featured.is_empty() {
        col = col.child(shelf_head(&t!("Популярные программы"), None)).child(card_grid(st, featured, n));
    }
    for c in CATEGORIES {
        // Уже показанные в «Популярных» — не повторять.
        let mut v: Vec<CatalogApp> = apps.iter().filter(|a| c.contains(a) && a.pkg.installed.is_none() && !shown.contains(&a.pkg.name)).cloned().collect();
        if v.is_empty() {
            continue;
        }
        let total = apps.iter().filter(|a| c.contains(a)).count();
        v.sort_by_cached_key(showcase_rank);
        v.truncate(n);
        let key = c.key;
        col = col.child(shelf_head(&c.title(), Some((t!("Все {total}", total = total), Box::new(move || st.category.set(Some(key))))))).child(card_grid(st, v, n));
    }
    Box::new(col.class("page"))
}

/// Категория: карточки программ (установленные — в конце).
pub fn category_page(st: St, cat: &'static Category, apps: Vec<CatalogApp>) -> W {
    let n = cols(240.0);
    let mut v: Vec<CatalogApp> = apps.into_iter().filter(|a| cat.contains(a)).collect();
    v.sort_by_cached_key(|a| (a.pkg.installed.is_some(), showcase_rank(a)));
    let pkgs: Vec<Pkg> = v.iter().map(|a| a.pkg.clone()).collect();
    let head = Row::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(Icon::new(cat.icon).class("cat-icon")).class("cat-icon-box"))
        .child(Column::new().gap(2.0).child(Text::new(cat.title()).class("h1")).child(Text::new(programs(v.len())).class("muted")).class("grow"));
    // Сотни карточек сразу — тяжело: первые 240, остальное находит поиск.
    let shown: Vec<CatalogApp> = v.into_iter().take(240).collect();
    Box::new(Column::new().gap(14.0).child(head).child(select_all_row(st, &pkgs, &t!("Программ"), false)).child(card_grid(st, shown, n)).class("page"))
}

/// Страница программы: шапка с действиями, снимки, описание и сведения.
fn detail_page(st: St, p: Pkg) -> W {
    let name = p.name.clone();
    let app = st.catalog.get_untracked().and_then(|c| c.into_iter().find(|a| a.pkg.name == name));
    let title = app.as_ref().map(|a| a.title.clone()).unwrap_or_else(|| name.clone());
    let summary = if p.description.is_empty() { app.as_ref().map(|a| a.pkg.description.clone()).unwrap_or_default() } else { p.description.clone() };
    let n2 = name.clone();
    let big_icon = Reactive::new(move || -> Vec<W> {
        match st.media.get().filter(|m| m.name == n2).and_then(|m| m.icon) {
            Some(path) => vec![Box::new(Image::new(path.to_string_lossy()).fit(ImageFit::Contain).placeholder(false).class("page-icon"))],
            None => vec![app_icon(&n2, "page-icon")],
        }
    });
    let mut chips = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(p.version.clone()).class("pkg-ver")).child(source_chip(&p));
    if p.installed.is_some() {
        chips = chips.child(chip(t!("установлен"), "chip-ok"));
    }
    if let Some(a) = &app {
        let c = category_label(a);
        if !c.is_empty() {
            chips = chips.child(chip(c, "chip-repo"));
        }
    }
    if title != name {
        chips = chips.child(Text::new(name.clone()).class("pkg-ver"));
    }
    let hero = Row::new()
        .gap(20.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(big_icon).class("page-icon-box"))
        .child(Column::new().gap(8.0).child(Text::new(title).max_lines(2).class("page-title")).child(Text::new(summary).max_lines(3).class("desc-lead")).child(chips).class("grow"))
        .child(detail_actions(st, p.clone()));

    // Левая колонка: описание, AUR, PKGBUILD, зависимости.
    let desc_local = app.as_ref().map(|a| a.description.clone()).unwrap_or_default();
    let n3 = name.clone();
    let aur = p.source == Source::Aur;
    let n4 = name.clone();
    let left = Reactive::new(move || -> Vec<W> {
        let mut c = Column::new().gap(10.0);
        let paras = if desc_local.is_empty() { st.media.get().filter(|m| m.name == n3).map(|m| m.description).unwrap_or_default() } else { desc_local.clone() };
        if !paras.is_empty() {
            c = c.child(Text::new(t!("Описание")).class("h2"));
            for p in paras.into_iter().take(16) {
                c = c.child(Text::new(p).class("desc"));
            }
        }
        if aur {
            let n = n4.clone();
            c = c.child(
                DecoratedBox::new()
                    .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new("\u{E002}").class("warn-icon")).child(Text::new(t!("Пакет из AUR собирают пользователи, а не Arch Linux. Проверьте PKGBUILD перед установкой.")).class("warn-text grow")).child(Button::new("PKGBUILD").class("small").on_click(move || show_pkgbuild(st, n.clone()))))
                    .class("warn"),
            );
        }
        if let Some(pb) = st.pkgbuild.get() {
            c = c.child(DecoratedBox::new().child(ScrollView::new().both().child(Text::new(pb).selectable(true).class("code"))).class("code-box"));
        }
        if let Some(d) = st.details.get() {
            if !d.depends.is_empty() {
                let mut deps = Flex::new().wrap().gap(6.0);
                for dep in &d.depends {
                    deps = deps.child(chip(dep.clone(), "chip-dep"));
                }
                c = c.child(Text::new(t!("Зависимости")).class("h2")).child(deps);
            }
            if !d.optional.is_empty() {
                let mut col = Column::new().gap(3.0);
                for o in &d.optional {
                    col = col.child(Text::new(o.clone()).class("pkg-desc"));
                }
                c = c.child(Text::new(t!("Дополнительно")).class("h2")).child(col);
            }
        }
        vec![Box::new(c)]
    });
    // Правая колонка: сведения и файлы.
    let right = Reactive::new(move || -> Vec<W> {
        let mut c = Column::new().gap(10.0).child(Text::new(t!("Сведения")).class("h2"));
        let Some(d) = st.details.get() else {
            return vec![Box::new(c.child(CircularProgress::new().indeterminate().size(22.0)))];
        };
        let mut table = Column::new().gap(0.0);
        for (k, v) in &d.fields {
            table = table.child(Row::new().gap(10.0).child(Text::new(k.clone()).class("k")).child(Text::new(v.clone()).selectable(true).class("v grow")).class("kv"));
        }
        if let Some(u) = &d.url {
            let u2 = u.clone();
            table = table.child(Row::new().gap(10.0).child(Text::new(t!("Сайт")).class("k")).child(GestureDetector::new().on_click(move || {
                let _ = std::process::Command::new("xdg-open").arg(&u2).spawn();
            }).child(Text::new(u.clone()).class("v link grow"))).class("kv"));
        }
        c = c.child(DecoratedBox::new().child(table).class("card"));
        if !d.files.is_empty() {
            let open = st.files_open.get();
            c = c.child(Row::new().child(Button::new(if open { t!("Скрыть файлы ({n})", n = d.files.len()) } else { t!("Показать файлы ({n})", n = d.files.len()) }).class("small").on_click(move || st.files_open.set(!open))));
            if open {
                let mut col = Column::new().gap(0.0);
                for f in d.files.iter().take(400) {
                    col = col.child(Text::new(f.clone()).class("log-line"));
                }
                c = c.child(DecoratedBox::new().child(col).class("code-box"));
            }
        }
        vec![Box::new(c)]
    });
    let columns = Row::new()
        .gap(28.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .child(DecoratedBox::new().child(left).class("grow page-left"))
        .child(DecoratedBox::new().child(right).class("page-right"));
    Box::new(
        ScrollView::new().vertical().child(
            Column::new()
                .gap(22.0)
                .child(hero)
                .child(screenshots(st, name, PAGE_SHOT_W, PAGE_SHOT_H))
                .child(columns)
                .class("page"),
        ),
    )
}
