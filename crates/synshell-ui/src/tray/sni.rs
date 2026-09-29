//! StatusNotifierItem по D-Bus: свой `org.kde.StatusNotifierWatcher` (если
//! на шине его нет), хост, чтение элементов и меню `com.canonical.dbusmenu`.
//!
//! Потоки: «tray-bus» ведёт соединение и подписки на сигналы, «tray-work»
//! выполняет запросы интерфейса (Activate, меню) — зависшая программа не
//! держит интерфейс. Результаты уходят в сигналы syngui (потокобезопасно).

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use syngui::signal::RwSignal;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::MatchRule;

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
const MENU_IFACE: &str = "com.canonical.dbusmenu";

/// Значок: путь к файлу или готовые пиксели.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum TrayIcon {
    #[default]
    None,
    Path(String),
    Rgba { key: String, w: u32, h: u32, data: Arc<Vec<u8>> },
}

impl TrayIcon {
    pub fn is_none(&self) -> bool {
        matches!(self, TrayIcon::None)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrayItem {
    /// `служба|путь` — ключ элемента.
    pub key: String,
    pub service: String,
    pub path: String,
    /// Уникальное имя владельца (`:1.42`) — по нему узнаём сигналы.
    pub owner: String,
    pub id: String,
    pub title: String,
    pub tooltip: String,
    /// `Active`, `Passive`, `NeedsAttention`.
    pub status: String,
    pub icon: TrayIcon,
    pub attention: TrayIcon,
    pub overlay: TrayIcon,
    pub item_is_menu: bool,
    pub menu: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MenuEntry {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub visible: bool,
    pub separator: bool,
    /// (радиокнопка, включено) для checkmark/radio.
    pub toggle: Option<(bool, bool)>,
    pub icon: TrayIcon,
    /// Сочетание клавиш для подписи («Ctrl+S»).
    pub shortcut: String,
    /// Подменю (`children-display = submenu`), даже если пункты ещё не
    /// загружены — Qt наполняет их по AboutToShow.
    pub submenu: bool,
    pub children: Vec<MenuEntry>,
}

pub enum Cmd {
    Activate { key: String, x: i32, y: i32 },
    Secondary { key: String, x: i32, y: i32 },
    ContextMenu { key: String, x: i32, y: i32 },
    Scroll { key: String, delta: i32 },
    OpenMenu { key: String },
    MenuEvent { key: String, id: i32 },
    Resync,
}

/// Сигналы для интерфейса.
#[derive(Clone, Copy)]
pub struct TraySignals {
    pub items: RwSignal<Vec<TrayItem>>,
    /// Меню, открытое сейчас: (ключ элемента, пункты).
    pub menu: RwSignal<Option<(String, Vec<MenuEntry>)>>,
}

static CMD: OnceLock<Mutex<Sender<Cmd>>> = OnceLock::new();

pub fn send(cmd: Cmd) {
    if let Some(tx) = CMD.get() {
        let _ = tx.lock().map(|t| t.send(cmd));
    }
}

// ─── Watcher ─────────────────────────────────────────────────────────────────

#[derive(Default)]
struct WatcherState {
    items: Vec<String>,
    hosts: Vec<String>,
}

struct Watcher {
    st: Arc<Mutex<WatcherState>>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] em: SignalEmitter<'_>,
    ) {
        let sender = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
        // Два варианта: имя шины (KDE) или путь объекта (libappindicator,
        // Electron) — тогда служба = отправитель.
        let full = if service.starts_with('/') {
            format!("{sender}{service}")
        } else if service.contains('/') {
            service.to_string()
        } else {
            format!("{service}/StatusNotifierItem")
        };
        let added = {
            let mut st = self.st.lock().unwrap();
            if st.items.contains(&full) {
                false
            } else {
                st.items.push(full.clone());
                true
            }
        };
        if added {
            log::info!("трей: зарегистрирован {full}");
            let _ = Self::status_notifier_item_registered(&em, &full).await;
        }
    }

    async fn register_status_notifier_host(&self, service: &str, #[zbus(signal_emitter)] em: SignalEmitter<'_>) {
        self.st.lock().unwrap().hosts.push(service.to_string());
        let _ = Self::status_notifier_host_registered(&em).await;
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.st.lock().unwrap().items.clone()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        !self.st.lock().unwrap().hosts.is_empty()
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(em: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(em: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(em: &SignalEmitter<'_>) -> zbus::Result<()>;
}

// ─── Разбор значений ─────────────────────────────────────────────────────────

fn prop<T: TryFrom<OwnedValue>>(p: &HashMap<String, OwnedValue>, k: &str) -> Option<T> {
    p.get(k).and_then(|v| v.try_clone().ok()).and_then(|v| T::try_from(v).ok())
}

/// `a(iiay)` ARGB32 (big-endian) → RGBA; размер — ближайший не меньше `want`.
fn pixmap(v: Option<Vec<(i32, i32, Vec<u8>)>>, want: i32, tag: &str) -> TrayIcon {
    let Some(list) = v else { return TrayIcon::None };
    let list: Vec<_> = list.into_iter().filter(|(w, h, d)| *w > 0 && *h > 0 && d.len() >= (*w * *h * 4) as usize).collect();
    let best = list
        .iter()
        .filter(|(w, _, _)| *w >= want)
        .min_by_key(|(w, _, _)| *w)
        .or_else(|| list.iter().max_by_key(|(w, _, _)| *w));
    let Some((w, h, d)) = best else { return TrayIcon::None };
    let mut rgba = Vec::with_capacity(d.len());
    for px in d.chunks_exact(4).take((*w * *h) as usize) {
        rgba.extend_from_slice(&[px[1], px[2], px[3], px[0]]);
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&rgba, &mut hasher);
    let key = format!("tray-{tag}-{:x}", std::hash::Hasher::finish(&hasher));
    TrayIcon::Rgba { key, w: *w as u32, h: *h as u32, data: Arc::new(rgba) }
}

/// Значок по имени: сначала в `IconThemePath` программы, потом в теме.
fn named_icon(name: &str, theme_path: &str) -> TrayIcon {
    if name.is_empty() {
        return TrayIcon::None;
    }
    if name.starts_with('/') {
        return if std::path::Path::new(name).exists() { TrayIcon::Path(name.into()) } else { TrayIcon::None };
    }
    if !theme_path.is_empty() {
        if let Some(p) = find_in(std::path::Path::new(theme_path), name, 0) {
            return TrayIcon::Path(p);
        }
    }
    crate::xdg::lookup_icon(name).map(|p| TrayIcon::Path(p.to_string_lossy().into_owned())).unwrap_or_default()
}

fn find_in(dir: &std::path::Path, name: &str, depth: u32) -> Option<String> {
    for ext in ["svg", "png"] {
        let p = dir.join(format!("{name}.{ext}"));
        if p.is_file() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    if depth >= 4 {
        return None;
    }
    let mut dirs: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    dirs.into_iter().find_map(|d| find_in(&d, name, depth + 1))
}

fn split_item(s: &str) -> (String, String) {
    match s.find('/') {
        Some(i) => (s[..i].to_string(), s[i..].to_string()),
        None => (s.to_string(), "/StatusNotifierItem".to_string()),
    }
}

pub(crate) fn owner_of(conn: &Connection, name: &str) -> String {
    if name.starts_with(':') {
        return name.to_string();
    }
    zbus::blocking::fdo::DBusProxy::new(conn)
        .ok()
        .and_then(|p| p.get_name_owner(name.try_into().ok()?).ok())
        .map(|o| o.to_string())
        .unwrap_or_default()
}

fn load_item(conn: &Connection, full: &str, icon_px: i32) -> Option<TrayItem> {
    let (service, path) = split_item(full);
    let props: HashMap<String, OwnedValue> = conn
        .call_method(Some(service.as_str()), path.as_str(), Some("org.freedesktop.DBus.Properties"), "GetAll", &(ITEM_IFACE,))
        .ok()?
        .body()
        .deserialize()
        .ok()?;
    let theme_path: String = prop(&props, "IconThemePath").unwrap_or_default();
    let name: String = prop(&props, "IconName").unwrap_or_default();
    let mut icon = named_icon(&name, &theme_path);
    if icon.is_none() {
        icon = pixmap(prop(&props, "IconPixmap"), icon_px, "i");
    }
    let att_name: String = prop(&props, "AttentionIconName").unwrap_or_default();
    let mut attention = named_icon(&att_name, &theme_path);
    if attention.is_none() {
        attention = pixmap(prop(&props, "AttentionIconPixmap"), icon_px, "a");
    }
    let ov_name: String = prop(&props, "OverlayIconName").unwrap_or_default();
    let mut overlay = named_icon(&ov_name, &theme_path);
    if overlay.is_none() {
        overlay = pixmap(prop(&props, "OverlayIconPixmap"), icon_px / 2, "o");
    }
    let tooltip = props
        .get("ToolTip")
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| <(String, Vec<(i32, i32, Vec<u8>)>, String, String)>::try_from(v).ok())
        .map(|(_, _, t, b)| if b.is_empty() { t } else if t.is_empty() { b } else { format!("{t}\n{b}") })
        .unwrap_or_default();
    let menu: Option<OwnedObjectPath> = prop(&props, "Menu");
    Some(TrayItem {
        key: format!("{service}|{path}"),
        owner: owner_of(conn, &service),
        id: prop(&props, "Id").unwrap_or_default(),
        title: prop(&props, "Title").unwrap_or_default(),
        tooltip: crate::notifications::strip_markup_pub(&tooltip),
        status: prop(&props, "Status").unwrap_or_else(|| "Active".into()),
        icon,
        attention,
        overlay,
        item_is_menu: prop(&props, "ItemIsMenu").unwrap_or(false),
        menu: menu.map(|p| p.to_string()).filter(|p| p != "/" && !p.is_empty()),
        service,
        path,
    })
}

// ─── dbusmenu ────────────────────────────────────────────────────────────────

pub(crate) type RawEntry = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

/// Ребёнок в `av` — вариант со структурой `(ia{sv}av)`.
fn child_entry(v: &OwnedValue) -> Option<MenuEntry> {
    let v = v.try_clone().ok()?;
    let raw: RawEntry = match <RawEntry>::try_from(v.try_clone().ok()?) {
        Ok(r) => r,
        Err(_) => {
            // Иногда структура вложена в ещё один вариант.
            let inner: OwnedValue = OwnedValue::try_from(v).ok()?;
            <RawEntry>::try_from(inner).ok()?
        }
    };
    Some(entry_from(raw))
}

pub(crate) fn entry_from((id, props, kids): RawEntry) -> MenuEntry {
    let s_prop = |k: &str| props.get(k).and_then(|v| v.try_clone().ok()).and_then(|v| String::try_from(v).ok());
    let b_prop = |k: &str, d: bool| props.get(k).and_then(|v| bool::try_from(v).ok()).unwrap_or(d);
    let label = s_prop("label").unwrap_or_default();
    // «_Файл» → «Файл», «__» → «_».
    let label = label.replace("__", "\u{1}").replace('_', "").replace('\u{1}', "_");
    let toggle = match s_prop("toggle-type").as_deref() {
        Some(t @ ("checkmark" | "radio")) => {
            let st = props.get("toggle-state").and_then(|v| i32::try_from(v).ok()).unwrap_or(0);
            Some((t == "radio", st == 1))
        }
        _ => None,
    };
    let mut icon = s_prop("icon-name").map(|n| named_icon(&n, "")).unwrap_or_default();
    if icon.is_none() {
        if let Some(png) = props.get("icon-data").and_then(|v| v.try_clone().ok()).and_then(|v| <Vec<u8>>::try_from(v).ok()) {
            // PNG — через файл во временном каталоге (Image грузит пути).
            let mut h = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(&png, &mut h);
            let p = std::env::temp_dir().join(format!("synshell-tray-{:x}.png", std::hash::Hasher::finish(&h)));
            if !p.exists() {
                let _ = std::fs::write(&p, &png);
            }
            icon = TrayIcon::Path(p.to_string_lossy().into_owned());
        }
    }
    // `aas`: [["Control", "S"]] — первое сочетание.
    let shortcut = props
        .get("shortcut")
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| <Vec<Vec<String>>>::try_from(v).ok())
        .and_then(|l| l.into_iter().next())
        .map(|keys| {
            keys.iter()
                .map(|k| match k.as_str() {
                    "Control" => "Ctrl",
                    "Super" => "Super",
                    other => other,
                })
                .collect::<Vec<_>>()
                .join("+")
        })
        .unwrap_or_default();
    MenuEntry {
        id,
        label,
        shortcut,
        submenu: s_prop("children-display").as_deref() == Some("submenu"),
        enabled: b_prop("enabled", true),
        visible: b_prop("visible", true),
        separator: s_prop("type").as_deref() == Some("separator"),
        toggle,
        icon,
        children: kids.iter().filter_map(child_entry).collect(),
    }
}

fn get_layout(conn: &Connection, item: &TrayItem) -> Option<Vec<MenuEntry>> {
    let path = item.menu.as_ref()?;
    let dest = if item.owner.is_empty() { item.service.as_str() } else { item.owner.as_str() };
    let _ = conn.call_method(Some(dest), path.as_str(), Some(MENU_IFACE), "AboutToShow", &(0i32,));
    let reply = conn
        .call_method(Some(dest), path.as_str(), Some(MENU_IFACE), "GetLayout", &(0i32, -1i32, Vec::<String>::new()))
        .ok()?;
    let (_rev, root): (u32, RawEntry) = reply.body().deserialize().ok()?;
    Some(entry_from(root).children)
}

// ─── Потоки ──────────────────────────────────────────────────────────────────

struct Shared {
    conn: Connection,
    sig: TraySignals,
    icon_px: i32,
    watcher: Option<Arc<Mutex<WatcherState>>>,
}

fn watcher_items(sh: &Shared) -> Vec<String> {
    if let Some(w) = &sh.watcher {
        return w.lock().unwrap().items.clone();
    }
    Proxy::new(&sh.conn, WATCHER, WATCHER_PATH, WATCHER)
        .ok()
        .and_then(|p| p.get_property::<Vec<String>>("RegisteredStatusNotifierItems").ok())
        .unwrap_or_default()
}

fn resync(sh: &Shared) {
    let list: Vec<TrayItem> = watcher_items(sh).iter().filter_map(|s| load_item(&sh.conn, s, sh.icon_px)).collect();
    sh.sig.items.set(list);
}

/// Перечитать один элемент (сигнал New*).
fn reload_one(sh: &Shared, sender: &str, path: &str) {
    let items = watcher_items(sh);
    for s in items {
        let (svc, p) = split_item(&s);
        if p != path {
            continue;
        }
        if svc == sender || owner_of(&sh.conn, &svc) == sender {
            if let Some(it) = load_item(&sh.conn, &s, sh.icon_px) {
                let sig = sh.sig.items;
                syngui::async_runtime::run_on_main_thread(move || {
                    sig.update(|l| match l.iter_mut().find(|x| x.key == it.key) {
                        Some(x) => *x = it.clone(),
                        None => l.push(it.clone()),
                    })
                });
            }
        }
    }
}

pub fn start(sig: TraySignals, icon_px: i32) {
    let (tx, rx) = channel::<Cmd>();
    if CMD.set(Mutex::new(tx.clone())).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("tray-bus".into())
        .spawn(move || {
            if let Err(e) = run(sig, icon_px, tx, rx) {
                log::warn!("трей: {e}");
            }
        })
        .ok();
}

fn run(sig: TraySignals, icon_px: i32, tx: Sender<Cmd>, rx: Receiver<Cmd>) -> zbus::Result<()> {
    let conn = Connection::session()?;
    // Свой наблюдатель, если на шине его ещё нет.
    let st = Arc::new(Mutex::new(WatcherState::default()));
    let own_watcher = match conn.request_name(WATCHER) {
        Ok(()) => {
            conn.object_server().at(WATCHER_PATH, Watcher { st: st.clone() })?;
            log::info!("трей: свой {WATCHER}");
            true
        }
        Err(_) => {
            log::info!("трей: {WATCHER} уже есть, становимся хостом");
            false
        }
    };
    let host = format!("org.kde.StatusNotifierHost-{}", std::process::id());
    let _ = conn.request_name(host.as_str());
    let _ = conn.call_method(Some(WATCHER), WATCHER_PATH, Some(WATCHER), "RegisterStatusNotifierHost", &(host.as_str(),));
    let sh = Arc::new(Shared { conn: conn.clone(), sig, icon_px, watcher: own_watcher.then_some(st) });
    resync(&sh);

    // Подписки: каждое правило — свой итератор в своём потоке → один канал.
    let rules = [
        MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(WATCHER)?.build(),
        MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(ITEM_IFACE)?.build(),
        MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(MENU_IFACE)?.build(),
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

    // Исполнитель команд интерфейса.
    let shw = sh.clone();
    std::thread::Builder::new().name("tray-work".into()).spawn(move || worker(shw, rx)).ok();

    // Страховочная пересинхронизация.
    let tx2 = tx.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(10));
        if tx2.send(Cmd::Resync).is_err() {
            break;
        }
    });

    for msg in mrx {
        let hdr = msg.header();
        let member = hdr.member().map(|m| m.to_string()).unwrap_or_default();
        let sender = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
        let path = hdr.path().map(|p| p.to_string()).unwrap_or_default();
        match member.as_str() {
            "StatusNotifierItemRegistered" | "StatusNotifierItemUnregistered" => {
                let _ = tx.send(Cmd::Resync);
            }
            "NewIcon" | "NewTitle" | "NewStatus" | "NewToolTip" | "NewAttentionIcon" | "NewOverlayIcon" | "NewMenu" => {
                reload_one(&sh, &sender, &path);
            }
            "LayoutUpdated" | "ItemsPropertiesUpdated" => {
                // Обновить открытое меню этого владельца.
                let open = OPEN_MENU.lock().ok().and_then(|g| g.clone());
                if let Some(key) = open {
                    let _ = tx.send(Cmd::OpenMenu { key });
                }
            }
            "NameOwnerChanged" => {
                let Ok((name, _old, new)) = msg.body().deserialize::<(String, String, String)>() else { continue };
                if !new.is_empty() {
                    continue;
                }
                if let Some(w) = &sh.watcher {
                    let gone: Vec<String> = {
                        let mut st = w.lock().unwrap();
                        let gone: Vec<String> = st.items.iter().filter(|s| split_item(s).0 == name).cloned().collect();
                        st.items.retain(|s| split_item(s).0 != name);
                        st.hosts.retain(|h| *h != name);
                        gone
                    };
                    for g in gone {
                        log::info!("трей: {g} ушёл с шины");
                        let _ = conn.emit_signal(None::<&str>, WATCHER_PATH, WATCHER, "StatusNotifierItemUnregistered", &(g.as_str(),));
                    }
                }
                let _ = tx.send(Cmd::Resync);
            }
            _ => {}
        }
    }
    Ok(())
}

/// Ключ открытого меню — копия для фонового потока (сигналы читаются
/// только в главном).
static OPEN_MENU: Mutex<Option<String>> = Mutex::new(None);

/// Какое меню сейчас открыто (для обновления по LayoutUpdated).
pub fn set_open_menu(key: Option<String>) {
    if let Ok(mut g) = OPEN_MENU.lock() {
        *g = key;
    }
}

fn find_item(sh: &Shared, key: &str) -> Option<TrayItem> {
    let (svc, path) = key.split_once('|')?;
    load_item(&sh.conn, &format!("{svc}{path}"), sh.icon_px)
}

fn dest(it: &TrayItem) -> &str {
    if it.owner.is_empty() {
        &it.service
    } else {
        &it.owner
    }
}

fn worker(sh: Arc<Shared>, rx: Receiver<Cmd>) {
    for cmd in rx {
        match cmd {
            Cmd::Resync => resync(&sh),
            Cmd::Activate { key, x, y } => call_item(&sh, &key, "Activate", x, y),
            Cmd::Secondary { key, x, y } => call_item(&sh, &key, "SecondaryActivate", x, y),
            Cmd::ContextMenu { key, x, y } => call_item(&sh, &key, "ContextMenu", x, y),
            Cmd::Scroll { key, delta } => {
                if let Some(it) = find_item(&sh, &key) {
                    let _ = sh.conn.call_method(Some(dest(&it)), it.path.as_str(), Some(ITEM_IFACE), "Scroll", &(delta, "vertical"));
                }
            }
            Cmd::OpenMenu { key } => {
                let menu = find_item(&sh, &key).and_then(|it| get_layout(&sh.conn, &it));
                sh.sig.menu.set(Some((key, menu.unwrap_or_default())));
            }
            Cmd::MenuEvent { key, id } => {
                if let Some(it) = find_item(&sh, &key) {
                    if let Some(path) = &it.menu {
                        let ts = (crate::clock::unix_now() & 0xffff_ffff) as u32;
                        let r = sh.conn.call_method(
                            Some(dest(&it)),
                            path.as_str(),
                            Some(MENU_IFACE),
                            "Event",
                            &(id, "clicked", Value::from(0i32), ts),
                        );
                        if let Err(e) = r {
                            log::debug!("трей: Event {id}: {e}");
                        }
                    }
                }
            }
        }
    }
}

fn call_item(sh: &Shared, key: &str, method: &str, x: i32, y: i32) {
    let Some(it) = find_item(sh, key) else { return };
    if let Err(e) = sh.conn.call_method(Some(dest(&it)), it.path.as_str(), Some(ITEM_IFACE), method, &(x, y)) {
        log::debug!("трей: {method} у {}: {e}", it.id);
    }
}
