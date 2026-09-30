//! synlink — связь оболочек synshell (телефон ↔ компьютер) по USB и Wi-Fi:
//! трансляция экрана и управление, снимки, файлы друг друга, уведомления,
//! ssh и отладка (CLI и MCP-сервер для Claude Code).

mod control;
mod daemon;
mod discovery;
mod exec;
mod fs;
mod fuse;
mod gestures;
mod identity;
mod mcp;
mod net;
mod netif;
mod notify;
mod proto;
mod rpc;
mod screen;
mod services;
mod ssh;

use std::io::{Read, Write};

use anyhow::{bail, Context, Result};
use synshell_common::link::{self, ExecFrame, Request, Response};

const USAGE: &str = "\
synlink — связь оболочек synshell (телефон ↔ компьютер) по USB и Wi-Fi

synlink daemon                         демон (запускается сеансом сам)
synlink status [--json]                эта машина, кабель, устройства
synlink events                         события демона (JSON)
synlink pair УСТР | pair accept|reject УСТР   спаривание
synlink unpair УСТР | disconnect УСТР
synlink shot УСТР [-o ФАЙЛ] [--max N] [--output ВЫВОД]   снимок экрана (PNG)
synlink tap УСТР X Y | hold УСТР X Y [МС] | swipe УСТР X1 Y1 X2 Y2 [МС]
synlink scroll УСТР X Y ШАГИ           колесо (вниз — положительные)
synlink type УСТР ТЕКСТ | key УСТР СОЧЕТАНИЕ | action УСТР ДЕЙСТВИЕ
synlink windows УСТР | outputs УСТР | wm УСТР JSON
synlink exec УСТР [--cwd К] -- КОМАНДА...   команда (вывод потоком, код выхода)
synlink shell УСТР                     терминал на устройстве
synlink logs УСТР [-f] [-n N] [-u ЮНИТ] [--grep Ш]   журнал systemd
synlink proxy УСТР [ПОРТ]              stdin/stdout ↔ 127.0.0.1:ПОРТ (по умолч. sshd)
synlink mount УСТР | umount УСТР       файлы устройства (FUSE)
synlink notifications [УСТР]           уведомления с устройств
synlink view УСТР                      окно трансляции (synlink-view)
synlink mcp                            MCP-сервер (stdio) для Claude Code

X Y — пиксели снимка экрана устройства или доли 0..1. УСТР — имя, id,
phone/desktop или local (эта машина). ssh: `ssh phone` / `ssh <имя>`.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let res = match args.first().map(String::as_str) {
        Some("daemon") => run_daemon(),
        Some("mcp") => mcp::run(),
        Some("-h" | "--help" | "help") | None => {
            println!("{USAGE}");
            Ok(())
        }
        Some(_) => {
            // CLI в конвейере (`synlink status | head`): закрытый вывод — выход, не паника.
            unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
            cli(&args)
        }
    };
    if let Err(e) = res {
        eprintln!("synlink: {e:#}");
        std::process::exit(1);
    }
}

fn run_daemon() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("SYNLINK_LOG").unwrap_or_else(|_| "info,quinn=warn,zbus=warn".into()))
        .with_writer(std::io::stderr)
        .init();
    let (cfg, err) = synshell_common::config::Config::load();
    if let Some(e) = err {
        tracing::warn!("конфиг: {e}");
    }
    if !cfg.link.enabled {
        tracing::info!("[link] enabled = false — выход");
        return Ok(());
    }
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(4).build()?;
    rt.block_on(async move {
        let id = identity::Identity::load_or_create()?;
        let (d, notes_rx) = daemon::Daemon::new(id, cfg)?;
        tracing::info!(id = %d.id.id, name = %d.self_info().name, kind = ?d.kind, port = d.port, "synlink");
        ssh::write_config(&d.trusted_all());
        notify::start(d.clone(), notes_rx);
        tokio::spawn(discovery::run(d.clone()));
        tokio::spawn(d.clone().accept_loop());
        tokio::spawn(d.clone().ticker());
        let ctl = tokio::spawn(control::run(d.clone()));
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let res = tokio::select! {
            r = ctl => r.map_err(anyhow::Error::from).and_then(|r| r),
            _ = tokio::signal::ctrl_c() => Ok(()),
            _ = term.recv() => Ok(()),
        };
        tracing::info!("выход");
        fuse::unmount_all();
        ssh::stop_user_sshd();
        d.ep.close(0u32.into(), b"exit");
        let _ = std::fs::remove_file(link::socket_path());
        res
    })
}

