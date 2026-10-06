//! Мост binder между Android и сеансом synshell — `syndroidd __bridge <экземпляр>`.
//!
//! Демон запускает его на каждый запуск контейнера от имени владельца сеанса (ярлыки — в его
//! `~/.local/share/applications`, уведомления — в его D-Bus). Мост открывает binder-устройство контейнера
//! (`/dev/binderfs/syndroid-binder`) и говорит с Android по протоколу образов Waydroid:
//! - регистрирует в servicemanager Android сервисы хоста: `waydroidusermonitor` (приложения установлены /
//!   удалены), `waydroidhardware` (выключение, перезагрузка, сон из Android), `waydroidnotifications`
//!   (уведомления Android → `org.freedesktop.Notifications`, нажатия — обратно);
//! - у `waydroidplatform` (system_server) берёт список приложений: имена, категории → ярлыки
//!   `~/.local/share/syndroid/applications/<экземпляр>/<пакет>.desktop` — отдельно от программ Linux
//!   (значки Android сам кладёт в /data/icons).
//!
//! Посылки описаны вручную: `AppInfo` и др. — старые Java-Parcelable без префикса размера, а servicemanager —
//! протокол Android 13 (`checkService` = 2, `addService` = 3), тогда как `rsbinder::hub` на Linux знает только
//! Android 16.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use rsbinder::{
    Binder, Interface, Parcel, ProcessState, Remotable, SIBinder, StatusCode, TransactionCode,
    FIRST_CALL_TRANSACTION, FLAG_ONEWAY, INTERFACE_HEADER,
};

use crate::api::{self, Request};
use crate::{container, images, paths};

const SM: &str = "android.os.IServiceManager";
const PLATFORM: &str = "lineageos.waydroid.IPlatform";
const USER_MONITOR: &str = "lineageos.waydroid.IUserMonitor";
const HARDWARE: &str = "lineageos.waydroid.IHardware";
const NOTIFICATIONS: &str = "lineageos.waydroid.INotifications";
const NOTIFICATION_CALLBACK: &str = "lineageos.waydroid.INotifications.INotificationCallback";

/// Маркер наших ярлыков (чужие `waydroid.*.desktop`, например от Waydroid, не трогаем).
const DESKTOP_MARK: &str = "X-Syndroid=true";

enum Event {
    AppsChanged,
    Suspend,
    Reboot,
    Shutdown,
}

// --- посылки ---------------------------------------------------------------------------------------

/// Токен интерфейса, как у libbinder (Android 11+): политика strict mode, work source, 'SYST', имя.
fn request(descriptor: &str) -> Result<Parcel> {
    let mut p = Parcel::new();
    p.write(&(1i32 << 31))?; // STRICT_MODE_PENALTY_GATHER
    p.write(&-1i32)?; // work source не задан
    p.write(&INTERFACE_HEADER)?;
    p.write(&descriptor.to_string())?;
    Ok(p)
}

fn call(target: &SIBinder, code: u32, data: &Parcel) -> Result<Parcel> {
    let proxy = target.as_proxy().context("не удалённый binder")?;
    let mut reply = proxy
        .submit_transact(FIRST_CALL_TRANSACTION + code - 1, data, 0)
        .map_err(|e| anyhow!("транзакция {code}: {e:?}"))?
        .context("нет ответа")?;
    let ex: i32 = reply.read().map_err(|e| anyhow!("{e:?}"))?;
    if ex != 0 {
        let msg: Option<String> = reply.read().unwrap_or(None);
        bail!("исключение Android {ex}: {}", msg.unwrap_or_default());
    }
    Ok(reply)
}

fn rd<T: rsbinder::Deserialize>(p: &mut Parcel) -> Result<T> {
    p.read::<T>().map_err(|e| anyhow!("посылка: {e:?}"))
}

fn rd_str(p: &mut Parcel) -> Result<String> {
    Ok(rd::<Option<String>>(p)?.unwrap_or_default())
}

// --- servicemanager Android 13 ------------------------------------------------------------------------

fn service_manager() -> Result<SIBinder> {
    ProcessState::as_self().context_object().map_err(|e| anyhow!("servicemanager: {e:?}"))
}

