//! Ориентация телефона по акселерометру (iio-sensor-proxy, D-Bus `net.hadess.SensorProxy`): поворот
//! устройства по часовой от естественного портрета — для EXIF снимков, поворота видео и превью в альбомном окне.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

static DEGREES: AtomicU32 = AtomicU32::new(0);

fn parse(v: &str) -> Option<u32> {
    // «левая сторона вверху» — телефон повёрнут на 90° по часовой
    match v {
        "normal" => Some(0),
        "left-up" => Some(90),
        "bottom-up" => Some(180),
        "right-up" => Some(270),
        _ => None,
    }
}

/// Слушать датчик в фоне; `on_change` — из фонового потока.
pub fn start(on_change: impl Fn(u32) + Send + Sync + 'static) {
    std::thread::Builder::new()
        .name("orientation".into())
        .spawn(move || loop {
            if let Err(e) = listen(&on_change) {
                tracing::debug!("акселерометр: {e}");
            }
            std::thread::sleep(Duration::from_secs(30));
        })
        .ok();
}

fn listen(on_change: &(impl Fn(u32) + Send + Sync)) -> zbus::Result<()> {
    let conn = zbus::blocking::Connection::system()?;
    let proxy = zbus::blocking::Proxy::new(&conn, "net.hadess.SensorProxy", "/net/hadess/SensorProxy", "net.hadess.SensorProxy")?;
    let has: bool = proxy.get_property("HasAccelerometer")?;
    if !has {
        return Err(zbus::Error::Failure("нет акселерометра".into()));
    }
    proxy.call_method("ClaimAccelerometer", &())?;
    let deliver = |v: &str| {
        if let Some(d) = parse(v) {
            if DEGREES.swap(d, Ordering::Relaxed) != d {
                on_change(d);
            }
        }
    };
    let changes = proxy.receive_property_changed::<String>("AccelerometerOrientation");
    if let Ok(v) = proxy.get_property::<String>("AccelerometerOrientation") {
        deliver(&v);
    }
    for c in changes {
        if let Ok(v) = c.get() {
            deliver(&v);
        }
    }
    Err(zbus::Error::Failure("поток свойств закрыт".into()))
}
