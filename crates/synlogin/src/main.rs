//! synlogin — вход в synshell: демон (`synlogin daemon [-- аргументы synwm]`,
//! от root) и экран входа (`synlogin greeter`, запускает демон внутри
//! композитора). Выход из оболочки возвращает к экрану входа.

mod daemon;
mod greeter;
mod users;

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("daemon") => {
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
            daemon::run()
        }
        Some("greeter") => greeter::run(),
        _ => {
            eprintln!("synlogin daemon [-- аргументы synwm] | synlogin greeter");
            std::process::exit(2);
        }
    }
}
