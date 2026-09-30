//! Меню «Пуск» в духе Windows 11 (`[launcher] style = "win11"`) — общее для
//! рабочего стола и телефона:
//!
//! - поиск сверху (приложения, калькулятор, команда);
//! - «Закреплённые» — сетка значков по страницам (листаются пальцем), без
//!   категорий; кнопка «Все ›» перетекает в список всех приложений;
//! - «Все приложения» — алфавитный список с заголовками букв и icon rail
//!   сбоку: тап или ведение пальцем по букве прокручивает к ней;
//! - «Рекомендуемые» — недавно запущенные;
//! - внизу — пользователь, параметры, питание.
//!
//! На телефоне два вида (`[mobile] launcher`, кнопка внизу): `pages` — все
//! приложения значками по страницам во всю высоту, `list` — как на
//! рабочем столе.
//!
//! Правый щелчок или удержание по значку — меню значка прямо внутри
//! «Пуска»: закрепить/открепить, на док, на домашний экран (телефон).
//! На рабочем столе — карточка у кнопки, на телефоне — лист почти на весь
//! экран. Все переходы — перетекания (`AnimatedSwitcher`, `Presence`).

use std::collections::BTreeMap;
use syngui::input::Key;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::containers::scroll_to_named;
use syngui::widgets::{Carousel, PanAxis};
use syngui::{GestureDetector, Named};
use syngui_layer::KeyInfo;

use crate::ctx::ShellCtx;
use crate::launcher::{self, Item};
use crate::launchers::Launchable;
use crate::ui::{icon, mi, rx, InputArea};
use crate::xdg::{self, DesktopEntry};

/// Вид меню: закреплённые или все приложения (поиск — поверх обоих).
#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Home,
    All,
}

#[derive(Clone, Copy)]
struct St {
    query: RwSignal<String>,
    view: RwSignal<View>,
    selected: RwSignal<usize>,
    results: RwSignal<Vec<Item>>,
    /// Меню значка внутри «Пуска»: id приложения и точка.
    item_menu: RwSignal<Option<(String, f32, f32)>>,
    /// Страница закреплённых.
    page: RwSignal<usize>,
    /// Телефон: все приложения значками по страницам (иначе — список).
    pages: RwSignal<bool>,
}

/// Телефон: высота значка в сетке (отступы, значок, две строки подписи) и
/// зазор между строками.
const PHONE_TILE_H: f32 = 104.0;
const PHONE_GRID_GAP: f32 = 2.0;
/// Место под точки страниц карусели.
const INDICATORS_H: f32 = 24.0;
/// Высота «Пуска» без тела: поиск, низ и зазоры между ними.
const CHROME_H: f32 = 58.0 + 57.0 + 2.0 * 14.0;

/// Сколько строк значков помещается в `h`.
fn phone_rows(h: f32) -> usize {
    (((h + PHONE_GRID_GAP) / (PHONE_TILE_H + PHONE_GRID_GAP)).floor() as usize).max(1)
}

/// Размер: рабочий стол — из `[launcher]`, телефон — почти весь экран.
fn size(ctx: &ShellCtx) -> (f32, f32) {
    let cfg = ctx.cfg();
    let (ow, oh) = crate::manager::output_size(None);
    if ctx.is_phone() {
        (ow - 20.0, (oh * 0.9).max(420.0))
    } else {
        ((cfg.launcher.width as f32).max(560.0), (cfg.launcher.height as f32).max(600.0).min(oh - 80.0))
    }
}

/// Колонок в сетке закреплённых.
fn columns(ctx: &ShellCtx) -> usize {
    if ctx.is_phone() {
        4
    } else {
        6
    }
}

