//! Перенос сетей Wi-Fi с компьютера на телефон.
//!
//! Начинает телефон: сразу после нового спаривания с компьютером (или по
//! `synlink wifi-import`, кнопке в Параметрах) он спрашивает у компьютера
//! список сетей без секретов (`Rpc::WifiList`), убирает те, что у него уже
//! есть, и спрашивает пользователя у себя: «Перенести сети Wi-Fi с
//! компьютера?» со списком имён (`PairPrompt::wifi`). Только после
//! «Разрешить» он просит пароли (`Rpc::WifiExport`) — по тому же QUIC с
//! закреплёнными при спаривании ключами — и создаёт профили NetworkManager.
//!
//! Где компьютер берёт сети: NetworkManager — D-Bus (`GetSettings`, пароли —
//! `GetSecrets`, права на них даёт polkit), iwd — список по D-Bus
//! (`KnownNetwork`), пароли — файлы `/var/lib/iwd/*.psk`, читать которые может
//! только root: их читает `synlink wifi-secrets` через `pkexec` (пароль
//! администратора спрашивает агент polkit оболочки компьютера). Переносятся
//! открытые сети и WPA/WPA2/WPA3 с паролем; 802.1X (сертификаты, учётные
//! записи) и WEP — нет, телефон показывает их в списке с пояснением.
//!
//! Пароли не пишутся в журнал и не ложатся на диск компьютера: из файла iwd —
//! в память помощника, по каналу (`stdout`) — демону, оттуда — телефону.
//!
//! Проверка без настоящих сетей: `SYNLINK_WIFI_DIR=КАТАЛОГ` — сети этой машины
//! берутся из файлов формата iwd в каталоге (без root и D-Bus), перенесённые
//! пишутся туда же.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use synshell_common::link::{PairPrompt, WifiBrief, WifiSecurity};
use synshell_tr::t;
use tokio::sync::oneshot;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zeroize::Zeroize;

use crate::daemon::D;
use crate::proto::{self, Reply, Rpc};

const IWD_DIR: &str = "/var/lib/iwd";
const NM: &str = "org.freedesktop.NetworkManager";
const IWD: &str = "net.connman.iwd";

/// Сеть с паролем. Пароль стирается из памяти, когда сеть больше не нужна.
#[derive(Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct WifiNet {
    pub ssid: String,
    pub security: WifiSecurity,
    #[serde(default)]
    pub hidden: bool,
    /// Парольная фраза или 64 шестнадцатеричных знака (PSK).
    #[serde(default)]
    pub psk: Option<String>,
}

impl Drop for WifiNet {
    fn drop(&mut self) {
        if let Some(p) = self.psk.as_mut() {
            p.zeroize();
        }
    }
}

/// Без пароля — чтобы он не попал в журнал через `{:?}`.
impl std::fmt::Debug for WifiNet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WifiNet").field("ssid", &self.ssid).field("security", &self.security).field("hidden", &self.hidden).finish_non_exhaustive()
    }
}

impl WifiNet {
    pub fn brief(&self) -> WifiBrief {
        WifiBrief { ssid: self.ssid.clone(), security: self.security, hidden: self.hidden }
    }
}

/// Ответ компьютера на `Rpc::WifiExport`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Export {
    pub nets: Vec<WifiNet>,
    /// Не отданы: сеть и почему.
    pub skipped: Vec<(String, String)>,
}

fn test_dir() -> Option<PathBuf> {
    std::env::var_os("SYNLINK_WIFI_DIR").filter(|v| !v.is_empty()).map(PathBuf::from)
}

// ─── формат iwd ─────────────────────────────────────────────────────────

/// Имя файла сети у iwd (без расширения): само имя, если в нём только латиница, цифры, `-`, `_` и
/// пробел, иначе `=` и шестнадцатеричная запись байтов.
pub fn iwd_file_stem(ssid: &str) -> String {
    if !ssid.is_empty() && ssid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b' ') {
        ssid.to_string()
    } else {
        let mut s = String::from("=");
        for b in ssid.bytes() {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }
}

/// Имя сети по имени файла iwd.
pub fn iwd_ssid(stem: &str) -> Option<String> {
    let Some(hex) = stem.strip_prefix('=') else { return Some(stem.to_string()) };
    if hex.len() % 2 != 0 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect();
    String::from_utf8(bytes?).ok()
}

/// Значение ключа файла настроек (ell/glib): `\s` `\n` `\t` `\r` `\\`.
fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn escape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for (i, c) in v.chars().enumerate() {
        match c {
            ' ' if i == 0 => out.push_str("\\s"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    // Пробел в конце тоже срезался бы при чтении.
    if out.ends_with(' ') {
        out.pop();
        out.push_str("\\s");
    }
    out
}

/// Файл настроек: (раздел, ключ) → значение.
fn parse_ini(text: &str) -> HashMap<(String, String), String> {
    let mut out = HashMap::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.trim().to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.insert((section.clone(), k.trim().to_string()), unescape(v.trim()));
        }
    }
    out
}

