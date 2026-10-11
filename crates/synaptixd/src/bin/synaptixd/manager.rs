use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use synaptixd::{DeviceKind, Error, ErrorCode, Event, LoadState, ModelEntry, ModelInfo, ModelSettings, SettingsChange, Status};

use crate::config::Config;
use crate::devices;
use crate::registry::{self, Manifest, StateFile};
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
}

impl Slot {
    fn new(manifest: Manifest, settings: ModelSettings) -> Self {
        let disk_bytes = manifest.disk_bytes();
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
        }
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
            package: m.package(),
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
        }
    }
}

pub struct Manager {
    pub cfg: Config,
    slots: Mutex<BTreeMap<String, Slot>>,
    changed: Condvar,
    load_lock: Mutex<()>,
    subscribers: Mutex<Vec<Sender<Event>>>,
    evictions: AtomicU64,
    since_ms: i64,
}

pub struct Lease {
    mgr: Arc<Manager>,
    id: String,
    pub model: Arc<Loaded>,
    pub device: DeviceKind,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let entry = {
            let mut slots = self.mgr.slots.lock().unwrap();
            let Some(s) = slots.get_mut(&self.id) else { return };
            s.busy = s.busy.saturating_sub(1);
            s.last_used = Instant::now();
            s.last_used_ms = now_ms();
            s.entry()
        };
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

fn no_model(id: &str) -> Error {
    Error::new(ErrorCode::NoModel, format!("модель {id} не установлена"))
}

impl Manager {
    pub fn new(cfg: Config) -> Arc<Self> {
        let state = StateFile::load(&cfg.state);
        let mut slots = BTreeMap::new();
        for (id, m) in registry::scan(&cfg.model_dirs) {
            let settings = state.models.get(&id).cloned().unwrap_or_default();
            slots.insert(id, Slot::new(m, settings));
        }
        tracing::info!("моделей: {}", slots.len());
        Arc::new(Self {
            cfg,
            slots: Mutex::new(slots),
            changed: Condvar::new(),
            load_lock: Mutex::new(()),
            subscribers: Mutex::new(Vec::new()),
            evictions: AtomicU64::new(0),
            since_ms: now_ms(),
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
        let found = registry::scan(&self.cfg.model_dirs);
        {
            let mut slots = self.slots.lock().unwrap();
            slots.retain(|id, s| found.contains_key(id) || s.model.is_some());
            for (id, m) in found {
                match slots.get_mut(&id) {
                    Some(s) => {
                        s.disk_bytes = m.disk_bytes();
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
        }
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
            if let Some(p) = ch.preload {
                s.settings.preload = p;
            }
            (s.entry(), reload)
        };
        self.save_state();
        if reload {
            self.unload(id);
        }
        self.notify_slot(id);
        Ok(if reload { self.entry(id)? } else { entry })
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
        if let Err(e) = state.save(&self.cfg.state) {
            tracing::warn!("{}: {e}", self.cfg.state.display());
        }
    }

    pub fn unload(&self, id: &str) -> bool {
        let model = {
            let mut slots = self.slots.lock().unwrap();
            let Some(s) = slots.get_mut(id) else { return false };
            let m = s.model.take();
            if m.is_some() {
                s.state = LoadState::Unloaded;
                s.device = None;
                s.mem_bytes = 0;
                s.vram_bytes = 0;
                s.precision.clear();
            }
            m
        };
        let was = model.is_some();
        drop(model);
        if was {
            trim();
            tracing::info!("{id}: выгружена");
            self.notify_slot(id);
        }
        was
    }

    fn pick_victim(&self, except: &str) -> Option<String> {
        let slots = self.slots.lock().unwrap();
        slots
            .iter()
            .filter(|(id, s)| s.model.is_some() && s.busy == 0 && !s.settings.pinned && id.as_str() != except)
            .min_by_key(|(_, s)| (s.clients > 0, s.last_used))
            .map(|(id, _)| id.clone())
    }

    fn evict_one(&self, except: &str, why: &str) -> bool {
        let Some(victim) = self.pick_victim(except) else { return false };
        if self.unload(&victim) {
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

    pub fn acquire(self: &Arc<Self>, id: &str) -> Result<Lease, Error> {
        loop {
            {
                let mut slots = self.slots.lock().unwrap();
                let s = slots.get_mut(id).ok_or_else(|| no_model(id))?;
                match (&s.model, s.state) {
                    (Some(m), _) => {
                        s.busy += 1;
                        s.last_used = Instant::now();
                        s.last_used_ms = now_ms();
                        return Ok(Lease { mgr: self.clone(), id: id.into(), model: m.clone(), device: s.device.unwrap_or(DeviceKind::Cpu) });
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
        let (manifest, want) = {
            let mut slots = self.slots.lock().unwrap();
            let s = slots.get_mut(id).ok_or_else(|| no_model(id))?;
            if s.model.is_some() && force.is_none() {
                return Ok(());
            }
            s.state = LoadState::Loading;
            s.error = None;
            let want = force.unwrap_or(if s.fallback { DeviceKind::Cpu } else { s.settings.device });
            (s.manifest.clone(), want)
        };
        self.notify_slot(id);
        let result = self.load_inner(&manifest, want);
        let mut slots = self.slots.lock().unwrap();
        let s = slots.get_mut(id).ok_or_else(|| no_model(id))?;
        let out = match result {
            Ok((model, dev, precision, bytes)) => {
                s.vram_bytes = if dev == DeviceKind::Cuda { manifest.vram_estimate(dev) } else { 0 };
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

    fn load_inner(&self, m: &Manifest, want: DeviceKind) -> Result<(Loaded, DeviceKind, String, u64), Error> {
        let supported = m.devices();
        let dev = match devices::resolve(want, &supported) {
            Some(d) => d,
            None if want != DeviceKind::Auto => devices::resolve(DeviceKind::Auto, &supported).ok_or_else(|| {
                Error::new(ErrorCode::Device, format!("нет доступного устройства для {}", m.id))
            })?,
            None => return Err(Error::new(ErrorCode::Device, format!("нет доступного устройства для {}", m.id))),
        };
        let variant = m.variant_for(dev).ok_or_else(|| Error::new(ErrorCode::Device, "нет варианта весов для устройства"))?;
        let path = m.path(variant);
        self.make_room(&m.id, m.ram_estimate(dev))?;
        let before = rss();
        let t0 = Instant::now();
        let loaded = {
            let _dl = devices::lock(dev);
            runners::load(m.kind, &m.arch, &path, dev)
        };
        let (model, dev, variant) = match loaded {
            Ok(model) => (model, dev, variant),
            Err(e) if dev != DeviceKind::Cpu && m.devices().contains(&DeviceKind::Cpu) => {
                tracing::warn!("{}: {} не загрузилась ({e}); загружаю на CPU", m.id, dev.id());
                let cpu = m.variant_for(DeviceKind::Cpu).ok_or_else(|| Error::new(ErrorCode::Device, "нет варианта весов для CPU"))?;
                let model = runners::load(m.kind, &m.arch, &m.path(cpu), DeviceKind::Cpu)
                    .map_err(|e| Error::new(ErrorCode::Failed, format!("{}: {e}", m.path(cpu).display())))?;
                (model, DeviceKind::Cpu, cpu)
            }
            Err(e) => return Err(Error::new(ErrorCode::Failed, format!("{}: {e}", path.display()))),
        };
        tracing::info!("{}: {} за {:.1} с", m.id, path.display(), t0.elapsed().as_secs_f32());
        let floor = match dev {
            DeviceKind::Npu => m.ram_estimate(dev),
            _ => m.resources.ram.get(&dev).map(|mb| mb << 20).unwrap_or(0),
        };
        let bytes = rss().saturating_sub(before).max(floor);
        Ok((model, dev, variant.precision.clone(), bytes))
    }

    pub fn run<T>(
        self: &Arc<Self>,
        id: &str,
        f: impl Fn(&Loaded) -> anyhow::Result<T>,
    ) -> Result<T, Error> {
        let lease = self.acquire(id)?;
        let dev = lease.device;
        let r = {
            let _dl = devices::lock(dev);
            f(&lease.model)
        };
        match r {
            Ok(v) => Ok(v),
            Err(e) if dev != DeviceKind::Cpu => {
                tracing::warn!("{id}: ошибка на {} ({e}); повтор на CPU", dev.id());
                drop(lease);
                self.unload(id);
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

    pub fn monitor(self: &Arc<Self>) {
        let min = self.cfg.min_available_mb << 20;
        loop {
            std::thread::sleep(Duration::from_secs(5));
            let (_, avail) = meminfo();
            if avail < min {
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
