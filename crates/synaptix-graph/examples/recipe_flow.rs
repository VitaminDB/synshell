//! Рецепт → граф ядра JSON (для `synaptixctl flow`): параметры по умолчанию,
//! поверх — переданные. `recipe_flow ИМЯ '{"параметр": значение}' > flow.json`
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let g = synaptix_graph::recipes::builtin().into_iter().find(|g| g.name == a[0]).expect("нет рецепта");
    let mut p: serde_json::Map<String, serde_json::Value> = serde_json::from_str(a.get(1).map(String::as_str).unwrap_or("{}")).expect("json");
    for (k, d) in &g.params {
        if !p.contains_key(k) {
            if let Some(v) = &d.default {
                p.insert(k.clone(), v.clone());
            }
        }
    }
    let f = synaptix_graph::compile::to_flow(&g, &p).expect("компиляция");
    println!("{}", serde_json::to_string_pretty(&f).expect("json"));
}
