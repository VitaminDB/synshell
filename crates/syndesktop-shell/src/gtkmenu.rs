//! Меню GTK-программ: модель `org.gtk.Menus` (GMenuModel по D-Bus) и
//! действия `org.gtk.Actions` — в пункты [`MenuEntry`] глобального меню.
//!
//! Модель — группы меню: `Start([группы])` отдаёт их пункты и подписывает на
//! сигнал `Changed`; пункты ссылаются на разделы (`:section`) и подменю
//! (`:submenu`) в других группах. Разделы разделяются чертой. Пункт вызывает
//! действие `app.имя` (по пути приложения) или `win.имя` (по пути окна).

use std::collections::HashMap;
use syndesktop_common::ipc::GtkMenu;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Signature, Value};

use crate::tray::sni::{MenuEntry, TrayIcon};

const MENUS: &str = "org.gtk.Menus";
const ACTIONS: &str = "org.gtk.Actions";

type Item = HashMap<String, OwnedValue>;

/// Загруженное меню GTK: что подписано и какому пункту какое действие.
pub struct GtkState {
    pub menu: GtkMenu,
    groups: Vec<u32>,
    menus: HashMap<(u32, u32), Vec<Item>>,
    /// id пункта → (префикс `app`/`win`, имя действия, параметр).
    actions: HashMap<i32, (String, String, Option<OwnedValue>)>,
}

/// Состояние действий: доступно ли и текущее значение.
type ActionStates = HashMap<String, (bool, Option<OwnedValue>)>;

fn describe_all(conn: &Connection, bus: &str, path: &str) -> ActionStates {
    if path.is_empty() {
        return HashMap::new();
    }
    let reply = match conn.call_method(Some(bus), path, Some(ACTIONS), "DescribeAll", &()) {
        Ok(r) => r,
        Err(e) => {
            log::debug!("меню GTK: DescribeAll {bus}{path}: {e}");
            return HashMap::new();
        }
    };
    let all: HashMap<String, (bool, Signature, Vec<OwnedValue>)> = reply.body().deserialize().unwrap_or_default();
    all.into_iter().map(|(k, (enabled, _, state))| (k, (enabled, state.into_iter().next()))).collect()
}

fn start(conn: &Connection, bus: &str, path: &str, groups: &[u32]) -> Vec<(u32, u32, Vec<Item>)> {
    match conn.call_method(Some(bus), path, Some(MENUS), "Start", &(groups.to_vec(),)) {
        Ok(r) => r.body().deserialize().unwrap_or_default(),
        Err(e) => {
            log::debug!("меню GTK: Start {bus}{path}: {e}");
            Vec::new()
        }
    }
}

fn str_of(item: &Item, key: &str) -> Option<String> {
    item.get(key).and_then(|v| v.try_clone().ok()).and_then(|v| String::try_from(v).ok())
}

fn link_of(item: &Item, key: &str) -> Option<(u32, u32)> {
    item.get(key).and_then(|v| v.try_clone().ok()).and_then(|v| <(u32, u32)>::try_from(v).ok())
}

/// «<Primary><Shift>n» → «Ctrl+Shift+N».
fn accel_label(accel: &str) -> String {
    let mut parts = Vec::new();
    let mut rest = accel;
    while let Some(s) = rest.strip_prefix('<') {
        let Some(end) = s.find('>') else { break };
        parts.push(match &s[..end] {
            "Primary" | "Control" | "Ctrl" => "Ctrl".to_string(),
            "Shift" => "Shift".to_string(),
            "Alt" | "Mod1" => "Alt".to_string(),
            "Super" => "Super".to_string(),
            other => other.to_string(),
        });
        rest = &s[end + 1..];
    }
    if !rest.is_empty() {
        let key = if rest.chars().count() == 1 { rest.to_uppercase() } else { rest.to_string() };
        parts.push(key);
    }
    parts.join("+")
}

/// «_Файл» → «Файл», «__» → «_».
fn strip_mnemonic(s: &str) -> String {
    s.replace("__", "\u{1}").replace('_', "").replace('\u{1}', "_")
}

struct Builder<'a> {
    menus: &'a HashMap<(u32, u32), Vec<Item>>,
    states: &'a HashMap<&'static str, ActionStates>,
    next_id: i32,
    actions: HashMap<i32, (String, String, Option<OwnedValue>)>,
}

