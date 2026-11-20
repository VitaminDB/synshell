//! Окна для агента (MCP): найти окно по программе, заголовку, pid или id,
//! снять только его (вместе с его меню и подсказками — окнами X11 того же
//! процесса) и пересчитать координаты снимка окна в пиксели экрана.

use anyhow::{bail, Context, Result};
use synshell_common::ipc::{self, OutputInfo, WindowInfo};
use synshell_common::link::{Request, Response};
use synshell_tr::t;

use crate::gestures;

pub fn windows(device: &str) -> Result<Vec<WindowInfo>> {
    match gestures::req(&Request::Wm { device: device.into(), wm: ipc::Request::Windows })? {
        Response::Wm { wm: ipc::Response::Windows { windows } } => Ok(windows),
        _ => bail!("{}", t!("нет списка окон")),
    }
}

pub fn outputs(device: &str) -> Result<Vec<OutputInfo>> {
    match gestures::req(&Request::Wm { device: device.into(), wm: ipc::Request::Outputs })? {
        Response::Wm { wm: ipc::Response::Outputs { outputs } } => Ok(outputs),
        _ => bail!("{}", t!("нет списка выводов")),
    }
}

/// Подходит ли окно под запрос: `id:N`, `pid:N`, иначе часть app_id или
/// заголовка (без учёта регистра). Вес: точный app_id > часть app_id > часть заголовка.
fn score(w: &WindowInfo, q: &str) -> Option<u32> {
    let q = q.trim();
    if let Some(id) = q.strip_prefix("id:") {
        return (id.trim().parse::<u64>().ok() == Some(w.id)).then_some(100);
    }
    if let Some(pid) = q.strip_prefix("pid:") {
        return (pid.trim().parse::<i32>().ok() == w.pid).then_some(if w.override_redirect { 1 } else { 50 });
    }
    let ql = q.to_lowercase();
    let app = w.app_id.to_lowercase();
    let title = w.title.to_lowercase();
    if ql.is_empty() {
        return None;
    }
    if app == ql {
        Some(40)
    } else if app.contains(&ql) {
        Some(30)
    } else if title.contains(&ql) {
        Some(20)
    } else {
        None
    }
}

/// Окно по запросу. Меню и подсказки (override-redirect) выбираются только по
/// `id:`; из равных — с фокусом, затем самое большое.
pub fn pick<'a>(list: &'a [WindowInfo], q: &str) -> Result<&'a WindowInfo> {
    let by_id = q.trim().starts_with("id:");
    list.iter()
        .filter(|w| by_id || !w.override_redirect)
        .filter_map(|w| score(w, q).map(|s| (s, w)))
        .max_by_key(|(s, w)| (*s, w.geometry[2] > 8 && w.geometry[3] > 8, w.focused, !w.minimized, w.geometry[2] as i64 * w.geometry[3] as i64))
        .map(|(_, w)| w)
        .with_context(|| {
            let names: Vec<String> = list.iter().filter(|w| !w.override_redirect).map(|w| format!("«{}» ({})", w.title, w.app_id)).collect();
            t!("нет окна «{q}»; есть: {names}", q = q, names = names.join(", "))
        })
}

/// Что снимать: окно с заголовком композитора и всё, что открыто от того же
/// процесса поверх (меню, списки, подсказки X11), — в логических координатах.
pub fn region(list: &[WindowInfo], w: &WindowInfo) -> [i32; 4] {
    let mut r = [w.geometry[0], w.geometry[1] - w.decor_top, w.geometry[2], w.geometry[3] + w.decor_top];
    if let Some(pid) = w.pid {
        for o in list.iter().filter(|o| o.override_redirect && o.pid == Some(pid) && o.workspace == w.workspace && !o.minimized) {
            let (x0, y0) = (r[0].min(o.geometry[0]), r[1].min(o.geometry[1]));
            let x1 = (r[0] + r[2]).max(o.geometry[0] + o.geometry[2]);
            let y1 = (r[1] + r[3]).max(o.geometry[1] + o.geometry[3]);
            r = [x0, y0, x1 - x0, y1 - y0];
        }
    }
    r
}

/// Вывод окна: по имени из окна, иначе тот, где его левый верхний угол.
pub fn output_of<'a>(outs: &'a [OutputInfo], w: &WindowInfo) -> Option<&'a OutputInfo> {
    w.output
        .as_deref()
        .and_then(|n| outs.iter().find(|o| o.name == n))
        .or_else(|| {
            outs.iter().find(|o| {
                let g = o.geometry;
                (g[0]..g[0] + g[2]).contains(&w.geometry[0]) && (g[1]..g[1] + g[3]).contains(&w.geometry[1])
            })
        })
        .or_else(|| outs.first())
}