fn check_service(name: &str) -> Result<Option<SIBinder>> {
    let mut d = request(SM)?;
    d.write(&name.to_string())?;
    let mut r = call(&service_manager()?, 2, &d)?;
    rd::<Option<SIBinder>>(&mut r)
}

fn add_service(name: &str, binder: SIBinder) -> Result<()> {
    let mut d = request(SM)?;
    d.write(&name.to_string())?;
    d.write(&binder)?;
    d.write(&false)?; // allowIsolated
    d.write(&8i32)?; // DUMP_FLAG_PRIORITY_DEFAULT
    call(&service_manager()?, 3, &d).map(drop)
}

// --- сервисы хоста ---------------------------------------------------------------------------------

fn ok_reply(reply: &mut Parcel) -> rsbinder::Result<()> {
    reply.write(&0i32)
}

struct UserMonitor(Mutex<Sender<Event>>);

impl Remotable for UserMonitor {
    fn descriptor() -> &'static str {
        USER_MONITOR
    }
    fn on_transact(&self, code: TransactionCode, reader: &mut Parcel, reply: &mut Parcel) -> rsbinder::Result<()> {
        match code - FIRST_CALL_TRANSACTION + 1 {
            // userUnlocked(int uid)
            1 => {
                let _uid: i32 = reader.read()?;
                let _ = self.0.lock().unwrap().send(Event::AppsChanged);
            }
            // packageStateChanged(int mode, String package, int uid)
            2 => {
                let mode: i32 = reader.read()?;
                let pkg: Option<String> = reader.read()?;
                tracing::info!("Android: пакет {} ({})", pkg.unwrap_or_default(), ["добавлен", "удалён", "обновлён"].get(mode as usize).unwrap_or(&"?"));
                let _ = self.0.lock().unwrap().send(Event::AppsChanged);
            }
            _ => return Err(StatusCode::UnknownTransaction),
        }
        ok_reply(reply)
    }
    fn on_dump(&self, _: &mut dyn std::io::Write, _: &[String]) -> rsbinder::Result<()> {
        Ok(())
    }
}

struct Hardware(Mutex<Sender<Event>>);

impl Remotable for Hardware {
    fn descriptor() -> &'static str {
        HARDWARE
    }
    fn on_transact(&self, code: TransactionCode, _reader: &mut Parcel, reply: &mut Parcel) -> rsbinder::Result<()> {
        let tx = self.0.lock().unwrap();
        match code - FIRST_CALL_TRANSACTION + 1 {
            // enableNFC / enableBluetooth (bool) → int: не поддерживается
            1 | 2 => {
                ok_reply(reply)?;
                return reply.write(&0i32);
            }
            3 => drop(tx.send(Event::Suspend)),
            4 => drop(tx.send(Event::Reboot)),
            // upgrade / upgrade2: образы обновляет syndroid
            5 | 6 => {}
            // shutdownRequest(String reason): "1…" — перезагрузка
            7 => {
                let reason: Option<String> = _reader.read().unwrap_or(None);
                let reboot = reason.as_deref().is_some_and(|r| r.starts_with('1'));
                drop(tx.send(if reboot { Event::Reboot } else { Event::Shutdown }));
            }
            _ => return Err(StatusCode::UnknownTransaction),
        }
        ok_reply(reply)
    }
    fn on_dump(&self, _: &mut dyn std::io::Write, _: &[String]) -> rsbinder::Result<()> {
        Ok(())
    }
}

/// Уведомления Android → org.freedesktop.Notifications сеанса.
struct Notifications {
    dbus: Option<zbus::blocking::Connection>,
    listeners: Arc<Mutex<Vec<SIBinder>>>,
    /// Наши id уведомлений (для ActionInvoked).
    ids: Arc<Mutex<HashSet<u32>>>,
}

