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
//! Приложения Android (syndroid) — отдельно от программ Linux: под поиском чипы «Linux» и «Android»
//! (по чипу на каждый экземпляр Android, если их несколько); «Все приложения» и страницы телефона
//! показывают выбранный источник, в смешанных местах (закреплённые, рекомендуемые) у значка Android —
//! метка, в поиске — подпись «Android · LineageOS 20.0».
//!
//! Правый щелчок или удержание по значку — меню значка прямо внутри
//! «Пуска»: закрепить/открепить, на док, на домашний экран (телефон).
//! На рабочем столе — карточка у кнопки, на телефоне — лист почти на весь
//! экран. Все переходы — перетекания (`AnimatedSwitcher`, `Presence`).

use std::collections::BTreeMap;
use syngui::input::Key;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::containers::Positioned;
use syngui::widgets::containers::scroll_to_named;
use syngui::widgets::{Carousel, PanAxis};
use syngui::{GestureDetector, Named};
use syngui_layer::KeyInfo;

use crate::ctx::ShellCtx;
use crate::launcher::{self, Item};
use crate::launchers::Launchable;
use crate::ui::{icon, mi, rx, InputArea};
use crate::xdg::{self, DesktopEntry};

/// Чьи приложения показывает меню: программы Linux или экземпляр Android (`xdg::AndroidOrigin::instance`).
#[derive(Clone, PartialEq, Eq, Debug)]
enum Source {
    Linux,
    Android(String),
}

impl Source {
    fn matches(&self, e: &DesktopEntry) -> bool {
        match (self, &e.android) {
            (Source::Linux, None) => true,
            (Source::Android(i), Some(a)) => a.instance == *i,
            _ => false,
        }
    }
}

/// Видимые приложения источника.
fn source_apps(source: &Source) -> Vec<DesktopEntry> {
    xdg::apps().iter().filter(|e| !e.no_display && source.matches(e)).cloned().collect()
}

/// Высота ряда чипов источников.
const CHIPS_H: f32 = 40.0;
/// Строка чипов (высота чипа) и зазор между строками, когда они переносятся.
const CHIP_ROW_H: f32 = 34.0;
const CHIP_GAP: f32 = 6.0;

/// Подписи чипов источников: «Linux» и Android (несколько экземпляров — по названию каждого).
fn chip_labels(instances: &[xdg::AndroidOrigin]) -> Vec<String> {
    let mut v = vec!["Linux".to_string()];
    for i in instances {
        v.push(if instances.len() > 1 { i.title.clone() } else { "Android".into() });
    }
    v
}

/// Сколько строк займут чипы в ширине `avail`. Ширина подписи — по числу знаков с запасом (шрифт 13 px):
/// ошибка в большую сторону даёт лишь зазор, в меньшую — низ меню уехал бы за край.
fn chip_rows(labels: &[String], avail: f32) -> usize {
    let (mut rows, mut x) = (1, 0.0f32);
    for l in labels {
        // поля 10 + 14, значок 18, зазор 6, рамка 2
        let w = 50.0 + 7.8 * l.chars().count() as f32;
        if x > 0.0 && x + CHIP_GAP + w > avail {
            rows += 1;
            x = w;
        } else {
            x += if x > 0.0 { CHIP_GAP + w } else { w };
        }
    }
    rows
}

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
    /// Linux или экземпляр Android.
    source: RwSignal<Source>,
    /// Рабочий стол: открыты настройки меню.
    settings: RwSignal<bool>,
    /// Открытая группа закреплённых (id).
    group: RwSignal<Option<String>>,
    /// Имя открытой группы правится.
    renaming: RwSignal<bool>,
    /// Меню значка открыто из группы (id): «Вынуть из группы».
    menu_group: RwSignal<Option<String>>,
    /// «Объединить в группу…»: выбран первый значок, ждём второй.
    merge_src: RwSignal<Option<String>>,
    /// Перетаскивание: над каким значком и куда ляжет.
    hint: RwSignal<Option<(String, Zone)>>,
}

/// Куда ляжет перетаскиваемый значок: перед целью, в группу с ней, после неё.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Zone {
    Before,
    Merge,
    After,
}

/// Средняя часть значка — объединить, края — переставить.
fn zone_of(x: f32, w: f32) -> Zone {
    let r = if w > 0.0 { x / w } else { 0.5 };
    if r < 0.28 {
        Zone::Before
    } else if r > 0.72 {
        Zone::After
    } else {
        Zone::Merge
    }
}

/// Тип переноса: закреплённый значок и значок внутри группы.
const PIN_DRAG: &str = "synshell/start-pin";
const GROUP_DRAG: &str = "synshell/start-group-item";

/// Телефон: высота значка в сетке (отступы, значок, две строки подписи) и
/// зазор между строками.
const PHONE_TILE_H: f32 = 104.0;
/// Рабочий стол: значок 36 и две строки подписи с полями `.start-tile`.
const DESK_TILE_H: f32 = 92.0;
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
        let (mw, mh) = crate::menu_prefs::min_size();
        ((cfg.launcher.width as f32).max(mw), (cfg.launcher.height as f32).max(mh).min(oh - 80.0))
    }
}

