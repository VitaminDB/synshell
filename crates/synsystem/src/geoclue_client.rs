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
    Error(String),
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
                    on_status(Status::Error(e.to_string()));
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