fn req(r: &Request) -> Result<Response> {
    gestures::req(r)
}

fn num(s: Option<&String>, what: &str) -> Result<f64> {
    s.with_context(|| format!("нужно {what}"))?.parse().with_context(|| format!("{what}: не число"))
}

fn dev(args: &[String]) -> Result<String> {
    args.get(1).cloned().context("нужно устройство (имя, id, phone или local)")
}

fn cli(args: &[String]) -> Result<()> {
    let cmd = args[0].as_str();
    match cmd {
        "status" => {
            let Response::Status { status } = req(&Request::Status)? else { bail!("нет состояния") };
            if args.iter().any(|a| a == "--json") {
                println!("{}", serde_json::to_string_pretty(&status)?);
                return Ok(());
            }
            println!("{} «{}» ({})", status.me.kind.title(), status.me.name, status.me.id);
            let u = &status.usb;
            println!(
                "USB: {}{}",
                if u.cable { "кабель подключён" } else { "нет кабеля" },
                u.interface.as_ref().map(|i| format!(" ({i} {})", u.address.clone().unwrap_or_default())).unwrap_or_default()
            );
            if status.peers.is_empty() {
                println!("устройств нет");
            }
            for p in &status.peers {
                let state = if p.connected {
                    format!("соединено по {}", p.transport.map(|t| t.title()).unwrap_or("?"))
                } else if p.paired {
                    "спарено, не в сети".into()
                } else {
                    format!("рядом ({}), не спарено", p.transport.map(|t| t.title()).unwrap_or("?"))
                };
                let mut extra = Vec::new();
                if let Some(r) = p.rtt_ms {
                    extra.push(format!("{r:.1} мс"));
                }
                if let Some(b) = &p.battery {
                    extra.push(format!("батарея {}%{}", b.percent, if b.charging { "⚡" } else { "" }));
                }
                if let Some(m) = p.files_path() {
                    extra.push(format!("файлы {m}"));
                }
                if let (true, Some(h)) = (p.paired, &p.ssh_host) {
                    extra.push(format!("ssh {h}"));
                }
                println!(
                    "  {} «{}» [{}] — {state}{}",
                    p.kind.title(),
                    p.name,
                    &p.id[..8.min(p.id.len())],
                    if extra.is_empty() { String::new() } else { format!(" · {}", extra.join(" · ")) }
                );
            }
            for pr in &status.prompts {
                println!(
                    "  ожидает спаривания: «{}» по {}{} — synlink pair accept {}",
                    pr.name,
                    pr.transport.title(),
                    pr.code.as_ref().map(|c| format!(", код {c}")).unwrap_or_default(),
                    &pr.id[..8]
                );
            }
        }
        "events" => {
            for e in link::Client::connect()?.subscribe()? {
                println!("{}", serde_json::to_string(&e?)?);
            }
        }
        "pair" => match args.get(1).map(String::as_str) {
            Some(a @ ("accept" | "reject")) => {
                let d = args.get(2).context("нужно устройство")?;
                req(&Request::PairReply { device: d.clone(), accept: a == "accept" })?;
            }
            _ => {
                req(&Request::Pair { device: dev(args)? })?;
                println!("запрос отправлен — подтвердите на устройстве (сверьте код)");
            }
        },
        "unpair" => {
            req(&Request::Unpair { device: dev(args)? })?;
        }
        "disconnect" => {
            req(&Request::Disconnect { device: dev(args)? })?;
        }
        "shot" => {
            let device = dev(args)?;
            let mut path = None;
            let mut max = None;
            let mut output = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "-o" => {
                        path = args.get(i + 1).map(|p| {
                            std::path::absolute(p).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| p.clone())
                        });
                        i += 1;
                    }
                    "--max" => {
                        max = args.get(i + 1).and_then(|m| m.parse().ok());
                        i += 1;
                    }
                    "--output" => {
                        output = args.get(i + 1).cloned();
                        i += 1;
                    }
                    _ => {}
                }
                i += 1;
            }
            let Response::Screenshot { path, width, height, output } =
                req(&Request::Screenshot { device, output, path, max_size: max })?
            else {
                bail!("нет снимка")
            };
            println!("{path} ({width}×{height}, {output})");
        }
        "tap" | "hold" => {
            let device = dev(args)?;
            let s = gestures::screen(&device, None)?;
            let (x, y) = (num(args.get(2), "X")?, num(args.get(3), "Y")?);
            if cmd == "tap" {
                gestures::tap(&device, &s, x, y)?;
            } else {
                gestures::hold(&device, &s, x, y, args.get(4).and_then(|m| m.parse().ok()).unwrap_or(700))?;
            }
        }
        "swipe" => {
            let device = dev(args)?;
            let s = gestures::screen(&device, None)?;
            let a = (num(args.get(2), "X1")?, num(args.get(3), "Y1")?);
            let b = (num(args.get(4), "X2")?, num(args.get(5), "Y2")?);
            gestures::swipe(&device, &s, a, b, args.get(6).and_then(|m| m.parse().ok()).unwrap_or(300))?;
        }
        "scroll" => {
            let device = dev(args)?;
            let s = gestures::screen(&device, None)?;
            gestures::scroll(&device, &s, num(args.get(2), "X")?, num(args.get(3), "Y")?, num(args.get(4), "ШАГИ")?, 0.0)?;
        }
        "type" => gestures::text(&dev(args)?, &args[2..].join(" "))?,
        "key" => gestures::combo(&dev(args)?, args.get(2).context("нужно сочетание, например Alt+Tab")?)?,
        "action" => gestures::action(&dev(args)?, &args[2..].join(" "))?,
        "windows" | "outputs" | "wm" => {
            let device = dev(args)?;
            let wm = match cmd {
                "windows" => synshell_common::ipc::Request::Windows,
                "outputs" => synshell_common::ipc::Request::Outputs,
                _ => serde_json::from_str(args.get(2).context("нужен JSON запроса композитору")?)?,
            };
            let Response::Wm { wm } = req(&Request::Wm { device, wm })? else { bail!("нет ответа") };
            println!("{}", serde_json::to_string_pretty(&wm)?);
        }
        "exec" => {
            let device = dev(args)?;
            let mut cwd = None;
            let mut i = 2;
            while i < args.len() && args[i] != "--" {
                if args[i] == "--cwd" {
                    cwd = args.get(i + 1).cloned();
                    i += 1;
                }
                i += 1;
            }
            let argv: Vec<String> = args.get(i + 1..).map(|a| a.to_vec()).unwrap_or_default();
            if argv.is_empty() {
                bail!("нужна команда после --");
            }
            let code = exec_stream(&device, argv, cwd, None)?;
            std::process::exit(code);
        }
        "shell" => {
            let device = dev(args)?;
            let code = shell(&device)?;
            std::process::exit(code);
        }
        "logs" => {
            let device = dev(args)?;
            let mut argv = vec!["journalctl".to_string(), "--no-pager".into(), "-o".into(), "short-iso".into()];
            let mut n = "200".to_string();
            let mut grep = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "-f" => argv.push("-f".into()),
                    "-n" => {
                        n = args.get(i + 1).cloned().unwrap_or(n);
                        i += 1;
                    }
                    "-u" => {
                        argv.push("-u".into());
                        argv.push(args.get(i + 1).cloned().unwrap_or_default());
                        i += 1;
                    }
                    "--grep" => {
                        grep = args.get(i + 1).cloned();
                        i += 1;
                    }
                    _ => {}
                }
                i += 1;
            }
            argv.push("-n".into());
            argv.push(n);
            if let Some(g) = grep {
                argv.push("-g".into());
                argv.push(g);
            }
            let code = exec_stream(&device, argv, None, None)?;
            std::process::exit(code);
        }
        "proxy" => {
            let device = dev(args)?;
            let port: u16 = args.get(2).and_then(|p| p.parse().ok()).unwrap_or(0);
            proxy(&device, port)?;
        }
        "mount" | "umount" => {
            let Response::Mount { path } = req(&Request::Mount { device: dev(args)?, mount: cmd == "mount" })? else {
                bail!("нет ответа")
            };
            if let Some(p) = path {
                println!("{p}");
            }
        }
        "notifications" => {
            let Response::Notifications { notifications } = req(&Request::Notifications { device: args.get(1).cloned() })? else {
                bail!("нет ответа")
            };
            for n in notifications {
                println!("[{}] {} — {}: {}", n.device, n.app, n.summary, n.body);
            }
        }
        "view" => {
            let device = dev(args)?;
            let exe = std::env::current_exe()?.with_file_name("synlink-view");
            let err = std::os::unix::process::CommandExt::exec(std::process::Command::new(exe).arg(device));
            bail!("synlink-view: {err}");
        }
        other => bail!("неизвестная команда «{other}» (synlink --help)"),
    }
    Ok(())
}

