//! GNSS модема: служба QMI LOC. Приёмник работает, пока есть читатели — поток `GnssWatch` протокола или
//! сокет NMEA [`NMEA_SOCKET`] (для GeoClue). Перед сеансом модему передаются время системы и данные XTRA
//! (орбиты спутников на неделю вперёд с серверов Qualcomm) — без них холодный старт ищет спутники минутами.

use std::collections::VecDeque;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, bail, Context, Result};

use crate::api::{Fix, Satellite};
use crate::qmi::{svc, Client, Indication, Message, QmiError, SERVICE_GONE};
use synshell_tr::n_;

/// Сокет NMEA: каждый подключившийся включает приёмник и получает GGA/RMC/GSA раз в секунду.
pub const NMEA_SOCKET: &str = "/run/synmodem/gnss.nmea";
/// Запрет сна syn-sleepd, пока приёмник работает (навигация не должна прерываться сном).
const SLEEP_INHIBIT: &str = "/run/syn-sleep/inhibit.d/gnss";
const XTRA_FILE: &str = "/var/lib/synmodem/xtra.bin";
/// Серверы XTRA, если модем не назвал свои.
const XTRA_SERVERS: [&str; 3] = [
    "https://path1.xtracloud.net/xtra3Mgrbeji.bin",
    "https://path2.xtracloud.net/xtra3Mgrbeji.bin",
    "https://path3.xtracloud.net/xtra3Mgrbeji.bin",
];
/// Скачанный XTRA обновлять раз в сутки (действует неделю).
const XTRA_REFRESH: Duration = Duration::from_secs(24 * 3600);
const XTRA_MAX_AGE: Duration = Duration::from_secs(6 * 24 * 3600);

const T: Duration = Duration::from_secs(10);
const SESSION: u8 = 1;
/// Решение старше — «нет решения».
const STALE: Duration = Duration::from_secs(5);

// Сообщения LOC (запрос и одноимённая индикация с результатом)
const REG_EVENTS: u16 = 0x21;
const START: u16 = 0x22;
const STOP: u16 = 0x23;
const IND_POSITION: u16 = 0x24;
const IND_SV_INFO: u16 = 0x25;
const IND_INJECT_TIME: u16 = 0x28;
const IND_INJECT_XTRA: u16 = 0x29;
const IND_ENGINE_STATE: u16 = 0x2B;
const IND_FIX_SESSION: u16 = 0x2C;
/// Inject Predicted Orbits Data — у новых модемов (SM8450) NotSupported, вместо него Inject XTRA Data.
const INJECT_ORBITS: u16 = 0x35;
const INJECT_XTRA: u16 = 0xA7;
const XTRA_VALIDITY: u16 = 0x37;
const INJECT_TIME: u16 = 0x38;
const SET_ENGINE_LOCK: u16 = 0x3A;
const GET_ENGINE_LOCK: u16 = 0x3B;
const SET_OPERATION_MODE: u16 = 0x4A;

// Маска событий Register Events
const EV_POSITION: u64 = 1 << 0;
const EV_SV_INFO: u64 = 1 << 1;
const EV_INJECT_TIME: u64 = 1 << 4;
const EV_INJECT_XTRA: u64 = 1 << 5;
const EV_ENGINE_STATE: u64 = 1 << 7;
const EV_FIX_SESSION: u64 = 1 << 8;

#[derive(Default)]
struct State {
    users: usize,
    fix: Fix,
    /// Когда пришло последнее решение.
    last_fix: Option<Instant>,
    /// Спутники последнего решения.
    used: Vec<u16>,
    subs: Vec<Sender<Fix>>,
}

pub struct Engine {
    st: Mutex<State>,
    cv: Condvar,
    on_active: Box<dyn Fn(bool) + Send + Sync>,
}

/// Читатель местоположения: пока жив, приёмник работает; в `rx` — решение раз в секунду.
pub struct Watch {
    engine: Arc<Engine>,
    pub rx: Receiver<Fix>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.engine.st.lock().unwrap().users -= 1;
        self.engine.cv.notify_all();
    }
}

