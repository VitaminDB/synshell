//! Экспорт рецепта шаблоном Synthos: `cargo run --example export -- <рецепт> [параметры.json]`.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let name = args.first().expect("рецепт");
    let dir = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share/synassist/recipes");
    let g = synaptix_graph::recipes::find(&dir, name).unwrap_or_else(|| panic!("рецепта {name} нет"));
    let params = args.get(1).map(|p| serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(p).expect("параметры")).expect("разбор")).and_then(|v| v.as_object().cloned()).unwrap_or_default();
    match synaptix_graph::synthos::export(&g, &params) {
        Ok(p) => println!("{}", p.display()),
        Err(e) => {
            eprintln!("ошибка: {e}");
            std::process::exit(1);
        }
    }
}
