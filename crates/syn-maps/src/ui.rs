//! Интерфейс «Карт»: карта во весь экран, поверх — строка поиска (или подсказка манёвра в пути), кнопки слоёв и
//! своего места, снизу — карточка точки или маршрута. Сеть и GeoClue — в своих потоках, результаты в сигналы
//! через `run_on_main_thread`; устаревшие ответы отбрасываются по поколению.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex as StdMutex;
use std::time::Instant;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use syngui::widgets::visual::map_view::{MapCamera, MapMarker, MapPolyline, MapView, MapViewport, TileCache, TileProvider};
use syngui::widgets::*;
use synsystem::geoclue_client::{self, Fix, Status};

use crate::api::{self, Place, Profile, Route};
use crate::icons;

type W = Box<dyn Widget>;

/// Слои: название, значок, источник.
pub const LAYERS: [(&str, &str); 4] = [(n_!("Схема"), icons::MAP), (n_!("Светлая"), icons::MAP), (n_!("Тёмная"), icons::DARK), (n_!("Спутник"), icons::SATELLITE)];

fn provider(i: usize) -> TileProvider {
    match i {
        1 => TileProvider::carto_voyager(),
        2 => TileProvider::carto_dark(),
        3 => TileProvider::esri_satellite(),
        _ => TileProvider::osm(),
    }
}

#[derive(Clone, Copy)]
pub struct St {
    view: RwSignal<MapViewport>,
    camera: RwSignal<MapCamera>,
    layer: RwSignal<usize>,
    layers_open: RwSignal<bool>,
    fix: RwSignal<Option<Fix>>,
    loc_error: RwSignal<Option<String>>,
    /// Карта следует за своим местом (кнопка «моё место»; сдвиг карты пальцем — выключает).
    follow: RwSignal<bool>,
    query: RwSignal<String>,
    results: RwSignal<Vec<Place>>,
    searching: RwSignal<bool>,
    place: RwSignal<Option<Place>>,
    profile: RwSignal<Profile>,
    route: RwSignal<Option<Route>>,
    routing: RwSignal<bool>,
    steps_open: RwSignal<bool>,
    /// В пути: карта за мной, сверху — следующий манёвр.
    navigating: RwSignal<bool>,
    error: RwSignal<Option<String>>,
}

impl St {
    pub fn new(lat: f64, lon: f64, zoom: f64, layer: usize, explicit: bool) -> Self {
        let st = Self {
            view: use_signal(MapViewport { center_lat: lat, center_lng: lon, zoom: zoom.round() as u8, zoom_level: zoom, viewport_w: 0.0, viewport_h: 0.0 }),
            camera: use_signal(MapCamera { lat, lng: lon, zoom, seq: next_seq(), animate: false }),
            layer: use_signal(layer.min(LAYERS.len() - 1)),
            layers_open: use_signal(false),
            fix: use_signal(None),
            loc_error: use_signal(None),
            // без явной точки при запуске — сразу к своему месту, как только оно появится
            follow: use_signal(!explicit),
            query: use_signal(String::new()),
            results: use_signal(Vec::new()),
            searching: use_signal(false),
            place: use_signal(None),
            profile: use_signal(Profile::Car),
            route: use_signal(None),
            routing: use_signal(false),
            steps_open: use_signal(false),
            navigating: use_signal(false),
            error: use_signal(None),
        };
        if explicit {
            st.place.set(Some(Place { name: t!("Точка").into(), address: format!("{lat:.5}, {lon:.5}"), lat, lon }));
            lookup_address(st, lat, lon);
        }
        st
    }
}

static SEQ: AtomicU64 = AtomicU64::new(1);
static SEARCH_GEN: AtomicU64 = AtomicU64::new(0);
static ROUTE_GEN: AtomicU64 = AtomicU64::new(0);
static ADDR_GEN: AtomicU64 = AtomicU64::new(0);
static WATCH: StdMutex<Option<geoclue_client::Watch>> = StdMutex::new(None);
static SAVED: StdMutex<Option<Instant>> = StdMutex::new(None);

fn next_seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

fn fly(st: St, lat: f64, lon: f64, zoom: Option<f64>) {
    let z = zoom.unwrap_or_else(|| st.view.get_untracked().zoom_level);
    st.camera.set(MapCamera { lat, lng: lon, zoom: z, seq: next_seq(), animate: true });
}

