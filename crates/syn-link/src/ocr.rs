//! Распознавание текста на снимке (tesseract): запасной путь, когда у окна
//! нет дерева элементов — меню Wine, графические кнопки, игры, OpenGL.
//! Узлы — слова и строки с рамками в пикселях картинки.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use synshell_tr::t;

use crate::uitree::Node;

/// Мелкий текст интерфейса tesseract читает плохо — увеличиваем вдвое (до этой высоты).
const UPSCALE_BELOW: u32 = 1400;

pub fn available() -> bool {
    Command::new("tesseract").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
}

/// Слова и строки на PNG (рамки — в пикселях исходной картинки).
pub fn read(png: &[u8]) -> Result<Vec<Node>> {
    if !available() {
        bail!("{}", t!("нет tesseract (пакеты tesseract, tesseract-data-rus, tesseract-data-eng)"));
    }
    let (w, h, rgba) = crate::winshot::decode_rgba(png)?;
    let k = if h < UPSCALE_BELOW { 2u32 } else { 1 };
    let input = if k == 1 {
        png.to_vec()
    } else {
        let (nw, nh) = (w * k, h * k);
        let mut out = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let s = (((y / k) * w + x / k) * 4) as usize;
                let d = ((y * nw + x) * 4) as usize;
                out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
            }
        }
        crate::services::encode_png(nw, nh, &out)?
    };
    let mut child = Command::new("tesseract")
        .args(["stdin", "stdout", "-l", "rus+eng", "--psm", "11", "tsv"])
        .env("OMP_THREAD_LIMIT", "2")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("tesseract")?;
    {
        let mut si = child.stdin.take().context("stdin")?;
        let data = input;
        std::thread::spawn(move || {
            let _ = si.write_all(&data);
        });
    }
    let out = child.wait_with_output()?;
    Ok(parse_tsv(&String::from_utf8_lossy(&out.stdout), k as f64))
}

/// TSV tesseract → слова (role `ocr`) и строки из нескольких слов (`ocr-line`).
pub fn parse_tsv(tsv: &str, k: f64) -> Vec<Node> {
    let mut words: Vec<((i64, i64, i64), [f64; 4], String)> = Vec::new();
    for l in tsv.lines().skip(1) {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() < 12 || f[0] != "5" {
            continue;
        }
        let conf: f64 = f[10].parse().unwrap_or(-1.0);
        let text = f[11].trim();
        if conf < 30.0 || text.is_empty() {
            continue;
        }
        let n = |i: usize| f[i].parse::<f64>().unwrap_or(0.0) / k;
        let key = (f[2].parse().unwrap_or(0), f[3].parse().unwrap_or(0), f[4].parse().unwrap_or(0));
        words.push((key, [n(6), n(7), n(8), n(9)], text.to_string()));
    }
    let mut nodes = Vec::new();
    let mut id = 1u64;
    let mut i = 0;
    while i < words.len() {
        let mut j = i;
        while j < words.len() && words[j].0 == words[i].0 {
            j += 1;
        }
        let line = &words[i..j];
        if line.len() > 1 {
            let x0 = line.iter().map(|w| w.1[0]).fold(f64::MAX, f64::min);
            let y0 = line.iter().map(|w| w.1[1]).fold(f64::MAX, f64::min);
            let x1 = line.iter().map(|w| w.1[0] + w.1[2]).fold(0.0, f64::max);
            let y1 = line.iter().map(|w| w.1[1] + w.1[3]).fold(0.0, f64::max);
            let text = line.iter().map(|w| w.2.as_str()).collect::<Vec<_>>().join(" ");
            nodes.push(Node { id, role: "ocr-line".into(), bounds: [x0, y0, x1 - x0, y1 - y0], label: Some(text), ..Default::default() });
            id += 1;
        }
        for w in line {
            nodes.push(Node { id, depth: 1, role: "ocr".into(), bounds: w.1, label: Some(w.2.clone()), ..Default::default() });
            id += 1;
        }
        i = j;
    }
    nodes
}

#[cfg(test)]
mod tests {
    #[test]
    fn tsv() {
        let t = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
5\t1\t1\t1\t1\t1\t20\t40\t60\t20\t95\tФайл\n\
5\t1\t1\t1\t1\t2\t100\t40\t80\t20\t91\tПравка\n\
5\t1\t2\t1\t1\t1\t20\t200\t40\t20\t10\tмусор\n";
        let n = super::parse_tsv(t, 2.0);
        assert_eq!(n.len(), 3);
        assert_eq!(n[0].label.as_deref(), Some("Файл Правка"));
        assert_eq!(n[0].bounds, [10.0, 20.0, 80.0, 10.0]);
        assert_eq!(n[2].label.as_deref(), Some("Правка"));
        assert_eq!(n[2].bounds, [50.0, 20.0, 40.0, 10.0]);
    }
}
