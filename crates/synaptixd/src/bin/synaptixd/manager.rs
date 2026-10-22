use std::collections::{BTreeMap, HashMap};
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use synaptixd::{DefaultModel, DeviceKind, EngineProfile, Error, ErrorCode, Event, LoadState, ModelEntry, ModelInfo, ModelKind, ModelSettings, Params, SettingsChange, Status};

use crate::config::Config;
use crate::defaults;
use crate::devices;
use crate::library;
use crate::registry::{self, LibraryConf, Manifest, StateFile};
use crate::runners::{self, Loaded};

pub struct Slot {
    pub manifest: Manifest,
    pub settings: ModelSettings,
    pub state: LoadState,
    pub model: Option<Arc<Loaded>>,
    pub device: Option<DeviceKind>,
    pub precision: String,
    pub fallback: bool,
    pub mem_bytes: u64,
    pub vram_bytes: u64,
    pub clients: u32,
    pub busy: u32,
    pub last_used: Instant,
    pub last_used_ms: i64,
    pub error: Option<String>,
    pub disk_bytes: u64,
    pub schema: Vec<synaptixd::ParamSpec>,
    pub optimal: Params,
    /// Действующий профиль движка LLM по (устройство, точность, правки).
    engine_shown: Mutex<Option<(EngineKey, EngineProfile)>>,
    /// KV-кэш чатов, удержанный в RAM после выгрузки (см. [`KeptSlot`]).
    kept_kv: Option<KeptSlot>,
}

/// Удержанные KV-сессии выгруженной модели: до какого момента ждать загрузки
/// и на каком устройстве и с какой точностью они посчитаны — с другими их
/// не продолжить.
struct KeptSlot {
    kv: runners::KeptKv,
    until: Instant,
    device: Option<DeviceKind>,
    precision: String,
}

type EngineKey = (DeviceKind, Option<String>, EngineProfile);

impl Slot {
    fn new(manifest: Manifest, settings: ModelSettings) -> Self {
        let disk_bytes = manifest.disk_bytes();
        let schema = defaults::schema(&manifest);
        let optimal = defaults::optimal(&schema);
        let settings = ModelSettings { params: settings.params.sanitize(&schema), ..settings };
        Self {
            manifest,
            settings,
            state: LoadState::Unloaded,
            model: None,
            device: None,
            precision: String::new(),
            fallback: false,
            mem_bytes: 0,
            vram_bytes: 0,
            clients: 0,
            busy: 0,
            last_used: Instant::now(),
            last_used_ms: 0,
            error: None,
            disk_bytes,
            schema,
            optimal,
            engine_shown: Mutex::new(None),
            kept_kv: None,
        }
    }

    /// Сколько минут держать KV-кэш чатов после выгрузки.
    fn kv_keep_min(&self, cfg: &Config) -> u32 {
        self.settings.kv_keep_min.unwrap_or(cfg.kv_keep_min)
    }

    /// Устройство, на котором модель работает или будет загружена.
    fn target_device(&self) -> Option<DeviceKind> {
        self.device.or_else(|| devices::resolve(self.settings.device, &self.manifest.devices()))
    }

    /// Действующий профиль движка (только у языковых моделей).
    fn engine(&self) -> Option<EngineProfile> {
        if self.manifest.kind != ModelKind::Llm {
            return None;
        }
        let dev = self.target_device()?;
        let key = (dev, self.settings.quant.clone(), self.settings.engine.clone());
        let mut cache = self.engine_shown.lock().unwrap();
        if let Some((k, v)) = cache.as_ref() {
            if *k == key {
                return Some(v.clone());
            }
        }
        let variant = self.manifest.variant_quant(dev, key.1.as_deref())?;
        let path = self.manifest.path(variant);
        let (_, _, shown) = runners::llm_engine(&path, dev, key.1.as_deref(), &key.2);
        *cache = Some((key, shown.clone()));
        Some(shown)
    }

