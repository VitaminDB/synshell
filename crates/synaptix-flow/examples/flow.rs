//! Прогон графа ядра из терминала:
//! `cargo run -p synaptix-flow --example flow -- граф.json [каталог] [--cancel-after секунд]`.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("использование: flow граф.json [каталог] [--cancel-after секунд]");
        std::process::exit(2);
    };
    let flow: synaptix_flow::Flow = serde_json::from_str(&std::fs::read_to_string(path).expect("граф")).expect("разбор графа");
    let dir = args.get(1).filter(|a| !a.starts_with("--")).map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("synaptix-flow-run"));
    let cancel = synaptix_flow::Cancel::new();
    if let Some(secs) = args.iter().position(|a| a == "--cancel-after").and_then(|i| args.get(i + 1)).and_then(|s| s.parse::<f64>().ok()) {
        let c = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs_f64(secs));
            eprintln!("— отмена");
            c.cancel();
        });
    }
    let t0 = std::time::Instant::now();
    let r = synaptix_flow::run(&flow, &dir, &cancel, &mut |e| eprintln!("[{:>6.1} с] {}", t0.elapsed().as_secs_f32(), serde_json::to_string(&e).unwrap_or_default()));
    match r {
        Ok(r) => {
            for s in &r.steps {
                println!("- {s}");
            }
            println!("файлы: {:?}", r.files);
        }
        Err(e) => {
            eprintln!("[{:>6.1} с] ошибка: {e}", t0.elapsed().as_secs_f32());
            std::process::exit(1);
        }
    }
}
