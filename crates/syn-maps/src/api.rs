//! Сеть «Карт»: поиск и адрес точки — Nominatim (nominatim.openstreetmap.org), маршруты — OSRM серверов FOSSGIS
//! (routing.openstreetmap.de: машина, велосипед, пешком). Политики обоих требуют представляться (User-Agent) и не
//! засыпать запросами: поиск — по Enter, не на каждую букву.

use std::time::Duration;

use serde::Deserialize;

pub const USER_AGENT: &str = concat!("syn-maps/", env!("CARGO_PKG_VERSION"), " (synshell; +https://github.com/VitaminDB/synshell)");

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(USER_AGENT)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(20)))
        .build()
        .into()
}

/// Найденное место или точка на карте.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    pub address: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Deserialize)]
struct NomItem {
    lat: String,
    lon: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    display_name: String,
}

fn place_of(it: NomItem) -> Option<Place> {
    let lat = it.lat.parse().ok()?;
    let lon = it.lon.parse().ok()?;
    // display_name: «Название, улица, город, …, страна» — первое — заголовок, остальное — адрес
    let mut parts = it.display_name.splitn(2, ", ");
    let first = parts.next().unwrap_or_default().to_string();
    let rest = parts.next().unwrap_or_default().to_string();
    let name = if it.name.is_empty() { first.clone() } else { it.name.clone() };
    let address = if name == first { rest } else { it.display_name.clone() };
    Some(Place { name, address, lat, lon })
}

fn enc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            b' ' => o.push('+'),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

fn search_url(query: &str, near: Option<(f64, f64, f64)>, bounded: bool) -> String {
    let mut url = format!(
        "https://nominatim.openstreetmap.org/search?q={}&format=jsonv2&limit=12&accept-language=ru,en",
        enc(query)
    );
    if let Some((lat, lon, span)) = near {
        url.push_str(&format!("&viewbox={},{},{},{}", lon - span, lat + span, lon + span, lat - span));
        if bounded {
            url.push_str("&bounded=1");
        }
    }
    url
}

fn fetch(url: &str) -> Result<Vec<Place>, String> {
    let mut r = agent().get(url).call().map_err(|e| format!("поиск: {e}"))?;
    let items: Vec<NomItem> = serde_json::from_reader(r.body_mut().as_reader()).map_err(|e| format!("поиск: {e}"))?;
    Ok(items.into_iter().filter_map(place_of).collect())
}

/// Поиск мест: сначала рядом с `near` (центр карты, полуширина области в градусах — не меньше ~50 км), ближние
/// выше; мало нашлось — добавить найденное везде (Nominatim сам «рядом» почти не учитывает).
pub fn search(query: &str, near: Option<(f64, f64, f64)>) -> Result<Vec<Place>, String> {
    let Some((lat, lon, span)) = near else { return fetch(&search_url(query, None, false)) };
    let mut local = fetch(&search_url(query, Some((lat, lon, span.max(0.5))), true))?;
    let d = |p: &Place| (p.lat - lat).powi(2) + ((p.lon - lon) * lat.to_radians().cos()).powi(2);
    local.sort_by(|a, b| d(a).total_cmp(&d(b)));
    if local.len() < 3 {
        // политика Nominatim — не чаще запроса в секунду
        std::thread::sleep(Duration::from_millis(1100));
        for p in fetch(&search_url(query, Some((lat, lon, span)), false))? {
            if !local.iter().any(|q| (q.lat - p.lat).abs() < 1e-6 && (q.lon - p.lon).abs() < 1e-6) {
                local.push(p);
            }
        }
    }
    Ok(local)
}

