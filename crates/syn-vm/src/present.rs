//! Слой кадров игр поверх экрана гостя: сервер паравиртуальной видеокарты `syn-gpu`
//! (проект syn-virt, docs/present.md, версия 1) отдаёт кадры swapchain из BAR2
//! (memfd хоста), syn-vm рисует их поверх кадра VNC в месте окна игры.
//!
//! Сокет `$XDG_RUNTIME_DIR/syn-gpu/<машина>.present.sock`; нет сокета — слоя нет, всё как
//! раньше. Сообщения: `u32 type, u32 len`, полезная нагрузка (little-endian, repr(C));
//! дескриптор BAR2 — SCM_RIGHTS при HELLO.
//!
//! Версия 2 (без копий): если окно рисует на той же видеокарте, что и syn-gpu, клиент шлёт
//! CAPS с UUID карты; сервер отдаёт кольцо изображений Vulkan (IMAGE + fd OPAQUE_FD), а FRAME с
//! флагом GPU_IMAGE называет номер изображения. syngui переписывает кадр в текстуру на GPU
//! (`LiveFrame::set_external`), RELEASE уходит, когда GPU дочитал. Импорт не удался — CAPS с
//! нулями, и сервер возвращается к BAR2.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use syngui::widgets::LiveFrame;

const HELLO: u32 = 1;
const FRAME: u32 = 2;
const GONE: u32 = 3;
/// Сервер → клиент: аппаратный курсор гостя (`Cursor`, пиксели BGRA в BAR2).
const CURSOR: u32 = 5;
/// Сервер → клиент: сообщение пользователю (`Notice { kind, _pad, bytes, free }`).
const NOTICE: u32 = 6;
/// Notice.kind: GPU хоста отказал гостю в видеопамяти.
pub const OUT_OF_VIDEO_MEMORY: u32 = 1;

/// Сообщение сервера syn-gpu пользователю.
#[derive(Clone, Copy, Debug)]
pub struct Notice {
    pub kind: u32,
    pub bytes: u64,
    pub free: u64,
}
/// Сервер → клиент: режимы экрана гостя (`Modes`: текущий, флаги, список на выбор).
const MODES: u32 = 7;
/// Клиент → сервер: выбор режима из списка (`ModeEntry`).
const SET_MODE: u32 = 21;
const IMAGE: u32 = 4;
const RELEASE: u32 = 16;
const CAPS: u32 = 18;
/// Клиент → сервер: желаемое разрешение экрана гостя (`Screen { width, height }`), «Разрешение под окно».
const MODE: u32 = 19;
/// Клиент → сервер: частота монитора окна в мГц (`Refresh { millihertz, _pad }`) — VSync гостя от хоста.
const REFRESH: u32 = 20;
/// FRAME.flags: кадр в изображении Vulkan (offset — его номер).
const GPU_IMAGE: u32 = 1 << 2;
/// FRAME.flags: рабочий стол гостя (DWM на карте syn-gpu, сервер с `--display`).
const DESKTOP: u32 = 1 << 3;
/// DRM fourcc: байты B,G,R,A (`AR24` — с альфой, `XR24` — без).
const AR24: u32 = u32::from_le_bytes(*b"AR24");
const XR24: u32 = u32::from_le_bytes(*b"XR24");

/// Поверхность игры: кадр и место окна на экране гостя.
#[derive(Clone)]
pub struct Layer {
    pub frame: Arc<LiveFrame>,
    /// x, y, w, h — физические пиксели экрана гостя.
    pub rect: (i32, i32, u32, u32),
    /// Окно игры видимо и активно (flags бит 0).
    pub show: bool,
    pub fullscreen: bool,
    /// Рабочий стол гостя (FRAME.flags DESKTOP), а не окно игры.
    pub desktop: bool,
    updated: Instant,
}

/// Клиент слоя одной машины.
pub struct Present {
    layers: Mutex<HashMap<u64, Layer>>,
    alive: AtomicBool,
    /// Желаемое разрешение (окно) — уходит сразу и после каждого (пере)подключения.
    mode: Mutex<Option<(u32, u32)>>,
    refresh: Mutex<Option<u32>>,
    tx: Mutex<Option<Arc<Mutex<UnixStream>>>>,
    /// Номер своего указателя syngui для курсора гостя (0 — не ставить: проверка без окна).
    cursor_id: u32,
    on_notice: Option<Arc<dyn Fn(Notice) + Send + Sync>>,
    /// Когда клиент начал ждать кадры.
    started: Instant,
    /// Машина (пусто — проверка без машины): размер рабочего стола уходит в файл для ввода из CLI.
    id: String,
    desk_size: Mutex<(u32, u32)>,
    /// Курсор гостя для записи экрана: левый верхний угол на экране гостя и картинка RGBA.
    cursor: Mutex<Option<GuestCursor>>,
    /// Режимы экрана гостя (последний MODES).
    modes: Mutex<Option<GuestModes>>,
}