impl Builder<'_> {
    fn id(&mut self) -> i32 {
        self.next_id += 1;
        self.next_id
    }

    /// Пункты меню (группа, меню); разделы — через черту.
    fn build(&mut self, key: (u32, u32), depth: u32) -> Vec<MenuEntry> {
        let Some(items) = self.menus.get(&key) else { return Vec::new() };
        if depth > 16 {
            return Vec::new();
        }
        // Идущие подряд пункты вне разделов — одна «секция».
        let mut sections: Vec<Vec<MenuEntry>> = vec![Vec::new()];
        for item in items {
            if let Some(sec) = link_of(item, ":section") {
                let entries = self.build(sec, depth + 1);
                sections.push(entries);
                sections.push(Vec::new());
                continue;
            }
            if let Some(e) = self.entry(item, depth) {
                sections.last_mut().unwrap().push(e);
            }
        }
        let mut out: Vec<MenuEntry> = Vec::new();
        for s in sections.into_iter().filter(|s| !s.is_empty()) {
            if !out.is_empty() {
                out.push(MenuEntry { id: self.id(), visible: true, separator: true, ..Default::default() });
            }
            out.extend(s);
        }
        out
    }

    fn entry(&mut self, item: &Item, depth: u32) -> Option<MenuEntry> {
        let label = strip_mnemonic(&str_of(item, "label").unwrap_or_default());
        let id = self.id();
        if let Some(sub) = link_of(item, ":submenu") {
            let children = self.build(sub, depth + 1);
            return Some(MenuEntry { id, label, enabled: true, visible: true, submenu: true, children, ..Default::default() });
        }
        let action = str_of(item, "action")?;
        let (prefix, name) = action.split_once('.').map(|(p, n)| (p.to_string(), n.to_string()))?;
        let target = item.get("target").and_then(|v| v.try_clone().ok());
        let state = self.states.get(prefix.as_str()).and_then(|m| m.get(&name));
        match str_of(item, "hidden-when").as_deref() {
            Some("action-missing") if state.is_none() => return None,
            Some("action-disabled") if !state.is_some_and(|s| s.0) => return None,
            _ => {}
        }
        let enabled = state.map(|s| s.0).unwrap_or(true);
        // Флажок — действие с логическим состоянием, радиокнопка — со
        // строковым и параметром пункта.
        let toggle = state.and_then(|(_, st)| st.as_ref()).and_then(|st| {
            if let Ok(b) = bool::try_from(st.try_clone().ok()?) {
                return Some((false, b));
            }
            let t = target.as_ref()?;
            Some((true, **st == **t))
        });
        let shortcut = str_of(item, "accel").map(|a| accel_label(&a)).unwrap_or_default();
        self.actions.insert(id, (prefix, name, target));
        Some(MenuEntry {
            id,
            label,
            enabled,
            visible: true,
            toggle,
            icon: TrayIcon::None,
            shortcut,
            ..Default::default()
        })
    }
}

/// Загрузить меню целиком: все группы, на которые ссылаются разделы и
/// подменю, и состояние действий.
pub fn load(conn: &Connection, menu: &GtkMenu) -> (Vec<MenuEntry>, GtkState) {
    let mut menus: HashMap<(u32, u32), Vec<Item>> = HashMap::new();
    let mut started: Vec<u32> = Vec::new();
    let mut want = vec![0u32];
    while !want.is_empty() && started.len() < 256 {
        let batch: Vec<u32> = want.drain(..).filter(|g| !started.contains(g)).collect();
        if batch.is_empty() {
            break;
        }
        started.extend(&batch);
        for (g, m, items) in start(conn, &menu.bus, &menu.menubar, &batch) {
            for it in &items {
                for key in [":section", ":submenu"] {
                    if let Some((lg, _)) = link_of(it, key) {
                        if !started.contains(&lg) && !want.contains(&lg) {
                            want.push(lg);
                        }
                    }
                }
            }
            menus.insert((g, m), items);
        }
    }
    let mut st = GtkState { menu: menu.clone(), groups: started, menus, actions: HashMap::new() };
    let tree = refresh(conn, &mut st);
    (tree, st)
}

/// Перечитать состояние действий и пересобрать пункты (модель та же).
pub fn refresh(conn: &Connection, st: &mut GtkState) -> Vec<MenuEntry> {
    let mut states = HashMap::new();
    states.insert("app", describe_all(conn, &st.menu.bus, &st.menu.app_path));
    states.insert("win", describe_all(conn, &st.menu.bus, &st.menu.window_path));
    let mut b = Builder { menus: &st.menus, states: &states, next_id: 0, actions: HashMap::new() };
    // Строка меню: пункты верхнего уровня без черт между разделами.
    let tree: Vec<MenuEntry> = b.build((0, 0), 0).into_iter().filter(|e| !e.separator).collect();
    st.actions = b.actions;
    tree
}

/// Отписаться от групп меню.
pub fn end(conn: &Connection, st: &GtkState) {
    let _ = conn.call_method(Some(st.menu.bus.as_str()), st.menu.menubar.as_str(), Some(MENUS), "End", &(st.groups.clone(),));
}

/// Выполнить действие пункта `id`.
pub fn activate(conn: &Connection, st: &GtkState, id: i32) {
    let Some((prefix, name, target)) = st.actions.get(&id) else { return };
    let path = match prefix.as_str() {
        "app" => &st.menu.app_path,
        "win" => &st.menu.window_path,
        _ => return,
    };
    if path.is_empty() {
        return;
    }
    let param: Vec<Value> = target.iter().filter_map(|t| t.try_clone().ok()).map(Value::from).collect();
    let platform: HashMap<&str, Value> = HashMap::new();
    if let Err(e) = conn.call_method(Some(st.menu.bus.as_str()), path.as_str(), Some(ACTIONS), "Activate", &(name.as_str(), param, platform)) {
        log::debug!("меню GTK: Activate {prefix}.{name}: {e}");
    }
}
