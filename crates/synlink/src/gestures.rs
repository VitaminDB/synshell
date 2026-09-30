//! Жесты для CLI и MCP: координаты — пиксели снимка экрана устройства
//! (как на картинке `synlink shot`), доли 0..1 — тоже можно. На сенсорных
//! устройствах — касания, на компьютере — мышь.

use std::time::Duration;

use anyhow::{bail, Result};
use synshell_common::ipc::{self, InputEvent};
use synshell_common::link::{self, Request, Response};

pub struct Screen {
    pub output: String,
    pub w: f64,
    pub h: f64,
    pub touch: bool,
}

pub fn req(r: &Request) -> Result<Response> {
    match link::request(r)? {
        Response::Error { message } => bail!("{message}"),
        other => Ok(other),
    }
}

pub fn screen(device: &str, output: Option<&str>) -> Result<Screen> {
    let Response::Wm { wm: ipc::Response::Outputs { outputs } } =
        req(&Request::Wm { device: device.into(), wm: ipc::Request::Outputs })?
    else {
        bail!("нет списка выводов")
    };
    let o = match output {
        Some(n) => outputs.iter().find(|o| o.name == n),
        None => outputs.first(),
    };
    let Some(o) = o else { bail!("нет вывода") };
    let touch = match req(&Request::Status)? {
        Response::Status { status } => {
            if matches!(device, "local" | "self" | "") {
                status.me.kind.is_touch()
            } else {
                status
                    .peers
                    .iter()
                    .find(|p| p.id == device || p.name.eq_ignore_ascii_case(device) || crate::ssh::slug(&p.name) == device || p.id.starts_with(device))
                    .map(|p| p.kind.is_touch())
                    .unwrap_or(false)
            }
        }
        _ => false,
    };
    // Пиксели кадра — текущий режим (с поворотом), логика×масштаб округляется иначе.
    let (w, h) = match o.current_mode.and_then(|i| o.modes.get(i)) {
        Some(m) if o.transform.contains("90") || o.transform.contains("270") => (m.height as f64, m.width as f64),
        Some(m) => (m.width as f64, m.height as f64),
        None => (o.geometry[2] as f64 * o.scale, o.geometry[3] as f64 * o.scale),
    };
    Ok(Screen { output: o.name.clone(), w: w.round().max(1.0), h: h.round().max(1.0), touch })
}

impl Screen {
    fn frac(&self, x: f64, y: f64) -> (f64, f64) {
        if x <= 1.0 && y <= 1.0 && (x.fract() != 0.0 || y.fract() != 0.0 || (x == 0.0 && y == 0.0)) {
            (x, y)
        } else {
            (x / self.w, y / self.h)
        }
    }
}

fn send(device: &str, s: &Screen, events: Vec<InputEvent>) -> Result<()> {
    req(&Request::Input { device: device.into(), output: Some(s.output.clone()), events })?;
    Ok(())
}

const BTN_LEFT: u32 = 0x110;

pub fn tap(device: &str, s: &Screen, x: f64, y: f64) -> Result<()> {
    hold(device, s, x, y, 60)
}

pub fn hold(device: &str, s: &Screen, x: f64, y: f64, ms: u64) -> Result<()> {
    let (fx, fy) = s.frac(x, y);
    if s.touch {
        send(device, s, vec![InputEvent::TouchDown { id: 0, x: fx, y: fy }, InputEvent::TouchFrame])?;
        std::thread::sleep(Duration::from_millis(ms));
        send(device, s, vec![InputEvent::TouchUp { id: 0 }, InputEvent::TouchFrame])
    } else {
        send(device, s, vec![InputEvent::Motion { x: fx, y: fy }, InputEvent::Button { button: BTN_LEFT, pressed: true }])?;
        std::thread::sleep(Duration::from_millis(ms));
        send(device, s, vec![InputEvent::Button { button: BTN_LEFT, pressed: false }])
    }
}

pub fn swipe(device: &str, s: &Screen, a: (f64, f64), b: (f64, f64), ms: u64) -> Result<()> {
    let (ax, ay) = s.frac(a.0, a.1);
    let (bx, by) = s.frac(b.0, b.1);
    let steps = (ms / 16).clamp(4, 120);
    let dt = Duration::from_millis(ms / steps);
    if s.touch {
        send(device, s, vec![InputEvent::TouchDown { id: 0, x: ax, y: ay }, InputEvent::TouchFrame])?;
    } else {
        send(device, s, vec![InputEvent::Motion { x: ax, y: ay }, InputEvent::Button { button: BTN_LEFT, pressed: true }])?;
    }
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let (x, y) = (ax + (bx - ax) * t, ay + (by - ay) * t);
        std::thread::sleep(dt);
        let ev = if s.touch {
            vec![InputEvent::TouchMotion { id: 0, x, y }, InputEvent::TouchFrame]
        } else {
            vec![InputEvent::Motion { x, y }]
        };
        send(device, s, ev)?;
    }
    if s.touch {
        send(device, s, vec![InputEvent::TouchUp { id: 0 }, InputEvent::TouchFrame])
    } else {
        send(device, s, vec![InputEvent::Button { button: BTN_LEFT, pressed: false }])
    }
}

/// Прокрутка колесом в точке (шаги; вниз — положительные).
pub fn scroll(device: &str, s: &Screen, x: f64, y: f64, dy: f64, dx: f64) -> Result<()> {
    let (fx, fy) = s.frac(x, y);
    send(device, s, vec![InputEvent::Motion { x: fx, y: fy }, InputEvent::Axis { dx: dx * 15.0, dy: dy * 15.0, discrete: true }])
}

pub fn text(device: &str, t: &str) -> Result<()> {
    req(&Request::Input { device: device.into(), output: None, events: vec![InputEvent::Text { text: t.into() }] })?;
    Ok(())
}

pub fn combo(device: &str, c: &str) -> Result<()> {
    req(&Request::Input { device: device.into(), output: None, events: vec![InputEvent::Combo { combo: c.into() }] })?;
    Ok(())
}

pub fn action(device: &str, a: &str) -> Result<()> {
    let action = a.parse().map_err(|e| anyhow::anyhow!("действие «{a}»: {e}"))?;
    match req(&Request::Wm { device: device.into(), wm: ipc::Request::Action { action } })? {
        Response::Wm { wm: ipc::Response::Error { message } } => bail!("{message}"),
        _ => Ok(()),
    }
}