impl Engine {
    /// Запустить потоки приёмника и сокет NMEA; `on_active` — приёмник включился/выключился.
    pub fn start(on_active: impl Fn(bool) + Send + Sync + 'static) -> Arc<Self> {
        let e = Arc::new(Self {
            st: Mutex::default(),
            cv: Condvar::new(),
            on_active: Box::new(on_active),
        });
        let r = e.clone();
        std::thread::Builder::new()
            .name("gnss".into())
            .spawn(move || r.run())
            .expect("поток gnss");
        let r = e.clone();
        std::thread::Builder::new()
            .name("gnss-tick".into())
            .spawn(move || r.ticker())
            .expect("поток gnss");
        let r = e.clone();
        std::thread::Builder::new()
            .name("gnss-nmea".into())
            .spawn(move || {
                if let Err(e) = r.serve_nmea() {
                    tracing::warn!("сокет NMEA: {e:#}");
                }
            })
            .expect("поток gnss");
        e
    }

    pub fn watch(self: &Arc<Self>) -> Watch {
        let (tx, rx) = mpsc::channel();
        let mut s = self.st.lock().unwrap();
        s.users += 1;
        s.subs.push(tx);
        self.cv.notify_all();
        Watch {
            engine: self.clone(),
            rx,
        }
    }

    fn users(&self) -> usize {
        self.st.lock().unwrap().users
    }

