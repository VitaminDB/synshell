//! Программный ISP для сырого Bayer ([`crate::mcam`]): кадр RAW8/10/12 (упакованный MIPI или по 16 бит)
//! → NV12 полного диапазона (BT.601, как у остальных источников) в несколько потоков.
//!
//! 1. Распаковка строк, чёрный уровень, баланс белого, усиление ISP и гамма — одной таблицей на цвет
//!    (значение датчика → 8 бит), в плоскость с полем в 1 точку; по ходу — статистика по зонам 16×12
//!    (средние R, G, B, пересвет).
//! 2. Дебайер билинейный: яркость по каждой точке, цветность — среднее квадрата 2×2 с усилением
//!    насыщенности (без матрицы цвета датчика краски бледные).
//! 3. 3A по статистике: баланс белого — «серый мир» по не пересвеченным и не тёмным зонам; автоэкспозиция
//!    — к средней яркости 18 % (центр или точка касания), порядок: выдержка до кадра 30 к/с, в фото — до
//!    15 к/с, затем аналоговое, цифровое усиление сенсора и усиление ISP; автофокус — контраст в окне
//!    (обход хода линзы грубо, затем точно; непрерывный — перезапуск, когда резкость упала).

use std::sync::Mutex;

use crate::mcam;
use crate::syncamd::{self, Control, Meta};

/// Наибольший цифровой зум (дебайер обрезанной середины и билинейное увеличение до размера потока).
pub const MAX_ZOOM: f32 = 4.0;
/// Самая длинная выдержка ручного режима.
pub const MAX_EXPOSURE_NS: i64 = 500_000_000;
/// Ближний предел фокуса для ручного режима (диоптрии, 10 см).
pub const MIN_FOCUS_DIOPTERS: f32 = 10.0;

const ZX: usize = 16;
const ZY: usize = 12;
const ZN: usize = ZX * ZY;
/// Целевая средняя яркость (линейная).
const AE_TARGET: f32 = 0.18;
/// Доля S-кривой в тоне (0 — чистая гамма sRGB).
const CONTRAST: f32 = 0.35;
/// Общее усиление, после которого в фото кадр удлиняется (до 15 к/с), а не растёт шум.
const GAIN_BEFORE_SLOW: f32 = 4.0;
/// Насыщенность цветности (вместо матрицы цвета датчика).
const SATURATION: f32 = 1.4;
/// Аналоговое усиление при максимальном коде (у большинства сенсоров ×16).
const AGAIN_MAX: f32 = 16.0;
const DGAIN_MAX: f32 = 4.0;
const ISP_GAIN_MAX: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bayer {
    Rggb = 0,
    Grbg = 1,
    Gbrg = 2,
    Bggr = 3,
}

const R: u8 = 0;
const G: u8 = 1;
const B: u8 = 2;
/// Типы точки для дебайера: зелёная в строке с красными / с синими.
const T_R: u8 = 0;
const T_B: u8 = 1;
const T_GR: u8 = 2;
const T_GB: u8 = 3;

impl Bayer {
    /// Цвет точки (x, y).
    pub fn color(self, x: usize, y: usize) -> u8 {
        let p = [[R, G, G, B], [G, R, B, G], [G, B, R, G], [B, G, G, R]][self as usize];
        p[(y & 1) * 2 + (x & 1)]
    }

