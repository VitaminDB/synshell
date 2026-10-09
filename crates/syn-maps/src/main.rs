//! syn-maps — «Карты» synshell: OpenStreetMap (MapView syngui: плавный масштаб, плитки в кэше), своё
//! местоположение (GeoClue: спутники модема), поиск мест (Nominatim), маршруты на машине, пешком и на
//! велосипеде (OSRM) с пошаговыми подсказками и слежением за положением.
//!
//!   syn-maps [geo:ШИРОТА,ДОЛГОТА[?q=…]]

mod api;
mod settings;
mod ui;

use syngui::prelude::*;
use synshell_common::Config;

/// `geo:55.75,37.61?z=15` → (широта, долгота, масштаб).
fn geo_uri(s: &str) -> Option<(f64, f64, Option<f64>)> {
    let rest = s.strip_prefix("geo:")?;
    let (coords, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut it = coords.split([',', ';']);
    let lat = it.next()?.trim().parse().ok()?;
    let lon = it.next()?.trim().parse().ok()?;
    let z = query.split('&').find_map(|kv| kv.strip_prefix("z=")).and_then(|z| z.parse().ok());
    Some((lat, lon, z))
}

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syn_maps=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let mut start: Option<(f64, f64, Option<f64>)> = None;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{}", t!("syn-maps [geo:ШИРОТА,ДОЛГОТА[?z=МАСШТАБ]]"));
                return;
            }
            _ => start = geo_uri(&a).or(start),
        }
    }
    syngui::widgets::visual::map_view::tile_loader::set_user_agent(api::USER_AGENT);
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    let set = settings::Settings::load();
    App::new()
        .title(t!("Карты"))
        .app_id("syn-maps")
        .size(960, 720)
        .min_size(320, 420)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .run(move |_| {
            let (lat, lon, zoom) = match start {
                Some((lat, lon, z)) => (lat, lon, z.unwrap_or(16.0)),
                None => (set.lat, set.lon, set.zoom),
            };
            let st = ui::St::new(lat, lon, zoom, set.layer, start.is_some());
            ui::start_location(st);
            ui::root(st)
        });
}

fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let mut s = a.mss_variables();
    s.push_str(&a.theme_mss_variables());
    s.push_str(include_str!("../styles/syn-maps.mss"));
    if !a.font.trim().is_empty() {
        s.push_str(&format!("Text, Button, TextField {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    s
}

/// Кодовые точки Material Icons.
pub mod icons {
    pub const SEARCH: &str = "\u{E8B6}";
    pub const CLOSE: &str = "\u{E5CD}";
    pub const BACK: &str = "\u{E5C4}";
    pub const MY_LOCATION: &str = "\u{E55C}";
    pub const LOCATION_SEARCHING: &str = "\u{E1B4}";
    pub const NAVIGATION: &str = "\u{E55D}";
    pub const LAYERS: &str = "\u{E53B}";
    pub const DIRECTIONS: &str = "\u{E52E}";
    pub const CAR: &str = "\u{E531}";
    pub const WALK: &str = "\u{E536}";
    pub const BIKE: &str = "\u{E52F}";
    pub const PLACE: &str = "\u{E55F}";
    pub const FLAG: &str = "\u{E153}";
    pub const ADD: &str = "\u{E145}";
    pub const REMOVE: &str = "\u{E15B}";
    pub const MAP: &str = "\u{E55B}";
    pub const SATELLITE: &str = "\u{E562}";
    pub const DARK: &str = "\u{E51C}";
    pub const LIST: &str = "\u{E896}";
    pub const STRAIGHT: &str = "\u{EB95}";
    pub const TURN_LEFT: &str = "\u{EBA6}";
    pub const TURN_RIGHT: &str = "\u{EBAB}";
    pub const TURN_SLIGHT_LEFT: &str = "\u{EBA4}";
    pub const TURN_SLIGHT_RIGHT: &str = "\u{EB9A}";
    pub const TURN_SHARP_LEFT: &str = "\u{EBA7}";
    pub const TURN_SHARP_RIGHT: &str = "\u{EBAA}";
    pub const U_TURN_LEFT: &str = "\u{EBA1}";
    pub const ROUNDABOUT_RIGHT: &str = "\u{EBA3}";
    pub const FORK_LEFT: &str = "\u{EBA0}";
    pub const FORK_RIGHT: &str = "\u{EBAC}";
    pub const MERGE: &str = "\u{EB98}";
}