/// Сеть из файла iwd: `stem` — имя файла без расширения, `ext` — `psk`, `open`, `8021x`.
pub fn iwd_parse(stem: &str, ext: &str, text: &str) -> Option<WifiNet> {
    let ssid = iwd_ssid(stem)?;
    let security = match ext {
        "psk" => WifiSecurity::Psk,
        "open" => WifiSecurity::Open,
        "8021x" => WifiSecurity::Enterprise,
        "wep" => WifiSecurity::Wep,
        _ => return None,
    };
    let mut ini = parse_ini(text);
    let get = |ini: &mut HashMap<(String, String), String>, s: &str, k: &str| ini.remove(&(s.to_string(), k.to_string()));
    let hidden = get(&mut ini, "Settings", "Hidden").is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let psk = if security == WifiSecurity::Psk {
        // Парольная фраза; если её нет — готовый ключ (64 hex), NetworkManager примет и его.
        get(&mut ini, "Security", "Passphrase")
            .filter(|p| !p.is_empty())
            .or_else(|| get(&mut ini, "Security", "PreSharedKey").filter(|k| k.len() == 64 && k.bytes().all(|b| b.is_ascii_hexdigit())))
    } else {
        None
    };
    // Остатки разбора (в том числе ключи SAE) — стереть.
    for (_, mut v) in ini.drain() {
        v.zeroize();
    }
    Some(WifiNet { ssid, security, hidden, psk })
}

/// Файл iwd для сети (для каталога проверки).
pub fn iwd_render(n: &WifiNet) -> (String, String) {
    let ext = match n.security {
        WifiSecurity::Open => "open",
        WifiSecurity::Enterprise => "8021x",
        WifiSecurity::Wep => "wep",
        WifiSecurity::Psk | WifiSecurity::Sae => "psk",
    };
    let mut text = String::new();
    if let Some(p) = &n.psk {
        text.push_str(&format!("[Security]\nPassphrase={}\n", escape(p)));
    }
    if n.hidden {
        text.push_str("[Settings]\nHidden=true\n");
    }
    (format!("{}.{ext}", iwd_file_stem(&n.ssid)), text)
}

/// Сети из каталога iwd (`wanted` — только эти).
pub fn iwd_dir_read(dir: &Path, wanted: Option<&[String]>) -> Result<Vec<WifiNet>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).with_context(|| dir.display().to_string())?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some((stem, ext)) = name.rsplit_once('.') else { continue };
        if !matches!(ext, "psk" | "open" | "8021x") {
            continue;
        }
        if wanted.is_some_and(|w| !iwd_ssid(stem).is_some_and(|s| w.contains(&s))) {
            continue;
        }
        // Пароль не нужен — файл не читаем (список по D-Bus этого не требует, а здесь — каталог проверки).
        let mut text = std::fs::read_to_string(e.path()).unwrap_or_default();
        if let Some(n) = iwd_parse(stem, ext, &text) {
            out.push(n);
        }
        text.zeroize();
    }
    out.sort_by(|a, b| a.ssid.cmp(&b.ssid));
    Ok(out)
}

// ─── D-Bus ──────────────────────────────────────────────────────────────

type Settings = HashMap<String, HashMap<String, OwnedValue>>;

fn has_name(conn: &Connection, name: &str) -> bool {
    conn.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &(name,))
        .ok()
        .and_then(|m| m.body().deserialize::<bool>().ok())
        .unwrap_or(false)
}

fn v_str(m: &HashMap<String, OwnedValue>, k: &str) -> Option<String> {
    m.get(k).and_then(|v| String::try_from(v.try_clone().ok()?).ok())
}

fn v_bool(m: &HashMap<String, OwnedValue>, k: &str) -> Option<bool> {
    m.get(k).and_then(|v| bool::try_from(v).ok())
}

fn v_bytes(m: &HashMap<String, OwnedValue>, k: &str) -> Option<Vec<u8>> {
    m.get(k).and_then(|v| Vec::<u8>::try_from(v.try_clone().ok()?).ok())
}

/// Сеть Wi-Fi из настроек подключения NetworkManager (`GetSettings`); пароль — из `secrets`
/// (`GetSecrets("802-11-wireless-security")`), если дали.
pub fn nm_net(s: &Settings, secrets: Option<&Settings>) -> Option<WifiNet> {
    let conn = s.get("connection")?;
    if v_str(conn, "type").as_deref() != Some("802-11-wireless") {
        return None;
    }
    let wifi = s.get("802-11-wireless")?;
    // Точка доступа и Wi-Fi Direct с этого компьютера телефону не нужны.
    if v_str(wifi, "mode").is_some_and(|m| m != "infrastructure") {
        return None;
    }
    let ssid = String::from_utf8(v_bytes(wifi, "ssid")?).ok().filter(|s| !s.is_empty())?;
    let hidden = v_bool(wifi, "hidden").unwrap_or(false);
    let sec = s.get("802-11-wireless-security");
    let security = match sec.and_then(|x| v_str(x, "key-mgmt")).as_deref() {
        None => WifiSecurity::Open,
        Some("wpa-psk") => WifiSecurity::Psk,
        Some("sae") => WifiSecurity::Sae,
        // OWE — открытая с шифрованием: телефон подключится сам, как к открытой.
        Some("owe") => WifiSecurity::Open,
        Some("none" | "ieee8021x") => WifiSecurity::Wep,
        Some(_) => WifiSecurity::Enterprise,
    };
    let psk = match security {
        WifiSecurity::Psk | WifiSecurity::Sae => {
            secrets.and_then(|x| x.get("802-11-wireless-security")).and_then(|x| v_str(x, "psk")).filter(|p| !p.is_empty())
        }
        _ => None,
    };
    Some(WifiNet { ssid, security, hidden, psk })
}

