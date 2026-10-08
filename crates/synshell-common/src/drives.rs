//! Подключаемые накопители: разделы через udisks2 (системная D-Bus, без sudo —
//! права даёт polkit активному сеансу) и телефоны/камеры через мониторы gvfs
//! (MTP, PTP, iPhone; сеансовая D-Bus, монтирование `gio mount` в
//! `/run/user/UID/gvfs`). Общее для проводника (боковая панель) и оболочки
//! (уведомление о подключении). Кроме съёмных — несмонтированные разделы
//! встроенных дисков с данными (второй диск, раздел NTFS с Windows): для них
//! polkit спросит пароль администратора. Все вызовы блокирующие: из главного
//! потока — только чтение списков (короткий таймаут).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use zbus::blocking::{connection, Connection};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Structure, Value};

const SERVICE: &str = "org.freedesktop.UDisks2";
const ROOT: &str = "/org/freedesktop/UDisks2";
const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";
const DRIVE: &str = "org.freedesktop.UDisks2.Drive";
const PARTITION: &str = "org.freedesktop.UDisks2.Partition";

/// Мониторы gvfs для устройств без блочного накопителя: (имя на шине, вид).
const GVFS_MONITORS: [(&str, Kind); 3] = [
    ("org.gtk.vfs.MTPVolumeMonitor", Kind::Phone),
    ("org.gtk.vfs.AfcVolumeMonitor", Kind::Phone),
    ("org.gtk.vfs.GPhoto2VolumeMonitor", Kind::Camera),
];
const GVFS_PATH: &str = "/org/gtk/Private/RemoteVolumeMonitor";
const GVFS_IFACE: &str = "org.gtk.Private.RemoteVolumeMonitor";

/// Служебные разделы, которые в боковой панели не нужны: GPT-типы (EFI,
/// Microsoft Reserved, среда восстановления Windows, BIOS boot, служебные
/// Lenovo/Dell) и MBR-типы (EFI, восстановление Windows, диагностика).
const SERVICE_PARTS: [&str; 9] = [
    "c12a7328-f81f-11d2-ba4b-00a0c93ec93b",
    "e3c9e316-0b5c-4db8-817d-f92df00215ae",
    "de94bba4-06d1-4d40-a16a-bfd50179d6ac",
    "21686148-6449-6e6f-744e-656564454649",
    "bfbfafe7-a34f-448a-9a5b-6213eb736c22",
    "0xef",
    "0x27",
    "0x12",
    "0x84",
];

type Props = HashMap<String, OwnedValue>;
type Objects = HashMap<OwnedObjectPath, HashMap<String, Props>>;

/// Раздел с файловой системой: на съёмном диске или несмонтированный на
/// встроенном.
#[derive(Debug, Clone, PartialEq)]
pub struct Volume {
    /// Объект блочного устройства раздела — по нему монтируем.
    pub block: String,
    /// Объект диска — по нему извлекаем.
    pub drive: String,
    pub title: String,
    pub size: u64,
    /// Точки монтирования; пусто — раздел не смонтирован.
    pub mounts: Vec<PathBuf>,
    /// Съёмный диск (флешка, карта): можно «Безопасно извлечь».
    pub removable: bool,
}

/// Устройство из монитора gvfs: телефон по MTP, iPhone, камера по PTP.
#[derive(Debug, Clone, PartialEq)]
pub struct Gadget {
    pub title: String,
    /// `mtp://SAMSUNG_SAMSUNG_Android_R5CY…/` — по нему монтируем и отключаем.
    pub uri: String,
    pub kind: Kind,
    /// Папка в `/run/user/UID/gvfs`, если смонтировано.
    pub mount: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Раздел встроенного диска.
    Disk,
    /// Флешка, карта памяти, внешний диск.
    Usb,
    Phone,
    Camera,
}

/// Подключённое устройство с файлами.
#[derive(Debug, Clone, PartialEq)]
pub enum Device {
    Disk(Volume),
    Gadget(Gadget),
}

