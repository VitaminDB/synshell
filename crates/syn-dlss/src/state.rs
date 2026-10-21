//! Состояние окна DLSS 5: вкладка, снимок состояния помощника и слоя (опрос общей памяти раз в
//! полсекунды), диалоги, профили. Настройки живут в общей памяти — сигналы только перестраивают
//! страницы, когда настройку поменял кто-то другой (dlssnr-shmctl, профиль, сброс).

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use syngui::prelude::*;

use crate::config;
use crate::paths;
use crate::shm::{self, Field, ShmHeader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Rendering,
    Quality,
    Composition,
    Inspect,
    Helper,
}

impl Tab {
    pub const ALL: [Tab; 5] = [Tab::Rendering, Tab::Quality, Tab::Composition, Tab::Inspect, Tab::Helper];

    pub fn label(self) -> String {
        match self {
            Tab::Rendering => t!("Рендеринг"),
            Tab::Quality => t!("Качество"),
            Tab::Composition => t!("Композиция"),
            Tab::Inspect => t!("Проверка"),
            Tab::Helper => t!("Помощник"),
        }
    }

    pub fn parse(s: &str) -> Option<Tab> {
        Some(match s {
            "rendering" => Tab::Rendering,
            "quality" => Tab::Quality,
            "composition" => Tab::Composition,
            "inspect" => Tab::Inspect,
            "helper" => Tab::Helper,
            _ => return None,
        })
    }
}

/// Слой в игре: кадры идут, слой подключён, но кадров нет (пауза, свёрнуто), или слоя нет.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layer {
    Active,
    Idle,
    #[default]
    None,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    pub attached: bool,
    /// Общая память другой версии: слой или помощник устарели.
    pub mismatch: Option<u32>,
    pub running: bool,
    pub helper_state: u32,
    pub reason: String,
    pub model_up: bool,
    pub layer: Layer,
    pub game: String,
    pub layer_size: (u32, u32),
    pub layer_reason: String,
    pub helper_frames: u64,
    pub eval_ms: f32,
    pub upload_ms: f32,
    pub readback_ms: f32,
    pub layer_ms: f32,
    pub vram_mb: u32,
    pub features: u32,
    pub dmabuf: bool,
    pub hdr: bool,
    pub model_missing: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Dialog {
    None,
    Import,
    SaveProfile,
    DeleteProfile(String),
}

#[derive(Clone, Copy)]
pub struct St {
    pub tab: RwSignal<Tab>,
    pub status: RwSignal<Status>,
    /// Перестроить страницы настроек (настройку поменяли снаружи).
    pub rev: RwSignal<u64>,
    pub bypass: RwSignal<bool>,
    pub pass_sel: RwSignal<usize>,
    pub busy: RwSignal<Option<String>>,
    pub toast: RwSignal<Option<(u64, String)>>,
    pub dialog: RwSignal<Dialog>,
    pub reset_open: RwSignal<bool>,
    pub profiles: RwSignal<Vec<String>>,
    pub profile: RwSignal<String>,
    pub runner: RwSignal<(String, String)>,
    pub binaries: RwSignal<String>,
    pub window: RwSignal<syngui::window::WindowState>,
    pub theme: RwSignal<String>,
}

struct Poll {
    blob: String,
    dirty: bool,
    frames: u64,
    first: bool,
    toast_seq: u64,
}

static POLL: Mutex<Poll> = Mutex::new(Poll { blob: String::new(), dirty: false, frames: 0, first: true, toast_seq: 0 });

pub fn header() -> Option<&'static ShmHeader> {
    shm::header()
}

impl St {
    pub fn new() -> Self {
        let ini = config::Ini::load();
        let runner = (ini.get("runner_type").unwrap_or("").to_string(), ini.get("runner_path").unwrap_or("").to_string());
        let binaries = ini.get("binaries").filter(|b| !b.is_empty()).map(str::to_string).unwrap_or_else(|| paths::default_binaries().display().to_string());
        let st = Self {
            tab: use_signal(Tab::Rendering),
            status: use_signal(Status::default()),
            rev: use_signal(0),
            bypass: use_signal(true),
            pass_sel: use_signal(0),
            busy: use_signal(None),
            toast: use_signal(None),
            dialog: use_signal(Dialog::None),
            reset_open: use_signal(false),
            profiles: use_signal(config::profiles()),
            profile: use_signal(ini.get("profile").map(profile_name).unwrap_or_default()),
            runner: use_signal(runner),
            binaries: use_signal(binaries),
            window: use_signal(syngui::window::WindowState::default()),
            theme: use_signal(String::new()),
        };
        // Сохранённые настройки — в общую память до первой страницы: окно открывается на том, что
        // оставил прошлый сеанс (как restoreSettings Qt-окна).
        if let Some(h) = header() {
            config::apply_settings(h, &ini);
            st.bypass.set(h.composition_bypass.load(std::sync::atomic::Ordering::SeqCst) != 0);
            POLL.lock().unwrap().blob = config::settings_blob(h);
        }
        st.poll();
        st
    }

