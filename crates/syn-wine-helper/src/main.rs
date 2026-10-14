//! syn-wine-helper — «Компоненты Wine» synshell: DLL, которых нет в самом Wine (MFC, рантаймы VB,
//! OpenAL…), ставятся один раз в общий каталог пользователя и видны всем префиксам
//! (`$XDG_DATA_HOME/syn-wine/dlls`, ищет наш Wine — pkgs/wine-arm64ec/syn-wine-helper.patch).
//! Не хватает DLL при запуске — Wine зовёт `syn-wine-helper missing …`: окно «поставить?», установка,
//! программа запускается снова.
//!
//!   syn-wine-helper                         окно со списком компонентов
//!   syn-wine-helper list                    пакеты и что поставлено
//!   syn-wine-helper install ПАКЕТ|DLL… [--arch x86|x64]
//!   syn-wine-helper remove ПАКЕТ…
//!   syn-wine-helper scan ПРОГРАММА.exe      каких DLL не хватит (без запуска)
//!   syn-wine-helper run ПРОГРАММА.exe [АРГ…]  проверить, предложить недостающее, запустить в Wine
//!   syn-wine-helper missing --machine M --exe … --cwd … --cmdline … -- DLL…   (из Wine)
//!   syn-wine-helper probe ФАЙЛ              какие DLL нашлись бы в установщике
//!   syn-wine-helper --screenshot out.png [--size 460x620] [--scale 2] [--page ask|list|busy|error]

mod cab;
mod db;
mod fetch;
mod install;
mod launch;
mod nsis;
mod pe;
mod scan;
mod store;
mod ui;
mod unpack;

use std::path::PathBuf;

use syngui::t;

use crate::db::Db;
use crate::pe::Arch;

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syn_wine_helper=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let mut args = std::env::args().skip(1);
    let Some(cmd) = args.next() else {
        ui::run(ui::Mode::List, None);
        return;
    };
    let code = match cmd.as_str() {
        "-h" | "--help" => {
            println!("{}", t!("syn-wine-helper [list | install ПАКЕТ|DLL… [--arch x86|x64] | remove ПАКЕТ… | scan ПРОГРАММА.exe | run ПРОГРАММА.exe [АРГ…] | probe ФАЙЛ]"));
            0
        }
        "list" => list(),
        "install" => cli_install(args.collect()),
        "remove" => cli_remove(args.collect()),
        "scan" => cli_scan(args.next()),
        "probe" => probe(args.next()),
        "run" => run(args.collect()),
        "missing" => {
            let f = launch::Failed::parse(&mut args);
            ui::run(ui::Mode::Ask, Some(f));
            0
        }
        "--screenshot" => {
            ui::screenshot(args.collect());
            0
        }
        other => {
            eprintln!("{}", t!("неизвестная команда «{c}» (см. --help)", c = other));
            2
        }
    };
    std::process::exit(code);
}

fn list() -> i32 {
    let db = Db::load();
    println!("{}", t!("Каталог DLL: {d}", d = store::dll_root().display()));
    for p in &db.packages {
        let st: Vec<String> = p.archs().iter().map(|a| format!("{}{}", a.name(), if store::installed(p, *a) { "✓" } else { "" })).collect();
        println!("{:<22} {:<14} {:<26} {}", p.id, st.join(" "), p.name, p.dlls.join(", "));
    }
    0
}

/// Пакеты по id или имени DLL; архитектуры — из `--arch` или все, что есть у пакета.
fn resolve(db: &Db, args: Vec<String>) -> Result<Vec<(String, Arch)>, String> {
    let mut archs: Option<Vec<Arch>> = None;
    let mut names = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "--arch" {
            let v = it.next().unwrap_or_default();
            archs = Some(v.split(',').filter_map(Arch::parse).collect());
        } else {
            names.push(a);
        }
    }
    let mut items = Vec::new();
    for n in names {
        let pkg = db.package(&n).or_else(|| db.packages.iter().find(|p| p.provides(&n))).ok_or_else(|| t!("нет пакета или DLL «{n}»", n = n))?;
        for a in archs.clone().unwrap_or_else(|| pkg.archs()) {
            if pkg.pick(a).is_some() && !items.contains(&(pkg.id.clone(), a)) {
                items.push((pkg.id.clone(), a));
            }
        }
    }
    Ok(items)
}