// ─── местоположение ────────────────────────────────────────────────────────

pub fn start_location(st: St) {
    let w = geoclue_client::watch("syn-maps", 0, move |s| {
        run_on_main_thread(move || match s {
            Status::Waiting => {}
            Status::Fix(f) => {
                let first = st.fix.get_untracked().is_none();
                st.fix.set(Some(f));
                st.loc_error.set(None);
                if st.follow.get_untracked() || st.navigating.get_untracked() {
                    let z = if first { Some(16.0f64.max(st.view.get_untracked().zoom_level)) } else { None };
                    fly(st, f.lat, f.lon, z);
                }
            }
            Status::Error(e) => st.loc_error.set(Some(e)),
        })
    });
    *WATCH.lock().unwrap() = Some(w);
}

fn my_location(st: St) {
    match st.fix.get_untracked() {
        Some(f) => {
            st.follow.set(true);
            fly(st, f.lat, f.lon, Some(16.0f64.max(st.view.get_untracked().zoom_level)));
        }
        None => {
            st.follow.set(true);
            let msg = st.loc_error.get_untracked().unwrap_or_else(|| t!("Определяем местоположение…").into());
            st.error.set(Some(msg));
        }
    }
}

/// Расстояние по сфере, м.
fn haversine(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (la1, lo1, la2, lo2) = (a.0.to_radians(), a.1.to_radians(), b.0.to_radians(), b.1.to_radians());
    let h = ((la2 - la1) / 2.0).sin().powi(2) + la1.cos() * la2.cos() * ((lo2 - lo1) / 2.0).sin().powi(2);
    2.0 * 6_371_000.0 * h.sqrt().asin()
}

// ─── поиск и адрес ─────────────────────────────────────────────────────────

fn search(st: St) {
    let q = st.query.get_untracked().trim().to_string();
    if q.is_empty() {
        return;
    }
    let gen = SEARCH_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let v = st.view.get_untracked();
    // область «рядом» — примерно видимая карта
    let span = 360.0 / 2f64.powf(v.zoom_level) * 2.0;
    st.searching.set(true);
    std::thread::spawn(move || {
        let r = api::search(&q, Some((v.center_lat, v.center_lng, span.clamp(0.01, 20.0))));
        run_on_main_thread(move || {
            if SEARCH_GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            st.searching.set(false);
            match r {
                Ok(list) if list.is_empty() => st.error.set(Some(t!("Ничего не найдено").into())),
                Ok(list) => st.results.set(list),
                Err(e) => st.error.set(Some(e)),
            }
        });
    });
}

fn choose(st: St, p: Place) {
    st.results.set(Vec::new());
    st.follow.set(false);
    fly(st, p.lat, p.lon, Some(16.0));
    st.place.set(Some(p));
    st.route.set(None);
}

fn lookup_address(st: St, lat: f64, lon: f64) {
    let gen = ADDR_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        let r = api::reverse(lat, lon);
        run_on_main_thread(move || {
            if ADDR_GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            if let Ok(p) = r {
                // точка всё ещё та же — подписать
                if st.place.get_untracked().is_some_and(|c| (c.lat - lat).abs() < 1e-9 && (c.lon - lon).abs() < 1e-9) {
                    st.place.set(Some(p));
                }
            }
        });
    });
}

fn drop_pin(st: St, lat: f64, lon: f64) {
    st.results.set(Vec::new());
    st.route.set(None);
    st.navigating.set(false);
    st.place.set(Some(Place { name: t!("Точка на карте").into(), address: format!("{lat:.5}, {lon:.5}"), lat, lon }));
    lookup_address(st, lat, lon);
}

// ─── маршрут ───────────────────────────────────────────────────────────────

fn build_route(st: St) {
    let Some(to) = st.place.get_untracked() else { return };
    // от своего места; пока его нет — от центра карты
    let from = match st.fix.get_untracked() {
        Some(f) => (f.lat, f.lon),
        None => {
            let v = st.view.get_untracked();
            st.error.set(Some(t!("Местоположение ещё не известно — маршрут от центра карты").into()));
            (v.center_lat, v.center_lng)
        }
    };
    let profile = st.profile.get_untracked();
    let gen = ROUTE_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    st.routing.set(true);
    std::thread::spawn(move || {
        let r = api::route(profile, from, (to.lat, to.lon));
        run_on_main_thread(move || {
            if ROUTE_GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            st.routing.set(false);
            match r {
                Ok(route) => {
                    fit_route(st, &route);
                    st.route.set(Some(route));
                }
                Err(e) => st.error.set(Some(e)),
            }
        });
    });
}

