//! Датчики для Android — `syndroidd __sensors`: HIDL-сервис `android.hardware.sensors@1.0::ISensors/default`
//! по hwbinder контейнера (его объявляет VINTF vendor-образа Waydroid; заглушку HAL в образе выключает свойство
//! `waydroid.stub_sensors_hal=0`). Данные — от источника платформы `/usr/lib/syndroid/sensors-source`
//! (строки `A x y z`, `G …`, `M …`, `L lux`, `P 0|1`; `--list` — какие датчики есть). Источник запускается
//! только для включённых Android датчиков — SLPI не опрашивает их зря.
//!
//! Раскладки HIDL (C, 64 бит): `SensorInfo` — 112 байт (строки — `hidl_string` по 16 байт на смещениях 8, 24,
//! 48, 88), `Event` — 80 байт (timestamp 0, handle 8, type 12, payload 16: Vec3 {x,y,z,status:int8} или scalar).

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};

use crate::hwbinder::{self, hidl_header, EventQueue, HwParcel, HwReader, Service};

pub const SOURCE: &str = "/usr/lib/syndroid/sensors-source";
const ISENSORS: &str = "android.hardware.sensors@1.0::ISensors";

// Типы и флаги HAL
const TYPE_META_DATA: i32 = 0;
const META_FLUSH_COMPLETE: i32 = 1;
const FLAG_WAKE_UP: u32 = 1;
const FLAG_ON_CHANGE: u32 = 2;
const STATUS_ACCURACY_HIGH: u8 = 3;
const RESULT_OK: i32 = 0;
const RESULT_BAD_VALUE: i32 = -22;
const RESULT_INVALID_OPERATION: i32 = -38;

/// Описание датчика: имя у источника, handle, тип HAL и свойства.
struct Def {
    key: &'static str,
    handle: i32,
    kind: i32,
    name: &'static str,
    type_str: &'static str,
    max_range: f32,
    resolution: f32,
    power: f32,
    min_delay_us: i32,
    max_delay_us: i32,
    flags: u32,
}

const DEFS: &[Def] = &[
    Def { key: "accel", handle: 1, kind: 1, name: "Accelerometer", type_str: "android.sensor.accelerometer", max_range: 78.4532, resolution: 0.0024, power: 0.17, min_delay_us: 5000, max_delay_us: 1_000_000, flags: 0 },
    Def { key: "gyro", handle: 2, kind: 4, name: "Gyroscope", type_str: "android.sensor.gyroscope", max_range: 34.906586, resolution: 0.0011, power: 0.5, min_delay_us: 5000, max_delay_us: 1_000_000, flags: 0 },
    Def { key: "mag", handle: 3, kind: 2, name: "Magnetometer", type_str: "android.sensor.magnetic_field", max_range: 4912.0, resolution: 0.15, power: 0.6, min_delay_us: 20000, max_delay_us: 1_000_000, flags: 0 },
    Def { key: "light", handle: 4, kind: 5, name: "Ambient Light", type_str: "android.sensor.light", max_range: 65535.0, resolution: 1.0, power: 0.1, min_delay_us: 0, max_delay_us: 0, flags: FLAG_ON_CHANGE },
    Def { key: "prox", handle: 5, kind: 8, name: "Proximity", type_str: "android.sensor.proximity", max_range: 5.0, resolution: 5.0, power: 0.1, min_delay_us: 0, max_delay_us: 0, flags: FLAG_ON_CHANGE | FLAG_WAKE_UP },
];

const VENDOR: &str = "syndroid (Qualcomm SSC)";

fn def_by_handle(h: i32) -> Option<&'static Def> {
    DEFS.iter().find(|d| d.handle == h)
}

fn boottime_ns() -> i64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut ts) };
    ts.tv_sec * 1_000_000_000 + ts.tv_nsec
}

fn event(handle: i32, kind: i32, ts: i64, payload: &[u8]) -> [u8; 80] {
    let mut e = [0u8; 80];
    e[0..8].copy_from_slice(&ts.to_le_bytes());
    e[8..12].copy_from_slice(&handle.to_le_bytes());
    e[12..16].copy_from_slice(&kind.to_le_bytes());
    e[16..16 + payload.len()].copy_from_slice(payload);
    e
}

fn vec3(x: f32, y: f32, z: f32) -> Vec<u8> {
    let mut p = Vec::with_capacity(13);
    for v in [x, y, z] {
        p.extend_from_slice(&v.to_le_bytes());
    }
    p.push(STATUS_ACCURACY_HIGH);
    p
}

struct State {
    /// Датчики, что есть у источника.
    available: Vec<&'static Def>,
    /// Включённые Android: handle → период, нс.
    active: HashMap<i32, i64>,
    /// Последнее отданное событие handle (для прореживания по периоду).
    last: HashMap<i32, i64>,
    child: Option<Child>,
    /// Набор, с которым запущен источник.
    running: Vec<&'static str>,
}