fn cli_install(args: Vec<String>) -> i32 {
    let db = Db::load();
    let items = match resolve(&db, args) {
        Ok(i) if !i.is_empty() => i,
        Ok(_) => {
            eprintln!("{}", t!("что поставить? (syn-wine-helper list)"));
            return 2;
        }
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut last = String::new();
    let r = install::install(&db, &items, &mut |p| {
        if p.stage != last {
            eprintln!("{}", p.stage);
            last = p.stage;
        }
    });
    match r {
        Ok(files) => {
            for f in files {
                println!("{}", f.display());
            }
            0
        }
        Err(e) => {
            eprintln!("{e:#}");
            1
        }
    }
}

fn cli_remove(args: Vec<String>) -> i32 {
    let db = Db::load();
    let mut code = 0;
    for id in args {
        match install::remove(&db, &id) {
            Ok(n) => println!("{}", t!("{id}: удалено файлов — {n}", id = &id, n = n)),
            Err(e) => {
                eprintln!("{e:#}");
                code = 1;
            }
        }
    }
    code
}

fn cli_scan(exe: Option<String>) -> i32 {
    let Some(exe) = exe else {
        eprintln!("{}", t!("какую программу проверить?"));
        return 2;
    };
    let db = Db::load();
    match scan::scan(&PathBuf::from(&exe)) {
        Ok(r) if r.missing.is_empty() => {
            println!("{}", t!("{exe} ({arch}): всех DLL хватает", exe = &exe, arch = r.arch.name()));
            0
        }
        Ok(r) => {
            for m in &r.missing {
                match db.for_dll(&m.dll, r.arch) {
                    Some(p) => println!("{} ({}) ← {}: {}", m.dll, r.arch.name(), m.needed_by, t!("пакет {id}", id = &p.id)),
                    None => println!("{} ({}) ← {}: {}", m.dll, r.arch.name(), m.needed_by, t!("в базе нет")),
                }
            }
            1
        }
        Err(e) => {
            eprintln!("{e:#}");
            2
        }
    }
}

fn probe(file: Option<String>) -> i32 {
    let Some(file) = file else { return 2 };
    let data = match std::fs::read(&file) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{file}: {e}");
            return 1;
        }
    };
    unpack::walk(&file, &data, &mut |path, blob| {
        if let Some(i) = pe::parse(blob) {
            if i.is_dll {
                println!("{:<6} {:<28} {:>9}  {path}", i.arch().map(Arch::name).unwrap_or("?"), i.export_name.as_deref().unwrap_or("-"), blob.len());
            }
        }
        unpack::Flow::Continue
    });
    0
}

/// Запуск программы с проверкой: не хватает известных DLL — окно «поставить?», затем Wine.
fn run(args: Vec<String>) -> i32 {
    let Some(exe) = args.first() else {
        eprintln!("{}", t!("какую программу запустить?"));
        return 2;
    };
    let exe_path = PathBuf::from(exe);
    if let Ok(r) = scan::scan(&exe_path) {
        if !r.missing.is_empty() {
            let f = launch::Failed {
                machine: match r.arch {
                    Arch::X86 => 0x14c,
                    Arch::X64 => 0x8664,
                },
                exe: exe_path.to_string_lossy().into_owned(),
                dlls: r.missing.iter().map(|m| m.dll.clone()).collect(),
                argv: args.clone(),
                ..Default::default()
            };
            if !ui::run(ui::Mode::AskBeforeRun, Some(f)) {
                return 1;
            }
        }
    }
    let wine = std::env::var("WINELOADER").unwrap_or_else(|_| "wine".into());
    match std::process::Command::new(wine).args(&args).status() {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("wine: {e}");
            1
        }
    }
}