/// Подключения Wi-Fi NetworkManager: путь и сеть (без пароля).
fn nm_connections(conn: &Connection) -> Result<Vec<(OwnedObjectPath, WifiNet)>> {
    let m = conn.call_method(Some(NM), "/org/freedesktop/NetworkManager/Settings", Some("org.freedesktop.NetworkManager.Settings"), "ListConnections", &())?;
    let paths: Vec<OwnedObjectPath> = m.body().deserialize()?;
    let mut out = Vec::new();
    for p in paths {
        let Ok(m) = conn.call_method(Some(NM), p.as_str(), Some("org.freedesktop.NetworkManager.Settings.Connection"), "GetSettings", &()) else {
            continue;
        };
        let Ok(s) = m.body().deserialize::<Settings>() else { continue };
        if let Some(n) = nm_net(&s, None) {
            out.push((p, n));
        }
    }
    Ok(out)
}

/// Пароль подключения NetworkManager. Отдаёт только то, что хранит сам NM (не связка ключей агента).
fn nm_psk(conn: &Connection, path: &OwnedObjectPath) -> Result<Option<String>> {
    let m = conn.call_method(Some(NM), path.as_str(), Some("org.freedesktop.NetworkManager.Settings.Connection"), "GetSecrets", &("802-11-wireless-security",))?;
    let s: Settings = m.body().deserialize()?;
    Ok(s.get("802-11-wireless-security").and_then(|x| v_str(x, "psk")).filter(|p| !p.is_empty()))
}

/// Известные сети iwd (`KnownNetwork`): D-Bus iwd открыт группам wheel и network.
fn iwd_known(conn: &Connection) -> Result<Vec<WifiBrief>> {
    type Objects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;
    let m = conn.call_method(Some(IWD), "/", Some("org.freedesktop.DBus.ObjectManager"), "GetManagedObjects", &())?;
    let objs: Objects = m.body().deserialize()?;
    let mut out = Vec::new();
    for ifs in objs.values() {
        let Some(k) = ifs.get("net.connman.iwd.KnownNetwork") else { continue };
        let Some(ssid) = v_str(k, "Name") else { continue };
        let security = match v_str(k, "Type").as_deref() {
            Some("open") => WifiSecurity::Open,
            Some("psk") => WifiSecurity::Psk,
            Some("wep") => WifiSecurity::Wep,
            _ => WifiSecurity::Enterprise,
        };
        out.push(WifiBrief { ssid, security, hidden: v_bool(k, "Hidden").unwrap_or(false) });
    }
    Ok(out)
}

// ─── компьютер: что отдать ──────────────────────────────────────────────

/// Сети этой машины без паролей (`Rpc::WifiList`).
pub fn source_list() -> Result<Vec<WifiBrief>> {
    if let Some(dir) = test_dir() {
        return Ok(iwd_dir_read(&dir, None)?.iter().map(WifiNet::brief).collect());
    }
    let conn = Connection::system().context("D-Bus")?;
    let mut out: Vec<WifiBrief> = Vec::new();
    if has_name(&conn, NM) {
        match nm_connections(&conn) {
            Ok(v) => out.extend(v.iter().map(|(_, n)| n.brief())),
            Err(e) => tracing::warn!("сети NetworkManager: {e:#}"),
        }
    }
    if has_name(&conn, IWD) {
        match iwd_known(&conn) {
            Ok(v) => out.extend(v),
            Err(e) => tracing::warn!("сети iwd: {e:#}"),
        }
    }
    // NetworkManager поверх iwd: одна сеть видна дважды.
    let mut seen = HashSet::new();
    out.retain(|n| seen.insert(n.ssid.clone()));
    out.sort_by_key(|n| n.ssid.to_lowercase());
    Ok(out)
}

