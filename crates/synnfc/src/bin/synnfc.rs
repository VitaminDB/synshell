//! synnfc — NFC из терминала: состояние, чтение меток, запись ссылки или текста, эмуляция.
//!
//!   synnfc status | read | write-url URL | write-text ТЕКСТ | emulate-url URL | stop

use synnfc::api::{self, Event, Request, Subscription};
use synnfc::ndef::Record;
use synshell_common::t;

fn usage() -> ! {
    eprintln!("{}", t!("synnfc status | read | write-url URL | write-text ТЕКСТ | emulate-url URL"));
    std::process::exit(2);
}

fn print_tag(tg: &api::Tag) {
    println!("{} {} · {} · {}", t!("метка"), tg.uid, tg.kind, tg.protocol);
    if !tg.error.is_empty() {
        println!("  {}", t!("ошибка: {e}", e = tg.error));
    }
    match &tg.ndef {
        Some(recs) if recs.is_empty() => println!("  {}", t!("NDEF: пусто")),
        Some(recs) => {
            for r in recs {
                println!("  {r:?}");
            }
        }
        None => println!("  {}", t!("без NDEF")),
    }
}

fn watch(req: Request) -> anyhow::Result<()> {
    let mut sub = Subscription::open()?;
    sub.sender()?.send(&req)?;
    println!("{}", t!("поднесите метку… (Ctrl+C — выход)"));
    loop {
        match sub.next_event()? {
            Event::Tag(tg) => print_tag(&tg),
            Event::Written { uid, ok, error } => {
                if ok {
                    println!("{}", t!("записано на {uid}", uid = uid));
                } else {
                    println!("{}", t!("не записано на {uid}: {error}", uid = uid, error = error));
                }
            }
            Event::EmulationRead => println!("{}", t!("эмулируемую метку прочитали")),
            Event::Status(s) => {
                if !s.error.is_empty() {
                    println!("{}", t!("ошибка: {e}", e = s.error));
                }
            }
        }
    }
}

fn main() {
    synshell_common::tr_init(&[include_str!("../../i18n/en.lang")]);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["status"] | [] => api::request(&Request::Status).map(|r| {
            let s = r.status.unwrap_or_default();
            println!("{}: {} · {} · {:?}{}", t!("контроллер"), s.device, s.controller, s.mode, if s.error.is_empty() { String::new() } else { format!(" · {}", s.error) });
        }),
        ["read"] => watch(Request::Read),
        ["write-url", url] => watch(Request::Write { records: vec![Record::Uri { uri: url.to_string() }] }),
        ["write-text", text @ ..] if !text.is_empty() => watch(Request::Write { records: vec![Record::Text { text: text.join(" "), lang: String::new() }] }),
        ["emulate-url", url] => watch(Request::Emulate { records: vec![Record::Uri { uri: url.to_string() }] }),
        _ => usage(),
    };
    if let Err(e) = r {
        eprintln!("synnfc: {e:#}");
        std::process::exit(1);
    }
}