impl Notifications {
    fn notify(&self, r: &mut Parcel) -> Result<u32> {
        let replaces: i32 = rd(r)?;
        let app_name = rd_str(r)?;
        let package = rd_str(r)?;
        let summary = rd_str(r)?;
        let body = rd_str(r)?;
        let mut actions: Vec<String> = Vec::new();
        let n: i32 = rd(r)?;
        for _ in 0..n.max(0) {
            if rd::<i32>(r)? != 0 {
                let _size: i32 = rd(r)?;
                actions.push(rd_str(r)?);
                actions.push(rd_str(r)?);
            }
        }
        let mut hints: BTreeMap<&str, zbus::zvariant::Value> = BTreeMap::new();
        if rd::<i32>(r)? != 0 {
            let _size: i32 = rd(r)?;
            let (w, h, stride): (i32, i32, i32) = (rd(r)?, rd(r)?, rd(r)?);
            let alpha: bool = rd(r)?;
            let data: Vec<u8> = rd::<Option<Vec<u8>>>(r)?.unwrap_or_default();
            hints.insert(
                "image-data",
                zbus::zvariant::Value::from((w, h, stride, alpha, 8i32, if alpha { 4i32 } else { 3 }, data)),
            );
        }
        let category = rd_str(r)?;
        let suppress_sound: bool = rd(r)?;
        let expire: i32 = rd(r)?;
        let resident: bool = rd(r)?;
        let transient: bool = rd(r)?;
        let urgency: i32 = rd(r)?; // byte в посылке — int32
        // Служебные уведомления системы Android («USB debugging», «Charging via USB»…) на хосте не нужны
        if matches!(package.as_str(), "android" | "com.android.systemui") {
            return Ok(0);
        }
        let entry = format!("waydroid.{package}");
        hints.insert("desktop-entry", entry.as_str().into());
        hints.insert("resident", resident.into());
        hints.insert("transient", transient.into());
        hints.insert("urgency", (urgency as u8).into());
        hints.insert("suppress-sound", suppress_sound.into());
        if !category.is_empty() {
            hints.insert("category", category.as_str().into());
        }
        let conn = self.dbus.as_ref().context("нет D-Bus сеанса")?;
        let reply = conn.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(app_name.as_str(), replaces.max(0) as u32, "", summary.as_str(), body.as_str(), &actions, &hints, expire),
        )?;
        let id: u32 = reply.body().deserialize()?;
        self.ids.lock().unwrap().insert(id);
        Ok(id)
    }
}

impl Remotable for Notifications {
    fn descriptor() -> &'static str {
        NOTIFICATIONS
    }
    fn on_transact(&self, code: TransactionCode, reader: &mut Parcel, reply: &mut Parcel) -> rsbinder::Result<()> {
        match code - FIRST_CALL_TRANSACTION + 1 {
            // registerListener(INotificationCallback)
            1 => {
                if let Some(b) = reader.read::<Option<SIBinder>>()? {
                    self.listeners.lock().unwrap().push(b);
                }
                ok_reply(reply)
            }
            // notify(…) → int id
            2 => {
                let id = self.notify(reader).unwrap_or_else(|e| {
                    tracing::warn!("уведомление: {e:#}");
                    0
                });
                ok_reply(reply)?;
                reply.write(&(id as i32))
            }
            // closeNotification(int id)
            3 => {
                let id: i32 = reader.read()?;
                if let Some(c) = &self.dbus {
                    let _ = c.call_method(
                        Some("org.freedesktop.Notifications"),
                        "/org/freedesktop/Notifications",
                        Some("org.freedesktop.Notifications"),
                        "CloseNotification",
                        &(id as u32),
                    );
                }
                self.ids.lock().unwrap().remove(&(id as u32));
                ok_reply(reply)
            }
            _ => Err(StatusCode::UnknownTransaction),
        }
    }
    fn on_dump(&self, _: &mut dyn std::io::Write, _: &[String]) -> rsbinder::Result<()> {
        Ok(())
    }
}

