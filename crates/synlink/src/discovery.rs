//! Поиск устройств: широковещательные анонсы UDP на каждом интерфейсе.
//!
//! По USB-сети (NCM телефона) и по Wi-Fi — один и тот же механизм; чем
//! именно устройство достижимо, решает подсеть адреса (`netif`). Анонс
//! без секретов: id (начало отпечатка), имя, вид, порт QUIC. Доверие
//! проверяется уже в TLS.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    // Сокет пересоздаётся, если перестали слышать даже собственные анонсы (широковещание возвращается
    // отправителю): раньше долгоживущий демон глох и не видел устройств до перезапуска.
    loop {
        let sock = match bind(port) {
            Ok(s) => Arc::new(s),
            Err(e) => {
                tracing::error!(?e, port, "поиск устройств: порт занят");
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        };
        let sent = Arc::new(Mutex::new(None::<Instant>));
        let announcer = {
            let (d, sock, sent) = (d.clone(), sock.clone(), sent.clone());
            tokio::spawn(async move {
                let mut n = 0u64;
                loop {
                    if announce(&d, &sock, port, n == 0).await {
                        sent.lock().unwrap().get_or_insert_with(Instant::now);
                    }
                    n += 1;
                    // Чаще в первые секунды — быстрее находим друг друга.
                    let wait = if n < 5 { 1 } else { 4 };
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(wait)) => {}
                        _ = d.announce_now.notified() => { n = 0; }
                    }
                }
            })
        };
        receive(&d, &sock, port, &sent).await;
        announcer.abort();
        tracing::warn!(port, "поиск устройств: не слышу своих анонсов — сокет пересоздан");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Приём анонсов, пока слышно своё эхо. `sent` — когда впервые с последнего эха анонс ушёл хоть в
/// один интерфейс (без интерфейсов и в невидимом режиме эха и не должно быть).
async fn receive(d: &Arc<Daemon>, sock: &UdpSocket, port: u16, sent: &Mutex<Option<Instant>>) {
    let mut buf = [0u8; 2048];
    loop {
        let r = tokio::time::timeout(Duration::from_secs(5), sock.recv_from(&mut buf)).await;
        if sent.lock().unwrap().is_some_and(|t| t.elapsed() > ECHO_TIMEOUT) {
            return;
        }
        let (n, from) = match r {
            Err(_) => continue,
            Ok(Ok(x)) => x,
            Ok(Err(e)) => {
                tracing::debug!(?e, "поиск устройств: приём");
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        let Ok(a) = serde_json::from_slice::<Announce>(&buf[..n]) else { continue };
        if a.synlink != crate::proto::PROTO {
            continue;
        }
        if a.id == d.id.id {
            *sent.lock().unwrap() = None;
            continue;
        }
        if crate::netif::usb_disabled() && crate::netif::in_usb_subnet(from.ip()) {
            continue;
        }
        if a.query {
            announce(d, sock, port, false).await;
        }
        d.clone().heard(a, from).await;
    }
}

/// Своё эхо не вернулось за это время — приём сломан.
const ECHO_TIMEOUT: Duration = Duration::from_secs(30);

fn bind(port: u16) -> std::io::Result<UdpSocket> {
    let s = std::net::UdpSocket::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))?;
    s.set_broadcast(true)?;
    s.set_nonblocking(true)?;
    UdpSocket::from_std(s)
}

/// Анонс во все интерфейсы; true — ушёл хотя бы в один широковещательно.
async fn announce(d: &Daemon, sock: &UdpSocket, port: u16, query: bool) -> bool {
    let me = d.self_info();
    let a = Announce { synlink: crate::proto::PROTO, id: me.id, name: me.name, kind: me.kind, port: d.port, query };
    let Ok(data) = serde_json::to_vec(&a) else { return false };
    let mut any = false;
    let sleeping = d.sleeping.load(std::sync::atomic::Ordering::Relaxed);
    for i in crate::netif::list() {
        // Невидимая машина анонсирует себя только по кабелю; спящий телефон — тоже.
        if (!me.discoverable || sleeping) && !i.usb {
            continue;
        }
        any |= sock.send_to(&data, SocketAddr::from((i.broadcast(), port))).await.is_ok();
        // Точка-точка USB: широковещание может не пройти — ещё и соседу.
        if i.usb {
            let peer = if i.addr.octets()[3] == 1 { 2 } else { 1 };
            let o = i.addr.octets();
            let _ = sock.send_to(&data, SocketAddr::from((Ipv4Addr::new(o[0], o[1], o[2], peer), port))).await;
        }
    }
    any
}
