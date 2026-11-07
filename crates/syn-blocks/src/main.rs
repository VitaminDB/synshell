//! Точка входа desktop-сборки.
fn main() {
    #[cfg(not(target_os = "android"))]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.first().map(String::as_str) == Some("--screenshot") {
            if let Err(e) = syn_blocks::screenshot(&args[1..]) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            return;
        }
    }
    syn_blocks::run_desktop();
}
