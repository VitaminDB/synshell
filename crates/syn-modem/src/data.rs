//! Мобильная передача данных: сеанс WDS модема и интерфейс `rmnet_data0`.
//!
//! Путь пакетов downstream-ядер Qualcomm (IPA): `rmnet_ipa0` — канал IPA к модему (драйвер rmnet_ipa из
//! ipam; регистрирует его `ipanetm`, а API IPA во фреймворке появляется только с `ipa_clientsm`), поверх —
//! `rmnet_data0` драйвера rmnet_core (QMAP, мукс 1). Каналы IPA настраиваются ioctl'ами `rmnet_ipa0`, как это
//! делает netmgrd Android. Модему: DPM — пара каналов IPA, WDA — формат QMAP, WDS — привязка мукса и сеанс.
//!
//! Mainline msm8916/msm8937 (SM-T295): BAM-DMUX — драйвер `qcom_bam_dmux` создаёт `wwan0…7`, когда модем
//! поднимет линию и откроет каналы; модем msm8937 делает это только после DPM open port с портом BAM_DMUX
//! (ep_type 5) — так netmgrd стока (`netmgr_config.xml`, раздел `feero`: DPM, WDA QMAP, `rmnet0` = канал 0).
//! Поверх `wwan0` — тот же `rmnet_data0` (rmnet, QMAP, мукс 1).
//! Сеанс WDS живёт, пока открыт сокет клиента: клиент держит [`Session`].

use std::net::Ipv4Addr;
use std::os::fd::AsRawFd;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::qmi::{svc, Client, Indication, Message};

/// Интерфейс сеанса у IPA: rmnet поверх rmnet_ipa0 (QMAP, мукс 1).
const IPA_IFACE: &str = "rmnet_data0";

/// Интерфейс, по которому идут пакеты сеанса: `rmnet_data0` (IPA) или `wwan0` (BAM-DMUX, raw IP без QMAP).
pub fn iface() -> &'static str {
    match Transport::detect() {
        Transport::Ipa => IPA_IFACE,
        Transport::Bam => BAM_DEV,
    }
}
const IPA_DEV: &str = "rmnet_ipa0";
/// Канал 0 BAM-DMUX (mainline `qcom_bam_dmux`).
const BAM_DEV: &str = "wwan0";
/// Тип конечной точки BAM_DMUX и номер интерфейса (канал 0) — как у netmgrd стока msm8937.
const BAM_EP_TYPE: u32 = 5;
const BAM_EP_IFACE: u32 = 0;
/// Управляющий канал данных (SMD) — `single_qmux_ch_name` netmgrd стока.
const BAM_CTL_PORT: &str = "DATA5_CNTL";
const MUX_ID: u8 = 1;
/// Тип конечной точки «встроенная» (EMBEDDED) и её номер у rmnet_ipa (RMNET_IOCTL_GET_EPID — всегда 1).
const EP_TYPE: u32 = 4;
const EP_IFACE: u32 = 1;
/// Метрика маршрута: Wi-Fi у NetworkManager — 600, мобильная сеть — запасная.
const ROUTE_METRIC: &str = "750";
const T: Duration = Duration::from_secs(10);

const RMNET_IOCTL_EXTENDED: libc::c_ulong = 0x89FD;
const EXT_ADD_MUX_CHANNEL: u32 = 0x05;
const EXT_SET_EGRESS_DATA_FORMAT: u32 = 0x06;
const EXT_SET_INGRESS_DATA_FORMAT: u32 = 0x07;
const EXT_GET_EP_PAIR: u32 = 0x10;
/// Вход: MAP, деагрегация, демукс; выход: MAP (без агрегации и контрольных сумм).
const INGRESS_FORMAT: u32 = (1 << 1) | (1 << 2) | (1 << 3);
const EGRESS_FORMAT: u32 = 1 << 1;

/// Настройки сеанса от модема.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub address: Option<Ipv4Addr>,
    pub prefix: u8,
    pub gateway: Option<Ipv4Addr>,
    pub dns: Vec<Ipv4Addr>,
    pub mtu: Option<u32>,
}

/// Открытый сеанс передачи данных.
pub struct Session {
    pub wds: Arc<Client>,
    pub handle: u32,
    pub settings: Settings,
}

/// `struct rmnet_ioctl_extended_s` (msm_rmnet.h): номер под-ioctl и объединение данных.
#[repr(C)]
struct ExtIoctl {
    cmd: u32,
    data: [u8; 32],
}