/// Адрес точки (обратное геокодирование).
pub fn reverse(lat: f64, lon: f64) -> Result<Place, String> {
    let url = format!("https://nominatim.openstreetmap.org/reverse?lat={lat}&lon={lon}&format=jsonv2&zoom=18&accept-language=ru,en");
    let mut r = agent().get(&url).call().map_err(|e| format!("адрес: {e}"))?;
    let it: NomItem = serde_json::from_reader(r.body_mut().as_reader()).map_err(|e| format!("адрес: {e}"))?;
    let mut p = place_of(it).ok_or("адрес: пустой ответ")?;
    // сама точка, а не центр найденного здания
    p.lat = lat;
    p.lon = lon;
    Ok(p)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Car,
    Foot,
    Bike,
}

impl Profile {
    pub const ALL: [Profile; 3] = [Profile::Car, Profile::Foot, Profile::Bike];
    fn server(self) -> &'static str {
        match self {
            Profile::Car => "routed-car",
            Profile::Foot => "routed-foot",
            Profile::Bike => "routed-bike",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Profile::Car => "На машине",
            Profile::Foot => "Пешком",
            Profile::Bike => "На велосипеде",
        }
    }
}

/// Шаг маршрута: подсказка, значок манёвра, длина участка после него и точка манёвра.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub text: String,
    pub icon: &'static str,
    pub distance: f64,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub profile: Profile,
    /// (широта, долгота)
    pub points: Vec<(f64, f64)>,
    /// метры, секунды
    pub distance: f64,
    pub duration: f64,
    pub steps: Vec<Step>,
}

#[derive(Deserialize)]
struct OsrmResp {
    code: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    routes: Vec<OsrmRoute>,
}
#[derive(Deserialize)]
struct OsrmRoute {
    distance: f64,
    duration: f64,
    geometry: OsrmGeom,
    legs: Vec<OsrmLeg>,
}
#[derive(Deserialize)]
struct OsrmGeom {
    coordinates: Vec<[f64; 2]>,
}
#[derive(Deserialize)]
struct OsrmLeg {
    steps: Vec<OsrmStep>,
}
#[derive(Deserialize)]
struct OsrmStep {
    distance: f64,
    #[serde(default)]
    name: String,
    maneuver: OsrmManeuver,
}
#[derive(Deserialize)]
struct OsrmManeuver {
    location: [f64; 2],
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    modifier: Option<String>,
    #[serde(default)]
    exit: Option<u32>,
}

pub fn route(profile: Profile, from: (f64, f64), to: (f64, f64)) -> Result<Route, String> {
    let url = format!(
        "https://routing.openstreetmap.de/{}/route/v1/driving/{},{};{},{}?overview=full&geometries=geojson&steps=true",
        profile.server(),
        from.1,
        from.0,
        to.1,
        to.0
    );
    let mut r = agent().get(&url).call().map_err(|e| format!("маршрут: {e}"))?;
    let resp: OsrmResp = serde_json::from_reader(r.body_mut().as_reader()).map_err(|e| format!("маршрут: {e}"))?;
    if resp.code != "Ok" {
        return Err(match resp.code.as_str() {
            "NoRoute" => "маршрут не найден".into(),
            _ => format!("маршрут: {}", resp.message.unwrap_or(resp.code)),
        });
    }
    let rt = resp.routes.into_iter().next().ok_or("маршрут не найден")?;
    let steps = rt
        .legs
        .iter()
        .flat_map(|l| l.steps.iter())
        .map(|s| Step {
            text: maneuver_text(&s.maneuver, &s.name),
            icon: maneuver_icon(&s.maneuver),
            distance: s.distance,
            lat: s.maneuver.location[1],
            lon: s.maneuver.location[0],
        })
        .collect();
    Ok(Route {
        profile,
        points: rt.geometry.coordinates.iter().map(|c| (c[1], c[0])).collect(),
        distance: rt.distance,
        duration: rt.duration,
        steps,
    })
}

fn direction(m: Option<&str>) -> &'static str {
    match m.unwrap_or("") {
        "left" => "налево",
        "right" => "направо",
        "slight left" => "плавно налево",
        "slight right" => "плавно направо",
        "sharp left" => "резко налево",
        "sharp right" => "резко направо",
        "uturn" => "с разворотом",
        _ => "прямо",
    }
}