    /// Раз в секунду — текущее решение всем читателям (и признак ушедших).
    fn ticker(&self) {
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let mut s = self.st.lock().unwrap();
            if s.subs.is_empty() {
                continue;
            }
            if s.last_fix.is_none_or(|t| t.elapsed() > STALE) {
                s.fix.valid = false;
            }
            let fix = s.fix.clone();
            s.subs.retain(|tx| tx.send(fix.clone()).is_ok());
        }
    }

    fn run(&self) {
        loop {
            {
                let mut s = self.st.lock().unwrap();
                while s.users == 0 {
                    s = self.cv.wait(s).unwrap();
                }
            }
            (self.on_active)(true);
            let _ = std::fs::create_dir_all(std::path::Path::new(SLEEP_INHIBIT).parent().unwrap());
            let _ = std::fs::write(SLEEP_INHIBIT, b"");
            tracing::info!("GNSS: приёмник включён");
            while self.users() > 0 {
                if let Err(e) = self.session() {
                    tracing::warn!("GNSS: {e:#}");
                    let s = self.st.lock().unwrap();
                    let _ = self
                        .cv
                        .wait_timeout_while(s, Duration::from_secs(5), |s| s.users > 0);
                }
            }
            {
                let mut s = self.st.lock().unwrap();
                s.fix = Fix::default();
                s.last_fix = None;
                s.used.clear();
            }
            let _ = std::fs::remove_file(SLEEP_INHIBIT);
            (self.on_active)(false);
            tracing::info!("GNSS: приёмник выключен");
        }
    }

    /// Один сеанс определения местоположения, пока есть читатели.
    fn session(&self) -> Result<()> {
        let (tx, rx): (Sender<Indication>, Receiver<Indication>) = mpsc::channel();
        let c = Client::connect(svc::LOC, Duration::from_secs(30), tx)?;
        let mut l = Link {
            c: &c,
            rx,
            stash: VecDeque::new(),
        };
        let mask = EV_POSITION
            | EV_SV_INFO
            | EV_INJECT_TIME
            | EV_INJECT_XTRA
            | EV_ENGINE_STATE
            | EV_FIX_SESSION;
        // Тип клиента (0x11) обязателен у LOC ревизии 0x98 (SM8450): без него InvalidId
        c.call(
            Message::new(REG_EVENTS)
                .tlv(0x01, mask.to_le_bytes().to_vec())
                .u32(0x11, 1),
            T,
        )
        .context("LOC Register Events")?;
        unlock_engine(&mut l);
        // Самостоятельно, без SUPL: помощь — время и XTRA
        if let Err(e) = c.call(Message::new(SET_OPERATION_MODE).u32(0x01, 4), T) {
            tracing::debug!("GNSS: режим standalone: {e:#}");
        }
        inject_time(&c);
        // Модем просит XTRA (0x29) раз за запуск — истекающие данные обновить самим
        let left = xtra_left(&mut l);
        if left.is_none_or(|s| s < 12 * 3600) {
            tracing::info!("GNSS: XTRA действует ещё {left:?} с — обновить");
            if let Err(e) = inject_xtra(&mut l, &Message::new(IND_INJECT_XTRA)) {
                tracing::warn!("GNSS: XTRA: {e:#}");
            }
        }
        // Периодически, высокая точность, промежуточные решения, раз в секунду
        c.call(
            Message::new(START)
                .u8(0x01, SESSION)
                .u32(0x10, 1)
                .u32(0x11, 3)
                .u32(0x12, 1)
                .u32(0x13, 1000),
            T,
        )
        .context("LOC Start")?;
        tracing::info!("GNSS: сеанс начат");
        let result = loop {
            if self.users() == 0 {
                break Ok(());
            }
            match l.next(Duration::from_secs(1)) {
                Ok(m) if m.id == SERVICE_GONE => break Err(anyhow!("служба LOC ушла")),
                Ok(m) => self.indication(&mut l, &m),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Err(anyhow!("клиент LOC закрыт"))
                }
            }
        };
        let _ = c.call(Message::new(STOP).u8(0x01, SESSION), T);
        result
    }

    fn indication(&self, l: &mut Link, m: &Message) {
        match m.id {
            IND_POSITION => self.position(m),
            IND_SV_INFO => self.sv_info(m),
            IND_INJECT_TIME => inject_time(l.c),
            IND_INJECT_XTRA => {
                if let Err(e) = inject_xtra(l, m) {
                    tracing::warn!("GNSS: XTRA: {e:#}");
                }
            }
            IND_ENGINE_STATE => tracing::debug!("GNSS: двигатель {:?}", u32_at(m, 0x01)),
            IND_FIX_SESSION => tracing::debug!("GNSS: сеанс {:?}", u32_at(m, 0x01)),
            _ => tracing::trace!("GNSS: индикация 0x{:04x}", m.id),
        }
    }

    fn position(&self, m: &Message) {
        let Some(status) = u32_at(m, 0x01) else {
            return;
        };
        // 0 — решение, 1 — промежуточное; 7 — двигатель заблокирован
        if status == 7 {
            tracing::warn!("GNSS: двигатель заблокирован (engine lock)");
        }
        let (Some(lat), Some(lon)) = (f64_at(m, 0x10), f64_at(m, 0x11)) else {
            return;
        };
        // Промежуточные бывают и по соте (погрешность — сотни км): решением считать от километра
        let accuracy = f32_at(m, 0x12);
        if status > 1 || accuracy.is_some_and(|a| a > 1000.0) {
            return;
        }
        let mut used: Vec<u16> = m
            .get(0x2C)
            .and_then(|b| {
                let n = *b.first()? as usize;
                Some(
                    b.get(1..1 + 2 * n)?
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect(),
                )
            })
            .unwrap_or_default();
        // Номер повторяется по частотам спутника
        used.sort_unstable();
        used.dedup();
        let dop = m
            .get(0x24)
            .filter(|b| b.len() >= 12)
            .map(|b| (f32_of(&b[0..4]), f32_of(&b[4..8]), f32_of(&b[8..12])));
        let time_ms = m
            .get(0x25)
            .filter(|b| b.len() >= 8)
            .map(|b| u64::from_le_bytes(b[..8].try_into().unwrap()) as i64);
        let mut s = self.st.lock().unwrap();
        let f = &mut s.fix;
        f.valid = true;
        f.latitude = lat;
        f.longitude = lon;
        f.accuracy = accuracy;
        f.altitude_ellipsoid = f32_at(m, 0x1A);
        f.altitude = f32_at(m, 0x1B);
        f.vertical_accuracy = f32_at(m, 0x1C);
        f.speed = f32_at(m, 0x18);
        f.heading = f32_at(m, 0x20);
        (f.pdop, f.hdop, f.vdop) = match dop {
            Some((p, h, v)) => (Some(p), Some(h), Some(v)),
            None => (None, None, None),
        };
        f.time_ms = time_ms.unwrap_or_else(now_ms);
        f.satellites_used = used.len() as u32;
        for sat in &mut f.satellites {
            sat.used = used.contains(&sat.id);
        }
        s.used = used;
        s.last_fix = Some(Instant::now());
    }

    fn sv_info(&self, m: &Message) {
        // Список спутников: расширенный (0x11, запись + частота ГЛОНАСС u8) или прежний (0x10). Запись: маска
        // полей u32, система u32, номер u16, здоровье u8, состояние u32, маска сведений u8, возвышение, азимут,
        // сигнал/шум (f32). В списке и те, что только ищутся, — видимые те, у кого есть сигнал.
        let Some((b, rec)) = m
            .get(0x11)
            .map(|b| (b, 29))
            .or_else(|| m.get(0x10).map(|b| (b, 28)))
        else {
            return;
        };
        let Some(&n) = b.first() else { return };
        let mut sats = Vec::new();
        for k in 0..n as usize {
            let Some(r) = b.get(1 + k * rec..1 + (k + 1) * rec) else {
                break;
            };
            let valid = u32::from_le_bytes(r[0..4].try_into().unwrap());
            let system = u32::from_le_bytes(r[4..8].try_into().unwrap());
            let id = u16::from_le_bytes([r[8], r[9]]);
            let snr = if valid & 0x80 != 0 {
                f32_of(&r[24..28])
            } else {
                0.0
            };
            // Один спутник приходит записью на каждую частоту (L1, L5…) — оставить сильнейшую
            let name = system_name(system);
            if snr <= 0.0
                || sats
                    .iter()
                    .any(|o: &Satellite| o.id == id && o.system == name && o.snr >= snr)
            {
                continue;
            }
            sats.retain(|o| !(o.id == id && o.system == name));
            sats.push(Satellite {
                system: name.into(),
                id,
                elevation: if valid & 0x20 != 0 {
                    f32_of(&r[16..20])
                } else {
                    0.0
                },
                azimuth: if valid & 0x40 != 0 {
                    f32_of(&r[20..24])
                } else {
                    0.0
                },
                snr,
                used: false,
            });
        }
        let mut s = self.st.lock().unwrap();
        let fresh = s.last_fix.is_some_and(|t| t.elapsed() <= STALE);
        for sat in &mut sats {
            sat.used = fresh && s.used.contains(&sat.id);
        }
        s.fix.satellites_visible = sats.len() as u32;
        s.fix.satellites = sats;
    }

    fn serve_nmea(self: &Arc<Self>) -> Result<()> {
        let _ = std::fs::remove_file(NMEA_SOCKET);
        let l = UnixListener::bind(NMEA_SOCKET).context(NMEA_SOCKET)?;
        // Местоположение — только root и GeoClue (группа geoclue), если она есть
        match group_id("geoclue") {
            Some(gid) => {
                let p = std::ffi::CString::new(NMEA_SOCKET).unwrap();
                unsafe { libc::chown(p.as_ptr(), 0, gid) };
                std::fs::set_permissions(NMEA_SOCKET, std::fs::Permissions::from_mode(0o660))?;
            }
            None => std::fs::set_permissions(NMEA_SOCKET, std::fs::Permissions::from_mode(0o600))?,
        }
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let e = self.clone();
            std::thread::spawn(move || {
                let w = e.watch();
                for fix in w.rx.iter() {
                    if s.write_all(nmea(&fix).as_bytes()).is_err() {
                        break;
                    }
                }
            });
        }
        Ok(())
    }
}

