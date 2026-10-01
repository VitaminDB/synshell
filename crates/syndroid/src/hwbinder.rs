//! Минимальный HIDL поверх hwbinder — ровно столько, сколько нужно, чтобы с хоста отдать Android HAL-сервис
//! (датчики, `sensors.rs`). В rsbinder HIDL нет; протокол здесь — по ioctl ядра напрямую.
//!
//! Посылка HIDL (HwParcel): токен интерфейса — C-строка; числа — как есть с выравниванием на 4; `hidl_string` и
//! `hidl_vec` — scatter-gather буферы (`BINDER_TYPE_PTR`): заголовок (указатель, размер, владение — 16 байт)
//! отдельным буфером, данные — дочерним буфером с родителем и смещением поля-указателя в нём; ядро копирует
//! буферы получателю и подставляет указатели. Отправка — `BC_TRANSACTION_SG`/`BC_REPLY_SG`.
//! Каждый HIDL-объект отвечает и на методы IBase (`interfaceChain`, `ping`, `getDebugInfo`…) — их вызывает
//! hwservicemanager при регистрации и клиент при приведении типа.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::Arc;

use anyhow::{bail, Context, Result};

// --- ioctl и команды драйвера (64-битный ABI) ----------------------------------------------------------

const BINDER_WRITE_READ: libc::c_ulong = 0xC030_6201;
const BINDER_SET_MAX_THREADS: libc::c_ulong = 0x4004_6205;
const BINDER_VERSION: libc::c_ulong = 0xC004_6209;

const BC_TRANSACTION_SG: u32 = 0x4048_6311;
const BC_REPLY_SG: u32 = 0x4048_6312;
const BC_FREE_BUFFER: u32 = 0x4008_6303;
const BC_INCREFS_DONE: u32 = 0x4010_6308;
const BC_ACQUIRE_DONE: u32 = 0x4010_6309;
const BC_REGISTER_LOOPER: u32 = 0x630b;
const BC_ENTER_LOOPER: u32 = 0x630c;

const BR_ERROR: u32 = 0x8004_7200;
const BR_TRANSACTION: u32 = 0x8040_7202;
const BR_TRANSACTION_SEC_CTX: u32 = 0x8048_7202;
const BR_REPLY: u32 = 0x8040_7203;
const BR_DEAD_REPLY: u32 = 0x7205;
const BR_INCREFS: u32 = 0x8010_7207;
const BR_ACQUIRE: u32 = 0x8010_7208;
const BR_SPAWN_LOOPER: u32 = 0x720d;
const BR_FAILED_REPLY: u32 = 0x7211;

const TF_ONE_WAY: u32 = 0x01;
const TF_ACCEPT_FDS: u32 = 0x10;

const BINDER_TYPE_BINDER: u32 = 0x7362_2a85;
const BINDER_TYPE_PTR: u32 = 0x7074_2a85;
const BINDER_BUFFER_FLAG_HAS_PARENT: u32 = 0x01;
const FLAT_BINDER_FLAG_ACCEPTS_FDS: u32 = 0x100;

/// Коды методов IBase (B_PACK_CHARS(0x0f, …)).
const IBASE_INTERFACE_CHAIN: u32 = 0x0f43_484e;
const IBASE_INTERFACE_DESCRIPTOR: u32 = 0x0f44_5343;
const IBASE_PING: u32 = 0x0f50_4e47;
const IBASE_HASH_CHAIN: u32 = 0x0f48_5348;
const IBASE_DEBUG_INFO: u32 = 0x0f49_4e46;
const IBASE_DEBUG: u32 = 0x0f44_4247;
const IBASE_SYSPROPS: u32 = 0x0f53_5052;

const IBASE: &str = "android.hidl.base@1.0::IBase";
const SERVICE_MANAGER: &str = "android.hidl.manager@1.0::IServiceManager";

/// Наш единственный локальный объект: ptr/cookie для ядра.
const OBJ_PTR: u64 = 0x5e45_5e00;
const OBJ_COOKIE: u64 = 0x5e45_5e01;