pub fn view(ctx: ShellCtx) -> impl Widget {
    let st = St {
        query: use_signal(String::new()),
        view: use_signal(View::Home),
        selected: use_signal(0usize),
        results: use_signal(Vec::new()),
        item_menu: use_signal(None),
        page: use_signal(0usize),
        pages: use_signal(ctx.is_phone() && ctx.cfg().mobile.launcher != "list"),
    };
    create_effect(move || {
        let q = st.query.get();
        st.results.set(launcher::search(&ctx, &q));
        st.selected.set(0);
    });
    crate::popup::set_key_handler(move |k: &KeyInfo| {
        if !k.pressed {
            return false;
        }
        if st.item_menu.get_untracked().is_some() && k.key == Key::Escape {
            st.item_menu.set(None);
            return true;
        }
        let searching = !st.query.get_untracked().trim().is_empty();
        if !searching {
            return false;
        }
        let n = st.results.get_untracked().len();
        match k.key {
            Key::Down if n > 0 => {
                st.selected.set((st.selected.get_untracked() + 1) % n);
                true
            }
            Key::Up if n > 0 => {
                st.selected.set((st.selected.get_untracked() + n - 1) % n);
                true
            }
            Key::Enter => {
                if let Some(it) = st.results.get_untracked().get(st.selected.get_untracked()) {
                    it.activate(ctx);
                }
                true
            }
            _ => false,
        }
    });
    let (w, _) = size(&ctx);
    let menu_layer = rx(move || match st.item_menu.get() {
        Some((id, x, y)) => Box::new(item_menu(ctx, st, &id, x, y)),
        None => Box::new(DecoratedBox::new()),
    });
    let phone = ctx.is_phone();
    // Телефон: высота — по месту поверхности (между панелями и клавиатурой),
    // с запасом сверху, и меняется, когда клавиатура выезжает.
    let sized = rx(move || {
        let (w, h) = if phone {
            // Лист `.popup-sheet` — не выше 86 % и с отступами 18 + 20 px:
            // меню меньше на них, иначе низ (пользователь, питание) срезался.
            let vp = syngui::viewport::viewport_size().get();
            (vp.width - 20.0, (vp.height * 0.86 - 40.0).max(300.0))
        } else {
            size(&ShellCtx::get())
        };
        let _ = w;
        Box::new(
            Column::new()
                .gap(14.0)
                .child(search(st))
                .child(body_ref(st, ctx, h - CHROME_H))
                .child(footer(ShellCtx::get(), st))
                .class("start")
                .style("height", StyleValue::px(h))
                .style("width", StyleValue::px(w - 24.0)),
        )
    });
    let _ = w;
    Stack::new().child(sized).child(menu_layer)
}

/// Середина меню: закреплённые, «Все приложения» или результаты поиска —
/// перетекают друг в друга.
fn body_ref(st: St, ctx: ShellCtx, body_h: f32) -> impl Widget {
    let dur = crate::anim::group_ms(&ctx, "menu", 260);
    rx(move || {
        let searching = !st.query.get().trim().is_empty();
        let v = st.view.get();
        let pages = st.pages.get();
        // Ключ задаёт направление перетекания: дальше по списку — въезд справа.
        let key = if searching { 3 } else if pages { 4 } else if v == View::All { 2 } else { 1 };
        let sw = AnimatedSwitcher::new(key, move || -> Box<dyn Widget> {
            if searching {
                Box::new(results(ctx, st))
            } else if pages {
                Box::new(app_pages(ctx, st, body_h))
            } else if v == View::All {
                Box::new(all_apps(ctx, st, body_h))
            } else {
                Box::new(home(ctx, st, body_h))
            }
        })
        .directional(true)
        .slide(36.0, 0.0)
        .duration_ms(dur)
        .exit_duration_ms(dur * 2 / 3)
        .animate_size(false)
        .class("start-body");
        // Телефон: тело — в колонке явной высоты (`flex-grow` не доходит
        // сквозь `rx`, а переключатель высоту из стиля не берёт), иначе
        // прокрутка занимала всё и выталкивала низ за край листа.
        if ctx.is_phone() {
            Box::new(Column::new().child(sw).style("height", StyleValue::px(body_h)))
        } else {
            Box::new(sw)
        }
    })
}

