//! Команды на устройстве: обычные (каналы stdin/stdout/stderr) и в
//! терминале (PTY — `synlink shell`). Сервер обобщён по потокам: это и
//! QUIC-поток от другого устройства, и `duplex` для `local`.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::proto::{self, ExecIn, ExecOut};
use synshell_tr::t;

/// Выполнить команду и гонять кадры `ExecIn`/`ExecOut` по потокам.
pub async fn serve<W, R>(mut w: W, mut r: R, argv: Vec<String>, cwd: Option<String>, pty: Option<(u16, u16)>) -> Result<()>
where
    W: AsyncWrite + Unpin + Send + 'static,
    R: AsyncRead + Unpin + Send + 'static,
{
    if argv.is_empty() {
        proto::send(&mut w, &ExecOut::Error(t!("пустая команда").into())).await?;
        return Ok(());
    }
    let (out_tx, mut out_rx) = mpsc::channel::<ExecOut>(64);
    let (in_tx, in_rx) = mpsc::channel::<ExecIn>(64);
    let started = match pty {
        Some((cols, rows)) => spawn_pty(&argv, cwd.as_deref(), cols, rows, out_tx.clone(), in_rx),
        None => spawn_pipes(&argv, cwd.as_deref(), out_tx.clone(), in_rx),
    };
    if let Err(e) = started {
        proto::send(&mut w, &ExecOut::Error(format!("{:#}", e))).await?;
        return Ok(());
    }
    drop(out_tx);
    proto::send(&mut w, &ExecOut::Started).await?;
    let reader = tokio::spawn(async move {
        while let Ok(Some(m)) = proto::recv::<ExecIn>(&mut r).await {
            if in_tx.send(m).await.is_err() {
                break;
            }
        }
        // Клиент ушёл — процесс убить.
        let _ = in_tx.send(ExecIn::Kill).await;
    });
    while let Some(m) = out_rx.recv().await {
        let last = matches!(m, ExecOut::Exit(_));
        if proto::send(&mut w, &m).await.is_err() {
            break;
        }
        if last {
            break;
        }
    }
    let _ = w.shutdown().await;
    reader.abort();
    Ok(())
}

fn command(argv: &[String], cwd: Option<&str>) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(&argv[0]);
    c.args(&argv[1..]);
    let home = synshell_common::paths::home();
    c.current_dir(cwd.map(std::path::PathBuf::from).filter(|p| p.is_dir()).unwrap_or(home));
    c.kill_on_drop(true);
    c
}

fn spawn_pipes(argv: &[String], cwd: Option<&str>, out: mpsc::Sender<ExecOut>, mut inp: mpsc::Receiver<ExecIn>) -> Result<()> {
    let mut c = command(argv, cwd);
    c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().with_context(|| format!("запуск {}", argv[0]))?;
    let mut stdin = child.stdin.take();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let o1 = out.clone();
    let t1 = tokio::spawn(async move {
        let mut b = vec![0u8; 32768];
        while let Ok(n) = stdout.read(&mut b).await {
            if n == 0 || o1.send(ExecOut::Stdout(b[..n].to_vec())).await.is_err() {
                break;
            }
        }
    });
    let o2 = out.clone();
    let t2 = tokio::spawn(async move {
        let mut b = vec![0u8; 32768];
        while let Ok(n) = stderr.read(&mut b).await {
            if n == 0 || o2.send(ExecOut::Stderr(b[..n].to_vec())).await.is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        let code = loop {
            tokio::select! {
                st = child.wait() => break st.map(exit_code).unwrap_or(-1),
                m = inp.recv() => match m {
                    Some(ExecIn::Stdin(d)) => {
                        if let Some(s) = stdin.as_mut() {
                            let _ = s.write_all(&d).await;
                        }
                    }
                    Some(ExecIn::Eof) => stdin = None,
                    Some(ExecIn::Resize(..)) => {}
                    Some(ExecIn::Kill) | None => {
                        let _ = child.start_kill();
                        break child.wait().await.map(exit_code).unwrap_or(-1);
                    }
                },
            }
        };
        // Вывод дочитать, но не ждать вечно (фоновые внуки держат каналы).
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let _ = t1.await;
            let _ = t2.await;
        })
        .await;
        let _ = out.send(ExecOut::Exit(code)).await;
    });
    Ok(())
}