pub struct Sensors {
    st: Mutex<State>,
    events: Arc<EventQueue<[u8; 80]>>,
    me: Mutex<Option<std::sync::Weak<Sensors>>>,
}

impl Sensors {
    /// (Пере)запустить источник под набор включённых датчиков.
    fn reconcile(self: &Arc<Self>, st: &mut State) {
        let mut want: Vec<&'static str> = st.available.iter().filter(|d| st.active.contains_key(&d.handle)).map(|d| d.key).collect();
        want.sort();
        if want == st.running {
            return;
        }
        if let Some(mut c) = st.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        st.running = want.clone();
        if want.is_empty() {
            tracing::info!("датчики: все выключены");
            return;
        }
        tracing::info!("датчики: источник для {}", want.join(" "));
        let child = Command::new(SOURCE).args(&want).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("датчики: {SOURCE}: {e}");
                return;
            }
        };
        let out = child.stdout.take();
        st.child = Some(child);
        let me = Arc::downgrade(self);
        if let Some(out) = out {
            std::thread::spawn(move || {
                for line in BufReader::new(out).lines() {
                    let Ok(line) = line else { break };
                    let Some(s) = me.upgrade() else { break };
                    s.on_line(&line);
                }
            });
        }
    }

    fn on_line(&self, line: &str) {
        let mut it = line.split_whitespace();
        let Some(tag) = it.next() else { return };
        let v: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
        let (key, payload) = match (tag, v.as_slice()) {
            ("A", [x, y, z]) => ("accel", vec3(*x, *y, *z)),
            ("G", [x, y, z]) => ("gyro", vec3(*x, *y, *z)),
            ("M", [x, y, z]) => ("mag", vec3(*x, *y, *z)),
            ("L", [lux]) => ("light", lux.to_le_bytes().to_vec()),
            // Приближение — расстояние, см: близко — 0, далеко — максимум
            ("P", [near]) => ("prox", (if *near != 0.0 { 0.0f32 } else { 5.0f32 }).to_le_bytes().to_vec()),
            _ => return,
        };
        let Some(d) = DEFS.iter().find(|d| d.key == key) else { return };
        let ts = boottime_ns();
        {
            let mut st = self.st.lock().unwrap();
            let Some(&period) = st.active.get(&d.handle) else { return };
            // Непрерывные — не чаще запрошенного периода (источник может давать больше)
            if d.flags & FLAG_ON_CHANGE == 0 {
                if let Some(&l) = st.last.get(&d.handle) {
                    if ts - l < period * 9 / 10 {
                        return;
                    }
                }
            }
            st.last.insert(d.handle, ts);
        }
        self.events.push(event(d.handle, d.kind, ts, &payload));
    }

    fn arc(&self) -> Option<Arc<Sensors>> {
        self.me.lock().unwrap().as_ref().and_then(|w| w.upgrade())
    }

    fn sensor_list(&self, reply: &mut HwParcel) {
        let avail = self.st.lock().unwrap().available.clone();
        let mut elems = Vec::with_capacity(avail.len() * 112);
        for d in &avail {
            let mut b = [0u8; 112];
            b[0..4].copy_from_slice(&d.handle.to_le_bytes());
            b[8..24].copy_from_slice(&hidl_header(d.name.len()));
            b[24..40].copy_from_slice(&hidl_header(VENDOR.len()));
            b[40..44].copy_from_slice(&1i32.to_le_bytes());
            b[44..48].copy_from_slice(&d.kind.to_le_bytes());
            b[48..64].copy_from_slice(&hidl_header(d.type_str.len()));
            b[64..68].copy_from_slice(&d.max_range.to_le_bytes());
            b[68..72].copy_from_slice(&d.resolution.to_le_bytes());
            b[72..76].copy_from_slice(&d.power.to_le_bytes());
            b[76..80].copy_from_slice(&d.min_delay_us.to_le_bytes());
            // fifoReserved/Max = 0; requiredPermission — пустая строка
            b[88..104].copy_from_slice(&hidl_header(0));
            b[104..108].copy_from_slice(&d.max_delay_us.to_le_bytes());
            b[108..112].copy_from_slice(&d.flags.to_le_bytes());
            elems.extend_from_slice(&b);
        }
        reply.ok();
        let data = reply.vec(elems, avail.len());
        for (i, d) in avail.iter().enumerate() {
            let base = (i * 112) as u64;
            reply.embedded_string(d.name, data, base + 8);
            reply.embedded_string(VENDOR, data, base + 24);
            reply.embedded_string(d.type_str, data, base + 48);
            reply.embedded_string("", data, base + 88);
        }
    }
}

