//! synmodemd — демон модема (root, `synmodem.service`).

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .without_time()
        .init();
    if let Err(e) = synmodem::daemon::run() {
        eprintln!("synmodemd: {e:#}");
        std::process::exit(1);
    }
}
