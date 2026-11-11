//! Один процесс «Телефона»: второй запуск (`synphone НОМЕР`, `--log`, ярлык, оболочка на входящий) передаёт
//! команду живущему процессу через `$XDG_RUNTIME_DIR/synphone.sock` и выходит.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

fn socket() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join("synphone.sock")
}

/// Команда для процесса.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Show,
    Log,
    Dial(String),
    Contacts,
}

impl Cmd {
    fn encode(&self) -> String {
        match self {
            Cmd::Show => "show".into(),
            Cmd::Log => "log".into(),
            Cmd::Contacts => "contacts".into(),
            Cmd::Dial(n) => format!("dial {}", n.replace('\n', " ")),
        }
    }

    fn decode(s: &str) -> Option<Cmd> {
        let s = s.trim();
        Some(match s {
            "show" => Cmd::Show,
            "log" => Cmd::Log,
            "contacts" => Cmd::Contacts,
            _ => Cmd::Dial(s.strip_prefix("dial ")?.to_string()),
        })
    }
}

/// Процесс уже есть — передать команду (`true` — передана, выходить).
pub fn forward(cmd: &Cmd) -> bool {
    let Ok(mut s) = UnixStream::connect(socket()) else { return false };
    s.write_all(format!("{}\n", cmd.encode()).as_bytes()).is_ok()
}

/// Слушать команды (из своего потока вызывается `f`).
pub fn listen(f: impl Fn(Cmd) + Send + 'static) {
    let p = socket();
    let _ = std::fs::remove_file(&p);
    let Ok(l) = UnixListener::bind(&p) else {
        tracing::warn!("synphone: сокет {} не создан", p.display());
        return;
    };
    std::thread::Builder::new()
        .name("synphone-ipc".into())
        .spawn(move || {
            for c in l.incoming().flatten() {
                let mut line = String::new();
                if BufReader::new(c).read_line(&mut line).is_ok() {
                    if let Some(cmd) = Cmd::decode(&line) {
                        f(cmd);
                    }
                }
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        for c in [Cmd::Show, Cmd::Log, Cmd::Contacts, Cmd::Dial("+7 701 111".into())] {
            assert_eq!(Cmd::decode(&c.encode()), Some(c));
        }
    }
}
