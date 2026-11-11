//! Камеры без своего ISP через media controller (mainline-ядра: Qualcomm camss и подобные): сенсор отдаёт
//! сырой Bayer, путь до видеоузла (CSIPHY → CSID → ISPIF → VFE RDI → `/dev/videoN` у camss) находится по
//! media-графу (`/dev/media*`), ссылки включаются, формат проставляется на каждом звене. Кадр собирает
//! программный ISP ([`crate::isp`]): распаковка RAW8/10/12, чёрный уровень, дебайер, баланс белого,
//! гамма → NV12 полного диапазона; автоэкспозиция — контролами сенсора (выдержка, аналоговое и цифровое
//! усиление, гашение кадра), автофокус — по контрасту мотором линзы (подустройство Lens, связанное с
//! сенсором). Только libc.

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use syngui::t;

use crate::isp::{self, Bayer, Isp, Limits, RawFmt};
use crate::syncamd::{self, Control, Meta};
use crate::v4l2::{self, fourcc, xioctl, zeroed, Buffer, Capability, FmtDesc, RequestBuffers};

// ─── ioctl ──────────────────────────────────────────────────────────────────

const fn ioc(dir: u32, ty: u8, nr: u32, size: usize) -> libc::c_ulong {
    ((dir << 30) | ((size as u32) << 16) | ((ty as u32) << 8) | nr) as libc::c_ulong
}
const RW: u32 = 3;

const MEDIA_IOC_DEVICE_INFO: libc::c_ulong = ioc(RW, b'|', 0, std::mem::size_of::<MediaDeviceInfo>());
const MEDIA_IOC_SETUP_LINK: libc::c_ulong = ioc(RW, b'|', 3, std::mem::size_of::<LinkDesc>());
const MEDIA_IOC_G_TOPOLOGY: libc::c_ulong = ioc(RW, b'|', 4, std::mem::size_of::<Topology>());
const SUBDEV_ENUM_MBUS_CODE: libc::c_ulong = ioc(RW, b'V', 2, std::mem::size_of::<MbusCodeEnum>());
const SUBDEV_G_FMT: libc::c_ulong = ioc(RW, b'V', 4, std::mem::size_of::<SubdevFormat>());
const SUBDEV_S_FMT: libc::c_ulong = ioc(RW, b'V', 5, std::mem::size_of::<SubdevFormat>());
const SUBDEV_ENUM_FRAME_SIZE: libc::c_ulong = ioc(RW, b'V', 74, std::mem::size_of::<FrameSizeEnum>());
const G_CTRL: libc::c_ulong = ioc(RW, b'V', 27, std::mem::size_of::<Ctrl>());
const S_CTRL: libc::c_ulong = ioc(RW, b'V', 28, std::mem::size_of::<Ctrl>());
const QUERYCTRL: libc::c_ulong = ioc(RW, b'V', 36, std::mem::size_of::<QueryCtrl>());
const G_EXT_CTRLS: libc::c_ulong = ioc(RW, b'V', 71, std::mem::size_of::<ExtControls>());
const S_FMT_MP: libc::c_ulong = ioc(RW, b'V', 5, std::mem::size_of::<FormatMp>());

const ENT_F_IO_V4L: u32 = 0x0001_0001;
const ENT_F_CAM_SENSOR: u32 = 0x0002_0001;
const ENT_F_LENS: u32 = 0x0002_0003;
const INTF_T_V4L_VIDEO: u32 = 0x200;
const INTF_T_V4L_SUBDEV: u32 = 0x203;
const LNK_FL_ENABLED: u32 = 1;
const LNK_FL_IMMUTABLE: u32 = 2;
const LNK_FL_TYPE: u32 = 0xf << 28;
const LNK_FL_DATA: u32 = 0;
const LNK_FL_INTERFACE: u32 = 1 << 28;
const LNK_FL_ANCILLARY: u32 = 2 << 28;
const PAD_FL_SINK: u32 = 1;
const PAD_FL_SOURCE: u32 = 2;

const BUF_TYPE_CAPTURE_MPLANE: u32 = 9;
const CAP_VIDEO_CAPTURE_MPLANE: u32 = 0x1000;
const FIELD_NONE: u32 = 1;
const FORMAT_ACTIVE: u32 = 1;

pub(crate) const CID_EXPOSURE: u32 = 0x0098_0911;
pub(crate) const CID_CAMERA_ORIENTATION: u32 = 0x009a_0922;
pub(crate) const CID_SENSOR_ROTATION: u32 = 0x009a_0923;
pub(crate) const CID_FOCUS_ABSOLUTE: u32 = 0x009a_090a;
pub(crate) const CID_VBLANK: u32 = 0x009e_0901;
pub(crate) const CID_HBLANK: u32 = 0x009e_0902;
pub(crate) const CID_ANALOGUE_GAIN: u32 = 0x009e_0903;
pub(crate) const CID_PIXEL_RATE: u32 = 0x009f_0902;
pub(crate) const CID_DIGITAL_GAIN: u32 = 0x009f_0905;

#[repr(C)]
struct MediaDeviceInfo {
    driver: [u8; 16],
    model: [u8; 32],
    serial: [u8; 40],
    bus_info: [u8; 32],
    media_version: u32,
    hw_revision: u32,
    driver_version: u32,
    reserved: [u32; 31],
}

