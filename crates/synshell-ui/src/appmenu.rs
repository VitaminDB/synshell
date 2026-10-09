//! Глобальное меню: меню активного окна по D-Bus (`com.canonical.dbusmenu`).
//!
//! Адрес меню окно сообщает композитору по `org_kde_kwin_appmenu` (Qt/KDE
//! на Wayland, приходит в `WindowInfo::appmenu`), по `gtk_shell1` (GTK3,
//! `WindowInfo::gtk_menu`, меню `org.gtk.Menus` — см. `crate::gtkmenu`) или
//! регистрирует у `com.canonical.AppMenu.Registrar` по идентификатору окна
//! X11 (XWayland).
//! Реестр оболочка держит, только пока на какой-нибудь панели есть апплет
//! `appmenu`: по его наличию на шине Qt решает, убирать ли строку меню из
//! окна — без апплета меню пропало бы совсем.
//!
//! Потоки как у лотка: «appmenu-bus» слушает сигналы, «appmenu-work»
//! выполняет запросы (зависшая программа не держит интерфейс).

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use synshell_common::ipc::{GtkMenu, WindowInfo};
use syngui::prelude::*;
use zbus::blocking::{Connection, MessageIterator};
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};
use zbus::MatchRule;

use crate::ctx::ShellCtx;
use crate::gtkmenu::{self, GtkState};
use crate::tray::sni::{entry_from, owner_of, MenuEntry, RawEntry};

const REGISTRAR: &str = "com.canonical.AppMenu.Registrar";
const REGISTRAR_PATH: &str = "/com/canonical/AppMenu/Registrar";
const MENU_IFACE: &str = "com.canonical.dbusmenu";

/// Загруженное меню: адрес (`служба|путь`) и пункты верхнего уровня (с
/// подгруженными подменю).
#[derive(Debug, Clone, PartialEq)]
pub struct AppMenu {
    pub key: String,
    pub entries: Vec<MenuEntry>,
}

/// Где меню окна.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuAddr {
    /// `com.canonical.dbusmenu`: служба и путь.
    DbusMenu(String, String),
    /// `org.gtk.Menus` + `org.gtk.Actions`.
    Gtk(GtkMenu),
}

impl MenuAddr {
    pub fn key(&self) -> String {
        match self {
            MenuAddr::DbusMenu(s, p) => format!("{s}|{p}"),
            MenuAddr::Gtk(g) => format!("gtk:{}|{}", g.bus, g.menubar),
        }
    }
}

enum Cmd {
    /// Меню активного окна (None — окна нет или у него нет меню).
    Watch(Option<MenuAddr>),
    /// Подгрузить подменю `id` (AboutToShow + GetLayout).
    Expand(i32),
    /// Выбран пункт.
    Activate(i32),
    /// Подменю верхнего уровня закрыто.
    Closed(i32),
    /// Меню у владельца `sender` изменилось.
    Reload(String),
    /// У владельца `sender` изменились действия GTK (доступность, флажки).
    Refresh(String),
    /// Держать ли реестр на шине.
    Registrar(bool),
}

#[derive(Clone, Copy)]
struct Signals {
    menu: RwSignal<Option<AppMenu>>,
    /// Меню окон X11: id окна → (служба, путь).
    x11: RwSignal<HashMap<u32, (String, String)>>,
}

thread_local! {
    static SIGNALS: std::cell::OnceCell<Signals> = const { std::cell::OnceCell::new() };
}

fn signals() -> Signals {
    SIGNALS.with(|s| *s.get_or_init(|| Signals { menu: use_signal(None), x11: use_signal(HashMap::new()) }))
}

static CMD: OnceLock<Mutex<Sender<Cmd>>> = OnceLock::new();

fn send(cmd: Cmd) {
    if CMD.get().is_none() {
        start();
    }
    if let Some(tx) = CMD.get() {
        let _ = tx.lock().map(|t| t.send(cmd));
    }
}

/// Держать реестр, пока в конфиге есть апплет `appmenu`.
pub fn install(ctx: ShellCtx) {
    let _ = signals();
    let last = std::cell::Cell::new(None::<bool>);
    create_effect(move || {
        let cfg = ctx.config.get();
        let want = cfg.panels.iter().any(|p| p.applets.iter().any(|a| a.kind == "appmenu"));
        if last.get() != Some(want) {
            last.set(Some(want));
            // Без апплета поток не нужен вовсе.
            if want || CMD.get().is_some() {
                send(Cmd::Registrar(want));
            }
        }
    });
    // Закрыто окно меню — сказать программе (aboutToHide).
    let open = std::cell::Cell::new(None::<i32>);
    create_effect(move || {
        let now = match ctx.popup.get().map(|p| p.kind) {
            Some(crate::ctx::PopupKind::GlobalMenu { id, .. }) => Some(id),
            _ => None,
        };
        if let Some(prev) = open.get().filter(|p| Some(*p) != now) {
            closed(prev);
        }
        open.set(now);
    });
}