/// Курсор гостя: где он на экране и как выглядит (для записи — в кадре его нет).
#[derive(Clone)]
pub struct GuestCursor {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

/// Вид режима экрана (`ModeEntry.kind`, syn-virt proto `mode_kind`).
pub mod mode_kind {
    /// Текст: колонки×строки.
    pub const TEXT: u16 = 1;
    /// Графика: точки и бит на точку.
    pub const GRAPHICS: u16 = 2;
    /// Под окно: DOS — текст размером с окно, Windows — разрешение следует за окном.
    pub const WINDOW: u16 = 3;
    /// Разрешение 3D (Glide): `width` 0 — по окну, 1 — как в игре, 2 — ×2, 3 — ×3; `bpp` = 1 у выбранного.
    pub const RENDER: u16 = 4;
}

/// Флаги `Modes.flags` (syn-virt proto `modes_flags`).
pub mod modes_flags {
    /// Экран у VGA/VBE (DOS): список — текстовые режимы.
    pub const DOS: u32 = 1 << 0;
    /// В DOS загружен SYNMOUSE — выбор применится; без него список только для сведения.
    pub const AGENT: u32 = 1 << 1;
    /// Рабочий стол Windows: список — разрешения.
    pub const DESKTOP: u32 = 1 << 2;
    /// Разрешение Windows задано вручную (не под окно).
    pub const FIXED: u32 = 1 << 3;
    /// Выбор ещё не применён (DOS: ждёт командной строки).
    pub const PENDING: u32 = 1 << 4;
    /// Идёт 3D (Glide): `GuestModes::game` и `render` — разрешение игры и кадра.
    pub const GLIDE: u32 = 1 << 5;
}

/// Выбранное у машины разрешение 3D (vm.toml `glide_res`): сервер syn-gpu его не помнит между запусками —
/// окно досылает его, когда в `MODES` выбран другой вариант.
static RENDER_PREF: Mutex<Option<HashMap<String, u16>>> = Mutex::new(None);

pub fn set_render_pref(id: &str, k: u16) {
    RENDER_PREF.lock().unwrap().get_or_insert_with(HashMap::new).insert(id.to_string(), k);
}

fn render_pref(id: &str) -> Option<u16> {
    RENDER_PREF.lock().unwrap().as_ref().and_then(|m| m.get(id).copied())
}

/// Режим экрана гостя (`ModeEntry`: kind, _pad, width, height, bpp, _pad2 — по u16).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModeEntry {
    pub kind: u16,
    pub width: u16,
    pub height: u16,
    pub bpp: u16,
}

impl ModeEntry {
    const SIZE: usize = 12;

    fn parse(b: &[u8]) -> Self {
        let u = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        Self { kind: u(0), width: u(4), height: u(6), bpp: u(8) }
    }

    fn bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..2].copy_from_slice(&self.kind.to_le_bytes());
        b[4..6].copy_from_slice(&self.width.to_le_bytes());
        b[6..8].copy_from_slice(&self.height.to_le_bytes());
        b[8..10].copy_from_slice(&self.bpp.to_le_bytes());
        b
    }
}

/// Режимы экрана гостя из `MODES`: что сейчас, флаги, варианты на выбор.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuestModes {
    pub current: ModeEntry,
    pub flags: u32,
    pub list: Vec<ModeEntry>,
    /// При флаге GLIDE: разрешение игры и кадра хоста (поля после списка, сервер с 2026-11-12).
    pub game: (u32, u32),
    pub render: (u32, u32),
}

impl GuestModes {
    /// `Modes { current, flags, count, list[24] }`.
    fn parse(b: &[u8]) -> Option<Self> {
        let n = ModeEntry::SIZE;
        if b.len() < n + 8 {
            return None;
        }
        let current = ModeEntry::parse(&b[..n]);
        let flags = u32_at(b, n);
        let count = (u32_at(b, n + 4) as usize).min(24);
        let list = (0..count).map(|i| n + 8 + i * n).take_while(|&o| o + n <= b.len()).map(|o| ModeEntry::parse(&b[o..o + n])).collect();
        let tail = n + 8 + 24 * n;
        let (game, render) = if b.len() >= tail + 16 {
            ((u32_at(b, tail), u32_at(b, tail + 4)), (u32_at(b, tail + 8), u32_at(b, tail + 12)))
        } else {
            ((0, 0), (0, 0))
        };
        Some(Self { current, flags, list, game, render })
    }

    pub fn has(&self, f: u32) -> bool {
        self.flags & f != 0
    }
}

/// Номер указателя syngui (`CursorIcon::Custom`) для курсора гостя машины.
pub fn cursor_id(id: &str) -> u32 {
    let h = id.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    0x5356_0000 | (h & 0xffff)
}

impl Present {
    /// Слой, который показывать: последняя обновлённая видимая поверхность.
    pub fn visible(&self) -> Option<Layer> {
        self.layers.lock().unwrap().values().filter(|l| l.show).max_by_key(|l| l.updated).cloned()
    }

    /// Видимое окно игры (не рабочий стол).
    pub fn game(&self) -> Option<Layer> {
        self.layers.lock().unwrap().values().filter(|l| l.show && !l.desktop).max_by_key(|l| l.updated).cloned()
    }

    /// Рабочий стол гостя — последний кадр DESKTOP.
    pub fn desktop(&self) -> Option<Layer> {
        self.layers.lock().unwrap().values().filter(|l| l.desktop).max_by_key(|l| l.updated).cloned()
    }

