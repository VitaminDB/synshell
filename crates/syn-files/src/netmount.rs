//! Сетевые расположения через gvfs (`gio mount`): sftp, smb, ftp, WebDAV,
//! NFS. gio спрашивает имя, пароль и вопросы (неизвестный ключ сервера) в
//! своём выводе — отвечаем из окна подключения; чего не знаем — прерываем
//! и возвращаем окну, что спросить. Подключённое видно как папка FUSE в
//! `/run/user/UID/gvfs`.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use syngui::t;

const SCHEMES: [&str; 10] = ["sftp", "ssh", "smb", "ftp", "ftps", "dav", "davs", "webdav", "nfs", "afp"];

/// Адрес сетевого ресурса (`sftp://…`, `smb://…`), который монтирует gio.
pub fn is_remote(s: &str) -> bool {
    s.trim().split_once("://").is_some_and(|(sch, rest)| SCHEMES.contains(&sch.to_ascii_lowercase().as_str()) && !rest.is_empty())
}

/// `ssh://` → `sftp://`, `webdav://` → `dav://`, без пробелов по краям.
pub fn normalize(uri: &str) -> String {
    let u = uri.trim();
    let (sch, rest) = u.split_once("://").unwrap_or(("", u));
    let sch = match sch.to_ascii_lowercase().as_str() {
        "ssh" => "sftp".to_string(),
        "webdav" => "dav".to_string(),
        s => s.to_string(),
    };
    format!("{sch}://{rest}")
}

/// Ответы на вопросы gio.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Answers {
    pub user: String,
    pub domain: String,
    pub password: String,
    /// Номер выбранного ответа на вопрос (с 1, как его печатает gio).
    pub choice: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Mounted(PathBuf),
    /// Нужны имя и пароль; `wrong` — данные были, но не подошли.
    NeedAuth { message: String, wrong: bool, domain: bool },
    /// Вопрос с вариантами (неизвестный ключ сервера и т. п.).
    Question { message: String, choices: Vec<String> },
    Failed(String),
}

fn gio() -> Command {
    let mut c = Command::new("gio");
    // Подсказки gio («User:», «Password:») — по-английски, чтобы их узнать.
    c.env("LC_ALL", "C.UTF-8").env("LANGUAGE", "C");
    c
}

/// Смонтировать и вернуть папку FUSE.
pub fn mount(uri: &str, a: &Answers) -> Outcome {
    let uri = normalize(uri);
    let mut child = match gio()
        .arg("mount")
        .arg(&uri)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return Outcome::Failed(t!("Не удалось запустить gio: {e}", e = e)),
    };
    let mut stdin = child.stdin.take();
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    // Ошибки gio — в stderr, отдельным потоком (иначе он может забиться).
    let err_text = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    // Вывод — по кускам: подсказки печатаются без перевода строки.
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = out.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let started = Instant::now();
    let mut text = String::new();
    let mut sent_password = false;
    let mut domain_asked = false;
    let answer = |stdin: &mut Option<std::process::ChildStdin>, s: &str| {
        if let Some(w) = stdin.as_mut() {
            let _ = w.write_all(format!("{s}\n").as_bytes());
            let _ = w.flush();
        }
    };
    let stop = |child: &mut std::process::Child, o: Outcome| {
        let _ = child.kill();
        let _ = child.wait();
        o
    };
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(chunk) => text.push_str(&String::from_utf8_lossy(&chunk)),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(_) => {}
        }
        if started.elapsed() > Duration::from_secs(90) {
            return stop(&mut child, Outcome::Failed(t!("Сервер не ответил").into()));
        }
        let tail = text.trim_end_matches(' ');
        // Сообщение перед подсказкой — то, что gio вывел после прошлого ответа.
        let message = || {
            let body = text.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>();
            body.iter().take(body.len().saturating_sub(1)).map(|l| l.trim()).collect::<Vec<_>>().join("\n")
        };
        if tail.ends_with("User:") {
            if a.user.is_empty() {
                let m = message();
                return stop(&mut child, Outcome::NeedAuth { message: m, wrong: false, domain: domain_asked });
            }
            answer(&mut stdin, &a.user);
            text.clear();
        } else if tail.ends_with("Domain:") {
            domain_asked = true;
            answer(&mut stdin, &a.domain);
            text.clear();
        } else if tail.ends_with("Password:") {
            if a.password.is_empty() || sent_password {
                let m = message();
                return stop(&mut child, Outcome::NeedAuth { message: m, wrong: sent_password, domain: domain_asked || uri.starts_with("smb") });
            }
            sent_password = true;
            answer(&mut stdin, &a.password);
            text.clear();
        } else if tail.ends_with("Choice:") {
            let mut choices = Vec::new();
            let mut msg = Vec::new();
            for l in text.lines() {
                let l = l.trim();
                match l.strip_prefix('[').and_then(|r| r.split_once(']')) {
                    Some((n, c)) if n.parse::<usize>().is_ok() => choices.push(c.trim().to_string()),
                    _ if !l.is_empty() && !l.ends_with("Choice:") => msg.push(l.to_string()),
                    _ => {}
                }
            }
            match a.choice {
                Some(n) if n >= 1 && n <= choices.len() => {
                    answer(&mut stdin, &n.to_string());
                    text.clear();
                }
                _ => return stop(&mut child, Outcome::Question { message: msg.join("\n"), choices }),
            }
        }
    }
    let status = child.wait();
    let err = err_text.join().unwrap_or_default();
    // Уже смонтировано (или смонтировалось, несмотря на ошибку) — папка есть.
    if let Some(p) = local_path(&uri) {
        return Outcome::Mounted(p);
    }
    if status.is_ok_and(|s| s.success()) {
        return Outcome::Failed(t!("Подключено, но папка gvfs не найдена (нужен gvfs-fuse)").into());
    }
    let line = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    // «gio: sftp://host/: Доступ запрещён» → «Доступ запрещён».
    let line = line.strip_prefix("gio: ").map(|l| l.strip_prefix(&format!("{uri}: ")).unwrap_or(l)).unwrap_or(line);
    Outcome::Failed(if line.is_empty() { t!("Не удалось подключиться").into() } else { line.to_string() })
}