/// Снять блокировку двигателя для запросов этого процессора (бывает выставлена в NV модема).
fn unlock_engine(l: &mut Link) {
    if l.c.call(Message::new(GET_ENGINE_LOCK), T).is_err() {
        return;
    }
    let Some(ind) = l.wait(GET_ENGINE_LOCK, T) else {
        return;
    };
    // 1 — не заблокирован, 2 — запросы AP, 3 — сети, 4 — все
    let lock = u32_at(&ind, 0x10);
    tracing::info!("GNSS: блокировка двигателя {lock:?}");
    if matches!(lock, Some(2 | 4)) {
        if let Err(e) = l.c.call(Message::new(SET_ENGINE_LOCK).u32(0x01, 1), T) {
            tracing::warn!("GNSS: снять блокировку: {e:#}");
        }
        let _ = l.wait(SET_ENGINE_LOCK, T);
    }
}

/// Время системы — модему, если оно сверено по сети.
fn inject_time(c: &Client) {
    let mut tx: libc::timex = unsafe { std::mem::zeroed() };
    let state = unsafe { libc::adjtimex(&mut tx) };
    if state == libc::TIME_ERROR {
        tracing::debug!("GNSS: часы не сверены — время модему не передаётся");
        return;
    }
    let m = Message::new(INJECT_TIME)
        .tlv(0x01, (now_ms() as u64).to_le_bytes().to_vec())
        .u32(0x02, 500);
    if let Err(e) = c.call(m, T) {
        tracing::debug!("GNSS: время: {e:#}");
    }
}