#[repr(C)]
#[derive(Default)]
struct WriteRead {
    write_size: u64,
    write_consumed: u64,
    write_buffer: u64,
    read_size: u64,
    read_consumed: u64,
    read_buffer: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct TxData {
    target: u64,
    cookie: u64,
    code: u32,
    flags: u32,
    sender_pid: i32,
    sender_euid: u32,
    data_size: u64,
    offsets_size: u64,
    buffer: u64,
    offsets: u64,
}

#[repr(C)]
struct TxDataSg {
    tr: TxData,
    buffers_size: u64,
}

fn bytes_of<T>(v: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts((v as *const T).cast::<u8>(), std::mem::size_of::<T>()) }
}

// --- драйвер -----------------------------------------------------------------------------------------

pub struct Driver {
    file: File,
}

impl Driver {
    pub fn open(path: &str) -> Result<Arc<Self>> {
        let file = OpenOptions::new().read(true).write(true).custom_flags(libc::O_CLOEXEC).open(path).with_context(|| path.to_string())?;
        let fd = file.as_raw_fd();
        let mut ver: i32 = 0;
        if unsafe { libc::ioctl(fd, BINDER_VERSION as _, &mut ver) } != 0 || ver != 8 {
            bail!("{path}: протокол binder {ver}, нужен 8");
        }
        let max: u32 = 4;
        unsafe { libc::ioctl(fd, BINDER_SET_MAX_THREADS as _, &max) };
        let map = unsafe {
            libc::mmap(std::ptr::null_mut(), 1 << 20, libc::PROT_READ, libc::MAP_PRIVATE | libc::MAP_NORESERVE, fd, 0)
        };
        if map == libc::MAP_FAILED {
            bail!("{path}: mmap: {}", io::Error::last_os_error());
        }
        Ok(Arc::new(Self { file }))
    }

    /// Записать команды и прочитать ответы ядра (read пустой — только запись).
    fn talk(&self, write: &[u8], read: &mut [u8]) -> io::Result<usize> {
        let mut bwr = WriteRead {
            write_size: write.len() as u64,
            write_buffer: write.as_ptr() as u64,
            read_size: read.len() as u64,
            read_buffer: read.as_mut_ptr() as u64,
            ..Default::default()
        };
        loop {
            let r = unsafe { libc::ioctl(self.file.as_raw_fd(), BINDER_WRITE_READ as _, &mut bwr) };
            if r == 0 {
                return Ok(bwr.read_consumed as usize);
            }
            let e = io::Error::last_os_error();
            if e.raw_os_error() != Some(libc::EINTR) {
                return Err(e);
            }
        }
    }

    fn cmd(&self, cmd: u32, payload: &[u8]) -> io::Result<()> {
        let mut w = cmd.to_le_bytes().to_vec();
        w.extend_from_slice(payload);
        self.talk(&w, &mut [])?;
        Ok(())
    }
}

// --- посылка -----------------------------------------------------------------------------------------

/// Исходящая посылка HIDL. Буферы scatter-gather живут в ней до отправки (ядро читает их по указателям).
#[derive(Default)]
pub struct HwParcel {
    data: Vec<u8>,
    offsets: Vec<u64>,
    bufs: Vec<Box<[u8]>>,
    buffers_size: u64,
}

impl HwParcel {
    pub fn new() -> Self {
        Self::default()
    }
    fn pad(&mut self) {
        while !self.data.len().is_multiple_of(4) {
            self.data.push(0);
        }
    }
    pub fn i32(&mut self, v: i32) {
        self.data.extend_from_slice(&v.to_le_bytes());
    }
    pub fn cstr(&mut self, s: &str) {
        self.data.extend_from_slice(s.as_bytes());
        self.data.push(0);
        self.pad();
    }
    /// Статус ответа HIDL: 0 — успех.
    pub fn ok(&mut self) {
        self.i32(0);
    }

    /// Буфер `BINDER_TYPE_PTR`; `parent` — (индекс объекта-родителя, смещение поля-указателя в нём).
    fn buffer(&mut self, bytes: Vec<u8>, parent: Option<(u64, u64)>) -> u64 {
        let b = bytes.into_boxed_slice();
        let (ptr, len) = if b.is_empty() { (0u64, 0u64) } else { (b.as_ptr() as u64, b.len() as u64) };
        self.pad();
        self.offsets.push(self.data.len() as u64);
        let (pidx, poff) = parent.unwrap_or((0, 0));
        for v in [BINDER_TYPE_PTR, if parent.is_some() { BINDER_BUFFER_FLAG_HAS_PARENT } else { 0 }] {
            self.data.extend_from_slice(&v.to_le_bytes());
        }
        for v in [ptr, len, pidx, poff] {
            self.data.extend_from_slice(&v.to_le_bytes());
        }
        self.buffers_size += (len + 7) & !7;
        self.bufs.push(b);
        (self.offsets.len() - 1) as u64
    }

