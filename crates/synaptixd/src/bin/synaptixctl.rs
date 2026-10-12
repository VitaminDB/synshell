use std::process::ExitCode;

use synaptixd::{Client, DeviceKind, ModelEntry, ModelKind, SettingsChange};

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
        "defaults" => {
            for d in c.defaults().map_err(|e| e.to_string())? {
                let chosen = d.model.unwrap_or_else(|| "auto".into());
                println!("{:<6} {:<28} → {}", d.kind.id(), chosen, d.resolved.unwrap_or_else(|| "-".into()));
            }
        }
        "default" => {
            let kind = ModelKind::parse(arg(1)?).ok_or("вид: llm|image|video|music|tts|asr|embed")?;
            let model = arg(2)?;
            c.set_default(kind, (model != "auto").then_some(model)).map_err(|e| e.to_string())?;
        }
        "params" => {
            let m = c.list().map_err(|e| e.to_string())?.into_iter().find(|m| m.id == args.get(1).cloned().unwrap_or_default()).ok_or("нет такой модели")?;
            let eff = m.effective();
            for k in &m.schema {
                let v = eff.get(&k.key).map(|v| v.display(&k.kind)).unwrap_or_else(|| "авто".into());
                let own = if m.settings.params.get(&k.key).is_some() { " *" } else { "" };
                println!("{:<18} {v}{own}  — {}", k.key, k.label);
            }
        }
        "libraries" => {
            for l in c.libraries().map_err(|e| e.to_string())? {
                println!(
                    "{}{} {:<40} {} моделей {} · свободно {} из {}{}",
                    if l.default { "*" } else { " " },
                    if l.online { " " } else { "!" },
                    l.path,
                    l.models,
                    mb(l.used),
                    mb(l.free),
                    mb(l.total),
                    if l.online { "" } else { " (не подключена)" }
                );
            }
        }
        "library-add" => {
            c.call(synaptixd::Request::AddLibrary { path: arg(1)?.into(), name: args.get(2).cloned() }, &[]).map_err(|e| e.to_string())?;
        }
        "library-remove" => {
            c.call(synaptixd::Request::RemoveLibrary { path: arg(1)?.into() }, &[]).map_err(|e| e.to_string())?;
        }
        "delete" => c.delete_model(arg(1)?).map_err(|e| e.to_string())?,
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
                    "quant" => ch.quant = Some((v != "auto").then(|| v.to_string())),
                    k if k.starts_with("param.") => {
                        let key = k.trim_start_matches("param.");
                        let entry = c.list().map_err(|e| e.to_string())?.into_iter().find(|m| m.id == id).ok_or("нет такой модели")?;
                        let spec = entry.schema.iter().find(|p| p.key == key).ok_or_else(|| format!("у модели нет параметра {key}"))?;
                        let mut params = ch.params.take().unwrap_or(entry.settings.params.clone());
                        let value = if v == "default" { None } else { Some(spec.parse(v).ok_or_else(|| format!("{key}: неверное значение {v}"))?) };
                        params.set(key, value);
                        ch.params = Some(params);
                    }
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
        "gen" => {
            let prompt = args[2..].join(" ");
            let mut params = synaptixd::Params::new();
            if let Some(n) = std::env::var("GEN_MAX").ok().and_then(|v| v.parse::<i64>().ok()) {
                params = params.with("max_tokens", n);
            }
            let model = if arg(1)? == "-" { "" } else { arg(1)? };
            let t0 = std::time::Instant::now();
            let tools: Vec<serde_json::Value> = std::env::var("TOOLS").ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            let g = c
                .chat(model, vec![synaptixd::Message::user(prompt)], tools, &params, |ev| {
                    match ev {
                        synaptixd::Event::Token { text } => print!("{text}"),
                        synaptixd::Event::Thinking { text } => print!("\x1b[2m{text}\x1b[0m"),
                        synaptixd::Event::ToolText { text } => print!("\x1b[33m{text}\x1b[0m"),
                        _ => {}
                    }
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                })
                .map_err(|e| e.to_string())?;
            for call in &g.tool_calls {
                println!("\n→ вызов {}({})", call.name, call.arguments);
            }
            println!("\n[{} токенов за {:.1} с]", g.tokens, t0.elapsed().as_secs_f32());
        }
        "asr" => {
            let (pcm, sr) = read_wav(arg(2)?)?;
            let t0 = std::time::Instant::now();
            let t = c.transcribe(arg(1)?, &pcm, sr, args.get(3).map(String::as_str)).map_err(|e| e.to_string())?;
            println!("{}\n[{:.1} с]", t.text, t0.elapsed().as_secs_f32());
        }
        "say" => {
            let out = arg(2)?;
            let text = args[3..].join(" ");
            let lang = std::env::var("LANG_CODE").ok();
            let t0 = std::time::Instant::now();
            let steps = std::env::var("TTS_STEPS").ok().and_then(|v| v.parse().ok());
            let model = if arg(1)? == "-" { "" } else { arg(1)? };
            let sp = c.speak_steps(model, &text, lang.as_deref(), None, None, steps).map_err(|e| e.to_string())?;
            let pcm = sp.audio.f32s();
            let mut wav = Vec::with_capacity(44 + pcm.len() * 2);
            let data_len = (pcm.len() * 2) as u32;
            wav.extend_from_slice(b"RIFF");
            wav.extend_from_slice(&(36 + data_len).to_le_bytes());
            wav.extend_from_slice(b"WAVEfmt ");
            wav.extend_from_slice(&16u32.to_le_bytes());
            wav.extend_from_slice(&1u16.to_le_bytes());
            wav.extend_from_slice(&1u16.to_le_bytes());
            wav.extend_from_slice(&sp.sample_rate.to_le_bytes());
            wav.extend_from_slice(&(sp.sample_rate * 2).to_le_bytes());
            wav.extend_from_slice(&2u16.to_le_bytes());
            wav.extend_from_slice(&16u16.to_le_bytes());
            wav.extend_from_slice(b"data");
            wav.extend_from_slice(&data_len.to_le_bytes());
            for &x in pcm {
                wav.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
            }
            std::fs::write(out, wav).map_err(|e| e.to_string())?;
            println!("{:.1} с звука за {:.1} с → {out}", pcm.len() as f32 / sp.sample_rate as f32, t0.elapsed().as_secs_f32());
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
        other => return Err(format!("неизвестная команда {other}: list|status|defaults|default|params|libraries|library-add|library-remove|delete|load|unload|set|ctc|asr|gen|say|watch|rescan")),
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
