//! synnfcd — служба NFC (root): см. `synnfc::daemon`.

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "synnfc=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::tr_init(&[include_str!("../../i18n/en.lang")]);
    if let Err(e) = synnfc::daemon::run() {
        tracing::error!("synnfcd: {e:#}");
        std::process::exit(1);
    }
}