/// Адрес меню окна.
pub fn address_of(w: &WindowInfo) -> Option<MenuAddr> {
    if let Some((s, p)) = &w.appmenu {
        return Some(MenuAddr::DbusMenu(s.clone(), p.clone()));
    }
    if let Some(g) = &w.gtk_menu {
        return Some(MenuAddr::Gtk(g.clone()));
    }
    let (s, p) = w.x11_id.and_then(|id| signals().x11.get().get(&id).cloned())?;
    Some(MenuAddr::DbusMenu(s, p))
}

/// Следить за меню этого окна (вызывается апплетом при смене окна).
pub fn watch(addr: Option<MenuAddr>) {
    thread_local! { static WATCHED: std::cell::RefCell<Option<Option<MenuAddr>>> = const { std::cell::RefCell::new(None) }; }
    let changed = WATCHED.with(|w| {
        let mut w = w.borrow_mut();
        if w.as_ref() == Some(&addr) {
            false
        } else {
            *w = Some(addr.clone());
            true
        }
    });
    if changed {
        send(Cmd::Watch(addr));
    }
}

/// Текущее меню (реактивно).
pub fn menu() -> Option<AppMenu> {
    signals().menu.get()
}

pub fn expand(id: i32) {
    send(Cmd::Expand(id));
}

pub fn activate(id: i32) {
    send(Cmd::Activate(id));
}

pub fn closed(id: i32) {
    send(Cmd::Closed(id));
}

// ─── Реестр ──────────────────────────────────────────────────────────────────

struct Registrar {
    sig: Signals,
    windows: Arc<Mutex<HashMap<u32, (String, String)>>>,
}

impl Registrar {
    fn publish(&self) {
        self.sig.x11.set(self.windows.lock().unwrap().clone());
    }
}

#[zbus::interface(name = "com.canonical.AppMenu.Registrar")]
impl Registrar {
    async fn register_window(
        &self,
        window_id: u32,
        menu_object_path: OwnedObjectPath,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] em: SignalEmitter<'_>,
    ) {
        let sender = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
        log::debug!("меню окна X11 {window_id:#x}: {sender}{}", menu_object_path.as_str());
        self.windows.lock().unwrap().insert(window_id, (sender.clone(), menu_object_path.to_string()));
        self.publish();
        let _ = Self::window_registered(&em, window_id, &sender, menu_object_path.as_ref()).await;
    }

    async fn unregister_window(&self, window_id: u32, #[zbus(signal_emitter)] em: SignalEmitter<'_>) {
        self.windows.lock().unwrap().remove(&window_id);
        self.publish();
        let _ = Self::window_unregistered(&em, window_id).await;
    }

    fn get_menu_for_window(&self, window_id: u32) -> zbus::fdo::Result<(String, OwnedObjectPath)> {
        let w = self.windows.lock().unwrap();
        let (s, p) = w.get(&window_id).ok_or_else(|| zbus::fdo::Error::Failed(t!("нет меню").into()))?;
        let p = OwnedObjectPath::try_from(p.as_str()).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok((s.clone(), p))
    }

    fn get_menus(&self) -> Vec<(u32, String, OwnedObjectPath)> {
        self.windows
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(id, (s, p))| Some((*id, s.clone(), OwnedObjectPath::try_from(p.as_str()).ok()?)))
            .collect()
    }

    #[zbus(signal)]
    async fn window_registered(em: &SignalEmitter<'_>, window_id: u32, service: &str, menu_object_path: ObjectPath<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn window_unregistered(em: &SignalEmitter<'_>, window_id: u32) -> zbus::Result<()>;
}

// ─── Потоки ──────────────────────────────────────────────────────────────────

fn start() {
    let (tx, rx) = channel::<Cmd>();
    if CMD.set(Mutex::new(tx.clone())).is_err() {
        return;
    }
    let sig = signals();
    std::thread::Builder::new()
        .name("appmenu-bus".into())
        .spawn(move || {
            if let Err(e) = run(sig, tx, rx) {
                log::warn!("глобальное меню: {e}");
            }
        })
        .ok();
}