    fn kind(self, x: usize, y: usize) -> u8 {
        match self.color(x, y) {
            R => T_R,
            B => T_B,
            _ => {
                if self.color(x ^ 1, y) == R {
                    T_GR
                } else {
                    T_GB
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawFmt {
    pub order: Bayer,
    pub bits: u32,
    /// Упакованный MIPI (RAW10: 4 точки в 5 байтах, RAW12: 2 в 3), иначе по 16 бит LE.
    pub packed: bool,
}

/// Пределы контролов сенсора: (min, max, step, default).
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub exposure: Option<(i32, i32, i32, i32)>,
    pub analogue: Option<(i32, i32, i32, i32)>,
    pub digital: Option<(i32, i32, i32, i32)>,
    pub vblank: Option<(i32, i32, i32, i32)>,
    pub height: u32,
    /// Время строки, нс.
    pub line_ns: f64,
    /// Выдержка не длиннее (строк кадра − margin).
    pub margin: i32,
}

impl Limits {
    fn again_f(&self, code: i32) -> f32 {
        match self.analogue {
            Some((lo, hi, ..)) if hi > lo => 1.0 + (code - lo) as f32 * (AGAIN_MAX - 1.0) / (hi - lo) as f32,
            _ => 1.0,
        }
    }

    fn again_code(&self, f: f32) -> i32 {
        match self.analogue {
            Some((lo, hi, step, _)) if hi > lo => {
                let c = lo + ((f - 1.0) * (hi - lo) as f32 / (AGAIN_MAX - 1.0)) as i32;
                (lo + (c - lo) / step * step).clamp(lo, hi)
            }
            Some((lo, ..)) => lo,
            None => 0,
        }
    }

    fn again_max(&self) -> f32 {
        self.analogue.map(|a| self.again_f(a.1)).unwrap_or(1.0)
    }

    fn dgain_unity(&self) -> i32 {
        self.digital.map(|d| if d.3 > 0 { d.3 } else { d.0.max(1) }).unwrap_or(1)
    }

    fn dgain_f(&self, code: i32) -> f32 {
        if self.digital.is_some() {
            code as f32 / self.dgain_unity() as f32
        } else {
            1.0
        }
    }

    fn dgain_max(&self) -> f32 {
        self.digital.map(|d| (d.1 as f32 / self.dgain_unity() as f32).min(DGAIN_MAX)).unwrap_or(1.0)
    }

    fn dgain_code(&self, f: f32) -> i32 {
        match self.digital {
            Some((lo, hi, ..)) => ((f * self.dgain_unity() as f32) as i32).clamp(lo, hi),
            None => 0,
        }
    }

    /// Наибольшее общее усиление (сенсор × ISP).
    pub fn max_gain(&self) -> f32 {
        self.again_max() * self.dgain_max() * ISP_GAIN_MAX
    }

    fn vb_min(&self) -> i32 {
        self.vblank.map(|v| v.0).unwrap_or(0)
    }

    /// Строк выдержки при длине кадра не больше `frame_ns`.
    fn lines_for_frame(&self, frame_ns: f64) -> i32 {
        let vts = (frame_ns / self.line_ns) as i32;
        let vb_max = self.vblank.map(|v| v.1).unwrap_or(self.vb_min());
        let vts = vts.clamp(self.height as i32 + self.vb_min(), self.height as i32 + vb_max);
        vts - self.margin
    }

    /// Строк при наименьшем гашении (обычная частота сенсора).
    fn lines_fast(&self) -> i32 {
        match (self.vblank, self.exposure) {
            (Some(v), _) => self.height as i32 + v.0 - self.margin,
            (None, Some(e)) => e.1,
            _ => self.height as i32,
        }
    }

    fn exp_min(&self) -> i32 {
        self.exposure.map(|e| e.0.max(1)).unwrap_or(1)
    }
}

#[derive(Clone, Copy, Debug)]
struct Ae {
    lines: i32,
    again: i32,
    dgain: i32,
    vblank: i32,
    isp: f32,
}

#[derive(Clone, Copy, Debug)]
struct Saved {
    ae: Ae,
    wb: [f32; 3],
    focus: i32,
}

/// Последние экспозиция, баланс и фокус по камере: переоткрытие начинается с них (без «вспышки»).
static SAVED: Mutex<Vec<(String, Saved)>> = Mutex::new(Vec::new());

#[derive(Clone, Copy)]
struct Zones {
    sum: [[u64; 3]; ZN],
    cnt: [[u32; 3]; ZN],
    sat: u32,
    total: u32,
}

impl Zones {
    fn new() -> Zones {
        Zones { sum: [[0; 3]; ZN], cnt: [[0; 3]; ZN], sat: 0, total: 0 }
    }

    fn add(&mut self, o: &Zones) {
        for z in 0..ZN {
            for c in 0..3 {
                self.sum[z][c] += o.sum[z][c];
                self.cnt[z][c] += o.cnt[z][c];
            }
        }
        self.sat += o.sat;
        self.total += o.total;
    }

    /// Средние R, G, B зоны (доли полной шкалы после чёрного).
    fn mean(&self, z: usize, range: f32) -> [f32; 3] {
        let mut m = [0.0; 3];
        for c in 0..3 {
            if self.cnt[z][c] > 0 {
                m[c] = self.sum[z][c] as f32 / self.cnt[z][c] as f32 / range;
            }
        }
        m
    }
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum AfState {
    Idle,
    Scan,
    Done,
}

struct Af {
    range: Option<(i32, i32, i32, i32)>,
    pos: i32,
    dirty: bool,
    state: AfState,
    /// Окно резкости — центр в долях кадра.
    roi: (f32, f32),
    positions: Vec<i32>,
    idx: usize,
    results: Vec<(i32, f64)>,
    fine: bool,
    settle: u32,
    /// Резкость после наводки и счётчик кадров с упавшей резкостью.
    base: f64,
    low: u32,
    result: u32,
}

pub struct Isp {
    name: String,
    /// Кадр сенсора.
    w: usize,
    h: usize,
    /// Выход (середина кадра сенсора с нужным соотношением сторон).
    ow: usize,
    oh: usize,
    fmt: RawFmt,
    lim: Limits,
    video: bool,
    black: u32,
    maxv: u32,
    gamma: Vec<u8>,
    lut: [Vec<u8>; 3],
    plane: Vec<u8>,
    zones: Zones,
    wb: [f32; 3],
    ae: Ae,
    ae_dirty: bool,
    ae_ok: bool,
    /// Кадров до того, как сенсор применит записанную экспозицию (решать по старым кадрам нельзя).
    ae_wait: u32,
    ctl: Control,
    frame: u64,
    af: Af,
    meta_due: bool,
    threads: usize,
    /// Обрезанная середина при зуме (NV12 до увеличения).
    zoom_buf: Vec<u8>,
}

impl Isp {
    /// `w × h` — кадр сенсора, `out` — размер выхода (середина кадра; `None` — весь кадр).
    pub fn new(name: &str, w: u32, h: u32, out: Option<(u32, u32)>, fmt: RawFmt, lim: Limits, focus: Option<(i32, i32, i32, i32)>, video: bool) -> Isp {
        let maxv = (1u32 << fmt.bits) - 1;
        let black = if fmt.bits >= 10 { 64 << (fmt.bits - 10) } else { 16 };
        let gamma = (0..4096)
            .map(|i| {
                let l = i as f32 / 4095.0;
                let v = if l <= 0.003_130_8 { 12.92 * l } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
                // лёгкая S-кривая: без неё кадр без тоновой обработки плоский
                let v = v + CONTRAST * (v * v * (3.0 - 2.0 * v) - v);
                (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8
            })
            .collect();
        let saved = SAVED.lock().unwrap().iter().find(|s| s.0 == name).map(|s| s.1);
        let fast = lim.lines_fast();
        let ae = saved.map(|s| s.ae).unwrap_or(Ae {
            lines: (fast / 2).max(lim.exp_min()),
            again: lim.again_code(4.0),
            dgain: lim.dgain_code(1.0),
            vblank: lim.vb_min(),
            isp: 1.0,
        });
        let mut ae = ae;
        if video {
            // видео — не медленнее 30 к/с
            let total = ae.lines as f32 * lim.again_f(ae.again) * lim.dgain_f(ae.dgain) * ae.isp;
            ae.lines = ae.lines.min(fast);
            ae.vblank = lim.vb_min();
            let rest = total / ae.lines.max(1) as f32;
            let ag = rest.clamp(1.0, lim.again_max());
            ae.again = lim.again_code(ag);
            ae.isp = (rest / lim.again_f(ae.again)).clamp(1.0, ISP_GAIN_MAX);
            ae.dgain = lim.dgain_code(1.0);
        }
        let af_pos = saved.map(|s| s.focus).or(focus.map(|f| f.3)).unwrap_or(0);
        let (w, h) = (w as usize & !1, h as usize & !1);
        let (ow, oh) = out.map(|(a, b)| ((a as usize).min(w) & !3, (b as usize).min(h) & !3)).unwrap_or((w, h));
        Isp {
            name: name.to_string(),
            w,
            h,
            ow,
            oh,
            fmt,
            lim,
            video,
            black,
            maxv,
            gamma,
            lut: [vec![0; maxv as usize + 1], vec![0; maxv as usize + 1], vec![0; maxv as usize + 1]],
            plane: vec![0; (w + 2) * (h + 2)],
            zones: Zones::new(),
            wb: saved.map(|s| s.wb).unwrap_or([1.8, 1.0, 1.6]),
            ae,
            ae_dirty: true,
            ae_ok: false,
            ae_wait: 3,
            ctl: Control::default(),
            frame: 0,
            af: Af {
                range: focus,
                pos: af_pos,
                dirty: focus.is_some(),
                state: AfState::Idle,
                roi: (0.5, 0.5),
                positions: Vec::new(),
                idx: 0,
                results: Vec::new(),
                fine: false,
                settle: 0,
                base: 0.0,
                low: 0,
                result: 0,
            },
            meta_due: false,
            zoom_buf: Vec::new(),
            threads: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).clamp(1, 4),
        }
    }

    /// Запомнить экспозицию, баланс и фокус для следующего открытия.
    pub fn remember(&self) {
        let mut s = SAVED.lock().unwrap();
        s.retain(|x| x.0 != self.name);
        s.push((self.name.clone(), Saved { ae: self.ae, wb: self.wb, focus: self.af.pos }));
    }

    /// Управление от программы (поля из `mask`).
    pub fn control(&mut self, c: &Control) {
        let m = c.mask;
        if m & syncamd::CTL_POINT != 0 {
            self.ctl.x = c.x;
            self.ctl.y = c.y;
            if c.x >= 0.0 && c.y >= 0.0 {
                self.af.roi = (c.x.clamp(0.0, 1.0), c.y.clamp(0.0, 1.0));
                self.start_af();
            } else {
                self.af.roi = (0.5, 0.5);
            }
        }
        if m & syncamd::CTL_AE_LOCK != 0 {
            self.ctl.ae_lock = c.ae_lock;
        }
        if m & syncamd::CTL_EV != 0 {
            self.ctl.ev = c.ev;
        }
        if m & syncamd::CTL_ZOOM != 0 {
            self.ctl.zoom = c.zoom;
        }
        if m & syncamd::CTL_MODE != 0 {
            self.ctl.mode = c.mode;
        }
        if m & syncamd::CTL_AWB != 0 {
            self.ctl.awb = c.awb;
        }
        if m & syncamd::CTL_MANUAL != 0 {
            self.ctl.iso = c.iso;
            self.ctl.exposure_ns = c.exposure_ns;
            let was_manual_focus = self.ctl.focus >= 0.0;
            self.ctl.focus = c.focus;
            if c.iso > 0 && c.exposure_ns > 0 {
                self.manual_exposure();
            }
            if c.focus >= 0.0 {
                if let Some((lo, hi, ..)) = self.af.range {
                    self.af.state = AfState::Done;
                    self.af.pos = lo + ((c.focus / MIN_FOCUS_DIOPTERS).clamp(0.0, 1.0) * (hi - lo) as f32) as i32;
                    self.af.dirty = true;
                }
            } else if was_manual_focus {
                self.start_af();
            }
        }
    }

    fn manual(&self) -> bool {
        self.ctl.iso > 0 && self.ctl.exposure_ns > 0
    }

    fn manual_exposure(&mut self) {
        let l = &self.lim;
        let lines = ((self.ctl.exposure_ns as f64 / l.line_ns) as i32).clamp(l.exp_min(), l.lines_for_frame(MAX_EXPOSURE_NS as f64 + 1e6));
        let gain = (self.ctl.iso as f32 / 100.0).max(1.0);
        let ag = gain.clamp(1.0, l.again_max());
        let again = l.again_code(ag);
        let rest = gain / l.again_f(again);
        let dg = rest.clamp(1.0, l.dgain_max());
        let dgain = l.dgain_code(dg);
        let isp = (rest / l.dgain_f(dgain)).clamp(1.0, ISP_GAIN_MAX);
        let vblank = self.vblank_for(lines);
        self.ae = Ae { lines, again, dgain, vblank, isp };
        self.ae_dirty = true;
    }

    fn vblank_for(&self, lines: i32) -> i32 {
        let l = &self.lim;
        let vb = (lines + l.margin - l.height as i32).max(l.vb_min());
        l.vblank.map(|v| vb.min(v.1)).unwrap_or(vb)
    }

    fn start_af(&mut self) {
        let Some((lo, hi, step, _)) = self.af.range else { return };
        if self.ctl.focus >= 0.0 {
            return;
        }
        let n = 8;
        let st = ((hi - lo) / n).max(step);
        self.af.positions = (0..=n).map(|i| (lo + i * st).min(hi)).collect();
        self.af.positions.dedup();
        self.af.idx = 0;
        self.af.results.clear();
        self.af.fine = false;
        self.af.state = AfState::Scan;
        self.af.pos = self.af.positions[0];
        self.af.dirty = true;
        self.af.settle = 2; // первая точка дальше всех — линзе нужно время
    }

    /// Записать экспозицию и фокус в сенсор и линзу (если менялись).
    pub fn apply_sensor(&mut self, sensor: i32, lens: Option<i32>, force: bool) {
        if self.ae_dirty && !force && self.ae_wait == 0 {
            self.ae_wait = 3; // ручная экспозиция и прочие записи — тоже ждать
        }
        if self.ae_dirty || force {
            let a = self.ae;
            let l = &self.lim;
            if l.vblank.is_some() {
                mcam::set(sensor, mcam::CID_VBLANK, a.vblank);
            }
            if l.exposure.is_some() {
                mcam::set(sensor, mcam::CID_EXPOSURE, a.lines);
            }
            if l.analogue.is_some() {
                mcam::set(sensor, mcam::CID_ANALOGUE_GAIN, a.again);
            }
            if l.digital.is_some() {
                mcam::set(sensor, mcam::CID_DIGITAL_GAIN, a.dgain);
            }
            self.ae_dirty = false;
        }
        if let Some(fd) = lens {
            if self.af.dirty || force {
                mcam::set(fd, mcam::CID_FOCUS_ABSOLUTE, self.af.pos);
                self.af.dirty = false;
            }
        }
    }

    /// Состояние 3A — раз в 6 кадров.
    pub fn take_meta(&mut self) -> Option<Meta> {
        if !std::mem::take(&mut self.meta_due) {
            return None;
        }
        let a = self.ae;
        let l = &self.lim;
        let mut m = Meta::default();
        m.af = match self.af.state {
            AfState::Scan => 1,
            AfState::Done => self.af.result,
            AfState::Idle => 0,
        };
        m.ae_state = if self.ae_ok { 2 } else { 1 };
        m.iso = (100.0 * l.again_f(a.again) * l.dgain_f(a.dgain) * a.isp) as i32;
        m.exposure_ns = (a.lines as f64 * l.line_ns) as i64;
        m.focus = match self.af.range {
            Some((lo, hi, ..)) if hi > lo => (self.af.pos - lo) as f32 / (hi - lo) as f32 * MIN_FOCUS_DIOPTERS,
            _ => -1.0,
        };
        Some(m)
    }

    /// Размер выхода NV12.
    pub fn out_size(&self) -> (u32, u32) {
        (self.ow as u32, self.oh as u32)
    }

    /// Кадр RAW (`stride` байт на строку) → NV12 размера [`Isp::out_size`] в `out`; затем 3A по статистике.
    pub fn process(&mut self, raw: &[u8], stride: usize, out: &mut [u8]) {
        let (w, h) = (self.w, self.h);
        self.build_luts();
        let pw = w + 2;
        // 1. распаковка + таблицы + статистика, полосами строк (чётное число строк на поток)
        let rows = ((h / self.threads).div_ceil(2) * 2).max(2);
        let fmt = self.fmt;
        let (black, maxv) = (self.black, self.maxv);
        let lut = &self.lut;
        let mut zones = Zones::new();
        {
            let interior = &mut self.plane[pw..pw * (h + 1)];
            let parts: Vec<Zones> = std::thread::scope(|s| {
                let mut hs = Vec::new();
                for (k, chunk) in interior.chunks_mut(rows * pw).enumerate() {
                    let y0 = k * rows;
                    hs.push(s.spawn(move || unpack_rows(raw, stride, fmt, w, h, y0, chunk, lut, black, maxv)));
                }
                hs.into_iter().map(|j| j.join().unwrap()).collect()
            });
            for z in &parts {
                zones.add(z);
            }
        }
        // поле в 1 точку (отражение через 2 — тот же цвет Bayer)
        let p = &mut self.plane;
        for y in 1..=h {
            let r = y * pw;
            p[r] = p[r + 2];
            p[r + w + 1] = p[r + w - 1];
        }
        p.copy_within(2 * pw..3 * pw, 0);
        p.copy_within((h - 1) * pw..h * pw, (h + 1) * pw);
        // 2. дебайер → NV12 (при зуме — середина, затем увеличение)
        let (x0, y0, cw, ch) = self.crop();
        let order = self.fmt.order;
        let sat = (SATURATION * 256.0) as i32;
        let plane = &self.plane;
        let threads = self.threads;
        let dem = |dst: &mut [u8]| {
            let (yp, uv) = dst.split_at_mut(cw * ch);
            let qrows = (ch / 2).div_ceil(threads).max(1);
            std::thread::scope(|s| {
                for (k, (yc, uvc)) in yp.chunks_mut(qrows * 2 * cw).zip(uv.chunks_mut(qrows * cw)).enumerate() {
                    s.spawn(move || demosaic_rows(plane, pw, x0, y0, cw, k * qrows, yc, uvc, order, sat));
                }
            });
        };
        let (ow, oh) = (self.ow, self.oh);
        if (cw, ch) == (ow, oh) {
            dem(out);
        } else {
            let mut tmp = std::mem::take(&mut self.zoom_buf);
            tmp.resize(cw * ch * 3 / 2, 0);
            dem(&mut tmp);
            upscale_nv12(&tmp, cw, ch, out, ow, oh, self.threads);
            self.zoom_buf = tmp;
        }
        self.zones = zones;
        self.frame += 1;
        self.awb();
        if !self.manual() && self.ctl.ae_lock == 0 && self.af.state != AfState::Scan {
            self.auto_exposure();
        }
        self.autofocus();
        if self.frame % 6 == 0 {
            self.meta_due = true;
        }
    }

    /// Обрезка кадра сенсора: середина размера выхода, при зуме — её середина; начало и размер (чётные).
    fn crop(&self) -> (usize, usize, usize, usize) {
        let z = self.ctl.zoom.clamp(1.0, MAX_ZOOM);
        let (cw, ch) = if z < 1.01 {
            (self.ow, self.oh)
        } else {
            (((self.ow as f32 / z) as usize & !3).max(16), ((self.oh as f32 / z) as usize & !3).max(16))
        };
        (((self.w - cw) / 2) & !1, ((self.h - ch) / 2) & !1, cw, ch)
    }

    fn build_luts(&mut self) {
        let range = (self.maxv - self.black).max(1) as f32;
        for c in 0..3 {
            let k = 4095.0 / range * self.wb[c] * self.ae.isp;
            let black = self.black;
            for (v, o) in self.lut[c].iter_mut().enumerate() {
                let l = (v as f32 - black as f32).max(0.0) * k;
                *o = self.gamma[(l as usize).min(4095)];
            }
        }
    }

    fn range(&self) -> f32 {
        (self.maxv - self.black).max(1) as f32
    }

    fn awb(&mut self) {
        let presets: &[(u32, [f32; 3])] = &[(2, [1.15, 1.0, 2.6]), (3, [1.5, 1.0, 2.2]), (4, [1.35, 1.0, 2.4]), (5, [1.85, 1.0, 1.55]), (6, [2.0, 1.0, 1.4]), (7, [1.6, 1.0, 1.9]), (8, [2.15, 1.0, 1.3])];
        if let Some(p) = presets.iter().find(|p| p.0 == self.ctl.awb) {
            self.wb = p.1;
            return;
        }
        if self.ctl.awb == 0 || self.ctl.ae_lock != 0 {
            return; // выключен или заблокирован вместе с экспозицией
        }
        let range = self.range();
        let (mut sr, mut sg, mut sb) = (0.0f64, 0.0f64, 0.0f64);
        for z in 0..ZN {
            let m = self.zones.mean(z, range);
            let mx = m[0].max(m[1]).max(m[2]);
            if m[1] < 0.02 || mx > 0.85 {
                continue;
            }
            sr += m[0] as f64;
            sg += m[1] as f64;
            sb += m[2] as f64;
        }
        if sr <= 0.0 || sb <= 0.0 || sg <= 0.0 {
            return;
        }
        let gr = ((sg / sr) as f32).clamp(0.5, 4.0);
        let gb = ((sg / sb) as f32).clamp(0.5, 4.0);
        let k = if self.frame < 4 { 1.0 } else { 0.15 };
        self.wb[0] += (gr - self.wb[0]) * k;
        self.wb[2] += (gb - self.wb[2]) * k;
        self.wb[1] = 1.0;
    }

    /// Средняя линейная яркость с весами (центр или точка касания) и доля пересвеченных точек.
    fn metered(&self) -> (f32, f32) {
        let range = self.range();
        let (px, py) = if self.ctl.x >= 0.0 && self.ctl.y >= 0.0 { (self.ctl.x, self.ctl.y) } else { (0.5, 0.5) };
        let spot = self.ctl.x >= 0.0;
        let (mut s, mut ws) = (0.0f32, 0.0f32);
        for zy in 0..ZY {
            for zx in 0..ZX {
                let z = zy * ZX + zx;
                let m = self.zones.mean(z, range);
                let y = 0.299 * m[0] * self.wb[0] + 0.587 * m[1] + 0.114 * m[2] * self.wb[2];
                let dx = (zx as f32 + 0.5) / ZX as f32 - px;
                let dy = (zy as f32 + 0.5) / ZY as f32 - py;
                let d2 = dx * dx + dy * dy;
                let wgt = if spot { (-d2 / 0.01).exp() * 8.0 + 0.2 } else { 1.0 + 2.0 * (-d2 / 0.06).exp() };
                s += y * wgt;
                ws += wgt;
            }
        }
        let sat = if self.zones.total > 0 { self.zones.sat as f32 / self.zones.total as f32 } else { 0.0 };
        (s / ws.max(1e-6), sat)
    }

    fn auto_exposure(&mut self) {
        // сенсор применяет новую экспозицию через 2–3 кадра: до этого кадры старые
        if self.ae_wait > 0 {
            self.ae_wait -= 1;
            return;
        }
        let l = self.lim;
        let a = self.ae;
        let (mean, sat) = self.metered();
        let bright = mean * a.isp;
        let target = AE_TARGET * 2f32.powf(self.ctl.ev as f32 / 3.0);
        let mut ratio = (target / bright.max(1e-4)).clamp(0.125, 8.0);
        if sat > 0.04 && bright > target * 0.5 {
            ratio = ratio.min(0.9);
        }
        self.ae_ok = (ratio - 1.0).abs() < 0.15;
        if std::env::var_os("SYNCAMERA_AE").is_some() {
            eprintln!("ae: кадр {} яркость {mean:.4} ×isp {:.2} пересвет {sat:.3} ratio {ratio:.2} | строк {} again {} dgain {} vblank {}", self.frame, a.isp, a.lines, a.again, a.dgain, a.vblank);
        }
        if (ratio - 1.0).abs() < 0.06 {
            return;
        }
        let total = a.lines as f32 * l.again_f(a.again) * l.dgain_f(a.dgain) * a.isp;
        // сходимость: большие шаги — сразу, у цели — мягко (без раскачки)
        let want = total * if (ratio - 1.0).abs() > 0.5 { ratio.clamp(0.25, 4.0) } else { ratio.powf(0.7) };
        // выдержка: до кадра 30 к/с, в фото — до 15 к/с
        let fast = l.lines_fast();
        let slow = if self.video { fast } else { l.lines_for_frame(1e9 / 15.0).max(fast) };
        let mut lines = (want as i32).clamp(l.exp_min(), fast);
        let mut rest = want / lines as f32;
        if rest > GAIN_BEFORE_SLOW && slow > fast {
            lines = (want as i32).clamp(l.exp_min(), slow);
            rest = want / lines as f32;
        }
        let again = l.again_code(rest.clamp(1.0, l.again_max()));
        rest /= l.again_f(again);
        let dgain = l.dgain_code(rest.clamp(1.0, l.dgain_max()));
        rest /= l.dgain_f(dgain);
        let isp = rest.clamp(1.0, ISP_GAIN_MAX);
        let vblank = self.vblank_for(lines);
        self.ae = Ae { lines, again, dgain, vblank, isp };
        self.ae_dirty = a.lines != lines || a.again != again || a.dgain != dgain || a.vblank != vblank;
        if self.ae_dirty {
            self.ae_wait = 3;
        }
    }

    /// Резкость в окне: сумма разностей соседей одного цвета, делённая на яркость.
    fn sharpness(&self) -> f64 {
        let (w, h, pw) = (self.w, self.h, self.w + 2);
        // окно — в долях видимой (при зуме — обрезанной) части
        let (x0, y0, cw, ch) = self.crop();
        let (rw, rh) = (cw / 5, ch / 5);
        let cx = (x0 + (self.af.roi.0 * cw as f32) as usize).clamp(rw / 2 + 2, w - rw / 2 - 2);
        let cy = (y0 + (self.af.roi.1 * ch as f32) as usize).clamp(rh / 2 + 2, h - rh / 2 - 2);
        let p = &self.plane;
        let (mut d, mut s) = (0u64, 0u64);
        for y in (cy - rh / 2..cy + rh / 2).step_by(2) {
            let r = (y + 1) * pw + 1;
            for x in cx - rw / 2..cx + rw / 2 {
                let i = r + x;
                let c = p[i] as i32;
                d += ((c - p[i + 2] as i32).unsigned_abs() + (c - p[i + 2 * pw] as i32).unsigned_abs()) as u64;
                s += c as u64;
            }
        }
        d as f64 / (s as f64 + 1.0)
    }

    fn autofocus(&mut self) {
        let Some((lo, hi, step, _)) = self.af.range else { return };
        if self.ctl.focus >= 0.0 {
            return;
        }
        match self.af.state {
            AfState::Idle => {
                // после того, как сошлась экспозиция (или через ~3 с)
                if (self.ae_ok && self.frame >= 6) || self.frame >= 90 || self.manual() {
                    self.start_af();
                }
            }
            AfState::Scan => {
                if self.af.settle > 0 {
                    self.af.settle -= 1;
                    return;
                }
                let sh = self.sharpness();
                self.af.results.push((self.af.pos, sh));
                self.af.idx += 1;
                if self.af.idx < self.af.positions.len() {
                    self.af.pos = self.af.positions[self.af.idx];
                    self.af.dirty = true;
                    self.af.settle = 1;
                    return;
                }
                let best = self.af.results.iter().cloned().fold((lo, f64::MIN), |b, r| if r.1 > b.1 { r } else { b });
                if !self.af.fine {
                    // точно — вокруг лучшей с шагом в четверть грубого
                    let coarse = if self.af.positions.len() > 1 { (self.af.positions[1] - self.af.positions[0]).abs() } else { step };
                    let st = (coarse / 4).max(step);
                    let mut v: Vec<i32> = (-3..=3).map(|k| (best.0 + k * st).clamp(lo, hi)).filter(|p| *p != best.0).collect();
                    v.dedup();
                    if !v.is_empty() {
                        self.af.fine = true;
                        self.af.positions = v;
                        self.af.idx = 0;
                        self.af.pos = self.af.positions[0];
                        self.af.dirty = true;
                        self.af.settle = 2; // переход к точному проходу — дальний ход линзы
                        return;
                    }
                }
                let min = self.af.results.iter().map(|r| r.1).fold(f64::MAX, f64::min);
                self.af.result = if best.1 > min * 1.08 { syncamd::AF_FOCUSED } else { syncamd::AF_FAILED };
                self.af.pos = best.0;
                self.af.dirty = true;
                self.af.state = AfState::Done;
                self.af.base = 0.0;
                self.af.low = 0;
                self.af.settle = 3;
                tracing::debug!("автофокус: {} (резкость {:.4}, мин {:.4})", best.0, best.1, min);
            }
            AfState::Done => {
                if self.ctl.ae_lock != 0 || self.ctl.x >= 0.0 {
                    return; // точка касания и блокировка держат фокус
                }
                if self.af.settle > 0 {
                    self.af.settle -= 1;
                    return;
                }
                let sh = self.sharpness();
                if self.af.base <= 0.0 {
                    self.af.base = sh;
                    return;
                }
                if sh < self.af.base * 0.6 {
                    self.af.low += 1;
                } else {
                    self.af.low = 0;
                    self.af.base = self.af.base * 0.95 + sh * 0.05;
                }
                if self.af.low >= 20 && self.ae_ok {
                    self.start_af();
                }
            }
        }
    }
}

/// Распаковать строки `y0..` в полосу плоскости (строки с полем, `chunk` — целые строки `w + 2`).
#[allow(clippy::too_many_arguments)]
fn unpack_rows(raw: &[u8], stride: usize, fmt: RawFmt, w: usize, h: usize, y0: usize, chunk: &mut [u8], lut: &[Vec<u8>; 3], black: u32, maxv: u32) -> Zones {
    let pw = w + 2;
    let mut z = Zones::new();
    let mut row = vec![0u16; w];
    let sat_thr = (maxv * 97 / 100) as u16;
    for (k, prow) in chunk.chunks_mut(pw).enumerate() {
        let y = y0 + k;
        if y >= h {
            break;
        }
        let src = &raw[y * stride..];
        match (fmt.bits, fmt.packed) {
            (10, true) => {
                for (q, px) in row.chunks_exact_mut(4).enumerate() {
                    let b = &src[q * 5..q * 5 + 5];
                    let lo = b[4] as u16;
                    px[0] = (b[0] as u16) << 2 | (lo & 3);
                    px[1] = (b[1] as u16) << 2 | (lo >> 2 & 3);
                    px[2] = (b[2] as u16) << 2 | (lo >> 4 & 3);
                    px[3] = (b[3] as u16) << 2 | (lo >> 6 & 3);
                }
            }
            (12, true) => {
                for (q, px) in row.chunks_exact_mut(2).enumerate() {
                    let b = &src[q * 3..q * 3 + 3];
                    px[0] = (b[0] as u16) << 4 | (b[2] as u16 & 0xf);
                    px[1] = (b[1] as u16) << 4 | (b[2] as u16 >> 4);
                }
            }
            (8, _) => {
                for (x, v) in row.iter_mut().enumerate() {
                    *v = src[x] as u16;
                }
            }
            _ => {
                for (x, v) in row.iter_mut().enumerate() {
                    *v = u16::from_le_bytes([src[2 * x], src[2 * x + 1]]) & maxv as u16;
                }
            }
        }
        let c0 = fmt.order.color(0, y) as usize;
        let c1 = fmt.order.color(1, y) as usize;
        let (l0, l1) = (&lut[c0], &lut[c1]);
        let out = &mut prow[1..=w];
        for x in (0..w).step_by(2) {
            out[x] = l0[row[x] as usize];
            out[x + 1] = l1[row[x + 1] as usize];
        }
        // статистика: строки чётных квадратов, точки чётных квадратов
        if (y >> 1) & 1 == 0 {
            let zy = y * ZY / h;
            for x in (0..w).step_by(4) {
                let zi = zy * ZX + x * ZX / w;
                for (dx, c) in [(0, c0), (1, c1)] {
                    let v = row[x + dx];
                    z.sum[zi][c] += v.saturating_sub(black as u16) as u64;
                    z.cnt[zi][c] += 1;
                    z.total += 1;
                    if v >= sat_thr {
                        z.sat += 1;
                    }
                }
            }
        }
    }
    z
}

#[inline(always)]
fn px(t: u8, p: &[u8], i: usize, pw: usize) -> (i32, i32, i32) {
    // SAFETY: у плоскости поле в 1 точку со всех сторон: i ± 1 и i ± pw внутри
    unsafe {
        let g = |j: usize| *p.get_unchecked(j) as i32;
        let c = g(i);
        match t {
            T_R => ((c), (g(i - 1) + g(i + 1) + g(i - pw) + g(i + pw) + 2) >> 2, (g(i - pw - 1) + g(i - pw + 1) + g(i + pw - 1) + g(i + pw + 1) + 2) >> 2),
            T_B => ((g(i - pw - 1) + g(i - pw + 1) + g(i + pw - 1) + g(i + pw + 1) + 2) >> 2, (g(i - 1) + g(i + 1) + g(i - pw) + g(i + pw) + 2) >> 2, c),
            T_GR => ((g(i - 1) + g(i + 1) + 1) >> 1, c, (g(i - pw) + g(i + pw) + 1) >> 1),
            _ => ((g(i - pw) + g(i + pw) + 1) >> 1, c, (g(i - 1) + g(i + 1) + 1) >> 1),
        }
    }
}

/// Квадратные строки `q0..` области (`x0`, `y0` — чётные, ширина `w`) → строки Y (`yc`) и UV (`uvc`) NV12.
#[allow(clippy::too_many_arguments)]
fn demosaic_rows(p: &[u8], pw: usize, x0: usize, y0: usize, w: usize, q0: usize, yc: &mut [u8], uvc: &mut [u8], order: Bayer, sat: i32) {
    let t = [[order.kind(0, 0), order.kind(1, 0)], [order.kind(0, 1), order.kind(1, 1)]];
    let nq = uvc.len() / w;
    for k in 0..nq {
        let y = y0 + 2 * (q0 + k);
        let b0 = (y + 1) * pw + 1 + x0;
        let b1 = b0 + pw;
        let (y0r, y1r) = yc[2 * k * w..(2 * k + 2) * w].split_at_mut(w);
        let uvr = &mut uvc[k * w..(k + 1) * w];
        for x in (0..w).step_by(2) {
            let (r00, g00, bb00) = px(t[0][0], p, b0 + x, pw);
            let (r01, g01, bb01) = px(t[0][1], p, b0 + x + 1, pw);
            let (r10, g10, bb10) = px(t[1][0], p, b1 + x, pw);
            let (r11, g11, bb11) = px(t[1][1], p, b1 + x + 1, pw);
            let luma = |r: i32, g: i32, b: i32| ((77 * r + 150 * g + 29 * b + 128) >> 8) as u8;
            y0r[x] = luma(r00, g00, bb00);
            y0r[x + 1] = luma(r01, g01, bb01);
            y1r[x] = luma(r10, g10, bb10);
            y1r[x + 1] = luma(r11, g11, bb11);
            let (rs, gs, bs) = (r00 + r01 + r10 + r11, g00 + g01 + g10 + g11, bb00 + bb01 + bb10 + bb11);
            // ×1/4 (сумма) × sat/256 × коэффициент/256
            let u = ((-43 * rs - 85 * gs + 128 * bs) >> 2) * sat >> 16;
            let v = ((128 * rs - 107 * gs - 21 * bs) >> 2) * sat >> 16;
            uvr[x] = (128 + u).clamp(0, 255) as u8;
            uvr[x + 1] = (128 + v).clamp(0, 255) as u8;
        }
    }
}

/// Билинейное увеличение NV12 `sw × sh` → `dw × dh` (Y — по точкам, UV — по парам).
fn upscale_nv12(src: &[u8], sw: usize, sh: usize, dst: &mut [u8], dw: usize, dh: usize, threads: usize) {
    let (sy, suv) = src.split_at(sw * sh);
    let (dy, duv) = dst.split_at_mut(dw * dh);
    // плоскость: (данные, ширина в элементах, высота, элементов на точку)
    fn plane(s: &[u8], sw: usize, sh: usize, d: &mut [u8], dw: usize, dh: usize, ch: usize, threads: usize) {
        let fx = ((sw - 1) << 16) / (dw - 1).max(1);
        let fy = ((sh - 1) << 16) / (dh - 1).max(1);
        let rows = dh.div_ceil(threads).max(1);
        std::thread::scope(|sc| {
            for (k, part) in d.chunks_mut(rows * dw * ch).enumerate() {
                sc.spawn(move || {
                    for (r, row) in part.chunks_mut(dw * ch).enumerate() {
                        let yy = (k * rows + r) * fy;
                        let (y0, wy) = (yy >> 16, (yy & 0xffff) as u32 >> 8);
                        let y1 = (y0 + 1).min(sh - 1);
                        let (a, b) = (&s[y0 * sw * ch..(y0 + 1) * sw * ch], &s[y1 * sw * ch..(y1 + 1) * sw * ch]);
                        for x in 0..dw {
                            let xx = x * fx;
                            let (x0, wx) = (xx >> 16, (xx & 0xffff) as u32 >> 8);
                            let x1 = (x0 + 1).min(sw - 1);
                            for c in 0..ch {
                                let (p00, p01) = (a[x0 * ch + c] as u32, a[x1 * ch + c] as u32);
                                let (p10, p11) = (b[x0 * ch + c] as u32, b[x1 * ch + c] as u32);
                                let top = p00 * (256 - wx) + p01 * wx;
                                let bot = p10 * (256 - wx) + p11 * wx;
                                row[x * ch + c] = ((top * (256 - wy) + bot * wy + 32768) >> 16) as u8;
                            }
                        }
                    }
                });
            }
        });
    }
    plane(sy, sw, sh, dy, dw, dh, 1, threads);
    plane(suv, sw / 2, sh / 2, duv, dw / 2, dh / 2, 2, threads);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lim() -> Limits {
        Limits {
            exposure: Some((6, 2524, 1, 2524)),
            analogue: Some((0, 240, 8, 0)),
            digital: Some((512, 8191, 1, 512)),
            vblank: Some((1302, 64311, 1, 1302)),
            height: 1224,
            line_ns: 3800.0 * 1e9 / 288e6,
            margin: 2,
        }
    }

    #[test]
    fn gains() {
        let l = lim();
        assert_eq!(l.again_f(240), 16.0);
        assert_eq!(l.again_code(16.0), 240);
        assert_eq!(l.again_code(2.0), 16);
        assert_eq!(l.dgain_f(1024), 2.0);
        assert_eq!(l.lines_fast(), 2524);
        // 15 к/с — вдвое длиннее
        assert!((l.lines_for_frame(1e9 / 15.0) - 5050).abs() < 5);
    }

    /// Ровный серый кадр RAW10 GBRG → ровный серый NV12 (цветность ~128).
    #[test]
    fn flat_gray() {
        let (w, h) = (64usize, 48usize);
        let fmt = RawFmt { order: Bayer::Gbrg, bits: 10, packed: true };
        let stride = w * 5 / 4;
        let mut raw = vec![0u8; stride * h];
        // R и B вдвое темнее G (баланс белого ×2 их выровняет)
        for y in 0..h {
            for q in 0..w / 4 {
                let mut lo = 0u8;
                for i in 0..4 {
                    let x = q * 4 + i;
                    let v: u16 = if fmt.order.color(x, y) == G { 64 + 400 } else { 64 + 200 };
                    raw[y * stride + q * 5 + i] = (v >> 2) as u8;
                    lo |= ((v & 3) as u8) << (2 * i);
                }
                raw[y * stride + q * 5 + 4] = lo;
            }
        }
        let mut isp = Isp::new("test-flat", w as u32, h as u32, None, fmt, lim(), None, false);
        let mut out = vec![0u8; w * h * 3 / 2];
        for _ in 0..3 {
            isp.process(&raw, stride, &mut out);
        }
        assert!((isp.wb[0] - 2.0).abs() < 0.05 && (isp.wb[2] - 2.0).abs() < 0.05, "wb {:?}", isp.wb);
        isp.process(&raw, stride, &mut out);
        let y = out[(h / 2) * w + w / 2];
        let (u, v) = (out[w * h + (h / 4) * w + w / 2], out[w * h + (h / 4) * w + w / 2 + 1]);
        assert!((u as i32 - 128).abs() <= 2 && (v as i32 - 128).abs() <= 2, "uv {u} {v}");
        assert!(y > 100, "y {y}");
        // края — тоже без цветных полос
        let (u0, v0) = (out[w * h], out[w * h + 1]);
        assert!((u0 as i32 - 128).abs() <= 2 && (v0 as i32 - 128).abs() <= 2, "edge uv {u0} {v0}");
    }

    #[test]
    fn zoom_keeps_size_and_gray() {
        let (w, h) = (64u32, 48u32);
        let fmt = RawFmt { order: Bayer::Rggb, bits: 10, packed: false };
        let mut isp = Isp::new("test-zoom", w, h, None, fmt, lim(), None, false);
        let raw: Vec<u8> = (0..w * h).flat_map(|_| 300u16.to_le_bytes()).collect();
        let mut out = vec![0u8; (w * h * 3 / 2) as usize];
        let mut c = Control::default();
        c.mask = syncamd::CTL_ZOOM | syncamd::CTL_AWB;
        c.zoom = 2.0;
        c.awb = 5; // дневной — без подстройки
        isp.control(&c);
        isp.wb = [1.0, 1.0, 1.0];
        assert_eq!(isp.crop(), (16, 12, 32, 24));
        isp.process(&raw, w as usize * 2, &mut out);
        let y = &out[..(w * h) as usize];
        assert!(y.iter().all(|v| (*v as i32 - y[0] as i32).abs() <= 1));
    }

    #[test]
    fn aspect_crop_16_9() {
        let fmt = RawFmt { order: Bayer::Gbrg, bits: 10, packed: false };
        let mut isp = Isp::new("test-aspect", 64, 48, Some((64, 36)), fmt, lim(), None, true);
        assert_eq!(isp.out_size(), (64, 36));
        assert_eq!(isp.crop(), (0, 6, 64, 36));
        let raw: Vec<u8> = (0..64 * 48).flat_map(|_| 300u16.to_le_bytes()).collect();
        let mut out = vec![0u8; 64 * 36 * 3 / 2];
        isp.process(&raw, 128, &mut out);
    }

    #[test]
    fn ae_brightens_dark() {
        let (w, h) = (64u32, 48u32);
        let fmt = RawFmt { order: Bayer::Gbrg, bits: 10, packed: false };
        let mut isp = Isp::new("test-ae", w, h, None, fmt, lim(), None, false);
        let raw: Vec<u8> = (0..w * h).flat_map(|_| 70u16.to_le_bytes()).collect();
        let mut out = vec![0u8; (w * h * 3 / 2) as usize];
        let before = isp.ae.lines as f32 * isp.lim.again_f(isp.ae.again);
        // первые кадры — ожидание сенсора
        for _ in 0..5 {
            isp.process(&raw, w as usize * 2, &mut out);
        }
        let after = isp.ae.lines as f32 * isp.lim.again_f(isp.ae.again) * isp.lim.dgain_f(isp.ae.dgain) * isp.ae.isp;
        assert!(after > before * 2.0, "{before} → {after}");
    }
}
