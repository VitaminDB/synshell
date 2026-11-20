//! Найти элемент по описанию на снимке локальной моделью со зрением
//! (synaptixd): значки без подписи, игры, нарисованный интерфейс — там, где нет
//! ни дерева элементов, ни текста. Последний запасной путь: модель грузится в
//! память видеокарты, ответ — секунды.

use anyhow::{bail, Context, Result};
use synaptixd::{Client, Media, MediaKind, Message, ModelKind, Params, MEDIA_MARK};
use synshell_tr::t;

/// Модель со зрением: заданная или самая лёгкая из умеющих картинки.
fn pick_model(c: &mut Client, want: Option<&str>) -> Result<String> {
    if let Some(m) = want.filter(|m| !m.is_empty()) {
        return Ok(m.to_string());
    }
    let mut list: Vec<_> = c.list()?.into_iter().filter(|m| m.kind == ModelKind::Llm).collect();
    list.sort_by_key(|m| m.disk_bytes);
    for m in list {
        if c.llm_info(&m.id).map(|i| i.media).unwrap_or(false) {
            return Ok(m.id);
        }
    }
    bail!("{}", t!("в synaptixd нет языковой модели со зрением"))
}

/// Центр элемента «what» на PNG: доли ширины и высоты (0…1) и модель.
pub fn locate(png: &[u8], what: &str, model: Option<&str>) -> Result<(f64, f64, String)> {
    let cfg = synshell_common::agent::load();
    if !cfg.vision {
        bail!("{}", t!("поиск моделью со зрением выключен (Параметры → Ассистент → Управление компьютером) — посмотрите снимок окна сами и нажмите по координатам"));
    }
    let model = model.filter(|m| !m.is_empty()).or(Some(cfg.vision_model.as_str()));
    let mut c = Client::connect().context(t!("нет службы synaptixd"))?;
    let model = pick_model(&mut c, model)?;
    let path = synshell_common::paths::runtime_dir().join(format!("synlink-vision-{}.png", std::process::id()));
    std::fs::write(&path, png)?;
    let (w, h, _) = crate::winshot::decode_rgba(png)?;
    let mut msg = Message::user(format!(
        "{MEDIA_MARK}\nНа снимке экрана ({w}×{h}) найди элемент интерфейса: «{what}». \
Ответь только координатами его центра: x,y — целые от 0 до 1000 (доли ширины и высоты снимка). \
Если такого элемента нет — ответь none."
    ));
    msg.media.push(Media { kind: MediaKind::Image, path: path.to_string_lossy().into_owned() });
    let params = Params::new().with("temperature", 0.0).with("thinking", false);
    let r = c.chat(&model, vec![msg], Vec::new(), &params, |_| {});
    let _ = std::fs::remove_file(&path);
    let text = r?.text;
    let (x, y) = parse_point(&text, w as f64, h as f64).with_context(|| t!("модель {model} не нашла «{what}»: {text}", model = model, what = what, text = text.trim()))?;
    Ok((x, y, model))
}

/// «x,y» в тысячных (или рамка из четырёх чисел, или пиксели) → доли.
pub fn parse_point(text: &str, w: f64, h: f64) -> Option<(f64, f64)> {
    if text.to_lowercase().contains("none") {
        return None;
    }
    let nums: Vec<f64> = text
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .filter_map(|s| s.trim_matches('.').parse::<f64>().ok())
        .collect();
    let (x, y) = match nums.as_slice() {
        [x0, y0, x1, y1, ..] if text.contains("box") || nums.len() == 4 => ((x0 + x1) / 2.0, (y0 + y1) / 2.0),
        [x, y, ..] => (*x, *y),
        _ => return None,
    };
    // Больше 1000 — модель ответила в пикселях снимка.
    let (fx, fy) = if x > 1000.0 || y > 1000.0 { (x / w, y / h) } else { (x / 1000.0, y / 1000.0) };
    ((0.0..=1.0).contains(&fx) && (0.0..=1.0).contains(&fy)).then_some((fx, fy))
}

#[cfg(test)]
mod tests {
    #[test]
    fn points() {
        assert_eq!(super::parse_point("512,250", 800.0, 600.0), Some((0.512, 0.25)));
        assert_eq!(super::parse_point("<box>100 200 300 400</box>", 800.0, 600.0), Some((0.2, 0.3)));
        assert_eq!(super::parse_point("(1600, 300)", 1600.0, 600.0), Some((1.0, 0.5)));
        assert_eq!(super::parse_point("none", 1.0, 1.0), None);
    }
}