/// Сколько секунд ещё действуют данные XTRA в модеме (`None` — их нет).
fn xtra_left(l: &mut Link) -> Option<i64> {
    l.c.call(Message::new(XTRA_VALIDITY), T).ok()?;
    let ind = l.wait(XTRA_VALIDITY, T)?;
    // Начало (UTC, с) u64 и длительность (ч) u16
    let b = ind.get(0x10).filter(|b| b.len() >= 10)?;
    let start = u64::from_le_bytes(b[0..8].try_into().unwrap()) as i64;
    let hours = u16::from_le_bytes([b[8], b[9]]) as i64;
    (hours > 0).then(|| start + hours * 3600 - now_ms() / 1000)
}

/// Данные XTRA по запросу модема (индикация 0x29: допустимые размеры и серверы): скачать (раз в сутки)
/// и передать частями.
fn inject_xtra(l: &mut Link, req: &Message) -> Result<()> {
    let (max_file, max_part) = req
        .get(0x01)
        .filter(|b| b.len() >= 8)
        .map(|b| {
            (
                u32::from_le_bytes(b[0..4].try_into().unwrap()),
                u32::from_le_bytes(b[4..8].try_into().unwrap()),
            )
        })
        .unwrap_or((128 * 1024, 1024));
    let mut servers: Vec<String> = req
        .get(0x10)
        .and_then(|b| {
            let mut rd = crate::qmi::Reader::new(b);
            let n = rd.u8()?;
            (0..n).map(|_| rd.str8()).collect::<Option<Vec<_>>>()
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.starts_with("http"))
        .collect();
    if servers.is_empty() {
        servers = XTRA_SERVERS.iter().map(|s| s.to_string()).collect();
    }
    let data = xtra_data(&servers)?;
    if data.len() > max_file as usize {
        bail!("XTRA {} байт больше допустимого {max_file}", data.len());
    }
    let part = (max_part as usize).clamp(256, 1024);
    let parts = data.len().div_ceil(part);
    let mut id = INJECT_XTRA;
    for (k, chunk) in data.chunks(part).enumerate() {
        let mut v = (chunk.len() as u16).to_le_bytes().to_vec();
        v.extend_from_slice(chunk);
        let m = Message::new(id)
            .u32(0x01, data.len() as u32)
            .u16(0x02, parts as u16)
            .u16(0x03, k as u16 + 1)
            .tlv(0x04, v)
            .u32(0x10, 0);
        match l.c.call(m.clone(), T) {
            Err(e)
                if k == 0
                    && e.downcast_ref::<QmiError>() == Some(&QmiError(QmiError::NOT_SUPPORTED)) =>
            {
                id = INJECT_ORBITS;
                l.c.call(Message { id, ..m }, T)
            }
            r => r,
        }
        .with_context(|| format!("часть {} из {parts}", k + 1))?;
        let ind = l
            .wait(id, T)
            .ok_or_else(|| anyhow!("нет подтверждения части {}", k + 1))?;
        if let Some(st) = u32_at(&ind, 0x01).filter(|s| *s != 0) {
            bail!("модем отверг часть {} из {parts} (статус {st})", k + 1);
        }
    }
    tracing::info!("GNSS: XTRA передан ({} байт, {parts} частей)", data.len());
    Ok(())
}

/// Файл XTRA: свежий из кэша или скачанный; при неудаче — устаревший, пока не старше недели.
fn xtra_data(servers: &[String]) -> Result<Vec<u8>> {
    let age = std::fs::metadata(XTRA_FILE)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok());
    if age.is_some_and(|a| a < XTRA_REFRESH) {
        return Ok(std::fs::read(XTRA_FILE)?);
    }
    let _ = std::fs::create_dir_all(std::path::Path::new(XTRA_FILE).parent().unwrap());
    let tmp = format!("{XTRA_FILE}.part");
    for url in servers {
        let ok = std::process::Command::new("curl")
            .args(["-sfL", "--max-time", "30", "-o", &tmp, url])
            .status()
            .is_ok_and(|s| s.success());
        if ok && std::fs::metadata(&tmp).is_ok_and(|m| m.len() > 1000) {
            std::fs::rename(&tmp, XTRA_FILE)?;
            tracing::info!("GNSS: XTRA скачан с {url}");
            return Ok(std::fs::read(XTRA_FILE)?);
        }
    }
    let _ = std::fs::remove_file(&tmp);
    if age.is_some_and(|a| a < XTRA_MAX_AGE) {
        return Ok(std::fs::read(XTRA_FILE)?);
    }
    bail!("XTRA не скачан (нет сети?)")
}