/// Нажатия на уведомления (ActionInvoked) — слушателям Android (`onActionInvoked`, oneway).
fn forward_actions(conn: zbus::blocking::Connection, listeners: Arc<Mutex<Vec<SIBinder>>>, ids: Arc<Mutex<HashSet<u32>>>) {
    std::thread::spawn(move || {
        let rule = "type='signal',interface='org.freedesktop.Notifications',member='ActionInvoked'";
        let Ok(rule) = zbus::MatchRule::try_from(rule) else { return };
        let Ok(it) = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, None) else { return };
        for msg in it.flatten() {
            let Ok((id, action)) = msg.body().deserialize::<(u32, String)>() else { continue };
            if !ids.lock().unwrap().contains(&id) {
                continue;
            }
            for l in listeners.lock().unwrap().iter() {
                let send = || -> Result<()> {
                    let mut d = request(NOTIFICATION_CALLBACK)?;
                    d.write(&(id as i32))?;
                    d.write(&action)?;
                    d.write(&String::new())?; // xdg activation token
                    l.as_proxy().context("proxy")?.submit_transact(FIRST_CALL_TRANSACTION, &d, FLAG_ONEWAY).map_err(|e| anyhow!("{e:?}"))?;
                    Ok(())
                };
                if let Err(e) = send() {
                    tracing::debug!("onActionInvoked: {e:#}");
                }
            }
        }
    });
}

// --- приложения → ярлыки -------------------------------------------------------------------------

struct AppInfo {
    name: String,
    package: String,
    categories: Vec<String>,
}

fn apps_info(platform: &SIBinder) -> Result<Vec<AppInfo>> {
    let mut r = call(platform, 3, &request(PLATFORM)?)?;
    let n: i32 = rd(&mut r)?;
    let mut v = Vec::new();
    for _ in 0..n.max(0) {
        if rd::<i32>(&mut r)? == 0 {
            continue;
        }
        let name = rd_str(&mut r)?;
        let package = rd_str(&mut r)?;
        for _ in 0..4 {
            rd_str(&mut r)?; // action, launchIntent, componentPackageName, componentClassName
        }
        let c: i32 = rd(&mut r)?;
        let mut categories = Vec::new();
        for _ in 0..c.max(0) {
            categories.push(rd_str(&mut r)?);
        }
        v.push(AppInfo { name, package, categories });
    }
    Ok(v)
}

/// Категории Android → freedesktop.
fn xdg_categories(cats: &[String]) -> String {
    let mut out: Vec<&str> = Vec::new();
    for c in cats {
        let m: &[&str] = match c.trim_start_matches("android.intent.category.") {
            "APP_BROWSER" => &["Network", "WebBrowser"],
            "APP_CALCULATOR" => &["Utility", "Calculator"],
            "APP_CALENDAR" => &["Office", "Calendar"],
            "APP_CONTACTS" => &["Office", "ContactManagement"],
            "APP_EMAIL" => &["Network", "Email"],
            "APP_GALLERY" => &["Graphics", "Viewer"],
            "APP_MAPS" => &["Utility", "Maps"],
            "APP_MESSAGING" => &["Network", "InstantMessaging"],
            "APP_MUSIC" => &["AudioVideo", "Audio", "Music"],
            "APP_FILES" => &["System", "FileManager"],
            "APP_FITNESS" => &["Utility"],
            "APP_WEATHER" => &["Utility"],
            "GAME" => &["Game"],
            _ => &[],
        };
        for x in m {
            if !out.contains(x) {
                out.push(x);
            }
        }
    }
    out.push("X-Android");
    out.iter().map(|c| format!("{c};")).collect()
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"))
}

/// Ярлыки приложений экземпляра — отдельно от программ Linux (не в `applications/`): меню synshell
/// показывает их своим разделом, другие среды их не видят.
fn applications_dir(instance: &str) -> PathBuf {
    data_home().join("syndroid/applications").join(instance)
}

/// Записать файл, только если содержимое другое (меню перечитывает ярлыки по mtime каталога).
fn write_if_changed(path: &std::path::Path, text: &str) -> Result<()> {
    if std::fs::read_to_string(path).ok().as_deref() != Some(text) {
        std::fs::write(path, text)?;
    }
    Ok(())
}