fn maneuver_text(m: &OsrmManeuver, name: &str) -> String {
    let dir = direction(m.modifier.as_deref());
    let base = match m.kind.as_str() {
        "depart" => "Начните движение".to_string(),
        "arrive" => return "Вы на месте".into(),
        "turn" | "end of road" if m.modifier.as_deref() == Some("uturn") => "Развернитесь".to_string(),
        "turn" => format!("Поверните {dir}"),
        "end of road" => format!("В конце дороги — {dir}"),
        "new name" | "continue" => {
            if dir == "прямо" {
                "Продолжайте прямо".to_string()
            } else {
                format!("Держитесь {dir}")
            }
        }
        "fork" => format!("На развилке — {dir}"),
        "merge" => format!("Перестройтесь {dir}"),
        "on ramp" => format!("Въезд {dir}"),
        "off ramp" => format!("Съезд {dir}"),
        "roundabout" | "rotary" => match m.exit {
            Some(n) => format!("На круге — {n}-й съезд"),
            None => "Въезжайте на круг".to_string(),
        },
        "exit roundabout" | "exit rotary" => "Съезжайте с круга".to_string(),
        _ => format!("Двигайтесь {dir}"),
    };
    if name.is_empty() {
        base
    } else {
        format!("{base} — {name}")
    }
}

fn maneuver_icon(m: &OsrmManeuver) -> &'static str {
    use crate::icons::*;
    match m.kind.as_str() {
        "arrive" => FLAG,
        "depart" => NAVIGATION,
        "roundabout" | "rotary" | "exit roundabout" | "exit rotary" => ROUNDABOUT_RIGHT,
        "merge" => MERGE,
        "fork" => {
            if m.modifier.as_deref().is_some_and(|d| d.contains("left")) {
                FORK_LEFT
            } else {
                FORK_RIGHT
            }
        }
        _ => match m.modifier.as_deref().unwrap_or("") {
            "left" => TURN_LEFT,
            "right" => TURN_RIGHT,
            "slight left" => TURN_SLIGHT_LEFT,
            "slight right" => TURN_SLIGHT_RIGHT,
            "sharp left" => TURN_SHARP_LEFT,
            "sharp right" => TURN_SHARP_RIGHT,
            "uturn" => U_TURN_LEFT,
            _ => STRAIGHT,
        },
    }
}

/// «350 м», «12,4 км», «120 км».
pub fn fmt_distance(m: f64) -> String {
    if m < 1000.0 {
        format!("{} м", ((m / 10.0).round() * 10.0) as i64)
    } else if m < 100_000.0 {
        format!("{:.1} км", m / 1000.0).replace('.', ",")
    } else {
        format!("{} км", (m / 1000.0).round() as i64)
    }
}

/// «7 мин», «1 ч 05 мин».
pub fn fmt_duration(s: f64) -> String {
    let min = (s / 60.0).round() as i64;
    if min < 60 {
        format!("{} мин", min.max(1))
    } else {
        format!("{} ч {:02} мин", min / 60, min % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(fmt_distance(347.0), "350 м");
        assert_eq!(fmt_distance(12_430.0), "12,4 км");
        assert_eq!(fmt_duration(65.0 * 60.0), "1 ч 05 мин");
        assert_eq!(fmt_duration(20.0), "1 мин");
    }

    #[test]
    fn maneuvers() {
        let m = OsrmManeuver { location: [0.0, 0.0], kind: "turn".into(), modifier: Some("left".into()), exit: None };
        assert_eq!(maneuver_text(&m, "Ленина"), "Поверните налево — Ленина");
        let r = OsrmManeuver { location: [0.0, 0.0], kind: "roundabout".into(), modifier: Some("right".into()), exit: Some(2) };
        assert_eq!(maneuver_text(&r, ""), "На круге — 2-й съезд");
    }
}