/// Сети с паролями (`Rpc::WifiExport`): NetworkManager — по D-Bus, iwd — помощником через pkexec.
pub fn export(ssids: &[String]) -> Result<Export> {
    let mut ex = Export::default();
    let mut rest: Vec<String> = ssids.to_vec();
    let take = |ex: &mut Export, rest: &mut Vec<String>, n: WifiNet| {
        if !rest.contains(&n.ssid) {
            return;
        }
        rest.retain(|s| *s != n.ssid);
        if !n.security.transferable() {
            ex.skipped.push((n.ssid.clone(), skip_reason(n.security)));
        } else if n.security != WifiSecurity::Open && n.psk.is_none() {
            ex.skipped.push((n.ssid.clone(), t!("пароль не сохранён на компьютере")));
        } else {
            ex.nets.push(n);
        }
    };
    if let Some(dir) = test_dir() {
        for n in iwd_dir_read(&dir, Some(ssids))? {
            take(&mut ex, &mut rest, n);
        }
    } else {
        let conn = Connection::system().context("D-Bus")?;
        if has_name(&conn, NM) {
            for (path, mut n) in nm_connections(&conn).unwrap_or_default() {
                if !rest.contains(&n.ssid) {
                    continue;
                }
                if matches!(n.security, WifiSecurity::Psk | WifiSecurity::Sae) {
                    match nm_psk(&conn, &path) {
                        Ok(p) => n.psk = p,
                        Err(e) => tracing::warn!(ssid = %n.ssid, "пароль сети NetworkManager: {e:#}"),
                    }
                    // Пароль у агента (связка ключей пользователя) — пусть попробует iwd, если и он знает сеть.
                    if n.psk.is_none() {
                        continue;
                    }
                }
                take(&mut ex, &mut rest, n);
            }
        }
        let iwd_wants: Vec<String> = if has_name(&conn, IWD) {
            match iwd_known(&conn) {
                Ok(k) => rest.iter().filter(|s| k.iter().any(|n| &n.ssid == *s)).cloned().collect(),
                // Списка не дали (нет группы) — помощник посмотрит сам.
                Err(_) => rest.clone(),
            }
        } else {
            Vec::new()
        };
        if !iwd_wants.is_empty() {
            for n in iwd_secrets_pkexec(&iwd_wants)? {
                take(&mut ex, &mut rest, n);
            }
        }
    }
    for s in rest {
        ex.skipped.push((s, t!("на компьютере такой сети нет")));
    }
    Ok(ex)
}

fn skip_reason(s: WifiSecurity) -> String {
    match s {
        WifiSecurity::Enterprise => t!("802.1X: сертификаты и учётная запись не переносятся"),
        WifiSecurity::Wep => t!("WEP не переносится"),
        _ => String::new(),
    }
}

/// Пароли сетей iwd: `pkexec synlink wifi-secrets ИМЯ…` (агент polkit оболочки спросит пароль администратора).
fn iwd_secrets_pkexec(ssids: &[String]) -> Result<Vec<WifiNet>> {
    use std::process::{Command, Stdio};
    let exe = crate::self_exe();
    tracing::info!(n = ssids.len(), "пароли сетей iwd — через pkexec");
    let out = Command::new("pkexec")
        .arg(&exe)
        .arg("wifi-secrets")
        .arg("--")
        .args(ssids)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("pkexec")?;
    let mut stdout = out.stdout;
    let res = match out.status.code() {
        Some(0) => serde_json::from_slice::<Vec<WifiNet>>(&stdout).context("wifi-secrets"),
        // 126 — пароль не ввели или отказали, 127 — нет агента polkit.
        Some(126) => Err(anyhow::anyhow!("{}", t!("на компьютере не подтвердили пароль администратора"))),
        Some(127) => Err(anyhow::anyhow!("{}", t!("на компьютере некому спросить пароль администратора (нет агента polkit)"))),
        _ => Err(anyhow::anyhow!("{}", t!("помощник wifi-secrets: {e}", e = String::from_utf8_lossy(&out.stderr).trim()))),
    };
    stdout.zeroize();
    res
}

/// `synlink wifi-secrets [--] ИМЯ…` от root (через pkexec): сети iwd с паролями — JSON в stdout.
/// Читает только каталог iwd и только названные сети.
pub fn secrets_helper(args: &[String]) -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        bail!("{}", t!("нужны права root (запускается через pkexec)"));
    }
    let wanted: Vec<String> = args.iter().filter(|a| a.as_str() != "--").cloned().collect();
    if wanted.is_empty() {
        bail!("{}", t!("нужны имена сетей"));
    }
    let nets = iwd_dir_read(Path::new(IWD_DIR), Some(&wanted))?;
    let mut json = serde_json::to_vec(&nets)?;
    use std::io::Write;
    let r = std::io::stdout().write_all(&json);
    json.zeroize();
    r?;
    Ok(())
}

// ─── телефон: куда положить ─────────────────────────────────────────────

/// Имена сетей, которые здесь уже есть.
fn local_known() -> HashSet<String> {
    if let Some(dir) = test_dir() {
        return iwd_dir_read(&dir, None).unwrap_or_default().iter().map(|n| n.ssid.clone()).collect();
    }
    let Ok(conn) = Connection::system() else { return HashSet::new() };
    let mut out: HashSet<String> = HashSet::new();
    if has_name(&conn, NM) {
        out.extend(nm_connections(&conn).unwrap_or_default().into_iter().map(|(_, n)| n.ssid.clone()));
    }
    if has_name(&conn, IWD) {
        out.extend(iwd_known(&conn).unwrap_or_default().into_iter().map(|n| n.ssid));
    }
    out
}