impl Device {
    /// Неизменный ключ устройства (пока подключено).
    pub fn key(&self) -> &str {
        match self {
            Device::Disk(v) => &v.block,
            Device::Gadget(g) => &g.uri,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            Device::Disk(v) => &v.title,
            Device::Gadget(g) => &g.title,
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Device::Disk(v) if v.removable => Kind::Usb,
            Device::Disk(_) => Kind::Disk,
            Device::Gadget(g) => g.kind,
        }
    }

    /// Съёмное: флешка, телефон, камера (можно извлечь, о подключении сообщают).
    pub fn removable(&self) -> bool {
        self.kind() != Kind::Disk
    }

    /// Где смонтировано; `None` — не смонтировано.
    pub fn mount_point(&self) -> Option<&Path> {
        match self {
            Device::Disk(v) => v.mounts.first().map(PathBuf::as_path),
            Device::Gadget(g) => g.mount.as_deref(),
        }
    }

    pub fn mounted_at(&self, p: &Path) -> bool {
        match self {
            Device::Disk(v) => v.mounts.iter().any(|m| m == p),
            Device::Gadget(g) => g.mount.as_deref() == Some(p),
        }
    }

    /// Смонтировать (или вернуть, где уже смонтировано — в том числе
    /// смонтированное после того, как получено это описание).
    pub fn mount(&self) -> Result<PathBuf, String> {
        if let Some(p) = self.mount_point() {
            return Ok(p.to_path_buf());
        }
        match self {
            Device::Disk(v) => {
                let now = volumes().into_iter().find(|x| x.block == v.block).and_then(|x| x.mounts.into_iter().next());
                now.map(Ok).unwrap_or_else(|| mount(&v.block))
            }
            Device::Gadget(g) => gvfs_dir(&g.uri).map(Ok).unwrap_or_else(|| gio_mount(g)),
        }
    }

    /// Безопасно извлечь: диск — отмонтировать разделы и выключить, телефон —
    /// отключить от gvfs.
    pub fn eject(&self) -> Result<(), String> {
        match self {
            Device::Disk(v) => eject(&v.drive),
            Device::Gadget(g) => gio(&["mount", "-u", &g.uri]).map(|_| ()),
        }
    }

    /// Подпись вида устройства для сообщений.
    pub fn kind_name(&self) -> &'static str {
        match self.kind() {
            Kind::Disk => "Диск",
            Kind::Usb => "Съёмный диск",
            Kind::Phone => "Телефон",
            Kind::Camera => "Камера",
        }
    }
}

/// Все устройства: разделы udisks (см. `volumes`) и телефоны/камеры gvfs.
pub fn devices() -> Vec<Device> {
    let mut out: Vec<Device> = volumes().into_iter().map(Device::Disk).collect();
    out.extend(gadgets().into_iter().map(Device::Gadget));
    out
}

fn bus() -> Option<&'static Connection> {
    static BUS: OnceLock<Option<Connection>> = OnceLock::new();
    BUS.get_or_init(|| connection::Builder::system().ok()?.method_timeout(Duration::from_secs(2)).build().ok()).as_ref()
}

/// Шина для монтирования и извлечения: polkit может спросить пароль
/// (раздел встроенного диска), и вызов ждёт, пока его вводят — двух секунд
/// `bus()` на это не хватит.
fn slow_bus() -> Option<&'static Connection> {
    static BUS: OnceLock<Option<Connection>> = OnceLock::new();
    BUS.get_or_init(|| connection::Builder::system().ok()?.method_timeout(Duration::from_secs(300)).build().ok()).as_ref()
}

fn session_bus() -> Option<&'static Connection> {
    static BUS: OnceLock<Option<Connection>> = OnceLock::new();
    BUS.get_or_init(|| connection::Builder::session().ok()?.method_timeout(Duration::from_secs(2)).build().ok()).as_ref()
}

fn objects() -> Option<Objects> {
    let m = bus()?
        .call_method(Some(SERVICE), ROOT, Some("org.freedesktop.DBus.ObjectManager"), "GetManagedObjects", &())
        .ok()?;
    m.body().deserialize::<Objects>().ok()
}