    /// Сколько клиент работает (ждёт кадры).
    pub fn age(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn close(&self) {
        self.alive.store(false, Ordering::Relaxed);
    }

    /// Курсор гостя (видимый), если сервер его прислал.
    pub fn cursor(&self) -> Option<GuestCursor> {
        self.cursor.lock().unwrap().clone()
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// Частота монитора окна (мГц): по ней сервер отсчитывает VSync и кадры гостя.
    pub fn request_refresh(&self, mhz: u32) {
        if *self.refresh.lock().unwrap() == Some(mhz) {
            return;
        }
        *self.refresh.lock().unwrap() = Some(mhz);
        if let Some(tx) = self.tx.lock().unwrap().clone() {
            if send(&mut tx.lock().unwrap(), REFRESH, &[mhz.to_le_bytes(), 0u32.to_le_bytes()].concat()).is_ok() {
                tracing::info!("syn-gpu present: частота монитора {:.3} Гц", mhz as f64 / 1000.0);
            }
        }
    }

    /// Режимы экрана гостя (последний `MODES` сервера; старый сервер — `None`).
    pub fn modes(&self) -> Option<GuestModes> {
        self.modes.lock().unwrap().clone()
    }

    /// Выбрать режим из списка `MODES` (сервер ответит новым `MODES`).
    pub fn set_mode(&self, e: ModeEntry) -> Result<()> {
        let tx = self.tx.lock().unwrap().clone().context("нет связи с сервером syn-gpu")?;
        send(&mut tx.lock().unwrap(), SET_MODE, &e.bytes())?;
        tracing::info!("syn-gpu present: выбран режим {e:?}");
        Ok(())
    }

    /// Разрешение экрана гостя под окно (физические пиксели).
    pub fn request_mode(&self, w: u32, h: u32) {
        if *self.mode.lock().unwrap() == Some((w, h)) {
            return;
        }
        *self.mode.lock().unwrap() = Some((w, h));
        if let Some(tx) = self.tx.lock().unwrap().clone() {
            let b = [w.to_le_bytes(), h.to_le_bytes()].concat();
            if send(&mut tx.lock().unwrap(), MODE, &b).is_ok() {
                tracing::info!("syn-gpu present: разрешение под окно {w}×{h}");
            }
        }
    }
}

/// Сервер syn-gpu машины работает: жив процесс из `<машина>.pid` (сокеты после падения сервера
/// остаются файлами). Без файла pid — по сокету кадров.
pub fn server_up(id: &str) -> bool {
    let pidf = socket(id).with_file_name(format!("{id}.pid"));
    match std::fs::read_to_string(&pidf).ok().and_then(|s| s.trim().parse::<i32>().ok()) {
        Some(pid) => unsafe { libc::kill(pid, 0) == 0 },
        None => socket(id).exists(),
    }
}

/// Размер рабочего стола syn-gpu (последний известный окну или проверке) — для ввода из CLI.
fn size_file(id: &str) -> PathBuf {
    socket(id).with_file_name(format!("{id}.desktop-size"))
}

/// Последний известный размер рабочего стола syn-gpu машины (физические пиксели).
pub fn desktop_size(id: &str) -> Option<(u32, u32)> {
    let s = std::fs::read_to_string(size_file(id)).ok()?;
    let mut it = s.split_whitespace().filter_map(|x| x.parse::<u32>().ok());
    Some((it.next()?, it.next()?)).filter(|(w, h)| *w > 0 && *h > 0)
}

impl Present {
    /// Кадр рабочего стола: размер изменился — записать (для `--click` без окна).
    fn note_desktop(&self, w: u32, h: u32) {
        if self.id.is_empty() || w == 0 || h == 0 {
            return;
        }
        let mut d = self.desk_size.lock().unwrap();
        if *d != (w, h) {
            *d = (w, h);
            let _ = std::fs::write(size_file(&self.id), format!("{w} {h}\n"));
        }
    }
}

fn socket(id: &str) -> PathBuf {
    let run = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    run.join("syn-gpu").join(format!("{id}.present.sock"))
}

/// Сокет наблюдателей (`<машина>.present-view.sock`): рабочий стол и курсор, окно не вытесняет и
/// режим гостя не трогает. Старый сервер без него — основной сокет.
fn view_socket(id: &str) -> PathBuf {
    let v = socket(id).with_file_name(format!("{id}.present-view.sock"));
    if v.exists() { v } else { socket(id) }
}

/// Запустить клиента: ждёт сокет, переподключается; `on_change` — новый кадр или слой.
pub fn start(id: &str, on_change: Arc<dyn Fn() + Send + Sync>, on_notice: Arc<dyn Fn(Notice) + Send + Sync>) -> Arc<Present> {
    let p = Arc::new(Present { layers: Mutex::new(HashMap::new()), alive: AtomicBool::new(true), mode: Mutex::new(None), refresh: Mutex::new(None), tx: Mutex::new(None), cursor_id: cursor_id(id), on_notice: Some(on_notice), started: Instant::now(), id: id.to_string(), desk_size: Mutex::new((0, 0)), cursor: Mutex::new(None), modes: Mutex::new(None) });
    let me = p.clone();
    let path = socket(id);
    std::thread::Builder::new()
        .name("syn-gpu-present".into())
        .spawn(move || {
            while me.alive.load(Ordering::Relaxed) {
                if path.exists() {
                    match UnixStream::connect(&path) {
                        Ok(s) => {
                            // Разрыв (другой клиент вытеснил нас, сервер перезапускается) — последний
                            // кадр остаётся: неподвижный экран (DOS, текстовый режим) нового кадра
                            // не шлёт, и без него окно стояло с заглушкой «Ждём драйвер», а ввод
                            // (он идёт через LiveView кадра) пропадал.
                            let t0 = std::time::Instant::now();
                            if let Err(e) = run(&me, s, &on_change, true) {
                                tracing::debug!("syn-gpu present: {e}");
                            }
                            // Разорвали почти сразу — экран забрал другой клиент (второе окно,
                            // --present-dump): не отбирать его обратно каждую секунду.
                            if t0.elapsed() < Duration::from_secs(3) {
                                std::thread::sleep(Duration::from_secs(5));
                            }
                        }
                        Err(e) => {
                            tracing::debug!("syn-gpu present {}: {e}", path.display());
                            // Сокет остался, а сервера нет (упал) — кадры убрать: окно покажет, что случилось.
                            if e.kind() == std::io::ErrorKind::ConnectionRefused && !me.layers.lock().unwrap().is_empty() {
                                me.layers.lock().unwrap().clear();
                                on_change();
                            }
                        }
                    }
                } else if me.modes.lock().unwrap().take().is_some() || !me.layers.lock().unwrap().is_empty() {
                    // Сервера syn-gpu больше нет (машина выключена) — кадры убрать.
                    me.layers.lock().unwrap().clear();
                    on_change();
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .ok();
    p
}

/// Наблюдатель без окна (запись экрана): кадры рабочего стола и курсор через сокет наблюдателей —
/// окно не вытесняет и режим гостя не трогает. Закрыть — [`Present::close`].
pub fn viewer(id: &str) -> Result<Arc<Present>> {
    let path = view_socket(id);
    let s = UnixStream::connect(&path).with_context(|| path.display().to_string())?;
    let p = Arc::new(Present { layers: Mutex::new(HashMap::new()), alive: AtomicBool::new(true), mode: Mutex::new(None), refresh: Mutex::new(None), tx: Mutex::new(None), cursor_id: 0, on_notice: None, started: Instant::now(), id: String::new(), desk_size: Mutex::new((0, 0)), cursor: Mutex::new(None), modes: Mutex::new(None) });
    let p2 = p.clone();
    std::thread::spawn(move || {
        let cb: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let _ = run(&p2, s, &cb, false);
        p2.close();
    });
    Ok(p)
}

/// Экран без окна: рабочий стол и поверх — видимые поверхности (окна игр, всплывающие окна,
/// которые гость выводит отдельными плоскостями) в их местах. Кадр RGBA.
pub fn grab_screen(id: &str, wait: Duration) -> Result<(u32, u32, Vec<u8>)> {
    // Наблюдателем: снимок не забирает у открытого окна главный поток и режим гостя. Наблюдателю
    // приходят рабочий стол и курсор; кадры игр — `grab(…, false)` (основной сокет).
    let path = view_socket(id);
    let s = UnixStream::connect(&path).with_context(|| path.display().to_string())?;
    let p = Arc::new(Present { layers: Mutex::new(HashMap::new()), alive: AtomicBool::new(true), mode: Mutex::new(None), refresh: Mutex::new(None), tx: Mutex::new(None), cursor_id: 0, on_notice: None, started: Instant::now(), id: id.to_string(), desk_size: Mutex::new((0, 0)), cursor: Mutex::new(None), modes: Mutex::new(None) });
    let p2 = p.clone();
    let t = std::thread::spawn(move || {
        let cb: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        run(&p2, s, &cb, false)
    });
    let until = Instant::now() + wait;
    while p.desktop().is_none() && !t.is_finished() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    // Прочие поверхности приходят следом за рабочим столом.
    std::thread::sleep(Duration::from_millis(400));
    p.close();
    let _ = t.join();
    let mut layers: Vec<Layer> = p.layers.lock().unwrap().values().cloned().collect();
    let desk = layers.iter().filter(|l| l.desktop).max_by_key(|l| l.updated).cloned().with_context(|| syngui::t!("нет кадра рабочего стола (драйвер syn-gpu в госте ещё не загрузился?)"))?;
    let (w, h, px, _) = desk.frame.latest();
    let mut out = px.to_vec();
    layers.retain(|l| !l.desktop && l.show);
    layers.sort_by_key(|l| l.updated);
    for l in layers {
        let (lw, lh, lpx, _) = l.frame.latest();
        let (x0, y0, rw, rh) = l.rect;
        if lw == 0 || lh == 0 || rw == 0 || rh == 0 {
            continue;
        }
        // Кадр поверхности — в прямоугольник окна (ближайший пиксель, без смешивания).
        for y in 0..rh {
            let dy = y0 + y as i32;
            if dy < 0 || dy >= h as i32 {
                continue;
            }
            let sy = (y as u64 * lh as u64 / rh as u64) as usize;
            for x in 0..rw {
                let dx = x0 + x as i32;
                if dx < 0 || dx >= w as i32 {
                    continue;
                }
                let sx = (x as u64 * lw as u64 / rw as u64) as usize;
                let si = (sy * lw as usize + sx) * 4;
                let di = (dy as usize * w as usize + dx as usize) * 4;
                if let (Some(src), true) = (lpx.get(si..si + 4), di + 4 <= out.len()) {
                    out[di..di + 4].copy_from_slice(src);
                }
            }
        }
    }
    Ok((w, h, out))
}

/// Проверка без окна: подключиться, дождаться видимого кадра игры (`desktop` — кадра рабочего
/// стола) до `wait`, вернуть слой.
pub fn grab(id: &str, wait: Duration, desktop: bool) -> Result<Layer> {
    // Кадр рабочего стола — наблюдателем (окно не вытесняется); кадры игр приходят только главному.
    let path = if desktop { view_socket(id) } else { socket(id) };
    let s = UnixStream::connect(&path).with_context(|| path.display().to_string())?;
    let p = Arc::new(Present { layers: Mutex::new(HashMap::new()), alive: AtomicBool::new(true), mode: Mutex::new(None), refresh: Mutex::new(None), tx: Mutex::new(None), cursor_id: 0, on_notice: None, started: Instant::now(), id: id.to_string(), desk_size: Mutex::new((0, 0)), cursor: Mutex::new(None), modes: Mutex::new(None) });
    let p2 = p.clone();
    let t = std::thread::spawn(move || {
        let cb: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        run(&p2, s, &cb, false)
    });
    let until = Instant::now() + wait;
    let layer = loop {
        if let Some(l) = if desktop { p.desktop() } else { p.visible() } {
            break Some(l);
        }
        if t.is_finished() || Instant::now() > until {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    p.close();
    let _ = t.join();
    if desktop {
        return layer.with_context(|| syngui::t!("нет кадра рабочего стола (драйвер syn-gpu в госте ещё не загрузился?)"));
    }
    layer.with_context(|| syngui::t!("нет видимого кадра игры (окно игры свёрнуто или не активно?)"))
}

/// Отображение BAR2 только для чтения.
struct Heap {
    ptr: *const u8,
    len: usize,
}

unsafe impl Send for Heap {}

impl Drop for Heap {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.ptr as *mut _, self.len) };
    }
}

/// Номера импортированных изображений — свои у каждого (по ним syngui кэширует импорт).
static IMAGE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// `gpu` — окно (может читать кадры из изображений GPU); проверка без окна — только BAR2.
fn run(p: &Present, mut s: UnixStream, on_change: &Arc<dyn Fn() + Send + Sync>, gpu: bool) -> Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(1)))?;
    let mut heap: Option<Heap> = None;
    // Запись RELEASE из потока GPU syngui (кадр дочитан) — отдельной копией сокета.
    let tx = Arc::new(Mutex::new(s.try_clone()?));
    *p.tx.lock().unwrap() = Some(tx.clone());
    if let Some((w, h)) = *p.mode.lock().unwrap() {
        send(&mut tx.lock().unwrap(), MODE, &[w.to_le_bytes(), h.to_le_bytes()].concat())?;
    }
    if let Some(mhz) = *p.refresh.lock().unwrap() {
        send(&mut tx.lock().unwrap(), REFRESH, &[mhz.to_le_bytes(), 0u32.to_le_bytes()].concat())?;
    }
    // CAPS уже отправлен (UUID карты окна известен только после создания GPU окна).
    let mut caps_sent = !gpu;
    // Импорт не удался — до конца соединения кадры через BAR2.
    let gpu_failed = Arc::new(AtomicBool::new(false));
    let mut images: HashMap<(u64, u32), Arc<syngui::gpu::external::ExternalImage>> = HashMap::new();
    // Последний поставленный курсор: (seq, видим).
    let mut cursor_state: Option<(u32, bool)> = None;
    while p.alive.load(Ordering::Relaxed) {
        if !caps_sent {
            if let Some((dev, drv)) = syngui::gpu::external::device_uuids() {
                let mut b = Vec::with_capacity(40);
                b.extend_from_slice(&2u32.to_le_bytes());
                b.extend_from_slice(&0u32.to_le_bytes());
                b.extend_from_slice(&dev);
                b.extend_from_slice(&drv);
                send(&mut tx.lock().unwrap(), CAPS, &b)?;
                caps_sent = true;
                tracing::info!("syn-gpu present: окно на видеокарте с UUID {:02x?} — прошу кадры без копий", &dev[..4]);
            }
        }
        let (ty, payload, fd) = match read_msg(&mut s) {
            Ok(m) => m,
            Err(e) if is_timeout(&e) => continue,
            Err(e) => return Err(e),
        };
        match ty {
            HELLO => {
                let fd = fd.context("HELLO без дескриптора BAR2")?;
                let version = u32_at(&payload, 0);
                let size = u64_at(&payload, 8) as usize;
                if version != 1 {
                    unsafe { libc::close(fd) };
                    bail!("неизвестная версия протокола present: {version}");
                }
                let ptr = unsafe { libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ, libc::MAP_SHARED, fd, 0) };
                unsafe { libc::close(fd) };
                if ptr == libc::MAP_FAILED {
                    bail!("mmap BAR2: {}", std::io::Error::last_os_error());
                }
                heap = Some(Heap { ptr: ptr as *const u8, len: size });
            }
            FRAME => {
                let surface = u64_at(&payload, 0);
                let seq = u64_at(&payload, 8);
                let offset = u64_at(&payload, 16) as usize;
                let (width, height, stride, format) = (u32_at(&payload, 24), u32_at(&payload, 28), u32_at(&payload, 32), u32_at(&payload, 36));
                let (x, y) = (u32_at(&payload, 40) as i32, u32_at(&payload, 44) as i32);
                let (w, h, flags) = (u32_at(&payload, 48), u32_at(&payload, 52), u32_at(&payload, 56));
                let release = [surface.to_le_bytes(), seq.to_le_bytes()].concat();
                if flags & GPU_IMAGE != 0 {
                    let Some(img) = images.get(&(surface, offset as u32)).cloned() else {
                        send(&mut tx.lock().unwrap(), RELEASE, &release)?;
                        continue;
                    };
                    let (tx2, failed) = (tx.clone(), gpu_failed.clone());
                    // Буфер свободен, когда GPU окна дочитал кадр (или кадр заменён, не показавшись).
                    let ext = syngui::gpu::external::ExternalFrame::new(img, move |ok| {
                        let mut t = tx2.lock().unwrap();
                        let _ = send(&mut t, RELEASE, &release);
                        if !ok && !failed.swap(true, Ordering::Relaxed) {
                            tracing::warn!("syn-gpu present: импорт изображения GPU не удался — кадры через BAR2");
                            let _ = send(&mut t, CAPS, &[2u32.to_le_bytes(), 0u32.to_le_bytes()].concat().into_iter().chain([0u8; 32]).collect::<Vec<u8>>());
                        }
                    });
                    let mut layers = p.layers.lock().unwrap();
                    let l = layers.entry(surface).or_insert_with(|| Layer {
                        frame: LiveFrame::new(),
                        rect: (0, 0, 0, 0),
                        show: false,
                        fullscreen: false,
                        desktop: false,
                        updated: Instant::now(),
                    });
                    l.frame.set_external(ext);
                    l.rect = (x, y, w, h);
                    l.show = flags & 1 != 0;
                    l.fullscreen = flags & 2 != 0;
                    l.desktop = flags & DESKTOP != 0;
                    l.updated = Instant::now();
                    drop(layers);
                    if flags & DESKTOP != 0 {
                        p.note_desktop(w, h);
                    }
                    on_change();
                    continue;
                }
                let pixels = heap.as_ref().and_then(|hp| copy_frame(hp, offset, width, height, stride, format));
                // Кадр прочитан (или негоден) — буфер свободен.
                send(&mut tx.lock().unwrap(), RELEASE, &release)?;
                if let Some(px) = pixels {
                    let mut layers = p.layers.lock().unwrap();
                    let l = layers.entry(surface).or_insert_with(|| Layer {
                        frame: LiveFrame::new(),
                        rect: (0, 0, 0, 0),
                        show: false,
                        fullscreen: false,
                        desktop: false,
                        updated: Instant::now(),
                    });
                    l.frame.set(width, height, px.into());
                    l.rect = (x, y, w, h);
                    l.show = flags & 1 != 0;
                    l.fullscreen = flags & 2 != 0;
                    l.desktop = flags & DESKTOP != 0;
                    l.updated = Instant::now();
                    drop(layers);
                    if flags & DESKTOP != 0 {
                        p.note_desktop(w, h);
                    }
                    on_change();
                }
            }
            IMAGE => {
                let fd = fd.context("IMAGE без дескриптора")?;
                let owned = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) };
                let surface = u64_at(&payload, 0);
                let index = u32_at(&payload, 8);
                let img = syngui::gpu::external::ExternalImage {
                    id: IMAGE_ID.fetch_add(1, Ordering::Relaxed),
                    fd: owned,
                    format: u32_at(&payload, 12) as i32,
                    width: u32_at(&payload, 16),
                    height: u32_at(&payload, 20),
                    flags: u32_at(&payload, 24),
                    usage: u32_at(&payload, 28),
                    size: u64_at(&payload, 32),
                };
                tracing::info!("syn-gpu present: поверхность {surface:#x}: изображение {index} {}×{}", img.width, img.height);
                images.insert((surface, index), Arc::new(img));
            }
            MODES => {
                if let Some(m) = GuestModes::parse(&payload) {
                    // Разрешение 3D машины — серверу, если у него другое (новый запуск машины).
                    if let Some(k) = render_pref(&p.id) {
                        if m.list.iter().any(|e| e.kind == mode_kind::RENDER && e.bpp != 0 && e.width != k) {
                            let e = ModeEntry { kind: mode_kind::RENDER, width: k, height: 0, bpp: 0 };
                            let _ = send(&mut s, SET_MODE, &e.bytes());
                        }
                    }
                    let mut cur = p.modes.lock().unwrap();
                    if cur.as_ref() != Some(&m) {
                        tracing::info!("syn-gpu present: режимы экрана: сейчас {:?}, флаги {:#x}, вариантов {}", m.current, m.flags, m.list.len());
                        *cur = Some(m);
                        drop(cur);
                        on_change();
                    }
                }
            }
            NOTICE if payload.len() >= 24 => {
                let n = Notice { kind: u32_at(&payload, 0), bytes: u64_at(&payload, 8), free: u64_at(&payload, 16) };
                tracing::warn!("syn-gpu: сообщение {n:?}");
                if let Some(f) = &p.on_notice {
                    f(n);
                }
            }
            CURSOR if p.cursor_id == 0 && payload.len() >= 48 => {
                // Без окна (запись экрана): запомнить положение и картинку курсора.
                let (seq, visible) = (u32_at(&payload, 0), u32_at(&payload, 4) != 0);
                let (x, y) = (u32_at(&payload, 8) as i32, u32_at(&payload, 12) as i32);
                let mut cur = p.cursor.lock().unwrap();
                if !visible {
                    *cur = None;
                    cursor_state = Some((seq, visible));
                    continue;
                }
                if cursor_state == Some((seq, visible)) {
                    if let Some(c) = cur.as_mut() {
                        (c.x, c.y) = (x, y);
                    }
                    continue;
                }
                let (w, h, stride) = (u32_at(&payload, 24).min(256) as usize, u32_at(&payload, 28).min(256) as usize, u32_at(&payload, 32) as usize);
                let off = u64_at(&payload, 40) as usize;
                let img = heap.as_ref().filter(|_| w > 0 && h > 0).and_then(|hp| {
                    if stride < w * 4 || off.checked_add(stride * (h - 1) + w * 4)? > hp.len {
                        return None;
                    }
                    let src = unsafe { std::slice::from_raw_parts(hp.ptr.add(off), stride * (h - 1) + w * 4) };
                    let mut rgba = vec![0u8; w * h * 4];
                    for row in 0..h {
                        for (d, s) in rgba[row * w * 4..(row + 1) * w * 4].chunks_exact_mut(4).zip(src[row * stride..row * stride + w * 4].chunks_exact(4)) {
                            d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
                        }
                    }
                    Some(rgba)
                });
                cursor_state = Some((seq, visible));
                *cur = img.map(|rgba| GuestCursor { x, y, width: w as u32, height: h as u32, rgba: Arc::new(rgba) });
            }
            CURSOR if p.cursor_id != 0 && payload.len() >= 48 => {
                let (seq, visible) = (u32_at(&payload, 0), u32_at(&payload, 4) != 0);
                if cursor_state == Some((seq, visible)) {
                    continue;
                }
                cursor_state = Some((seq, visible));
                let (hx, hy, w, h, stride) = (u32_at(&payload, 16), u32_at(&payload, 20), u32_at(&payload, 24), u32_at(&payload, 28), u32_at(&payload, 32) as usize);
                let off = u64_at(&payload, 40) as usize;
                let img = if !visible || w == 0 || h == 0 {
                    None
                } else {
                    heap.as_ref().and_then(|hp| {
                        let (w, h) = (w.min(256) as usize, h.min(256) as usize);
                        if stride < w * 4 || off.checked_add(stride * (h - 1) + w * 4)? > hp.len {
                            return None;
                        }
                        let src = unsafe { std::slice::from_raw_parts(hp.ptr.add(off), stride * (h - 1) + w * 4) };
                        let mut rgba = vec![0u8; w * h * 4];
                        for row in 0..h {
                            for (d, s) in rgba[row * w * 4..(row + 1) * w * 4].chunks_exact_mut(4).zip(src[row * stride..row * stride + w * 4].chunks_exact(4)) {
                                d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
                            }
                        }
                        Some(syngui::input::CursorImage { width: w as u16, height: h as u16, hot_x: hx.min(w as u32 - 1) as u16, hot_y: hy.min(h as u32 - 1) as u16, rgba })
                    })
                };
                syngui::input::set_custom_cursor(p.cursor_id, img);
            }
            GONE => {
                let surface = u64_at(&payload, 0);
                images.retain(|k, _| k.0 != surface);
                if p.layers.lock().unwrap().remove(&surface).is_some() {
                    on_change();
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// BGRA (AR24/XR24) из BAR2 → RGBA для LiveFrame.
fn copy_frame(hp: &Heap, offset: usize, width: u32, height: u32, stride: u32, format: u32) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || (format != AR24 && format != XR24) {
        return None;
    }
    let (w, h, stride) = (width as usize, height as usize, stride as usize);
    if stride < w * 4 || offset.checked_add(stride.checked_mul(h - 1)?.checked_add(w * 4)?)? > hp.len {
        return None;
    }
    let src = unsafe { std::slice::from_raw_parts(hp.ptr.add(offset), stride * (h - 1) + w * 4) };
    let mut out = vec![0u8; w * h * 4];
    let opaque = format == XR24;
    for row in 0..h {
        let s = &src[row * stride..row * stride + w * 4];
        let d = &mut out[row * w * 4..(row + 1) * w * 4];
        for (dp, sp) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
            dp[0] = sp[2];
            dp[1] = sp[1];
            dp[2] = sp[0];
            dp[3] = if opaque { 255 } else { sp[3] };
        }
    }
    Some(out)
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    b.get(i..i + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap())).unwrap_or(0)
}

fn u64_at(b: &[u8], i: usize) -> u64 {
    b.get(i..i + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap())).unwrap_or(0)
}