    /// Наш объект (сервис) — для `IServiceManager::add`.
    fn local_binder(&mut self) {
        self.pad();
        self.offsets.push(self.data.len() as u64);
        self.data.extend_from_slice(&BINDER_TYPE_BINDER.to_le_bytes());
        self.data.extend_from_slice(&(0x7f | FLAT_BINDER_FLAG_ACCEPTS_FDS).to_le_bytes());
        self.data.extend_from_slice(&OBJ_PTR.to_le_bytes());
        self.data.extend_from_slice(&OBJ_COOKIE.to_le_bytes());
    }

    /// `hidl_string` верхнего уровня (аргумент или возвращаемое значение).
    pub fn string(&mut self, s: &str) {
        let h = self.buffer(hidl_header(s.len()).to_vec(), None);
        self.embedded_string(s, h, 0);
    }

    /// Символы `hidl_string`, заголовок которой лежит в буфере `parent` по смещению `offset`.
    pub fn embedded_string(&mut self, s: &str, parent: u64, offset: u64) {
        let mut chars = s.as_bytes().to_vec();
        chars.push(0);
        self.buffer(chars, Some((parent, offset)));
    }

    /// `hidl_vec` верхнего уровня: элементы — готовые байты; возвращает индекс буфера данных (родитель вложенных
    /// строк элементов).
    pub fn vec(&mut self, elems: Vec<u8>, count: usize) -> u64 {
        let h = self.buffer(hidl_header(count).to_vec(), None);
        self.buffer(elems, Some((h, 0)))
    }

    /// Структура верхнего уровня (без указателей внутри).
    pub fn struct_bytes(&mut self, bytes: Vec<u8>) {
        self.buffer(bytes, None);
    }
}

/// Заголовок `hidl_string`/`hidl_vec`: указатель (ставит ядро), размер, «владеет буфером».
pub fn hidl_header(size: usize) -> [u8; 16] {
    let mut h = [0u8; 16];
    h[8..12].copy_from_slice(&(size as u32).to_le_bytes());
    h
}

/// Входящая посылка (буфер ядра).
pub struct HwReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> HwReader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self.data.get(self.pos..self.pos + n).context("посылка короче ожидаемого")?;
        self.pos += n;
        Ok(s)
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into()?))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into()?))
    }
    /// bool HIDL — младший байт 32-битного слова.
    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.i32()? & 0xff != 0)
    }
    pub fn cstr(&mut self) -> Result<String> {
        let rest = &self.data[self.pos..];
        let end = rest.iter().position(|&b| b == 0).context("нет конца строки")?;
        let s = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.pos += (end + 1 + 3) & !3;
        Ok(s)
    }
}

// --- сервис ------------------------------------------------------------------------------------------

/// HIDL-сервис: цепочка интерфейсов (от своего к IBase не включая) и обработка своих методов.
pub trait Service: Send + Sync + 'static {
    /// Дескрипторы интерфейса, самый производный первым («android.hardware.sensors@1.0::ISensors»).
    fn chain(&self) -> &'static [&'static str];
    /// Метод интерфейса: аргументы после токена; ответ — статус и значения. Err — неизвестный метод.
    fn call(&self, code: u32, args: &mut HwReader, reply: &mut HwParcel) -> Result<()>;
}

