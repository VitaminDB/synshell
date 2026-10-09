//! Bluetooth через bluez (`org.bluez` на системной шине): адаптер
//! (включить, поиск, видимость), устройства (сопряжение, подключение,
//! отключение, забыть). Для сопряжения регистрируется свой агент
//! «NoInputNoOutput» — наушники, колонки и клавиатуры без PIN сопрягаются
//! сами; устройства, требующие ввода PIN, — через `bluetoothctl`.
//!
//! Вызовы блокирующие — из фонового потока.
//!
//! Соединение с шиной одно на процесс: bluez привязывает поиск устройств и
//! агента к клиенту и снимает их, когда его соединение закрывается, — с
//! соединением на каждый вызов поиск останавливался сразу после включения.

use std::collections::HashMap;
use std::sync::Mutex;

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use synshell_tr::t;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Device {
    pub path: String,
    pub address: String,
    pub name: String,
    /// Значок freedesktop (`audio-headphones`, `input-keyboard`…).
    pub icon: String,
    pub paired: bool,
    pub connected: bool,
    pub trusted: bool,
    pub rssi: Option<i16>,
    pub battery: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BtState {
    pub present: bool,
    pub adapter: Option<String>,
    pub powered: bool,
    pub discovering: bool,
    pub discoverable: bool,
    pub devices: Vec<Device>,
}

type Objects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

pub struct Bluetooth {
    conn: Connection,
}

fn s(p: &HashMap<String, OwnedValue>, k: &str) -> Option<String> {
    p.get(k).and_then(|v| String::try_from(v.try_clone().ok()?).ok())
}

fn b(p: &HashMap<String, OwnedValue>, k: &str) -> bool {
    p.get(k).and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
}

/// Агент bluez без ввода-вывода: подтверждает сопряжение.
struct Agent;

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    fn release(&self) {}
    fn request_confirmation(&self, _device: OwnedObjectPath, _passkey: u32) {}
    fn request_authorization(&self, _device: OwnedObjectPath) {}
    fn authorize_service(&self, _device: OwnedObjectPath, _uuid: String) {}
    fn cancel(&self) {}
}

const AGENT_PATH: &str = "/org/synshell/bt_agent";

static CONN: Mutex<Option<Connection>> = Mutex::new(None);

/// Общее соединение процесса с системной шиной (zbus `Connection` — разделяемая ссылка).
fn shared_conn() -> Option<Connection> {
    let mut g = CONN.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(Connection::system().ok()?);
    }
    g.clone()
}

impl Bluetooth {
    pub fn new() -> Option<Self> {
        let conn = shared_conn()?;
        crate::wifi::has_name(&conn, "org.bluez").then_some(Self { conn })
    }

    fn objects(&self) -> Result<Objects, String> {
        let m = self
            .conn
            .call_method(Some("org.bluez"), "/", Some("org.freedesktop.DBus.ObjectManager"), "GetManagedObjects", &())
            .map_err(|e| e.to_string())?;
        m.body().deserialize::<Objects>().map_err(|e| e.to_string())
    }

    fn adapter(&self, objs: &Objects) -> Option<String> {
        objs.iter().find(|(_, i)| i.contains_key("org.bluez.Adapter1")).map(|(p, _)| p.to_string())
    }

    pub fn state(&self) -> Result<BtState, String> {
        let objs = self.objects()?;
        let mut st = BtState::default();
        let Some(ap) = self.adapter(&objs) else { return Ok(st) };
        st.present = true;
        let a = &objs.iter().find(|(p, _)| p.as_str() == ap).unwrap().1["org.bluez.Adapter1"];
        st.adapter = s(a, "Alias").or_else(|| s(a, "Name"));
        st.powered = b(a, "Powered");
        st.discovering = b(a, "Discovering");
        st.discoverable = b(a, "Discoverable");
        for (path, ifs) in &objs {
            let Some(d) = ifs.get("org.bluez.Device1") else { continue };
            let battery = ifs.get("org.bluez.Battery1").and_then(|x| x.get("Percentage")).and_then(|v| u8::try_from(v).ok());
            st.devices.push(Device {
                path: path.to_string(),
                address: s(d, "Address").unwrap_or_default(),
                name: s(d, "Alias").or_else(|| s(d, "Name")).unwrap_or_else(|| s(d, "Address").unwrap_or_default()),
                icon: s(d, "Icon").unwrap_or_else(|| "bluetooth".into()),
                paired: b(d, "Paired"),
                connected: b(d, "Connected"),
                trusted: b(d, "Trusted"),
                rssi: d.get("RSSI").and_then(|v| i16::try_from(v).ok()),
                battery,
            });
        }
        st.devices.sort_by(|x, y| y.connected.cmp(&x.connected).then(y.paired.cmp(&x.paired)).then(y.rssi.cmp(&x.rssi)).then(x.name.cmp(&y.name)));
        Ok(st)
    }

