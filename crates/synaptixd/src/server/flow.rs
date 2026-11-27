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
use crate::{wire, Error, ErrorCode, FlowInput, FlowReply, ModelKind, Params, Reply};

use crate::server::manager::Manager;
use crate::server::{gen, stages};

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
        sf::ops::provide("omnivoice", omnivoice);
        sf::ops::provide("voxcpm", voxcpm);
        sf::ops::provide("vibevoice", vibevoice);
        sf::ops::provide("diarize", diarize);
        #[cfg(feature = "cuda")]
        {
            sf::ops::provide("vfx_upscale", super::vfx::upscale);
            sf::ops::provide("vfx_interpolate", super::vfx::interpolate);
        }
        stages::install();
        let _ = std::thread::Builder::new().name("flow-cache".into()).spawn(|| loop {
            std::thread::sleep(Duration::from_secs(60));
            sweep(TTL);
            stages::release_idle_ltx();
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
        crate::server::manager::release_vram(Some(crate::DeviceKind::Cuda));
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
        crate::flow::marker_index(w, "@in:").and_then(|i| self.paths.get(i)).map(|p| p.display().to_string())
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
    /// Входы клиента: на выходе (нода «Файл») — обратно меткой, без копии.
    inputs: Vec<PathBuf>,
    /// Уже отправленные файлы (элементы пакета): повторно — той же меткой.
    sent: std::collections::HashMap<PathBuf, String>,
}

impl Sink<'_> {
    fn send(&self, ev: crate::Event, fds: &[OwnedFd]) {
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
                let v = v.map_files(&mut |p| {
                    if let Some(i) = self.inputs.iter().position(|x| x == p) {
                        return PathBuf::from(crate::flow::in_marker(i));
                    }
                    if let Some(m) = self.sent.get(p) {
                        return PathBuf::from(m);
                    }
                    match std::fs::File::open(p) {
                    Ok(f) => {
                        let m = crate::flow::out_marker(self.next_out);
                        self.next_out += 1;
                        self.sent.insert(p.to_path_buf(), m.clone());
                        files.push((OwnedFd::from(f), p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                        PathBuf::from(m)
                    }
                    Err(_) => p.to_path_buf(),
                    }
                });
                (k.clone(), v)
            })
            .collect();
        for chunk in files.chunks_mut(wire::MAX_FDS) {
            let names: Vec<String> = chunk.iter().map(|(_, n)| n.clone()).collect();
            let fds: Vec<OwnedFd> = chunk.iter().filter_map(|(f, _)| f.try_clone().ok()).collect();
            self.send(crate::Event::FlowFiles { first, names }, &fds);
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
    let _watch = {
        let c = cancel.clone();
        crate::server::hangup::watch(sock.as_raw_fd(), move || c.cancel())
    };
    let live = crate::server::activity::current();
    if let Some(l) = &live {
        let c = cancel.clone();
        l.on_cancel(move || c.cancel());
    }
    let mut sink = Sink { sock, id, cancel, next_out: 0, inputs: ins.paths.clone(), sent: Default::default() };
    let t0 = Instant::now();
    let r = sf::run_with(&flow, &dir, &opts, &mut |e| {
        let e = match e {
            Event::Finished { id, name, summary, outputs, cached } => Event::Finished { id, name, summary, outputs: sink.outputs(&outputs), cached },
            Event::Item { id, name, index, total, outputs } => Event::Item { id, name, index, total, outputs: sink.outputs(&outputs) },
            other => other,
        };
        if let (Some(l), Event::Progress { step, total, .. }) = (&live, &e) {
            l.progress(*step, *total);
        }
        let shot = match (&live, &e) {
            (Some(l), Event::Progress { id, .. }) => previews().lock().unwrap_or_else(|e| e.into_inner()).remove(&l.id).map(|p| (*id, p)),
            _ => None,
        };
        sink.send(crate::Event::Flow { node: e }, &[]);
        if let Some((id, (path, step, total))) = shot {
            let o: sf::Outputs = [("image".to_string(), Val::File(path.clone()))].into_iter().collect();
            let outputs = sink.outputs(&o);
            sink.send(crate::Event::Flow { node: Event::Preview { id, step, total, outputs } }, &[]);
            let _ = std::fs::remove_file(path);
        }
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
    let live = crate::server::activity::current();
    let live_id = live.as_ref().map(|l| l.id);
    let mut shot = 0usize;
    let _ = synaptix_core::gen_status::take_preview();
    std::thread::scope(|scope| {
        let h = scope.spawn(|| {
            let _work = crate::server::activity::adopt(live);
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
            // Превью генерации (модель кладёт его в gen_status): файл PNG —
            // рядом с выходами ноды, клиенту — событием Preview вслед за шагом.
            if let (Some(p), Some(lid)) = (synaptix_core::gen_status::take_preview(), live_id) {
                shot += 1;
                let path = cx.out_path(&format!("preview-{shot:03}.png"));
                if write_preview(&p, &path).is_ok() {
                    previews().lock().unwrap_or_else(|e| e.into_inner()).insert(lid, (path, p.step + 1, p.total));
                    cx.progress(last.0.max(1), last.1.max(p.total).max(1));
                }
            }
        }
        h.join().unwrap_or_else(|p| std::panic::resume_unwind(p))
    })
}

/// Готовые превью по работам соединений: (файл, шаг, всего).
fn previews() -> &'static std::sync::Mutex<std::collections::HashMap<u64, (PathBuf, usize, usize)>> {
    static P: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<u64, (PathBuf, usize, usize)>>> = std::sync::OnceLock::new();
    P.get_or_init(Default::default)
}

fn write_preview(p: &synaptix_core::gen_status::Preview, path: &Path) -> Result<(), String> {
    let (h, w) = (p.height, p.width);
    let mut chw = vec![0f32; 3 * h * w];
    for i in 0..h * w {
        for c in 0..3 {
            chw[c * h * w + i] = p.rgb[i * 3 + c] as f32 / 255.0;
        }
    }
    let t = synaptix_core::tensor::Tensor::from_vec(chw, vec![3, h, w], synaptix_core::device::Device::Cpu).map_err(|e| e.to_string())?;
    synaptix_gen::image::save_png(&t, path)
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
        msgs.push(crate::Message::system(sys));
    }
    msgs.push(crate::Message::user(cx.need("prompt")?));
    let mut call = Params::new().with("thinking", cx.bool("think"));
    for k in ["max_tokens", "context"] {
        if let Some(n) = cx.pos(k) {
            call = call.with(k, n as i64);
        }
    }
    // Отрицательное — умолчание модели.
    for k in ["temperature", "top_p", "min_p", "repeat_penalty"] {
        if let Some(v) = cx.num(k).filter(|v| *v >= 0.0) {
            call = call.with(k, v);
        }
    }
    if let Some(v) = cx.num("top_k").filter(|v| *v >= 0.0) {
        call = call.with("top_k", v as i64);
    }
    match cx.num("seed").filter(|v| *v >= 0.0) {
        Some(v) => call = call.with("seed", v as i64),
        None => cx.uncached(),
    }
    let m = mgr();
    let model = m.resolve_id(&cx.str("model").unwrap_or_default(), ModelKind::Llm).map_err(|e| e.message)?;
    let params = m.params(&model, &call);
    let g = with_progress(cx, |_, stop| m.run(&model, |l| l.generate(&msgs, &params, &[], &|_| {}, stop))).map_err(|e| stopped(cx, e.message))?;
    cx.label = format!("{} токенов", g.tokens);
    cx.out("text", Val::Text(g.text.trim().to_string()));
    if let Some(t) = Some(g.thinking.trim()).filter(|t| !t.is_empty()) {
        cx.out("thinking", Val::Text(t.to_string()));
    }
    Ok(())
}

fn image(cx: &mut Cx) -> R {
    let mut images = cx.files("images");
    images.retain(|s| !s.is_empty());
    let p = crate::ImageParams {
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

/// Дескриптор латента хвоста ролика (передаётся между `video.tail` → `video.context`,
/// а также в стадийные ноды `h3_encode`/`h3_sample` и `ltx_stage1`/`ltx_stage2`).
/// Держит путь к `.safetensors` в каталоге прогона и копию CPU-тензоров в памяти
/// (несколько МБ) на случай, если стадия взята из кэша после удаления старого каталога.
pub struct VideoTailHandle {
    pub path: PathBuf,
    pub tail: Arc<synaptix_gen::tail::TailLatent>,
}

impl VideoTailHandle {
    pub fn from_file(path: PathBuf) -> Result<Self, String> {
        let tail = synaptix_gen::tail::TailLatent::load(&path)?;
        Ok(Self { path, tail: Arc::new(tail) })
    }

    /// Гарантировать наличие `.safetensors` на диске (если путь из кэша удалён — записать в `fallback`).
    pub fn ensure_file(&self, fallback: PathBuf) -> Result<PathBuf, String> {
        if self.path.is_file() {
            return Ok(self.path.clone());
        }
        self.tail.save(&fallback)?;
        Ok(fallback)
    }
}

/// Прочитать вход `context` (`Val::Handle(VideoTailHandle)` или путь к `.safetensors`).
pub fn read_context_tail(cx: &Cx) -> Result<Option<(PathBuf, Arc<synaptix_gen::tail::TailLatent>)>, String> {
    if let Some(h) = cx.opt_handle::<VideoTailHandle>("context")? {
        let fallback = cx.scratch().join("context.safetensors");
        let path = h.ensure_file(fallback)?;
        return Ok(Some((path, h.tail.clone())));
    }
    if let Some(p) = cx.file("context") {
        let pb = PathBuf::from(p);
        let tail = synaptix_gen::tail::TailLatent::load(&pb)?;
        return Ok(Some((pb, Arc::new(tail))));
    }
    Ok(None)
}

fn is_ltx(m: &crate::ModelEntry) -> bool {
    m.arch.to_lowercase().starts_with("ltx")
}

/// H3 нужного варианта (`ref2va` — по образцам, `fl2va` — по кадрам).
fn is_h3(m: &crate::ModelEntry, variant: &str) -> bool {
    m.arch.to_lowercase().contains("minimax_h3") && m.id.to_lowercase().contains(variant)
}

/// Модель видео с нужным свойством: выбранная по умолчанию, если подходит,
/// иначе из библиотеки (дистиллированная раньше полной).
pub(crate) fn video_fit(fit: &dyn Fn(&crate::ModelEntry) -> bool, what: &str) -> Result<String, String> {
    mgr()
        .resolve_fit(ModelKind::Video, fit, &|m| !m.id.contains("distilled") as u8)
        .ok_or_else(|| format!("в библиотеке нет модели видео: {what}"))
}

fn has_ref2va() -> bool {
    mgr().list().iter().any(|m| m.kind == ModelKind::Video && is_h3(m, "ref2va"))
}

/// Модель видео по нужным возможностям (как у инструмента media).
fn video_model(cx: &Cx, p: &crate::VideoParams, ctx_tail: Option<&synaptix_gen::tail::TailLatent>) -> Result<String, String> {
    if let Some(tail) = ctx_tail {
        if let Some(explicit) = cx.str("model").filter(|m| !matches!(m.trim(), "" | "auto" | "default")) {
            let resolved = mgr().resolve_id(&explicit, ModelKind::Video).map_err(|e| e.message)?;
            let want_fam = if resolved.contains("ltx") { "ltx" } else { "h3" };
            tail.meta.check_compat(want_fam, Some(&resolved), p.width, p.height, None)?;
            return Ok(resolved);
        }
        if !tail.meta.model.is_empty() {
            return Ok(tail.meta.model.clone());
        }
        return match tail.meta.family.as_str() {
            "ltx" => video_fit(&is_ltx, "LTX (продолжение сцены)"),
            _ if !p.refs.is_empty() => video_fit(&|m| is_h3(m, "ref2va"), "MiniMax-H3 по образцам"),
            _ => video_fit(&|m| is_h3(m, "fl2va"), "MiniMax-H3 по кадрам"),
        };
    }
    if let Some(m) = cx.str("model").filter(|m| !matches!(m.trim(), "" | "auto" | "default")) {
        return Ok(m);
    }
    let restyle = cx.bool("restyle") && p.source_video.is_some();
    let ltx_only = !restyle && (p.audio.is_some() || p.source_video.is_some() || p.control_video.is_some() || p.two_stage);
    if ltx_only {
        video_fit(&is_ltx, "LTX (звук, исходное или управляющее видео, две стадии)")
    } else if !p.refs.is_empty() && (has_ref2va() || p.first_frame.is_none()) {
        video_fit(&|m| is_h3(m, "ref2va"), "MiniMax-H3 по образцам")
    } else if p.first_frame.is_some() || p.last_frame.is_some() || restyle {
        video_fit(&|m| is_h3(m, "fl2va"), "MiniMax-H3 по кадрам")
    } else {
        // Без особых требований — модель видео из настроек по умолчанию.
        Ok(String::new())
    }
}

/// Короткое описание → подробная аудиовизуальная подпись в стиле обучающих
/// подписей LTX-2.5 (улучшитель промпта из комплекта модели).
fn enhance_prompt(cx: &mut Cx, prompt: &str) -> Result<String, String> {
    use synaptix_gen::ltx_text as lt;
    let m = mgr();
    // Улучшитель — модель из комплекта LTX: ищется по назначению, не по имени файла.
    let model = m
        .list()
        .into_iter()
        .filter(|e| e.kind == ModelKind::Llm && e.id.contains("prompt-enhancer"))
        .map(|e| e.id)
        .min()
        .ok_or_else(|| "улучшитель промпта LTX не найден в библиотеке".to_string())?;
    let msgs = vec![crate::Message::system(lt::ENHANCER_T2V_SYSTEM), crate::Message::user(lt::enhancer_user_message(prompt, false))];
    let call = Params::new().with("thinking", false).with("max_tokens", lt::ENHANCER_MAX_TOKENS as i64).with("temperature", 0.0).with("seed", 42i64);
    let params = m.params(&model, &call);
    let g = with_progress(cx, |_, stop| m.run(&model, |l| l.generate(&msgs, &params, &[], &|_| {}, stop))).map_err(|e| stopped(cx, e.message))?;
    let text = lt::enhancer_clean(&g.text);
    if text.split_whitespace().count() < 20 {
        return Err(format!("улучшитель промпта вернул слишком короткий текст: «{text}»"));
    }
    tracing::info!("улучшитель промпта: {} → {} слов", prompt.split_whitespace().count(), text.split_whitespace().count());
    Ok(text)
}

fn video(cx: &mut Cx) -> R {
    let ctx = read_context_tail(cx)?;
    let tail_out = cx.out_path("tail.safetensors");
    let mut p = crate::VideoParams {
        negative: cx.str("negative"),
        width: cx.pos("width").map(|v| v as usize),
        height: cx.pos("height").map(|v| v as usize),
        seconds: cx.pos("seconds"),
        steps: cx.pos("steps").map(|v| v as usize),
        guidance: cx.pos("guidance").map(|v| v as f32),
        seed: cx.pos("seed").map(|v| v as u64),
        // При продолжении с хвоста first_frame не используется.
        first_frame: if ctx.is_some() { None } else { cx.file("first_frame") },
        last_frame: cx.file("last_frame"),
        refs: cx.files("refs").into_iter().filter(|s| !s.is_empty()).collect(),
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
        fps_x2: cx.bool("fps_x2"),
        dfr: cx.bool("dfr"),
        diff_decoder: cx.bool("diff_decoder"),
        strength: cx.pos("strength").map(|v| v as f32),
        context: ctx.as_ref().map(|(p, _)| p.display().to_string()),
        context_frames: cx.pos("context_frames").map(|v| v as usize),
        tail_out: Some(tail_out.display().to_string()),
        // H3: LoRA узла (путь или «off»); не задана — Turbo 8 шагов по умолчанию
        lora: cx.file("lora").or_else(|| cx.str("lora")),
        lora_strength: cx.pos("lora_strength").map(|v| v as f32),
    };
    let model = video_model(cx, &p, ctx.as_ref().map(|(_, t)| t.as_ref()))?;
    // Универсальные графы (например story-video) подключают и refs (для Ref2VA),
    // и first_frame (запасной вариант для FL2VA / LTX): разводим их по выбранной модели.
    let resolved = mgr().resolve_id(&model, ModelKind::Video).unwrap_or_else(|_| model.clone());
    if resolved.contains("ref2va") {
        if !p.refs.is_empty() {
            p.first_frame = None;
            p.last_frame = None;
        }
    } else if !p.refs.is_empty() && (p.first_frame.is_some() || ctx.is_some()) {
        p.refs.clear();
    }
    let mut prompt = cx.need("prompt")?;
    // LTX-2.5: улучшитель промпта (Gemma 4 E2B, системный промпт эталона, жадно, 600 токенов).
    if cx.bool("enhance") {
        prompt = enhance_prompt(cx, &prompt)?;
    }
    let r = with_progress(cx, |progress, stop| gen::video(mgr(), &model, prompt, p, stop, progress)).map_err(|e| stopped(cx, e.message))?;
    let path = cx.out_path("mp4");
    std::fs::write(&path, &r.mp4).map_err(|e| e.to_string())?;
    cx.label = r.out.label;
    cx.out("video", Val::File(path));
    if let Some(tp) = r.out.tail.filter(|p| p.is_file()) {
        if let Ok(th) = VideoTailHandle::from_file(tp) {
            let lbl = th.tail.label();
            cx.out("tail", Val::Handle(sf::Handle::new(lbl, th)));
        }
    }
    Ok(())
}

fn music(cx: &mut Cx) -> R {
    let lyrics = cx.str("lyrics").unwrap_or_default();
    let prompt = cx.need("prompt")?;
    // Инструментал — ACE-Step (YuE2 — песенная модель).
    let model = cx.str("model").or_else(|| {
        if lyrics.is_empty() {
            mgr().resolve_fit(ModelKind::Music, &|m| m.arch.to_lowercase().contains("acestep"), &|m| !m.id.contains("turbo") as u8)
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
    // Файл голоса — образец для клона; слова — описание голоса.
    if let Some(f) = cx.file("voice").filter(|f| Path::new(f).is_file()) {
        call = call.with("ref_audio", f);
    } else if let Some(v) = cx.str("voice").filter(|v| !v.starts_with('/')) {
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

/// Сид поля; 0 — случайный (результат тогда не повторяется).
fn seed_field(cx: &mut Cx) -> u64 {
    match cx.pos("seed") {
        Some(s) => s as u64,
        None => {
            cx.uncached();
            gen::random_seed()
        }
    }
}

/// Синтез речи нодой с явными параметрами: модель поля, образцы со входов.
fn tts_node(cx: &mut Cx, text_port: &str, voice_ports: &[&str]) -> R {
    let seed = seed_field(cx);
    let req = crate::server::runners::TtsRequest {
        text: cx.need(text_port)?,
        voices: voice_ports.iter().filter_map(|p| cx.file(p)).map(PathBuf::from).collect(),
        ref_text: cx.str("ref_text"),
        prompt_audio: cx.file("prompt_audio").map(PathBuf::from),
        prompt_text: cx.str("prompt_text"),
        language: cx.str("language"),
        instruct: cx.str("instruct"),
        steps: cx.pos("steps").map(|v| v as usize),
        guidance: cx.pos("guidance").map(|v| v as f32),
        t_shift: cx.num("t_shift").map(|v| v as f32),
        speed: cx.pos("speed").map(|v| v as f32),
        max_len: cx.pos("max_len").map(|v| v as usize),
        max_length_times: cx.pos("max_length_times").map(|v| v as f32),
        seed,
    };
    let m = mgr();
    let model = m.resolve_id(&cx.str("model").unwrap_or_default(), ModelKind::Tts).map_err(|e| e.message)?;
    let (pcm, rate) = with_progress(cx, |progress, _| m.run(&model, |l| l.tts(&req, &mut |i, n| progress(i, n)))).map_err(|e| stopped(cx, e.message))?;
    let path = cx.out_path("wav");
    std::fs::write(&path, sf::ops::media::wav_mono(&pcm, rate)).map_err(|e| e.to_string())?;
    cx.label = format!("речь {:.1} с, сид {seed}", pcm.len() as f32 / rate.max(1) as f32);
    cx.out("audio", Val::File(path));
    Ok(())
}

fn omnivoice(cx: &mut Cx) -> R {
    tts_node(cx, "text", &["ref_audio"])
}

fn voxcpm(cx: &mut Cx) -> R {
    tts_node(cx, "text", &["ref_audio"])
}

fn vibevoice(cx: &mut Cx) -> R {
    // Голоса — по порядку говорящих, без пропусков.
    let ports = ["voice1", "voice2", "voice3", "voice4"];
    let n = ports.iter().take_while(|p| cx.file(p).is_some()).count();
    tts_node(cx, "script", &ports[..n])
}

fn diarize(cx: &mut Cx) -> R {
    let src = cx.need_file("audio")?;
    let pcm = sf::ops::media::decode_pcm(&src, 16000)?;
    let threshold = cx.num("threshold").unwrap_or(0.5) as f32;
    let overlap = cx.bool("overlap");
    let m = mgr();
    let model = m.resolve_id(&cx.str("model").unwrap_or_default(), ModelKind::Diarize).map_err(|e| e.message)?;
    let (pretty, json) = with_progress(cx, |_, _| m.run(&model, |l| l.diarize(&pcm, 16000, threshold, overlap))).map_err(|e| stopped(cx, e.message))?;
    cx.label = format!("{} строк", pretty.lines().count());
    cx.out("text", Val::Text(pretty));
    cx.out("json", Val::Text(json));
    Ok(())
}

fn transcribe(cx: &mut Cx) -> R {
    let src = cx.need_file("file")?;
    let pcm = sf::ops::media::decode_pcm(&src, 16000)?;
    let m = mgr();
    let model = m.resolve_id(&cx.str("model").unwrap_or_default(), ModelKind::Asr).map_err(|e| e.message)?;
    let mut call = Params::new();
    if let Some(l) = cx.str("language") {
        call = call.with("language", l);
    }
    let language = m.params(&model, &call).text("language").map(String::from);
    let t = with_progress(cx, |_, _| m.run(&model, |l| l.transcribe(&pcm, 16000, language.as_deref(), None))).map_err(|e| stopped(cx, e.message))?;
    cx.out("text", Val::Text(t.text));
    Ok(())
}