    /// Опрос раз в полсекунды (в окне; в снимке — один раз в `new`).
    pub fn start(self) {
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(500));
            run_on_main_thread(move || self.poll());
        });
    }

    pub fn poll(self) {
        let Some(h) = header() else {
            self.status.set(Status { model_missing: self.model_missing(), ..Status::default() });
            return;
        };
        let mut p = POLL.lock().unwrap();
        let mismatch = (!h.valid()).then(|| h.version.load(std::sync::atomic::Ordering::SeqCst));
        let ld = |a: &std::sync::atomic::AtomicU32| a.load(std::sync::atomic::Ordering::SeqCst);
        let frames = ShmHeader::load64(&h.layer_frames_lo, &h.layer_frames_hi);
        let layer_alive = paths::pid_alive(ld(&h.layer_pid));
        let idle = layer_alive && ld(&h.layer_composition_up) != 0;
        let active = !p.first && idle && frames != p.frames;
        p.frames = frames;
        p.first = false;
        let st = Status {
            attached: true,
            mismatch,
            running: paths::helper_running(),
            helper_state: ld(&h.helper_state),
            reason: h.helper_reason(),
            model_up: ld(&h.model_up) != 0,
            layer: if active { Layer::Active } else if idle { Layer::Idle } else { Layer::None },
            game: if layer_alive { h.game_name() } else { String::new() },
            layer_size: (ld(&h.layer_width), ld(&h.layer_height)),
            layer_reason: if layer_alive { h.layer_reason() } else { String::new() },
            helper_frames: ShmHeader::load64(&h.helper_frames_lo, &h.helper_frames_hi),
            eval_ms: shm::b2f(ld(&h.helper_eval_ms_bits)),
            upload_ms: shm::b2f(ld(&h.helper_upload_ms_bits)),
            readback_ms: shm::b2f(ld(&h.helper_readback_ms_bits)),
            layer_ms: shm::b2f(ld(&h.layer_ms_bits)),
            vram_mb: ld(&h.helper_vram_mb),
            features: ld(&h.helper_features),
            dmabuf: ld(&h.layer_proxy_seq) != 0 && ld(&h.layer_proxy_seq) == ld(&h.proxy_export_seq),
            hdr: ld(&h.hdr_active) != 0,
            model_missing: self.model_missing(),
        };
        if self.status.get_untracked() != st {
            self.status.set(st);
        }
        if mismatch.is_some() {
            return; // в чужую раскладку ничего не сохраняем
        }
        let blob = config::settings_blob(h);
        if blob != p.blob {
            // настройку поменяли не из окна — страницы перечитывают значения
            p.blob = blob;
            p.dirty = true;
            self.bypass.set(ld(&h.composition_bypass) != 0);
            self.rev.update(|r| *r += 1);
        }
        if p.dirty {
            p.dirty = false;
            drop(p);
            self.save();
        }
    }

    /// Запись настройки из окна: в общую память сразу, в config.ini — с ближайшим опросом.
    pub fn write(self, f: Field, raw: u32, latch: bool) {
        let Some(h) = header() else { return };
        if h.get(f) == raw {
            return;
        }
        h.set(f, raw, latch);
        let mut p = POLL.lock().unwrap();
        p.blob = config::settings_blob(h);
        p.dirty = true;
    }

    /// Переопределения прохода: маска и значение, `tuningSeq` (модель читает их при создании функции).
    pub fn write_pass(self, pass: usize, bit: u32, on: bool, field: Option<(crate::shm::PassField, u32)>) {
        use std::sync::atomic::Ordering::SeqCst;
        let Some(h) = header() else { return };
        let pc = &h.pass[pass.min(shm::MAX_PASSES - 1)];
        if let Some((f, raw)) = field {
            f(pc).store(raw, SeqCst);
        }
        let mask = pc.override_mask.load(SeqCst);
        pc.override_mask.store(if on { mask | bit } else { mask & !bit }, SeqCst);
        h.control_seq.fetch_add(1, SeqCst);
        h.tuning_seq.fetch_add(1, SeqCst);
        let mut p = POLL.lock().unwrap();
        p.blob = config::settings_blob(h);
        p.dirty = true;
    }

    /// После массовой перемены (профиль, сброс): страницы перечитываются, файл сохраняется.
    pub fn reloaded(self) {
        if let Some(h) = header() {
            let mut p = POLL.lock().unwrap();
            p.blob = config::settings_blob(h);
            p.dirty = true;
            self.bypass.set(h.composition_bypass.load(std::sync::atomic::Ordering::SeqCst) != 0);
        }
        self.rev.update(|r| *r += 1);
    }

    pub fn save(self) {
        let (rt, rp) = self.runner.get_untracked();
        let profile = self.profile.get_untracked();
        let profile = if profile.is_empty() { String::new() } else { config::profile_path(&profile).display().to_string() };
        config::save_config(header(), &[("runner_type", rt), ("runner_path", rp), ("profile", profile)]);
    }

    pub fn toast(self, text: impl Into<String>) {
        let id = {
            let mut p = POLL.lock().unwrap();
            p.toast_seq += 1;
            p.toast_seq
        };
        self.toast.set(Some((id, text.into())));
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(4500));
            run_on_main_thread(move || {
                if self.toast.get_untracked().map(|(i, _)| i) == Some(id) {
                    self.toast.set(None);
                }
            });
        });
    }

    pub fn binaries_dir(self) -> PathBuf {
        PathBuf::from(self.binaries.get_untracked())
    }

    pub fn model_missing(self) -> bool {
        !self.binaries_dir().join("nvngx_dlssnr.dll").is_file()
    }

    /// `dlssnr-helper start|stop` в отдельном потоке: запуск поднимает префикс Wine и может идти минуту.
    pub fn helper(self, cmd: &'static str) {
        let Some(cli) = paths::helper_cli() else {
            self.toast(t!("Не найдена команда dlssnr-helper"));
            return;
        };
        if let Some(h) = header() {
            h.quit.store(if cmd == "stop" { 1 } else { 0 }, std::sync::atomic::Ordering::SeqCst);
        }
        self.save();
        self.busy.set(Some(if cmd == "stop" { t!("Остановка помощника…") } else { t!("Запуск помощника…") }));
        std::thread::spawn(move || {
            let out = std::process::Command::new(&cli).arg(cmd).output();
            run_on_main_thread(move || {
                self.busy.set(None);
                match out {
                    Ok(o) if o.status.success() => {}
                    Ok(o) => {
                        let err = String::from_utf8_lossy(&o.stderr);
                        let line = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
                        self.toast(t!("dlssnr-helper {c}: {e}", c = cmd, e = line));
                    }
                    Err(e) => self.toast(t!("dlssnr-helper {c}: {e}", c = cmd, e = e)),
                }
                self.poll();
            });
        });
    }

    pub fn restart_helper(self) {
        let Some(cli) = paths::helper_cli() else { return self.toast(t!("Не найдена команда dlssnr-helper")) };
        self.save();
        self.busy.set(Some(t!("Перезапуск помощника…")));
        std::thread::spawn(move || {
            let _ = std::process::Command::new(&cli).arg("stop").output();
            if let Some(h) = header() {
                h.quit.store(0, std::sync::atomic::Ordering::SeqCst);
            }
            let out = std::process::Command::new(&cli).arg("start").output();
            run_on_main_thread(move || {
                self.busy.set(None);
                if !matches!(out, Ok(ref o) if o.status.success()) {
                    self.toast(t!("Помощник не запустился — подробности в журнале"));
                }
                self.poll();
            });
        });
    }

    pub fn set_runner(self, kind: &str, path: &str) {
        self.runner.set((kind.to_string(), path.to_string()));
        self.save();
    }

    /// Копирование DLL NGX из каталога пользователя — как `dlssnr-helper import-binaries`.
    pub fn import(self, src: &str) {
        let src = PathBuf::from(src.trim());
        let dest = paths::default_binaries();
        if !src.is_dir() {
            return self.toast(t!("Каталог не найден: {p}", p = src.display()));
        }
        if let Err(e) = std::fs::create_dir_all(&dest) {
            return self.toast(t!("Не создать {p}: {e}", p = dest.display(), e = e));
        }
        let mut copied = Vec::new();
        let mut failed = Vec::new();
        let mut names: Vec<String> = std::fs::read_dir(&src)
            .map(|rd| rd.flatten().filter(|e| e.path().is_file()).filter_map(|e| e.file_name().into_string().ok()).collect())
            .unwrap_or_default();
        names.sort();
        for name in names {
            let l = name.to_lowercase();
            let wanted = l == "nvngx_dlssnr.dll"
                || l == "nvngx.dll"
                || l == "nvapi64.dll"
                || (l.starts_with("sl.") && l.ends_with(".dll"))
                || l.ends_with(".license.txt");
            if !wanted {
                continue;
            }
            let target = dest.join(&name);
            let tmp = dest.join(format!(".{name}.new"));
            match std::fs::copy(src.join(&name), &tmp).and_then(|_| std::fs::rename(&tmp, &target)) {
                Ok(()) => copied.push(name),
                Err(_) => failed.push(name),
            }
        }
        if copied.is_empty() {
            return self.toast(t!("В каталоге нет DLL NVIDIA NGX — нужен каталог с nvngx_dlssnr.dll"));
        }
        self.binaries.set(dest.display().to_string());
        let mut ini = config::Ini::load();
        ini.set("binaries", dest.display().to_string());
        let _ = ini.save_to(&paths::config_file(), "");
        self.save();
        self.dialog.set(Dialog::None);
        let has_model = dest.join("nvngx_dlssnr.dll").is_file();
        let msg = if !failed.is_empty() {
            t!("Скопировано: {ok}; не удалось: {bad}", ok = copied.join(", "), bad = failed.join(", "))
        } else if !has_model {
            t!("Скопировано: {ok}. nvngx_dlssnr.dll среди них нет — нейрорендеринг выключен", ok = copied.join(", "))
        } else if self.status.get_untracked().running {
            t!("Модель импортирована — перезапустите помощник")
        } else {
            t!("Модель импортирована — запустите помощник")
        };
        self.toast(msg);
        self.poll();
    }

    pub fn reset_all(self) {
        if let Some(h) = header() {
            h.reset_settings();
        }
        self.profile.set(String::new());
        self.reloaded();
        self.toast(t!("Настройки сброшены"));
    }

    pub fn load_profile(self, name: &str) {
        let Some(h) = header() else { return };
        if name.is_empty() {
            h.reset_settings();
        } else if config::load_profile(h, name) == 0 {
            return self.toast(t!("Профиль «{n}» пуст или не читается", n = name));
        }
        self.profile.set(name.to_string());
        self.reloaded();
    }

    pub fn save_profile(self, name: &str) {
        let name = name.trim();
        let Some(h) = header() else { return };
        if name.is_empty() {
            return;
        }
        match config::save_profile(h, name) {
            Ok(()) => {
                self.profiles.set(config::profiles());
                self.profile.set(name.to_string());
                self.save();
                self.dialog.set(Dialog::None);
                self.toast(t!("Профиль «{n}» сохранён", n = name));
            }
            Err(e) => self.toast(t!("Профиль не сохранён: {e}", e = e)),
        }
    }

    pub fn delete_profile(self, name: &str) {
        let _ = std::fs::remove_file(config::profile_path(name));
        self.profiles.set(config::profiles());
        if self.profile.get_untracked() == name {
            self.profile.set(String::new());
            self.save();
        }
        self.dialog.set(Dialog::None);
    }
}

