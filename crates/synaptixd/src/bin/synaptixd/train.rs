//! Обучение адаптеров LoRA в службе: очередь задач (по одной на карте),
//! ход — событиями `train` подписчикам, история — в
//! `/var/lib/synaptix/train_jobs.json`.
//!
//! Задача живёт в службе, а не в соединении: программа может закрыться,
//! обучение продолжится. Файлы (датасет, база, адаптер) читаются и пишутся
//! с правами программы, поставившей задачу. Карта берётся на один шаг
//! оптимизатора: между шагами её получают генерации других программ.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Instant;

use synaptix_core::dtype::DType;
use synaptixd::{AppInfo, DatasetCheck, DeviceKind, Error, ErrorCode, Event, ModelKind, TrainJob, TrainOptions, TrainPoint, TrainState};

use crate::lora;
use crate::manager::{self, now_ms, Manager};

struct Job {
    job: TrainJob,
    cancel: Arc<AtomicBool>,
    uid: u32,
    gid: u32,
    model_path: PathBuf,
}

struct Queue {
    mgr: Arc<Manager>,
    jobs: Mutex<Vec<Job>>,
    wake: Condvar,
    file: PathBuf,
    next: AtomicU64,
}

static Q: OnceLock<Queue> = OnceLock::new();

/// Сколько завершённых задач держать в истории.
const HISTORY: usize = 50;

fn q() -> &'static Queue {
    Q.get().expect("train::init не вызван")
}

pub fn init(mgr: Arc<Manager>) {
    let file = mgr.cfg.state.parent().map(|d| d.join("train_jobs.json")).unwrap_or_else(|| PathBuf::from("/var/lib/synaptix/train_jobs.json"));
    let mut old: Vec<TrainJob> = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    for j in old.iter_mut().filter(|j| !j.state.finished()) {
        j.state = TrainState::Failed;
        j.error = Some("служба перезапущена во время обучения".into());
        j.finished_ms.get_or_insert(now_ms());
    }
    let next = old.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let jobs = old.into_iter().map(|job| Job { job, cancel: Arc::new(AtomicBool::new(false)), uid: 0, gid: 0, model_path: PathBuf::new() }).collect();
    let _ = Q.set(Queue { mgr, jobs: Mutex::new(jobs), wake: Condvar::new(), file, next: AtomicU64::new(next) });
    let _ = std::thread::Builder::new().name("train".into()).spawn(worker);
}

fn save(jobs: &[Job]) {
    let list: Vec<&TrainJob> = jobs.iter().map(|j| &j.job).collect();
    if let Ok(b) = serde_json::to_vec(&list) {
        let tmp = q().file.with_extension("json.tmp");
        if std::fs::write(&tmp, b).is_ok() {
            let _ = std::fs::rename(&tmp, &q().file);
        }
    }
}

fn publish(job: &TrainJob) {
    q().mgr.notify(Event::Train { job: job.clone() });
}

fn update(id: u64, f: impl FnOnce(&mut TrainJob), persist: bool) {
    let snapshot = {
        let mut jobs = q().jobs.lock().unwrap();
        let Some(j) = jobs.iter_mut().find(|j| j.job.id == id) else { return };
        f(&mut j.job);
        let s = j.job.clone();
        if persist {
            save(&jobs);
        }
        s
    };
    publish(&snapshot);
}

pub fn jobs() -> Vec<TrainJob> {
    q().jobs.lock().unwrap().iter().map(|j| j.job.clone()).collect()
}

