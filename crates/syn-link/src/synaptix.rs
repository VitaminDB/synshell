//! Мост к службе нейросетей этой машины для спаренного устройства
//! (федерация synaptixd): кадры моста из сети — в сокет службы (файлы
//! становятся memfd), сообщения службы — обратно кадрами. Поток закрылся —
//! сокет службы закрывается, и она отменяет начатую работу.

use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::sync::Arc;

use anyhow::{Context, Result};
use synshell_tr::t;
use synaptixd::bridge::{Frame, MAX_FRAME, REMOTE_PREFIX};
use synaptixd::wire;

use crate::daemon::D;
use crate::proto::{self, Reply};

fn open_service(device: &str) -> Result<OwnedFd> {
    let path = synaptixd::socket_path();
    let sock = wire::connect(&path).with_context(|| t!("служба нейросетей ({path})", path = path.display()))?;
    // Метка: соединение с другого устройства — только свои модели, без
    // пересылки дальше и без смены настроек.
    let hello = serde_json::json!({ "id": 0, "op": "hello", "client": format!("{REMOTE_PREFIX}{device}") });
    wire::send(sock.as_fd(), hello.to_string().as_bytes(), &[])?;
    wire::recv(sock.as_fd()).with_context(|| t!("служба не ответила"))?;
    Ok(sock)
}

pub async fn serve(d: D, peer: String, mut w: quinn::SendStream, mut r: quinn::RecvStream) -> Result<()> {
    let name = d.trusted(&peer).map(|t| t.name).unwrap_or(peer);
    let sock = match tokio::task::spawn_blocking(move || open_service(&name)).await? {
        Ok(s) => Arc::new(s),
        Err(e) => {
            proto::send(&mut w, &Reply::Err(format!("{e:#}"))).await?;
            return Ok(());
        }
    };
    proto::send(&mut w, &Reply::Done).await?;

    // Служба → сеть: чтение SEQPACKET блокирующее — в своём потоке.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);
    {
        let sock = sock.clone();
        std::thread::Builder::new().name("synaptix-bridge".into()).spawn(move || {
            while let Ok(f) = Frame::recv_from(sock.as_fd()) {
                if tx.blocking_send(f.encode()).is_err() {
                    break;
                }
            }
        })?;
    }
    let up = async {
        while let Some(b) = rx.recv().await {
            w.write_all(&b).await?;
        }
        let _ = w.finish();
        Ok::<(), anyhow::Error>(())
    };
    let down = async {
        loop {
            let mut len = [0u8; 8];
            match r.read_exact(&mut len).await {
                Ok(()) => {}
                Err(_) => return Ok(()),
            }
            let n = u64::from_be_bytes(len);
            anyhow::ensure!(n <= MAX_FRAME, "{}", t!("кадр моста {n} байт — больше предела", n = n));
            let mut body = vec![0u8; n as usize];
            r.read_exact(&mut body).await.with_context(|| t!("обрыв кадра"))?;
            let s = sock.clone();
            tokio::task::spawn_blocking(move || Frame::decode_body(&body)?.send_to(s.as_fd())).await??;
        }
    };
    let res = tokio::select! {
        r = up => r,
        r = down => r,
    };
    // Обрыв с любой стороны: служба видит закрытие и отменяет работу,
    // поток чтения выходит.
    unsafe {
        libc::shutdown(sock.as_raw_fd(), libc::SHUT_RDWR);
    }
    res
}