#[repr(C)]
#[derive(Default)]
struct Topology {
    version: u64,
    num_entities: u32,
    r1: u32,
    ptr_entities: u64,
    num_interfaces: u32,
    r2: u32,
    ptr_interfaces: u64,
    num_pads: u32,
    r3: u32,
    ptr_pads: u64,
    num_links: u32,
    r4: u32,
    ptr_links: u64,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct V2Entity {
    id: u32,
    name: [u8; 64],
    function: u32,
    flags: u32,
    reserved: [u32; 5],
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct V2Interface {
    id: u32,
    intf_type: u32,
    flags: u32,
    reserved: [u32; 9],
    major: u32,
    minor: u32,
    raw: [u32; 14],
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct V2Pad {
    id: u32,
    entity_id: u32,
    flags: u32,
    index: u32,
    reserved: [u32; 4],
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct V2Link {
    id: u32,
    source_id: u32,
    sink_id: u32,
    flags: u32,
    reserved: [u32; 6],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PadDesc {
    entity: u32,
    index: u16,
    flags: u32,
    reserved: [u32; 2],
}

#[repr(C)]
#[derive(Default)]
struct LinkDesc {
    source: PadDesc,
    sink: PadDesc,
    flags: u32,
    reserved: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct MbusFmt {
    width: u32,
    height: u32,
    code: u32,
    field: u32,
    colorspace: u32,
    ycbcr_enc: u16,
    quantization: u16,
    xfer_func: u16,
    flags: u16,
    reserved: [u16; 10],
}

#[repr(C)]
#[derive(Default)]
struct SubdevFormat {
    which: u32,
    pad: u32,
    format: MbusFmt,
    stream: u32,
    reserved: [u32; 7],
}

#[repr(C)]
#[derive(Default)]
struct MbusCodeEnum {
    pad: u32,
    index: u32,
    code: u32,
    which: u32,
    flags: u32,
    stream: u32,
    reserved: [u32; 6],
}

#[repr(C)]
#[derive(Default)]
struct FrameSizeEnum {
    index: u32,
    pad: u32,
    code: u32,
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
    which: u32,
    stream: u32,
    reserved: [u32; 7],
}

#[repr(C)]
#[derive(Default)]
struct Ctrl {
    id: u32,
    value: i32,
}

#[repr(C)]
struct QueryCtrl {
    id: u32,
    ty: u32,
    name: [u8; 32],
    minimum: i32,
    maximum: i32,
    step: i32,
    default_value: i32,
    flags: u32,
    reserved: [u32; 2],
}

#[repr(C, packed)]
struct ExtControl {
    id: u32,
    size: u32,
    reserved2: u32,
    value64: i64,
}

#[repr(C)]
struct ExtControls {
    which: u32,
    count: u32,
    error_idx: u32,
    request_fd: i32,
    reserved: u32,
    controls: *mut ExtControl,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PlanePixFmt {
    sizeimage: u32,
    bytesperline: u32,
    reserved: [u16; 6],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PixMp {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    colorspace: u32,
    planes: [PlanePixFmt; 8],
    num_planes: u8,
    flags: u8,
    ycbcr_enc: u8,
    quantization: u8,
    xfer_func: u8,
    reserved: [u8; 7],
}

#[repr(C, align(8))]
struct FormatMp {
    ty: u32,
    _pad: u32,
    pix: PixMp,
    rest: [u8; 200 - std::mem::size_of::<PixMp>()],
}

/// Однопланарный формат (`struct v4l2_format` с `v4l2_pix_format`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PixSp {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    priv_: u32,
    flags: u32,
    ycbcr_enc: u32,
    quantization: u32,
    xfer_func: u32,
}

#[repr(C, align(8))]
struct FormatSp {
    ty: u32,
    _pad: u32,
    pix: PixSp,
    rest: [u8; 200 - std::mem::size_of::<PixSp>()],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Plane {
    bytesused: u32,
    length: u32,
    m: u64,
    data_offset: u32,
    reserved: [u32; 11],
}

fn cstr(b: &[u8]) -> String {
    v4l2::cstr(b)
}

fn open_rw(path: &Path) -> io::Result<OwnedFd> {
    v4l2::open_dev(path)
}

// ─── Форматы Bayer ──────────────────────────────────────────────────────────

/// Код шины → (порядок, бит).
fn mbus_bayer(code: u32) -> Option<(Bayer, u32)> {
    Some(match code {
        0x3001 => (Bayer::Bggr, 8),
        0x3013 => (Bayer::Gbrg, 8),
        0x3002 => (Bayer::Grbg, 8),
        0x3014 => (Bayer::Rggb, 8),
        0x3007 => (Bayer::Bggr, 10),
        0x300e => (Bayer::Gbrg, 10),
        0x300a => (Bayer::Grbg, 10),
        0x300f => (Bayer::Rggb, 10),
        0x3008 => (Bayer::Bggr, 12),
        0x3010 => (Bayer::Gbrg, 12),
        0x3011 => (Bayer::Grbg, 12),
        0x3012 => (Bayer::Rggb, 12),
        _ => return None,
    })
}

/// Форматы видеоузла для порядка и разрядности: упакованный MIPI (как у RDI), затем по 16 бит на точку.
fn pixfmts(order: Bayer, bits: u32) -> Vec<(u32, RawFmt)> {
    let o = order as usize;
    let f = |packed: bool, cc: [&[u8; 4]; 4]| (fourcc(cc[o]), RawFmt { order, bits, packed });
    match bits {
        8 => vec![f(false, [b"RGGB", b"GRBG", b"GBRG", b"BA81"])],
        10 => vec![f(true, [b"pRAA", b"pgAA", b"pGAA", b"pBAA"]), f(false, [b"RG10", b"BA10", b"GB10", b"BG10"])],
        _ => vec![f(true, [b"pRCC", b"pgCC", b"pGCC", b"pBCC"]), f(false, [b"RG12", b"BA12", b"GB12", b"BG12"])],
    }
}

// ─── Граф ───────────────────────────────────────────────────────────────────

struct Graph {
    entities: Vec<(u32, String, u32)>,
    pads: Vec<V2Pad>,
    links: Vec<V2Link>,
    /// Узел /dev сущности (через интерфейс).
    nodes: Vec<(u32, PathBuf)>,
}

impl Graph {
    fn read(fd: i32) -> io::Result<Graph> {
        for _ in 0..4 {
            let mut t = Topology::default();
            xioctl(fd, MEDIA_IOC_G_TOPOLOGY, &mut t)?;
            let mut ents = vec![zeroed::<V2Entity>(); t.num_entities as usize];
            let mut intfs = vec![zeroed::<V2Interface>(); t.num_interfaces as usize];
            let mut pads = vec![zeroed::<V2Pad>(); t.num_pads as usize];
            let mut links = vec![zeroed::<V2Link>(); t.num_links as usize];
            let ver = t.version;
            t.ptr_entities = ents.as_mut_ptr() as u64;
            t.ptr_interfaces = intfs.as_mut_ptr() as u64;
            t.ptr_pads = pads.as_mut_ptr() as u64;
            t.ptr_links = links.as_mut_ptr() as u64;
            xioctl(fd, MEDIA_IOC_G_TOPOLOGY, &mut t)?;
            if t.version != ver {
                continue; // граф изменился между вызовами
            }
            let entities = ents.iter().map(|e| (e.id, cstr(&{ e.name }), e.function)).collect();
            let mut nodes = Vec::new();
            for l in links.iter().filter(|l| l.flags & LNK_FL_TYPE == LNK_FL_INTERFACE) {
                let Some(i) = intfs.iter().find(|i| i.id == l.source_id) else { continue };
                if i.intf_type != INTF_T_V4L_VIDEO && i.intf_type != INTF_T_V4L_SUBDEV {
                    continue;
                }
                if let Some(p) = devnode(i.major, i.minor) {
                    nodes.push((l.sink_id, p));
                }
            }
            return Ok(Graph { entities, pads, links, nodes });
        }
        Err(io::Error::other("media topology keeps changing"))
    }

    fn name(&self, ent: u32) -> String {
        self.entities.iter().find(|e| e.0 == ent).map(|e| e.1.clone()).unwrap_or_default()
    }

    fn function(&self, ent: u32) -> u32 {
        self.entities.iter().find(|e| e.0 == ent).map(|e| e.2).unwrap_or(0)
    }

    fn node(&self, ent: u32) -> Option<PathBuf> {
        self.nodes.iter().find(|n| n.0 == ent).map(|n| n.1.clone())
    }

    fn pad(&self, id: u32) -> Option<&V2Pad> {
        self.pads.iter().find(|p| p.id == id)
    }

    /// Ссылки данных: (исток: сущность, пад) → (сток: сущность, пад), флаги.
    fn data_links(&self) -> Vec<(u32, u16, u32, u16, u32)> {
        self.links
            .iter()
            .filter(|l| l.flags & LNK_FL_TYPE == LNK_FL_DATA)
            .filter_map(|l| {
                let s = self.pad(l.source_id)?;
                let k = self.pad(l.sink_id)?;
                if s.flags & PAD_FL_SOURCE == 0 || k.flags & PAD_FL_SINK == 0 {
                    return None;
                }
                Some((s.entity_id, s.index as u16, k.entity_id, k.index as u16, l.flags))
            })
            .collect()
    }

    /// Все пути от пада сенсора до видеоузла (не длиннее 10 звеньев), короткие первыми; пути через
    /// «pix»-блоки ISP (им нужен свой формат) — в конце.
    fn paths(&self, sensor: u32) -> Vec<Vec<Hop>> {
        let links = self.data_links();
        let mut out = Vec::new();
        let mut cur: Vec<Hop> = Vec::new();
        self.walk(sensor, &links, &mut cur, &mut out);
        out.sort_by_key(|p: &Vec<Hop>| (p.iter().any(|h| h.name.contains("pix")), p.len()));
        out
    }

    fn walk(&self, ent: u32, links: &[(u32, u16, u32, u16, u32)], cur: &mut Vec<Hop>, out: &mut Vec<Vec<Hop>>) {
        if cur.len() >= 10 || out.len() >= 256 {
            return;
        }
        for &(se, sp, ke, kp, fl) in links.iter().filter(|l| l.0 == ent) {
            if cur.iter().any(|h| h.sink_ent == ke) || self.function(ke) == ENT_F_CAM_SENSOR {
                continue;
            }
            cur.push(Hop { src_ent: se, src_pad: sp, sink_ent: ke, sink_pad: kp, immutable: fl & LNK_FL_IMMUTABLE != 0, name: self.name(ke) });
            if self.function(ke) == ENT_F_IO_V4L {
                out.push(cur.clone());
            } else {
                self.walk(ke, links, cur, out);
            }
            cur.pop();
        }
    }
}

fn devnode(major: u32, minor: u32) -> Option<PathBuf> {
    let s = std::fs::read_to_string(format!("/sys/dev/char/{major}:{minor}/uevent")).ok()?;
    let name = s.lines().find_map(|l| l.strip_prefix("DEVNAME="))?;
    Some(Path::new("/dev").join(name))
}

/// Звено пути: ссылка (исток → сток) и имя сущности-стока.
#[derive(Clone, Debug, PartialEq)]
struct Hop {
    src_ent: u32,
    src_pad: u16,
    sink_ent: u32,
    sink_pad: u16,
    immutable: bool,
    name: String,
}

// ─── Контролы ───────────────────────────────────────────────────────────────

/// Диапазон контрола: (min, max, step, default).
pub(crate) fn query(fd: i32, id: u32) -> Option<(i32, i32, i32, i32)> {
    let mut q: QueryCtrl = zeroed();
    q.id = id;
    xioctl(fd, QUERYCTRL, &mut q).ok()?;
    if q.flags & 1 != 0 {
        return None; // V4L2_CTRL_FLAG_DISABLED
    }
    Some((q.minimum, q.maximum, q.step.max(1), q.default_value))
}

pub(crate) fn get(fd: i32, id: u32) -> Option<i32> {
    let mut c = Ctrl { id, value: 0 };
    xioctl(fd, G_CTRL, &mut c).ok()?;
    Some(c.value)
}

pub(crate) fn set(fd: i32, id: u32, v: i32) -> bool {
    let mut c = Ctrl { id, value: v };
    xioctl(fd, S_CTRL, &mut c).is_ok()
}

fn get64(fd: i32, id: u32) -> Option<i64> {
    let mut c = ExtControl { id, size: 0, reserved2: 0, value64: 0 };
    let mut cs = ExtControls { which: 0, count: 1, error_idx: 0, request_fd: 0, reserved: 0, controls: &mut c };
    xioctl(fd, G_EXT_CTRLS, &mut cs).ok()?;
    Some(c.value64)
}

/// Пределы управления сенсором (на открытом подустройстве).
fn limits(fd: i32, width: u32, height: u32) -> Limits {
    let exposure = query(fd, CID_EXPOSURE);
    let vblank = query(fd, CID_VBLANK);
    let vb_cur = get(fd, CID_VBLANK).or(vblank.map(|v| v.3));
    let hblank = get(fd, CID_HBLANK);
    let rate = get64(fd, CID_PIXEL_RATE).filter(|r| *r > 0);
    // время строки: (ширина + гашение строки) / частота точек; без них — 30 к/с на текущий кадр
    let vts = height as i64 + vb_cur.unwrap_or(0) as i64;
    let line_ns = match (rate, hblank) {
        (Some(r), Some(hb)) => ((width as i64 + hb as i64) as f64 * 1e9 / r as f64).max(1.0),
        _ => 1e9 / 30.0 / vts.max(1) as f64,
    };
    // запас выдержки до длины кадра: max выдержки при текущем гашении
    let margin = match (exposure, vb_cur) {
        (Some(e), Some(vb)) => (height as i32 + vb - e.1).clamp(0, 64),
        _ => 4,
    };
    Limits {
        exposure,
        analogue: query(fd, CID_ANALOGUE_GAIN),
        digital: query(fd, CID_DIGITAL_GAIN),
        vblank,
        height,
        line_ns,
        margin,
    }
}

// ─── Камера ─────────────────────────────────────────────────────────────────

/// Камера media controller: сенсор, пути до видеоузла, размеры кадра.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub media: PathBuf,
    /// Имя сущности сенсора («hi846 3-0020»).
    pub name: String,
    /// Драйвер media («qcom-camss»).
    pub driver: String,
    sensor_ent: u32,
    sensor_pad: u16,
    pub sensor_node: PathBuf,
    /// Мотор линзы (автофокус).
    pub lens_node: Option<PathBuf>,
    paths: Vec<Vec<Hop>>,
    /// Код шины (Bayer) и размеры кадра сенсора, по убыванию площади.
    pub code: u32,
    pub modes: Vec<(u32, u32)>,
    pub front: bool,
    /// Поворот датчика (camera_sensor_rotation), градусы по часовой.
    pub rotation: u32,
    /// Описание возможностей в виде камеры syncamd (интерфейс «Камеры» общий).
    pub caps: syncamd::Camera,
}

/// Камеры всех `/dev/media*` с сенсором Bayer и путём до видеоузла. `first_id` — номер первой.
pub fn devices(first_id: u32) -> Vec<Device> {
    let mut medias: Vec<PathBuf> = std::fs::read_dir("/dev")
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("media"))).collect())
        .unwrap_or_default();
    medias.sort();
    let mut out = Vec::new();
    for m in medias {
        let Ok(fd) = open_rw(&m) else { continue };
        let mut info: MediaDeviceInfo = zeroed();
        if xioctl(fd.as_raw_fd(), MEDIA_IOC_DEVICE_INFO, &mut info).is_err() {
            continue;
        }
        let driver = cstr(&info.driver);
        // веб-камеры UVC — у них свой ISP, их берёт бэкенд V4L2
        if driver == "uvcvideo" {
            continue;
        }
        let Ok(g) = Graph::read(fd.as_raw_fd()) else { continue };
        for &(ent, ref name, func) in &g.entities {
            if func != ENT_F_CAM_SENSOR {
                continue;
            }
            let id = first_id + out.len() as u32;
            match probe(&m, &driver, &g, ent, name, id) {
                Some(d) => out.push(d),
                None => tracing::debug!("mcam: {name}: не подходит"),
            }
        }
    }
    out
}

fn probe(media: &Path, driver: &str, g: &Graph, ent: u32, name: &str, id: u32) -> Option<Device> {
    let sensor_node = g.node(ent)?;
    let paths = g.paths(ent);
    let first = paths.first()?;
    let sensor_pad = first[0].src_pad;
    let fd = open_rw(&sensor_node).ok()?;
    let raw = fd.as_raw_fd();
    // код Bayer: 10 бит, затем 12, затем 8
    let mut codes = Vec::new();
    for i in 0..64 {
        let mut e = MbusCodeEnum { pad: sensor_pad as u32, index: i, which: FORMAT_ACTIVE, ..Default::default() };
        if xioctl(raw, SUBDEV_ENUM_MBUS_CODE, &mut e).is_err() {
            break;
        }
        if let Some((_, bits)) = mbus_bayer(e.code) {
            codes.push((e.code, bits));
        }
    }
    let code = codes.iter().min_by_key(|(_, b)| match b {
        10 => 0,
        12 => 1,
        _ => 2,
    })?.0;
    let mut modes = Vec::new();
    for i in 0..64 {
        let mut e = FrameSizeEnum { index: i, pad: sensor_pad as u32, code, which: FORMAT_ACTIVE, ..Default::default() };
        if xioctl(raw, SUBDEV_ENUM_FRAME_SIZE, &mut e).is_err() {
            break;
        }
        modes.push((e.max_width & !1, e.max_height & !1));
    }
    if modes.is_empty() {
        let mut f = SubdevFormat { which: FORMAT_ACTIVE, pad: sensor_pad as u32, ..Default::default() };
        xioctl(raw, SUBDEV_G_FMT, &mut f).ok()?;
        modes.push((f.format.width & !1, f.format.height & !1));
    }
    modes.sort_by_key(|(w, h)| std::cmp::Reverse(*w as u64 * *h as u64));
    modes.dedup();
    let front = get(raw, CID_CAMERA_ORIENTATION) == Some(0);
    let rotation = get(raw, CID_SENSOR_ROTATION).unwrap_or(0).rem_euclid(360) as u32;
    // мотор линзы: вспомогательная ссылка сенсор → линза, иначе единственная линза графа
    let lenses: Vec<u32> = g.entities.iter().filter(|e| e.2 == ENT_F_LENS).map(|e| e.0).collect();
    let linked = g.links.iter().find(|l| l.flags & LNK_FL_TYPE == LNK_FL_ANCILLARY && l.source_id == ent && lenses.contains(&{ l.sink_id })).map(|l| l.sink_id);
    let sensors = g.entities.iter().filter(|e| e.2 == ENT_F_CAM_SENSOR).count();
    let lens = linked.or(if lenses.len() == 1 && sensors == 1 { Some(lenses[0]) } else { None });
    let lens_node = lens.and_then(|l| g.node(l)).filter(|p| open_rw(p).ok().is_some_and(|f| query(f.as_raw_fd(), CID_FOCUS_ABSOLUTE).is_some()));
    let (fw, fh) = modes[0];
    let lim = limits(raw, fw, fh);
    let caps = caps(id, front, rotation, &modes, &lim, lens_node.is_some());
    Some(Device {
        media: media.to_path_buf(),
        name: name.to_string(),
        driver: driver.to_string(),
        sensor_ent: ent,
        sensor_pad,
        sensor_node,
        lens_node,
        paths,
        code,
        modes,
        front,
        rotation,
        caps,
    })
}

/// Возможности для интерфейса «Камеры».
fn caps(id: u32, front: bool, rotation: u32, modes: &[(u32, u32)], lim: &Limits, af: bool) -> syncamd::Camera {
    // SAFETY: структура из целых и float, нули допустимы
    let mut c: syncamd::Camera = unsafe { std::mem::zeroed() };
    c.id = id;
    c.facing = if front { syncamd::FACING_FRONT } else { 0 };
    c.orientation = rotation as i32;
    let (fw, fh) = modes[0];
    c.sensor_width = fw;
    c.sensor_height = fh;
    let n = modes.len().min(c.sizes.len());
    c.nsizes = n as u32;
    for (i, m) in modes.iter().take(n).enumerate() {
        c.sizes[i] = syncamd::Size { width: m.0, height: m.1, min_frame_ns: 33_333_333 };
    }
    let nj = modes.len().min(c.jpeg.len());
    c.njpeg = nj as u32;
    for (i, m) in modes.iter().take(nj).enumerate() {
        c.jpeg[i] = [m.0, m.1];
    }
    c.photo_width = fw;
    c.photo_height = fh;
    c.full_width = fw;
    c.full_height = fh;
    c.max_zoom = isp::MAX_ZOOM;
    c.ev_min = -6;
    c.ev_max = 6;
    c.ev_step_num = 1;
    c.ev_step_den = 3;
    c.active = [0, 0, fw as i32, fh as i32];
    c.af = af as u32;
    c.iso_min = 100;
    c.iso_max = (100.0 * lim.max_gain()) as i32;
    if let Some(e) = lim.exposure {
        c.exposure_min = (e.0.max(1) as f64 * lim.line_ns) as i64;
        let max_lines = lim.vblank.map(|v| fh as i64 + v.1 as i64 - lim.margin as i64).unwrap_or(e.1 as i64);
        c.exposure_max = ((max_lines as f64 * lim.line_ns) as i64).min(isp::MAX_EXPOSURE_NS);
    }
    c.min_focus = if af { isp::MIN_FOCUS_DIOPTERS } else { 0.0 };
    c
}

impl Device {
    /// Размер кадра сенсора для запроса `w × h`: то же соотношение сторон — наименьший не меньше
    /// запрошенного (иначе наибольший), без совпадения — ближайший по площади.
    pub fn mode_for(&self, w: u32, h: u32) -> (u32, u32) {
        let same: Vec<(u32, u32)> = self.modes.iter().copied().filter(|(mw, mh)| (*mw as u64 * h as u64).abs_diff(*mh as u64 * w as u64) * 50 <= *mh as u64 * w as u64).collect();
        if let Some(m) = same.iter().filter(|(mw, mh)| *mw >= w && *mh >= h).min_by_key(|(mw, mh)| *mw as u64 * *mh as u64) {
            return *m;
        }
        if let Some(m) = same.first() {
            return *m;
        }
        let area = w as i64 * h as i64;
        self.modes.iter().copied().min_by_key(|(mw, mh)| (*mw as i64 * *mh as i64 - area).abs()).unwrap_or((w, h))
    }

    pub fn size_for(&self, aw: u32, ah: u32, max_w: u32) -> Option<(u32, u32)> {
        self.caps.yuv_for(aw, ah, max_w)
    }
}

// ─── Поток ──────────────────────────────────────────────────────────────────

pub struct OpenError {
    pub busy: bool,
    pub text: String,
}

fn os_err(what: &str, e: io::Error) -> OpenError {
    OpenError { busy: e.raw_os_error() == Some(libc::EBUSY), text: format!("{what}: {e}") }
}

/// Открытый поток: видеоузел RDI, подустройство сенсора, ISP.
pub struct Stream {
    fd: OwnedFd,
    sensor: OwnedFd,
    lens: Option<OwnedFd>,
    mplane: bool,
    bufs: Vec<(*mut u8, usize)>,
    /// Размер выхода (кадр NV12).
    pub width: u32,
    pub height: u32,
    /// Кадр сенсора.
    sw: u32,
    sh: u32,
    stride: usize,
    fmt: RawFmt,
    stop: AtomicBool,
    isp: Mutex<Isp>,
    timing: bool,
}

// SAFETY: отображения только читаются; ioctl на дескрипторах потокобезопасны, состояние ISP — под Mutex
unsafe impl Send for Stream {}
unsafe impl Sync for Stream {}

impl Stream {
    /// Включить конвейер и поток кадров размера сенсора, ближайшего к `w × h`. `video` — держать 30 к/с
    /// (в фото при темноте кадр удлиняется до 15 к/с).
    pub fn open(dev: &Device, w: u32, h: u32, video: bool) -> Result<Stream, OpenError> {
        let want = (w, h);
        let (w, h) = dev.mode_for(w, h);
        let media = open_rw(&dev.media).map_err(|e| os_err(&dev.media.display().to_string(), e))?;
        let g = Graph::read(media.as_raw_fd()).map_err(|e| os_err("media", e))?;
        let mut last = OpenError { busy: false, text: t!("нет пути от сенсора до видеоузла").into() };
        // сначала пути, у которых больше связей уже включено (настроенный конвейер; занятые чужим потоком
        // общие CSID/VFE так обходятся стороной — путей бывают десятки, перебор ограничен), а уже
        // заработавший — первым
        let links = g.data_links();
        let enabled = |p: &[Hop]| {
            p.iter().filter(|h| links.iter().any(|l| (l.0, l.1, l.2, l.3) == (h.src_ent, h.src_pad, h.sink_ent, h.sink_pad) && l.4 & LNK_FL_ENABLED != 0)).count()
        };
        let mut order: Vec<usize> = (0..dev.paths.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(enabled(&dev.paths[i])));
        let good = GOOD_PATH.lock().unwrap().iter().find(|(n, _)| *n == dev.name).map(|(_, i)| *i);
        if let Some(i) = good.filter(|i| *i < order.len()) {
            order.retain(|j| *j != i);
            order.insert(0, i);
        }
        // в предел — только неудачи с форматом и потоком; «занято» (один ioctl ссылки) — дёшево, пробуем дальше
        let mut tried = 0;
        for i in order {
            if tried >= 6 {
                break;
            }
            let path = &dev.paths[i];
            match Self::try_path(dev, &g, media.as_raw_fd(), path, w, h, want, video) {
                Ok(s) => {
                    let mut gp = GOOD_PATH.lock().unwrap();
                    gp.retain(|(n, _)| *n != dev.name);
                    gp.push((dev.name.clone(), i));
                    tracing::info!("mcam: {} {}×{} через {}", dev.name, s.width, s.height, path.iter().map(|h| h.name.as_str()).collect::<Vec<_>>().join(" → "));
                    return Ok(s);
                }
                // путь занят другим потоком (общий CSID/VFE с другой камерой) — пробовать следующий: у передней
                // и задней бывают свои; «занята» — только если свободного пути не нашлось
                Err(e) => {
                    tracing::debug!("mcam: путь {i}: {}", e.text);
                    if !e.busy {
                        tried += 1;
                    }
                    if e.busy || !last.busy {
                        last = e;
                    }
                }
            }
        }
        Err(last)
    }

    #[allow(clippy::too_many_arguments)]
    fn try_path(dev: &Device, g: &Graph, media: i32, path: &[Hop], w: u32, h: u32, want: (u32, u32), video: bool) -> Result<Stream, OpenError> {
        // ссылки: чужие включённые в стоки пути — выключить, свои — включить
        let links = g.data_links();
        for hop in path.iter().filter(|h| !h.immutable) {
            for &(se, sp, ke, kp, fl) in links.iter().filter(|l| l.2 == hop.sink_ent && l.3 == hop.sink_pad && l.4 & LNK_FL_ENABLED != 0) {
                if (se, sp) != (hop.src_ent, hop.src_pad) && fl & LNK_FL_IMMUTABLE == 0 {
                    setup_link(media, se, sp, ke, kp, false).map_err(|e| os_err(&t!("ссылка {a} → {b}", a = g.name(se), b = g.name(ke)), e))?;
                }
            }
            setup_link(media, hop.src_ent, hop.src_pad, hop.sink_ent, hop.sink_pad, true).map_err(|e| os_err(&t!("ссылка {a} → {b}", a = g.name(hop.src_ent), b = g.name(hop.sink_ent)), e))?;
        }
        // формат: сенсор, затем стоки и истоки каждого подустройства пути
        let sensor = open_rw(&dev.sensor_node).map_err(|e| os_err(&dev.sensor_node.display().to_string(), e))?;
        let mut mf = MbusFmt { width: w, height: h, code: dev.code, field: FIELD_NONE, ..Default::default() };
        mf = subdev_fmt(sensor.as_raw_fd(), dev.sensor_pad as u32, mf).map_err(|e| os_err(&dev.name, e))?;
        let (w, h, code) = (mf.width, mf.height, mf.code);
        let (order, bits) = mbus_bayer(code).ok_or_else(|| OpenError { busy: false, text: t!("сенсор отдал не Bayer") .into() })?;
        for (k, hop) in path.iter().enumerate() {
            let Some(node) = g.node(hop.sink_ent).filter(|_| g.function(hop.sink_ent) != ENT_F_IO_V4L) else { continue };
            let fd = open_rw(&node).map_err(|e| os_err(&node.display().to_string(), e))?;
            let got = subdev_fmt(fd.as_raw_fd(), hop.sink_pad as u32, mf).map_err(|e| os_err(&hop.name, e))?;
            if got.code != code || got.width != w || got.height != h {
                return Err(OpenError { busy: false, text: format!("{}: {:#x} {}×{}", hop.name, got.code, got.width, got.height) });
            }
            if let Some(next) = path.get(k + 1) {
                let _ = subdev_fmt(fd.as_raw_fd(), next.src_pad as u32, mf);
            }
        }
        let vnode = g.node(path.last().unwrap().sink_ent).ok_or_else(|| OpenError { busy: false, text: t!("у видеоузла нет /dev").into() })?;
        let fd = open_rw(&vnode).map_err(|e| os_err(&vnode.display().to_string(), e))?;
        let raw = fd.as_raw_fd();
        let mut cap: Capability = zeroed();
        xioctl(raw, v4l2::QUERYCAP, &mut cap).map_err(|e| os_err(&vnode.display().to_string(), e))?;
        let caps = if cap.capabilities & v4l2::CAP_DEVICE_CAPS != 0 { cap.device_caps } else { cap.capabilities };
        let mplane = caps & CAP_VIDEO_CAPTURE_MPLANE != 0;
        let ty = if mplane { BUF_TYPE_CAPTURE_MPLANE } else { v4l2::BUF_TYPE_CAPTURE };
        // какие форматы умеет узел
        let mut have = Vec::new();
        for i in 0..64 {
            let mut d: FmtDesc = zeroed();
            d.index = i;
            d.ty = ty;
            if xioctl(raw, v4l2::ENUM_FMT, &mut d).is_err() {
                break;
            }
            have.push(d.pixelformat);
        }
        let mut chosen = None;
        for (pf, rf) in pixfmts(order, bits) {
            if !have.is_empty() && !have.contains(&pf) {
                continue;
            }
            if let Ok((got, stride)) = set_video_fmt(raw, mplane, w, h, pf) {
                if got == pf {
                    chosen = Some((rf, stride));
                    break;
                }
            }
        }
        let (fmt, stride) = chosen.ok_or_else(|| OpenError { busy: false, text: t!("видеоузел не принял формат Bayer").into() })?;
        let mut rb = RequestBuffers { count: 4, ty, memory: v4l2::MEMORY_MMAP, capabilities: 0, flags: 0 };
        xioctl(raw, v4l2::REQBUFS, &mut rb).map_err(|e| os_err("REQBUFS", e))?;
        let mut bufs = Vec::new();
        for i in 0..rb.count {
            let mut planes = [zeroed::<Plane>(); 1];
            let mut b = new_buf(ty, mplane, &mut planes);
            b.index = i;
            xioctl(raw, v4l2::QUERYBUF, &mut b).map_err(|e| os_err("QUERYBUF", e))?;
            let (len, off) = if mplane { (planes[0].length as usize, planes[0].m & 0xffff_ffff) } else { (b.length as usize, b.m & 0xffff_ffff) };
            // SAFETY: отображение буфера драйвера по его смещению
            let ptr = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, raw, off as libc::off_t) };
            if ptr == libc::MAP_FAILED {
                return Err(os_err("mmap", io::Error::last_os_error()));
            }
            bufs.push((ptr as *mut u8, len));
            xioctl(raw, v4l2::QBUF, &mut b).map_err(|e| os_err("QBUF", e))?;
        }
        let lens = dev.lens_node.as_ref().and_then(|p| open_rw(p).ok());
        let lim = limits(sensor.as_raw_fd(), w, h);
        let focus = lens.as_ref().and_then(|l| query(l.as_raw_fd(), CID_FOCUS_ABSOLUTE));
        // сенсор мог дать другой размер (драйвер принимает не все объявленные режимы): соотношение сторон —
        // кадрированием середины
        let out = aspect_crop(w, h, want.0, want.1);
        let mut isp = Isp::new(&dev.name, w, h, out, fmt, lim, focus, video);
        let (ow, oh) = isp.out_size();
        // начальная экспозиция (прошлая для этой камеры) — до включения потока
        isp.apply_sensor(sensor.as_raw_fd(), lens.as_ref().map(|l| l.as_raw_fd()), true);
        let s = Stream {
            fd,
            sensor,
            lens,
            mplane,
            bufs,
            width: ow,
            height: oh,
            sw: w,
            sh: h,
            stride,
            fmt,
            stop: AtomicBool::new(false),
            isp: Mutex::new(isp),
            timing: std::env::var_os("SYNCAMERA_TIMING").is_some(),
        };
        let mut t = ty as i32;
        xioctl(raw, v4l2::STREAMON, &mut t).map_err(|e| os_err("STREAMON", e))?;
        Ok(s)
    }

    fn ty(&self) -> u32 {
        if self.mplane {
            BUF_TYPE_CAPTURE_MPLANE
        } else {
            v4l2::BUF_TYPE_CAPTURE
        }
    }

    /// Следующий кадр в NV12 (`width × height`) и его время, нс; `None` — поток остановлен.
    /// Если кадры копятся — берётся самый свежий.
    pub fn next_nv12(&self, out: &mut Vec<u8>) -> io::Result<Option<u64>> {
        let raw = self.fd.as_raw_fd();
        let ty = self.ty();
        let mut idle = 0;
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return Ok(None);
            }
            let mut pfd = libc::pollfd { fd: raw, events: libc::POLLIN, revents: 0 };
            // SAFETY: poll одного дескриптора
            let n = unsafe { libc::poll(&mut pfd, 1, 200) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            if n == 0 {
                idle += 1;
                if idle >= 15 {
                    return Err(io::Error::other(t!("камера не присылает кадры")));
                }
                continue;
            }
            if pfd.revents & libc::POLLERR != 0 && pfd.revents & libc::POLLIN == 0 {
                return Err(io::Error::other(t!("ошибка потока камеры")));
            }
            let mut planes = [zeroed::<Plane>(); 1];
            let mut b = new_buf(ty, self.mplane, &mut planes);
            match xioctl(raw, v4l2::DQBUF, &mut b) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(e),
            }
            // свежее есть — прежний вернуть
            loop {
                let mut p2 = [zeroed::<Plane>(); 1];
                let mut b2 = new_buf(ty, self.mplane, &mut p2);
                if xioctl(raw, v4l2::DQBUF, &mut b2).is_err() {
                    break;
                }
                let mut back = b;
                back.m = if self.mplane { planes.as_mut_ptr() as u64 } else { back.m };
                let _ = xioctl(raw, v4l2::QBUF, &mut back);
                b = b2;
                planes = p2;
                b.m = if self.mplane { planes.as_mut_ptr() as u64 } else { b.m };
            }
            let ts = b.timestamp.tv_sec as u64 * 1_000_000_000 + b.timestamp.tv_usec as u64 * 1000;
            let used = if self.mplane { planes[0].bytesused } else { b.bytesused } as usize;
            let (ptr, len) = self.bufs[b.index as usize];
            let error = b.flags & 0x40 != 0; // V4L2_BUF_FLAG_ERROR
            let need = self.stride * self.sh as usize;
            if !error && used.max(need) <= len {
                // SAFETY: буфер у нас до QBUF
                let data = unsafe { std::slice::from_raw_parts(ptr, need) };
                let (w, h) = (self.width as usize, self.height as usize);
                out.resize(w * h * 3 / 2, 0);
                let t = std::time::Instant::now();
                let mut isp = self.isp.lock().unwrap();
                isp.process(data, self.stride, out);
                isp.apply_sensor(self.sensor.as_raw_fd(), self.lens.as_ref().map(|l| l.as_raw_fd()), false);
                drop(isp);
                if self.timing {
                    eprintln!("isp {}×{} → {w}×{h}: {:.1} мс", self.sw, self.sh, t.elapsed().as_secs_f64() * 1e3);
                }
            }
            b.m = if self.mplane { planes.as_mut_ptr() as u64 } else { b.m };
            let _ = xioctl(raw, v4l2::QBUF, &mut b);
            if !error {
                return Ok(Some(ts));
            }
        }
    }