/// Параметры движка из параметров задачи.
fn engine_options(o: &TrainOptions, out: &Path, resume: Option<PathBuf>) -> Result<synaptix::facade::train::LoraTrainOptions, Error> {
    use synaptix::facade::train::{LoraConfig, LoraTrainOptions, TrainConfig};
    let quant = match o.quant.as_deref().unwrap_or("nvfp4").to_ascii_lowercase().as_str() {
        "nvfp4" | "fp4" | "auto" => DType::NVFP4,
        "mxfp8" | "fp8" => DType::MXFP8,
        "bf16" | "none" => DType::BF16,
        q if q.starts_with("sq") => DType::Sq { bits: q[2..].parse().map_err(|_| Error::new(ErrorCode::Protocol, format!("формат базы {q}")))? },
        q => return Err(Error::new(ErrorCode::Protocol, format!("формат базы {q}: nvfp4 | mxfp8 | sqN | bf16"))),
    };
    let r = o.lora_r.unwrap_or(16).clamp(1, 64);
    let targets: Vec<String> = match o.targets.as_deref().unwrap_or("all") {
        "all" => LoraConfig::default().targets,
        "attn" => ["q_proj", "k_proj", "v_proj", "o_proj", "in_proj_qkv", "in_proj_z", "out_proj"].iter().map(|s| s.to_string()).collect(),
        t => t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
    };
    let d = TrainConfig::default();
    Ok(LoraTrainOptions {
        base_quant: quant,
        lora: LoraConfig { r, alpha: o.lora_alpha.unwrap_or(2.0 * r as f32), targets, seed: o.seed.unwrap_or(0), ..LoraConfig::default() },
        train: TrainConfig {
            lr: o.lr.unwrap_or(d.lr),
            epochs: o.epochs.unwrap_or(1).max(1),
            max_len: o.max_len.unwrap_or(d.max_len).clamp(128, 32768),
            grad_accum: o.grad_accum.unwrap_or(d.grad_accum).max(1),
            warmup_ratio: o.warmup.unwrap_or(d.warmup_ratio),
            seed: o.seed.unwrap_or(0),
            out_dir: out.to_path_buf(),
            save_every: o.save_every.unwrap_or(0),
            eval_every: o.eval_every.unwrap_or(0),
            ..d
        },
        thinking: o.thinking,
        eval_fraction: o.eval_fraction.unwrap_or(0.02),
        max_samples: o.max_samples,
        init_adapter: resume,
        ..LoraTrainOptions::default()
    })
}

/// Поставить задачу в очередь.
pub fn start(model: &str, dataset: &str, name: &str, options: TrainOptions, uid: u32, gid: u32, app: AppInfo) -> Result<TrainJob, Error> {
    let mgr = &q().mgr;
    if !lora::valid_name(name) {
        return Err(Error::new(ErrorCode::Protocol, format!("имя адаптера «{name}»: без «/», не с точки и не с «_»")));
    }
    let id = mgr.resolve_id(model, ModelKind::Llm)?;
    let info = mgr.model_file(&id).ok_or_else(|| Error::new(ErrorCode::NoModel, format!("нет модели {id}")))?;
    let lib = info.library.clone().or_else(|| mgr.default_library()).ok_or_else(|| Error::new(ErrorCode::Failed, "нет библиотеки моделей для адаптера"))?;
    let out = lib.join(lora::LORA_DIR).join(name);
    let busy = q().jobs.lock().unwrap().iter().any(|j| !j.job.state.finished() && j.job.out == out.to_string_lossy());
    if busy {
        return Err(Error::new(ErrorCode::Failed, format!("адаптер «{name}» уже обучается")));
    }
    // Проверка параметров сразу, а не через полчаса очереди.
    engine_options(&options, &out, None)?;
    let job = TrainJob {
        id: q().next.fetch_add(1, Ordering::Relaxed),
        model: id.clone(),
        model_title: info.title.clone(),
        dataset: dataset.to_string(),
        name: name.to_string(),
        out: out.to_string_lossy().into_owned(),
        options,
        state: TrainState::Queued,
        created_ms: now_ms(),
        app,
        ..Default::default()
    };
    {
        let mut jobs = q().jobs.lock().unwrap();
        jobs.push(Job { job: job.clone(), cancel: Arc::new(AtomicBool::new(false)), uid, gid, model_path: info.path.clone() });
        save(&jobs);
    }
    q().wake.notify_all();
    publish(&job);
    Ok(job)
}

pub fn stop(id: u64, uid: u32, admin: bool) -> Result<(), Error> {
    let snapshot = {
        let mut jobs = q().jobs.lock().unwrap();
        let j = jobs.iter_mut().find(|j| j.job.id == id).ok_or_else(|| Error::new(ErrorCode::Failed, format!("нет задачи {id}")))?;
        if !admin && j.uid != uid {
            return Err(Error::new(ErrorCode::Denied, "чужую задачу останавливают root и группа wheel"));
        }
        if j.job.state.finished() {
            return Ok(());
        }
        j.cancel.store(true, Ordering::Relaxed);
        if j.job.state == TrainState::Queued {
            j.job.state = TrainState::Cancelled;
            j.job.finished_ms = Some(now_ms());
        }
        let s = j.job.clone();
        save(&jobs);
        s
    };
    publish(&snapshot);
    Ok(())
}

