//! syn-steam-setup — первая установка нативного aarch64-клиента Steam (пакет syn-steam). Клиента Valve в пакете нет:
//! запускатель `steam` при первом запуске зовёт эту программу, она скачивает компоненты по манифесту
//! `steam_client_linuxarm64` с CDN Valve (размер и sha256 — из манифеста) и раскладывает их в каталог клиента
//! (`~/.local/share/Steam`); дальше клиент обновляет себя сам.
//!
//!   syn-steam-setup [--root КАТАЛОГ] [--channel stable|publicbeta]          окно: согласие, ход загрузки
//!   syn-steam-setup --no-window [--root …] [--channel …]                    то же в терминале
//!   syn-steam-setup --check [--root …]                                      0 — клиент поставлен
//!   syn-steam-setup --manifest [--channel …]                                что будет скачано
//!   syn-steam-setup --screenshot out.png [--size ШxВ] [--scale N] [--page ask|busy|offline|error] [--phone]
//!
//! Код выхода: 0 — клиент поставлен, 1 — ошибка, 2 — отменено.

mod install;
mod manifest;
mod ui;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use syngui::t;

use crate::install::Step;
use crate::manifest::Channel;

pub struct Opts {
    pub root: PathBuf,
    pub channel: Channel,
}

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "syn_steam_setup=info".into());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
    synshell_common::i18n::init(&synshell_common::catalogs!("../i18n"));

    let mut root = None;
    let mut channel = std::env::var("STEAM_ARM64_CHANNEL")
        .ok()
        .and_then(|c| Channel::parse(&c))
        .unwrap_or(Channel::Stable);
    let mut cmd = "window";
    let mut rest = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--root" => root = args.next().map(PathBuf::from),
            "--channel" => match args.next().as_deref().and_then(Channel::parse) {
                Some(c) => channel = c,
                None => {
                    eprintln!("{}", t!("канал: stable | publicbeta"));
                    std::process::exit(2);
                }
            },
            "--no-window" => cmd = "cli",
            "--check" => cmd = "check",
            "--manifest" => cmd = "manifest",
            "--screenshot" => {
                cmd = "screenshot";
                rest.extend(args.by_ref());
            }
            "-h" | "--help" => {
                println!("{}", t!("syn-steam-setup [--root КАТАЛОГ] [--channel stable|publicbeta] [--no-window | --check | --manifest]"));
                return;
            }
            other => {
                eprintln!("{}", t!("неизвестный ключ «{a}» (см. --help)", a = other));
                std::process::exit(2);
            }
        }
    }
    let opts = Opts {
        root: root.unwrap_or_else(install::default_root),
        channel,
    };
    let code = match cmd {
        "check" => i32::from(!install::installed(&opts.root)),
        "manifest" => print_manifest(&opts),
        "cli" => cli(&opts),
        "screenshot" => {
            ui::screenshot(rest);
            0
        }
        _ => {
            if install::installed(&opts.root) {
                0
            } else {
                ui::run(opts)
            }
        }
    };
    std::process::exit(code);
}

fn mb(n: u64) -> String {
    synshell_common::decimal(n as f64 / 1048576.0, 1)
}

fn print_manifest(o: &Opts) -> i32 {
    match install::fetch_manifest(o.channel) {
        Ok((m, cdn)) => {
            println!(
                "{}",
                t!(
                    "Steam {v} ({c}), {cdn}",
                    v = &m.version,
                    c = o.channel.name(),
                    cdn = cdn
                )
            );
            for c in m.selected() {
                println!("{:<40} {:>12}  {}  {}", c.name, c.size, c.sha256, c.file);
            }
            println!("{}", t!("всего: {n} МБ", n = mb(m.download_size())));
            0
        }
        Err(e) => {
            eprintln!("{e:#}");
            1
        }
    }
}

fn cli(o: &Opts) -> i32 {
    if install::installed(&o.root) {
        println!("{}", t!("клиент уже поставлен: {r}", r = o.root.display()));
        return 0;
    }
    eprintln!(
        "{}",
        t!(
            "Загружая клиент, вы принимаете Соглашение подписчика Steam: {u}",
            u = ui::SSA_URL
        )
    );
    let cancel = AtomicBool::new(false);
    let mut last = (usize::MAX, 0u64);
    let r = install::install(&o.root, o.channel, &cancel, &mut |s| match s {
        Step::Download {
            done,
            total,
            index,
            count,
        } => {
            // раз в 10 МБ и на каждом компоненте
            if index != last.0 || done >= last.1 + (10 << 20) || done == total {
                last = (index, done);
                eprintln!(
                    "{}",
                    t!(
                        "скачивание {i}/{n}: {d} из {t} МБ",
                        i = index + 1,
                        n = count,
                        d = mb(done),
                        t = mb(total)
                    )
                );
            }
        }
        Step::Unpack { index, count } => {
            eprintln!("{}", t!("распаковка {i}/{n}", i = index + 1, n = count))
        }
    });
    match r {
        Ok(m) => {
            println!(
                "{}",
                t!(
                    "Steam {v} поставлен в {r}",
                    v = &m.version,
                    r = o.root.display()
                )
            );
            0
        }
        Err(e) if install::is_offline(&e) => {
            eprintln!("{}\n{e:#}", ui::offline_text());
            1
        }
        Err(e) => {
            eprintln!("{e:#}");
            1
        }
    }
}
