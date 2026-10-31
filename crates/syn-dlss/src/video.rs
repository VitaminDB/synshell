//! Кадры видео через помощник DLSS 5 без Vulkan-слоя: приложение (видеоплеер) само кладёт кадр RGBA8
//! в общую память и забирает ответ модели — тот же обмен, что у слоя по общей памяти
//! (`seq_req` → `seq_resp`, `answeredW/H`, `seq_ok`), только без захвата swapchain. Модель видит
//! только кадр видео, без интерфейса плеера; векторы движения и смены сцен помощник считает сам
//! (оптический поток NVIDIA).
//!
//! Пока клиент работает, он подписывается в поля слоя (`layerPid`, кадры, размер, время, имя), и окно
//! syn-dlss показывает плеер как активную программу. Канал один: если в общей памяти живой слой игры,
//! клиент уступает ([`FrameError::Busy`]).

use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::sync::atomic::{fence, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::paths;
use crate::shm::{self, ShmHeader, HEADER_BYTES, MAX_FRAME, MAX_H, MAX_W};

const SC: Ordering = Ordering::SeqCst;

/// Меньше этого помощник кадр не берёт (kMinW/kMinH в shm_protocol.h).
pub const MIN_SIDE: u32 = 64;

/// Почему кадр не обработан — плеер показывает кадр как есть.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// Помощник не запущен или ещё поднимается.
    NoHelper,
    /// Помощник жив, но модель не загрузилась (нет nvngx_dlssnr.dll, нет GPU NVIDIA…).
    ModelFailed(String),
    /// Канал занят слоем игры (её имя).
    Busy(String),
    /// Размер вне пределов помощника.
    BadSize,
    /// Ответ не пришёл вовремя (модель собирается после смены настроек или размера).
    Timeout,
    /// Помощник ответил, но кадр не принял (переключение HDR, пересборка).
    Refused,
}

/// Время одного кадра, мс.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameTiming {
    /// Весь обмен: запись, ожидание ответа, чтение.
    pub total_ms: f32,
    /// Ожидание помощника: загрузка, оптический поток, модель, выгрузка.
    pub helper_ms: f32,
}

struct Map {
    base: *mut u8,
    len: usize,
}

// SAFETY: отображение живёт столько же, сколько клиент; доступ к пикселям — под `Mutex` клиента,
// к заголовку — только атомиками.
unsafe impl Send for Map {}
unsafe impl Sync for Map {}

impl Drop for Map {
    fn drop(&mut self) {
        // SAFETY: base/len — ровно то, что вернул mmap.
        unsafe { libc::munmap(self.base as *mut libc::c_void, self.len) };
    }
}

pub struct VideoClient {
    map: Map,
    name: String,
    pid: u32,
    state: Mutex<ClientState>,
    /// Канал наш (то же, что `ClientState::claimed`, но без ожидания кадра под замком).
    claimed_flag: std::sync::atomic::AtomicBool,
}

#[derive(Default)]
struct ClientState {
    frames: u64,
    claimed: bool,
    /// Разрешение модели из профиля, пока плеер временно его снизил.
    base_scale: Option<u32>,
}