/// Точка доступа с этим именем видна и умеет только WPA3-SAE (профилю нужен `sae`, а не `wpa-psk`).
fn nm_sae_only(conn: &Connection, ssid: &str) -> bool {
    let get = |path: &str, iface: &str, prop: &str| -> Option<OwnedValue> {
        let m = conn.call_method(Some(NM), path, Some("org.freedesktop.DBus.Properties"), "Get", &(iface, prop)).ok()?;
        m.body().deserialize::<OwnedValue>().ok()
    };
    let Some(devs) = conn
        .call_method(Some(NM), "/org/freedesktop/NetworkManager", Some(NM), "GetDevices", &())
        .ok()
        .and_then(|m| m.body().deserialize::<Vec<OwnedObjectPath>>().ok())
    else {
        return false;
    };
    let (mut sae, mut psk) = (false, false);
    for d in devs {
        // 2 — Wi-Fi.
        if get(d.as_str(), "org.freedesktop.NetworkManager.Device", "DeviceType").and_then(|v| u32::try_from(v).ok()) != Some(2) {
            continue;
        }
        let aps = get(d.as_str(), "org.freedesktop.NetworkManager.Device.Wireless", "AccessPoints")
            .and_then(|v| Vec::<OwnedObjectPath>::try_from(v).ok())
            .unwrap_or_default();
        for ap in aps {
            let name = get(ap.as_str(), "org.freedesktop.NetworkManager.AccessPoint", "Ssid").and_then(|v| Vec::<u8>::try_from(v).ok());
            if name.as_deref() != Some(ssid.as_bytes()) {
                continue;
            }
            let rsn = get(ap.as_str(), "org.freedesktop.NetworkManager.AccessPoint", "RsnFlags").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
            let wpa = get(ap.as_str(), "org.freedesktop.NetworkManager.AccessPoint", "WpaFlags").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
            // NM_802_11_AP_SEC_KEY_MGMT_PSK = 0x100, _SAE = 0x400.
            sae |= rsn & 0x400 != 0;
            psk |= (rsn | wpa) & 0x100 != 0;
        }
    }
    sae && !psk
}

/// Настройки нового подключения NetworkManager для сети.
fn nm_settings(n: &WifiNet, sae: bool) -> HashMap<&'static str, HashMap<&'static str, Value<'static>>> {
    let mut s: HashMap<&'static str, HashMap<&'static str, Value<'static>>> = HashMap::new();
    let mut c = HashMap::new();
    c.insert("id", Value::from(n.ssid.clone()));
    c.insert("uuid", Value::from(uuid4()));
    c.insert("type", Value::from("802-11-wireless"));
    c.insert("autoconnect", Value::from(true));
    s.insert("connection", c);
    let mut w = HashMap::new();
    w.insert("ssid", Value::from(n.ssid.as_bytes().to_vec()));
    w.insert("mode", Value::from("infrastructure"));
    if n.hidden {
        w.insert("hidden", Value::from(true));
    }
    s.insert("802-11-wireless", w);
    if let (true, Some(p)) = (n.security != WifiSecurity::Open, &n.psk) {
        let mut sec = HashMap::new();
        let km = if n.security == WifiSecurity::Sae || sae { "sae" } else { "wpa-psk" };
        sec.insert("key-mgmt", Value::from(km));
        sec.insert("psk", Value::from(p.clone()));
        s.insert("802-11-wireless-security", sec);
    }
    for ip in ["ipv4", "ipv6"] {
        let mut m = HashMap::new();
        m.insert("method", Value::from("auto"));
        s.insert(ip, m);
    }
    s
}