// ─── Поиск ───────────────────────────────────────────────────────────────────

fn search(st: St) -> impl Widget {
    let q = st.query;
    DecoratedBox::new()
        .child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(icon(mi::SEARCH).class("start-search-icon"))
                .child(
                    TextField::new()
                        .placeholder(if ShellCtx::get().is_phone() { "Поиск приложений и настроек" } else { "Поиск приложений, настроек, вычислений" })
                        // Телефон: клавиатура — по касанию поля, иначе
                        // закрывала бы половину значков.
                        .autofocus(!ShellCtx::get().is_phone())
                        .on_change(move |t| q.set(t.to_string()))
                        .class("start-search-field grow"),
                ),
        )
        .class("start-search")
}

fn results(ctx: ShellCtx, st: St) -> impl Widget {
    let items = st.results.get_untracked();
    let sel = st.selected;
    let mut col = Column::new().gap(2.0).child(Text::new("Лучшие совпадения").class("start-caption"));
    if items.is_empty() {
        col = col.child(Text::new("Ничего не найдено").class("start-empty"));
    }
    for (i, it) in items.into_iter().enumerate() {
        let (name, sub) = launcher::item_text(&it);
        let mut text = Column::new().gap(0.0).child(Text::new(name).max_lines(1).class("start-row-name"));
        if !sub.is_empty() {
            text = text.child(Text::new(sub).max_lines(1).class("start-row-sub"));
        }
        let it2 = it.clone();
        let row = rx(move || {
            let class = if sel.get() == i { "start-row start-row-selected" } else { "start-row" };
            Box::new(DecoratedBox::new().class(class))
        });
        let content = Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(launcher::item_icon(&it, "start-row-icon"))
            .child(text.class("grow"))
            .class("start-row-body");
        col = col.child(
            GestureDetector::new()
                .on_hover_change(move |h| {
                    if h {
                        sel.set(i);
                    }
                })
                .on_click(move || it2.activate(ShellCtx::get()))
                .child(Stack::new().child(row).child(content)),
        );
    }
    let _ = ctx;
    ScrollView::new().vertical().child(col).class("start-scroll")
}

// ─── Закреплённые и рекомендуемые ────────────────────────────────────────────

fn pinned_ids(ctx: &ShellCtx) -> Vec<String> {
    ctx.cfg().launcher.favorites.clone()
}

fn home(ctx: ShellCtx, st: St, body_h: f32) -> impl Widget {
    let cols = columns(&ctx);
    let phone = ctx.is_phone();
    // Телефон: не больше половины тела, чтобы рекомендуемые были видны.
    let rows = if phone { phone_rows(body_h * 0.5 - 40.0).min(4) } else { 3 };
    let per_page = cols * rows;
    let apps: Vec<DesktopEntry> = pinned_ids(&ctx).iter().filter_map(|id| xdg::app_by_id(id)).collect();
    let header = Row::new()
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new("Закреплённые").class("start-heading grow"))
        .child(pill("Все", mi::CHEVRON_RIGHT, move || st.view.set(View::All)));
    let pinned: Box<dyn Widget> = if apps.is_empty() {
        Box::new(Text::new("Закрепите приложения из списка «Все» — правый щелчок или удержание по значку.").class("start-empty"))
    } else {
        let paged = apps.len() > per_page;
        // Телефон: высота — по занятым строкам, без пустоты под одним рядом.
        let phone_h = phone.then(|| {
            let used = apps.len().div_ceil(cols).min(rows) as f32;
            used * (PHONE_TILE_H + PHONE_GRID_GAP) + if paged { INDICATORS_H } else { 0.0 }
        });
        let mut pages = Carousel::new().page_signal(st.page).show_arrows(false).show_indicators(paged);
        for chunk in apps.chunks(per_page) {
            let mut grid = Grid::new(cols).gap(2.0);
            for e in chunk {
                grid = grid.child(tile(ctx, st, e));
            }
            let page = Column::new().main_axis_alignment(MainAxisAlignment::Start).child(grid);
            let page: Box<dyn Widget> = match phone_h {
                Some(h) => Box::new(page.style("height", StyleValue::px(h))),
                None => Box::new(page),
            };
            pages = pages.child(page);
        }
        let pages = pages.class("start-pinned");
        match phone_h {
            Some(h) => Box::new(pages.style("height", StyleValue::px(h))),
            None => Box::new(pages),
        }
    };
    let recent: Vec<DesktopEntry> = launcher::recent().iter().filter_map(|id| xdg::app_by_id(id)).take(if ctx.is_phone() { 4 } else { 6 }).collect();
    let mut rec = Grid::new(if ctx.is_phone() { 1 } else { 2 }).gap(4.0);
    for e in &recent {
        rec = rec.child(recent_row(ctx, st, e));
    }
    let mut col = Column::new().gap(10.0).child(header).child(pinned);
    if !recent.is_empty() && ctx.cfg().launcher.show_recent {
        col = col.child(Text::new("Рекомендуемые").class("start-heading")).child(rec);
    }
    ScrollView::new().vertical().child(col).class("start-scroll")
}