/// Связь со службой LOC на сеанс: индикации, пропущенные при ожидании ответа, откладываются.
struct Link<'a> {
    c: &'a Client,
    rx: Receiver<Indication>,
    stash: VecDeque<Message>,
}

impl Link<'_> {
    fn next(&mut self, timeout: Duration) -> Result<Message, mpsc::RecvTimeoutError> {
        match self.stash.pop_front() {
            Some(m) => Ok(m),
            None => self.rx.recv_timeout(timeout).map(|(_, m)| m),
        }
    }

    /// Дождаться индикации `id`; остальные — в очередь.
    fn wait(&mut self, id: u16, timeout: Duration) -> Option<Message> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            let (_, m) = self.rx.recv_timeout(left).ok()?;
            if m.id == id {
                return Some(m);
            }
            let gone = m.id == SERVICE_GONE;
            self.stash.push_back(m);
            if gone {
                return None;
            }
        }
    }
}

fn u32_at(m: &Message, t: u8) -> Option<u32> {
    m.get(t)
        .filter(|b| b.len() >= 4)
        .map(|b| u32::from_le_bytes(b[..4].try_into().unwrap()))
}
fn f32_of(b: &[u8]) -> f32 {
    f32::from_le_bytes(b[..4].try_into().unwrap())
}
fn f32_at(m: &Message, t: u8) -> Option<f32> {
    m.get(t).filter(|b| b.len() >= 4).map(f32_of)
}
fn f64_at(m: &Message, t: u8) -> Option<f64> {
    m.get(t)
        .filter(|b| b.len() >= 8)
        .map(|b| f64::from_le_bytes(b[..8].try_into().unwrap()))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn system_name(s: u32) -> &'static str {
    match s {
        1 => "GPS",
        2 => "Galileo",
        3 => "SBAS",
        4 | 6 => "BeiDou",
        5 => n_!("ГЛОНАСС"),
        7 => "QZSS",
        8 => "NavIC",
        _ => "?",
    }
}

fn group_id(name: &str) -> Option<u32> {
    std::fs::read_to_string("/etc/group")
        .ok()?
        .lines()
        .find(|l| l.split(':').next() == Some(name))
        .and_then(|l| l.split(':').nth(2)?.parse().ok())
}

/// Дата UTC (год, месяц, день) по дням от эпохи.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

fn sentence(body: &str) -> String {
    let cs = body.bytes().fold(0u8, |a, b| a ^ b);
    format!("${body}*{cs:02X}\r\n")
}

