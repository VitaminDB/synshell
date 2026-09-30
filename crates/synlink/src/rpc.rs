//! Вызовы между устройствами: каждый — свой поток QUIC. Здесь сервер
//! (что делать, когда другое устройство просит) и клиентский `call`.

use anyhow::{bail, Context, Result};
use synshell_common::ipc::InputEvent;

use crate::daemon::D;
use crate::proto::{self, Reply, Rpc};

pub async fn serve(d: D, peer: String, mut w: quinn::SendStream, mut r: quinn::RecvStream) -> Result<()> {
    // Вызовы — только от спаренных.
    if d.trusted(&peer).is_none() {
        proto::send(&mut w, &Reply::Err("не спарено".into())).await?;
        return Ok(());
    }
    let Some(req) = proto::recv::<Rpc>(&mut r).await? else { return Ok(()) };
    match req {
        Rpc::Screenshot { output } => {
            let res = tokio::task::spawn_blocking(move || crate::services::screenshot(output)).await?;
            let reply = match res {
                Ok((output, width, height, png)) => Reply::Shot { output, width, height, png },
                Err(e) => Reply::Err(format!("{e:#}")),
            };
            proto::send(&mut w, &reply).await?;
        }
        Rpc::Input { output, events } => {
            let reply = match serde_json::from_str::<Vec<InputEvent>>(&events) {
                Ok(ev) => match tokio::task::spawn_blocking(move || crate::services::input(output, ev)).await? {
                    Ok(()) => Reply::Done,
                    Err(e) => Reply::Err(format!("{e:#}")),
                },
                Err(e) => Reply::Err(format!("события: {e}")),
            };
            proto::send(&mut w, &reply).await?;
        }
        Rpc::Wm { request } => {
            let reply = match serde_json::from_str(&request) {
                Ok(req) => match tokio::task::spawn_blocking(move || crate::services::wm(req)).await? {
                    Ok(resp) => Reply::Json(serde_json::to_string(&resp)?),
                    Err(e) => Reply::Err(format!("{e:#}")),
                },
                Err(e) => Reply::Err(format!("запрос: {e}")),
            };
            proto::send(&mut w, &reply).await?;
        }
        Rpc::Exec { argv, cwd, pty } => crate::exec::serve(w, r, argv, cwd, pty).await?,
        Rpc::Tcp { port } => {
            let port = if port == 0 { crate::ssh::sshd_port().await? } else { port };
            match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                Ok(mut tcp) => {
                    proto::send(&mut w, &Reply::Done).await?;
                    let mut quic = tokio::io::join(r, w);
                    let _ = tokio::io::copy_bidirectional(&mut quic, &mut tcp).await;
                }
                Err(e) => proto::send(&mut w, &Reply::Err(format!("127.0.0.1:{port}: {e}"))).await?,
            }
        }
        Rpc::Screen { output, cursor } => crate::screen::serve(w, r, output, cursor).await?,
        Rpc::Fs(req) => {
            let resp = tokio::task::spawn_blocking(move || crate::fs::handle(req)).await?;
            proto::send(&mut w, &Reply::Fs(resp)).await?;
        }
    }
    Ok(())
}

/// Простой вызов: запрос → ответ.
pub async fn call(conn: &quinn::Connection, req: &Rpc) -> Result<Reply> {
    let (mut w, mut r) = conn.open_bi().await.context("соединение")?;
    proto::send(&mut w, req).await?;
    let _ = w.finish();
    match proto::recv::<Reply>(&mut r).await? {
        Some(Reply::Err(e)) => bail!("{e}"),
        Some(rep) => Ok(rep),
        None => bail!("устройство не ответило"),
    }
}

/// Открыть поток для вызова с продолжением (команда, туннель, экран).
pub async fn open(conn: &quinn::Connection, req: &Rpc) -> Result<(quinn::SendStream, quinn::RecvStream)> {
    let (mut w, r) = conn.open_bi().await.context("соединение")?;
    proto::send(&mut w, req).await?;
    Ok((w, r))
}