/// Высота шапки меню горизонтально: поиск, а кнопки низа — в той же строке.
const CHROME_H_WIDE: f32 = 58.0 + 14.0;
/// Телефон: ширина колонки значков (горизонтально колонок больше, а не шире значки).
const PHONE_TILE_W: f32 = 96.0;

/// Колонок в сетке закреплённых.
fn columns(ctx: &ShellCtx) -> usize {
    if ctx.is_phone() {
        let (ow, _) = crate::manager::output_size(crate::manager::focused_output(ctx).as_deref());
        (((ow - 60.0) / PHONE_TILE_W).floor() as usize).clamp(4, 10)
    } else {
        ctx.cfg().launcher.columns.clamp(3, 10) as usize
    }
}

pub fn view(ctx: ShellCtx) -> Box<dyn Widget> {
    let st = St {
        query: use_signal(String::new()),
        view: use_signal(View::Home),
        selected: use_signal(0usize),
        results: use_signal(Vec::new()),
        item_menu: use_signal(None),
        page: use_signal(0usize),
        pages: use_signal(ctx.is_phone() && ctx.cfg().mobile.launcher != "list"),
        source: use_signal(Source::Linux),
        settings: use_signal(!ctx.is_phone() && crate::menu_prefs::take_reopen_settings()),
        group: use_signal(None),
        renaming: use_signal(false),
        menu_group: use_signal(None),
        merge_src: use_signal(None),
        hint: use_signal(None),
    };
    // Рабочий стол: размер меню — уголком, живьём.
    let msize = crate::menu_prefs::size_signal(&ctx);
    let instances = xdg::android_instances(&xdg::apps());
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
        if k.key == Key::Escape && st.renaming.get_untracked() {
            st.renaming.set(false);
            return true;
        }
        if k.key == Key::Escape && st.group.get_untracked().is_some() {
            st.group.set(None);
            return true;
        }
        if k.key == Key::Escape && st.merge_src.get_untracked().is_some() {
            st.merge_src.set(None);
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
    // Меню значка — слоем ровно в размер «Пуска» (иначе слой растягивал карточку),
    // точка нажатия — в координатах меню (по его границам на поверхности).
    let bounds: std::sync::Arc<syngui::core::sync::Mutex<syngui::core::Rect>> = Default::default();
    let b2 = bounds.clone();
    let menu_layer = rx(move || match st.item_menu.get() {
        Some((id, x, y)) => {
            let o = b2.lock().map(|r| r.origin).unwrap_or_default();
            let (w, h) = if ctx.is_phone() { size(&ctx) } else { msize.get_untracked() };
            Box::new(item_menu(ctx, st, &id, x - o.x, y - o.y).style("width", StyleValue::px(w - 24.0)).style("height", StyleValue::px(h)))
        }
        None => Box::new(DecoratedBox::new()),
    });
    let group_layer = rx(move || match st.group.get() {
        Some(gid) => {
            let (w, h) = if ctx.is_phone() { size(&ctx) } else { msize.get_untracked() };
            Box::new(group_view(ctx, st, gid).style("width", StyleValue::px(w - 24.0)).style("height", StyleValue::px(h)))
        }
        None => Box::new(DecoratedBox::new()),
    });
    let phone = ctx.is_phone();
    // Телефон: высота — по месту поверхности (между панелями и клавиатурой),
    // с запасом сверху, и меняется, когда клавиатура выезжает.
    let sized = rx(move || {
        let wide = crate::manager::is_landscape(&ctx);
        let (w, h) = if phone {
            // Лист `.popup-sheet` — не выше 86 % (горизонтально — почти во всю высоту, см.
            // popup.rs) и с отступами 18 + 20 px: меню меньше на них, иначе низ срезался.
            let vp = syngui::viewport::viewport_size().get();
            let share = if wide { vp.height - 16.0 } else { vp.height * 0.86 };
            (vp.width - 20.0, (share - 40.0).max(if wide { 200.0 } else { 300.0 }))
        } else {
            msize.get()
        };
        if !phone && st.settings.get() {
            let close = move || st.settings.set(false);
            return Box::new(Column::new().child(crate::menu_prefs::view(ctx, msize, close)).class("start").style("height", StyleValue::px(h)).style("width", StyleValue::px(w - 24.0)));
        }
        let chips_h = if instances.is_empty() {
            0.0
        } else {
            // меню: ширина листа минус поля `.start`
            let rows = chip_rows(&chip_labels(&instances), w - 24.0 - 8.0);
            CHIPS_H + (rows - 1) as f32 * (CHIP_ROW_H + CHIP_GAP)
        };
        // Горизонтально высоты мало: кнопки низа — справа от поиска, а не отдельной строкой.
        let top: Box<dyn Widget> = if wide {
            Box::new(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(search(st).class("grow")).child(footer(ShellCtx::get(), st, true)))
        } else {
            Box::new(search(st))
        };
        let mut col = Column::new().gap(14.0).child(top);
        if !instances.is_empty() {
            col = col.child(chips(st, instances.clone()));
        }
        let chrome = if wide { CHROME_H_WIDE } else { CHROME_H };
        col = col.child(body_ref(st, ctx, h - chrome - chips_h));
        if !wide {
            col = col.child(footer(ShellCtx::get(), st, false));
        }
        let menu: Box<dyn Widget> = Box::new(
            col
                .class("start")
                .style("height", StyleValue::px(h))
                .style("width", StyleValue::px(w - 24.0)),
        );
        menu
    });
    let _ = w;
    // Меню значка — поверх «Пуска» внутри его границ; уголок — последним слоем, иначе
    // слой меню значка (на всю площадь) забирал бы его нажатия.
    let body = syngui::widgets::EventHook::new().report_bounds(bounds).child(Stack::new().child(sized).child(group_layer).child(menu_layer));
    if phone {
        Box::new(body) as Box<dyn Widget>
    } else {
        Box::new(crate::menu_prefs::with_grip(ctx, msize, 24.0, body))
    }
}

/// Середина меню: закреплённые, «Все приложения» или результаты поиска —
/// перетекают друг в друга.
fn body_ref(st: St, ctx: ShellCtx, body_h: f32) -> impl Widget {
    let dur = crate::anim::group_ms(&ctx, "menu", 260);
    rx(move || {
        let searching = !st.query.get().trim().is_empty();
        let v = st.view.get();
        let pages = st.pages.get();
        let source = st.source.get();
        // Поставили или удалили программу — пересобрать списки.
        let rev = ctx.apps_rev.get().wrapping_add(ctx.pins_rev.get().wrapping_mul(0x9E37_79B9));
        // Закрепили, открепили, сгруппировали, сменили колонки — [launcher] перечитан на лету;
        // при том же ключе переключатель пересобирает содержимое только по смене версии.
        let _ = ctx.config.get();
        let launcher_rev = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            format!("{:?}", ctx.cfg().launcher).hash(&mut h);
            h.finish()
        };
        // Ключ задаёт направление перетекания: дальше по списку — въезд справа.
        let key = if searching { 3 } else if pages { 4 } else if v == View::All { 2 } else { 1 };
        // Смена источника — тоже перетекание
        let key = key + 10 * source_key(&source);
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
        .version(rev ^ launcher_rev)
        .directional(true)
        .slide(36.0, 0.0)
        .duration_ms(dur)
        .exit_duration_ms(dur * 2 / 3)
        .animate_size(false)
        .class("start-body");
        // Телефон: тело — в колонке явной высоты (`flex-grow` не доходит
        // сквозь `rx`, а переключатель высоту из стиля не берёт), иначе
        // прокрутка занимала всё и выталкивала низ за край листа.
        // На рабочем столе так же: иначе тело вылезало за карточку меню.
        Box::new(Column::new().child(sw).style("height", StyleValue::px(body_h)))
    })
}