/// Команда с выводом потоком; код выхода.
fn exec_stream(device: &str, argv: Vec<String>, cwd: Option<String>, pty: Option<[u16; 2]>) -> Result<i32> {
    let mut c = link::Client::connect().context("synlink не запущен")?;
    match c.request(&Request::Exec { device: device.into(), argv, cwd, stream: true, pty, timeout_ms: None })? {
        Response::Ok => {}
        Response::Error { message } => bail!("{message}"),
        other => bail!("неожиданный ответ: {other:?}"),
    }
    let (mut rd, mut wr) = c.into_parts();
    // stdin → устройство (в своём потоке).
    let mut w2 = wr.try_clone()?;
    let raw = pty.is_some();
    std::thread::spawn(move || {
        let mut b = [0u8; 8192];
        let mut stdin = std::io::stdin().lock();
        loop {
            match stdin.read(&mut b) {
                Ok(0) | Err(_) => {
                    let _ = link::write_frame(&mut w2, &serde_json::to_vec(&ExecFrame::Eof).unwrap());
                    break;
                }
                Ok(n) => {
                    let f = ExecFrame::Stdin { data: b[..n].to_vec() };
                    if link::write_frame(&mut w2, &serde_json::to_vec(&f).unwrap()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    if raw {
        // Размер терминала — при SIGWINCH.
        let mut w3 = wr.try_clone()?;
        std::thread::spawn(move || {
            let mut last = term_size();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(250));
                let now = term_size();
                if now != last {
                    last = now;
                    if let Some([cols, rows]) = now {
                        let f = ExecFrame::Resize { cols, rows };
                        if link::write_frame(&mut w3, &serde_json::to_vec(&f).unwrap()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
    }
    let _ = &mut wr;
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    while let Some(f) = link::read_frame(&mut rd)? {
        match serde_json::from_slice::<ExecFrame>(&f)? {
            ExecFrame::Stdout { data } => {
                out.write_all(&data)?;
                out.flush()?;
            }
            ExecFrame::Stderr { data } => {
                err.write_all(&data)?;
                err.flush()?;
            }
            ExecFrame::Exit { code } => return Ok(code),
            ExecFrame::Error { message } => bail!("{message}"),
            _ => {}
        }
    }
    bail!("соединение оборвалось")
}

fn term_size() -> Option<[u16; 2]> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(0, libc::TIOCGWINSZ, &mut ws) } == 0 && ws.ws_col > 0 {
        Some([ws.ws_col, ws.ws_row])
    } else {
        None
    }
}

/// Терминал: локальный tty в сырой режим, PTY на устройстве.
fn shell(device: &str) -> Result<i32> {
    let size = term_size().unwrap_or([80, 24]);
    let mut old: libc::termios = unsafe { std::mem::zeroed() };
    let is_tty = unsafe { libc::tcgetattr(0, &mut old) } == 0;
    if is_tty {
        let mut raw = old;
        unsafe { libc::cfmakeraw(&mut raw) };
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) };
    }
    let argv = vec!["/bin/sh".to_string(), "-c".into(), "exec ${SHELL:-/bin/bash} -l".into()];
    let res = exec_stream(device, argv, None, Some(size));
    if is_tty {
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &old) };
    }
    res
}

/// Туннель: stdin/stdout ↔ порт устройства (ProxyCommand для ssh).
fn proxy(device: &str, port: u16) -> Result<()> {
    let mut c = link::Client::connect().context("synlink не запущен")?;
    match c.request(&Request::Tcp { device: device.into(), port })? {
        Response::Ok => {}
        Response::Error { message } => bail!("{message}"),
        other => bail!("неожиданный ответ: {other:?}"),
    }
    let (mut rd, mut wr) = c.into_parts();
    // Без буферов stdio: stdout построчный, а поток ssh двоичный.
    use std::os::fd::FromRawFd;
    let mut stdin = unsafe { std::fs::File::from_raw_fd(0) };
    let mut stdout = unsafe { std::fs::File::from_raw_fd(1) };
    let t = std::thread::spawn(move || {
        let mut b = vec![0u8; 65536];
        loop {
            match stdin.read(&mut b) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if wr.write_all(&b[..n]).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = wr.shutdown(std::net::Shutdown::Write);
        std::mem::forget(stdin);
    });
    let mut b = vec![0u8; 65536];
    loop {
        match rd.read(&mut b) {
            Ok(0) | Err(_) => break,
            Ok(n) => stdout.write_all(&b[..n])?,
        }
    }
    std::mem::forget(stdout);
    drop(t);
    Ok(())
}