fn text(p: &Props, k: &str) -> String {
    p.get(k).and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default()
}

fn flag(p: &Props, k: &str) -> bool {
    p.get(k).and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
}

fn num(p: &Props, k: &str) -> u64 {
    p.get(k).and_then(|v| u64::try_from(v).ok()).unwrap_or(0)
}

fn object_path(p: &Props, k: &str) -> String {
    p.get(k).and_then(|v| OwnedObjectPath::try_from(v.try_clone().ok()?).ok()).map(|o| o.to_string()).unwrap_or_default()
}

/// `MountPoints` — массив строк-байтов, каждая с завершающим нулём.
fn mount_points(p: &Props) -> Vec<PathBuf> {
    let Some(v) = p.get("MountPoints") else { return Vec::new() };
    let Some(list) = v.try_clone().ok().and_then(|v| Vec::<Vec<u8>>::try_from(v).ok()) else { return Vec::new() };
    list.iter()
        .map(|b| {
            let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
            PathBuf::from(String::from_utf8_lossy(&b[..end]).into_owned())
        })
        .filter(|p| !p.as_os_str().is_empty())
        .collect()
}

/// Служебный раздел (EFI, MSR, восстановление Windows…) по типу и флагам
/// таблицы разделов.
fn service_partition(part: &Props) -> bool {
    let ty = text(part, "Type").to_ascii_lowercase();
    if SERVICE_PARTS.contains(&ty.as_str()) {
        return true;
    }
    // GPT-атрибуты: бит 0 — «нужен платформе», бит 62 — «скрытый» (так
    // помечены разделы восстановления производителей). У MBR во флагах
    // только «загрузочный» (0x80) — его не смотрим.
    let gpt = !ty.starts_with("0x");
    gpt && num(part, "Flags") & (1 | 1 << 62) != 0
}

/// «1,5 ГБ».
pub fn format_size(n: u64) -> String {
    const U: [&str; 6] = ["Б", "КБ", "МБ", "ГБ", "ТБ", "ПБ"];
    if n < 1024 {
        return format!("{n} Б");
    }
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    let s = if v < 10.0 { format!("{v:.1}") } else { format!("{v:.0}") };
    format!("{} {}", s.replace('.', ","), U[i])
}

/// Разделы с файловой системой для боковой панели: на съёмных дисках — все
/// (смонтированные и нет), на встроенных — только несмонтированные с
/// данными; смонтированные встроенные (корень, /home, /boot) показывает
/// сама панель по `/proc/self/mounts`. Пропускаются `HintIgnore` (udev
/// просит не показывать) и служебные разделы. `HintSystem` не помеха: udisks
/// ставит его всем разделам несъёмных дисков, в том числе диску с данными.
pub fn volumes() -> Vec<Volume> {
    let Some(objs) = objects() else { return Vec::new() };
    let mut out = Vec::new();
    for (path, ifs) in &objs {
        let (Some(blk), Some(fs)) = (ifs.get(BLOCK), ifs.get(FILESYSTEM)) else { continue };
        if flag(blk, "HintIgnore") || text(blk, "IdUsage") != "filesystem" {
            continue;
        }
        if ifs.get(PARTITION).is_some_and(service_partition) {
            continue;
        }
        let drive = object_path(blk, "Drive");
        let Some(drv) = objs.iter().find(|(p, _)| p.as_str() == drive).and_then(|(_, i)| i.get(DRIVE)) else { continue };
        let removable = flag(drv, "Removable") || flag(drv, "MediaRemovable");
        let mounts = mount_points(fs);
        if !removable && !mounts.is_empty() {
            continue;
        }
        let label = text(blk, "IdLabel");
        let model = format!("{} {}", text(drv, "Vendor"), text(drv, "Model")).trim().to_string();
        let title = if !label.is_empty() {
            label
        } else if removable && !model.is_empty() {
            model
        } else if removable {
            "Съёмный диск".into()
        } else {
            format!("Локальный диск {}", format_size(num(blk, "Size")))
        };
        out.push(Volume { block: path.to_string(), drive, title, size: num(blk, "Size"), mounts, removable });
    }
    out.sort_by(|a, b| a.block.cmp(&b.block));
    out
}