fn ext_ioctl(cmd: u32, payload: &[u8]) -> std::io::Result<[u8; 32]> {
    let s = std::net::UdpSocket::bind("127.0.0.1:0")?;
    let mut ext = ExtIoctl { cmd, data: [0; 32] };
    ext.data[..payload.len()].copy_from_slice(payload);
    // struct ifreq: имя интерфейса и указатель ifru_data
    let mut ifr = [0u8; 40];
    ifr[..IPA_DEV.len()].copy_from_slice(IPA_DEV.as_bytes());
    let ptr = (&mut ext as *mut ExtIoctl) as u64;
    ifr[16..24].copy_from_slice(&ptr.to_ne_bytes());
    let r = unsafe { libc::ioctl(s.as_raw_fd(), RMNET_IOCTL_EXTENDED as _, ifr.as_mut_ptr()) };
    if r < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(ext.data)
}

fn sh(args: &[&str]) -> Result<()> {
    let out = Command::new(args[0]).args(&args[1..]).output().with_context(|| args.join(" "))?;
    if !out.status.success() {
        bail!("{}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

fn exists(iface: &str) -> bool {
    std::path::Path::new("/sys/class/net").join(iface).exists()
}

/// Путь пакетов к модему.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// downstream: rmnet_ipa (Xiaomi, SM8450)
    Ipa,
    /// mainline: qcom_bam_dmux (SM-T295)
    Bam,
}

impl Transport {
    pub fn detect() -> Transport {
        // привязанное устройство драйвера bam-dmux — запись «…:bam-dmux» в каталоге драйвера
        let bam = std::fs::read_dir("/sys/bus/platform/drivers/bam-dmux")
            .map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().contains("bam-dmux")))
            .unwrap_or(false);
        if bam {
            Transport::Bam
        } else {
            Transport::Ipa
        }
    }
    fn endpoint(self) -> Vec<u8> {
        let (t, i) = match self {
            Transport::Ipa => (EP_TYPE, EP_IFACE),
            Transport::Bam => (BAM_EP_TYPE, BAM_EP_IFACE),
        };
        let mut ep = t.to_le_bytes().to_vec();
        ep.extend_from_slice(&i.to_le_bytes());
        ep
    }
}

/// Ядро и модем к передаче данных (один раз за запуск модема; повтор безвреден).
pub fn prepare(ind: &std::sync::mpsc::Sender<Indication>) -> Result<Transport> {
    let t = Transport::detect();
    match t {
        Transport::Ipa => {
            let pair = setup_kernel()?;
            setup_modem(ind, pair)?;
        }
        Transport::Bam => setup_bam(ind)?,
    }
    Ok(t)
}

/// BAM-DMUX: DPM open port (порт BAM_DMUX) → модем поднимает линию; WDA — QMAP на этом порту.
fn setup_bam(ind: &std::sync::mpsc::Sender<Indication>) -> Result<()> {
    let _ = Command::new("modprobe").arg("rmnet").status();
    let dpm = Client::connect(svc::DPM, T, ind.clone())?;
    // control_port_list: число, затем имя (длина + байты), ep_type, iface_id — управляющий канал SMD стока
    let mut ctl = vec![1u8, BAM_CTL_PORT.len() as u8];
    ctl.extend_from_slice(BAM_CTL_PORT.as_bytes());
    for v in [BAM_EP_TYPE, BAM_EP_IFACE] {
        ctl.extend_from_slice(&v.to_le_bytes());
    }
    // hardware_data_port_list: число, затем ep_type, iface_id, rx и tx (модем их не смотрит)
    let mut hw = vec![1u8];
    for v in [BAM_EP_TYPE, BAM_EP_IFACE, 0, 0] {
        hw.extend_from_slice(&v.to_le_bytes());
    }
    // управляющий порт — только по SYN_DPM_CTL=1 (опыт: с ним модем не поднимает линию BAM)
    let req = if std::env::var_os("SYN_DPM_CTL").is_some() { Message::new(0x20).tlv(0x10, ctl).tlv(0x11, hw) } else { Message::new(0x20).tlv(0x11, hw) };
    if let Err(e) = dpm.call(req, T) {
        tracing::info!("DPM open port (BAM_DMUX): {e:#}");
    }
    let wda = Client::connect(svc::WDA, T, ind.clone())?;
    wda.call(
        Message::new(0x20)
            .u32(0x11, 2) // raw IP, без QMAP и агрегации: мукс на BAM-DMUX mainline не поддерживается
            .u32(0x12, 0)
            .u32(0x13, 0)
            .tlv(0x17, Transport::Bam.endpoint()),
        T,
    )
    .context("WDA set data format")?;
    Ok(())
}

