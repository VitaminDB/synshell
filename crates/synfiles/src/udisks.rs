//! Съёмные накопители через udisks2 (D-Bus, без sudo — права даёт polkit
//! активному сеансу): что подключено, монтирование по щелчку и безопасное
//! извлечение. Все вызовы блокирующие — не из главного потока, кроме чтения
//! списка в боковой панели (короткий таймаут).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use zbus::blocking::{connection, Connection};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const SERVICE: &str = "org.freedesktop.UDisks2";
const ROOT: &str = "/org/freedesktop/UDisks2";
const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";
const DRIVE: &str = "org.freedesktop.UDisks2.Drive";

type Props = HashMap<String, OwnedValue>;
type Objects = HashMap<OwnedObjectPath, HashMap<String, Props>>;

/// Раздел с файловой системой на съёмном диске.
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
}

fn bus() -> Option<&'static Connection> {
    static BUS: OnceLock<Option<Connection>> = OnceLock::new();
    BUS.get_or_init(|| connection::Builder::system().ok()?.method_timeout(Duration::from_secs(2)).build().ok()).as_ref()
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

/// Все съёмные разделы с файловой системой: смонтированные и нет.
/// Системные (`HintSystem`, `HintIgnore`) и несъёмные диски пропускаются.
pub fn removable() -> Vec<Volume> {
    let Some(objs) = objects() else { return Vec::new() };
    let mut out = Vec::new();
    for (path, ifs) in &objs {
        let (Some(blk), Some(fs)) = (ifs.get(BLOCK), ifs.get(FILESYSTEM)) else { continue };
        if flag(blk, "HintIgnore") || flag(blk, "HintSystem") || text(blk, "IdUsage") != "filesystem" {
            continue;
        }
        let drive = object_path(blk, "Drive");
        let Some(drv) = objs.iter().find(|(p, _)| p.as_str() == drive).and_then(|(_, i)| i.get(DRIVE)) else { continue };
        if !flag(drv, "Removable") && !flag(drv, "MediaRemovable") {
            continue;
        }
        let label = text(blk, "IdLabel");
        let model = format!("{} {}", text(drv, "Vendor"), text(drv, "Model")).trim().to_string();
        let title = if !label.is_empty() {
            label
        } else if !model.is_empty() {
            model
        } else {
            "Съёмный диск".into()
        };
        out.push(Volume { block: path.to_string(), drive, title, size: num(blk, "Size"), mounts: mount_points(fs) });
    }
    out.sort_by(|a, b| a.block.cmp(&b.block));
    out
}

/// Смонтировать раздел (точка выбирается udisks); вернуть её.
pub fn mount(block: &str) -> Result<PathBuf, String> {
    let bus = bus().ok_or("нет системной шины D-Bus")?;
    let m = bus
        .call_method(Some(SERVICE), block, Some(FILESYSTEM), "Mount", &(HashMap::<String, Value>::new(),))
        .map_err(explain)?;
    let mp: String = m.body().deserialize().map_err(|e| e.to_string())?;
    Ok(PathBuf::from(mp))
}

/// Безопасно извлечь диск: отмонтировать все его смонтированные разделы и
/// выключить диск (`PowerOff`), после чего его можно вынимать.
pub fn eject(drive: &str) -> Result<(), String> {
    let bus = bus().ok_or("нет системной шины D-Bus")?;
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
            if n.contains("NotAuthorized") {
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

/// Перестроить боковую панель, когда udisks сообщает об изменениях (вставили,
/// вынули, смонтировали, извлекли): `/proc/self/mounts` при вставке флешки не
/// меняется, поэтому смотрим сигналы udisks.
pub fn watch() {
    std::thread::Builder::new()
        .name("files-udisks".into())
        .spawn(|| {
            let Ok(conn) = Connection::system() else { return };
            let rule = "type='signal',sender='org.freedesktop.UDisks2'";
            let Ok(signals) = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(64)) else { return };
            for msg in signals {
                if msg.is_err() {
                    break;
                }
                syngui::async_runtime::run_on_main_thread(|| {
                    if let Some(ctx) = crate::state::try_ctx() {
                        ctx.places_rev.update(|r| *r += 1);
                    }
                });
            }
        })
        .ok();
}