    fn entry(&self) -> ModelEntry {
        let m = &self.manifest;
        ModelEntry {
            id: m.id.clone(),
            title: m.title.clone(),
            kind: m.kind,
            arch: m.arch.clone(),
            languages: m.languages.clone(),
            description: m.description.clone(),
            license: m.license.clone(),
            package: m.package.clone().unwrap_or_default(),
            disk_bytes: self.disk_bytes,
            devices: m.devices(),
            settings: self.settings.clone(),
            state: self.state,
            device: self.device,
            precision: self.precision.clone(),
            fallback: self.fallback,
            mem_bytes: self.mem_bytes,
            vram_bytes: self.vram_bytes,
            clients: self.clients,
            busy: self.busy > 0,
            last_used_ms: self.last_used_ms,
            error: self.error.clone(),
            library: m.library.as_ref().map(|p| p.to_string_lossy().into_owned()),
            components: m.components.keys().cloned().collect(),
            files: library::files_of(m).iter().map(|f| f.to_string_lossy().into_owned()).collect(),
            npu_cache: m.npu_cache_dir().is_dir(),
            variants: m
                .variants
                .iter()
                .map(|v| synaptixd::VariantInfo {
                    precision: v.precision.clone(),
                    devices: v.devices.clone(),
                    bytes: std::fs::metadata(m.path(v)).map(|x| x.len()).unwrap_or(0),
                })
                .collect(),
            quant_options: defaults::quant_options(m),
            schema: self.schema.clone(),
            defaults: self.optimal.clone(),
            runnable: runners::runnable(m.kind),
            engine: self.engine(),
            path: self
                .target_device()
                .and_then(|d| m.variant_quant(d, self.settings.quant.as_deref()))
                .or(m.variants.first())
                .map(|v| m.path(v).to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }

    fn params(&self) -> Params {
        self.optimal.over(&self.settings.params)
    }
}

pub struct ModelFile {
    pub path: std::path::PathBuf,
    pub library: Option<std::path::PathBuf>,
    pub title: String,
}

pub struct Manager {
    pub cfg: Config,
    slots: Mutex<BTreeMap<String, Slot>>,
    changed: Condvar,
    load_lock: Mutex<()>,
    subscribers: Mutex<Vec<Sender<Event>>>,
    defaults: Mutex<BTreeMap<String, String>>,
    libraries: Mutex<(Vec<LibraryConf>, Option<String>)>,
    evictions: AtomicU64,
    since_ms: i64,
    /// Кому уступать видеокарту (`yield_vram_to`, меняется из «Параметров»).
    vram_yield: Mutex<Vec<String>>,
    /// Сведения о языковых моделях с прошлой загрузки: `llm_info` для
    /// выгруженной модели отвечает из них, а не поднимает её в память.
    llm_infos: Mutex<HashMap<String, synaptixd::LlmInfo>>,
}

fn scan_all(cfg: &Config, libs: &[LibraryConf]) -> BTreeMap<String, Manifest> {
    let mut out: BTreeMap<String, Manifest> = BTreeMap::new();
    for l in libs {
        for m in library::scan(std::path::Path::new(&l.path)) {
            out.entry(m.id.clone()).or_insert(m);
        }
    }
    let legacy: Vec<(std::path::PathBuf, Option<std::path::PathBuf>)> = cfg.model_dirs.iter().map(|d| (d.clone(), None)).collect();
    for (id, m) in registry::scan(&legacy) {
        out.entry(id).or_insert(m);
    }
    out
}

pub struct Lease {
    mgr: Arc<Manager>,
    id: String,
    pub model: ManuallyDrop<Arc<Loaded>>,
    pub device: DeviceKind,
}

impl Drop for Lease {
    fn drop(&mut self) {
        // SAFETY: поле больше не используется — Drop выполняется один раз.
        let model = unsafe { ManuallyDrop::take(&mut self.model) };
        let (entry, unloaded) = {
            let mut slots = self.mgr.slots.lock().unwrap();
            let Some(s) = slots.get_mut(&self.id) else { return };
            s.busy = s.busy.saturating_sub(1);
            s.last_used = Instant::now();
            s.last_used_ms = now_ms();
            (s.entry(), s.model.is_none())
        };
        // Модель выгрузили, пока ей пользовались: последняя ссылка уходит
        // здесь, и только теперь память можно вернуть драйверу.
        let last = unloaded && Arc::strong_count(&model) == 1;
        drop(model);
        if last {
            trim();
            release_vram(Some(self.device));
            tracing::info!("{}: память освобождена после завершения работы", self.id);
        }
        self.mgr.notify(Event::ModelState { model: entry });
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn meminfo() -> (u64, u64) {
    let text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let field = |name: &str| {
        text.lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .map(|kb| kb * 1024)
            .unwrap_or(0)
    };
    (field("MemTotal:"), field("MemAvailable:"))
}

fn rss() -> u64 {
    let text = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: u64 = text.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    pages * unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64
}

fn trim() {
    #[cfg(target_env = "gnu")]
    unsafe {
        libc::malloc_trim(0);
    }
}

/// Вернуть драйверу видеопамять выгруженной модели. Веса и активации живут
/// в CUDA-пулах с порогом «не отдавать» (`cuMemPoolTrimTo` зовётся только
/// при OOM), так что после `Drop` модели её гигабайты оставались за
/// процессом; кэши ядер (TMA-дескрипторы, скретчи) к тому же держат
/// страницы пулов и мешают триму — чистим их первыми.
pub fn release_vram(dev: Option<DeviceKind>) {
    if dev != Some(DeviceKind::Cuda) {
        return;
    }
    let (descs, scratch) = synaptix::facade::llm::cuda_release_kernel_caches();
    let arena = synaptix_core::memory::expert_arena::release_empty(0) >> 20;
    let freed = synaptix::facade::llm::cuda_trim_pool(0);
    let mb = |r: Result<(u64, u64), _>| r.map(|(a, b)| (a >> 20, b >> 20)).unwrap_or((0, 0));
    let dflt = mb(synaptix_core::memory::cuda_pool::cuda_mempool_stats(0));
    let w = mb(synaptix_core::device::cuda::weights_pool_stats(0));
    let a = mb(synaptix_core::device::cuda::activations_pool_stats(0));
    let e = mb(synaptix_core::device::cuda::experts_pool_stats(0));
    let (free, total) = synaptix_core::device::cuda::mem_info(0).map(|(f, t)| (f >> 20, t >> 20)).unwrap_or((0, 0));
    tracing::info!(
        "cuda: пулы ужаты, возвращено {freed} МБ (арена экспертов {arena} МБ, дескрипторов {descs}, скретчей {} МБ); удержано/занято МБ: default {}/{}, weights {}/{}, activations {}/{}, experts {}/{}; свободно {free} из {total}",
        scratch >> 20, dflt.0, dflt.1, w.0, w.1, a.0, a.1, e.0, e.1
    );
}

/// Ошибка — нехватка памяти карты (аллокаторы CUDA пишут её по-разному).
fn is_oom(e: &anyhow::Error) -> bool {
    let s = e.to_string().to_ascii_lowercase();
    s.contains("out of memory") || s.contains(": oom") || s.contains("alloc_zeros")
}

fn no_device(id: &str) -> Error {
    match devices::cuda_yielded() {
        Some(who) => Error::new(ErrorCode::Device, format!("{id} работает только на видеокарте, а она отдана программе «{who}» — модель загрузится, когда программа закроется")),
        None => Error::new(ErrorCode::Device, format!("нет доступного устройства для {id}")),
    }
}

fn no_model(id: &str) -> Error {
    Error::new(ErrorCode::NoModel, format!("модель {id} не установлена"))
}

impl Manager {
    pub fn new(cfg: Config) -> Arc<Self> {
        let state = StateFile::load(&cfg.state);
        let mut slots = BTreeMap::new();
        for (id, m) in scan_all(&cfg, &state.libraries) {
            let settings = state.models.get(&id).cloned().unwrap_or_default();
            slots.insert(id, Slot::new(m, settings));
        }
        tracing::info!("моделей: {}", slots.len());
        Arc::new(Self {
            slots: Mutex::new(slots),
            changed: Condvar::new(),
            load_lock: Mutex::new(()),
            subscribers: Mutex::new(Vec::new()),
            defaults: Mutex::new(state.defaults.clone()),
            libraries: Mutex::new((state.libraries.clone(), state.default_library.clone())),
            evictions: AtomicU64::new(0),
            since_ms: now_ms(),
            vram_yield: Mutex::new(state.vram_yield.clone().unwrap_or_else(|| cfg.yield_vram_to.clone())),
            llm_infos: Mutex::new(HashMap::new()),
            cfg,
        })
    }

    pub fn subscribe(&self, tx: Sender<Event>) {
        self.subscribers.lock().unwrap().push(tx);
    }

    pub fn notify(&self, ev: Event) {
        self.subscribers.lock().unwrap().retain(|tx| tx.send(ev.clone()).is_ok());
    }

    fn notify_slot(&self, id: &str) {
        let entry = self.slots.lock().unwrap().get(id).map(Slot::entry);
        if let Some(model) = entry {
            self.notify(Event::ModelState { model });
        }
    }

    pub fn rescan(&self) {
        let state = StateFile::load(&self.cfg.state);
        let libs = self.libraries.lock().unwrap().0.clone();
        let found = scan_all(&self.cfg, &libs);
        {
            let mut slots = self.slots.lock().unwrap();
            slots.retain(|id, s| found.contains_key(id) || s.model.is_some());
            for (id, m) in found {
                match slots.get_mut(&id) {
                    Some(s) => {
                        s.disk_bytes = m.disk_bytes();
                        s.schema = defaults::schema(&m);
                        s.optimal = defaults::optimal(&s.schema);
                        s.manifest = m;
                    }
                    None => {
                        let settings = state.models.get(&id).cloned().unwrap_or_default();
                        slots.insert(id, Slot::new(m, settings));
                    }
                }
            }
        }
        self.notify(Event::ModelsChanged);
    }

    pub fn list(&self) -> Vec<ModelEntry> {
        self.slots.lock().unwrap().values().map(Slot::entry).collect()
    }

    pub fn entry(&self, id: &str) -> Result<ModelEntry, Error> {
        self.slots.lock().unwrap().get(id).map(Slot::entry).ok_or_else(|| no_model(id))
    }

    pub fn params(&self, id: &str, call: &Params) -> Params {
        self.slots.lock().unwrap().get(id).map(|s| s.params().over(&call.sanitize(&s.schema))).unwrap_or_default()
    }

    /// Стартовый промпт, который пользователь сам вписал в настройки модели
    /// (не значение по умолчанию). Программы со своим системным сообщением
    /// (ассистент) получают его в дополнение, а не теряют.
    pub fn user_system_prompt(&self, id: &str) -> Option<String> {
        let slots = self.slots.lock().unwrap();
        let s = slots.get(id)?;
        let v = s.settings.params.text("system_prompt")?.trim();
        (!v.is_empty() && s.optimal.text("system_prompt").map(str::trim) != Some(v)).then(|| v.to_string())
    }

    fn auto_pick(&self, kind: ModelKind) -> Option<String> {
        let slots = self.slots.lock().unwrap();
        let rank = |s: &Slot| {
            let supported = s.manifest.devices();
            let dev = devices::resolve(s.settings.device, &supported).or_else(|| devices::resolve(DeviceKind::Auto, &supported));
            let dev_rank = match dev {
                Some(DeviceKind::Cuda) => 0,
                Some(DeviceKind::Npu) => 1,
                Some(DeviceKind::Cpu) => 2,
                _ => 3,
            };
            (s.model.is_none(), s.state == LoadState::Failed, dev_rank)
        };
        slots.iter().filter(|(_, s)| s.manifest.kind == kind).min_by_key(|(_, s)| rank(s)).map(|(id, _)| id.clone())
    }

    pub fn resolve(&self, kind: ModelKind) -> Result<String, Error> {
        let chosen = self.defaults.lock().unwrap().get(kind.id()).cloned();
        if let Some(id) = chosen {
            if self.slots.lock().unwrap().get(&id).is_some_and(|s| s.manifest.kind == kind) {
                return Ok(id);
            }
        }
        self.auto_pick(kind).ok_or_else(|| Error::new(ErrorCode::NoModel, format!("нет установленных моделей вида {}", kind.id())))
    }

    /// Модель по id; пусто — по умолчанию; абсолютный путь — та, чей файл
    /// (вариант) лежит по этому пути (шаблоны редактора узлов хранят пути).
    pub fn resolve_id(&self, id: &str, kind: ModelKind) -> Result<String, Error> {
        match id.trim() {
            "" | "auto" | "default" => self.resolve(kind),
            p if p.starts_with('/') => self.id_by_path(std::path::Path::new(p), kind),
            other => Ok(other.to_string()),
        }
    }

    /// Id модели или путь к её файлу (любого вида) — id.
    pub fn resolve_any(&self, id: &str) -> Result<String, Error> {
        if id.trim().starts_with('/') {
            self.id_by_path_of(std::path::Path::new(id.trim()), None)
        } else {
            Ok(id.to_string())
        }
    }

    fn id_by_path(&self, path: &std::path::Path, kind: ModelKind) -> Result<String, Error> {
        self.id_by_path_of(path, Some(kind))
    }

    fn id_by_path_of(&self, path: &std::path::Path, kind: Option<ModelKind>) -> Result<String, Error> {
        let want = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let slots = self.slots.lock().unwrap();
        let of_kind = || slots.iter().filter(|(_, s)| kind.is_none_or(|k| s.manifest.kind == k));
        // Тот же файл; иначе — тот же бандл под тем же именем в другом каталоге
        // (копия, старый путь в настройках программы).
        let same_file = of_kind().find(|(_, s)| {
            s.manifest.variants.iter().any(|v| {
                let p = s.manifest.path(v);
                std::fs::canonicalize(&p).unwrap_or(p) == want
            })
        });
        let same_name = || {
            let name = path.file_name()?;
            of_kind().find(|(_, s)| s.manifest.variants.iter().any(|v| s.manifest.path(v).file_name() == Some(name)))
        };
        same_file
            .or_else(same_name)
            .map(|(id, _)| id.clone())
            .ok_or_else(|| Error::new(ErrorCode::NoModel, format!("{}: такой модели нет в библиотеке службы", path.display())))
    }

    /// Id модели по её файлу (`None` — нет такой).
    pub fn id_of_file(&self, path: &std::path::Path) -> Option<String> {
        self.id_by_path_of(path, Some(ModelKind::Llm)).ok()
    }

    pub fn defaults(&self) -> Vec<DefaultModel> {
        let chosen = self.defaults.lock().unwrap().clone();
        ModelKind::DEFAULTABLE
            .into_iter()
            .map(|kind| DefaultModel { kind, model: chosen.get(kind.id()).cloned(), resolved: self.resolve(kind).ok() })
            .collect()
    }

    pub fn set_default(&self, kind: ModelKind, model: Option<String>) -> Result<Vec<DefaultModel>, Error> {
        if let Some(id) = &model {
            let ok = self.slots.lock().unwrap().get(id).map(|s| s.manifest.kind == kind).ok_or_else(|| no_model(id))?;
            if !ok {
                return Err(Error::new(ErrorCode::WrongKind, format!("{id}: модель другого вида")));
            }
        }
        {
            let mut d = self.defaults.lock().unwrap();
            match model {
                Some(id) => d.insert(kind.id().to_string(), id),
                None => d.remove(kind.id()),
            };
        }
        self.save_state();
        self.notify(Event::ModelsChanged);
        Ok(self.defaults())
    }

    pub fn status(&self) -> Status {
        let (total, avail) = meminfo();
        let slots = self.slots.lock().unwrap();
        Status {
            version: synaptixd::VERSION,
            since_ms: self.since_ms,
            mem_total: total,
            mem_available: avail,
            models_bytes: slots.values().map(|s| s.mem_bytes).sum(),
            loaded: slots.values().filter(|s| s.model.is_some()).count() as u32,
            evictions: self.evictions.load(Ordering::Relaxed),
            devices: devices::list(),
            min_available_mb: self.cfg.min_available_mb,
            vram_yield: self.vram_yield.lock().unwrap().clone(),
            vram_yielded_to: devices::cuda_yielded(),
        }
    }

    /// Задать, кому уступать видеокарту; пустые и повторы отбрасываются.
    pub fn set_vram_yield(&self, programs: Vec<String>) -> Status {
        let mut list: Vec<String> = Vec::new();
        for p in programs {
            let p = p.trim().to_string();
            if !p.is_empty() && !list.contains(&p) {
                list.push(p);
            }
        }
        tracing::info!("уступать видеокарту: {list:?}");
        *self.vram_yield.lock().unwrap() = list;
        self.save_state();
        self.status()
    }

    /// Сведения о языковой модели. Загруженная отвечает сама (и обновляет
    /// запомненное); выгруженная — из запомненного при прошлой загрузке:
    /// окно и пресеты не стоят того, чтобы поднимать модель на карту.
    pub fn llm_info(self: &Arc<Self>, id: &str) -> Result<synaptixd::LlmInfo, Error> {
        let loaded = self.slots.lock().unwrap().get(id).is_some_and(|s| s.model.is_some());
        if !loaded {
            if let Some(info) = self.llm_infos.lock().unwrap().get(id) {
                return Ok(info.clone());
            }
        }
        let info = self.run(id, |m| m.llm_info())?;
        self.llm_infos.lock().unwrap().insert(id.to_string(), info.clone());
        Ok(info)
    }

    /// Память загруженных моделей: (RAM, видеопамять), байт.
    pub fn models_mem(&self) -> (u64, u64) {
        let slots = self.slots.lock().unwrap();
        let loaded = slots.values().filter(|s| s.model.is_some());
        loaded.fold((0, 0), |(r, v), s| if s.device == Some(DeviceKind::Cuda) { (r, v + s.vram_bytes) } else { (r + s.mem_bytes, v) })
    }

    pub fn metrics(&self, history: bool) -> synaptixd::Metrics {
        let snap = crate::activity::snapshot(history);
        // Модели — под замком, сессии — уже без него: замок моделей нужен
        // всем запросам, а сессии заняты, пока идёт ход.
        let loaded: Vec<(String, Arc<Loaded>)> = self.slots.lock().unwrap().iter().filter_map(|(id, s)| Some((id.clone(), s.model.clone()?))).collect();
        let mut kv = Vec::new();
        for (id, m) in loaded {
            for (key, app, tokens, capacity, bytes, parked) in m.kv_usage() {
                kv.push(synaptixd::KvUsage { model: id.clone(), key, app, tokens, capacity, bytes, parked });
            }
        }
        // Удержанный KV выгруженных моделей — тоже память программ.
        for (id, s) in self.slots.lock().unwrap().iter() {
            for (key, app, tokens, capacity, bytes, parked) in s.kept_kv.iter().flat_map(|k| k.kv.rows().iter().cloned()) {
                kv.push(synaptixd::KvUsage { model: id.clone(), key, app, tokens, capacity, bytes, parked });
            }
        }
        let mut apps = snap.apps;
        for k in &kv {
            match apps.iter_mut().find(|u| u.app.pid == k.app.pid && u.app.label == k.app.label) {
                Some(u) => u.kv_bytes += k.bytes,
                None if k.app.pid != 0 && std::path::Path::new(&format!("/proc/{}", k.app.pid)).exists() => {
                    apps.push(synaptixd::AppUsage { app: k.app.clone(), kv_bytes: k.bytes, models: vec![k.model.clone()], ..Default::default() })
                }
                None => {}
            }
        }
        synaptixd::Metrics { now_ms: now_ms(), running: snap.running, recent: snap.recent, apps, kv, history: snap.history }
    }

    pub fn open(self: &Arc<Self>, id: &str) -> Result<ModelInfo, Error> {
        {
            let mut slots = self.slots.lock().unwrap();
            let s = slots.get_mut(id).ok_or_else(|| no_model(id))?;
            s.clients += 1;
        }
        let info = match self.acquire(id) {
            Ok(lease) => {
                let ctc = lease.model.ctc_info();
                drop(lease);
                Ok(ModelInfo { entry: self.entry(id)?, ctc })
            }
            Err(e) => Err(e),
        };
        if info.is_err() {
            self.close(id);
        } else {
            self.notify_slot(id);
        }
        info
    }

    pub fn close(&self, id: &str) {
        if let Some(s) = self.slots.lock().unwrap().get_mut(id) {
            s.clients = s.clients.saturating_sub(1);
        }
        self.notify_slot(id);
    }

    pub fn set(&self, id: &str, ch: SettingsChange) -> Result<ModelEntry, Error> {
        let (entry, reload) = {
            let mut slots = self.slots.lock().unwrap();
            let s = slots.get_mut(id).ok_or_else(|| no_model(id))?;
            let mut reload = false;
            if let Some(d) = ch.device {
                if d != s.settings.device {
                    s.settings.device = d;
                    s.fallback = false;
                    reload = s.model.is_some();
                }
            }
            if let Some(p) = ch.pinned {
                s.settings.pinned = p;
            }
            if let Some(i) = ch.idle_unload_min {
                s.settings.idle_unload_min = i;
            }
            if let Some(k) = ch.kv_keep_min {
                s.settings.kv_keep_min = k;
                if s.kv_keep_min(&self.cfg) == 0 {
                    s.kept_kv = None;
                }
            }
            if let Some(p) = ch.preload {
                s.settings.preload = p;
            }
            if let Some(q) = ch.quant {
                let q = q.filter(|q| q != "auto");
                if q != s.settings.quant {
                    s.settings.quant = q;
                    reload |= s.model.is_some();
                }
            }
            if let Some(p) = ch.params {
                s.settings.params = p.sanitize(&s.schema);
            }
            if let Some(l) = ch.loras {
                // Адаптеры применяются перед следующей генерацией, без перезагрузки.
                s.settings.loras = l;
            }
            if let Some(e) = ch.engine {
                if e != s.settings.engine {
                    let differs = e.reload_differs(&s.settings.engine);
                    s.settings.engine = e;
                    if differs {
                        reload |= s.model.is_some();
                    } else if let (Some(m), Some(dev)) = (&s.model, s.device) {
                        // Граф декода и синхронизация слоёв — на лету.
                        let variant = s.manifest.variant_quant(dev, s.settings.quant.as_deref());
                        if let Some(v) = variant {
                            let (_, rt, _) = runners::llm_engine(&s.manifest.path(v), dev, s.settings.quant.as_deref(), &s.settings.engine);
                            m.set_runtime(rt);
                        }
                    }
                }
            }
            if reload {
                // Новая точность или устройство: посчитанный KV им не годится.
                s.kept_kv = None;
            }
            (s.entry(), reload)
        };
        self.save_state();
        if reload {
            self.unload_with(id, false);
        }
        self.notify_slot(id);
        Ok(if reload { self.entry(id)? } else { entry })
    }

    /// Корни библиотек (существующие).
    pub fn library_paths(&self) -> Vec<std::path::PathBuf> {
        self.libraries.lock().unwrap().0.iter().map(|l| std::path::PathBuf::from(&l.path)).filter(|p| p.is_dir()).collect()
    }

    pub fn default_library(&self) -> Option<std::path::PathBuf> {
        let l = self.libraries.lock().unwrap();
        l.1.clone().or_else(|| l.0.first().map(|c| c.path.clone())).map(std::path::PathBuf::from)
    }

    /// Основной файл модели (вариант для CUDA или первый), её библиотека и название.
    pub fn model_file(&self, id: &str) -> Option<ModelFile> {
        let slots = self.slots.lock().unwrap();
        let s = slots.get(id)?;
        let m = &s.manifest;
        let v = m.variant_quant(DeviceKind::Cuda, s.settings.quant.as_deref()).or(m.variants.first())?;
        Some(ModelFile { path: m.path(v), library: m.library.clone(), title: m.title.clone() })
    }

    /// Набор адаптеров LoRA модели из её настроек.
    pub fn model_loras(&self, id: &str) -> Vec<synaptixd::LoraUse> {
        self.slots.lock().unwrap().get(id).map(|s| s.settings.loras.clone()).unwrap_or_default()
    }

    fn save_state(&self) {
        let mut state = StateFile::load(&self.cfg.state);
        for (id, s) in self.slots.lock().unwrap().iter() {
            if s.settings == ModelSettings::default() {
                state.models.remove(id);
            } else {
                state.models.insert(id.clone(), s.settings.clone());
            }
        }
        state.defaults = self.defaults.lock().unwrap().clone();
        {
            let l = self.libraries.lock().unwrap();
            state.libraries = l.0.clone();
            state.default_library = l.1.clone();
        }
        let vy = self.vram_yield.lock().unwrap().clone();
        state.vram_yield = (vy != self.cfg.yield_vram_to).then_some(vy);
        if let Err(e) = state.save(&self.cfg.state) {
            tracing::warn!("{}: {e}", self.cfg.state.display());
        }
    }

    pub fn libraries(&self) -> Vec<synaptixd::LibraryInfo> {
        let (libs, def) = self.libraries.lock().unwrap().clone();
        let slots = self.slots.lock().unwrap();
        libs.iter()
            .map(|l| {
                let p = std::path::Path::new(&l.path);
                let online = p.is_dir();
                let (total, free) = library::statvfs(p).unwrap_or((0, 0));
                let mine: Vec<&Slot> = slots.values().filter(|s| s.manifest.library.as_deref() == Some(p)).collect();
                let mut files: Vec<std::path::PathBuf> = mine.iter().flat_map(|s| library::files_of(&s.manifest)).collect();
                files.sort();
                files.dedup();
                synaptixd::LibraryInfo {
                    path: l.path.clone(),
                    name: if l.name.is_empty() { l.path.clone() } else { l.name.clone() },
                    online,
                    default: def.as_deref() == Some(l.path.as_str()) || (def.is_none() && libs.first().map(|x| &x.path) == Some(&l.path)),
                    total,
                    free,
                    models: mine.len() as u32,
                    used: files.iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum(),
                }
            })
            .collect()
    }

    pub fn add_library(&self, path: &str, name: Option<String>) -> Result<Vec<synaptixd::LibraryInfo>, Error> {
        let path = path.trim().trim_end_matches('/').to_string();
        if !path.starts_with('/') {
            return Err(Error::new(ErrorCode::Failed, "нужен абсолютный путь к папке"));
        }
        let name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| {
            std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone())
        });
        library::ensure(std::path::Path::new(&path)).map_err(|e| Error::new(ErrorCode::Failed, format!("{path}: {e}")))?;
        {
            let mut l = self.libraries.lock().unwrap();
            if !l.0.iter().any(|x| x.path == path) {
                l.0.push(LibraryConf { path, name });
            }
        }
        self.save_state();
        self.rescan();
        Ok(self.libraries())
    }

    pub fn remove_library(&self, path: &str) -> Result<Vec<synaptixd::LibraryInfo>, Error> {
        {
            let mut l = self.libraries.lock().unwrap();
            l.0.retain(|x| x.path != path);
            if l.1.as_deref() == Some(path) {
                l.1 = None;
            }
        }
        self.save_state();
        self.rescan();
        Ok(self.libraries())
    }

    pub fn set_default_library(&self, path: &str) -> Result<Vec<synaptixd::LibraryInfo>, Error> {
        {
            let mut l = self.libraries.lock().unwrap();
            if !l.0.iter().any(|x| x.path == path) {
                return Err(Error::new(ErrorCode::Failed, "нет такой библиотеки"));
            }
            l.1 = Some(path.to_string());
        }
        self.save_state();
        Ok(self.libraries())
    }

    pub fn delete_model(&self, id: &str) -> Result<(), Error> {
        let (manifest, others) = {
            let slots = self.slots.lock().unwrap();
            let s = slots.get(id).ok_or_else(|| no_model(id))?;
            if s.manifest.library.is_none() {
                return Err(Error::new(ErrorCode::Failed, "модель лежит вне библиотек — удалите её файлы сами"));
            }
            let others: Vec<std::path::PathBuf> = slots.iter().filter(|(k, _)| k.as_str() != id).flat_map(|(_, s)| library::files_of(&s.manifest)).collect();
            (s.manifest.clone(), others)
        };
        self.unload_with(id, false);
        if let Some(s) = self.slots.lock().unwrap().get_mut(id) {
            s.kept_kv = None;
        }
        let r = if library::is_own_dir(&manifest) {
            std::fs::remove_dir_all(&manifest.dir).map(|_| ())
        } else {
            library::files_of(&manifest).iter().filter(|f| !others.contains(f)).try_for_each(std::fs::remove_file)
        };
        r.map_err(|e| Error::new(ErrorCode::Failed, e.to_string()))?;
        tracing::info!("{id}: удалена");
        self.rescan();
        Ok(())
    }

    /// Выгрузить модель; KV-кэш чатов остаётся ждать её в RAM (см.
    /// [`Self::unload_with`]).
    pub fn unload(&self, id: &str) -> bool {
        self.unload_with(id, true)
    }

    /// `keep_kv` — увести KV-сессии языковой модели в RAM и держать их
    /// `kv_keep_min` минут: загрузится снова — чаты продолжатся без префилла.
    /// Не держим при нехватке RAM и когда модель грузится с другими
    /// настройками.
    fn unload_with(&self, id: &str, keep_kv: bool) -> bool {
        let (model, dev, keep) = {
            let mut slots = self.slots.lock().unwrap();
            let Some(s) = slots.get_mut(id) else { return false };
            let m = s.model.take();
            let dev = s.device;
            let keep_min = if keep_kv { s.kv_keep_min(&self.cfg) } else { 0 };
            let keep = (keep_min > 0 && m.is_some()).then(|| (keep_min, s.precision.clone()));
            if m.is_some() {
                s.state = LoadState::Unloaded;
                s.device = None;
                s.mem_bytes = 0;
                s.vram_bytes = 0;
                s.precision.clear();
                s.fallback = false;
            }
            (m, dev, keep)
        };
        let Some(model) = model else { return false };
        if let Some((keep_min, precision)) = keep {
            if let Some(kv) = model.park_sessions() {
                tracing::info!(
                    "{id}: KV-кэш чатов ({} МБ, сессий {}) ждёт в RAM {keep_min} мин",
                    kv.bytes() >> 20,
                    kv.rows().len()
                );
                if let Some(s) = self.slots.lock().unwrap().get_mut(id) {
                    let until = Instant::now() + Duration::from_secs(keep_min as u64 * 60);
                    s.kept_kv = Some(KeptSlot { kv, until, device: dev, precision });
                }
            }
        }
        model.release_device_caches();
        // Модель занята (`Lease` в работе): последняя ссылка уйдёт в его
        // Drop, там же вернётся и память.
        let last = Arc::strong_count(&model) == 1;
        drop(model);
        if last {
            trim();
            release_vram(dev);
        }
        tracing::info!("{id}: выгружена");
        self.notify_slot(id);
        true
    }

    /// Кого выгрузить: незанятая, не закреплённая, давно не использованная;
    /// `dev` — только модели на этом устройстве (освобождение видеопамяти).
    fn pick_victim(&self, except: &str, dev: Option<DeviceKind>) -> Option<String> {
        let slots = self.slots.lock().unwrap();
        slots
            .iter()
            .filter(|(id, s)| s.model.is_some() && s.busy == 0 && !s.settings.pinned && id.as_str() != except)
            .filter(|(_, s)| dev.is_none() || s.device == dev)
            .min_by_key(|(_, s)| (s.clients > 0, s.last_used))
            .map(|(id, _)| id.clone())
    }

    /// Выгрузить с устройства все незанятые модели, кроме `except` —
    /// генерации нужна вся видеопамять (пайплайн грузит этапы сам).
    pub fn clear_device(&self, except: &str, dev: DeviceKind, why: &str) {
        // DiT и Gemma, что держат стадии LTX графа, — тоже место на карте.
        crate::stages::release_ltx(Some(except));
        while self.evict_one_on(except, Some(dev), why) {}
    }

    fn evict_one(&self, except: &str, why: &str) -> bool {
        self.evict_one_on(except, None, why)
    }

    fn evict_one_on(&self, except: &str, dev: Option<DeviceKind>, why: &str) -> bool {
        let Some(victim) = self.pick_victim(except, dev) else { return false };
        // Ради видеопамяти KV-кэш уезжает ждать в RAM; ради самой RAM — нет.
        if self.unload_with(&victim, dev.is_some()) {
            self.evictions.fetch_add(1, Ordering::Relaxed);
            tracing::info!("{victim}: выгружена ({why})");
            true
        } else {
            false
        }
    }

    fn make_room(&self, id: &str, need: u64) -> Result<(), Error> {
        let min = self.cfg.min_available_mb << 20;
        if self.cfg.max_loaded > 0 {
            loop {
                let loaded = self.slots.lock().unwrap().values().filter(|s| s.model.is_some()).count();
                if loaded < self.cfg.max_loaded || !self.evict_one(id, "предел числа моделей") {
                    break;
                }
            }
        }
        loop {
            let (_, avail) = meminfo();
            if avail >= need + min {
                return Ok(());
            }
            if self.drop_kept_kv("не хватает памяти для новой") {
                continue;
            }
            if !self.evict_one(id, "не хватает памяти для новой") {
                if avail >= need {
                    return Ok(());
                }
                return Err(Error::new(
                    ErrorCode::NoMemory,
                    format!("не хватает памяти: нужно {} МБ, свободно {} МБ", need >> 20, avail >> 20),
                ));
            }
        }
    }

    /// Освободить видеопамять под новую модель: пока свободной VRAM меньше
    /// оценки с запасом, выгружать с карты незанятые модели. Без жертв —
    /// грузим как есть (оценка груба, а загрузчик сам умеет стримить слои);
    /// раньше при занятой карте загрузка падала и модель тихо уходила на CPU.
    pub fn make_vram_room(&self, id: &str, dev: DeviceKind, need: u64) {
        if dev != DeviceKind::Cuda || need == 0 {
            return;
        }
        let free = || synaptix_core::device::cuda::mem_info(0).map(|(f, _)| f as u64).unwrap_or(u64::MAX);
        let margin = 768u64 << 20;
        while free() < need + margin {
            if !self.evict_one_on(id, Some(DeviceKind::Cuda), "не хватает видеопамяти для новой") {
                break;
            }
        }
    }

    pub fn acquire(self: &Arc<Self>, id: &str) -> Result<Lease, Error> {
        if let Some(l) = crate::activity::current() {
            l.model(id);
        }
        loop {
            {
                let mut slots = self.slots.lock().unwrap();
                let s = slots.get_mut(id).ok_or_else(|| no_model(id))?;
                match (&s.model, s.state) {
                    (Some(m), _) => {
                        s.busy += 1;
                        s.last_used = Instant::now();
                        s.last_used_ms = now_ms();
                        return Ok(Lease { mgr: self.clone(), id: id.into(), model: ManuallyDrop::new(m.clone()), device: s.device.unwrap_or(DeviceKind::Cpu) });
                    }
                    (None, LoadState::Loading) => {
                        let _ = self.changed.wait_timeout(slots, Duration::from_millis(200)).unwrap();
                        continue;
                    }
                    _ => {}
                }
            }
            self.load(id, None)?;
        }
    }

    pub fn load(&self, id: &str, force: Option<DeviceKind>) -> Result<(), Error> {
        let _guard = self.load_lock.lock().unwrap_or_else(|e| e.into_inner());
        let (manifest, want, quant, engine) = {
            let mut slots = self.slots.lock().unwrap();
            let s = slots.get_mut(id).ok_or_else(|| no_model(id))?;
            if s.model.is_some() && force.is_none() {
                return Ok(());
            }
            s.state = LoadState::Loading;
            s.error = None;
            // Каждая загрузка снова пробует выбранное устройство: откат на CPU
            // был разовым (не хватило видеопамяти), а не приговором модели.
            let want = force.unwrap_or(s.settings.device);
            (s.manifest.clone(), want, s.settings.quant.clone(), s.settings.engine.clone())
        };
        match crate::activity::requester() {
            Some(who) => tracing::info!("{id}: загрузка по запросу {who}"),
            None => tracing::info!("{id}: загрузка (служба сама)"),
        }
        self.notify_slot(id);
        let result = self.load_inner(&manifest, want, quant.as_deref(), &engine);
        let mut slots = self.slots.lock().unwrap();
        let s = slots.get_mut(id).ok_or_else(|| no_model(id))?;
        let out = match result {
            Ok((model, dev, precision, bytes, vram)) => {
                // Замер прироста пула при загрузке; нет замера — оценка.
                s.vram_bytes = match dev {
                    DeviceKind::Cuda if vram > 0 => vram,
                    DeviceKind::Cuda => manifest.vram_estimate(dev, quant.as_deref()),
                    _ => 0,
                };
                if let Some(k) = s.kept_kv.take() {
                    if k.until > Instant::now() && k.device == Some(dev) && k.precision == precision {
                        tracing::info!("{id}: KV-кэш чатов ({} МБ) вернулся к модели", k.kv.bytes() >> 20);
                        model.adopt_sessions(k.kv);
                    } else {
                        tracing::info!("{id}: удержанный KV-кэш не подошёл (другое устройство или точность) — сброшен");
                    }
                }
                if let Ok(info) = model.llm_info() {
                    self.llm_infos.lock().unwrap().insert(id.to_string(), info);
                }
                s.model = Some(Arc::new(model));
                s.device = Some(dev);
                s.precision = precision;
                s.mem_bytes = bytes;
                s.state = LoadState::Loaded;
                s.fallback = (s.settings.device != DeviceKind::Auto && s.settings.device != dev)
                    || (force == Some(DeviceKind::Cpu) && s.settings.device != DeviceKind::Cpu);
                s.last_used = Instant::now();
                s.last_used_ms = now_ms();
                tracing::info!("{id}: загружена на {} ({} МБ)", dev.id(), bytes >> 20);
                Ok(())
            }
            Err(e) => {
                s.state = LoadState::Failed;
                s.error = Some(e.message.clone());
                tracing::warn!("{id}: {}", e.message);
                Err(e)
            }
        };
        drop(slots);
        self.changed.notify_all();
        self.notify_slot(id);
        out
    }

    fn load_inner(&self, m: &Manifest, want: DeviceKind, quant: Option<&str>, engine: &EngineProfile) -> Result<(Loaded, DeviceKind, String, u64, u64), Error> {
        let supported = m.devices();
        let dev = match devices::resolve(want, &supported) {
            Some(d) => d,
            None if want != DeviceKind::Auto => devices::resolve(DeviceKind::Auto, &supported).ok_or_else(|| no_device(&m.id))?,
            None => return Err(no_device(&m.id)),
        };
        let variant = m.variant_quant(dev, quant).ok_or_else(|| Error::new(ErrorCode::Device, "нет варианта весов для устройства"))?;
        let path = m.path(variant);
        self.make_room(&m.id, m.ram_estimate(dev, quant))?;
        self.make_vram_room(&m.id, dev, m.vram_estimate(dev, quant));
        let before = rss();
        let vram_before = synaptix_core::memory::cuda_pool::cuda_allocated_bytes() as u64;
        let t0 = Instant::now();
        let loaded = {
            let _dl = devices::lock(dev);
            runners::load(m.kind, &m.arch, &path, &m.component_paths(), dev, quant, engine)
        };
        let (model, dev, variant) = match loaded {
            Ok(model) => (model, dev, variant),
            Err(e) if dev != DeviceKind::Cpu && m.devices().contains(&DeviceKind::Cpu) => {
                tracing::warn!("{}: {} не загрузилась ({e}); загружаю на CPU", m.id, dev.id());
                let cpu = m.variant_for(DeviceKind::Cpu).ok_or_else(|| Error::new(ErrorCode::Device, "нет варианта весов для CPU"))?;
                let model = runners::load(m.kind, &m.arch, &m.path(cpu), &m.component_paths(), DeviceKind::Cpu, quant, engine)
                    .map_err(|e| Error::new(ErrorCode::Failed, format!("{}: {e}", m.path(cpu).display())))?;
                (model, DeviceKind::Cpu, cpu)
            }
            Err(e) => return Err(Error::new(ErrorCode::Failed, format!("{}: {e}", path.display()))),
        };
        tracing::info!("{}: {} за {:.1} с", m.id, path.display(), t0.elapsed().as_secs_f32());
        let floor = match dev {
            DeviceKind::Npu => m.ram_estimate(dev, quant),
            _ => m.resources.ram.get(&dev).map(|mb| mb << 20).unwrap_or(0),
        };
        let bytes = rss().saturating_sub(before).max(floor);
        let vram = if dev == DeviceKind::Cuda { (synaptix_core::memory::cuda_pool::cuda_allocated_bytes() as u64).saturating_sub(vram_before) } else { 0 };
        let (model, label) = model;
        Ok((model, dev, label.unwrap_or_else(|| variant.precision.clone()), bytes, vram))
    }

    pub fn run<T>(
        self: &Arc<Self>,
        id: &str,
        f: impl Fn(&Loaded) -> anyhow::Result<T>,
    ) -> Result<T, Error> {
        let live = crate::activity::current();
        let lease = self.acquire(id)?;
        let dev = lease.device;
        let mut r = {
            let _dl = devices::lock(dev);
            if let Some(l) = &live {
                l.running();
            }
            f(&lease.model)
        };
        // Нехватка видеопамяти — не повод уходить на CPU: модель отдаёт
        // хосту часть блоков и ход повторяется на карте.
        // Повторов — не больше восьми: модель запоминает отданное и следующий
        // план его учитывает, но круг «отдал — вернул — упал» недопустим.
        let mut retries = 0;
        while dev == DeviceKind::Cuda && retries < 8 && r.as_ref().err().is_some_and(is_oom) && lease.model.shed_vram() {
            retries += 1;
            tracing::warn!("{id}: ход упёрся в видеопамять ({}); часть блоков — на хост, повтор на cuda", r.as_ref().err().unwrap());
            release_vram(Some(DeviceKind::Cuda));
            let _dl = devices::lock(dev);
            r = f(&lease.model);
        }
        match r {
            Ok(v) => Ok(v),
            // Отмена хода — не сбой устройства: повторять её на CPU незачем.
            Err(e) if e.to_string().contains(synaptix_llm_common::generate::INTERRUPTED) => Err(Error::new(ErrorCode::Failed, e.to_string())),
            Err(e) if dev != DeviceKind::Cpu && lease.model.cpu_fallback() => {
                tracing::warn!("{id}: ошибка на {} ({e}); повтор на CPU", dev.id());
                drop(lease);
                self.unload_with(id, false);
                if let Some(s) = self.slots.lock().unwrap().get_mut(id) {
                    s.fallback = true;
                }
                self.load(id, Some(DeviceKind::Cpu))?;
                let lease = self.acquire(id)?;
                f(&lease.model).map_err(|e| Error::new(ErrorCode::Failed, e.to_string()))
            }
            Err(e) => Err(Error::new(ErrorCode::Failed, e.to_string())),
        }
    }

    pub fn preload(self: &Arc<Self>) {
        let ids: Vec<String> =
            self.slots.lock().unwrap().iter().filter(|(_, s)| s.settings.preload).map(|(id, _)| id.clone()).collect();
        for id in ids {
            if let Err(e) = self.acquire(&id).and_then(|l| l.model.warmup().map_err(|e| Error::new(ErrorCode::Failed, e.to_string()))) {
                tracing::warn!("{id}: предзагрузка: {e}");
            }
        }
    }

    /// Сбросить KV-кэш, чей срок ожидания вышел.
    fn expire_kept_kv(&self) {
        let now = Instant::now();
        let mut slots = self.slots.lock().unwrap();
        for (id, s) in slots.iter_mut() {
            if s.kept_kv.as_ref().is_some_and(|k| k.until <= now) {
                let k = s.kept_kv.take();
                tracing::info!("{id}: удержанный KV-кэш ({} МБ) сброшен по сроку", k.map(|k| k.kv.bytes() >> 20).unwrap_or(0));
            }
        }
    }

    /// Сбросить весь удержанный KV-кэш; `true` — было что сбрасывать.
    fn drop_kept_kv(&self, why: &str) -> bool {
        let dropped: Vec<(String, KeptSlot)> = {
            let mut slots = self.slots.lock().unwrap();
            slots.iter_mut().filter_map(|(id, s)| Some((id.clone(), s.kept_kv.take()?))).collect()
        };
        for (id, k) in &dropped {
            tracing::info!("{id}: удержанный KV-кэш ({} МБ) сброшен ({why})", k.kv.bytes() >> 20);
        }
        let any = !dropped.is_empty();
        drop(dropped);
        if any {
            trim();
        }
        any
    }

    /// Уступить видеокарту программе: выгрузить с неё все модели (и
    /// закреплённые), отпустить стадии LTX. Занятая модель вернёт память,
    /// закончив текущую работу; KV-кэш чатов ждёт в RAM.
    fn yield_cuda(&self, who: &str) {
        crate::stages::release_ltx(None);
        let ids: Vec<String> = self
            .slots
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, s)| s.model.is_some() && s.device == Some(DeviceKind::Cuda))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if self.unload_with(&id, true) {
                self.evictions.fetch_add(1, Ordering::Relaxed);
                tracing::info!("{id}: выгружена (видеокарта нужна программе «{who}»)");
            }
        }
        release_vram(Some(DeviceKind::Cuda));
    }

    /// Следить за программами из `yield_vram_to`: запущена — карта её,
    /// модели грузятся на CPU/NPU; закрылась — карта снова нейросетям.
    pub fn watch_vram_yield(self: &Arc<Self>) {
        loop {
            let names = self.vram_yield.lock().unwrap().clone();
            let found = if names.is_empty() { None } else { find_process(&names) };
            match (found, devices::cuda_yielded()) {
                (Some(who), None) => {
                    tracing::info!("запущена «{who}» — видеокарта уступлена ей");
                    devices::set_cuda_yielded(Some(who.clone()));
                    self.yield_cuda(&who);
                }
                // Загрузка, начатая до уступки, могла закончиться на карте.
                (Some(_), Some(who)) => {
                    if self.slots.lock().unwrap().values().any(|s| s.model.is_some() && s.device == Some(DeviceKind::Cuda)) {
                        self.yield_cuda(&who);
                    }
                }
                (None, Some(who)) => {
                    devices::set_cuda_yielded(None);
                    tracing::info!("«{who}» закрыта — видеокарта снова у нейросетей");
                }
                (None, None) => {}
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    pub fn monitor(self: &Arc<Self>) {
        let min = self.cfg.min_available_mb << 20;
        loop {
            std::thread::sleep(Duration::from_secs(5));
            self.expire_kept_kv();
            let (_, avail) = meminfo();
            if avail < min {
                // Сначала — удержанный KV выгруженных моделей: он дешевле
                // любой загруженной модели.
                if self.drop_kept_kv("мало свободной памяти") {
                    continue;
                }
                while meminfo().1 < min && self.evict_one("", "мало свободной памяти") {}
            }
            let idle: Vec<String> = {
                let slots = self.slots.lock().unwrap();
                slots
                    .iter()
                    .filter(|(_, s)| {
                        let limit = s.settings.idle_unload_min.unwrap_or(self.cfg.idle_unload_min);
                        s.model.is_some()
                            && s.busy == 0
                            && !s.settings.pinned
                            && limit > 0
                            && s.last_used.elapsed() > Duration::from_secs(limit as u64 * 60)
                    })
                    .map(|(id, _)| id.clone())
                    .collect()
            };
            for id in idle {
                if self.unload(&id) {
                    tracing::info!("{id}: выгружена по простою");
                }
            }
        }
    }
}

/// Первый процесс, чьё имя (`comm`, до 15 символов) есть в `names`;
/// возвращает имя программы по её ярлыку (`DaVinci Resolve`).
fn find_process(names: &[String]) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let names: Vec<String> = names.iter().map(|n| n.chars().take(15).collect()).collect();
    for e in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else { continue };
        let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) else { continue };
        if names.iter().any(|n| n == comm.trim()) {
            let uid = e.metadata().map(|m| m.uid()).unwrap_or(0);
            return Some(crate::activity::app_of(pid, uid).name);
        }
    }
    None
}