/// Папка FUSE смонтированного адреса.
pub fn local_path(uri: &str) -> Option<PathBuf> {
    let out = gio().arg("info").arg("-a").arg("standard::name").arg(uri).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.trim().strip_prefix("local path:").map(|p| PathBuf::from(p.trim())))
        .filter(|p| p.exists())
}

/// Отключить ресурс по его папке gvfs.
pub fn unmount(path: &std::path::Path) -> Result<(), String> {
    let out = gio().arg("mount").arg("-u").arg(path).stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("").trim().trim_start_matches("gio: ").to_string())
    }
}

/// Папка ресурса gvfs (а не подпапка внутри него).
pub fn is_gvfs_root(p: &std::path::Path) -> bool {
    p.parent() == Some(&PathBuf::from(format!("/run/user/{}/gvfs", unsafe { libc::getuid() })))
}

fn recent_file() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| synshell_common::paths::home().join(".local/state"))
        .join("synshell/files-servers")
}

/// Недавние адреса (без паролей), свежие первыми.
pub fn recent() -> Vec<String> {
    std::fs::read_to_string(recent_file()).unwrap_or_default().lines().map(str::trim).filter(|l| is_remote(l)).map(String::from).collect()
}

pub fn remember(uri: &str) {
    let uri = normalize(uri);
    let mut v = recent();
    v.retain(|u| *u != uri);
    v.insert(0, uri);
    v.truncate(10);
    let f = recent_file();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, v.join("\n") + "\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris() {
        assert!(is_remote("sftp://me@host/home"));
        assert!(is_remote(" SMB://nas/share "));
        assert!(!is_remote("file:///tmp"));
        assert!(!is_remote("sftp://"));
        assert!(!is_remote("/home/me"));
        assert_eq!(normalize("ssh://h"), "sftp://h");
        assert_eq!(normalize("WebDAV://h/x"), "dav://h/x");
    }

    /// Против живого сервера: `SYNFILES_TEST_SFTP=sftp://127.0.0.1:2222/`
    /// (вход по паролю разрешён, пароль неверный).
    #[test]
    #[ignore]
    fn asks_for_auth_then_reports_wrong_password() {
        let Ok(uri) = std::env::var("SYNFILES_TEST_SFTP") else { return };
        let o = mount(&uri, &Answers::default());
        assert!(matches!(o, Outcome::NeedAuth { wrong: false, .. }), "{o:?}");
        let o = mount(&uri, &Answers { user: "nobody".into(), password: "wrong".into(), ..Answers::default() });
        assert!(matches!(o, Outcome::NeedAuth { wrong: true, .. }), "{o:?}");
    }

    /// Тестовый бэкенд gvfs (`gvfsd-localtest`): подключить, найти папку, отключить.
    #[test]
    #[ignore]
    fn mounts_and_unmounts_localtest() {
        if !std::path::Path::new("/usr/lib/gvfsd-localtest").exists() {
            return;
        }
        let Outcome::Mounted(p) = mount("localtest:///", &Answers::default()) else { panic!("не подключилось") };
        assert!(is_gvfs_root(&p), "{p:?}");
        unmount(&p).unwrap();
        assert!(local_path("localtest:///").is_none());
    }
}