/// Весь маршрут в кадре — в видимой части карты: между строкой поиска сверху и карточкой маршрута снизу.
fn fit_route(st: St, r: &Route) {
    use syngui::widgets::visual::map_view::tile_math::{world_px, world_px_to_geo};
    if r.points.is_empty() {
        return;
    }
    let v = st.view.get_untracked();
    let (w, h) = (v.viewport_w.max(200.0) as f64, v.viewport_h.max(300.0) as f64);
    let (top, bottom, side) = (72.0, 200.0f64.min(h * 0.4), 24.0);
    let (vis_w, vis_h) = ((w - 2.0 * side).max(80.0), (h - top - bottom - 24.0).max(80.0));
    // размах маршрута в мировых пикселях при масштабе 0 — масштаб, при котором он влезает
    let pts0: Vec<(f64, f64)> = r.points.iter().map(|&(la, lo)| world_px(la, lo, 0.0)).collect();
    let (x0, x1) = pts0.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
    let (y0, y1) = pts0.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
    let z = (vis_w / (x1 - x0).max(1e-9)).log2().min((vis_h / (y1 - y0).max(1e-9)).log2()).clamp(2.0, 17.0);
    // середина маршрута должна оказаться посередине видимой части: центр карты ниже на сдвиг
    let k = 2f64.powf(z);
    let (mx, my) = ((x0 + x1) / 2.0 * k, (y0 + y1) / 2.0 * k);
    let target_y = top + vis_h / 2.0 + 12.0;
    let (lat, lon) = world_px_to_geo(mx, my + (h / 2.0 - target_y), z);
    st.follow.set(false);
    fly(st, lat, lon, Some(z));
}

/// Следующий манёвр в пути: шаг после ближайшей ко мне точки манёвра, и расстояние до него.
fn next_step(r: &Route, me: (f64, f64)) -> Option<(usize, f64)> {
    if r.steps.is_empty() {
        return None;
    }
    let near = r
        .steps
        .iter()
        .enumerate()
        .min_by(|x, y| haversine(me, (x.1.lat, x.1.lon)).total_cmp(&haversine(me, (y.1.lat, y.1.lon))))
        .map(|(i, _)| i)?;
    let d_near = haversine(me, (r.steps[near].lat, r.steps[near].lon));
    // ещё не дошли до ближайшего манёвра (дальше 25 м) — он и следующий
    let i = if d_near > 25.0 { near } else { (near + 1).min(r.steps.len() - 1) };
    Some((i, haversine(me, (r.steps[i].lat, r.steps[i].lon))))
}

// ─── интерфейс ─────────────────────────────────────────────────────────────

pub fn root(st: St) -> W {
    let narrow = syngui::viewport::viewport_below(720.0);
    let overlay = Reactive::new(move || -> Vec<W> {
        let phone = narrow.get();
        let mut col = Column::new().child(top_bar(st, phone)).child(DecoratedBox::new().class("grow")).child(fabs(st)).child(sheet(st, phone));
        if !phone {
            col = col.class("desktop");
        }
        vec![Box::new(col)]
    });
    Box::new(
        GestureDetector::new()
            .on_back(move || {
                if !st.results.get_untracked().is_empty() {
                    st.results.set(Vec::new());
                    return true;
                }
                if st.navigating.get_untracked() {
                    st.navigating.set(false);
                    return true;
                }
                if st.route.get_untracked().is_some() {
                    st.route.set(None);
                    return true;
                }
                if st.place.get_untracked().is_some() {
                    st.place.set(None);
                    return true;
                }
                false
            })
            .child(Stack::new().fit(StackFit::Expand).child(map(st)).child(overlay).child(toast(st))),
    )
}