    /// Состояние 3A для интерфейса (раз в несколько кадров).
    pub fn take_meta(&self) -> Option<Meta> {
        self.isp.lock().unwrap().take_meta()
    }

    pub fn control(&self, c: &Control) {
        self.isp.lock().unwrap().control(c);
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    pub fn format(&self) -> RawFmt {
        self.fmt
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let mut t = self.ty() as i32;
        let _ = xioctl(self.fd.as_raw_fd(), v4l2::STREAMOFF, &mut t);
        self.isp.lock().unwrap().remember();
        for (p, len) in &self.bufs {
            // SAFETY: отображения из open
            unsafe { libc::munmap(*p as *mut libc::c_void, *len) };
        }
    }
}

/// Рабочий путь по имени сенсора (в пределах процесса).
static GOOD_PATH: Mutex<Vec<(String, usize)>> = Mutex::new(Vec::new());

fn new_buf(ty: u32, mplane: bool, planes: &mut [Plane; 1]) -> Buffer {
    let mut b: Buffer = zeroed();
    b.ty = ty;
    b.memory = v4l2::MEMORY_MMAP;
    if mplane {
        b.m = planes.as_mut_ptr() as u64;
        b.length = 1;
    }
    b
}

/// Середина кадра `w × h` с соотношением сторон `aw:ah` (кратно 4), если оно отличается больше чем на 1 %.
fn aspect_crop(w: u32, h: u32, aw: u32, ah: u32) -> Option<(u32, u32)> {
    if aw == 0 || ah == 0 || (w as u64 * ah as u64).abs_diff(h as u64 * aw as u64) * 100 <= h as u64 * aw as u64 {
        return None;
    }
    if w as u64 * ah as u64 > h as u64 * aw as u64 {
        Some(((h as u64 * aw as u64 / ah as u64) as u32 & !3, h & !3))
    } else {
        Some((w & !3, (w as u64 * ah as u64 / aw as u64) as u32 & !3))
    }
}

fn setup_link(media: i32, se: u32, sp: u16, ke: u32, kp: u16, on: bool) -> io::Result<()> {
    let mut d = LinkDesc {
        source: PadDesc { entity: se, index: sp, ..Default::default() },
        sink: PadDesc { entity: ke, index: kp, ..Default::default() },
        flags: if on { LNK_FL_ENABLED } else { 0 },
        reserved: [0; 2],
    };
    xioctl(media, MEDIA_IOC_SETUP_LINK, &mut d)
}

fn subdev_fmt(fd: i32, pad: u32, f: MbusFmt) -> io::Result<MbusFmt> {
    let mut s = SubdevFormat { which: FORMAT_ACTIVE, pad, format: f, ..Default::default() };
    xioctl(fd, SUBDEV_S_FMT, &mut s)?;
    Ok(s.format)
}

/// Формат видеоузла; ответ — принятый формат и длина строки.
fn set_video_fmt(fd: i32, mplane: bool, w: u32, h: u32, pf: u32) -> io::Result<(u32, usize)> {
    if mplane {
        let mut f: FormatMp = zeroed();
        f.ty = BUF_TYPE_CAPTURE_MPLANE;
        f.pix.width = w;
        f.pix.height = h;
        f.pix.pixelformat = pf;
        f.pix.field = FIELD_NONE;
        f.pix.num_planes = 1;
        xioctl(fd, S_FMT_MP, &mut f)?;
        if f.pix.width != w || f.pix.height != h {
            return Err(io::Error::other("size"));
        }
        Ok((f.pix.pixelformat, f.pix.planes[0].bytesperline as usize))
    } else {
        let mut f: FormatSp = zeroed();
        f.ty = v4l2::BUF_TYPE_CAPTURE;
        f.pix.width = w;
        f.pix.height = h;
        f.pix.pixelformat = pf;
        f.pix.field = FIELD_NONE;
        xioctl(fd, v4l2::S_FMT, &mut f)?;
        if f.pix.width != w || f.pix.height != h {
            return Err(io::Error::other("size"));
        }
        Ok((f.pix.pixelformat, f.pix.bytesperline as usize))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_sizes() {
        assert_eq!(std::mem::size_of::<MediaDeviceInfo>(), 256);
        assert_eq!(std::mem::size_of::<Topology>(), 72);
        assert_eq!(std::mem::size_of::<V2Entity>(), 96);
        assert_eq!(std::mem::size_of::<V2Interface>(), 112);
        assert_eq!(std::mem::size_of::<V2Pad>(), 32);
        assert_eq!(std::mem::size_of::<V2Link>(), 40);
        assert_eq!(std::mem::size_of::<PadDesc>(), 20);
        assert_eq!(std::mem::size_of::<LinkDesc>(), 52);
        assert_eq!(std::mem::size_of::<SubdevFormat>(), 88);
        assert_eq!(std::mem::size_of::<MbusCodeEnum>(), 48);
        assert_eq!(std::mem::size_of::<FrameSizeEnum>(), 64);
        assert_eq!(std::mem::size_of::<QueryCtrl>(), 68);
        assert_eq!(std::mem::size_of::<ExtControl>(), 20);
        assert_eq!(std::mem::size_of::<ExtControls>(), 32);
        assert_eq!(std::mem::size_of::<PixMp>(), 192);
        assert_eq!(std::mem::size_of::<FormatMp>(), 208);
        assert_eq!(std::mem::size_of::<FormatSp>(), 208);
        assert_eq!(std::mem::size_of::<Plane>(), 64);
        assert_eq!(fourcc(b"pGAA"), 0x4141_4770);
    }

    #[test]
    fn aspect() {
        assert_eq!(aspect_crop(1632, 1224, 1280, 720), Some((1632, 916)));
        assert_eq!(aspect_crop(1632, 1224, 1632, 1224), None);
        assert_eq!(aspect_crop(1632, 1224, 1000, 1000), Some((1224, 1224)));
    }

    #[test]
    fn bayer_formats() {
        assert_eq!(mbus_bayer(0x300e), Some((Bayer::Gbrg, 10)));
        let f = pixfmts(Bayer::Gbrg, 10);
        assert_eq!(f[0].0, fourcc(b"pGAA"));
        assert!(f[0].1.packed);
    }
}
