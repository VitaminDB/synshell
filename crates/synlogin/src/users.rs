//! Пользователи системы: список для экрана входа, создание нового.

use std::io::Write;
use std::process::{Command, Stdio};
use syngui::t;

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub name: String,
    pub full_name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: String,
    pub shell: String,
}

impl User {
    pub fn display(&self) -> String {
        if self.full_name.is_empty() { self.name.clone() } else { self.full_name.clone() }
    }
}

pub fn all() -> Vec<User> {
    let text = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            Some(User {
                name: f[0].into(),
                uid: f[2].parse().ok()?,
                gid: f[3].parse().ok()?,
                full_name: f[4].split(',').next().unwrap_or("").trim().into(),
                home: f[5].into(),
                shell: f[6].into(),
            })
        })
        .collect()
}

pub fn find(name: &str) -> Option<User> {
    all().into_iter().find(|u| u.name == name)
}

/// Кого показывать на экране входа: обычные пользователи (UID 1000–59999 с
/// оболочкой входа) и root последним.
pub fn login_users() -> Vec<User> {
    let mut v: Vec<User> = all()
        .into_iter()
        .filter(|u| (1000..60000).contains(&u.uid) && !u.shell.ends_with("nologin") && !u.shell.ends_with("false"))
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    if let Some(root) = find("root") {
        v.push(root);
    }
    v
}

/// Логин: строчные латинские буквы, цифры, `_` и `-`, с буквы, до 32.
pub fn valid_login(name: &str) -> bool {
    let mut ch = name.chars();
    matches!(ch.next(), Some('a'..='z' | '_'))
        && name.len() <= 32
        && ch.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Создать пользователя (нужен root): домашний каталог, группы для экрана,
/// ввода, звука и сети (`network` — управление NetworkManager через polkit),
/// администратор — ещё `wheel` и `systemd-journal` (журнал системы — без
/// ACL на телефоне его иначе не прочитать: `synlink logs`); пароль (пустой — без пароля).
pub fn create(login: &str, full_name: &str, password: &str, admin: bool) -> Result<(), String> {
    if !valid_login(login) {
        return Err(t!("Логин: латинские строчные буквы, цифры, _ и -, начинается с буквы").into());
    }
    if find(login).is_some() {
        return Err(t!("Такой пользователь уже есть").into());
    }
    // Группы, которых нет в системе, useradd не примет — берём только существующие.
    let groups_file = std::fs::read_to_string("/etc/group").unwrap_or_default();
    let exists = |g: &str| groups_file.lines().any(|l| l.split(':').next() == Some(g));
    let mut groups: Vec<&str> = ["video", "input", "audio", "render", "network"].into_iter().filter(|g| exists(g)).collect();
    if admin {
        groups.extend(["wheel", "systemd-journal"].into_iter().filter(|g| exists(g)));
    }
    let shell = if std::path::Path::new("/bin/bash").exists() { "/bin/bash" } else { "/bin/sh" };
    let mut cmd = Command::new("useradd");
    cmd.args(["-m", "-s", shell]);
    if !full_name.trim().is_empty() {
        cmd.args(["-c", full_name.trim()]);
    }
    if !groups.is_empty() {
        cmd.args(["-G", &groups.join(",")]);
    }
    cmd.arg(login);
    let out = cmd.output().map_err(|e| format!("useradd: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    set_password(login, password)
}

pub fn set_password(login: &str, password: &str) -> Result<(), String> {
    if password.is_empty() {
        let st = Command::new("passwd").args(["-d", login]).status().map_err(|e| e.to_string())?;
        return if st.success() { Ok(()) } else { Err(t!("passwd -d не удался").into()) };
    }
    let mut child = Command::new("chpasswd").stdin(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("chpasswd: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{login}:{password}");
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Пароль пустой (вход без пароля).
pub fn has_empty_password(login: &str) -> bool {
    std::fs::read_to_string("/etc/shadow")
        .ok()
        .and_then(|s| s.lines().find(|l| l.split(':').next() == Some(login)).map(|l| l.split(':').nth(1).unwrap_or("x").is_empty()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    #[test]
    fn logins() {
        assert!(super::valid_login("vitaly"));
        assert!(super::valid_login("dev_1"));
        assert!(!super::valid_login("1abc"));
        assert!(!super::valid_login("Vitaly"));
        assert!(!super::valid_login("ви"));
    }
}
