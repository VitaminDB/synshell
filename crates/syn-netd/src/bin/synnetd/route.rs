//! Маршрутизация: каналы в интернет — маршруты по умолчанию таблицы `main`
//! (их ставят dhcpcd, iwd, NetworkManager, модем, свой DHCP USB-модема).
//! Сама `main` не меняется никогда: всё своё — в отдельных таблицах и
//! правилах с приоритетами [`PRIO_FIRST`]..=[`PRIO_LAST`], поэтому любая
//! ошибка или остановка службы оставляет систему как была.
//!
//! Таблицы (на каждое семейство адресов отдельно):
//! * `2000 + ifindex` — маршрут по умолчанию одного канала;
//! * [`T_MULTI`] — маршрут через все каналы (nexthop каждого канала);
//! * [`T_PRI`] — маршрут основного канала.
//!
//! Правила:
//! * 10000 `lookup main suppress_prefixlength 0` — локальные сети из `main`
//!   (без её маршрута по умолчанию);
//! * 10010 `from <адрес канала> lookup <таблица канала>` — пакеты с адресом
//!   канала уходят только через него (иначе провайдер их отбросит);
//! * 10100 `fwmark` «только основной» → [`T_PRI`], 10110 `fwmark` «оба» →
//!   [`T_MULTI`] — метки ставит BPF-программа cgroup программы;
//! * 10200 — всё остальное: [`T_MULTI`] в режиме «оба», иначе [`T_PRI`].

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use serde_json::Value;
use syn_netd::{Config, Mode, Uplink};

pub const PRIO_FIRST: u32 = 10000;
pub const PRIO_LAST: u32 = 10299;
pub const T_MULTI: u32 = 1990;
pub const T_PRI: u32 = 1991;
pub const T_IF_BASE: u32 = 2000;
/// Метки сокетов (ставит BPF-программа cgroup программы).
pub const MARK_BOTH: u32 = 0x5359_0001;
pub const MARK_PRIMARY: u32 = 0x5359_0002;

/// Метрика маршрута по умолчанию у USB-модемов, поднятых самой службой: выше
/// Wi-Fi dhcpcd (3000+), чтобы подключение модема не уводило трафик с Wi-Fi —
/// основной канал выбирается в настройках.
pub const USB_METRIC: u32 = 4000;

