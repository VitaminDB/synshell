fn main() {
    #[cfg(feature = "desktop")]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.first().map(String::as_str) == Some("--screenshot") {
            if let Err(e) = quitsmoke::screenshot(&args[1..]) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            return;
        }
    }
    quitsmoke::run_desktop();
}