impl Service for Sensors {
    fn chain(&self) -> &'static [&'static str] {
        &[ISENSORS]
    }

    fn call(&self, code: u32, r: &mut HwReader, reply: &mut HwParcel) -> Result<()> {
        match code {
            // getSensorsList() generates (vec<SensorInfo>)
            1 => self.sensor_list(reply),
            // setOperationMode(OperationMode) → Result
            2 => {
                let mode = r.i32()?;
                reply.ok();
                reply.i32(if mode == 0 { RESULT_OK } else { RESULT_INVALID_OPERATION });
            }
            // activate(int32 handle, bool enabled) → Result
            3 => {
                let (h, on) = (r.i32()?, r.bool()?);
                let res = match (def_by_handle(h), self.arc()) {
                    (Some(d), Some(me)) => {
                        let mut st = me.st.lock().unwrap();
                        if on {
                            let p = (d.min_delay_us.max(20_000) as i64) * 1000;
                            st.active.entry(h).or_insert(p);
                        } else {
                            st.active.remove(&h);
                            st.last.remove(&h);
                        }
                        me.reconcile(&mut st);
                        RESULT_OK
                    }
                    _ => RESULT_BAD_VALUE,
                };
                reply.ok();
                reply.i32(res);
            }
            // poll(int32 maxCount) generates (Result, vec<Event> data, vec<SensorInfo> dynamicSensorsAdded)
            4 => {
                // SensorService зовёт poll и синхронно при инициализации (поток system-server-init): блок
                // «до первого события» вешал system_server (сторож — через 60 с). Поэтому — не дольше 0,5 с;
                // пустой ответ его цикл опроса просто повторит
                let max = r.i32()?.max(0) as usize;
                let evs = if max == 0 { Vec::new() } else { self.events.take(max, std::time::Duration::from_millis(500)) };
                let mut bytes = Vec::with_capacity(evs.len() * 80);
                for e in &evs {
                    bytes.extend_from_slice(e);
                }
                reply.ok();
                reply.i32(RESULT_OK);
                reply.vec(bytes, evs.len());
                reply.vec(Vec::new(), 0);
            }
            // batch(int32 handle, int64 samplingPeriodNs, int64 maxReportLatencyNs) → Result
            5 => {
                let (h, period, _lat) = (r.i32()?, r.i64()?, r.i64()?);
                let res = match def_by_handle(h) {
                    Some(d) => {
                        let p = period.max(d.min_delay_us as i64 * 1000);
                        let mut st = self.st.lock().unwrap();
                        if let Some(v) = st.active.get_mut(&h) {
                            *v = p;
                        } else {
                            // batch до activate — запомнить период на будущее включение
                            st.last.remove(&h);
                        }
                        RESULT_OK
                    }
                    None => RESULT_BAD_VALUE,
                };
                reply.ok();
                reply.i32(res);
            }
            // flush(int32 handle) → Result; отвечаем событием META_DATA FLUSH_COMPLETE
            6 => {
                let h = r.i32()?;
                if def_by_handle(h).is_some() {
                    self.events.push(event(h, TYPE_META_DATA, 0, &META_FLUSH_COMPLETE.to_le_bytes()));
                    reply.ok();
                    reply.i32(RESULT_OK);
                } else {
                    reply.ok();
                    reply.i32(RESULT_BAD_VALUE);
                }
            }
            // injectSensorData, unregisterDirectChannel → Result
            7 | 9 => {
                reply.ok();
                reply.i32(RESULT_INVALID_OPERATION);
            }
            // registerDirectChannel, configDirectReport → (Result, int32)
            8 | 10 => {
                reply.ok();
                reply.i32(RESULT_INVALID_OPERATION);
                reply.i32(-1);
            }
            _ => bail!("неизвестный метод ISensors {code}"),
        }
        Ok(())
    }
}

/// Какие датчики есть у источника платформы (`--list`).
fn available() -> Vec<&'static Def> {
    let Ok(out) = Command::new(SOURCE).arg("--list").stderr(Stdio::null()).output() else { return Vec::new() };
    let text = String::from_utf8_lossy(&out.stdout);
    let keys: HashSet<&str> = text.lines().find_map(|l| l.strip_prefix("sensors")).unwrap_or("").split_whitespace().collect();
    DEFS.iter().filter(|d| keys.contains(d.key)).collect()
}

/// `syndroidd __sensors <узел hwbinder>`
pub fn sensors_main(args: &[String]) -> ! {
    let dev = args.first().cloned().unwrap_or_default();
    let r = (|| -> Result<()> {
        let avail = available();
        if avail.is_empty() {
            bail!("у источника {SOURCE} нет датчиков");
        }
        tracing::info!("датчики Android: {}", avail.iter().map(|d| d.key).collect::<Vec<_>>().join(" "));
        let s = Arc::new(Sensors {
            st: Mutex::new(State { available: avail, active: HashMap::new(), last: HashMap::new(), child: None, running: Vec::new() }),
            events: Arc::new(EventQueue::default()),
            me: Mutex::new(None),
        });
        *s.me.lock().unwrap() = Some(Arc::downgrade(&s));
        hwbinder::serve(&dev, "default", s).context("ISensors")
    })();
    if let Err(e) = r {
        eprintln!("syndroid: датчики: {e:#}");
        std::process::exit(1);
    }
    std::process::exit(0)
}