fn is_timeout(e: &anyhow::Error) -> bool {
    e.downcast_ref::<std::io::Error>().is_some_and(|e| matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

fn send(s: &mut UnixStream, ty: u32, payload: &[u8]) -> Result<()> {
    let mut b = Vec::with_capacity(8 + payload.len());
    b.extend_from_slice(&ty.to_le_bytes());
    b.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    b.extend_from_slice(payload);
    s.write_all(&b)?;
    Ok(())
}

/// Сообщение: заголовок читается recvmsg (с ним может прийти дескриптор), нагрузка — следом.
fn read_msg(s: &mut UnixStream) -> Result<(u32, Vec<u8>, Option<RawFd>)> {
    let mut hdr = [0u8; 8];
    let mut cmsg = [0u8; 64];
    let mut iov = libc::iovec { iov_base: hdr.as_mut_ptr() as *mut _, iov_len: hdr.len() };
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg.as_mut_ptr() as *mut _;
    msg.msg_controllen = cmsg.len() as _;
    let n = unsafe { libc::recvmsg(s.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC) };
    if n < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if n == 0 {
        bail!("syn-gpu закрыл соединение");
    }
    let mut fd = None;
    unsafe {
        let mut c = libc::CMSG_FIRSTHDR(&msg);
        while !c.is_null() {
            if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                fd = Some(std::ptr::read_unaligned(libc::CMSG_DATA(c) as *const RawFd));
            }
            c = libc::CMSG_NXTHDR(&msg, c);
        }
    }
    let got = n as usize;
    if got < 8 {
        // Остаток заголовка — обычным чтением (без тайм-аута посередине сообщения).
        s.set_read_timeout(None)?;
        s.read_exact(&mut hdr[got..])?;
    }
    let ty = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
    let len = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
    if len > 1 << 20 {
        bail!("сообщение syn-gpu слишком длинное: {len}");
    }
    let mut payload = vec![0u8; len];
    s.set_read_timeout(None)?;
    s.read_exact(&mut payload)?;
    s.set_read_timeout(Some(Duration::from_secs(1)))?;
    Ok((ty, payload, fd))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::io::FromRawFd;

    #[test]
    fn notice_reaches_callback() {
        let (mut srv, cli) = UnixStream::pair().unwrap();
        let got: Arc<Mutex<Vec<(u32, u64)>>> = Arc::default();
        let g = got.clone();
        let p = Present {
            layers: Mutex::new(HashMap::new()),
            alive: AtomicBool::new(true),
            mode: Mutex::new(None),
            refresh: Mutex::new(None),
            tx: Mutex::new(None),
            cursor_id: 0,
            on_notice: Some(Arc::new(move |n: Notice| g.lock().unwrap().push((n.kind, n.bytes)))),
            started: Instant::now(),
            id: String::new(),
            desk_size: Mutex::new((0, 0)),
            cursor: Mutex::new(None),
            modes: Mutex::new(None),
        };
        let mut m = Vec::new();
        m.extend_from_slice(&NOTICE.to_le_bytes());
        m.extend_from_slice(&24u32.to_le_bytes());
        m.extend_from_slice(&OUT_OF_VIDEO_MEMORY.to_le_bytes());
        m.extend_from_slice(&0u32.to_le_bytes());
        m.extend_from_slice(&(256u64 << 20).to_le_bytes());
        m.extend_from_slice(&0u64.to_le_bytes());
        srv.write_all(&m).unwrap();
        drop(srv);
        let cb: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let _ = run(&p, cli, &cb, false);
        assert_eq!(*got.lock().unwrap(), vec![(OUT_OF_VIDEO_MEMORY, 256 << 20)]);
    }

    /// Сервер-заглушка: HELLO с memfd, FRAME 2×1 в BGRA → слой RGBA, RELEASE в ответ, GONE.
    #[test]
    fn hello_frame_release_gone() {
        let (mut srv, cli) = UnixStream::pair().unwrap();
        let memfd = unsafe { libc::memfd_create(c"bar2".as_ptr(), 0) };
        let mut f = unsafe { std::fs::File::from_raw_fd(memfd) };
        let mut bar = vec![0u8; 4096];
        bar[256..264].copy_from_slice(&[10, 20, 30, 0, 40, 50, 60, 0]); // два пикселя B,G,R,X
        f.write_all(&bar).unwrap();
        // HELLO с дескриптором.
        let mut hello = Vec::new();
        hello.extend_from_slice(&HELLO.to_le_bytes());
        hello.extend_from_slice(&16u32.to_le_bytes());
        hello.extend_from_slice(&1u32.to_le_bytes());
        hello.extend_from_slice(&0u32.to_le_bytes());
        hello.extend_from_slice(&4096u64.to_le_bytes());
        send_fd(&srv, &hello, f.as_raw_fd());
        // FRAME: surface 7, seq 1, offset 256, 2×1, stride 8, XR24, x=100,y=50,w=2,h=1, flags 1.
        let mut fr = Vec::new();
        for v in [7u64, 1, 256] {
            fr.extend_from_slice(&v.to_le_bytes());
        }
        for v in [2u32, 1, 8, XR24, 100, 50, 2, 1, 1, 0] {
            fr.extend_from_slice(&v.to_le_bytes());
        }
        send(&mut srv, FRAME, &fr).unwrap();
        let p = Arc::new(Present { layers: Mutex::new(HashMap::new()), alive: AtomicBool::new(true), mode: Mutex::new(None), refresh: Mutex::new(None), tx: Mutex::new(None), cursor_id: 0, on_notice: None, started: Instant::now(), id: String::new(), desk_size: Mutex::new((0, 0)), cursor: Mutex::new(None), modes: Mutex::new(None) });
        let changed = Arc::new(AtomicBool::new(false));
        let ch = changed.clone();
        let p2 = p.clone();
        let t = std::thread::spawn(move || {
            let cb: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ch.store(true, Ordering::Relaxed));
            let _ = run(&p2, cli, &cb, false);
        });
        // RELEASE.
        let mut rel = [0u8; 24];
        srv.read_exact(&mut rel).unwrap();
        assert_eq!(u32_at(&rel, 0), RELEASE);
        assert_eq!((u64_at(&rel, 8), u64_at(&rel, 16)), (7, 1));
        std::thread::sleep(Duration::from_millis(50));
        let l = p.visible().expect("слой");
        assert_eq!(l.rect, (100, 50, 2, 1));
        let (w, h, px, _) = l.frame.latest();
        assert_eq!((w, h), (2, 1));
        assert_eq!(&px[..8], &[30, 20, 10, 255, 60, 50, 40, 255]);
        assert!(changed.load(Ordering::Relaxed));
        // GONE.
        send(&mut srv, GONE, &7u64.to_le_bytes()).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert!(p.visible().is_none());
        p.close();
        drop(srv);
        t.join().unwrap();
    }

    /// v2: IMAGE (fd) → FRAME с GPU_IMAGE → слой с кадром во внешнем изображении; RELEASE —
    /// только когда кадр отпущен (здесь — GONE убирает слой), а не сразу после FRAME.
    #[test]
    fn gpu_image_release_on_drop() {
        let (mut srv, cli) = UnixStream::pair().unwrap();
        let memfd = unsafe { libc::memfd_create(c"img".as_ptr(), 0) };
        // IMAGE: surface 7, index 1, B8G8R8A8_UNORM (44), 640×480, MUTABLE_FORMAT (8), usage 6, size 1 МиБ.
        let mut im = Vec::new();
        im.extend_from_slice(&IMAGE.to_le_bytes());
        im.extend_from_slice(&40u32.to_le_bytes());
        im.extend_from_slice(&7u64.to_le_bytes());
        for v in [1u32, 44, 640, 480, 8, 6] {
            im.extend_from_slice(&v.to_le_bytes());
        }
        im.extend_from_slice(&(1u64 << 20).to_le_bytes());
        send_fd(&srv, &im, memfd);
        unsafe { libc::close(memfd) };
        let mut fr = Vec::new();
        for v in [7u64, 5, 1] {
            fr.extend_from_slice(&v.to_le_bytes());
        }
        for v in [640u32, 480, 0, XR24, 10, 20, 640, 480, 1 | GPU_IMAGE, 0] {
            fr.extend_from_slice(&v.to_le_bytes());
        }
        send(&mut srv, FRAME, &fr).unwrap();
        let p = Arc::new(Present { layers: Mutex::new(HashMap::new()), alive: AtomicBool::new(true), mode: Mutex::new(None), refresh: Mutex::new(None), tx: Mutex::new(None), cursor_id: 0, on_notice: None, started: Instant::now(), id: String::new(), desk_size: Mutex::new((0, 0)), cursor: Mutex::new(None), modes: Mutex::new(None) });
        let p2 = p.clone();
        let t = std::thread::spawn(move || {
            let cb: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
            let _ = run(&p2, cli, &cb, false);
        });
        std::thread::sleep(Duration::from_millis(100));
        let l = p.visible().expect("слой");
        assert_eq!(l.rect, (10, 20, 640, 480));
        assert_eq!(l.frame.size(), (640, 480));
        assert!(l.frame.latest().2.is_empty(), "кадр во внешнем изображении — без пикселей в памяти");
        drop(l);
        // Пока слой держит кадр, RELEASE нет.
        srv.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        let mut rel = [0u8; 24];
        assert!(srv.read_exact(&mut rel).is_err());
        send(&mut srv, GONE, &7u64.to_le_bytes()).unwrap();
        srv.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        srv.read_exact(&mut rel).unwrap();
        assert_eq!(u32_at(&rel, 0), RELEASE);
        assert_eq!((u64_at(&rel, 8), u64_at(&rel, 16)), (7, 5));
        p.close();
        drop(srv);
        t.join().unwrap();
    }

    fn send_fd(s: &UnixStream, bytes: &[u8], fd: RawFd) {
        let mut cmsg = [0u8; 64];
        let mut iov = libc::iovec { iov_base: bytes.as_ptr() as *mut _, iov_len: bytes.len() };
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cmsg.as_mut_ptr() as *mut _;
        msg.msg_controllen = unsafe { libc::CMSG_SPACE(4) } as _;
        unsafe {
            let c = libc::CMSG_FIRSTHDR(&msg);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN(4) as _;
            std::ptr::write_unaligned(libc::CMSG_DATA(c) as *mut RawFd, fd);
            assert!(libc::sendmsg(s.as_raw_fd(), &msg, 0) > 0);
        }
    }
}