/// BAM-DMUX: канал модем открывает, когда к нему привязан сеанс — `wwan0` ждём после старта WDS.
fn bam_link() -> Result<()> {
    for _ in 0..100 {
        if exists(BAM_DEV) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !exists(BAM_DEV) {
        bail!("{}", synshell_tr::t!("модем не открыл канал BAM-DMUX (нет {dev})", dev = BAM_DEV));
    }
    sh(&["ip", "link", "set", BAM_DEV, "up"])?;
    Ok(())
}

/// Каналы IPA и `rmnet_data0` (один раз за запуск модема; повтор безвреден). Возвращает пару каналов IPA
/// (consumer, producer) для DPM.
fn setup_kernel() -> Result<(u32, u32)> {
    // ipanetm регистрирует rmnet_ipa; ipa_clientsm открывает API IPA во фреймворке (без него «not registered
    // on ipa_fmwk» и «Embedded datapath not supported»); rmnet_core — rmnet_data*
    for m in ["ipa_clientsm", "ipanetm", "rmnet_core"] {
        let _ = Command::new("modprobe").arg(m).status();
    }
    for _ in 0..50 {
        if exists(IPA_DEV) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !exists(IPA_DEV) {
        bail!("нет {IPA_DEV} (драйвер rmnet_ipa)");
    }
    let pair = ext_ioctl(EXT_GET_EP_PAIR, &[]).context("каналы IPA (GET_EP_PAIR)")?;
    let consumer = u32::from_ne_bytes(pair[0..4].try_into().unwrap());
    let producer = u32::from_ne_bytes(pair[4..8].try_into().unwrap());
    // Форматы каналов: повторная настройка уже поднятого канала даёт ошибку — её не считаем
    if let Err(e) = ext_ioctl(EXT_SET_INGRESS_DATA_FORMAT, &INGRESS_FORMAT.to_ne_bytes()) {
        tracing::debug!("формат приёма IPA: {e}");
    }
    if let Err(e) = ext_ioctl(EXT_SET_EGRESS_DATA_FORMAT, &EGRESS_FORMAT.to_ne_bytes()) {
        tracing::debug!("формат передачи IPA: {e}");
    }
    sh(&["ip", "link", "set", IPA_DEV, "up"])?;
    if !exists(IPA_IFACE) {
        sh(&["ip", "link", "add", "link", IPA_DEV, "name", IPA_IFACE, "type", "rmnet", "mux_id", &MUX_ID.to_string(), "ingress-deaggregation", "on"])?;
    }
    let mut mux = (MUX_ID as u32).to_ne_bytes().to_vec();
    let mut name = [0u8; 16];
    name[..IPA_IFACE.len()].copy_from_slice(IPA_IFACE.as_bytes());
    mux.extend_from_slice(&name);
    ext_ioctl(EXT_ADD_MUX_CHANNEL, &mux).context("мукс-канал IPA")?;
    Ok((consumer, producer))
}

/// Модем: пара каналов IPA (DPM) и формат QMAP (WDA) на встроенной конечной точке.
fn setup_modem(ind: &std::sync::mpsc::Sender<Indication>, pair: (u32, u32)) -> Result<()> {
    let dpm = Client::connect(svc::DPM, T, ind.clone())?;
    let mut hw = vec![1u8];
    for v in [EP_TYPE, EP_IFACE, pair.0, pair.1] {
        hw.extend_from_slice(&v.to_le_bytes());
    }
    if let Err(e) = dpm.call(Message::new(0x20).tlv(0x11, hw), T) {
        // Порт уже открыт (повторный запуск демона) — модем отвечает ошибкой, это не мешает
        tracing::debug!("DPM open port: {e:#}");
    }
    let wda = Client::connect(svc::WDA, T, ind.clone())?;
    let mut ep = EP_TYPE.to_le_bytes().to_vec();
    ep.extend_from_slice(&EP_IFACE.to_le_bytes());
    wda.call(
        Message::new(0x20)
            .u32(0x11, 2) // raw IP
            .u32(0x12, 5) // QMAP
            .u32(0x13, 5)
            .u32(0x15, 32)
            .u32(0x16, 16384)
            .tlv(0x17, ep),
        T,
    )
    .context("WDA set data format")?;
    Ok(())
}

/// Сеанс: свой клиент WDS (индикации о сеансе — в `ind`), привязка к муксу, старт по профилю 1.
pub fn connect(ind: &std::sync::mpsc::Sender<Indication>, transport: Transport) -> Result<Session> {
    let wds = Client::connect(svc::WDS, T, ind.clone())?;
    let ep = transport.endpoint();
    // BAM-DMUX: Bind Mux Data Port с муксом 0 (Bind Data Port на этих модемах не работает — ModemManager)
    let mux = if transport == Transport::Bam { 0 } else { MUX_ID };
    wds.call(Message::new(0xA2).tlv(0x10, ep).u8(0x11, mux), T).context("WDS bind mux data port")?;
    let r = wds.call_raw(Message::new(0x20).u8(0x19, 4).u8(0x31, 1), Duration::from_secs(60))?;
    if let Err(e) = r.result() {
        let reason = r.reader(0x11).and_then(|mut rd| Some((rd.u16()?, rd.i16()?)));
        bail!("{}", match reason {
            Some((t, v)) => synshell_tr::t!("сеанс не открыт: {e} (причина {t}/{v})", e = e, t = t, v = v),
            None => synshell_tr::t!("сеанс не открыт: {e}", e = e),
        });
    }
    let handle = r.reader(0x01).and_then(|mut rd| rd.u32()).context("нет номера сеанса")?;
    if transport == Transport::Bam {
        if let Err(e) = bam_link() {
            let _ = wds.call(Message::new(0x21).u32(0x01, handle), T);
            return Err(e);
        }
    }
    let settings = current_settings(&wds)?;
    Ok(Session { wds, handle, settings })
}

fn current_settings(wds: &Client) -> Result<Settings> {
    let mask: u32 = (1 << 4) | (1 << 8) | (1 << 9) | (1 << 13) | (1 << 15);
    let r = wds.call(Message::new(0x2D).u32(0x10, mask), T)?;
    let ip = |t: u8| r.reader(t).and_then(|mut rd| rd.u32()).filter(|v| *v != 0).map(Ipv4Addr::from);
    let mask = r.reader(0x21).and_then(|mut rd| rd.u32()).unwrap_or(0xFFFF_FF00);
    Ok(Settings {
        address: ip(0x1E),
        prefix: mask.count_ones() as u8,
        gateway: ip(0x20),
        dns: [ip(0x15), ip(0x16)].into_iter().flatten().collect(),
        mtu: r.reader(0x29).and_then(|mut rd| rd.u32()).filter(|v| *v > 0),
    })
}

/// Адрес, MTU, маршрут по умолчанию и DNS (systemd-resolved, на интерфейс).
pub fn configure(s: &Settings) -> Result<()> {
    let Some(addr) = s.address else { bail!("{}", synshell_tr::t!("модем не выдал адрес IPv4")) };
    let _ = sh(&["ip", "addr", "flush", "dev", iface()]);
    sh(&["ip", "addr", "add", &format!("{addr}/{}", s.prefix.clamp(1, 32)), "dev", iface()])?;
    if let Some(mtu) = s.mtu {
        let _ = sh(&["ip", "link", "set", iface(), "mtu", &mtu.to_string()]);
    }
    sh(&["ip", "link", "set", iface(), "up"])?;
    // Raw IP: шлюз не нужен, маршрут — в интерфейс
    let _ = sh(&["ip", "route", "replace", "default", "dev", iface(), "metric", ROUTE_METRIC]);
    if !s.dns.is_empty() {
        let mut args = vec!["resolvectl", "dns", iface()];
        let dns: Vec<String> = s.dns.iter().map(|d| d.to_string()).collect();
        args.extend(dns.iter().map(String::as_str));
        let _ = sh(&args);
        let _ = sh(&["resolvectl", "default-route", iface(), "true"]);
    }
    Ok(())
}

/// Закрыть сеанс (если модем жив) и снять настройки интерфейса.
pub fn disconnect(session: Option<&Session>) {
    if let Some(s) = session {
        if let Err(e) = s.wds.call(Message::new(0x21).u32(0x01, s.handle), T) {
            tracing::debug!("WDS stop network: {e:#}");
        }
    }
    deconfigure();
}

pub fn deconfigure() {
    if exists(iface()) {
        let _ = sh(&["resolvectl", "revert", iface()]);
        let _ = sh(&["ip", "route", "flush", "dev", iface()]);
        let _ = sh(&["ip", "addr", "flush", "dev", iface()]);
        let _ = sh(&["ip", "link", "set", iface(), "down"]);
    }
}
