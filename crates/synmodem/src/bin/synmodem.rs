//! synmodem — команды демона модема: состояние, SMS, звонки, радио.

use anyhow::{bail, Result};
use synmodem::api::{self, Request, Response};

const USAGE: &str = "synmodem status | watch | radio on|off | data on|off | sms [list] | sms send НОМЕР ТЕКСТ… | sms read НОМЕР
         | dial НОМЕР | answer ID | hangup ID | dtmf ID ЦИФРА | calls | info | cell | esim | apn
         | req JSON (любой запрос протокола, например {\"request\":\"modes\"}; ответ — JSON)";

fn main() {
    if let Err(e) = run() {
        eprintln!("synmodem: {e:#}");
        std::process::exit(1);
    }
}

fn print_resp(r: &Request) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&api::request(r)?)?);
    Ok(())
}

fn ok(r: &Request) -> Result<()> {
    api::request(r).map(|_| ())
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        [] | ["status"] => println!("{}", serde_json::to_string_pretty(&api::status()?)?),
        ["watch"] => api::subscribe(|ev| {
            println!("{}", serde_json::to_string(&ev).unwrap_or_default());
            true
        })?,
        ["radio", v @ ("on" | "off")] => ok(&Request::SetRadio { on: *v == "on" })?,
        ["data", v @ ("on" | "off")] => ok(&Request::SetData { on: *v == "on" })?,
        ["sms"] | ["sms", "list"] => {
            for m in api::sms_list()? {
                let dir = if m.incoming { "←" } else { "→" };
                println!("{} {dir} {} [{:?}{}] {}", m.id, m.number, m.status, if m.read { "" } else { ", новое" }, m.text);
            }
        }
        ["sms", "send", number, text @ ..] if !text.is_empty() => {
            if let Response::Sms { message } = api::request(&Request::SmsSend { number: number.to_string(), text: text.join(" ") })? {
                println!("{} поставлено в отправку", message.id);
            }
        }
        ["sms", "read", number] => ok(&Request::SmsRead { number: number.to_string() })?,
        ["dial", number] => {
            if let Response::CallId { id } = api::request(&Request::Dial { number: number.to_string() })? {
                println!("звонок {id}");
            }
        }
        ["answer", id] => ok(&Request::Answer { id: id.parse()? })?,
        ["hangup", id] => ok(&Request::Hangup { id: id.parse()? })?,
        ["dtmf", id, d] if d.chars().count() == 1 => {
            ok(&Request::Dtmf { id: id.parse()?, digit: d.chars().next().unwrap() })?
        }
        ["info"] => print_resp(&Request::Info)?,
        ["cell"] => print_resp(&Request::Cell)?,
        ["esim"] => print_resp(&Request::EsimProfiles)?,
        ["apn"] => print_resp(&Request::ApnList)?,
        ["req", json] => print_resp(&serde_json::from_str(json)?)?,
        ["calls"] => {
            for c in api::call_log()? {
                println!("{} {:?} {} {} с, {}", c.id, c.kind, c.number, c.duration, c.time);
            }
        }
        _ => bail!("{USAGE}"),
    }
    Ok(())
}
