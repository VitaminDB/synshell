//! Клиент GeoClue: текущее местоположение для программ (карты, компас).
//!
//! `Manager.GetClient()` → у клиента задаются `DesktopId` (id .desktop программы без суффикса — по нему агент
//! оболочки, [`crate::geoclue_agent`], решает, давать ли место, и спрашивает пользователя), точность и порог
//! расстояния; после `Start()` GeoClue шлёт `LocationUpdated(old, new)`, свойства точки — на объекте
//! `org.freedesktop.GeoClue2.Location`. Всё в своём потоке на блокирующем zbus; [`Watch`] при удалении
//! останавливает клиента.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;
use synshell_tr::t;

const GEOCLUE: &str = "org.freedesktop.GeoClue2";

/// Точка от GeoClue. Высота, курс и скорость — если источник их знает (спутники).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fix {
    pub lat: f64,
    pub lon: f64,
    /// Радиус точности, м.
    pub accuracy: f64,
    pub altitude: Option<f64>,
    /// Курс, градусы от севера.
    pub heading: Option<f64>,
    /// Скорость, м/с.
    pub speed: Option<f64>,
}

/// Состояние слежения для программы.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// Ждём первой точки.
    Waiting,
    Fix(Fix),
    /// GeoClue нет, агент отказал или ошибка шины.
    Error(Problem),
}

/// Почему местоположения нет — чтобы программа показала понятный текст и, где можно, кнопку исправления.
#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    /// Местоположение выключено в системе (`[location] enabled = false`: агент оболочки
    /// отдаёт `MaxAccuracyLevel` 0, GeoClue отвечает «Geolocation disabled for UID»).
    Disabled,
    /// Пользователь запретил местоположение этой программе (агент отказал).
    Denied,
    /// Службы GeoClue нет или она не запускается.
    NoService,
    /// Нет агента местоположения (оболочка не зарегистрирована в GeoClue).
    NoAgent,
    /// Прочее — уже человеческим текстом с подробностями.
    Other(String),
}

