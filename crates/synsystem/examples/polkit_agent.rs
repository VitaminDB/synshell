//! Проверка агента polkit без интерфейса: `SYN_PASS=пароль polkit_agent [команда…]`
//! (по умолчанию `pkexec true`); пароль отвечается на каждый запрос, попытки печатаются.
use synsystem::polkit_agent::{self, AuthRequest, Prompter};

struct Env;
impl Prompter for Env {
    fn ask(&self, req: AuthRequest) {
        eprintln!("запрос #{}: {} — пользователь {}, подсказка {:?}, ошибка {:?}", req.id, req.message, req.user, req.prompt, req.error);
        req.respond(std::env::var("SYN_PASS").ok());
    }
    fn cancel(&self, id: u64) {
        eprintln!("окно #{id} закрыто");
    }
}

fn main() {
    polkit_agent::set_prompter(Env);
    if let Err(e) = polkit_agent::ensure() {
        eprintln!("{e}");
        std::process::exit(1);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args = if args.is_empty() { vec!["true".to_string()] } else { args };
    let st = std::process::Command::new("pkexec").args(&args).status().expect("pkexec");
    eprintln!("pkexec: {st}");
}