fn run(sig: Signals, tx: Sender<Cmd>, rx: Receiver<Cmd>) -> zbus::Result<()> {
    let conn = Connection::session()?;
    let windows = Arc::new(Mutex::new(HashMap::new()));
    conn.object_server().at(REGISTRAR_PATH, Registrar { sig, windows: windows.clone() })?;

    let rules = [
        MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(MENU_IFACE)?.member("LayoutUpdated")?.build(),
        MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(MENU_IFACE)?.member("ItemsPropertiesUpdated")?.build(),
        MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.gtk.Menus")?.member("Changed")?.build(),
        MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.gtk.Actions")?.member("Changed")?.build(),
        MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")?
            .member("NameOwnerChanged")?
            .build(),
    ];
    let (mtx, mrx) = channel::<zbus::Message>();
    for rule in rules {
        let it = MessageIterator::for_match_rule(rule, &conn, Some(256))?;
        let mtx = mtx.clone();
        std::thread::spawn(move || {
            for m in it.flatten() {
                if mtx.send(m).is_err() {
                    break;
                }
            }
        });
    }

    let connw = conn.clone();
    std::thread::Builder::new().name("appmenu-work".into()).spawn(move || worker(connw, sig, rx)).ok();

    for msg in mrx {
        let hdr = msg.header();
        let member = hdr.member().map(|m| m.to_string()).unwrap_or_default();
        let sender = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
        let iface = hdr.interface().map(|i| i.to_string()).unwrap_or_default();
        match member.as_str() {
            "Changed" if iface == "org.gtk.Actions" => {
                let _ = tx.send(Cmd::Refresh(sender));
            }
            "LayoutUpdated" | "ItemsPropertiesUpdated" | "Changed" => {
                let _ = tx.send(Cmd::Reload(sender));
            }
            "NameOwnerChanged" => {
                let Ok((name, _old, new)) = msg.body().deserialize::<(String, String, String)>() else { continue };
                if !new.is_empty() {
                    continue;
                }
                // Программа ушла с шины — забыть меню её окон.
                let mut w = windows.lock().unwrap();
                let before = w.len();
                w.retain(|_, (s, _)| *s != name);
                if w.len() != before {
                    sig.x11.set(w.clone());
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Откуда меню, которое показываем.
enum Source {
    /// dbusmenu: служба, путь, уникальное имя владельца.
    DbusMenu { path: String, owner: String },
    Gtk(GtkState),
}

impl Source {
    fn owner(&self) -> &str {
        match self {
            Source::DbusMenu { owner, .. } => owner,
            Source::Gtk(st) => &st.menu.bus,
        }
    }
}

/// Состояние исполнителя: чьё меню показываем и что в нём раскрыто.
#[derive(Default)]
struct Current {
    key: String,
    source: Option<Source>,
    tree: Vec<MenuEntry>,
    /// Подгруженные подменю dbusmenu — после обновления подгрузить заново.
    expanded: Vec<i32>,
}

fn get_layout(conn: &Connection, dest: &str, path: &str, parent: i32) -> Option<MenuEntry> {
    let reply = conn
        .call_method(Some(dest), path, Some(MENU_IFACE), "GetLayout", &(parent, -1i32, Vec::<String>::new()))
        .map_err(|e| log::debug!("глобальное меню: GetLayout {dest}{path} {parent}: {e}"))
        .ok()?;
    let (_rev, root): (u32, RawEntry) = reply.body().deserialize().ok()?;
    Some(entry_from(root))
}

fn about_to_show(conn: &Connection, dest: &str, path: &str, id: i32) {
    let _ = conn.call_method(Some(dest), path, Some(MENU_IFACE), "AboutToShow", &(id,));
}

fn event(conn: &Connection, dest: &str, path: &str, id: i32, kind: &str) {
    let ts = (crate::clock::unix_now() & 0xffff_ffff) as u32;
    if let Err(e) = conn.call_method(Some(dest), path, Some(MENU_IFACE), "Event", &(id, kind, Value::from(0i32), ts)) {
        log::debug!("глобальное меню: Event {id} {kind}: {e}");
    }
}

/// Заменить детей пункта `id` в дереве.
fn splice(tree: &mut [MenuEntry], id: i32, children: Vec<MenuEntry>) -> bool {
    for e in tree.iter_mut() {
        if e.id == id {
            e.children = children;
            return true;
        }
        if splice(&mut e.children, id, children.clone()) {
            return true;
        }
    }
    false
}

fn publish(sig: Signals, cur: &Current) {
    let menu = cur.source.as_ref().map(|_| AppMenu { key: cur.key.clone(), entries: cur.tree.clone() });
    sig.menu.set(menu);
}

/// Перечитать меню целиком (у dbusmenu — и раскрытые подменю).
fn reload(conn: &Connection, cur: &mut Current) {
    match cur.source.take() {
        Some(Source::DbusMenu { path, owner }) => {
            cur.tree = get_layout(conn, &owner, &path, 0).map(|r| r.children).unwrap_or_default();
            for id in cur.expanded.clone() {
                if let Some(sub) = get_layout(conn, &owner, &path, id) {
                    splice(&mut cur.tree, id, sub.children);
                }
            }
            cur.source = Some(Source::DbusMenu { path, owner });
        }
        Some(Source::Gtk(old)) => {
            gtkmenu::end(conn, &old);
            let (tree, st) = gtkmenu::load(conn, &old.menu);
            cur.tree = tree;
            cur.source = Some(Source::Gtk(st));
        }
        None => {}
    }
}

/// Начать показывать меню `addr`.
fn watch_addr(conn: &Connection, cur: &mut Current, addr: Option<MenuAddr>) {
    if let Some(Source::Gtk(old)) = &cur.source {
        gtkmenu::end(conn, old);
    }
    *cur = Current::default();
    let Some(addr) = addr else { return };
    cur.key = addr.key();
    match addr {
        MenuAddr::DbusMenu(service, path) => {
            let owner = owner_of(conn, &service);
            let owner = if owner.is_empty() { service } else { owner };
            // Qt наполняет строку меню лениво — попросить показать.
            about_to_show(conn, &owner, &path, 0);
            cur.source = Some(Source::DbusMenu { path, owner });
            reload(conn, cur);
        }
        MenuAddr::Gtk(g) => {
            let (tree, st) = gtkmenu::load(conn, &g);
            log::debug!("меню GTK {}{}: {} пунктов", g.bus, g.menubar, tree.len());
            cur.tree = tree;
            cur.source = Some(Source::Gtk(st));
        }
    }
}

fn worker(conn: Connection, sig: Signals, rx: Receiver<Cmd>) {
    let mut cur = Current::default();
    let mut registrar = false;
    let mut queue = std::collections::VecDeque::new();
    while let Some(cmd) = queue.pop_front().or_else(|| rx.recv().ok()) {
        match cmd {
            Cmd::Registrar(on) => {
                if on == registrar {
                    continue;
                }
                registrar = on;
                if on {
                    match conn.request_name(REGISTRAR) {
                        Ok(()) => log::info!("глобальное меню: {REGISTRAR} на шине"),
                        Err(e) => log::warn!("глобальное меню: {REGISTRAR} занят ({e}) — меню окон X11 не будет"),
                    }
                } else {
                    let _ = conn.release_name(REGISTRAR);
                    log::info!("глобальное меню: {REGISTRAR} снят");
                }
            }
            Cmd::Watch(addr) => {
                watch_addr(&conn, &mut cur, addr);
                publish(sig, &cur);
            }
            Cmd::Reload(ref sender) | Cmd::Refresh(ref sender) => {
                if cur.source.as_ref().is_none_or(|s| s.owner() != sender) {
                    continue;
                }
                // Программа шлёт обновления пачками — собрать их в одно,
                // прочие команды выполнить следом.
                std::thread::sleep(Duration::from_millis(60));
                let mut full = matches!(cmd, Cmd::Reload(_));
                for c in rx.try_iter() {
                    match c {
                        Cmd::Reload(_) => full = true,
                        Cmd::Refresh(_) => {}
                        other => queue.push_back(other),
                    }
                }
                match (&mut cur.source, full) {
                    (Some(Source::Gtk(st)), false) => cur.tree = gtkmenu::refresh(&conn, st),
                    _ => reload(&conn, &mut cur),
                }
                publish(sig, &cur);
            }
            Cmd::Expand(id) => match &mut cur.source {
                Some(Source::DbusMenu { path, owner }) => {
                    let (path, owner) = (path.clone(), owner.clone());
                    about_to_show(&conn, &owner, &path, id);
                    event(&conn, &owner, &path, id, "opened");
                    if !cur.expanded.contains(&id) {
                        cur.expanded.push(id);
                    }
                    if let Some(sub) = get_layout(&conn, &owner, &path, id) {
                        splice(&mut cur.tree, id, sub.children);
                        publish(sig, &cur);
                    }
                }
                // Меню GTK загружено целиком — обновить флажки и доступность.
                Some(Source::Gtk(st)) => {
                    cur.tree = gtkmenu::refresh(&conn, st);
                    publish(sig, &cur);
                }
                None => {}
            },
            Cmd::Closed(id) => {
                if let Some(Source::DbusMenu { path, owner }) = &cur.source {
                    event(&conn, owner, path, id, "closed");
                }
                cur.expanded.clear();
            }
            Cmd::Activate(id) => {
                // Сначала закрывается окно меню и фокус возвращается
                // программе — действие («Вставить») идёт уже в неё.
                std::thread::sleep(Duration::from_millis(60));
                match &cur.source {
                    Some(Source::DbusMenu { path, owner }) => event(&conn, owner, path, id, "clicked"),
                    Some(Source::Gtk(st)) => gtkmenu::activate(&conn, st, id),
                    None => {}
                }
            }
        }
    }
}