    fn set_adapter(&self, prop: &str, v: bool) -> Result<(), String> {
        let objs = self.objects()?;
        let ap = self.adapter(&objs).ok_or(t!("нет адаптера Bluetooth"))?;
        self.conn
            .call_method(Some("org.bluez"), ap.as_str(), Some("org.freedesktop.DBus.Properties"), "Set", &("org.bluez.Adapter1", prop, Value::from(v)))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn set_powered(&self, on: bool) -> Result<(), String> {
        // Заблокирован rfkill — снять.
        if on {
            let _ = std::process::Command::new("rfkill").args(["unblock", "bluetooth"]).status();
        }
        self.set_adapter("Powered", on)
    }

    pub fn set_discoverable(&self, on: bool) -> Result<(), String> {
        // видимым нас могут начать сопрягать с другого устройства — агент нужен заранее
        if on {
            self.ensure_agent();
        }
        self.set_adapter("Discoverable", on)
    }

    pub fn discovery(&self, on: bool) -> Result<(), String> {
        let objs = self.objects()?;
        let ap = self.adapter(&objs).ok_or(t!("нет адаптера Bluetooth"))?;
        if on {
            self.ensure_agent();
        }
        let m = if on { "StartDiscovery" } else { "StopDiscovery" };
        match self.conn.call_method(Some("org.bluez"), ap.as_str(), Some("org.bluez.Adapter1"), m, &()) {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("InProgress") || e.to_string().contains("NotReady") => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn device_call(&self, path: &str, method: &str) -> Result<(), String> {
        self.conn.call_method(Some("org.bluez"), path, Some("org.bluez.Device1"), method, &()).map(|_| ()).map_err(|e| e.to_string())
    }

    /// Сопрячь, доверять и подключить.
    pub fn pair(&self, path: &str) -> Result<(), String> {
        self.ensure_agent();
        match self.device_call(path, "Pair") {
            Ok(()) => {}
            Err(e) if e.contains("AlreadyExists") => {}
            Err(e) => return Err(e),
        }
        let _ = self.conn.call_method(Some("org.bluez"), path, Some("org.freedesktop.DBus.Properties"), "Set", &("org.bluez.Device1", "Trusted", Value::from(true)));
        self.device_call(path, "Connect")
    }

    pub fn connect(&self, path: &str) -> Result<(), String> {
        self.device_call(path, "Connect")
    }

    pub fn disconnect(&self, path: &str) -> Result<(), String> {
        self.device_call(path, "Disconnect")
    }

    pub fn forget(&self, path: &str) -> Result<(), String> {
        let objs = self.objects()?;
        let ap = self.adapter(&objs).ok_or(t!("нет адаптера Bluetooth"))?;
        let dev = OwnedObjectPath::try_from(path.to_string()).map_err(|e| e.to_string())?;
        self.conn.call_method(Some("org.bluez"), ap.as_str(), Some("org.bluez.Adapter1"), "RemoveDevice", &(dev,)).map(|_| ()).map_err(|e| e.to_string())
    }

    /// Зарегистрировать агента. Объект на соединении — один раз; регистрация в bluez — при каждом
    /// сопряжении (после перезапуска bluetoothd прежняя теряется; повтор даёт AlreadyExists — не ошибка).
    fn ensure_agent(&self) {
        let _ = self.conn.object_server().at(AGENT_PATH, Agent);
        let path = OwnedObjectPath::try_from(AGENT_PATH.to_string()).unwrap();
        let _ = self.conn.call_method(Some("org.bluez"), "/org/bluez", Some("org.bluez.AgentManager1"), "RegisterAgent", &(path.clone(), "NoInputNoOutput"));
        let _ = self.conn.call_method(Some("org.bluez"), "/org/bluez", Some("org.bluez.AgentManager1"), "RequestDefaultAgent", &(path,));
    }
}