pub fn ip(args: &[&str]) -> Result<String, String> {
    let out = Command::new("ip").args(args).env("LC_ALL", "C").output().map_err(|e| format!("ip: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!("ip {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn ip_json(args: &[&str]) -> Vec<Value> {
    let mut a = vec!["-j"];
    a.extend_from_slice(args);
    ip(&a).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

/// Маршрут по умолчанию одного канала.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gw {
    pub iface: String,
    pub ifindex: u32,
    pub via: Option<String>,
    pub metric: u32,
}

impl Gw {
    fn nexthop(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(g) = &self.via {
            v.push("via".into());
            v.push(g.clone());
        }
        v.push("dev".into());
        v.push(self.iface.clone());
        v
    }
}

/// Семейство адресов.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fam {
    V4,
    V6,
}

impl Fam {
    pub fn flag(self) -> &'static str {
        match self {
            Fam::V4 => "-4",
            Fam::V6 => "-6",
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Snapshot {
    pub v4: Vec<Gw>,
    pub v6: Vec<Gw>,
    /// Глобальные адреса интерфейсов: (адрес, длина префикса).
    pub addrs4: BTreeMap<String, Vec<(String, u8)>>,
    pub addrs6: BTreeMap<String, Vec<(String, u8)>>,
    /// Активный VPN (имя интерфейса) — служба ничего не меняет.
    pub vpn: Option<String>,
}

/// Интерфейс туннеля VPN.
fn is_vpn(name: &str, kind: &str) -> bool {
    matches!(kind, "wireguard" | "tun" | "tap" | "ipip" | "gre" | "sit" | "vti" | "xfrm")
        || ["wg", "tun", "tap", "tailscale", "zt", "nordlynx", "proton", "mullvad"].iter().any(|p| name.starts_with(p))
}

/// Тип канала для показа.
pub fn kind_of(iface: &str) -> String {
    let base = format!("/sys/class/net/{iface}");
    if std::path::Path::new(&format!("{base}/wireless")).exists() || std::path::Path::new(&format!("{base}/phy80211")).exists() {
        return "wifi".into();
    }
    if ["rmnet", "wwan", "ccmni", "rmnet_data", "qmimux"].iter().any(|p| iface.starts_with(p)) {
        return "mobile".into();
    }
    let driver = std::fs::read_link(format!("{base}/device/driver")).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
    if matches!(driver.as_str(), "cdc_mbim" | "qmi_wwan" | "cdc_mbim_ntb") {
        return "mobile".into();
    }
    let dev = std::fs::canonicalize(format!("{base}/device")).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    if crate::dhcp::USB_MODEM_DRIVERS.contains(&driver.as_str()) || dev.contains("/usb") {
        return "usb".into();
    }
    if !dev.is_empty() {
        return "ethernet".into();
    }
    "other".into()
}

fn gws(fam: Fam, vpn: &BTreeSet<String>) -> Vec<Gw> {
    let mut out: Vec<Gw> = Vec::new();
    for r in ip_json(&[fam.flag(), "route", "show", "table", "main", "default"]) {
        // Чужой multipath в main — не трогаем.
        let Some(dev) = r.get("dev").and_then(Value::as_str) else { continue };
        if vpn.contains(dev) || dev == "lo" {
            continue;
        }
        let ifindex = std::fs::read_to_string(format!("/sys/class/net/{dev}/ifindex")).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        if ifindex == 0 {
            continue;
        }
        let gw = Gw {
            iface: dev.to_string(),
            ifindex,
            via: r.get("gateway").and_then(Value::as_str).map(str::to_string),
            metric: r.get("metric").and_then(Value::as_u64).unwrap_or(0) as u32,
        };
        // Один канал — один маршрут (меньшая метрика).
        match out.iter_mut().find(|g| g.iface == gw.iface) {
            Some(g) if g.metric > gw.metric => *g = gw,
            Some(_) => {}
            None => out.push(gw),
        }
    }
    out.sort_by(|a, b| a.metric.cmp(&b.metric).then(a.iface.cmp(&b.iface)));
    out
}

fn addrs(fam: Fam) -> BTreeMap<String, Vec<(String, u8)>> {
    let mut m: BTreeMap<String, Vec<(String, u8)>> = BTreeMap::new();
    for l in ip_json(&[fam.flag(), "addr", "show"]) {
        let Some(name) = l.get("ifname").and_then(Value::as_str) else { continue };
        for a in l.get("addr_info").and_then(Value::as_array).cloned().unwrap_or_default() {
            if a.get("scope").and_then(Value::as_str) != Some("global") {
                continue;
            }
            if a.get("tentative").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            let (Some(local), Some(pl)) = (a.get("local").and_then(Value::as_str), a.get("prefixlen").and_then(Value::as_u64)) else { continue };
            m.entry(name.to_string()).or_default().push((local.to_string(), pl as u8));
        }
    }
    m
}

pub fn read() -> Snapshot {
    // Туннели VPN.
    let mut vpn_ifaces = BTreeSet::new();
    for l in ip_json(&["-d", "link", "show"]) {
        let Some(name) = l.get("ifname").and_then(Value::as_str) else { continue };
        let kind = l.pointer("/linkinfo/info_kind").and_then(Value::as_str).unwrap_or("");
        if is_vpn(name, kind) {
            vpn_ifaces.insert(name.to_string());
        }
    }
    // VPN включён: у туннеля маршрут по умолчанию в любой таблице (wg-quick и
    // NetworkManager кладут его в свою таблицу со своими правилами).
    let mut vpn = None;
    'outer: for fam in [Fam::V4, Fam::V6] {
        for r in ip_json(&[fam.flag(), "route", "show", "table", "all"]) {
            let dst = r.get("dst").and_then(Value::as_str).unwrap_or("");
            if !matches!(dst, "default" | "0.0.0.0/1" | "128.0.0.0/1" | "::/1" | "8000::/1") {
                continue;
            }
            if let Some(dev) = r.get("dev").and_then(Value::as_str) {
                if vpn_ifaces.contains(dev) {
                    vpn = Some(dev.to_string());
                    break 'outer;
                }
            }
        }
    }
    Snapshot { v4: gws(Fam::V4, &vpn_ifaces), v6: gws(Fam::V6, &vpn_ifaces), addrs4: addrs(Fam::V4), addrs6: addrs(Fam::V6), vpn }
}

/// Что поставить: команды `ip` (таблицы, затем правила).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub cmds: Vec<Vec<String>>,
    /// Основной канал (по IPv4, без него — по IPv6).
    pub primary: Option<String>,
    pub note: Option<String>,
}

impl Plan {
    pub fn active(&self) -> bool {
        !self.cmds.is_empty()
    }
}

/// Основной канал семейства: выбранный в настройках, иначе меньшая метрика.
fn primary_of<'a>(gws: &'a [Gw], cfg: &Config) -> Option<&'a Gw> {
    gws.iter().find(|g| !cfg.primary.is_empty() && g.iface == cfg.primary).or_else(|| gws.first())
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

/// План одного семейства. `primary` — основной канал системы (по IPv4,
/// без него — по IPv6): у семейства его может не быть (IPv6 только у
/// модема) — тогда в режиме «основной» это семейство через другие каналы
/// недоступно, и программы сразу берут IPv4 через основной канал, а не
/// уходят на модем по IPv6 мимо выбора.
fn plan_family(fam: Fam, gws: &[Gw], addrs: &BTreeMap<String, Vec<(String, u8)>>, cfg: &Config, primary: Option<&str>, out: &mut Vec<Vec<String>>) {
    let pri = gws.iter().find(|g| Some(g.iface.as_str()) == primary);
    // Один канал и он же основной — всё как без службы.
    if gws.is_empty() || (gws.len() == 1 && pri.is_some()) {
        return;
    }
    let f = fam.flag();
    // Таблицы каналов.
    for g in gws {
        let t = (T_IF_BASE + g.ifindex).to_string();
        let mut c = s(&[f, "route", "replace", "default"]);
        c.extend(g.nexthop());
        c.extend(s(&["table", &t]));
        out.push(c);
    }
    // Основной.
    let mut c = s(&[f, "route", "replace"]);
    match pri {
        Some(p) => {
            c.push("default".into());
            c.extend(p.nexthop());
        }
        None => c.extend(s(&["unreachable", "default"])),
    }
    c.extend(s(&["table", &T_PRI.to_string()]));
    out.push(c);
    // Все каналы.
    let mut c = s(&[f, "route", "replace", "default"]);
    if let [g] = gws {
        c.extend(g.nexthop());
        c.extend(s(&["table", &T_MULTI.to_string()]));
    } else {
        c.extend(s(&["table", &T_MULTI.to_string()]));
        for g in gws {
            c.push("nexthop".into());
            c.extend(g.nexthop());
            c.extend(s(&["weight", "1"]));
        }
    }
    out.push(c);
    // Правила.
    out.push(s(&[f, "rule", "add", "pref", &PRIO_FIRST.to_string(), "lookup", "main", "suppress_prefixlength", "0"]));
    // У основного канала этого семейства нет: закрыть его раньше правил по
    // адресу источника — IPv6 при ошибке первого поиска маршрута сам
    // выбирает адрес источника и ищет снова (ip6_dst_lookup_tail), и правило
    // `from <адрес модема>` иначе пропустило бы соединение мимо выбора.
    if pri.is_none() {
        let t = T_PRI.to_string();
        let both = format!("{MARK_BOTH:#x}");
        let prim = format!("{MARK_PRIMARY:#x}");
        out.push(if cfg.mode == Mode::Primary {
            s(&[f, "rule", "add", "pref", "10005", "not", "fwmark", &both, "lookup", &t])
        } else {
            s(&[f, "rule", "add", "pref", "10005", "fwmark", &prim, "lookup", &t])
        });
    }
    for g in gws {
        let t = (T_IF_BASE + g.ifindex).to_string();
        for (a, _) in addrs.get(&g.iface).map(Vec::as_slice).unwrap_or(&[]) {
            out.push(s(&[f, "rule", "add", "pref", "10010", "from", a, "lookup", &t]));
        }
    }
    out.push(s(&[f, "rule", "add", "pref", "10100", "fwmark", &format!("{MARK_PRIMARY:#x}"), "lookup", &T_PRI.to_string()]));
    out.push(s(&[f, "rule", "add", "pref", "10110", "fwmark", &format!("{MARK_BOTH:#x}"), "lookup", &T_MULTI.to_string()]));
    let rest = if cfg.mode == Mode::Both { T_MULTI } else { T_PRI };
    out.push(s(&[f, "rule", "add", "pref", "10200", "lookup", &rest.to_string()]));
}

pub fn plan(snap: &Snapshot, cfg: &Config) -> Plan {
    let primary = primary_of(&snap.v4, cfg).or_else(|| primary_of(&snap.v6, cfg)).map(|g| g.iface.clone());
    if let Some(v) = &snap.vpn {
        return Plan { cmds: vec![], primary, note: Some(format!("vpn:{v}")) };
    }
    let mut cmds = Vec::new();
    plan_family(Fam::V4, &snap.v4, &snap.addrs4, cfg, primary.as_deref(), &mut cmds);
    plan_family(Fam::V6, &snap.v6, &snap.addrs6, cfg, primary.as_deref(), &mut cmds);
    let note = if cmds.is_empty() { Some(if snap.v4.is_empty() && snap.v6.is_empty() { "none".into() } else { "single".into() }) } else { None };
    Plan { cmds, primary, note }
}

/// Убрать всё своё: правила в своём диапазоне приоритетов и свои таблицы.
pub fn teardown() {
    for fam in [Fam::V4, Fam::V6] {
        let f = fam.flag();
        let prios: BTreeSet<u32> = ip_json(&[f, "rule", "show"])
            .iter()
            .filter_map(|r| r.get("priority").and_then(Value::as_u64).map(|p| p as u32))
            .filter(|p| (PRIO_FIRST..=PRIO_LAST).contains(p))
            .collect();
        for p in prios {
            // `del pref` удаляет по одному правилу.
            for _ in 0..256 {
                if ip(&[f, "rule", "del", "pref", &p.to_string()]).is_err() {
                    break;
                }
            }
        }
        let tables: BTreeSet<u32> = ip_json(&[f, "route", "show", "table", "all"])
            .iter()
            .filter_map(|r| r.get("table").and_then(|t| t.as_str().and_then(|s| s.parse().ok()).or_else(|| t.as_u64().map(|x| x as u32))))
            .filter(|t| *t == T_MULTI || *t == T_PRI || (T_IF_BASE..T_IF_BASE + 1000).contains(t))
            .collect();
        for t in tables {
            let _ = ip(&[f, "route", "flush", "table", &t.to_string()]);
        }
    }
}

/// Поставить план: прежнее своё убрать, новое — таблицы, затем правила.
/// Пока правил нет, всё идёт по `main` — как без службы.
pub fn apply(plan: &Plan) -> Vec<String> {
    teardown();
    let mut errs = Vec::new();
    if plan.active() {
        // Соединения расходятся по каналам по адресам и портам (L4), а не
        // только по адресу назначения.
        let _ = std::fs::write("/proc/sys/net/ipv4/fib_multipath_hash_policy", "1");
        let _ = std::fs::write("/proc/sys/net/ipv6/fib_multipath_hash_policy", "1");
    }
    for c in &plan.cmds {
        let a: Vec<&str> = c.iter().map(String::as_str).collect();
        if let Err(e) = ip(&a) {
            errs.push(e);
        }
    }
    errs
}

/// Каналы для показа.
pub fn uplinks(snap: &Snapshot, plan: &Plan, ours: &dyn Fn(&str) -> bool) -> Vec<Uplink> {
    let mut out: Vec<Uplink> = Vec::new();
    for g in snap.v4.iter().chain(snap.v6.iter()) {
        if out.iter().any(|u| u.iface == g.iface) {
            continue;
        }
        let v4 = snap.v4.iter().find(|x| x.iface == g.iface);
        let stat = |n: &str| std::fs::read_to_string(format!("/sys/class/net/{}/statistics/{n}", g.iface)).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        out.push(Uplink {
            iface: g.iface.clone(),
            kind: kind_of(&g.iface),
            ipv4: snap.addrs4.get(&g.iface).and_then(|v| v.first()).map(|(a, p)| format!("{a}/{p}")),
            gateway4: v4.and_then(|x| x.via.clone()),
            ipv6: snap.v6.iter().any(|x| x.iface == g.iface),
            metric: v4.map(|x| x.metric).unwrap_or(g.metric),
            primary: plan.primary.as_deref() == Some(g.iface.as_str()),
            dhcp_by_synnetd: ours(&g.iface),
            rx_bytes: stat("rx_bytes"),
            tx_bytes: stat("tx_bytes"),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gw(iface: &str, idx: u32, via: &str, metric: u32) -> Gw {
        Gw { iface: iface.into(), ifindex: idx, via: Some(via.into()), metric }
    }

    fn snap() -> Snapshot {
        let mut s = Snapshot { v4: vec![gw("wlan0", 5, "192.168.10.1", 3005), gw("usb0", 7, "192.168.42.129", 4000)], ..Default::default() };
        s.addrs4.insert("wlan0".into(), vec![("192.168.10.106".into(), 24)]);
        s.addrs4.insert("usb0".into(), vec![("192.168.42.10".into(), 24)]);
        s
    }

    fn joined(p: &Plan) -> Vec<String> {
        p.cmds.iter().map(|c| c.join(" ")).collect()
    }

    #[test]
    fn single_uplink_changes_nothing() {
        let mut s = snap();
        s.v4.pop();
        let p = plan(&s, &Config::default());
        assert!(!p.active());
        assert_eq!(p.note.as_deref(), Some("single"));
        assert_eq!(p.primary.as_deref(), Some("wlan0"));
    }

    #[test]
    fn both_mode() {
        let cfg = Config { mode: Mode::Both, ..Default::default() };
        let p = plan(&snap(), &cfg);
        let j = joined(&p);
        assert!(j.contains(&"-4 route replace default table 1990 nexthop via 192.168.10.1 dev wlan0 weight 1 nexthop via 192.168.42.129 dev usb0 weight 1".to_string()), "{j:#?}");
        assert!(j.contains(&"-4 route replace default via 192.168.10.1 dev wlan0 table 1991".to_string()));
        assert!(j.contains(&"-4 rule add pref 10010 from 192.168.42.10 lookup 2007".to_string()));
        assert!(j.contains(&"-4 rule add pref 10000 lookup main suppress_prefixlength 0".to_string()));
        assert_eq!(j.last().unwrap(), "-4 rule add pref 10200 lookup 1990");
    }

    #[test]
    fn primary_choice() {
        let cfg = Config { primary: "usb0".into(), ..Default::default() };
        let p = plan(&snap(), &cfg);
        assert_eq!(p.primary.as_deref(), Some("usb0"));
        let j = joined(&p);
        assert!(j.contains(&"-4 route replace default via 192.168.42.129 dev usb0 table 1991".to_string()));
        assert_eq!(j.last().unwrap(), "-4 rule add pref 10200 lookup 1991");
        // Выбранного канала нет — меньшая метрика.
        let cfg = Config { primary: "eth9".into(), ..Default::default() };
        assert_eq!(plan(&snap(), &cfg).primary.as_deref(), Some("wlan0"));
    }

    #[test]
    fn ipv6_only_on_secondary() {
        // IPv6 только у модема, основной — Wi-Fi.
        let mut s = snap();
        s.v6 = vec![Gw { iface: "usb0".into(), ifindex: 7, via: Some("fe80::1".into()), metric: 1024 }];
        s.addrs6.insert("usb0".into(), vec![("2a03::5".into(), 64)]);
        let p = plan(&s, &Config::default());
        let j = joined(&p);
        // Режим «основной»: IPv6 мимо Wi-Fi недоступен (программы возьмут IPv4).
        assert!(j.contains(&"-6 route replace unreachable default table 1991".to_string()), "{j:#?}");
        assert!(j.contains(&"-6 route replace default via fe80::1 dev usb0 table 1990".to_string()));
        assert!(j.contains(&"-6 rule add pref 10200 lookup 1991".to_string()));
        assert!(j.contains(&"-6 rule add pref 10010 from 2a03::5 lookup 2007".to_string()));
        assert!(j.contains(&"-6 rule add pref 10005 not fwmark 0x53590001 lookup 1991".to_string()));
        // «Оба»: IPv6 через модем, закрыт только программам «только основной».
        let j = joined(&plan(&s, &Config { mode: Mode::Both, ..Default::default() }));
        assert!(j.contains(&"-6 rule add pref 10200 lookup 1990".to_string()));
        assert!(j.contains(&"-6 rule add pref 10005 fwmark 0x53590002 lookup 1991".to_string()));
        // У IPv4 основной есть — без закрывающего правила.
        assert!(!j.iter().any(|c| c.starts_with("-4") && c.contains("10005")));
        // Основной — модем: IPv6 как без службы.
        let j = joined(&plan(&s, &Config { primary: "usb0".into(), ..Default::default() }));
        assert!(!j.iter().any(|c| c.starts_with("-6")), "{j:#?}");
    }

    #[test]
    fn vpn_pauses() {
        let mut s = snap();
        s.vpn = Some("wg0".into());
        let p = plan(&s, &Config { mode: Mode::Both, ..Default::default() });
        assert!(!p.active());
        assert_eq!(p.note.as_deref(), Some("vpn:wg0"));
    }

    #[test]
    fn vpn_names() {
        assert!(is_vpn("wg0", ""));
        assert!(is_vpn("x", "wireguard"));
        assert!(!is_vpn("wlan0", ""));
        assert!(!is_vpn("enp128s20f0u1", ""));
    }
}