#[cfg(test)]
mod modes_tests {
    use super::*;

    /// `Modes` как его шлёт syn-gpu (repr(C), 308 байт): текущий, флаги, count, list[24].
    #[test]
    fn modes_parse_and_set_mode_bytes() {
        let e = |k: u16, w: u16, h: u16, b: u16| ModeEntry { kind: k, width: w, height: h, bpp: b };
        let mut buf = Vec::new();
        buf.extend_from_slice(&e(mode_kind::GRAPHICS, 640, 480, 8).bytes());
        buf.extend_from_slice(&(modes_flags::DOS | modes_flags::AGENT | modes_flags::PENDING).to_le_bytes());
        buf.extend_from_slice(&2u32.to_le_bytes());
        let list = [e(mode_kind::TEXT, 132, 50, 0), e(mode_kind::WINDOW, 166, 52, 0)];
        for i in 0..24 {
            buf.extend_from_slice(&list.get(i).copied().unwrap_or_default().bytes());
        }
        assert_eq!(buf.len(), 308);
        let m = GuestModes::parse(&buf).unwrap();
        assert_eq!(m.current, e(mode_kind::GRAPHICS, 640, 480, 8));
        assert!(m.has(modes_flags::PENDING) && !m.has(modes_flags::DESKTOP));
        assert_eq!(m.list, list.to_vec());
        // SET_MODE: тот же ModeEntry — kind, _pad, width, height, bpp, _pad2.
        assert_eq!(list[0].bytes(), [1, 0, 0, 0, 132, 0, 50, 0, 0, 0, 0, 0]);
    }
}