fn write_desktop_files(instance: &str, apps: &[AppInfo]) -> Result<()> {
    let dir = applications_dir(instance);
    std::fs::create_dir_all(&dir)?;
    let title = images::instance_title(instance);
    let icons = paths::data(instance).join("icons");
    // Свои копии значков: Android перезаписывает /data/icons при каждой загрузке — оболочка, читающая значок
    // в этот момент, получала бы недописанный файл
    let my_icons = data_home().join("syndroid/icons").join(instance);
    std::fs::create_dir_all(&my_icons)?;
    let head = |name: &str, comment: &str, exec: &str, icon: &str| {
        format!(
            "[Desktop Entry]\nType=Application\nName={}\nComment={comment}\nExec={exec}\nIcon={icon}\n\
             X-Syndroid-Instance={instance}\nX-Syndroid-Title={title}\n{DESKTOP_MARK}\n",
            name.replace('\n', " ")
        )
    };
    let mut keep = HashSet::new();
    // Весь Android одним окном — тоже в разделе этого экземпляра
    // Окно всего Android hwcomposer образа называет «Waydroid» (app_id и заголовок зашиты в HAL) — по
    // StartupWMClass оно сопоставляется с этим ярлыком: значок и имя syndroid
    let full = head("Весь Android", &format!("{title} одним окном"), &format!("syndroid show --instance {instance}"), "syndroid");
    write_if_changed(&dir.join("android.desktop"), &format!("{full}Categories=System;\nStartupWMClass=Waydroid\n"))?;
    keep.insert("android.desktop".to_string());
    for a in apps {
        let file = format!("{}.desktop", a.package);
        let src = icons.join(format!("{}.png", a.package));
        let dst = my_icons.join(format!("{}.png", a.package));
        if let Ok(png) = std::fs::read(&src) {
            // Целый PNG: сигнатура в начале и блок IEND в конце (Android мог ещё не дописать)
            if png.starts_with(b"\x89PNG") && png.ends_with(b"IEND\xaeB`\x82") && std::fs::read(&dst).ok().as_deref() != Some(png.as_slice()) {
                let tmp = dst.with_extension("png.tmp");
                std::fs::write(&tmp, &png)?;
                std::fs::rename(&tmp, &dst)?;
            }
        }
        let icon = if dst.exists() { dst.display().to_string() } else { "syndroid".into() };
        let text = format!(
            "{}Categories={}\nStartupWMClass=waydroid.{}\nX-Android-Package={}\n",
            head(&a.name, &format!("Android · {title}"), &format!("syndroid app launch --instance {instance} {}", a.package), &icon),
            xdg_categories(&a.categories),
            a.package,
            a.package,
        );
        write_if_changed(&dir.join(&file), &text)?;
        keep.insert(file);
    }
    // Удалённые из Android — убрать (только наши)
    for e in std::fs::read_dir(&dir)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(".desktop")
            && !keep.contains(&name)
            && std::fs::read_to_string(e.path()).is_ok_and(|t| t.contains(DESKTOP_MARK))
        {
            let _ = std::fs::remove_file(e.path());
        }
    }
    Ok(())
}

/// Убрать ярлыки и значки экземпляров, у которых больше нет образов (удалены набор или весь
/// экземпляр): демон работает под root и в домашний каталог не пишет — чистит процесс пользователя
/// (окно syndroid после удаления, `syndroid session`). Данные экземпляра не трогаются: при новых
/// образах мост снова напишет ярлыки. Демон недоступен — ничего не делать.
pub fn prune_entries() {
    let Ok(crate::api::Response::Instances { instances }) = crate::api::call(&crate::api::Request::Instances) else { return };
    let live: HashSet<String> = instances.into_iter().filter(|i| !i.sets.is_empty()).map(|i| i.id).collect();
    for sub in ["syndroid/applications", "syndroid/icons"] {
        for e in std::fs::read_dir(data_home().join(sub)).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.path().is_dir() && !live.contains(&name) {
                tracing::info!("ярлыки экземпляра {name} без образов — удалены");
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}

/// Ярлыки прежних версий syndroid лежали среди программ Linux (`applications/waydroid.*.desktop`) — убрать.
fn remove_legacy_entries() {
    let dir = data_home().join("applications");
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("waydroid.") && std::fs::read_to_string(e.path()).is_ok_and(|t| t.contains(DESKTOP_MARK)) {
            let _ = std::fs::remove_file(e.path());
        }
    }
    for e in std::fs::read_dir(data_home().join("syndroid/icons")).into_iter().flatten().flatten() {
        if e.path().extension().is_some_and(|x| x == "png") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

fn sync_apps(instance: &str) -> Result<usize> {
    let platform = check_service("waydroidplatform")?.context("нет waydroidplatform")?;
    let apps = apps_info(&platform)?;
    write_desktop_files(instance, &apps)?;
    Ok(apps.len())
}

// --- главный цикл --------------------------------------------------------------------------------

fn wait_for<T>(what: &str, timeout: Duration, mut f: impl FnMut() -> Result<Option<T>>) -> Result<T> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match f() {
            Ok(Some(v)) => return Ok(v),
            Ok(None) | Err(_) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_secs(1)),
            Ok(None) => bail!("не дождались: {what}"),
            Err(e) => return Err(e.context(format!("не дождались: {what}"))),
        }
    }
}