impl VideoClient {
    /// Отобразить общую память целиком (файл разреженный: пиксели занимают место, только пока идут
    /// кадры). `name` — что показать в окне DLSS 5 вместо имени игры.
    pub fn open(name: &str) -> std::io::Result<Self> {
        let path = paths::shm_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        use std::os::unix::fs::OpenOptionsExt;
        let file = OpenOptions::new().read(true).write(true).create(true).mode(0o666).open(&path)?;
        let len = shm::total_bytes();
        if (file.metadata()?.len() as usize) < len {
            file.set_len(len as u64)?;
        }
        // SAFETY: общее отображение файла нужного размера; снимается в Drop.
        let p = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, file.as_raw_fd(), 0)
        };
        if p == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }
        let c = Self {
            map: Map { base: p as *mut u8, len },
            name: name.to_string(),
            // SAFETY: getpid не может завершиться ошибкой.
            pid: unsafe { libc::getpid() } as u32,
            state: Mutex::new(ClientState::default()),
            claimed_flag: std::sync::atomic::AtomicBool::new(false),
        };
        let h = c.header();
        if !h.valid() || h.passes.load(SC) == 0 {
            // общая память новая: настройки — сохранённые окном DLSS 5 (config.ini), как при его запуске
            h.init_defaults();
            crate::config::apply_settings(h, &crate::config::Ini::load());
        }
        // прошлый плеер упал, не вернув настройки игр; игра с профилем ушла без окна DLSS 5
        crate::config::restore_after_video(h);
        crate::config::restore_after_game(h);
        Ok(c)
    }

    pub fn header(&self) -> &ShmHeader {
        // SAFETY: начало отображения выровнено по странице, заголовок — атомики.
        unsafe { &*(self.map.base as *const ShmHeader) }
    }

    /// Готов ли помощник принять кадр (без самого кадра) — для подписи в плеере.
    pub fn ready(&self) -> Result<(), FrameError> {
        let h = self.header();
        if !h.valid() {
            return Err(FrameError::NoHelper);
        }
        if let Some(game) = self.foreign_layer() {
            return Err(FrameError::Busy(game));
        }
        match h.helper_state.load(SC) {
            shm::HELPER_RUNNING if paths::helper_running() => Ok(()),
            shm::HELPER_NO_VULKAN | shm::HELPER_NO_BINARIES | shm::HELPER_MODEL_FAILED => {
                Err(FrameError::ModelFailed(h.helper_reason()))
            }
            _ => Err(FrameError::NoHelper),
        }
    }

    /// Живой слой другой программы в канале — её имя.
    fn foreign_layer(&self) -> Option<String> {
        let h = self.header();
        let pid = h.layer_pid.load(SC);
        (pid != 0 && pid != self.pid && paths::pid_alive(pid)).then(|| {
            let n = h.game_name();
            if n.is_empty() { format!("pid {pid}") } else { n }
        })
    }

    /// Обработать кадр RGBA8 `w×h`: `answer` получает ответ модели (тот же размер) прямо из общей
    /// памяти — без лишней копии. `timeout` — сколько ждать ответа; первый кадр нового размера ждёт
    /// сборки модели (~0,1–0,3 с на проход).
    pub fn process(
        &self,
        w: u32,
        h: u32,
        rgba: &[u8],
        timeout: Duration,
        answer_to: impl FnOnce(&[u8]),
    ) -> Result<FrameTiming, FrameError> {
        let bytes = w as usize * h as usize * 4;
        if w < MIN_SIDE || h < MIN_SIDE || w as usize > MAX_W || h as usize > MAX_H || rgba.len() < bytes {
            return Err(FrameError::BadSize);
        }
        self.ready()?;
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let hd = self.header();
        let t0 = Instant::now();

        // SAFETY: области пикселей внутри отображения: HEADER_BYTES + MAX_FRAME * 2 = len,
        // bytes <= MAX_W * MAX_H * 4 < MAX_FRAME.
        let (input, answer) = unsafe {
            (
                std::slice::from_raw_parts_mut(self.map.base.add(HEADER_BYTES), bytes),
                std::slice::from_raw_parts(self.map.base.add(HEADER_BYTES + MAX_FRAME), bytes),
            )
        };
        input.copy_from_slice(&rgba[..bytes]);
        hd.width.store(w, SC);
        hd.height.store(h, SC);
        // 8 бит, дисплейный сигнал, обмен — общей памятью (не dma-buf)
        hd.hdr_detected.store(0, SC);
        hd.hdr_active.store(0, SC);
        hd.hdr_encode.store(0, SC);
        hd.layer_proxy_seq.store(0, SC);
        hd.layer_answer_seq.store(0, SC);
        if !st.claimed {
            st.claimed = true;
            hd.layer_pid.store(self.pid, SC);
            hd.layer_composition_up.store(1, SC);
            hd.layer_format.store(37, SC); // VK_FORMAT_R8G8B8A8_UNORM
            store_string(&hd.game_name_seq, &hd.game_name, &self.name);
            self.enter_video_profile();
            self.claimed_flag.store(true, SC);
        }
        hd.layer_width.store(w, SC);
        hd.layer_height.store(h, SC);
        fence(Ordering::Release);
        let req = hd.seq_req.fetch_add(1, SC).wrapping_add(1);
        let t_sent = Instant::now();

        // Ждём ответ: короткий опрос, затем сон по 0,2 мс — модель отвечает за миллисекунды.
        let deadline = t0 + timeout;
        let mut spins = 0u32;
        loop {
            let resp = hd.seq_resp.load(SC);
            if resp == req || (resp.wrapping_sub(req) as i32) > 0 {
                break;
            }
            if Instant::now() >= deadline {
                return Err(FrameError::Timeout);
            }
            spins += 1;
            if spins < 200 {
                std::hint::spin_loop();
            } else {
                std::thread::sleep(Duration::from_micros(200));
            }
        }
        fence(Ordering::Acquire);
        let helper_ms = t_sent.elapsed().as_secs_f32() * 1000.0;
        if hd.seq_ok.load(SC) != req || hd.answered_w.load(SC) != w || hd.answered_h.load(SC) != h {
            return Err(FrameError::Refused);
        }
        answer_to(answer);

        st.frames += 1;
        let lo = st.frames as u32;
        let hi = (st.frames >> 32) as u32;
        hd.layer_frames_hi.store(hi, SC);
        hd.layer_frames_lo.store(lo, SC);
        hd.layer_heartbeat.fetch_add(1, SC);
        let total_ms = t0.elapsed().as_secs_f32() * 1000.0;
        hd.layer_ms_bits.store(shm::f2b(total_ms), SC);
        Ok(FrameTiming { total_ms, helper_ms })
    }

    /// Снять подпись слоя (плеер выключил DLSS 5 или закрылся): видеопрофиль — в video.ini,
    /// в общую память — настройки игр.
    pub fn release(&self) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.claimed {
            st.claimed = false;
            let h = self.header();
            self.claimed_flag.store(false, SC);
            self.leave_video_profile(st.base_scale.take());
            if h.layer_pid.compare_exchange(self.pid, 0, SC, SC).is_ok() {
                h.layer_composition_up.store(0, SC);
                store_string(&h.game_name_seq, &h.game_name, "");
            }
        }
    }

    /// Канал наш: метка видеоплеера (окно DLSS 5 сохраняет настройки в видеопрофиль) и видеопрофиль
    /// в общую память. Пересборка модели — только если он отличается от того, что там уже стоит.
    fn enter_video_profile(&self) {
        if let Err(e) = std::fs::write(paths::video_lock(), self.pid.to_string()) {
            tracing::warn!("метка видеоплеера DLSS 5: {e}");
        }
        crate::config::apply_if_changed(self.header(), &crate::config::video_profile());
    }

    fn leave_video_profile(&self, base_scale: Option<u32>) {
        let lock = paths::video_lock();
        let ours = std::fs::read_to_string(&lock).ok().and_then(|s| s.trim().parse::<u32>().ok()) == Some(self.pid);
        if !ours {
            return;
        }
        let h = self.header();
        if let Some(raw) = base_scale {
            // разрешение модели снижали, чтобы успевать — в профиль идёт выбранное пользователем
            h.working_scale_bits.store(raw, SC);
        }
        crate::config::save_video_profile(h);
        let _ = std::fs::remove_file(&lock);
        crate::config::apply_if_changed(h, &crate::config::Ini::load());
    }

    /// Держит ли плеер канал (настройки модели сейчас — видеопрофиль в общей памяти).
    pub fn claimed(&self) -> bool {
        self.claimed_flag.load(SC)
    }

    /// Поменять значения видеопрофиля: в video.ini и, пока канал наш, сразу в общую память.
    /// `latch` — значение модель читает при сборке функции (стиль, проходы).
    pub fn set_profile(&self, pairs: &[(&str, String)], latch: bool) {
        crate::config::set_video_values(pairs);
        if self.claimed() {
            let ini = crate::config::Ini { entries: pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect() };
            crate::config::apply_pairs(self.header(), &ini, latch);
        }
    }

    /// Разрешение модели на ходу, без записи в профиль: плеер снижает его (`factor` — доля от
    /// выбранного в профиле), когда модель не успевает за роликом. `None` — вернуть выбранное.
    pub fn set_auto_scale(&self, factor: Option<f32>) {
        let Ok(mut st) = self.state.lock() else { return };
        if !st.claimed {
            return;
        }
        let h = self.header();
        match factor {
            Some(f) => {
                let base = *st.base_scale.get_or_insert_with(|| h.working_scale_bits.load(SC));
                // меньше 25% помощник не берёт
                h.working_scale_bits.store(shm::f2b((shm::b2f(base) * f).max(0.25)), SC);
            }
            None => {
                if let Some(raw) = st.base_scale.take() {
                    h.working_scale_bits.store(raw, SC);
                }
            }
        }
        h.control_seq.fetch_add(1, SC);
    }
}

impl Drop for VideoClient {
    fn drop(&mut self) {
        self.release();
    }
}

fn store_string(seq: &AtomicU32, dst: &[std::sync::atomic::AtomicU8], s: &str) {
    let b = s.as_bytes();
    let n = b.len().min(dst.len() - 1);
    for (i, d) in dst.iter().enumerate() {
        d.store(if i < n { b[i] } else { 0 }, SC);
    }
    seq.fetch_add(1, SC);
}

/// Запустить помощник (`dlssnr-helper start`) в фоне; `Err` — команды нет.
pub fn start_helper() -> Result<(), String> {
    let cli = paths::helper_cli().ok_or_else(|| "dlssnr-helper не найден".to_string())?;
    std::process::Command::new(cli)
        .arg("start")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