impl Problem {
    /// Разобрать ошибку D-Bus/GeoClue.
    pub fn classify(e: &zbus::Error) -> Problem {
        let text = e.to_string();
        let name = match e {
            zbus::Error::MethodError(n, _, _) => n.as_str().to_string(),
            zbus::Error::FDO(f) => match f.as_ref() {
                zbus::fdo::Error::ServiceUnknown(_) => "org.freedesktop.DBus.Error.ServiceUnknown".into(),
                zbus::fdo::Error::NameHasNoOwner(_) => "org.freedesktop.DBus.Error.NameHasNoOwner".into(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        Problem::from_text(&format!("{name} {text}"), &text)
    }

    fn from_text(s: &str, text: &str) -> Problem {
        if s.contains("Geolocation disabled") {
            Problem::Disabled
        } else if s.contains("Agent rejected") || s.contains("disallowed") && !s.contains("no agent") {
            Problem::Denied
        } else if s.contains("no agent") {
            Problem::NoAgent
        } else if s.contains("ServiceUnknown") || s.contains("NameHasNoOwner") || s.contains("startup job failed") {
            Problem::NoService
        } else {
            Problem::Other(text.trim().to_string())
        }
    }

    /// Текст для пользователя на языке интерфейса.
    pub fn message(&self) -> String {
        match self {
            Problem::Disabled => t!("Местоположение выключено").into(),
            Problem::Denied => t!("Этой программе запрещено определять местоположение").into(),
            Problem::NoService => t!("Служба местоположения GeoClue не установлена или не запускается").into(),
            Problem::NoAgent => t!("Местоположение недоступно: оболочка не зарегистрирована агентом GeoClue").into(),
            Problem::Other(e) => t!("Не удалось определить местоположение: {e}", e = e).into(),
        }
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

/// Работающее слежение; удаление — остановка клиента.
pub struct Watch {
    stop: Arc<AtomicBool>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Следить за местоположением: `on_status` вызывается в потоке клиента (переносите в UI сами).
/// `distance_m` — порог сдвига для новой точки (0 — каждое обновление).
pub fn watch(desktop_id: &str, distance_m: u32, on_status: impl Fn(Status) + Send + 'static) -> Watch {
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    let id = desktop_id.to_string();
    std::thread::Builder::new()
        .name("geoclue-client".into())
        .spawn(move || {
            on_status(Status::Waiting);
            // GeoClue запускается по D-Bus и может пропасть — переподключаемся, пока слежение нужно
            while !s.load(Ordering::SeqCst) {
                if let Err(e) = run(&id, distance_m, &s, &on_status) {
                    tracing::warn!(error = %e, "geoclue: клиент");
                    on_status(Status::Error(Problem::classify(&e)));
                }
                for _ in 0..50 {
                    if s.load(Ordering::SeqCst) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        })
        .expect("geoclue-client");
    Watch { stop }
}

fn run(desktop_id: &str, distance_m: u32, stop: &AtomicBool, on_status: &dyn Fn(Status)) -> zbus::Result<()> {
    // Ошибки GeoClue (Start: выключено, запрещено, нет агента) приходят как MethodError с текстом —
    // [`Problem::classify`] разбирает их сам.
    let conn = Connection::system()?;
    let manager = Proxy::new(&conn, GEOCLUE, "/org/freedesktop/GeoClue2/Manager", "org.freedesktop.GeoClue2.Manager")?;
    let path: OwnedObjectPath = manager.call("GetClient", &())?;
    let client = Proxy::new(&conn, GEOCLUE, path.as_str(), "org.freedesktop.GeoClue2.Client")?;
    client.set_property("DesktopId", desktop_id)?;
    client.set_property("RequestedAccuracyLevel", crate::geoclue_agent::ACCURACY_EXACT)?;
    client.set_property("DistanceThreshold", distance_m)?;
    let mut updates = client.receive_signal("LocationUpdated")?;
    client.call_method("Start", &())?;

    // последняя известная точка (если клиент уже что-то знает)
    if let Ok(p) = client.get_property::<OwnedObjectPath>("Location") {
        if p.as_str() != "/" {
            if let Some(f) = read_fix(&conn, p.as_str()) {
                on_status(Status::Fix(f));
            }
        }
    }
    // сигналы ждём в отдельном потоке: итератор блокирует, а остановку надо видеть
    let (tx, rx) = std::sync::mpsc::channel::<OwnedObjectPath>();
    std::thread::Builder::new()
        .name("geoclue-signals".into())
        .spawn(move || {
            for msg in &mut updates {
                let Ok((_old, new)) = msg.body().deserialize::<(OwnedObjectPath, OwnedObjectPath)>() else { continue };
                if tx.send(new).is_err() {
                    break;
                }
            }
        })
        .ok();
    let result = loop {
        if stop.load(Ordering::SeqCst) {
            break Ok(());
        }
        match rx.recv_timeout(Duration::from_millis(300)) {
            Ok(p) => {
                if let Some(f) = read_fix(&conn, p.as_str()) {
                    on_status(Status::Fix(f));
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // клиент жив? (GeoClue перезапустился — свойство не читается)
                if client.get_property::<bool>("Active").map(|a| !a).unwrap_or(true) {
                    break Err(zbus::Error::Failure(t!("клиент GeoClue остановлен (агент отказал или служба перезапущена)").into()));
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break Err(zbus::Error::Failure(t!("сигналы GeoClue прекратились").into())),
        }
    };
    let _ = client.call_method("Stop", &());
    result
}

fn read_fix(conn: &Connection, path: &str) -> Option<Fix> {
    let loc = Proxy::new(conn, GEOCLUE, path, "org.freedesktop.GeoClue2.Location").ok()?;
    let lat: f64 = loc.get_property("Latitude").ok()?;
    let lon: f64 = loc.get_property("Longitude").ok()?;
    let accuracy: f64 = loc.get_property("Accuracy").unwrap_or(f64::NAN);
    // GeoClue: неизвестное — -1 (курс, скорость) или -f64::MAX (высота)
    let opt = |v: Option<f64>, min: f64| v.filter(|x| x.is_finite() && *x >= min);
    Some(Fix {
        lat,
        lon,
        accuracy,
        altitude: opt(loc.get_property("Altitude").ok(), -1.0e6),
        heading: opt(loc.get_property("Heading").ok(), 0.0),
        speed: opt(loc.get_property("Speed").ok(), 0.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn method_error(name: &str, text: &str) -> zbus::Error {
        let msg = zbus::Message::method_call("/", "X").unwrap().build(&()).unwrap();
        zbus::Error::MethodError(name.try_into().unwrap(), Some(text.into()), msg)
    }

    #[test]
    fn classify_geoclue_errors() {
        let denied = "org.freedesktop.DBus.Error.AccessDenied";
        assert_eq!(Problem::classify(&method_error(denied, "Geolocation disabled for UID 1000")), Problem::Disabled);
        assert_eq!(
            Problem::classify(&method_error(denied, "Agent rejected 'syn-maps' for user '1000'. Please ensure that 'syn-maps' has installed a valid syn-maps.desktop file.")),
            Problem::Denied
        );
        assert_eq!(Problem::classify(&method_error(denied, "'syn-maps' disallowed, no agent for UID 1000")), Problem::NoAgent);
        assert_eq!(
            Problem::classify(&method_error("org.freedesktop.DBus.Error.NameHasNoOwner", "Could not activate remote peer 'org.freedesktop.GeoClue2': startup job failed")),
            Problem::NoService
        );
        assert_eq!(Problem::classify(&method_error("org.freedesktop.DBus.Error.ServiceUnknown", "The name is not activatable")), Problem::NoService);
        assert!(matches!(Problem::classify(&zbus::Error::Failure("x".into())), Problem::Other(t) if t == "x"));
    }
}