fn map(st: St) -> W {
    let cache = std::sync::Arc::new(TileCache::new(synshell_common::paths::cache_home().join("syn-maps/tiles")));
    Box::new(Reactive::new(move || -> Vec<W> {
        let mut markers = Vec::new();
        let mut lines = Vec::new();
        if let Some(r) = st.route.get() {
            lines.push(MapPolyline::new(r.points.clone()).color(Color::new(0.16, 0.45, 0.98, 1.0)).width(6.0).outline(Color::new(0.05, 0.2, 0.55, 0.9), 9.0));
            if let Some(&(lat, lon)) = r.points.first() {
                markers.push(MapMarker::new(lat, lon).color(Color::new(0.2, 0.75, 0.35, 1.0)).size(14.0));
            }
        }
        if let Some(p) = st.place.get() {
            markers.push(MapMarker::new(p.lat, p.lon).id(1).color(Color::new(0.9, 0.2, 0.25, 1.0)).size(22.0));
        }
        for (i, p) in st.results.get().iter().enumerate() {
            markers.push(MapMarker::new(p.lat, p.lon).id(100 + i as u64).color(Color::new(0.95, 0.45, 0.1, 1.0)).size(16.0));
        }
        if let Some(f) = st.fix.get() {
            markers.push(MapMarker::new(f.lat, f.lon).color(Color::new(0.1, 0.5, 1.0, 1.0)).size(20.0));
        }
        let results = st.results.get_untracked();
        vec![Box::new(
            MapView::new()
                .provider(provider(st.layer.get()))
                .tile_cache_arc(cache.clone())
                .smooth_zoom(true)
                .overzoom(2)
                .camera(st.camera.get())
                .markers(markers)
                .polylines(lines)
                .on_viewport_change(move |vp| {
                    st.view.set(vp);
                    save_view(st);
                })
                // сдвинул карту сам — больше не следовать за мной (в пути — следовать всё равно)
                .on_interaction(move || {
                    if st.follow.get_untracked() {
                        st.follow.set(false);
                    }
                })
                .on_marker_click(move |id| {
                    if let Some(p) = id.checked_sub(100).and_then(|i| results.get(i as usize).cloned()) {
                        choose(st, p);
                    }
                })
                .on_tap(move |_, _| {
                    st.layers_open.set(false);
                    st.follow.set(false);
                })
                .on_long_press(move |lat, lon| drop_pin(st, lat, lon)),
        )]
    }))
}

fn save_view(st: St) {
    let mut last = SAVED.lock().unwrap();
    if last.is_some_and(|t| t.elapsed().as_secs_f32() < 2.0) {
        return;
    }
    *last = Some(Instant::now());
    let v = st.view.get_untracked();
    crate::settings::Settings { lat: v.center_lat, lon: v.center_lng, zoom: v.zoom_level, layer: st.layer.get_untracked() }.save();
}

fn icon_button(icon: &'static str, class: &'static str, f: impl Fn() + Send + Sync + 'static) -> W {
    Box::new(GestureDetector::new().on_click(f).cursor(syngui::CursorIcon::Pointer).child(DecoratedBox::new().child(Icon::new(icon).class("ib-icon")).class(class)))
}

/// Сверху: поиск (с результатами) или, в пути, следующий манёвр.
fn top_bar(st: St, phone: bool) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        if st.navigating.get() {
            return vec![nav_banner(st)];
        }
        let field = TextField::new()
            .text(st.query.get_untracked())
            .placeholder(t!("Поиск мест и адресов"))
            .on_change(move |t| st.query.set(t.to_string()))
            .on_submit(move |_| search(st))
            .class("search-field");
        let has_results = !st.results.get().is_empty();
        let mut bar = Row::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(if has_results { icon_button(icons::BACK, "ib", move || st.results.set(Vec::new())) } else { Box::new(DecoratedBox::new().child(Icon::new(icons::SEARCH).class("ib-icon muted")).class("ib")) as W })
            .child(DecoratedBox::new().child(field).class("grow"));
        if st.searching.get() {
            bar = bar.child(Text::new("…").class("muted"));
        } else if !st.query.get_untracked().is_empty() {
            bar = bar.child(icon_button(icons::SEARCH, "ib", move || search(st)));
        }
        let mut col = Column::new().gap(6.0).child(DecoratedBox::new().child(bar).class("card search"));
        if has_results {
            let rows = st.results.get().into_iter().map(|p| result_row(st, p)).fold(Column::new(), |c, r| c.child(r));
            col = col.child(DecoratedBox::new().child(ScrollView::new().vertical().child(rows)).class(if phone { "card results" } else { "card results results-desktop" }));
        }
        vec![Box::new(DecoratedBox::new().child(col).class(if phone { "top" } else { "top top-desktop" }))]
    }))
}