/// Номер источника для ключа перетекания: Linux — 0, Android — по экземпляру.
fn source_key(s: &Source) -> u64 {
    match s {
        Source::Linux => 0,
        Source::Android(i) => 1 + i.bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64)) % 1000,
    }
}

/// Чипы источников под поиском: «Linux», «Android» (несколько экземпляров — по названию каждого).
fn chips(st: St, instances: Vec<xdg::AndroidOrigin>) -> impl Widget {
    rx(move || {
        let cur = st.source.get();
        let searching = !st.query.get().trim().is_empty();
        let labels = chip_labels(&instances);
        let mut items: Vec<(Source, String, &'static str)> = vec![(Source::Linux, labels[0].clone(), mi::COMPUTER)];
        for (i, label) in instances.iter().zip(labels.into_iter().skip(1)) {
            items.push((Source::Android(i.instance.clone()), label, mi::ANDROID));
        }
        // Не влезли в ширину меню (несколько экземпляров Android с длинными названиями) — переносятся
        let mut row = Flex::row().gap(CHIP_GAP).wrap().cross_axis_alignment(CrossAxisAlignment::Center);
        for (src, label, glyph) in items {
            let on = cur == src && !searching;
            let android = matches!(src, Source::Android(_));
            let class = match (on, android) {
                (true, true) => "start-chip start-chip-android start-chip-on",
                (true, false) => "start-chip start-chip-on",
                (false, true) => "start-chip start-chip-android",
                (false, false) => "start-chip",
            };
            row = row.child(
                GestureDetector::new()
                    .on_click(move || {
                        st.source.set(src.clone());
                        st.page.set(0);
                        // Из закреплённых — сразу в список источника
                        if st.view.get_untracked() == View::Home {
                            st.view.set(View::All);
                        }
                    })
                    .child(
                        DecoratedBox::new()
                            .child(
                                Row::new()
                                    .gap(6.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .child(icon(glyph).class("start-chip-icon"))
                                    .child(Text::new(label).max_lines(1).class("start-chip-text")),
                            )
                            .class(class),
                    ),
            );
        }
        Box::new(row.class("start-chips"))
    })
}

/// Значок приложения; `badge` — у приложений Android метка Android в углу (там, где они рядом с
/// программами Linux: закреплённые, рекомендуемые).
fn app_icon(e: &DesktopEntry, class: &str, size: f32, badge: bool) -> Box<dyn Widget> {
    let l = Launchable::from_entry(e);
    let ic = crate::launchers::icon_widget(&l.icon, &None, class, size);
    if e.android.is_none() || !badge {
        return ic;
    }
    let b = (size * 0.42).round();
    Box::new(
        Stack::new()
            .child(ic)
            .child(Positioned::new(DecoratedBox::new().child(icon(mi::ANDROID).class("start-android-badge-icon")).class("start-android-badge")).at(size - b * 0.8, size - b * 0.8))
            .style("width", StyleValue::px(size))
            .style("height", StyleValue::px(size)),
    )
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
                        .placeholder(if ShellCtx::get().is_phone() { t!("Поиск приложений и настроек") } else { t!("Поиск приложений, настроек, вычислений") })
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
    let mut col = Column::new().gap(2.0).child(Text::new(t!("Лучшие совпадения")).class("start-caption"));
    if items.is_empty() {
        col = col.child(Text::new(t!("Ничего не найдено")).class("start-empty"));
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

fn home(ctx: ShellCtx, st: St, body_h: f32) -> impl Widget {
    let cols = columns(&ctx);
    let phone = ctx.is_phone();
    // Телефон: не больше половины тела, чтобы рекомендуемые были видны.
    let rows = if phone { phone_rows(body_h * 0.5 - 40.0).min(4) } else { 3 };
    let per_page = cols * rows;
    let pins = crate::pins::Pins::from_cfg(&ctx.cfg().launcher);
    // Метки, которые есть что показать: приложение установлено, в группе есть установленные.
    let tokens: Vec<String> = pins
        .fav
        .iter()
        .filter(|t| match crate::pins::group_id(t) {
            Some(id) => pins.groups.get(id).is_some_and(|g| g.items.iter().any(|a| xdg::app_by_id(a).is_some())),
            None => xdg::app_by_id(t).is_some(),
        })
        .cloned()
        .collect();
    let header = Row::new()
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(t!("Закреплённые")).class("start-heading grow"))
        .child(pill(&t!("Все"), mi::CHEVRON_RIGHT, move || st.view.set(View::All)));
    let pinned: Box<dyn Widget> = if tokens.is_empty() {
        Box::new(Text::new(t!("Закрепите приложения из списка «Все» — правый щелчок или удержание по значку.")).class("start-empty"))
    } else {
        let paged = tokens.len() > per_page;
        // Высота — по занятым строкам, без пустоты под одним рядом (и на рабочем столе:
        // раньше там было 300 px при любом числе значков).
        let tile_h = if phone { PHONE_TILE_H } else { DESK_TILE_H };
        let pinned_h = Some({
            let used = tokens.len().div_ceil(cols).min(rows) as f32;
            used * (tile_h + PHONE_GRID_GAP) + if paged { INDICATORS_H } else { 0.0 }
        });
        let mut pages = Carousel::new().page_signal(st.page).show_arrows(false).show_indicators(paged);
        for chunk in tokens.chunks(per_page) {
            let mut grid = Grid::new(cols).gap(2.0);
            for t in chunk {
                grid = grid.child(pin_tile(ctx, st, &pins, t));
            }
            let page = Column::new().main_axis_alignment(MainAxisAlignment::Start).child(grid);
            let page: Box<dyn Widget> = match pinned_h {
                Some(h) => Box::new(page.style("height", StyleValue::px(h))),
                None => Box::new(page),
            };
            pages = pages.child(page);
        }
        let pages = pages.class("start-pinned");
        match pinned_h {
            Some(h) => Box::new(pages.style("height", StyleValue::px(h))),
            None => Box::new(pages),
        }
    };
    let recent: Vec<DesktopEntry> = launcher::recent().iter().filter_map(|id| xdg::app_by_id(id)).take(if ctx.is_phone() { 4 } else { 6 }).collect();
    let mut rec = Grid::new(if ctx.is_phone() { 1 } else { 2 }).gap(4.0);
    for e in &recent {
        rec = rec.child(recent_row(ctx, st, e, true));
    }
    let mut col = Column::new().gap(10.0).child(header).child(merge_banner(st)).child(pinned);
    if !recent.is_empty() && ctx.cfg().launcher.show_recent {
        col = col.child(Text::new(t!("Рекомендуемые")).class("start-heading")).child(rec);
    }
    ScrollView::new().vertical().child(col).class("start-scroll")
}

/// Телефон, вид `pages`: все приложения по алфавиту значками, страницы
/// во всю высоту тела листаются пальцем.
fn app_pages(ctx: ShellCtx, st: St, body_h: f32) -> impl Widget {
    let cols = columns(&ctx);
    let mut apps = source_apps(&st.source.get_untracked());
    apps.sort_by_key(|a| a.name.to_lowercase());
    let rows = phone_rows(body_h - INDICATORS_H);
    let per_page = cols * rows;
    let mut pages = Carousel::new().page_signal(st.page).show_arrows(false).show_indicators(apps.len() > per_page);
    for chunk in apps.chunks(per_page) {
        let mut grid = Grid::new(cols).gap(PHONE_GRID_GAP);
        for e in chunk {
            grid = grid.child(tile(ctx, st, e, false));
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
fn tile(ctx: ShellCtx, st: St, e: &DesktopEntry, mixed: bool) -> impl Widget {
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
                        .child(app_icon(e, "start-tile-icon", if ctx.is_phone() { 48.0 } else { 36.0 }, mixed))
                        .child(Text::new(e.name.clone()).max_lines(2).class("start-tile-name")),
                )
                .class("start-tile"),
        )
}

// ─── Группы закреплённых ─────────────────────────────────────────────────────

/// Имя новой группы — по общей категории приложений (как на телефонах), иначе «Группа».
fn group_name(items: &[String]) -> String {
    const CATS: &[(&str, &str)] = &[
        ("Game", "Игры"),
        ("Development", "Разработка"),
        ("Graphics", "Графика"),
        ("Office", "Офис"),
        ("Audio", "Музыка"),
        ("Video", "Видео"),
        ("AudioVideo", "Мультимедиа"),
        ("Network", "Интернет"),
        ("Education", "Обучение"),
        ("Science", "Наука"),
        ("Settings", "Настройки"),
        ("System", "Система"),
        ("Utility", "Утилиты"),
    ];
    let entries: Vec<DesktopEntry> = items.iter().filter_map(|a| xdg::app_by_id(a)).collect();
    CATS.iter()
        .find(|(c, _)| !entries.is_empty() && entries.iter().all(|e| e.categories.iter().any(|x| x == c)))
        .map(|(_, name)| t!(name).to_string())
        .unwrap_or_else(|| t!("Группа").to_string())
}

/// Изменить закреплённые и записать.
fn edit_pins(f: impl FnOnce(&mut crate::pins::Pins) -> bool) {
    let mut p = crate::pins::Pins::from_cfg(&ShellCtx::get().cfg().launcher);
    if f(&mut p) {
        p.save();
    }
}

/// Бросили или выбрали `src` на `dst` в зоне `zone`.
fn apply_drop(src: &str, dst: &str, zone: Zone) {
    edit_pins(|p| match zone {
        Zone::Merge => p.merge(src, dst, group_name).is_some(),
        Zone::Before => p.reorder(src, dst, false),
        Zone::After => p.reorder(src, dst, true),
    });
}

/// «Объединить в группу…» выбран: подсказка и отмена над закреплёнными.
fn merge_banner(st: St) -> impl Widget {
    rx(move || match st.merge_src.get() {
        Some(src) => {
            let name = crate::pins::group_id(&src)
                .and_then(|id| synshell_common::config::load_launcher_groups().get(id).map(|g| g.name.clone()))
                .or_else(|| xdg::app_by_id(&src).map(|e| e.name))
                .unwrap_or(src);
            Box::new(
                DecoratedBox::new()
                    .child(
                        Row::new()
                            .gap(8.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(icon(mi::GROUP_ADD).class("start-merge-icon"))
                            .child(Text::new(t!("Выберите значок, с которым объединить «{name}»", name = name)).max_lines(2).class("start-merge-text grow"))
                            .child(pill(&t!("Отмена"), mi::CLOSE, move || st.merge_src.set(None))),
                    )
                    .class("start-merge"),
            )
        }
        None => Box::new(DecoratedBox::new()),
    })
}

/// Значок закреплённого: приложение или группа. На рабочем столе перетаскивается:
/// на середину другого значка — в группу с ним, на край — перед ним или после.
fn pin_tile(ctx: ShellCtx, st: St, pins: &crate::pins::Pins, token: &str) -> Box<dyn Widget> {
    let phone = ctx.is_phone();
    let size = if phone { 48.0 } else { 36.0 };
    let name = match crate::pins::group_id(token) {
        Some(id) => pins.groups.get(id).map(|g| g.name.clone()).unwrap_or_default(),
        None => match xdg::app_by_id(token) {
            Some(e) => e.name,
            None => return Box::new(DecoratedBox::new()),
        },
    };
    let tok = token.to_string();
    // Внешний вид реагирует на подсказку переноса и режим «объединить»; сама плитка — нет.
    let t1 = tok.clone();
    let look = rx(move || {
        let hint = st.hint.get();
        let picking = st.merge_src.get();
        let mut class = String::from("start-tile");
        match hint {
            Some((ref t, z)) if *t == t1 => class.push_str(match z {
                Zone::Merge => " start-tile-merge",
                Zone::Before => " start-tile-before",
                Zone::After => " start-tile-after",
            }),
            _ => {}
        }
        if let Some(src) = picking {
            class.push_str(if src == t1 { " start-tile-source" } else { " start-tile-pick" });
        }
        let f = pin_face(&t1, size);
        Box::new(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .gap(6.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(f)
                        .child(Text::new(name.clone()).max_lines(2).class("start-tile-name")),
                )
                .class(class),
        )
    });
    let t2 = tok.clone();
    let activate = move || {
        if let Some(src) = st.merge_src.get_untracked() {
            st.merge_src.set(None);
            if src != t2 {
                apply_drop(&src, &t2, Zone::Merge);
            }
            return;
        }
        match crate::pins::group_id(&t2) {
            Some(id) => {
                st.renaming.set(false);
                st.group.set(Some(id.to_string()));
            }
            None => {
                if let Some(e) = xdg::app_by_id(&t2) {
                    let ctx = ShellCtx::get();
                    launcher::launch(ctx, &e);
                    ctx.close_popup();
                }
            }
        }
    };
    let (m1, m2) = (tok.clone(), tok.clone());
    if phone {
        // Телефон: перенос мешал бы листать страницы — объединение через меню значка.
        return Box::new(
            GestureDetector::new()
                .on_click(activate)
                .on_secondary_click(move |_| open_menu(st, m1.clone()))
                .on_long_press(move |_| open_menu(st, m2.clone()))
                .child(look),
        );
    }
    let drag = Draggable::new(PIN_DRAG, tok.clone()).threshold(6.0).label(tok.clone()).on_click(activate).child(look);
    let (t3, t4, t5) = (tok.clone(), tok.clone(), tok.clone());
    Box::new(
        DropArea::new()
            .accept_types(vec![PIN_DRAG.to_string()])
            .on_drag_over(move |info| {
                let z = if info.data.payload == t3 { None } else { Some((t3.clone(), zone_of(info.local_position.x, info.size.width))) };
                if st.hint.get_untracked() != z {
                    st.hint.set(z);
                }
            })
            .on_drag_leave(move || {
                if st.hint.get_untracked().is_some_and(|(t, _)| t == t4) {
                    st.hint.set(None);
                }
            })
            .on_drop_positioned(move |info| {
                st.hint.set(None);
                let src = info.data.payload.clone();
                if src != t5 {
                    apply_drop(&src, &t5, zone_of(info.local_position.x, info.size.width));
                }
            })
            .child(GestureDetector::new().on_secondary_click(move |_| open_menu(st, m1.clone())).child(drag)),
    )
}

/// Значок метки: приложение или превью группы 2×2.
fn pin_face(token: &str, size: f32) -> Box<dyn Widget> {
    match crate::pins::group_id(token) {
        Some(id) => {
            let g = synshell_common::config::load_launcher_groups().get(id).cloned().unwrap_or_default();
            let entries: Vec<crate::launchers::StackEntry> =
                g.items.iter().filter_map(|a| xdg::app_by_id(a)).map(|e| crate::launchers::StackEntry::App(Launchable::from_entry(&e))).collect();
            crate::launchers::group_preview(&entries, size)
        }
        None => xdg::app_by_id(token).map(|e| app_icon(&e, "start-tile-icon", size, true)).unwrap_or_else(|| Box::new(DecoratedBox::new())),
    }
}

/// Открытая группа: имя (правится по щелчку), значки (переставляются
/// перетаскиванием, брошенный за пределы карточки — вынимается из группы).
fn group_view(ctx: ShellCtx, st: St, gid: String) -> DecoratedBox {
    let _ = ctx.pins_rev.get();
    let Some(g) = synshell_common::config::load_launcher_groups().get(&gid).cloned() else {
        st.group.set(None);
        return DecoratedBox::new();
    };
    let phone = ctx.is_phone();
    let close = move || {
        st.group.set(None);
        st.renaming.set(false);
    };
    let g1 = gid.clone();
    let title: Box<dyn Widget> = if st.renaming.get() {
        let g2 = gid.clone();
        Box::new(
            TextField::with_text(g.name.clone())
                .autofocus(true)
                .submit_on_focus_lost(true)
                .on_submit(move |v| {
                    let v = v.to_string();
                    let g = g2.clone();
                    st.renaming.set(false);
                    edit_pins(|p| p.rename(&g, &v));
                })
                .class("start-group-field grow"),
        )
    } else {
        Box::new(
            GestureDetector::new()
                .on_click(move || st.renaming.set(true))
                .child(
                    DecoratedBox::new()
                        .child(
                            Row::new()
                                .gap(8.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(Text::new(g.name.clone()).max_lines(1).class("start-group-title"))
                                .child(icon(mi::EDIT).class("start-group-edit")),
                        )
                        .class("start-group-title-box"),
                )
                .class("grow"),
        )
    };
    let cols = g.items.len().clamp(1, if phone { 4 } else { 5 });
    let mut grid = Grid::new(cols).gap(2.0);
    for a in &g.items {
        let Some(e) = xdg::app_by_id(a) else { continue };
        grid = grid.child(group_item(ctx, st, &g1, &e));
    }
    let card = InputArea::new(
        DecoratedBox::new()
            .child(
                Column::new()
                    .gap(12.0)
                    .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(title))
                    .child(grid)
                    .child(Text::new(if phone { t!("Удерживайте значок, чтобы вынуть его из группы") } else { t!("Перетащите значок за пределы группы, чтобы вынуть его") }).max_lines(2).class("start-group-hint")),
            )
            .class("popup-card start-group")
            .style("width", StyleValue::px(if phone { 320.0 } else { (cols as f32 * 104.0 + 40.0).max(300.0) })),
    )
    .absorb();
    let dur = crate::anim::group_ms(&ctx, "menu", 200);
    let card = Presence::new(true, card).enter(Motion::fade().scale(0.85)).origin(TransformOrigin::Center).duration_ms(dur).initial(dur > 0);
    let g3 = gid.clone();
    // Затемнение (как у шторки — коробкой прямо в слое), над ним щелчок — закрыть,
    // выше — приём переноса: брошенный за карточку значок вынимается из группы.
    let scrim = DecoratedBox::new().class("start-group-scrim");
    let click = InputArea::new(DecoratedBox::new()).on_press(move |_, _, _| close());
    let take_out = DropArea::new()
        .accept_types(vec![GROUP_DRAG.to_string()])
        .on_drop(move |data| {
            let g = g3.clone();
            edit_pins(|p| p.take_out(&g, &data.payload));
        })
        // с ребёнком: без него область рисует заглушку «Drop here»
        .child(DecoratedBox::new());
    DecoratedBox::new().child(
        Stack::new()
            .fit(StackFit::Expand)
            .child(scrim)
            .child(click)
            .child(take_out)
            .child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(card)),
    )
}

/// Значок внутри открытой группы.
fn group_item(ctx: ShellCtx, st: St, gid: &str, e: &DesktopEntry) -> Box<dyn Widget> {
    let phone = ctx.is_phone();
    let e2 = e.clone();
    let launch = move || {
        let ctx = ShellCtx::get();
        launcher::launch(ctx, &e2);
        ctx.close_popup();
    };
    let face = DecoratedBox::new()
        .child(
            Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(app_icon(e, "start-tile-icon", if phone { 48.0 } else { 36.0 }, true))
                .child(Text::new(e.name.clone()).max_lines(2).class("start-tile-name")),
        )
        .class("start-tile start-group-tile");
    let (id1, id2, g1, g2) = (e.id.clone(), e.id.clone(), gid.to_string(), gid.to_string());
    let menu = move |id: String, g: String| {
        st.menu_group.set(Some(g));
        open_menu(st, id);
    };
    let menu2 = menu.clone();
    if phone {
        return Box::new(
            GestureDetector::new()
                .on_click(launch)
                .on_secondary_click(move |_| menu(id1.clone(), g1.clone()))
                .on_long_press(move |_| menu2(id2.clone(), g2.clone()))
                .child(face),
        );
    }
    let (g3, id3) = (gid.to_string(), e.id.clone());
    Box::new(
        DropArea::new()
            .accept_types(vec![GROUP_DRAG.to_string()])
            .on_drop_positioned(move |info| {
                let after = info.local_position.x > info.size.width / 2.0;
                let (g, dst, src) = (g3.clone(), id3.clone(), info.data.payload.clone());
                edit_pins(|p| p.reorder_in_group(&g, &src, &dst, after));
            })
            .child(
                GestureDetector::new()
                    .on_secondary_click(move |_| menu(id1.clone(), g1.clone()))
                    .child(Draggable::new(GROUP_DRAG, e.id.clone()).threshold(6.0).label(e.name.clone()).on_click(launch).child(face)),
            ),
    )
}

fn recent_row(ctx: ShellCtx, st: St, e: &DesktopEntry, mixed: bool) -> impl Widget {
    let e2 = e.clone();
    let (id, id2) = (e.id.clone(), e.id.clone());
    let (_, sub) = launcher::item_text(&Item::App(e.clone()));
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
                        .child(app_icon(e, "start-row-icon", 32.0, mixed))
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
    let source = st.source.get_untracked();
    let title = match &source {
        Source::Linux => t!("Все приложения").to_string(),
        Source::Android(i) => {
            let t = xdg::android_instances(&xdg::apps()).into_iter().find(|a| a.instance == *i).map(|a| a.title).unwrap_or_default();
            format!("Android · {t}")
        }
    };
    for e in source_apps(&source).iter() {
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
            list = list.child(recent_row(ctx, st, e, false));
        }
        letters.push(letter);
    }
    let header = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            GestureDetector::new().on_click(move || st.view.set(View::Home)).child(
                DecoratedBox::new()
                    .child(Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(mi::CHEVRON_LEFT).class("start-pill-icon")).child(Text::new(t!("Назад")).class("start-pill-text")))
                    .class("start-pill"),
            ),
        )
        .child(Text::new(title).class("start-heading grow"));
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

fn item_menu(ctx: ShellCtx, st: St, id: &str, x: f32, y: f32) -> DecoratedBox {
    let cfg = ctx.cfg();
    let close = move || {
        st.item_menu.set(None);
        st.menu_group.set(None);
    };
    // Группа закреплённых — своё меню.
    if let Some(gid) = crate::pins::group_id(id) {
        let name = synshell_common::config::load_launcher_groups().get(gid).map(|g| g.name.clone()).unwrap_or_default();
        let (g1, g2, g3, t4) = (gid.to_string(), gid.to_string(), gid.to_string(), id.to_string());
        let col = Column::new()
            .gap(2.0)
            .child(Text::new(name).max_lines(1).class("popup-title"))
            .child(crate::popup::menu_item("\u{E89E}", t!("Открыть"), move || {
                close();
                st.renaming.set(false);
                st.group.set(Some(g1.clone()));
            }))
            .child(crate::popup::menu_item(mi::EDIT, t!("Переименовать"), move || {
                close();
                st.group.set(Some(g2.clone()));
                st.renaming.set(true);
            }))
            .child(crate::popup::menu_item(mi::GROUP_ADD, t!("Объединить с другим значком…"), {
                let t = t4.clone();
                move || {
                    close();
                    st.merge_src.set(Some(t.clone()));
                }
            }))
            .child(crate::popup::menu_item(mi::UNGROUP, t!("Разгруппировать"), move || {
                close();
                let g = g3.clone();
                edit_pins(|p| {
                    p.dissolve(&g);
                    true
                });
            }))
            .child(crate::popup::menu_item(mi::PUSH_PIN, t!("Открепить группу"), move || {
                close();
                let t = t4.clone();
                edit_pins(|p| {
                    p.unpin(&t);
                    true
                });
            }));
        return menu_card(ctx, st, col, x, y);
    }
    let in_group = st.menu_group.get_untracked();
    let pinned = cfg.launcher.favorite_apps().iter().any(|f| f == id);
    let top_level = cfg.launcher.favorites.iter().any(|f| f == id);
    let entry = xdg::app_by_id(id);
    let name = entry.as_ref().map(|e| e.name.clone()).unwrap_or_else(|| id.to_string());
    let mut col = Column::new().gap(2.0).child(Text::new(name).max_lines(1).class("popup-title"));
    if let Some(e) = entry.clone() {
        col = col.child(crate::popup::menu_item("\u{E89E}", t!("Открыть"), move || {
            let ctx = ShellCtx::get();
            launcher::launch(ctx, &e);
            ctx.close_popup();
        }));
    }
    if let Some(gid) = in_group.clone() {
        let (g, i) = (gid.clone(), id.to_string());
        col = col.child(crate::popup::menu_item(mi::UNGROUP, t!("Вынуть из группы"), move || {
            close();
            let (g, i) = (g.clone(), i.clone());
            edit_pins(|p| p.take_out(&g, &i));
        }));
    } else if top_level && cfg.launcher.favorites.len() > 1 {
        let i = id.to_string();
        col = col.child(crate::popup::menu_item(mi::GROUP_ADD, t!("Объединить в группу…"), move || {
            close();
            st.merge_src.set(Some(i.clone()));
        }));
    }
    let i1 = id.to_string();
    col = col.child(crate::popup::menu_item(
        mi::PUSH_PIN,
        if pinned { t!("Открепить от «Пуска»") } else { t!("Закрепить в «Пуске»") },
        move || {
            set_pinned(&i1, !pinned);
            close();
        },
    ));
    let dock = cfg.panels.iter().enumerate().find(|(_, p)| p.is_dock() && p.shows_on(ctx.form_factor)).map(|(i, _)| i);
    if let Some(panel) = dock {
        let i2 = id.to_string();
        col = col.child(crate::popup::menu_item("\u{E30C}", t!("Закрепить на доке"), move || {
            crate::edit::pin_app(panel, &i2);
            close();
        }));
    }
    if ctx.is_phone() && !cfg.mobile.home_apps.is_empty() && !cfg.mobile.home_apps.iter().any(|a| a == id) {
        let i3 = id.to_string();
        col = col.child(crate::popup::menu_item(mi::HOME, t!("На домашний экран"), move || {
            let mut apps = ShellCtx::get().cfg().mobile.home_apps.clone();
            apps.push(i3.clone());
            crate::edit::set_home_apps(apps);
        }));
    }
    menu_card(ctx, st, col, x, y)
}

/// Карточка меню значка у точки нажатия, поверх «Пуска».
fn menu_card(ctx: ShellCtx, st: St, col: Column, x: f32, y: f32) -> DecoratedBox {
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
    let backdrop = InputArea::new(DecoratedBox::new().class("start-menu-scrim")).on_press(move |_, _, _| {
        st.item_menu.set(None);
        st.menu_group.set(None);
    });
    DecoratedBox::new().child(Stack::new().fit(StackFit::Expand).child(backdrop).child(
        Column::new()
            .child(Row::new().child(menu).style("padding-left", StyleValue::px(left)))
            .style("padding-top", StyleValue::px((y - 8.0).max(8.0))),
    ))
}

/// Закрепить или открепить приложение (открепить — и из группы).
fn set_pinned(id: &str, pin: bool) {
    edit_pins(|p| {
        if pin {
            if p.fav.iter().any(|f| f == id) {
                return false;
            }
            p.fav.push(id.to_string());
        } else {
            p.unpin(id);
            let gids: Vec<String> = p.groups.iter().filter(|(_, g)| g.items.iter().any(|a| a == id)).map(|(k, _)| k.clone()).collect();
            for g in gids {
                p.remove_from_group(&g, id);
            }
        }
        true
    });
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

/// Низ меню: пользователь и кнопки; `compact` — только кнопки, без черты
/// (горизонтально — в строке поиска).
fn footer(ctx: ShellCtx, st: St, compact: bool) -> impl Widget {
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
    let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if !compact {
        row = row.child(avatar).child(Text::new(user).class("start-user grow"));
    }
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
    if !ctx.is_phone() {
        row = row.child(crate::menu_prefs::button(st.settings, "start-footer-btn", "start-footer-icon"));
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
                anchor: crate::ctx::PopupAnchor { output: None, rect: None, edge: synshell_common::config::Edge::Bottom, attached: false, panel_opacity: None, on_panel: false },
            }));
        }))
        .class(if compact { "start-footer-compact" } else { "start-footer" })
}

