//! syndroidd — демон Android-контейнера (root, `syndroid.service`).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("__bridge" | "__sensors" | "__ir" | "__notify")) {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
            .without_time()
            .init();
    }
    syn_droid::i18n_init();
    match args.first().map(String::as_str) {
        Some("__container") => syn_droid::container::starter_main(args.get(1).map_or("", String::as_str)),
        Some("__exec") => syn_droid::container::exec_main(&args[1..]),
        Some("__bridge") => syn_droid::bridge::bridge_main(&args[1..]),
        Some("__sensors") => syn_droid::sensors::sensors_main(&args[1..]),
        Some("__ir") => syn_droid::ir::ir_main(&args[1..]),
        Some("__notify") => syn_droid::notify::job_main(&args[1..]),
        _ => {}
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .without_time()
        .init();
    if let Err(e) = syn_droid::daemon::run() {
        eprintln!("syndroidd: {e:#}");
        std::process::exit(1);
    }
}