fn result_row(st: St, p: Place) -> W {
    let q = p.clone();
    Box::new(
        GestureDetector::new().on_click(move || choose(st, q.clone())).cursor(syngui::CursorIcon::Pointer).child(
            DecoratedBox::new()
                .child(
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Icon::new(icons::PLACE).class("row-icon"))
                        .child(DecoratedBox::new().child(Column::new().gap(2.0).child(Text::new(p.name.clone()).max_lines(1).class("row-title")).child(Text::new(p.address.clone()).max_lines(2).class("row-sub"))).class("grow")),
                )
                .class("row"),
        ),
    )
}

fn nav_banner(st: St) -> W {
    let (Some(r), Some(f)) = (st.route.get(), st.fix.get()) else {
        return Box::new(DecoratedBox::new().child(Text::new(t!("Ждём местоположение…")).class("nav-text")).class("card nav"));
    };
    let Some((i, d)) = next_step(&r, (f.lat, f.lon)) else {
        return Box::new(DecoratedBox::new().class("card nav"));
    };
    let s = &r.steps[i];
    // осталось до конца: шаги после текущего + до него
    let rest: f64 = d + r.steps[i..].iter().map(|s| s.distance).sum::<f64>();
    Box::new(
        DecoratedBox::new()
            .child(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(s.icon).class("nav-icon"))
                    .child(DecoratedBox::new().child(Column::new().gap(2.0).child(Text::new(api::fmt_distance(d)).class("nav-dist")).child(Text::new(s.text.clone()).max_lines(2).class("nav-text"))).class("grow"))
                    .child(Column::new().cross_axis_alignment(CrossAxisAlignment::End).child(Text::new(api::fmt_distance(rest)).class("row-sub")))
                    .child(icon_button(icons::CLOSE, "ib", move || st.navigating.set(false))),
            )
            .class("card nav"),
    )
}

/// Кнопки справа внизу: слои (с меню) и своё место.
fn fabs(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let mut col = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::End);
        if st.layers_open.get() {
            let mut menu = Column::new().gap(2.0);
            for (i, (name, icon)) in LAYERS.iter().enumerate() {
                let active = st.layer.get() == i;
                menu = menu.child(
                    GestureDetector::new()
                        .on_click(move || {
                            st.layer.set(i);
                            st.layers_open.set(false);
                            save_view(st);
                        })
                        .cursor(syngui::CursorIcon::Pointer)
                        .child(
                            DecoratedBox::new()
                                .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(*icon).class(if active { "row-icon active" } else { "row-icon" })).child(Text::new(syngui::i18n::t(name)).class(if active { "row-title active" } else { "row-title" })))
                                .class("row"),
                        ),
                );
            }
            col = col.child(DecoratedBox::new().child(menu).class("card layers"));
        }
        let loc_icon = if st.follow.get() && st.fix.get().is_some() { icons::MY_LOCATION } else { icons::LOCATION_SEARCHING };
        col = col
            .child(icon_button(icons::LAYERS, "fab", move || st.layers_open.update(|o| *o = !*o)))
            .child(icon_button(loc_icon, if st.follow.get() { "fab fab-on" } else { "fab" }, move || my_location(st)));
        vec![Box::new(DecoratedBox::new().child(Row::new().child(DecoratedBox::new().class("grow")).child(col)).class("fabs"))]
    }))
}

/// Снизу: маршрут или выбранная точка.
fn sheet(st: St, phone: bool) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let class = if phone { "card sheet" } else { "card sheet sheet-desktop" };
        if st.navigating.get() {
            return vec![];
        }
        if let Some(r) = st.route.get() {
            return vec![Box::new(DecoratedBox::new().child(route_card(st, r)).class(class))];
        }
        let Some(p) = st.place.get() else { return vec![] };
        let routing = st.routing.get();
        let body = Column::new()
            .gap(6.0)
            .child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .child(DecoratedBox::new().child(Column::new().gap(2.0).child(Text::new(p.name.clone()).max_lines(2).class("sheet-title")).child(Text::new(p.address.clone()).max_lines(3).class("row-sub"))).class("grow"))
                    .child(icon_button(icons::CLOSE, "ib", move || st.place.set(None))),
            )
            .child(Text::new(format!("{:.5}, {:.5}", p.lat, p.lon)).class("coords"))
            .child(
                Row::new()
                    .gap(8.0)
                    .child(profile_chips(st))
                    .child(DecoratedBox::new().class("grow"))
                    .child(GestureDetector::new().on_click(move || build_route(st)).cursor(syngui::CursorIcon::Pointer).child(
                        DecoratedBox::new()
                            .child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icons::DIRECTIONS).class("btn-icon")).child(Text::new(if routing { t!("Строим…") } else { t!("Маршрут") }).class("btn-text")))
                            .class("btn"),
                    )),
            );
        vec![Box::new(DecoratedBox::new().child(body).class(class))]
    }))
}

