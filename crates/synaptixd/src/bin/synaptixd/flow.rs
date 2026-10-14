//! `op flow` — граф ядра synaptix-flow целиком в службе. Ноды генерации
//! (LLM, картинка, видео, музыка, речь, расшифровка) и стадии моделей
//! исполняются здесь же, без сокета; тензоры между стадиями — дескрипторы в
//! памяти службы, результаты стадий кэшируются (повторный прогон с другим
//! сидом заново считает только сэмплер).
//!
//! Файлы: входы клиента приходят дескрипторами (`@in:<n>` в графе) и видны
//! нодам ссылками `in/<n>-<имя>` в каталоге прогона; ноды читают и пишут
//! только там (песочница ядра). Файлы выходов уходят клиенту дескрипторами
//! по мере готовности нод; каталог прогона удаляется.

use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;
use synaptix_flow::{self as sf, Cx, Event, Flow, Outputs, RunOpts, Sandbox, StageCache, Val};
use synaptixd::{wire, Error, ErrorCode, FlowInput, FlowReply, ModelKind, Params, Reply};

use crate::manager::Manager;
use crate::{gen, stages};

type R = Result<(), String>;

static MGR: OnceLock<Arc<Manager>> = OnceLock::new();

pub fn mgr() -> &'static Arc<Manager> {
    MGR.get().expect("служба не запущена")
}

/// Исполнители нод службы — один раз.
fn install(m: &Arc<Manager>) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = MGR.set(m.clone());
        sf::ops::provide("llm", llm);
        sf::ops::provide("image", image);
        sf::ops::provide("video", video);
        sf::ops::provide("music", music);
        sf::ops::provide("speak", speak);
        sf::ops::provide("transcribe", transcribe);
        stages::install();
        let _ = std::thread::Builder::new().name("flow-cache".into()).spawn(|| loop {
            std::thread::sleep(Duration::from_secs(60));
            sweep(TTL);
        });
    });
}

// ── Кэш стадий ────────────────────────────────────────────────────────────

/// Сколько держать результат стадии без обращений.
const TTL: Duration = Duration::from_secs(15 * 60);
const CAPACITY: usize = 32;

struct Entry {
    key: u64,
    outputs: Outputs,
    used: Instant,
}

fn entries() -> &'static Mutex<Vec<Entry>> {
    static E: OnceLock<Mutex<Vec<Entry>>> = OnceLock::new();
    E.get_or_init(Default::default)
}

/// Кэш одного пользователя: ключи смешиваются с uid.
struct Cache {
    uid: u32,
}

impl Cache {
    fn key(&self, k: u64) -> u64 {
        k ^ (self.uid as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
    }
}

impl StageCache for Cache {
    fn get(&self, key: u64) -> Option<Outputs> {
        let key = self.key(key);
        let mut e = entries().lock().ok()?;
        let hit = e.iter_mut().find(|x| x.key == key)?;
        hit.used = Instant::now();
        Some(hit.outputs.clone())
    }