/// Телефон, вид `pages`: все приложения по алфавиту значками, страницы
/// во всю высоту тела листаются пальцем.
fn app_pages(ctx: ShellCtx, st: St, body_h: f32) -> impl Widget {
    let cols = columns(&ctx);
    let mut apps: Vec<DesktopEntry> = xdg::apps().iter().filter(|e| !e.no_display).cloned().collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    let rows = phone_rows(body_h - INDICATORS_H);
    let per_page = cols * rows;
    let mut pages = Carousel::new().page_signal(st.page).show_arrows(false).show_indicators(apps.len() > per_page);
    for chunk in apps.chunks(per_page) {
        let mut grid = Grid::new(cols).gap(PHONE_GRID_GAP);
        for e in chunk {
            grid = grid.child(tile(ctx, st, e));
        }
        pages = pages.child(Column::new().main_axis_alignment(MainAxisAlignment::Start).child(grid).style("height", StyleValue::px(body_h)));
    }
    pages.class("start-pages").style("height", StyleValue::px(body_h))
}

fn pill(label: &str, glyph: &str, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    GestureDetector::new().on_click(f).child(
        DecoratedBox::new()
            .child(
                Row::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(label.to_string()).class("start-pill-text"))
                    .child(icon(glyph).class("start-pill-icon")),
            )
            .class("start-pill"),
    )
}

fn open_menu(st: St, id: String) {
    // Точка — у последнего нажатия, в координатах поверхности «Пуска».
    let p = syngui::input::last_press().unwrap_or_default();
    st.item_menu.set(Some((id, p.x, p.y)));
}

/// Значок в сетке закреплённых.
fn tile(ctx: ShellCtx, st: St, e: &DesktopEntry) -> impl Widget {
    let l = Launchable::from_entry(e);
    let e2 = e.clone();
    let (id, id2) = (e.id.clone(), e.id.clone());
    GestureDetector::new()
        .on_click(move || {
            let ctx = ShellCtx::get();
            launcher::launch(ctx, &e2);
            ctx.close_popup();
        })
        .on_secondary_click(move |_| open_menu(st, id.clone()))
        .on_long_press(move |_| open_menu(st, id2.clone()))
        .child(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .gap(6.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(crate::launchers::icon_widget(&l.icon, &None, "start-tile-icon", if ctx.is_phone() { 48.0 } else { 36.0 }))
                        .child(Text::new(e.name.clone()).max_lines(2).class("start-tile-name")),
                )
                .class("start-tile"),
        )
}

