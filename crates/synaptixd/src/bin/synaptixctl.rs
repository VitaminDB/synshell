use std::process::ExitCode;

use synaptixd::{Client, DeviceKind, ModelEntry, SettingsChange};

fn mb(b: u64) -> String {
    format!("{} МБ", b >> 20)
}

fn print_entry(m: &ModelEntry) {
    let dev = m.device.map(|d| d.id()).unwrap_or("-");
    let fb = if m.fallback { " (откат)" } else { "" };
    println!(
        "{:<28} {:<5} {:<9} {:<5}{} pref={:<4} {:>9} диск {:>9} клиентов {}{}{}",
        m.id,
        format!("{:?}", m.kind).to_lowercase(),
        format!("{:?}", m.state).to_lowercase(),
        dev,
        fb,
        m.settings.device.id(),
        mb(m.mem_bytes),
        mb(m.disk_bytes),
        m.clients,
        if m.settings.pinned { " закреплена" } else { "" },
        m.error.as_ref().map(|e| format!(" ошибка: {e}")).unwrap_or_default(),
    );
}

fn read_wav(path: &str) -> Result<(Vec<f32>, u32), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("не WAV".into());
    }
    let mut pos = 12;
    let (mut ch, mut sr, mut bits, mut fmt) = (1u16, 16000u32, 16u16, 1u16);
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = &bytes[pos + 8..(pos + 8 + len).min(bytes.len())];
        if id == b"fmt " {
            fmt = u16::from_le_bytes([body[0], body[1]]);
            ch = u16::from_le_bytes([body[2], body[3]]);
            sr = u32::from_le_bytes(body[4..8].try_into().unwrap());
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            let samples: Vec<f32> = match (fmt, bits) {
                (1, 16) => body.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect(),
                (3, 32) => body.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect(),
                _ => return Err(format!("формат WAV {fmt}/{bits} не поддержан")),
            };
            let mono = samples.chunks(ch as usize).map(|f| f.iter().sum::<f32>() / f.len() as f32).collect();
            return Ok((mono, sr));
        }
        pos += 8 + len + (len & 1);
    }
    Err("нет данных в WAV".into())
}

fn run(args: &[String]) -> Result<(), String> {
    let mut c = Client::connect().map_err(|e| e.to_string())?;
    let arg = |i: usize| args.get(i).map(String::as_str).ok_or_else(|| "мало аргументов".to_string());
    match args.first().map(String::as_str).unwrap_or("list") {
        "list" => {
            for m in c.list().map_err(|e| e.to_string())? {
                print_entry(&m);
            }
        }
        "status" => {
            let s = c.status().map_err(|e| e.to_string())?;
            println!(
                "память {} из {}, моделей загружено {} ({}), выгрузок {}, устройства {:?}",
                mb(s.mem_available),
                mb(s.mem_total),
                s.loaded,
                mb(s.models_bytes),
                s.evictions,
                s.devices.iter().map(|d| d.id()).collect::<Vec<_>>()
            );
        }
        "load" => print_entry(&c.load(arg(1)?).map_err(|e| e.to_string())?),
        "unload" => c.unload(arg(1)?).map_err(|e| e.to_string())?,
        "rescan" => {
            c.call(synaptixd::Request::Rescan, &[]).map_err(|e| e.to_string())?;
        }
        "set" => {
            let id = arg(1)?;
            let mut ch = SettingsChange::default();
            for kv in &args[2..] {
                let (k, v) = kv.split_once('=').ok_or("ожидается ключ=значение")?;
                match k {
                    "device" => ch.device = Some(DeviceKind::parse(v).ok_or("устройство: auto|cpu|cuda|gpu|npu")?),
                    "pinned" => ch.pinned = Some(v == "1" || v == "true"),
                    "preload" => ch.preload = Some(v == "1" || v == "true"),
                    "idle" => ch.idle_unload_min = Some(if v == "default" { None } else { Some(v.parse().map_err(|_| "idle: минуты")?) }),
                    _ => return Err(format!("неизвестный параметр {k}")),
                }
            }
            print_entry(&c.set(id, ch).map_err(|e| e.to_string())?);
        }
        "ctc" => {
            let (pcm, sr) = read_wav(arg(2)?)?;
            let info = c.open(arg(1)?).map_err(|e| e.to_string())?;
            let t0 = std::time::Instant::now();
            let out = c.ctc(arg(1)?, &pcm, sr, true).map_err(|e| e.to_string())?;
            let dt = t0.elapsed();
            let ctc = info.ctc.ok_or("модель не CTC")?;
            let lp = out.logprobs.f32s();
            let mut prev = u32::MAX;
            let mut text = Vec::new();
            for t in 0..out.frames {
                let row = &lp[t * out.vocab..(t + 1) * out.vocab];
                let best = row.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i as u32).unwrap_or(0);
                if best != prev && best != ctc.blank_id {
                    text.push(ctc.vocab[best as usize].clone());
                }
                prev = best;
            }
            println!("{} кадров, словарь {}, hidden {} за {:.0} мс на {:?}", out.frames, out.vocab, out.hidden_dim, dt.as_secs_f64() * 1e3, info.entry.device);
            println!("{}", text.join(" "));
        }
        "watch" => {
            c.subscribe(|ev| {
                match ev {
                    synaptixd::Event::ModelState { model } => print_entry(&model),
                    other => println!("{other:?}"),
                }
                true
            })
            .map_err(|e| e.to_string())?;
        }
        other => return Err(format!("неизвестная команда {other}: list|status|load|unload|set|ctc|watch|rescan")),
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("synaptixctl: {e}");
            ExitCode::FAILURE
        }
    }
}