fn coord(v: f64, deg_digits: usize, pos: char, neg: char) -> String {
    let a = v.abs();
    let mut d = a.trunc() as u32;
    let mut min = (a - a.trunc()) * 60.0;
    if min >= 59.999995 {
        d += 1;
        min = 0.0;
    }
    format!(
        "{d:0deg_digits$}{min:08.5},{}",
        if v < 0.0 { neg } else { pos }
    )
}

/// Решение в NMEA: GGA, RMC и GSA (без решения — пустые поля, как у приёмников без фиксации).
pub fn nmea(f: &Fix) -> String {
    let t = if f.time_ms > 0 { f.time_ms } else { now_ms() };
    let secs = t.div_euclid(1000);
    let (y, mo, d) = civil(secs.div_euclid(86400));
    let sod = secs.rem_euclid(86400);
    let hms = format!(
        "{:02}{:02}{:02}.{:02}",
        sod / 3600,
        sod / 60 % 60,
        sod % 60,
        t.rem_euclid(1000) / 10
    );
    let date = format!("{d:02}{mo:02}{:02}", y.rem_euclid(100));
    let opt = |v: Option<f32>, p: usize| v.map(|v| format!("{v:.p$}")).unwrap_or_default();
    let mut out = String::new();
    if f.valid {
        let lat = coord(f.latitude, 2, 'N', 'S');
        let lon = coord(f.longitude, 3, 'E', 'W');
        let sep = match (f.altitude_ellipsoid, f.altitude) {
            (Some(e), Some(m)) => format!("{:.1}", e - m),
            _ => String::new(),
        };
        let alt = f.altitude.or(f.altitude_ellipsoid);
        out += &sentence(&format!(
            "GNGGA,{hms},{lat},{lon},1,{:02},{},{},M,{sep},M,,",
            f.satellites_used,
            opt(f.hdop, 1),
            opt(alt, 1)
        ));
        let knots = f.speed.map(|v| v * 1.943_844);
        out += &sentence(&format!(
            "GNRMC,{hms},A,{lat},{lon},{},{},{date},,,A",
            opt(knots, 2),
            opt(f.heading, 1)
        ));
        out += &sentence(&format!(
            "GNGSA,A,{},,,,,,,,,,,,,{},{},{}",
            if alt.is_some() { 3 } else { 2 },
            opt(f.pdop, 1),
            opt(f.hdop, 1),
            opt(f.vdop, 1)
        ));
    } else {
        out += &sentence(&format!(
            "GNGGA,{hms},,,,,0,{:02},,,M,,M,,",
            f.satellites_used
        ));
        out += &sentence(&format!("GNRMC,{hms},V,,,,,,,{date},,,N"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20727), (2026, 10, 1));
        assert_eq!(civil(11016), (2000, 2, 29));
    }

    #[test]
    fn nmea_fix() {
        let f = Fix {
            valid: true,
            latitude: 43.238949,
            longitude: -76.889709,
            altitude: Some(850.0),
            altitude_ellipsoid: Some(810.0),
            hdop: Some(0.9),
            satellites_used: 9,
            time_ms: 20727 * 86400 * 1000 + 12 * 3600 * 1000 + 34 * 60 * 1000 + 56_780,
            ..Default::default()
        };
        let s = nmea(&f);
        let gga = s.lines().next().unwrap();
        assert!(
            gga.starts_with(
                "$GNGGA,123456.78,4314.33694,N,07653.38254,W,1,09,0.9,850.0,M,-40.0,M,,*"
            ),
            "{gga}"
        );
        let rmc = s.lines().nth(1).unwrap();
        assert!(
            rmc.contains(",A,4314.33694,N,07653.38254,W,") && rmc.contains(",011026,"),
            "{rmc}"
        );
        // Контрольная сумма: XOR между $ и *
        let body = &gga[1..gga.find('*').unwrap()];
        let cs = body.bytes().fold(0u8, |a, b| a ^ b);
        assert!(gga.ends_with(&format!("*{cs:02X}")));
    }
}