fn recent_row(ctx: ShellCtx, st: St, e: &DesktopEntry) -> impl Widget {
    let l = Launchable::from_entry(e);
    let e2 = e.clone();
    let (id, id2) = (e.id.clone(), e.id.clone());
    let sub = if !e.generic_name.is_empty() && e.generic_name != e.name { e.generic_name.clone() } else { e.comment.clone() };
    let _ = ctx;
    GestureDetector::new()
        .on_click(move || {
            let ctx = ShellCtx::get();
            launcher::launch(ctx, &e2);
            ctx.close_popup();
        })
        .on_secondary_click(move |_| open_menu(st, id.clone()))
        .on_long_press(move |_| open_menu(st, id2.clone()))
        .child(
            DecoratedBox::new()
                .child(
                    Row::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(crate::launchers::icon_widget(&l.icon, &None, "start-row-icon", 32.0))
                        .child(
                            Column::new()
                                .gap(0.0)
                                .child(Text::new(e.name.clone()).max_lines(1).class("start-row-name"))
                                .child(Text::new(sub).max_lines(1).class("start-row-sub"))
                                .class("grow"),
                        ),
                )
                .class("start-row"),
        )
}

// ─── Все приложения ──────────────────────────────────────────────────────────

/// Буква указателя: первая буква имени в верхнем регистре, цифры и прочее — «#».
fn letter_of(name: &str) -> String {
    match name.chars().next() {
        Some(c) if c.is_alphabetic() => c.to_uppercase().collect(),
        _ => "#".into(),
    }
}

fn all_apps(ctx: ShellCtx, st: St, body_h: f32) -> impl Widget {
    let mut groups: BTreeMap<(u8, String), Vec<DesktopEntry>> = BTreeMap::new();
    for e in xdg::apps().iter().filter(|e| !e.no_display) {
        let l = letter_of(&e.name);
        // Порядок: «#», латиница, кириллица, остальное.
        let rank = match l.chars().next() {
            Some('#') => 0,
            Some(c) if c.is_ascii_alphabetic() => 1,
            Some(c) if ('А'..='Я').contains(&c) || c == 'Ё' => 2,
            _ => 3,
        };
        groups.entry((rank, l)).or_default().push(e.clone());
    }
    let mut list = Column::new().gap(2.0);
    let mut letters = Vec::new();
    for ((_, letter), mut apps) in groups {
        apps.sort_by_key(|a| a.name.to_lowercase());
        list = list.child(Named::new(format!("start-letter-{letter}"), Text::new(letter.clone()).class("start-letter")));
        for e in &apps {
            list = list.child(recent_row(ctx, st, e));
        }
        letters.push(letter);
    }
    let header = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            GestureDetector::new().on_click(move || st.view.set(View::Home)).child(
                DecoratedBox::new()
                    .child(Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(mi::CHEVRON_LEFT).class("start-pill-icon")).child(Text::new("Назад").class("start-pill-text")))
                    .class("start-pill"),
            ),
        )
        .child(Text::new("Все приложения").class("start-heading grow"));
    Column::new()
        .gap(8.0)
        .child(header)
        .child(
            Row::new()
                .gap(6.0)
                .child(ScrollView::new().vertical().child(list).class("start-scroll grow"))
                .child(rail(ctx, letters, body_h))
                .class("grow"),
        )
        .class("grow")
}

/// Высота строки буквы в icon rail: не больше 18 px, меньше — если букв
/// много, чтобы весь алфавит помещался по высоте меню.
fn rail_row(ctx: &ShellCtx, letters: usize, body_h: f32) -> f32 {
    // Телефон: тело минус заголовок «Все приложения» и поля указателя.
    let avail = if ctx.is_phone() { body_h - 50.0 } else { size(ctx).1 - 190.0 }.max(120.0);
    (avail / letters.max(1) as f32).clamp(9.0, 18.0)
}