/// Смонтировать раздел (точка выбирается udisks); вернуть её.
pub fn mount(block: &str) -> Result<PathBuf, String> {
    let bus = slow_bus().ok_or("нет системной шины D-Bus")?;
    let m = bus
        .call_method(Some(SERVICE), block, Some(FILESYSTEM), "Mount", &(HashMap::<String, Value>::new(),))
        .map_err(explain)?;
    let mp: String = m.body().deserialize().map_err(|e| e.to_string())?;
    Ok(PathBuf::from(mp))
}

/// Безопасно извлечь диск: отмонтировать все его смонтированные разделы и
/// выключить диск (`PowerOff`), после чего его можно вынимать.
pub fn eject(drive: &str) -> Result<(), String> {
    let bus = slow_bus().ok_or("нет системной шины D-Bus")?;
    let objs = objects().ok_or("udisks2 не отвечает")?;
    for (path, ifs) in &objs {
        let Some(blk) = ifs.get(BLOCK) else { continue };
        if object_path(blk, "Drive") != drive {
            continue;
        }
        let mounted = ifs.get(FILESYSTEM).is_some_and(|fs| !mount_points(fs).is_empty());
        if mounted {
            bus.call_method(Some(SERVICE), path.as_str(), Some(FILESYSTEM), "Unmount", &(HashMap::<String, Value>::new(),))
                .map_err(explain)?;
        }
    }
    bus.call_method(Some(SERVICE), drive, Some(DRIVE), "PowerOff", &(HashMap::<String, Value>::new(),)).map_err(explain)?;
    Ok(())
}

/// Понятный текст ошибки udisks (вместо имени ошибки D-Bus).
fn explain(e: zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(name, msg, _) => {
            let n = name.as_str();
            if n.ends_with("NotAuthorizedDismissed") {
                "пароль не введён".into()
            } else if n.contains("NotAuthorized") {
                "нет прав на это действие (polkit)".into()
            } else if n.ends_with("DeviceBusy") {
                "устройство занято: закройте программы, открытые с него".into()
            } else {
                match msg {
                    Some(m) if !m.is_empty() => m,
                    _ => n.to_string(),
                }
            }
        }
        e => e.to_string(),
    }
}

// ─── gvfs: телефоны и камеры ────────────────────────────────────────────────

fn gvfs_root() -> PathBuf {
    PathBuf::from(format!("/run/user/{}/gvfs", unsafe { libc::getuid() }))
}

/// Телефоны и камеры из мониторов gvfs. Монитор запускается шиной по первому
/// вызову; без gvfs — пусто.
pub fn gadgets() -> Vec<Gadget> {
    let Some(bus) = session_bus() else { return Vec::new() };
    let mut out = Vec::new();
    for (name, kind) in GVFS_MONITORS {
        let Ok(m) = bus.call_method(Some(name), GVFS_PATH, Some(GVFS_IFACE), "List", &()) else { continue };
        let body = m.body();
        // В кортеже диска 17 полей — больше, чем умеет serde; разбираем как Value.
        let Ok(s) = body.deserialize::<Structure>() else { continue };
        let Some(Value::Array(vols)) = s.fields().get(1) else { continue };
        for v in vols.iter() {
            let Value::Structure(v) = v else { continue };
            let f = v.fields();
            let str_at = |i: usize| match f.get(i) {
                Some(Value::Str(s)) => s.to_string(),
                _ => String::new(),
            };
            let (title, uri) = (str_at(1), str_at(5));
            if uri.is_empty() {
                continue;
            }
            let mount = gvfs_dir(&uri).filter(|p| p.is_dir());
            out.push(Gadget { title: if title.is_empty() { uri.clone() } else { title }, uri, kind, mount });
        }
    }
    out
}