pub fn forget(id: u64, uid: u32, admin: bool) -> Result<(), Error> {
    let mut jobs = q().jobs.lock().unwrap();
    if let Some(i) = jobs.iter().position(|j| j.job.id == id) {
        if !admin && jobs[i].uid != uid && jobs[i].uid != 0 {
            return Err(Error::new(ErrorCode::Denied, "чужую задачу убирают root и группа wheel"));
        }
        if !jobs[i].job.state.finished() {
            return Err(Error::new(ErrorCode::Failed, "задача ещё идёт — сначала остановите"));
        }
        jobs.remove(i);
        save(&jobs);
    }
    Ok(())
}

/// Проверка датасета токенизатором модели (с правами программы).
pub fn check_dataset(mgr: &Manager, model: &str, path: &str, o: &TrainOptions, uid: u32, gid: u32) -> Result<DatasetCheck, Error> {
    let id = mgr.resolve_id(model, ModelKind::Llm)?;
    let info = mgr.model_file(&id).ok_or_else(|| Error::new(ErrorCode::NoModel, format!("нет модели {id}")))?;
    let max_len = o.max_len.unwrap_or(2048);
    crate::as_peer(uid, gid, || {
        let tok = synaptix::facade::llm::load_llm_tokenizer(&info.path).map_err(|e| Error::new(ErrorCode::Failed, e.to_string()))?;
        let (_, r) = synaptix::facade::train::load_samples(Path::new(path), &tok, o.thinking, max_len, o.max_samples)
            .map_err(|e| Error::new(ErrorCode::Failed, e.to_string()))?;
        Ok(DatasetCheck { samples: r.samples, skipped: r.skipped, truncated: r.truncated, tokens: r.tokens, target_tokens: r.target_tokens, last_turn_only: r.last_turn_only })
    })
}

fn worker() {
    loop {
        let (id, cancel, uid, gid, model_path, job) = {
            let mut jobs = q().jobs.lock().unwrap();
            loop {
                if let Some(j) = jobs.iter_mut().find(|j| j.job.state == TrainState::Queued) {
                    j.job.state = TrainState::Preparing;
                    j.job.started_ms = Some(now_ms());
                    break (j.job.id, j.cancel.clone(), j.uid, j.gid, j.model_path.clone(), j.job.clone());
                }
                jobs = q().wake.wait(jobs).unwrap();
            }
        };
        publish(&job);
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(id, &job, &model_path, cancel.clone(), uid, gid)))
            .unwrap_or_else(|_| Err("сбой обучения".into()));
        manager::release_vram(Some(DeviceKind::Cuda));
        update(
            id,
            |j| {
                j.finished_ms = Some(now_ms());
                match res {
                    Ok(()) if cancel.load(Ordering::Relaxed) => j.state = TrainState::Cancelled,
                    Ok(()) => j.state = TrainState::Done,
                    Err(e) => {
                        j.state = TrainState::Failed;
                        j.error = Some(e);
                    }
                }
            },
            true,
        );
        trim_history();
        q().mgr.notify(Event::LorasChanged);
    }
}

fn trim_history() {
    let mut jobs = q().jobs.lock().unwrap();
    let done: Vec<u64> = jobs.iter().filter(|j| j.job.state.finished()).map(|j| j.job.id).collect();
    if done.len() > HISTORY {
        let drop_ids: Vec<u64> = done[..done.len() - HISTORY].to_vec();
        jobs.retain(|j| !drop_ids.contains(&j.job.id));
        save(&jobs);
    }
}

