//! Поиск устройств: широковещательные анонсы UDP на каждом интерфейсе.
//!
//! По USB-сети (NCM телефона) и по Wi-Fi — один и тот же механизм; чем
//! именно устройство достижимо, решает подсеть адреса (`netif`). Анонс
//! без секретов: id (начало отпечатка), имя, вид, порт QUIC. Доверие
//! проверяется уже в TLS.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use synshell_common::link::DeviceKind;
use tokio::net::UdpSocket;

use crate::daemon::Daemon;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Announce {
    pub synlink: u32,
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub port: u16,
    /// Ответь анонсом (новое устройство в сети хочет узнать соседей).
    #[serde(default)]
    pub query: bool,
}

pub async fn run(d: Arc<Daemon>) {
    let port = d.port.saturating_sub(1);
    let sock = match bind(port) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(?e, port, "поиск устройств: порт занят");
            return;
        }
    };
    let sock = Arc::new(sock);
    {
        let (d, sock) = (d.clone(), sock.clone());
        tokio::spawn(async move {
            let mut n = 0u64;
            loop {
                announce(&d, &sock, port, n == 0).await;
                n += 1;
                // Чаще в первые секунды — быстрее находим друг друга.
                let wait = if n < 5 { 1 } else { 4 };
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(wait)) => {}
                    _ = d.announce_now.notified() => { n = 0; }
                }
            }
        });
    }
    let mut buf = [0u8; 2048];
    loop {
        let Ok((n, from)) = sock.recv_from(&mut buf).await else { continue };
        let Ok(a) = serde_json::from_slice::<Announce>(&buf[..n]) else { continue };
        if a.synlink != crate::proto::PROTO || a.id == d.id.id {
            continue;
        }
        if crate::netif::usb_disabled() && crate::netif::in_usb_subnet(from.ip()) {
            continue;
        }
        if a.query {
            announce(&d, &sock, port, false).await;
        }
        d.clone().heard(a, from).await;
    }
}

fn bind(port: u16) -> std::io::Result<UdpSocket> {
    let s = std::net::UdpSocket::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))?;
    s.set_broadcast(true)?;
    s.set_nonblocking(true)?;
    UdpSocket::from_std(s)
}

async fn announce(d: &Daemon, sock: &UdpSocket, port: u16, query: bool) {
    let me = d.self_info();
    let a = Announce { synlink: crate::proto::PROTO, id: me.id, name: me.name, kind: me.kind, port: d.port, query };
    let Ok(data) = serde_json::to_vec(&a) else { return };
    for i in crate::netif::list() {
        // Невидимая машина анонсирует себя только по кабелю.
        if !me.discoverable && !i.usb {
            continue;
        }
        let _ = sock.send_to(&data, SocketAddr::from((i.broadcast(), port))).await;
        // Точка-точка USB: широковещание может не пройти — ещё и соседу.
        if i.usb {
            let peer = if i.addr.octets()[3] == 1 { 2 } else { 1 };
            let o = i.addr.octets();
            let _ = sock.send_to(&data, SocketAddr::from((Ipv4Addr::new(o[0], o[1], o[2], peer), port))).await;
        }
    }
}