fn run(instance: &str) -> Result<()> {
    let node = PathBuf::from(paths::BINDERFS).join(container::BINDER_NODES[0].0);
    ProcessState::init(&node.to_string_lossy(), 4).map_err(|e| anyhow!("{}: {e}", node.display()))?;
    ProcessState::start_thread_pool();
    let (tx, rx) = channel();

    // servicemanager Android готов — регистрируем сервисы хоста
    wait_for("servicemanager", Duration::from_secs(120), || Ok(check_service("manager").ok().map(|_| ())))?;
    let dbus = zbus::blocking::Connection::session().map_err(|e| tracing::warn!("D-Bus сеанса: {e}")).ok();
    let listeners = Arc::new(Mutex::new(Vec::new()));
    let ids = Arc::new(Mutex::new(HashSet::new()));
    if let Some(c) = &dbus {
        forward_actions(c.clone(), listeners.clone(), ids.clone());
    }
    let services: Vec<(&str, SIBinder)> = vec![
        ("waydroidusermonitor", Binder::new(UserMonitor(Mutex::new(tx.clone()))).as_binder()),
        ("waydroidhardware", Binder::new(Hardware(Mutex::new(tx.clone()))).as_binder()),
        ("waydroidnotifications", Binder::new(Notifications { dbus, listeners, ids }).as_binder()),
    ];
    for (name, b) in &services {
        add_service(name, b.clone()).with_context(|| format!("addService {name}"))?;
    }
    tracing::info!("мост: сервисы хоста зарегистрированы");

    // system_server поднял waydroidplatform — первый список приложений
    wait_for("waydroidplatform", Duration::from_secs(180), || check_service("waydroidplatform"))?;
    remove_legacy_entries();
    match sync_apps(instance) {
        Ok(n) => tracing::info!("мост: ярлыков приложений Android: {n}"),
        Err(e) => tracing::warn!("мост: приложения: {e:#}"),
    }

    loop {
        let ev = rx.recv()?;
        match ev {
            Event::AppsChanged => {
                // Пакеты меняются пачками — подождать, пока утихнет
                std::thread::sleep(Duration::from_millis(800));
                while rx.try_recv().is_ok_and(|e| matches!(e, Event::AppsChanged)) {}
                if let Err(e) = sync_apps(instance) {
                    tracing::warn!("мост: приложения: {e:#}");
                }
            }
            // Демон по этим запросам перезапускает/останавливает и мост — отвечаем Android сначала
            Event::Suspend => drop(std::thread::spawn(|| api::call(&Request::Freeze))),
            Event::Reboot => drop(std::thread::spawn(|| api::call(&Request::Restart))),
            Event::Shutdown => drop(std::thread::spawn(|| api::call(&Request::Stop))),
        }
    }
}

/// `syndroidd __bridge <экземпляр>`
pub fn bridge_main(args: &[String]) -> ! {
    let instance = args.first().cloned().unwrap_or_default();
    match run(&instance) {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("syndroid: мост: {e:#}");
            std::process::exit(1)
        }
    }
}