    fn put(&self, key: u64, outputs: &Outputs) {
        let key = self.key(key);
        let Ok(mut e) = entries().lock() else { return };
        e.retain(|x| x.key != key);
        e.push(Entry { key, outputs: outputs.clone(), used: Instant::now() });
        while e.len() > CAPACITY {
            let oldest = e.iter().enumerate().min_by_key(|(_, x)| x.used).map(|(i, _)| i).unwrap_or(0);
            e.remove(oldest);
        }
    }
}

/// Выбросить результаты стадий старше `age` (`Duration::ZERO` — все).
pub fn sweep(age: Duration) {
    let dropped = match entries().lock() {
        Ok(mut e) => {
            let before = e.len();
            e.retain(|x| x.used.elapsed() < age);
            before - e.len()
        }
        Err(_) => 0,
    };
    if dropped > 0 {
        tracing::debug!("кэш стадий графа: выброшено {dropped}");
        crate::manager::release_vram(Some(synaptixd::DeviceKind::Cuda));
    }
}

// ── Прогон ────────────────────────────────────────────────────────────────

fn run_dir(id: u64) -> std::io::Result<PathBuf> {
    let base = std::env::var_os("SYNAPTIX_CACHE_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("flow");
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = base.join(format!("{}-{id}-{nanos}", std::process::id()));
    std::fs::create_dir_all(dir.join("in"))?;
    Ok(dir)
}

fn safe_name(s: &str) -> String {
    let n: String = Path::new(s).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default().chars().map(|c| if c == '/' || c == '\0' { '_' } else { c }).collect();
    if n.is_empty() || n == "." || n == ".." {
        "file".into()
    } else {
        n
    }
}

/// Метки `@in:<n>` → ссылки на входы клиента.
struct Inputs {
    paths: Vec<PathBuf>,
}

impl Inputs {
    fn word(&self, w: &str) -> Option<String> {
        synaptixd::flow::marker_index(w, "@in:").and_then(|i| self.paths.get(i)).map(|p| p.display().to_string())
    }

    fn text(&self, s: &str) -> Option<String> {
        if !s.contains("@in:") {
            return None;
        }
        Some(s.split(' ').map(|w| self.word(w).unwrap_or_else(|| w.to_string())).collect::<Vec<_>>().join(" "))
    }

    fn json(&self, v: &mut Value) {
        match v {
            Value::String(s) => {
                if let Some(t) = self.text(s) {
                    *s = t;
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| self.json(x)),
            _ => {}
        }
    }

    fn val(&self, v: &Val) -> Val {
        match v {
            Val::File(p) => Val::File(self.word(&p.to_string_lossy()).map(PathBuf::from).unwrap_or_else(|| p.clone())),
            Val::Text(s) => Val::Text(self.text(s).unwrap_or_else(|| s.clone())),
            Val::List(l) => Val::List(l.iter().map(|x| self.val(x)).collect()),
            other => other.clone(),
        }
    }
}

/// Отправка событий клиенту; обрыв — отмена прогона.
struct Sink<'a> {
    sock: BorrowedFd<'a>,
    id: u64,
    cancel: sf::Cancel,
    next_out: usize,
}

impl Sink<'_> {
    fn send(&self, ev: synaptixd::Event, fds: &[OwnedFd]) {
        let r = Reply { id: self.id, event: Some(ev), ..Default::default() };
        let borrowed: Vec<BorrowedFd> = fds.iter().map(|f| unsafe { BorrowedFd::borrow_raw(f.as_raw_fd()) }).collect();
        if wire::send(self.sock, &serde_json::to_vec(&r).unwrap_or_default(), &borrowed).is_err() {
            self.cancel.cancel();
        }
    }

    /// Файлы выходов — дескрипторами (пачками), в выходах — метки.
    fn outputs(&mut self, o: &Outputs) -> Outputs {
        let mut files: Vec<(OwnedFd, String)> = Vec::new();
        let mut first = self.next_out;
        let mapped: Outputs = o
            .iter()
            .map(|(k, v)| {
                let v = v.map_files(&mut |p| match std::fs::File::open(p) {
                    Ok(f) => {
                        let m = synaptixd::flow::out_marker(self.next_out);
                        self.next_out += 1;
                        files.push((OwnedFd::from(f), p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                        PathBuf::from(m)
                    }
                    Err(_) => p.to_path_buf(),
                });
                (k.clone(), v)
            })
            .collect();
        for chunk in files.chunks_mut(wire::MAX_FDS) {
            let names: Vec<String> = chunk.iter().map(|(_, n)| n.clone()).collect();
            let fds: Vec<OwnedFd> = chunk.iter().filter_map(|(f, _)| f.try_clone().ok()).collect();
            self.send(synaptixd::Event::FlowFiles { first, names }, &fds);
            first += chunk.len();
        }
        mapped
    }
}

pub fn serve(m: &Arc<Manager>, sock: BorrowedFd, id: u64, flow: Value, inputs: Vec<FlowInput>, files: Vec<String>, fds: Vec<OwnedFd>) -> Result<(Value, Vec<OwnedFd>), Error> {
    install(m);
    let proto = |e: String| Error::new(ErrorCode::Protocol, e);
    let mut flow: Flow = serde_json::from_value(flow).map_err(|e| proto(format!("граф: {e}")))?;
    if fds.len() != files.len() {
        return Err(proto(format!("файлов {} при именах {}", fds.len(), files.len())));
    }
    let dir = run_dir(id).map_err(|e| Error::new(ErrorCode::Failed, format!("каталог прогона: {e}")))?;
    let pid = std::process::id();
    let mut paths = Vec::with_capacity(fds.len());
    for (i, (fd, name)) in fds.iter().zip(&files).enumerate() {
        let link = dir.join("in").join(format!("{i}-{}", safe_name(name)));
        std::os::unix::fs::symlink(format!("/proc/{pid}/fd/{}", fd.as_raw_fd()), &link).map_err(|e| Error::new(ErrorCode::Failed, format!("{}: {e}", link.display())))?;
        paths.push(link);
    }
    let ins = Inputs { paths };
    for n in &mut flow.nodes {
        n.fields.values_mut().for_each(|v| ins.json(v));
    }
    let uid = wire::peer(sock).map(|p| p.uid).unwrap_or(u32::MAX);
    let cache = Cache { uid };
    let cancel = sf::Cancel::new();
    let opts = RunOpts {
        cancel: cancel.clone(),
        inputs: inputs.iter().map(|i| ((i.node, i.port.clone()), ins.val(&i.value))).collect(),
        sandbox: Some(Sandbox { roots: vec![dir.clone()] }),
        cache: Some(&cache),
    };
    let mut sink = Sink { sock, id, cancel, next_out: 0 };
    let t0 = Instant::now();
    let r = sf::run_with(&flow, &dir, &opts, &mut |e| {
        let e = match e {
            Event::Finished { id, name, summary, outputs, cached } => Event::Finished { id, name, summary, outputs: sink.outputs(&outputs), cached },
            other => other,
        };
        sink.send(synaptixd::Event::Flow { node: e }, &[]);
    });
    let _ = std::fs::remove_dir_all(&dir);
    drop(fds);
    match r {
        Ok(rep) => {
            tracing::info!("граф: {} нод за {:.1} с", flow.nodes.len(), t0.elapsed().as_secs_f32());
            Ok((serde_json::to_value(FlowReply { steps: rep.steps }).unwrap_or(Value::Null), Vec::new()))
        }
        Err(e) => {
            tracing::warn!("граф: {e}");
            Err(Error::new(ErrorCode::Failed, e))
        }
    }
}

// ── Ход и отмена долгих вызовов ───────────────────────────────────────────

/// Выполнить `f` в отдельном потоке; ход (`progress(шаг, всего)` из любых
/// потоков) и отмена прогона доходят до него через атомики.
pub fn with_progress<T: Send>(cx: &mut Cx, f: impl FnOnce(&(dyn Fn(usize, usize) + Sync), &Arc<AtomicBool>) -> T + Send) -> T {
    let stop = Arc::new(AtomicBool::new(cx.cancel.is_cancelled()));
    let _g = {
        let s = stop.clone();
        cx.cancel.on_cancel(move || s.store(true, Ordering::SeqCst))
    };
    let (step, total, done) = (AtomicUsize::new(0), AtomicUsize::new(0), AtomicBool::new(false));
    std::thread::scope(|scope| {
        let h = scope.spawn(|| {
            let progress = |s: usize, t: usize| {
                step.store(s, Ordering::Relaxed);
                total.store(t, Ordering::Relaxed);
            };
            let r = f(&progress, &stop);
            done.store(true, Ordering::SeqCst);
            r
        });
        let mut last = (0, 0);
        while !done.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(100));
            let now = (step.load(Ordering::Relaxed), total.load(Ordering::Relaxed));
            if now != last && now.1 > 0 {
                cx.progress(now.0, now.1);
                last = now;
            }
        }
        h.join().unwrap_or_else(|p| std::panic::resume_unwind(p))
    })
}

fn stopped(cx: &Cx, e: impl ToString) -> String {
    if cx.cancel.is_cancelled() {
        "прогон остановлен".into()
    } else {
        e.to_string()
    }
}

// ── Ноды генерации ────────────────────────────────────────────────────────

fn llm(cx: &mut Cx) -> R {
    let mut msgs = Vec::new();
    if let Some(sys) = cx.str("system") {
        msgs.push(synaptixd::Message::system(sys));
    }
    msgs.push(synaptixd::Message::user(cx.need("prompt")?));
    let mut call = Params::new().with("thinking", false);
    if let Some(n) = cx.pos("max_tokens") {
        call = call.with("max_tokens", n as i64);
    }
    let m = mgr();
    let model = m.resolve_id(&cx.str("model").unwrap_or_default(), ModelKind::Llm).map_err(|e| e.message)?;
    let params = m.params(&model, &call);
    let g = with_progress(cx, |_, _| m.run(&model, |l| l.generate(&msgs, &params, &[], &|_| {}))).map_err(|e| e.message)?;
    cx.label = format!("{} токенов", g.tokens);
    cx.out("text", Val::Text(g.text.trim().to_string()));
    Ok(())
}

fn image(cx: &mut Cx) -> R {
    let mut images = cx.files("images");
    images.retain(|s| !s.is_empty());
    let p = synaptixd::ImageParams {
        negative: cx.str("negative"),
        width: cx.pos("width").map(|v| v as usize),
        height: cx.pos("height").map(|v| v as usize),
        steps: cx.pos("steps").map(|v| v as usize),
        guidance: cx.pos("guidance").map(|v| v as f32),
        seed: cx.pos("seed").map(|v| v as u64),
        images,
        init: cx.file("init"),
        strength: cx.pos("strength").map(|v| v as f32),
    };
    let prompt = cx.need("prompt")?;
    let model = cx.str("model").unwrap_or_default();
    let r = with_progress(cx, |progress, stop| gen::image(mgr(), &model, prompt, p, &mut |i, n| {
        progress(i, n);
        !stop.load(Ordering::SeqCst)
    }))
    .map_err(|e| stopped(cx, e.message))?;
    let path = cx.out_path("png");
    std::fs::write(&path, &r.png).map_err(|e| e.to_string())?;
    cx.label = r.label;
    cx.out("image", Val::File(path));
    Ok(())
}

/// Модель видео по нужным возможностям (как у инструмента media).
fn video_model(cx: &Cx, p: &synaptixd::VideoParams) -> String {
    if let Some(m) = cx.str("model") {
        return m;
    }
    let restyle = cx.bool("restyle") && p.source_video.is_some();
    let ltx_only = !restyle && (p.audio.is_some() || p.source_video.is_some() || p.control_video.is_some() || p.two_stage);
    if ltx_only {
        "ltx-2-3-22b-distilled-1-1".into()
    } else if p.first_frame.is_some() || p.last_frame.is_some() || restyle {
        "minimax-h3-fl2va".into()
    } else {
        String::new()
    }
}

fn video(cx: &mut Cx) -> R {
    let p = synaptixd::VideoParams {
        negative: cx.str("negative"),
        width: cx.pos("width").map(|v| v as usize),
        height: cx.pos("height").map(|v| v as usize),
        seconds: cx.pos("seconds"),
        steps: cx.pos("steps").map(|v| v as usize),
        guidance: cx.pos("guidance").map(|v| v as f32),
        seed: cx.pos("seed").map(|v| v as u64),
        first_frame: cx.file("first_frame"),
        last_frame: cx.file("last_frame"),
        refs: cx.files("refs"),
        audio: cx.file("audio"),
        source_video: cx.file("source_video"),
        retake: match (cx.num("retake_start"), cx.pos("retake_end")) {
            (Some(a), Some(b)) if b > a => Some((a, b)),
            _ => None,
        },
        extend: cx.bool("extend"),
        control_video: cx.file("control_video"),
        control: cx.str("control"),
        two_stage: cx.bool("two_stage"),
        strength: cx.pos("strength").map(|v| v as f32),
    };
    let model = video_model(cx, &p);
    let prompt = cx.need("prompt")?;
    let r = with_progress(cx, |progress, stop| gen::video(mgr(), &model, prompt, p, stop, progress)).map_err(|e| stopped(cx, e.message))?;
    let path = cx.out_path("mp4");
    std::fs::write(&path, &r.mp4).map_err(|e| e.to_string())?;
    cx.label = r.out.label;
    cx.out("video", Val::File(path));
    Ok(())
}

fn music(cx: &mut Cx) -> R {
    let lyrics = cx.str("lyrics").unwrap_or_default();
    let prompt = cx.need("prompt")?;
    // Инструментал — ACE-Step (YuE2 — песенная модель).
    let model = cx.str("model").or_else(|| {
        if lyrics.is_empty() {
            mgr().list().into_iter().filter(|m| m.kind == ModelKind::Music && m.id.contains("acestep")).min_by_key(|m| !m.id.contains("turbo")).map(|m| m.id)
        } else {
            None
        }
    });
    let secs = cx.pos("seconds").map(|v| v as u32);
    let seed = cx.pos("seed").map(|v| v as u64);
    let r = with_progress(cx, |progress, stop| gen::music(mgr(), &model.unwrap_or_default(), prompt, lyrics, secs, seed, None, stop.clone(), &mut |i, n| progress(i, n)))
    .map_err(|e| stopped(cx, e.message))?;
    let path = cx.out_path("wav");
    std::fs::write(&path, &r.wav).map_err(|e| e.to_string())?;
    cx.label = r.out.label;
    cx.out("audio", Val::File(path));
    Ok(())
}

fn speak(cx: &mut Cx) -> R {
    let mut call = Params::new();
    if let Some(v) = cx.file("voice").or_else(|| cx.str("voice").filter(|v| !v.starts_with('/'))) {
        call = call.with("voice", v);
    }
    if let Some(l) = cx.str("language") {
        call = call.with("language", l);
    }
    let text = cx.need("text")?;
    let m = mgr();
    let model = m.resolve_id(&cx.str("model").unwrap_or_default(), ModelKind::Tts).map_err(|e| e.message)?;
    let params = m.params(&model, &call);
    let (pcm, rate, _) = with_progress(cx, |_, _| m.run(&model, |l| l.speak(&text, &params, false))).map_err(|e| stopped(cx, e.message))?;
    let path = cx.out_path("wav");
    std::fs::write(&path, sf::ops::media::wav_mono(&pcm, rate)).map_err(|e| e.to_string())?;
    cx.label = format!("речь {:.1} с", pcm.len() as f32 / rate.max(1) as f32);
    cx.out("audio", Val::File(path));
    Ok(())
}

fn transcribe(cx: &mut Cx) -> R {
    let src = cx.need_file("file")?;
    let pcm = sf::ops::media::decode_pcm(&src, 16000)?;
    let m = mgr();
    let model = m.resolve_id("", ModelKind::Asr).map_err(|e| e.message)?;
    let mut call = Params::new();
    if let Some(l) = cx.str("language") {
        call = call.with("language", l);
    }
    let language = m.params(&model, &call).text("language").map(String::from);
    let t = with_progress(cx, |_, _| m.run(&model, |l| l.transcribe(&pcm, 16000, language.as_deref()))).map_err(|e| stopped(cx, e.message))?;
    cx.out("text", Val::Text(t.text));
    Ok(())
}