/// Папка gvfs для адреса: `mtp://HOST/` → `/run/user/UID/gvfs/mtp:host=HOST`.
/// Имена папок экранированы как URI — сравниваем раскодированные.
fn gvfs_dir(uri: &str) -> Option<PathBuf> {
    let (scheme, rest) = uri.split_once("://")?;
    let host = unescape(rest.split('/').next()?);
    let want = format!("{scheme}:host={host}");
    let root = gvfs_root();
    let rd = std::fs::read_dir(&root).ok()?;
    rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).find(|n| unescape(n) == want).map(|n| root.join(n))
}

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(x) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(x);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn gio(args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("gio")
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("gio: {e} (нужен пакет glib2 и gvfs-mtp)"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let err = err.lines().last().unwrap_or("").trim();
    // «gio: mtp://…/: Unable to open MTP device …» — без адреса.
    let err = err.rsplit_once(": ").map(|(_, e)| e).unwrap_or(err);
    Err(if err.is_empty() { format!("gio завершился с ошибкой {}", out.status) } else { err.to_string() })
}

/// Смонтировать телефон или камеру через gvfs. Телефон, пока экран
/// заблокирован или не разрешён доступ к файлам, не открывается — тогда об
/// этом и сообщаем.
fn gio_mount(g: &Gadget) -> Result<PathBuf, String> {
    gio(&["mount", &g.uri]).map_err(|e| {
        if g.kind == Kind::Phone {
            format!("{e}. Разблокируйте телефон и разрешите доступ к файлам (режим «Передача файлов»)")
        } else {
            e
        }
    })?;
    gvfs_dir(&g.uri).ok_or_else(|| "gvfs смонтировал устройство, но его папки нет".into())
}

// ─── Слежение ───────────────────────────────────────────────────────────────

/// Звать `changed` (в фоновом потоке), когда подключили, вынули,
/// смонтировали или извлекли устройство: сигналы udisks и мониторов gvfs,
/// пачка сигналов за 300 мс — один вызов.
pub fn watch(changed: impl Fn() + Send + 'static) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let udisks_tx = tx.clone();
    std::thread::Builder::new()
        .name("drives-udisks".into())
        .spawn(move || {
            let Ok(conn) = Connection::system() else { return };
            let rule = "type='signal',sender='org.freedesktop.UDisks2'";
            let Ok(signals) = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(64)) else { return };
            for msg in signals {
                if msg.is_err() || udisks_tx.send(()).is_err() {
                    break;
                }
            }
        })
        .ok();
    std::thread::Builder::new()
        .name("drives-gvfs".into())
        .spawn(move || {
            let Ok(conn) = Connection::session() else { return };
            // Мониторы запускаются шиной по вызову — иначе сигналов не будет.
            for (name, _) in GVFS_MONITORS {
                let _ = conn.call_method(Some(name), GVFS_PATH, Some(GVFS_IFACE), "IsSupported", &());
            }
            let rule = format!("type='signal',interface='{GVFS_IFACE}'");
            let Ok(signals) = zbus::blocking::MessageIterator::for_match_rule(rule.as_str(), &conn, Some(64)) else { return };
            for msg in signals {
                let Ok(msg) = msg else { break };
                // Свои сигналы шлют и мониторы udisks2/GOA — их хватает от udisks.
                let from: Option<String> = msg.body().deserialize::<Structure>().ok().and_then(|s| match s.fields().first() {
                    Some(Value::Str(s)) => Some(s.to_string()),
                    _ => None,
                });
                if from.is_some_and(|f| !GVFS_MONITORS.iter().any(|(n, _)| *n == f)) {
                    continue;
                }
                if tx.send(()).is_err() {
                    break;
                }
            }
        })
        .ok();
    std::thread::Builder::new()
        .name("drives-watch".into())
        .spawn(move || {
            while rx.recv().is_ok() {
                std::thread::sleep(Duration::from_millis(300));
                while rx.try_recv().is_ok() {}
                changed();
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    #[test]
    fn unescape() {
        assert_eq!(super::unescape("gphoto2:host=usb%3A003%2C028"), "gphoto2:host=usb:003,028");
        assert_eq!(super::unescape("mtp:host=SAMSUNG_X"), "mtp:host=SAMSUNG_X");
        assert_eq!(super::unescape("100%"), "100%");
    }
}