fn run(id: u64, job: &TrainJob, model_path: &Path, cancel: Arc<AtomicBool>, uid: u32, gid: u32) -> Result<(), String> {
    let mgr = &q().mgr;
    let out = PathBuf::from(&job.out);
    let resume = match &job.options.resume {
        Some(n) => Some(PathBuf::from(lora::find_lora(&mgr.library_paths(), n).ok_or_else(|| format!("адаптер «{n}» для продолжения не найден"))?.path)),
        None => None,
    };
    let opts = engine_options(&job.options, &out, resume).map_err(|e| e.message)?;
    // Видеопамять: база с запасом на копии и активации слоя.
    let file = std::fs::metadata(model_path).map(|m| m.len()).unwrap_or(0);
    let need = file + file / 8 + (3u64 << 30) + (opts.train.max_len as u64 / 2048) * (2u64 << 30);
    mgr.make_vram_room("train", DeviceKind::Cuda, need);

    let hooks = synaptix::facade::train::TrainHooks {
        step_guard: Some(Arc::new(|| Box::new(crate::devices::lock(DeviceKind::Cuda)) as Box<dyn std::any::Any>)),
        cancel: Some(cancel),
    };
    let t0 = Instant::now();
    let mut last_pub = Instant::now();
    let dataset = PathBuf::from(&job.dataset);
    let report = crate::as_peer(uid, gid, || {
        synaptix::facade::train::train_lora(
            model_path,
            &dataset,
            opts.clone(),
            hooks,
            |d, n_train, n_eval| {
                update(
                    id,
                    |j| {
                        j.state = TrainState::Running;
                        j.dataset_check = Some(DatasetCheck {
                            samples: d.samples,
                            skipped: d.skipped,
                            truncated: d.truncated,
                            tokens: d.tokens,
                            target_tokens: d.target_tokens,
                            last_turn_only: d.last_turn_only,
                        });
                        j.train_samples = n_train;
                        j.eval_samples = n_eval;
                    },
                    true,
                );
            },
            |p| {
                let first = p.step == 1;
                let snapshot = {
                    let mut jobs = q().jobs.lock().unwrap();
                    jobs.iter_mut().find(|j| j.job.id == id).map(|j| {
                        let t = &mut j.job;
                        t.state = TrainState::Running;
                        t.step = p.step;
                        t.total_steps = p.total_steps;
                        t.epoch = p.epoch;
                        t.loss = p.loss;
                        if p.eval_loss.is_some() {
                            t.eval_loss = p.eval_loss;
                        }
                        t.lr = p.lr;
                        t.grad_norm = p.grad_norm;
                        t.tokens_per_sec = p.tokens_per_sec;
                        t.eta_secs = p.eta_secs;
                        t.elapsed_secs = p.elapsed_secs;
                        t.history.push(TrainPoint { step: p.step, loss: p.loss, eval_loss: p.eval_loss });
                        // Длинная кривая — прореживается вдвое (первая и последние точки остаются).
                        if t.history.len() > 2000 {
                            let h = std::mem::take(&mut t.history);
                            t.history = h.into_iter().enumerate().filter(|(i, pt)| i % 2 == 0 || pt.eval_loss.is_some()).map(|(_, pt)| pt).collect();
                        }
                        t.clone()
                    })
                };
                if let Some(s) = snapshot {
                    if first || p.eval_loss.is_some() || p.step == p.total_steps || last_pub.elapsed().as_millis() >= 700 {
                        last_pub = Instant::now();
                        publish(&s);
                    }
                    if p.step % 20 == 0 {
                        save(&q().jobs.lock().unwrap());
                    }
                }
                true
            },
        )
    })
    .map_err(|e| e.0)?;
    // Сведения обучения рядом с адаптером (для списка адаптеров и UI).
    let snap = jobs().into_iter().find(|j| j.id == id).unwrap_or_default();
    let meta = serde_json::json!({
        "base": job.model,
        "base_title": job.model_title,
        "base_path": model_path.to_string_lossy(),
        "dataset": job.dataset,
        "options": job.options,
        "steps": snap.step,
        "total_steps": snap.total_steps,
        "loss": snap.loss,
        "eval_loss": snap.eval_loss,
        "train_samples": report.dataset.samples.saturating_sub(report.eval_samples),
        "eval_samples": report.eval_samples,
        "target_tokens": report.dataset.target_tokens,
        "trainable_params": report.trainable_params,
        "seconds": t0.elapsed().as_secs_f32(),
        "finished_ms": now_ms(),
        "cancelled": snap.step < snap.total_steps,
    });
    crate::as_peer(uid, gid, || std::fs::write(out.join(lora::TRAIN_META), serde_json::to_vec_pretty(&meta).unwrap_or_default()))
        .map_err(|e| format!("{}: {e}", out.display()))?;
    update(id, |j| j.trainable_params = report.trainable_params as u64, true);
    Ok(())
}