fn exit_code(st: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    st.code().unwrap_or_else(|| 128 + st.signal().unwrap_or(0))
}

fn set_winsize(fd: i32, cols: u16, rows: u16) {
    let ws = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
    unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &ws) };
}

fn spawn_pty(
    argv: &[String],
    cwd: Option<&str>,
    cols: u16,
    rows: u16,
    out: mpsc::Sender<ExecOut>,
    mut inp: mpsc::Receiver<ExecIn>,
) -> Result<()> {
    let (mut master, mut slave) = (0i32, 0i32);
    if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), std::ptr::null()) } != 0 {
        anyhow::bail!("openpty: {}", std::io::Error::last_os_error());
    }
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    set_winsize(master.as_raw_fd(), cols, rows);
    let mut c = command(argv, cwd);
    c.env("TERM", std::env::var("TERM").ok().filter(|t| !t.is_empty() && t != "dumb").unwrap_or_else(|| "xterm-256color".into()));
    c.stdin(Stdio::from(slave.try_clone()?)).stdout(Stdio::from(slave.try_clone()?)).stderr(Stdio::from(slave));
    unsafe {
        c.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::ioctl(0, libc::TIOCSCTTY, 0);
            Ok(())
        });
    }
    let mut child = c.spawn().with_context(|| format!("запуск {}", argv[0]))?;
    let mfd = master.as_raw_fd();
    // Чтение мастера — в своём потоке (блокирующий fd).
    let mut rd = std::fs::File::from(master.try_clone()?);
    let o = out.clone();
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut b = vec![0u8; 32768];
        loop {
            match rd.read(&mut b) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if o.blocking_send(ExecOut::Stdout(b[..n].to_vec())).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let mut wr = std::fs::File::from(master);
    tokio::spawn(async move {
        let code = loop {
            tokio::select! {
                st = child.wait() => break st.map(exit_code).unwrap_or(-1),
                m = inp.recv() => match m {
                    Some(ExecIn::Stdin(d)) => {
                        use std::io::Write;
                        let _ = wr.write_all(&d);
                    }
                    Some(ExecIn::Resize(c, r)) => set_winsize(mfd, c, r),
                    Some(ExecIn::Eof) => {}
                    Some(ExecIn::Kill) | None => {
                        let _ = child.start_kill();
                        break child.wait().await.map(exit_code).unwrap_or(-1);
                    }
                },
            }
        };
        drop(wr);
        let _ = tokio::task::spawn_blocking(move || {
            let _ = reader.join();
        })
        .await;
        let _ = out.send(ExecOut::Exit(code)).await;
    });
    Ok(())
}

/// Выполнить и собрать вывод (для CLI без потока и MCP).
pub async fn collect<W, R>(mut w: W, mut r: R, stdin: Option<Vec<u8>>, timeout: std::time::Duration) -> Result<(i32, Vec<u8>, Vec<u8>)>
where
    W: AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
{
    if let Some(d) = stdin {
        proto::send(&mut w, &ExecIn::Stdin(d)).await?;
    }
    proto::send(&mut w, &ExecIn::Eof).await?;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let deadline = tokio::time::Instant::now() + timeout;
    let code = loop {
        match tokio::time::timeout_at(deadline, proto::recv::<ExecOut>(&mut r)).await {
            Err(_) => {
                let _ = proto::send(&mut w, &ExecIn::Kill).await;
                err.extend_from_slice(t!("\n[synlink: тайм-аут {as_secs} с — процесс остановлен]\n", as_secs = timeout.as_secs()).as_bytes());
                break 124;
            }
            Ok(res) => match res? {
                Some(ExecOut::Started) => {}
                Some(ExecOut::Stdout(d)) => out.extend_from_slice(&d),
                Some(ExecOut::Stderr(d)) => err.extend_from_slice(&d),
                Some(ExecOut::Exit(c)) => break c,
                Some(ExecOut::Error(e)) => anyhow::bail!("{e}"),
                None => anyhow::bail!("соединение оборвалось"),
            },
        }
    };
    Ok((code, out, err))
}
