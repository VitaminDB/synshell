//! syndroidd — демон Android-контейнера (root, `syndroid.service`).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("__bridge") {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
            .without_time()
            .init();
    }
    match args.first().map(String::as_str) {
        Some("__container") => syndroid::container::starter_main(args.get(1).map_or("", String::as_str)),
        Some("__exec") => syndroid::container::exec_main(&args[1..]),
        Some("__bridge") => syndroid::bridge::bridge_main(&args[1..]),
        _ => {}
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .without_time()
        .init();
    if let Err(e) = syndroid::daemon::run() {
        eprintln!("syndroidd: {e:#}");
        std::process::exit(1);
    }
}
