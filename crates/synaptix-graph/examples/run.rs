//! Прогон графа или рецепта из терминала:
//! `cargo run --example run -- <рецепт|граф.json> [параметры.json] [каталог-результата]`.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(what) = args.first() else {
        eprintln!("использование: run <рецепт|граф.json> [параметры.json] [каталог]");
        std::process::exit(2);
    };
    let graph = if what.ends_with(".json") {
        serde_json::from_str(&std::fs::read_to_string(what).expect("граф")).expect("разбор графа")
    } else {
        let dir = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share/synassist/recipes");
        synaptix_graph::recipes::find(&dir, what).unwrap_or_else(|| panic!("рецепта {what} нет"))
    };
    let params = args.get(1).map(|p| serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(p).expect("параметры")).expect("разбор параметров")).and_then(|v| v.as_object().cloned()).unwrap_or_default();
    let dir = args.get(2).map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("synaptix-graph-run"));
    let t0 = std::time::Instant::now();
    match synaptix_graph::run(&graph, &params, &dir, &mut |s| eprintln!("[{:>6.1} с] {s}", t0.elapsed().as_secs_f32())) {
        Ok(r) => {
            for s in &r.steps {
                println!("- {s}");
            }
            println!("файлы: {:?}", r.files);
        }
        Err(e) => {
            eprintln!("ошибка: {e}");
            std::process::exit(1);
        }
    }
}