/// Icon rail: буквы столбиком; тап — прокрутка к букве, ведение пальцем —
/// прокрутка следом за пальцем, текущая буква подсвечивается.
fn rail(ctx: ShellCtx, letters: Vec<String>, body_h: f32) -> impl Widget {
    let active = use_signal(None::<usize>);
    let n = letters.len().max(1);
    let row = rail_row(&ctx, n, body_h);
    let ls = letters.clone();
    let jump = move |y: f32| {
        let i = (y / row).floor().clamp(0.0, (n - 1) as f32) as usize;
        if active.get_untracked() != Some(i) {
            active.set(Some(i));
            if let Some(l) = ls.get(i) {
                scroll_to_named(format!("start-letter-{l}"));
            }
        }
    };
    let (j1, j2) = (jump.clone(), jump);
    let items = rx(move || {
        let a = active.get();
        let mut col = Column::new().gap(0.0);
        for (i, l) in letters.iter().enumerate() {
            let class = if a == Some(i) { "start-rail-letter start-rail-letter-on" } else { "start-rail-letter" };
            col = col.child(
                Text::new(l.clone())
                    .class(class)
                    .style("height", StyleValue::px(row))
                    .style("font-size", StyleValue::px((row * 0.62).clamp(8.0, 11.0))),
            );
        }
        Box::new(col)
    });
    GestureDetector::new()
        .pan_axis(PanAxis::Vertical)
        .on_click_with_bounds(move |p, b| j1(p.y - b.origin.y))
        .on_pan_update(move |u| j2(u.position.y))
        .on_pan_end(move |_| active.set(None))
        .child(DecoratedBox::new().child(items).class("start-rail"))
}

// ─── Меню значка ─────────────────────────────────────────────────────────────

fn item_menu(ctx: ShellCtx, st: St, id: &str, x: f32, y: f32) -> impl Widget {
    let cfg = ctx.cfg();
    let pinned = cfg.launcher.favorites.iter().any(|f| f == id);
    let entry = xdg::app_by_id(id);
    let name = entry.as_ref().map(|e| e.name.clone()).unwrap_or_else(|| id.to_string());
    let close = move || st.item_menu.set(None);
    let mut col = Column::new().gap(2.0).child(Text::new(name).max_lines(1).class("popup-title"));
    if let Some(e) = entry.clone() {
        col = col.child(crate::popup::menu_item("\u{E89E}", "Открыть", move || {
            let ctx = ShellCtx::get();
            launcher::launch(ctx, &e);
            ctx.close_popup();
        }));
    }
    let i1 = id.to_string();
    col = col.child(crate::popup::menu_item(
        mi::PUSH_PIN,
        if pinned { "Открепить от «Пуска»" } else { "Закрепить в «Пуске»" },
        move || {
            set_pinned(&i1, !pinned);
            close();
        },
    ));
    let dock = cfg.panels.iter().enumerate().find(|(_, p)| p.is_dock() && p.shows_on(ctx.form_factor)).map(|(i, _)| i);
    if let Some(panel) = dock {
        let i2 = id.to_string();
        col = col.child(crate::popup::menu_item("\u{E30C}", "Закрепить на доке", move || {
            crate::edit::pin_app(panel, &i2);
            close();
        }));
    }
    if ctx.is_phone() && !cfg.mobile.home_apps.is_empty() && !cfg.mobile.home_apps.iter().any(|a| a == id) {
        let i3 = id.to_string();
        col = col.child(crate::popup::menu_item(mi::HOME, "На домашний экран", move || {
            let mut apps = ShellCtx::get().cfg().mobile.home_apps.clone();
            apps.push(i3.clone());
            crate::edit::set_home_apps(apps);
        }));
    }
    let (w, _) = size(&ctx);
    let card_w = 240.0;
    let left = (x - 12.0).clamp(8.0, (w - card_w - 8.0).max(8.0));
    let dur = crate::anim::group_ms(&ctx, "menu", 180);
    let menu = Presence::new(
        true,
        InputArea::new(DecoratedBox::new().child(col).class("popup-card start-item-menu").style("width", StyleValue::px(card_w))).absorb(),
    )
    .enter(Motion::fade().scale(0.9))
    .origin(TransformOrigin::TopLeft)
    .duration_ms(dur)
    .initial(dur > 0);
    let backdrop = InputArea::new(DecoratedBox::new().class("start-menu-scrim")).on_press(move |_, _, _| st.item_menu.set(None));
    Stack::new().fit(StackFit::Expand).child(backdrop).child(
        Column::new()
            .child(Row::new().child(menu).style("padding-left", StyleValue::px(left)))
            .style("padding-top", StyleValue::px((y - 8.0).max(8.0))),
    )
}

