//! Локальные действия над этой машиной: композитор (окна, действия,
//! ввод), снимок экрана. Их выполняет и сервер вызовов (для другого
//! устройства), и локальный сокет для устройства `local`.
//!
//! Всё блокирующее (синхронный клиент IPC композитора) — вызывать из
//! `spawn_blocking`.

use anyhow::{bail, Context, Result};
use synshell_common::ipc::{Client, InputEvent, Request, Response};

pub fn wm(req: Request) -> Result<Response> {
    let mut c = Client::connect().context("нет связи с композитором")?;
    Ok(c.request(&req)?)
}

/// Ошибка старого композитора (до synlink) — понятным текстом.
pub fn old_wm(message: &str) -> Option<&'static str> {
    message
        .contains("unknown variant")
        .then_some("композитор этой машины старый (без удалённого ввода и потока кадров) — обновите synshell (synwm) и перезайдите")
}

pub fn input(output: Option<String>, events: Vec<InputEvent>) -> Result<()> {
    match wm(Request::Input { output, events })? {
        Response::Ok => Ok(()),
        Response::Error { message } => bail!("{}", old_wm(&message).map(String::from).unwrap_or(message)),
        other => bail!("неожиданный ответ: {other:?}"),
    }
}

/// Кадр вывода целиком: (вывод, ширина, высота, RGBA). Через поток кадров
/// композитора (работает и на pixman, и при блокировке экрана), у старого
/// композитора — через `capture`.
pub fn frame(output: Option<String>) -> Result<(String, u32, u32, Vec<u8>)> {
    let mut c = Client::connect().context("нет связи с композитором")?;
    match c.request(&Request::FrameStream { output: output.clone(), cursor: false })? {
        Response::Frame { frame } => {
            let data = std::fs::read(&frame.path).context("файл кадра")?;
            let need = frame.width as usize * frame.height as usize * 4;
            if data.len() < need {
                bail!("кадр неполный");
            }
            Ok((frame.output, frame.width, frame.height, data[..need].to_vec()))
        }
        Response::Error { message } if !message.starts_with("неверный запрос") => bail!("{message}"),
        _ => {
            // Композитор без потока кадров.
            let mut c = Client::connect()?;
            let Response::Capture { capture } = c.request(&Request::Capture)? else { bail!("захват экрана не поддерживается") };
            let o = match &output {
                Some(n) => capture.outputs.iter().find(|o| &o.name == n),
                None => capture.outputs.first(),
            }
            .context("нет такого вывода")?;
            let data = std::fs::read(&o.path)?;
            for x in &capture.outputs {
                let _ = std::fs::remove_file(&x.path);
            }
            Ok((o.name.clone(), o.width, o.height, data))
        }
    }
}

/// Снимок в PNG: (вывод, ширина, высота, PNG).
pub fn screenshot(output: Option<String>) -> Result<(String, u32, u32, Vec<u8>)> {
    let (name, w, h, rgba) = frame(output)?;
    Ok((name, w, h, encode_png(w, h, &rgba)?))
}

pub fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut wr = enc.write_header()?;
        wr.write_image_data(rgba)?;
    }
    Ok(out)
}

/// Уменьшить PNG так, чтобы длинная сторона была не больше `max`
/// (для MCP: картинки поменьше — дешевле). Усреднение по блокам.
pub fn downscale_png(png_data: &[u8], max: u32) -> Result<(u32, u32, Vec<u8>)> {
    let dec = png::Decoder::new(std::io::Cursor::new(png_data));
    let mut rd = dec.read_info()?;
    let mut buf = vec![0u8; rd.output_buffer_size()];
    let info = rd.next_frame(&mut buf)?;
    let (w, h) = (info.width, info.height);
    if w.max(h) <= max || info.color_type != png::ColorType::Rgba {
        return Ok((w, h, png_data.to_vec()));
    }
    let k = w.max(h) as f64 / max as f64;
    let (nw, nh) = (((w as f64) / k).round().max(1.0) as u32, ((h as f64) / k).round().max(1.0) as u32);
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        let y0 = (y as f64 * k) as u32;
        let y1 = (((y + 1) as f64 * k) as u32).clamp(y0 + 1, h);
        for x in 0..nw {
            let x0 = (x as f64 * k) as u32;
            let x1 = (((x + 1) as f64 * k) as u32).clamp(x0 + 1, w);
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for yy in y0..y1 {
                for xx in x0..x1 {
                    let i = ((yy * w + xx) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += buf[i + c] as u32;
                    }
                    n += 1;
                }
            }
            let o = ((y * nw + x) * 4) as usize;
            for c in 0..4 {
                out[o + c] = (acc[c] / n.max(1)) as u8;
            }
        }
    }
    Ok((nw, nh, encode_png(nw, nh, &out)?))
}