fn dispatch(svc: &dyn Service, code: u32, data: &[u8]) -> Option<HwParcel> {
    let mut r = HwReader { data, pos: 0 };
    let _token = r.cstr().unwrap_or_default();
    let mut reply = HwParcel::new();
    let chain: Vec<&str> = svc.chain().iter().copied().chain([IBASE]).collect();
    match code {
        IBASE_INTERFACE_CHAIN => {
            reply.ok();
            let mut elems = Vec::new();
            for s in &chain {
                elems.extend_from_slice(&hidl_header(s.len()));
            }
            let d = reply.vec(elems, chain.len());
            for (i, s) in chain.iter().enumerate() {
                reply.embedded_string(s, d, (i * 16) as u64);
            }
        }
        IBASE_INTERFACE_DESCRIPTOR => {
            reply.ok();
            reply.string(chain[0]);
        }
        IBASE_HASH_CHAIN => {
            reply.ok();
            reply.vec(vec![0u8; 32 * chain.len()], chain.len());
        }
        IBASE_DEBUG_INFO => {
            reply.ok();
            // DebugInfo { int32 pid; uint64 ptr; Architecture arch } — 64 бит
            let mut b = vec![0u8; 24];
            b[0..4].copy_from_slice(&(std::process::id() as i32).to_le_bytes());
            b[16..20].copy_from_slice(&1i32.to_le_bytes());
            reply.struct_bytes(b);
        }
        IBASE_PING | IBASE_DEBUG => reply.ok(),
        IBASE_SYSPROPS => return None,
        _ => {
            if let Err(e) = svc.call(code, &mut r, &mut reply) {
                tracing::warn!("hwbinder: метод {code:#x}: {e:#}");
                let mut err = HwParcel::new();
                err.i32(-1); // EX_SECURITY-подобная ошибка: клиент получит исключение
                return Some(err);
            }
        }
    }
    Some(reply)
}

/// Разобрать ответ ядра; входящие вызовы обслужить; вернуть данные BR_REPLY (если ждём ответа).
fn process(drv: &Arc<Driver>, svc: &Arc<dyn Service>, read: &[u8], want_reply: bool) -> Result<Option<Vec<u8>>> {
    let mut pos = 0;
    let mut out = Vec::new();
    while pos + 4 <= read.len() {
        let cmd = u32::from_le_bytes(read[pos..pos + 4].try_into()?);
        pos += 4;
        let size = ((cmd >> 16) & 0x3fff) as usize;
        let payload = &read[pos..(pos + size).min(read.len())];
        pos += size;
        match cmd {
            BR_INCREFS | BR_ACQUIRE => {
                out.extend_from_slice(&(if cmd == BR_INCREFS { BC_INCREFS_DONE } else { BC_ACQUIRE_DONE }).to_le_bytes());
                out.extend_from_slice(payload);
            }
            BR_SPAWN_LOOPER => {
                let (d, s) = (drv.clone(), svc.clone());
                std::thread::spawn(move || looper(d, s, false));
            }
            BR_TRANSACTION | BR_TRANSACTION_SEC_CTX => {
                let tr: TxData = unsafe { std::ptr::read_unaligned(payload.as_ptr().cast()) };
                let data = unsafe { std::slice::from_raw_parts(tr.buffer as *const u8, tr.data_size as usize) };
                let reply = dispatch(svc.as_ref(), tr.code, data);
                // Ответ — сразу (пока нас ждут), буфер запроса — освободить
                let mut w = BC_FREE_BUFFER.to_le_bytes().to_vec();
                w.extend_from_slice(&tr.buffer.to_le_bytes());
                if tr.flags & TF_ONE_WAY == 0 {
                    let reply = reply.unwrap_or_else(|| {
                        let mut p = HwParcel::new();
                        p.ok();
                        p
                    });
                    let sg = TxDataSg {
                        tr: TxData {
                            data_size: reply.data.len() as u64,
                            offsets_size: (reply.offsets.len() * 8) as u64,
                            buffer: reply.data.as_ptr() as u64,
                            offsets: reply.offsets.as_ptr() as u64,
                            ..Default::default()
                        },
                        buffers_size: reply.buffers_size,
                    };
                    w.extend_from_slice(&BC_REPLY_SG.to_le_bytes());
                    w.extend_from_slice(bytes_of(&sg));
                    drv.talk(&w, &mut [])?;
                    drop(reply);
                } else {
                    drv.talk(&w, &mut [])?;
                }
            }
            BR_REPLY => {
                let tr: TxData = unsafe { std::ptr::read_unaligned(payload.as_ptr().cast()) };
                let data = unsafe { std::slice::from_raw_parts(tr.buffer as *const u8, tr.data_size as usize) }.to_vec();
                drv.cmd(BC_FREE_BUFFER, &tr.buffer.to_le_bytes())?;
                if want_reply {
                    if !out.is_empty() {
                        drv.talk(&out, &mut [])?;
                    }
                    return Ok(Some(data));
                }
            }
            BR_DEAD_REPLY | BR_FAILED_REPLY => {
                if want_reply {
                    bail!("транзакция не дошла ({})", if cmd == BR_DEAD_REPLY { "получатель мёртв" } else { "отказ" });
                }
            }
            BR_ERROR => bail!("binder: ошибка {}", i32::from_le_bytes(payload.try_into().unwrap_or([0; 4]))),
            _ => {}
        }
    }
    if !out.is_empty() {
        drv.talk(&out, &mut [])?;
    }
    Ok(None)
}