/// `profile=` в config.ini хранит путь к файлу профиля (как Qt-окно) — показывается имя.
fn profile_name(path: &str) -> String {
    std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Открыть каталог или файл в программе по умолчанию.
pub fn open_path(p: &std::path::Path) {
    if p.extension().is_none() {
        let _ = std::fs::create_dir_all(p);
    }
    if let Err(e) = std::process::Command::new("xdg-open").arg(p).spawn() {
        tracing::warn!("xdg-open {}: {e}", p.display());
    }
}

/// Выбор каталога: портал (в synshell — окно Проводника), без него — kdialog или zenity. Блокирующий.
pub fn pick_dir(start: &std::path::Path) -> Option<String> {
    match synsystem::portal_files::pick(&t!("Каталог с DLL NVIDIA NGX"), true, start, &[]) {
        Ok(p) => return p.map(|p| p.display().to_string()),
        Err(e) => tracing::info!("портал выбора файлов: {e} — kdialog/zenity"),
    }
    let start = start.display().to_string();
    let out = if paths::which("kdialog").is_some() {
        std::process::Command::new("kdialog").args(["--getexistingdirectory", &start]).output().ok()?
    } else if paths::which("zenity").is_some() {
        std::process::Command::new("zenity").args(["--file-selection", "--directory", &format!("--filename={start}/")]).output().ok()?
    } else {
        return None;
    };
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}