fn uuid4() -> String {
    use std::io::Read;
    let mut b = [0u8; 16];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_err() {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        b = (t ^ ((std::process::id() as u128) << 64)).to_le_bytes();
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// Создать профили сетей здесь: (добавлены, не добавлены с причиной).
fn import(nets: &[WifiNet]) -> (Vec<String>, Vec<(String, String)>) {
    let (mut ok, mut bad) = (Vec::new(), Vec::new());
    if let Some(dir) = test_dir() {
        for n in nets {
            let (name, mut text) = iwd_render(n);
            let path = dir.join(name);
            let r = (|| -> std::io::Result<()> {
                use std::io::Write;
                use std::os::unix::fs::OpenOptionsExt;
                let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
                f.write_all(text.as_bytes())
            })();
            text.zeroize();
            match r {
                Ok(()) => ok.push(n.ssid.clone()),
                Err(e) => bad.push((n.ssid.clone(), e.to_string())),
            }
        }
        return (ok, bad);
    }
    let conn = match Connection::system() {
        Ok(c) if has_name(&c, NM) => c,
        _ => {
            for n in nets {
                bad.push((n.ssid.clone(), t!("здесь нет NetworkManager")));
            }
            return (ok, bad);
        }
    };
    let known: HashSet<String> = nm_connections(&conn).unwrap_or_default().into_iter().map(|(_, n)| n.ssid.clone()).collect();
    for n in nets {
        if known.contains(&n.ssid) {
            bad.push((n.ssid.clone(), t!("уже есть")));
            continue;
        }
        let sae = n.security == WifiSecurity::Psk && nm_sae_only(&conn, &n.ssid);
        let s = nm_settings(n, sae);
        let r = conn.call_method(Some(NM), "/org/freedesktop/NetworkManager/Settings", Some("org.freedesktop.NetworkManager.Settings"), "AddConnection", &(s,));
        match r {
            Ok(_) => ok.push(n.ssid.clone()),
            Err(e) => bad.push((n.ssid.clone(), e.to_string())),
        }
    }
    (ok, bad)
}

// ─── ход переноса (телефон) ─────────────────────────────────────────────

/// Id запроса согласия в `Status::prompts`.
pub fn prompt_id(peer: &str) -> String {
    format!("wifi:{peer}")
}

/// Предложить перенос сетей с компьютера `peer` и, если здесь согласились, перенести.
/// `manual` — попросил пользователь (CLI, Параметры): тогда «нечего переносить» — ответ, а не молчание.
pub async fn offer(d: D, peer: String, manual: bool) -> Result<String> {
    if !d.kind.is_touch() {
        bail!("{}", t!("сети Wi-Fi переносятся с компьютера на телефон — запустите перенос на телефоне"));
    }
    let (conn, trusted) = d.conn(&peer).with_context(|| t!("устройство не соединено"))?;
    if trusted.kind.is_touch() {
        bail!("{}", t!("«{name}» — не компьютер", name = trusted.name));
    }
    let (proto_v, transport) = d.session_info(&peer).with_context(|| t!("устройство не соединено"))?;
    if proto_v < proto::PROTO_WIFI {
        bail!("{}", t!("на «{name}» synlink без переноса сетей Wi-Fi — обновите synshell там", name = trusted.name));
    }
    let id = prompt_id(&peer);
    if d.prompt_pending(&id) {
        bail!("{}", t!("вопрос о переносе сетей уже ждёт ответа на экране"));
    }
    let list: Vec<WifiBrief> = match crate::rpc::call(&conn, &Rpc::WifiList).await? {
        Reply::Json(s) => serde_json::from_str(&s)?,
        _ => bail!("{}", t!("неожиданный ответ")),
    };
    let known = tokio::task::spawn_blocking(local_known).await?;
    let new: Vec<WifiBrief> = list.into_iter().filter(|n| !known.contains(&n.ssid)).collect();
    if !new.iter().any(|n| n.security.transferable()) {
        tracing::info!(peer = %trusted.name, offered = new.len(), "новых сетей Wi-Fi для переноса нет");
        if manual {
            return Ok(if new.is_empty() {
                t!("новых сетей Wi-Fi на «{name}» нет — все уже здесь", name = trusted.name)
            } else {
                t!("на «{name}» только сети, которые не переносятся (802.1X, WEP)", name = trusted.name)
            });
        }
        return Ok(String::new());
    }
    tracing::info!(peer = %trusted.name, n = new.len(), "спрашиваю, перенести ли сети Wi-Fi");
    let (wtx, wrx) = oneshot::channel();
    d.add_prompt(
        PairPrompt {
            id: id.clone(),
            name: trusted.name.clone(),
            kind: trusted.kind,
            transport,
            code: None,
            outgoing: true,
            wifi: Some(new.clone()),
        },
        Some(wtx),
    );
    let ok = tokio::select! {
        r = wrx => r.unwrap_or(false),
        _ = tokio::time::sleep(Duration::from_secs(180)) => false,
        _ = conn.closed() => false,
    };
    d.drop_prompt(&id);
    if !ok {
        tracing::info!(peer = %trusted.name, "перенос сетей Wi-Fi не разрешён");
        return Ok(t!("перенос сетей Wi-Fi не разрешён"));
    }
    let ssids: Vec<String> = new.iter().filter(|n| n.security.transferable()).map(|n| n.ssid.clone()).collect();
    crate::notify::info(
        &t!("Сети Wi-Fi"),
        &t!("Получаю сети с «{name}»… На компьютере может понадобиться пароль администратора.", name = trusted.name),
    );
    // Компьютер может ждать пароль администратора — не торопить.
    let reply = tokio::time::timeout(Duration::from_secs(300), crate::rpc::call(&conn, &Rpc::WifiExport { ssids: ssids.clone() }))
        .await
        .with_context(|| t!("компьютер не ответил вовремя"));
    let ex: Export = match reply {
        Ok(Ok(Reply::Json(mut s))) => {
            let ex = serde_json::from_str(&s);
            s.zeroize();
            ex?
        }
        Ok(Ok(_)) => bail!("{}", t!("неожиданный ответ")),
        Ok(Err(e)) | Err(e) => {
            let m = t!("сети Wi-Fi не получены: {e}", e = format!("{e:#}"));
            crate::notify::info(&t!("Сети Wi-Fi"), &m);
            bail!("{m}");
        }
    };
    let (added, failed) = tokio::task::spawn_blocking(move || {
        let mut ex = ex;
        let (a, mut f) = import(&ex.nets);
        f.append(&mut ex.skipped);
        (a, f)
    })
    .await?;
    tracing::info!(peer = %trusted.name, added = added.len(), failed = failed.len(), "сети Wi-Fi перенесены");
    let mut text = t!("Перенесено сетей Wi-Fi с «{name}»: {n}", name = trusted.name, n = added.len());
    if !added.is_empty() {
        text.push_str(&format!(" ({})", added.join(", ")));
    }
    for (s, why) in &failed {
        text.push_str(&format!("\n{s}: {why}"));
    }
    crate::notify::info(&t!("Сети Wi-Fi"), &text);
    Ok(text)
}

/// После нового спаривания с компьютером: предложить перенос, если есть что.
pub fn offer_after_pairing(d: D, peer: String) {
    tokio::spawn(async move {
        // Дать диалогу спаривания закрыться.
        tokio::time::sleep(Duration::from_secs(1)).await;
        if let Err(e) = offer(d, peer, false).await {
            tracing::info!("перенос сетей Wi-Fi: {e:#}");
        }
    });
}

/// Компьютер: телефон просит сети (`Rpc::WifiList`, `Rpc::WifiExport`).
pub async fn serve(d: &D, peer: &str, req: Rpc) -> Reply {
    let name = d.trusted(peer).map(|t| t.name).unwrap_or_else(|| peer.to_string());
    let res = match req {
        Rpc::WifiList => tokio::task::spawn_blocking(source_list).await.map_err(anyhow::Error::from).and_then(|r| r).and_then(|v| Ok(serde_json::to_string(&v)?)),
        Rpc::WifiExport { ssids } => {
            tracing::info!(peer = %name, n = ssids.len(), "устройство просит сети Wi-Fi с паролями");
            let r = tokio::task::spawn_blocking(move || export(&ssids)).await.map_err(anyhow::Error::from).and_then(|r| r);
            match r {
                Ok(ex) => {
                    tracing::info!(peer = %name, sent = ex.nets.len(), skipped = ex.skipped.len(), "сети Wi-Fi отданы");
                    if !ex.nets.is_empty() {
                        let names: Vec<&str> = ex.nets.iter().map(|n| n.ssid.as_str()).collect();
                        crate::notify::info(&t!("Сети Wi-Fi"), &t!("Переданы на «{name}»: {list}", name = name, list = names.join(", ")));
                    }
                    serde_json::to_string(&ex).map_err(Into::into)
                }
                Err(e) => Err(e),
            }
        }
        _ => Err(anyhow::anyhow!("{}", t!("неожиданный ответ"))),
    };
    match res {
        Ok(s) => Reply::Json(s),
        Err(e) => {
            tracing::info!(peer = %name, "сети Wi-Fi: {e:#}");
            Reply::Err(format!("{e:#}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iwd_names() {
        assert_eq!(iwd_file_stem("Home Net_5G-2"), "Home Net_5G-2");
        assert_eq!(iwd_file_stem("Кафе"), "=d09ad0b0d184d0b5");
        assert_eq!(iwd_file_stem("a.b"), "=612e62");
        for s in ["Home Net_5G-2", "Кафе", "a.b", "x=y"] {
            assert_eq!(iwd_ssid(&iwd_file_stem(s)).as_deref(), Some(s));
        }
        assert_eq!(iwd_ssid("=6"), None);
    }

    #[test]
    fn iwd_psk_file() {
        let text = "[Security]\nPreSharedKey=0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9\nPassphrase=\\sсекрет с пробелами\\\\x\nSAE-PT-Group19=abcdef\n\n[Settings]\nAutoConnect=true\nHidden=true\n";
        let n = iwd_parse("Home Net", "psk", text).unwrap();
        assert_eq!(n.ssid, "Home Net");
        assert_eq!(n.security, WifiSecurity::Psk);
        assert!(n.hidden);
        assert_eq!(n.psk.as_deref(), Some(" секрет с пробелами\\x"));
        // Только ключ — годится 64 hex.
        let n = iwd_parse("=d09ad0b0d184d0b5", "psk", "[Security]\nPreSharedKey=0A1B2C3D4E5F60718293A4B5C6D7E8F90A1B2C3D4E5F60718293A4B5C6D7E8F9\n").unwrap();
        assert_eq!(n.ssid, "Кафе");
        assert_eq!(n.psk.as_deref().map(str::len), Some(64));
        assert!(!n.hidden);
        // Открытая и 802.1X: без пароля.
        let n = iwd_parse("Guest", "open", "[Settings]\nAutoConnect=false\n").unwrap();
        assert_eq!((n.security, n.psk.is_none()), (WifiSecurity::Open, true));
        let n = iwd_parse("Corp", "8021x", "[Security]\nEAP-Method=PEAP\nEAP-Identity=user\nEAP-Password=p\n").unwrap();
        assert_eq!((n.security, n.psk.is_none()), (WifiSecurity::Enterprise, true));
        assert!(iwd_parse("x", "conf", "").is_none());
    }

    #[test]
    fn iwd_roundtrip() {
        let n = WifiNet { ssid: "Дача 2.4".into(), security: WifiSecurity::Psk, hidden: true, psk: Some(" a\\b ".into()) };
        let (name, text) = iwd_render(&n);
        assert_eq!(name, format!("{}.psk", iwd_file_stem("Дача 2.4")));
        let (stem, ext) = name.rsplit_once('.').unwrap();
        assert_eq!(iwd_parse(stem, ext, &text).unwrap(), n);
    }

    #[test]
    fn iwd_dir() {
        let dir = std::env::temp_dir().join(format!("synlink-wifi-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("A.psk"), "[Security]\nPassphrase=aaaaaaaa\n").unwrap();
        std::fs::write(dir.join("B.open"), "").unwrap();
        std::fs::write(dir.join("main.conf"), "[General]\n").unwrap();
        let all = iwd_dir_read(&dir, None).unwrap();
        assert_eq!(all.iter().map(|n| n.ssid.as_str()).collect::<Vec<_>>(), ["A", "B"]);
        let only = iwd_dir_read(&dir, Some(&["B".to_string()])).unwrap();
        assert_eq!(only.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn sect(items: &[(&str, Value<'_>)]) -> HashMap<String, OwnedValue> {
        items.iter().map(|(k, v)| (k.to_string(), OwnedValue::try_from(v.try_clone().unwrap()).unwrap())).collect()
    }

    #[test]
    fn nm_settings_parse() {
        let mut s: Settings = HashMap::new();
        s.insert("connection".into(), sect(&[("id", Value::from("Дом")), ("type", Value::from("802-11-wireless"))]));
        s.insert("802-11-wireless".into(), sect(&[("ssid", Value::from(b"Home 5G".to_vec())), ("mode", Value::from("infrastructure")), ("hidden", Value::from(true))]));
        s.insert("802-11-wireless-security".into(), sect(&[("key-mgmt", Value::from("sae"))]));
        let mut sec: Settings = HashMap::new();
        sec.insert("802-11-wireless-security".into(), sect(&[("psk", Value::from("пароль123"))]));
        let n = nm_net(&s, Some(&sec)).unwrap();
        assert_eq!((n.ssid.as_str(), n.security, n.hidden, n.psk.as_deref()), ("Home 5G", WifiSecurity::Sae, true, Some("пароль123")));
        // Без секретов — без пароля.
        assert_eq!(nm_net(&s, None).unwrap().psk, None);
        // 802.1X.
        s.insert("802-11-wireless-security".into(), sect(&[("key-mgmt", Value::from("wpa-eap"))]));
        assert_eq!(nm_net(&s, Some(&sec)).unwrap().security, WifiSecurity::Enterprise);
        // Открытая.
        s.remove("802-11-wireless-security");
        assert_eq!(nm_net(&s, None).unwrap().security, WifiSecurity::Open);
        // Точка доступа этого компьютера и проводные — не сети для телефона.
        s.insert("802-11-wireless".into(), sect(&[("ssid", Value::from(b"Hotspot".to_vec())), ("mode", Value::from("ap"))]));
        assert!(nm_net(&s, None).is_none());
        s.insert("connection".into(), sect(&[("type", Value::from("802-3-ethernet"))]));
        assert!(nm_net(&s, None).is_none());
    }

    #[test]
    fn nm_new_connection() {
        let n = WifiNet { ssid: "Home".into(), security: WifiSecurity::Psk, hidden: false, psk: Some("12345678".into()) };
        let s = nm_settings(&n, false);
        assert_eq!(s["802-11-wireless-security"]["key-mgmt"], Value::from("wpa-psk"));
        assert_eq!(nm_settings(&n, true)["802-11-wireless-security"]["key-mgmt"], Value::from("sae"));
        assert_eq!(s["802-11-wireless"]["ssid"], Value::from(b"Home".to_vec()));
        let Value::Str(u) = &s["connection"]["uuid"] else { panic!("uuid") };
        assert_eq!((u.len(), u.as_str().as_bytes()[14]), (36, b'4'));
        let open = WifiNet { ssid: "Free".into(), security: WifiSecurity::Open, hidden: false, psk: None };
        assert!(!nm_settings(&open, false).contains_key("802-11-wireless-security"));
    }

    #[test]
    fn no_secret_in_debug() {
        let n = WifiNet { ssid: "X".into(), security: WifiSecurity::Psk, hidden: false, psk: Some("topsecret".into()) };
        assert!(!format!("{n:?}").contains("topsecret"));
        let ex = Export { nets: vec![n], skipped: vec![] };
        assert!(!format!("{ex:?}").contains("topsecret"));
    }
}
