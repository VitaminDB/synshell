//! Дерево элементов программ через AT-SPI (шина доступности): GTK, Qt,
//! Firefox, Chromium, LibreOffice и всё, что пишет для экранного диктора.
//! Программа находится по pid окна. Qt и браузеры открывают дерево, только
//! когда доступность включена (`org.a11y.Status.IsEnabled`) — включаем при
//! первом запросе; уже запущенным программам может понадобиться перезапуск.

use anyhow::{bail, Context, Result};
use synshell_tr::t;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::uitree::{Node, Tree};

const ACC: &str = "org.a11y.atspi.Accessible";
const COMP: &str = "org.a11y.atspi.Component";
const MAX_NODES: usize = 3000;

fn a11y_bus() -> Result<Connection> {
    let session = Connection::session().context(t!("нет сеансовой D-Bus"))?;
    // Включить доступность: Qt и браузеры иначе не отдают дерево.
    let _ = session.call_method(
        Some("org.a11y.Bus"),
        "/org/a11y/bus",
        Some("org.freedesktop.DBus.Properties"),
        "Set",
        &("org.a11y.Status", "IsEnabled", zbus::zvariant::Value::from(true)),
    );
    let reply = session
        .call_method(Some("org.a11y.Bus"), "/org/a11y/bus", Some("org.a11y.Bus"), "GetAddress", &())
        .context(t!("нет шины доступности (at-spi2-core)"))?;
    let addr: String = reply.body().deserialize()?;
    Ok(zbus::blocking::connection::Builder::address(addr.as_str())?.build()?)
}

fn children(c: &Connection, dest: &str, path: &str) -> Vec<(String, OwnedObjectPath)> {
    c.call_method(Some(dest), path, Some(ACC), "GetChildren", &())
        .ok()
        .and_then(|m| m.body().deserialize::<Vec<(String, OwnedObjectPath)>>().ok())
        .unwrap_or_default()
}

fn prop_str(c: &Connection, dest: &str, path: &str, name: &str) -> String {
    c.call_method(Some(dest), path, Some("org.freedesktop.DBus.Properties"), "Get", &(ACC, name))
        .ok()
        .and_then(|m| m.body().deserialize::<OwnedValue>().ok())
        .and_then(|v| String::try_from(v).ok())
        .unwrap_or_default()
}

fn call<T: serde::de::DeserializeOwned + zbus::zvariant::Type>(c: &Connection, dest: &str, path: &str, iface: &str, method: &str, body: &(impl serde::Serialize + zbus::zvariant::DynamicType)) -> Option<T> {
    c.call_method(Some(dest), path, Some(iface), method, body).ok().and_then(|m| m.body().deserialize::<T>().ok())
}

fn bit(states: &[u32], n: u32) -> bool {
    states.get((n / 32) as usize).is_some_and(|w| w & (1 << (n % 32)) != 0)
}

/// Дерево окон процесса `pid`.
pub fn tree(pid: i32) -> Result<Tree> {
    let c = a11y_bus()?;
    let apps = children(&c, "org.a11y.atspi.Registry", "/org/a11y/atspi/accessible/root");
    let dbus = zbus::blocking::fdo::DBusProxy::new(&c)?;
    let app = apps
        .iter()
        .find(|(bus, _)| {
            zbus::names::BusName::try_from(bus.as_str())
                .ok()
                .and_then(|b| dbus.get_connection_unix_process_id(b).ok())
                .is_some_and(|p| p as i32 == pid)
        })
        .with_context(|| t!("процесс {pid} не на шине доступности (программа без AT-SPI или запущена до включения доступности)", pid = pid))?;
    let (dest, root) = (app.0.clone(), app.1.to_string());
    let mut nodes = Vec::new();
    // Окна программы (frame/window) — верхний уровень; обходим только показанные.
    let mut stack: Vec<(String, u32)> = children(&c, &dest, &root).into_iter().rev().map(|(_, p)| (p.to_string(), 0)).collect();
    while let Some((path, depth)) = stack.pop() {
        if nodes.len() >= MAX_NODES {
            break;
        }
        let states: Vec<u32> = call(&c, &dest, &path, ACC, "GetState", &()).unwrap_or_default();
        const SHOWING: u32 = 25;
        if !states.is_empty() && !bit(&states, SHOWING) {
            continue;
        }
        let role: String = call(&c, &dest, &path, ACC, "GetRoleName", &()).unwrap_or_default();
        // Координаты относительно окна (1); на Wayland «экранных» нет.
        let ext: Option<(i32, i32, i32, i32)> = call(&c, &dest, &path, COMP, "GetExtents", &(1u32,));
        let name = prop_str(&c, &dest, &path, "Name");
        let desc = prop_str(&c, &dest, &path, "Description");
        let mut st = Vec::new();
        for (n, f) in [(4u32, "checked"), (12, "focused"), (20, "pressed"), (23, "selected"), (10, "expanded")] {
            if bit(&states, n) {
                st.push(f);
            }
        }
        if !states.is_empty() && !bit(&states, 24) {
            st.push("disabled");
        }
        let id = nodes.len() as u64 + 1;
        nodes.push(Node {
            id,
            depth,
            role,
            bounds: ext.map(|(x, y, w, h)| [x as f64, y as f64, w as f64, h as f64]).unwrap_or_default(),
            label: (!name.is_empty()).then_some(name),
            description: (!desc.is_empty()).then_some(desc),
            state: (!st.is_empty()).then(|| st.join(" ")),
            ..Default::default()
        });
        for (_, p) in children(&c, &dest, &path).into_iter().rev() {
            stack.push((p.to_string(), depth + 1));
        }
    }
    if nodes.is_empty() {
        bail!("{}", t!("у процесса {pid} нет показанных элементов в AT-SPI", pid = pid));
    }
    Ok(Tree { source: "atspi".into(), nodes })
}
