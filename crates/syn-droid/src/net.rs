//! Сеть контейнера: мост `syndroid0` (192.168.240.1/24), veth в контейнер (там — `eth0`, Android получает
//! адрес по DHCP), NAT наружу и DHCP/DNS от dnsmasq. NAT — через `iptables-legacy`: в GKI нет nf_tables.

use std::process::{Child, Command, Stdio};

use anyhow::{bail, Context, Result};

pub const BRIDGE: &str = "syndroid0";
pub const HOST_VETH: &str = "vsyndroid";
pub const PEER_VETH: &str = "vsyndroid-c";
pub const ADDR: &str = "192.168.240.1";
pub const NETWORK: &str = "192.168.240.0/24";
pub const MAC: &str = "00:16:3e:f9:d3:03";

pub fn run(cmd: &str, args: &[&str]) -> Result<()> {
    let out = Command::new(cmd).args(args).stdin(Stdio::null()).output().with_context(|| cmd.to_string())?;
    if !out.status.success() {
        bail!("{cmd} {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

fn ok(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd).args(args).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn iptables() -> &'static str {
    if ok("iptables-legacy", &["-V"]) {
        "iptables-legacy"
    } else {
        "iptables"
    }
}

/// Правила iptables: (таблица, цепочка, правило).
fn rules() -> Vec<(&'static str, &'static str, Vec<&'static str>)> {
    vec![
        ("filter", "INPUT", vec!["-i", BRIDGE, "-p", "udp", "--dport", "67", "-j", "ACCEPT"]),
        ("filter", "INPUT", vec!["-i", BRIDGE, "-p", "udp", "--dport", "53", "-j", "ACCEPT"]),
        ("filter", "INPUT", vec!["-i", BRIDGE, "-p", "tcp", "--dport", "53", "-j", "ACCEPT"]),
        ("filter", "FORWARD", vec!["-i", BRIDGE, "-j", "ACCEPT"]),
        ("filter", "FORWARD", vec!["-o", BRIDGE, "-j", "ACCEPT"]),
        ("nat", "POSTROUTING", vec!["-s", NETWORK, "!", "-d", NETWORK, "-j", "MASQUERADE"]),
    ]
}

fn rule_args<'a>(op: &'a str, table: &'a str, chain: &'a str, rule: &[&'a str]) -> Vec<&'a str> {
    let mut a = vec!["-w", "-t", table, op, chain];
    a.extend_from_slice(rule);
    a
}

/// Поднять мост, NAT и dnsmasq. Возвращает процесс dnsmasq (его надо остановить в [`down`]).
pub fn up() -> Result<Option<Child>> {
    if !ok("ip", &["link", "show", BRIDGE]) {
        run("ip", &["link", "add", BRIDGE, "type", "bridge"])?;
    }
    let _ = run("ip", &["addr", "flush", "dev", BRIDGE]);
    run("ip", &["addr", "add", &format!("{ADDR}/24"), "dev", BRIDGE])?;
    run("ip", &["link", "set", BRIDGE, "address", "00:16:3e:00:00:01", "up"])?;
    std::fs::write("/proc/sys/net/ipv4/ip_forward", "1").context("ip_forward")?;
    let ipt = iptables();
    for (t, c, r) in rules() {
        if !ok(ipt, &rule_args("-C", t, c, &r)) {
            run(ipt, &rule_args("-I", t, c, &r))?;
        }
    }
    let dnsmasq = which("dnsmasq");
    let Some(dnsmasq) = dnsmasq else {
        tracing::warn!("нет dnsmasq — у Android не будет адреса по DHCP");
        return Ok(None);
    };
    std::fs::create_dir_all(crate::paths::RUN)?;
    let child = Command::new(dnsmasq)
        .args([
            "--keep-in-foreground",
            "--conf-file=/dev/null",
            "--strict-order",
            "--bind-interfaces",
            "--except-interface=lo",
            &format!("--interface={BRIDGE}"),
            &format!("--listen-address={ADDR}"),
            "--dhcp-range=192.168.240.2,192.168.240.254",
            "--dhcp-lease-max=253",
            "--dhcp-no-override",
            "--dhcp-authoritative",
            "--dhcp-leasefile=/run/syndroid/dnsmasq.leases",
            "--pid-file=",
            "--user=dnsmasq",
        ])
        .stdin(Stdio::null())
        .spawn()
        .context("dnsmasq")?;
    Ok(Some(child))
}

/// Пара veth: хостовая половина — в мост, вторая — в сетевое пространство процесса `pid`.
pub fn attach(pid: i32) -> Result<()> {
    if ok("ip", &["link", "show", HOST_VETH]) {
        let _ = run("ip", &["link", "del", HOST_VETH]);
    }
    run("ip", &["link", "add", HOST_VETH, "type", "veth", "peer", "name", PEER_VETH])?;
    run("ip", &["link", "set", HOST_VETH, "master", BRIDGE, "up"])?;
    run("ip", &["link", "set", PEER_VETH, "netns", &pid.to_string()])?;
    Ok(())
}

/// Внутри контейнера (до pivot_root): переименовать veth в eth0, поднять lo и eth0.
pub fn configure_inside() -> Result<()> {
    run("ip", &["link", "set", "lo", "up"])?;
    if ok("ip", &["link", "show", PEER_VETH]) {
        run("ip", &["link", "set", PEER_VETH, "name", "eth0"])?;
        run("ip", &["link", "set", "eth0", "address", MAC, "mtu", "1500", "up"])?;
    }
    Ok(())
}

pub fn down(dnsmasq: Option<Child>) {
    if let Some(mut c) = dnsmasq {
        let _ = c.kill();
        let _ = c.wait();
    }
    let _ = run("ip", &["link", "del", HOST_VETH]);
    let ipt = iptables();
    for (t, c, r) in rules() {
        while ok(ipt, &rule_args("-C", t, c, &r)) {
            if run(ipt, &rule_args("-D", t, c, &r)).is_err() {
                break;
            }
        }
    }
    let _ = run("ip", &["link", "del", BRIDGE]);
}

pub fn which(cmd: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_else(|| vec!["/usr/bin".into(), "/usr/sbin".into()])
        .into_iter()
        .map(|d| d.join(cmd))
        .find(|p| p.is_file())
}