/// Поток-обработчик входящих вызовов.
pub fn looper(drv: Arc<Driver>, svc: Arc<dyn Service>, main: bool) {
    if drv.cmd(if main { BC_ENTER_LOOPER } else { BC_REGISTER_LOOPER }, &[]).is_err() {
        return;
    }
    let mut buf = vec![0u8; 4096];
    loop {
        match drv.talk(&[], &mut buf) {
            Ok(n) => {
                if let Err(e) = process(&drv, &svc, &buf[..n], false) {
                    tracing::warn!("hwbinder: {e:#}");
                }
            }
            Err(e) => {
                tracing::error!("hwbinder: {e}");
                return;
            }
        }
    }
}

/// Синхронный вызов handle 0 (hwservicemanager); входящие вызовы во время ожидания обслуживаются.
fn call_manager(drv: &Arc<Driver>, svc: &Arc<dyn Service>, code: u32, p: &HwParcel) -> Result<Vec<u8>> {
    let sg = TxDataSg {
        tr: TxData {
            target: 0,
            code,
            flags: TF_ACCEPT_FDS,
            data_size: p.data.len() as u64,
            offsets_size: (p.offsets.len() * 8) as u64,
            buffer: p.data.as_ptr() as u64,
            offsets: p.offsets.as_ptr() as u64,
            ..Default::default()
        },
        buffers_size: p.buffers_size,
    };
    let mut w = BC_TRANSACTION_SG.to_le_bytes().to_vec();
    w.extend_from_slice(bytes_of(&sg));
    let mut buf = vec![0u8; 4096];
    let mut first = true;
    loop {
        let n = drv.talk(if first { &w } else { &[] }, &mut buf)?;
        first = false;
        if let Some(r) = process(drv, svc, &buf[..n], true)? {
            return Ok(r);
        }
    }
}

/// Зарегистрировать сервис в hwservicemanager (`IServiceManager::add(name, service)`), затем обслуживать вызовы
/// (не возвращается при успехе). Пока hwservicemanager не запущен — повторять.
pub fn serve(binder_dev: &str, instance: &str, svc: Arc<dyn Service>) -> Result<()> {
    let drv = Driver::open(binder_dev)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        let mut p = HwParcel::new();
        p.cstr(SERVICE_MANAGER);
        p.string(instance);
        p.local_binder();
        match call_manager(&drv, &svc, 2, &p) {
            Ok(reply) => {
                let mut r = HwReader { data: &reply, pos: 0 };
                let status = r.i32()?;
                let ok = r.bool().unwrap_or(false);
                if status != 0 || !ok {
                    bail!("hwservicemanager отказал в регистрации {} (статус {status})", svc.chain()[0]);
                }
                break;
            }
            Err(e) if std::time::Instant::now() < deadline => {
                tracing::debug!("hwservicemanager ещё нет: {e:#}");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            Err(e) => return Err(e.context("hwservicemanager")),
        }
    }
    tracing::info!("hwbinder: {}/{instance} зарегистрирован", svc.chain()[0]);
    looper(drv, svc, true);
    Ok(())
}

/// Очередь событий с ожиданием (для блокирующего `poll`).
pub struct EventQueue<T> {
    q: std::sync::Mutex<VecDeque<T>>,
    cv: std::sync::Condvar,
}

impl<T> Default for EventQueue<T> {
    fn default() -> Self {
        Self { q: Default::default(), cv: Default::default() }
    }
}

impl<T> EventQueue<T> {
    pub fn push(&self, v: T) {
        let mut q = self.q.lock().unwrap();
        // Не копить бесконечно, если Android не забирает
        if q.len() > 4096 {
            q.pop_front();
        }
        q.push_back(v);
        self.cv.notify_one();
    }
    /// Забрать до `max` событий, ожидая первое не дольше `timeout` (пусто — не дождались).
    pub fn take(&self, max: usize, timeout: std::time::Duration) -> Vec<T> {
        let q = self.q.lock().unwrap();
        let (mut q, _) = self.cv.wait_timeout_while(q, timeout, |q| q.is_empty()).unwrap();
        let n = q.len().min(max);
        q.drain(..n).collect()
    }
}
