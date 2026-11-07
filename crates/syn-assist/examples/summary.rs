//! Подпись действия инструмента на языке интерфейса — как её покажет
//! ассистент (демо-чаты для снимков README, проверка переводов):
//!
//!     SYNSHELL_LANG=en cargo run --example summary -- files '{"action":"list","path":"~/Downloads"}'
//!     → Folder ~/Downloads

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(name), Some(json)) = (args.next(), args.next()) else {
        eprintln!("summary ИНСТРУМЕНТ АРГУМЕНТЫ_JSON");
        std::process::exit(2);
    };
    let lang = ["SYNSHELL_LANG", "LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|k| std::env::var(k).ok()).find(|v| !v.is_empty()).unwrap_or_default();
    synshell_tr::init("ru", &lang, syn_assist::CATALOGS);
    let args: serde_json::Value = serde_json::from_str(&json).unwrap_or_else(|e| {
        eprintln!("аргументы не JSON: {e}");
        std::process::exit(2);
    });
    match syn_assist::builtin::all().into_iter().find(|t| t.name() == name) {
        Some(t) => println!("{}", t.summary(&args)),
        None => {
            eprintln!("нет инструмента «{name}»");
            std::process::exit(1);
        }
    }
}