fn profile_chips(st: St) -> W {
    let mut row = Row::new().gap(4.0);
    for p in Profile::ALL {
        let icon = match p {
            Profile::Car => icons::CAR,
            Profile::Foot => icons::WALK,
            Profile::Bike => icons::BIKE,
        };
        let on = st.profile.get() == p;
        row = row.child(GestureDetector::new().on_click(move || {
            if st.profile.get_untracked() != p {
                st.profile.set(p);
                if st.route.get_untracked().is_some() {
                    build_route(st);
                }
            }
        })
        .cursor(syngui::CursorIcon::Pointer)
        .child(Tooltip::new(DecoratedBox::new().child(Icon::new(icon).class(if on { "chip-icon on" } else { "chip-icon" })).class(if on { "chip chip-on" } else { "chip" }), p.label())));
    }
    Box::new(row)
}

fn route_card(st: St, r: Route) -> W {
    let steps_open = st.steps_open.get();
    let mut col = Column::new()
        .gap(8.0)
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().child(Column::new().gap(2.0).child(Text::new(api::fmt_duration(r.duration)).class("sheet-title")).child(Text::new(format!("{} · {}", api::fmt_distance(r.distance), r.profile.label().to_lowercase())).class("row-sub"))).class("grow"))
                .child(icon_button(icons::LIST, if steps_open { "ib ib-on" } else { "ib" }, move || st.steps_open.update(|o| *o = !*o)))
                .child(icon_button(icons::CLOSE, "ib", move || {
                    st.route.set(None);
                    st.steps_open.set(false);
                })),
        )
        .child(
            Row::new()
                .gap(8.0)
                .child(profile_chips(st))
                .child(DecoratedBox::new().class("grow"))
                .child(GestureDetector::new().on_click(move || {
                    st.navigating.set(true);
                    st.steps_open.set(false);
                    if let Some(f) = st.fix.get_untracked() {
                        fly(st, f.lat, f.lon, Some(17.0));
                    }
                })
                .cursor(syngui::CursorIcon::Pointer)
                .child(
                    DecoratedBox::new()
                        .child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icons::NAVIGATION).class("btn-icon")).child(Text::new(t!("В путь")).class("btn-text")))
                        .class("btn"),
                )),
        );
    if steps_open {
        let list = r.steps.iter().fold(Column::new().gap(2.0), |c, s| {
            let (lat, lon) = (s.lat, s.lon);
            c.child(
                GestureDetector::new().on_click(move || {
                    st.follow.set(false);
                    fly(st, lat, lon, Some(17.0));
                })
                .cursor(syngui::CursorIcon::Pointer)
                .child(
                    DecoratedBox::new()
                        .child(
                            Row::new()
                                .gap(10.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(Icon::new(s.icon).class("row-icon"))
                                .child(DecoratedBox::new().child(Text::new(s.text.clone()).max_lines(2).class("row-title")).class("grow"))
                                .child(Text::new(if s.distance > 0.0 { api::fmt_distance(s.distance) } else { String::new() }).class("row-sub")),
                        )
                        .class("row"),
                ),
            )
        });
        col = col.child(DecoratedBox::new().child(ScrollView::new().vertical().child(list)).class("steps"));
    }
    Box::new(col)
}

fn toast(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let Some(e) = st.error.get() else { return vec![] };
        // убрать само через 4 с
        let shown = e.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(4));
            run_on_main_thread(move || {
                if st.error.get_untracked().as_deref() == Some(shown.as_str()) {
                    st.error.set(None);
                }
            });
        });
        vec![Box::new(
            Column::new().child(DecoratedBox::new().class("grow")).child(
                GestureDetector::new().on_click(move || st.error.set(None)).child(DecoratedBox::new().child(Text::new(e).max_lines(3).class("toast-text")).class("toast")),
            ),
        )]
    }))
}