/// Логический прямоугольник → пиксели кадра вывода шириной `frame_w`, обрезанный по кадру.
pub fn to_frame(o: &OutputInfo, frame_w: f64, frame_h: f64, r: [i32; 4]) -> Option<[u32; 4]> {
    let k = frame_w / o.geometry[2].max(1) as f64;
    let x0 = ((r[0] - o.geometry[0]) as f64 * k).round().clamp(0.0, frame_w);
    let y0 = ((r[1] - o.geometry[1]) as f64 * k).round().clamp(0.0, frame_h);
    let x1 = ((r[0] + r[2] - o.geometry[0]) as f64 * k).round().clamp(0.0, frame_w);
    let y1 = ((r[1] + r[3] - o.geometry[1]) as f64 * k).round().clamp(0.0, frame_h);
    (x1 > x0 && y1 > y0).then(|| [x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32])
}

/// Вырезать часть PNG.
pub fn crop_png(png_data: &[u8], c: [u32; 4]) -> Result<Vec<u8>> {
    let (w, h, rgba) = decode_rgba(png_data)?;
    let [x, y, cw, ch] = c;
    if x + cw > w || y + ch > h {
        bail!("{}", t!("область за краем снимка"));
    }
    let mut out = Vec::with_capacity((cw * ch * 4) as usize);
    for row in y..y + ch {
        let s = ((row * w + x) * 4) as usize;
        out.extend_from_slice(&rgba[s..s + (cw * 4) as usize]);
    }
    crate::services::encode_png(cw, ch, &out)
}

pub fn decode_rgba(png_data: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(png_data));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut rd = dec.read_info()?;
    let mut buf = vec![0u8; rd.output_buffer_size()];
    let info = rd.next_frame(&mut buf)?;
    buf.truncate(info.buffer_size());
    if info.color_type != png::ColorType::Rgba {
        bail!("{}", t!("снимок не RGBA"));
    }
    Ok((info.width, info.height, buf))
}

/// Короткая строка об окне для списка.
pub fn line(w: &WindowInfo) -> String {
    let mut flags = Vec::new();
    for (on, f) in [
        (w.focused, "focused"),
        (w.minimized, "minimized"),
        (w.maximized, "maximized"),
        (w.fullscreen, "fullscreen"),
        (w.override_redirect, "popup"),
        (w.urgent, "urgent"),
    ] {
        if on {
            flags.push(f);
        }
    }
    let g = w.geometry;
    let mut s = format!(
        "id:{} {} «{}» pid:{} стол {} [{} {} {}×{}]",
        w.id,
        if w.app_id.is_empty() { "-" } else { &w.app_id },
        w.title,
        w.pid.map(|p| p.to_string()).unwrap_or_else(|| "?".into()),
        w.workspace + 1,
        g[0],
        g[1],
        g[2],
        g[3]
    );
    if let (Some(x), Some(sc)) = (w.x11_geometry, w.x11_scale) {
        s.push_str(&format!(" X11 [{} {} {}×{}]×{sc}", x[0], x[1], x[2], x[3]));
        // X-программа думает, что окно в другом месте (меню откроются не там).
        let exp = |v: i32| (v as f64 * sc).round() as i32;
        if !w.override_redirect && ((x[0] - exp(g[0])).abs() > 2 || (x[1] - exp(g[1])).abs() > 2) {
            s.push_str(" ⚠ X11-положение не совпадает");
        }
    }
    if !flags.is_empty() {
        s.push_str(&format!(" {}", flags.join(",")));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(id: u64, app: &str, title: &str, pid: i32, g: [i32; 4], or: bool) -> WindowInfo {
        WindowInfo { id, app_id: app.into(), title: title.into(), pid: Some(pid), geometry: g, override_redirect: or, ..Default::default() }
    }

    #[test]
    fn pick_and_region() {
        let list = vec![
            win(1, "synfiles", "ShapeLab", 10, [0, 0, 800, 600], false),
            win(2, "shapelab-d104.exe", "ShapeLab — Без имени *", 20, [700, 30, 1300, 860], false),
            win(3, "shapelab-d104.exe", "", 20, [720, 60, 210, 120], true),
        ];
        assert_eq!(pick(&list, "shapelab-d104.exe").unwrap().id, 2);
        assert_eq!(pick(&list, "Без имени").unwrap().id, 2);
        assert_eq!(pick(&list, "pid:20").unwrap().id, 2);
        assert_eq!(pick(&list, "id:3").unwrap().id, 3);
        assert_eq!(pick(&list, "synfiles").unwrap().id, 1);
        assert!(pick(&list, "нет-такого").is_err());
        let mut w = list[1].clone();
        w.decor_top = 34;
        let mut l2 = list.clone();
        l2[2].geometry = [650, 40, 200, 300];
        assert_eq!(region(&l2, &w), [650, -4, 1350, 894]);
    }

    #[test]
    fn frame_rect() {
        let o = OutputInfo { geometry: [0, 0, 2000, 1125], ..Default::default() };
        assert_eq!(to_frame(&o, 2560.0, 1440.0, [100, 50, 1000, 500]), Some([128, 64, 1280, 640]));
        assert_eq!(to_frame(&o, 2560.0, 1440.0, [-10, -10, 20, 20]), Some([0, 0, 13, 13]));
        assert_eq!(to_frame(&o, 2560.0, 1440.0, [3000, 0, 10, 10]), None);
    }
}