fn set_pinned(id: &str, pin: bool) {
    let mut fav = ShellCtx::get().cfg().launcher.favorites.clone();
    fav.retain(|f| f != id);
    if pin {
        fav.push(id.to_string());
    }
    let arr = toml_edit::Value::Array(fav.iter().map(|a| toml_edit::Value::from(a.as_str())).collect());
    if let Err(e) = synshell_common::config_edit::set_value(&["launcher", "favorites"], arr) {
        log::error!("«Пуск»: {e:#}");
    }
    crate::reload_after_write();
}

// ─── Низ ─────────────────────────────────────────────────────────────────────

fn avatar_path() -> Option<std::path::PathBuf> {
    let user = std::env::var("USER").unwrap_or_default();
    [
        synshell_common::paths::expand_tilde("~/.face"),
        synshell_common::paths::expand_tilde("~/.face.icon"),
        std::path::PathBuf::from(format!("/var/lib/AccountsService/icons/{user}")),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

/// Переключить вид «Пуска» на телефоне и запомнить в `[mobile] launcher`.
fn set_pages(st: St, pages: bool) {
    st.pages.set(pages);
    st.page.set(0);
    st.view.set(View::Home);
    if let Err(e) = synshell_common::config_edit::set_value(&["mobile", "launcher"], toml_edit::Value::from(if pages { "pages" } else { "list" })) {
        log::error!("«Пуск»: {e:#}");
    }
    crate::reload_after_write();
}

fn footer(ctx: ShellCtx, st: St) -> impl Widget {
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    let avatar: Box<dyn Widget> = match avatar_path() {
        Some(p) => Box::new(Image::new(p.to_string_lossy()).fit(ImageFit::Cover).class("start-avatar")),
        None => Box::new(
            DecoratedBox::new()
                .child(Text::new(user.chars().next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default()).class("start-avatar-letter"))
                .class("start-avatar"),
        ),
    };
    let btn = |glyph: &'static str, f: fn()| {
        GestureDetector::new().on_click(f).child(DecoratedBox::new().child(icon(glyph).class("start-footer-icon")).class("start-footer-btn"))
    };
    let mut row = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(avatar)
        .child(Text::new(user).class("start-user grow"));
    if ctx.is_phone() {
        // Вид: значки по страницам ⇄ список.
        let toggle = rx(move || {
            let pages = st.pages.get();
            Box::new(
                GestureDetector::new()
                    .on_click(move || set_pages(st, !pages))
                    .child(DecoratedBox::new().child(icon(if pages { mi::LIST } else { mi::GRID }).class("start-footer-icon")).class("start-footer-btn")),
            )
        });
        row = row.child(toggle);
    }
    row
        .child(btn(mi::SETTINGS, || {
            ShellCtx::get().close_popup();
            crate::actions::spawn("synsettings");
        }))
        .child(btn(mi::POWER, || {
            let c = ShellCtx::get();
            c.popup.set(Some(crate::ctx::Popup {
                kind: crate::ctx::PopupKind::Power,
                anchor: crate::ctx::PopupAnchor { output: None, rect: None, edge: synshell_common::config::Edge::Bottom, attached: false },
            }));
        }))
        .class("start-footer")
}

