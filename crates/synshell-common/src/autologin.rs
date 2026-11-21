use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FILE_ENV: &str = "SYNSHELL_AUTOLOGIN_FILE";
pub const SESSION_DIRS_ENV: &str = "SYNSHELL_SESSION_DIRS";
pub const HELPER: &str = "/usr/lib/synlogin/autologin-helper";
pub const SYNSHELL: &str = "synshell";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Autologin {
    pub enabled: bool,
    pub user: String,
    pub session: String,
}

impl Default for Autologin {
    fn default() -> Self {
        Self { enabled: true, user: String::new(), session: SYNSHELL.into() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Synshell,
    Wayland,
    X11,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub kind: SessionKind,
    pub exec: String,
    pub desktop_names: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoginUser {
    pub name: String,
    pub full_name: String,
    pub uid: u32,
}

impl LoginUser {
    pub fn display(&self) -> String {
        if self.full_name.is_empty() || self.full_name == self.name {
            self.name.clone()
        } else {
            format!("{} ({})", self.full_name, self.name)
        }
    }
}

pub fn file() -> PathBuf {
    std::env::var_os(FILE_ENV).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/etc/synshell/autologin.toml"))
}

pub fn load() -> Option<Autologin> {
    let text = std::fs::read_to_string(file()).ok()?;
    Some(toml::from_str(&text).unwrap_or_else(|_| Autologin { enabled: false, ..Autologin::default() }))
}

pub fn to_toml(a: &Autologin) -> String {
    toml::to_string(a).unwrap_or_default()
}

pub fn is_login_user(uid: u32, shell: &str) -> bool {
    (1000..60000).contains(&uid) && !shell.ends_with("nologin") && !shell.ends_with("false")
}

pub fn parse_login_users(passwd: &str) -> Vec<LoginUser> {
    let mut v: Vec<LoginUser> = passwd
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            let uid: u32 = f[2].parse().ok()?;
            is_login_user(uid, f[6]).then(|| LoginUser { name: f[0].into(), full_name: f[4].split(',').next().unwrap_or("").trim().into(), uid })
        })
        .collect();
    v.sort_by(|a, b| a.uid.cmp(&b.uid).then_with(|| a.name.cmp(&b.name)));
    v
}

pub fn login_users() -> Vec<LoginUser> {
    let path = std::env::var("SYNLOGIN_PASSWD").unwrap_or_else(|_| "/etc/passwd".into());
    parse_login_users(&std::fs::read_to_string(path).unwrap_or_default())
}

pub fn valid_session_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || "._+@-/".contains(c)) && !id.contains("..")
}

fn session_dirs() -> Vec<(PathBuf, SessionKind)> {
    let roots: Vec<PathBuf> = match std::env::var(SESSION_DIRS_ENV) {
        Ok(v) if !v.is_empty() => v.split(':').map(PathBuf::from).collect(),
        _ => vec!["/usr/local/share".into(), "/usr/share".into()],
    };
    let mut v = Vec::new();
    for r in &roots {
        v.push((r.join("wayland-sessions"), SessionKind::Wayland));
    }
    for r in &roots {
        v.push((r.join("xsessions"), SessionKind::X11));
    }
    v
}

fn in_path(prog: &str) -> bool {
    if prog.contains('/') {
        return Path::new(prog).is_file();
    }
    std::env::var("PATH")
        .unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into())
        .split(':')
        .any(|d| Path::new(d).join(prog).is_file())
}

fn strip_field_codes(exec: &str) -> String {
    exec.split_whitespace().filter(|w| !(w.len() == 2 && w.starts_with('%'))).collect::<Vec<_>>().join(" ")
}

fn localized(entries: &[(String, String)], key: &str, lang: &str) -> Option<String> {
    let base = lang.split('.').next().unwrap_or("");
    let short = base.split('_').next().unwrap_or("");
    for l in [base, short] {
        if l.is_empty() {
            continue;
        }
        let k = format!("{key}[{l}]");
        if let Some((_, v)) = entries.iter().find(|(kk, _)| *kk == k) {
            return Some(v.clone());
        }
    }
    entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

pub fn parse_session(stem: &str, text: &str, kind: SessionKind, lang: &str) -> Option<Session> {
    let mut entries = Vec::new();
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if in_entry {
            if let Some((k, v)) = line.split_once('=') {
                entries.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
    }
    let get = |k: &str| entries.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone());
    if get("Hidden").as_deref() == Some("true") || get("NoDisplay").as_deref() == Some("true") {
        return None;
    }
    let exec = strip_field_codes(&get("Exec")?);
    if exec.is_empty() {
        return None;
    }
    let desktop_names = get("DesktopNames").unwrap_or_default().trim_end_matches(';').replace(';', ":");
    let name = localized(&entries, "Name", lang).unwrap_or_else(|| stem.to_string());
    let prefix = if kind == SessionKind::X11 { "x11" } else { "wayland" };
    Some(Session { id: format!("{prefix}/{stem}"), name, kind, exec, desktop_names })
}

fn is_synshell(s: &Session) -> bool {
    s.desktop_names.split(':').any(|d| d.eq_ignore_ascii_case(SYNSHELL)) || s.exec.contains("synshell-session")
}

pub fn builtin() -> Session {
    Session { id: SYNSHELL.into(), name: "synshell".into(), kind: SessionKind::Synshell, exec: String::new(), desktop_names: SYNSHELL.into() }
}

pub fn sessions() -> Vec<Session> {
    let lang = synshell_tr::language();
    let x11 = in_path("xinit");
    let mut v = vec![builtin()];
    for (dir, kind) in session_dirs() {
        if kind == SessionKind::X11 && !x11 {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "desktop")).collect();
        files.sort();
        for f in files {
            let Some(stem) = f.file_stem().and_then(|s| s.to_str()) else { continue };
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            let Some(s) = parse_session(stem, &text, kind, &lang) else { continue };
            if is_synshell(&s) || !valid_session_id(&s.id) || v.iter().any(|o| o.id == s.id) {
                continue;
            }
            let try_exec = text.lines().find_map(|l| l.trim().strip_prefix("TryExec=").map(|x| x.trim().to_string()));
            if try_exec.is_some_and(|t| !t.is_empty() && !in_path(&t)) {
                continue;
            }
            v.push(s);
        }
    }
    v
}

pub fn effective(saved: Option<&Autologin>, users: &[LoginUser], preferred: Option<&str>) -> Autologin {
    let mut a = saved.cloned().unwrap_or_default();
    if a.user.is_empty() {
        a.user = preferred
            .filter(|p| users.iter().any(|u| u.name == *p))
            .map(String::from)
            .or_else(|| users.first().map(|u| u.name.clone()))
            .unwrap_or_default();
    }
    if a.session.is_empty() {
        a.session = SYNSHELL.into();
    }
    a
}

pub fn decide(saved: Option<&Autologin>, users: &[LoginUser], sessions: &[Session]) -> Option<(String, Session)> {
    let a = effective(saved, users, None);
    if !a.enabled || !users.iter().any(|u| u.name == a.user) {
        return None;
    }
    let session = sessions.iter().find(|s| s.id == a.session).cloned().unwrap_or_else(builtin);
    Some((a.user, session))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "root:x:0:0::/root:/bin/bash\n\
        nobody:x:65534:65534:Nobody:/:/usr/bin/nologin\n\
        anna:x:1001:1001:Anna:/home/anna:/bin/bash\n\
        master:x:1000:1000:Vitaly,,,:/home/master:/usr/bin/zsh\n\
        svc:x:1002:1002::/var/svc:/usr/bin/nologin\n";

    fn users() -> Vec<LoginUser> {
        parse_login_users(PASSWD)
    }

    fn sess() -> Vec<Session> {
        vec![
            builtin(),
            Session { id: "wayland/plasma".into(), name: "Plasma".into(), kind: SessionKind::Wayland, exec: "startplasma-wayland".into(), desktop_names: "KDE".into() },
        ]
    }

    #[test]
    fn login_users_sorted_by_uid() {
        let u = users();
        assert_eq!(u.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(), ["master", "anna"]);
        assert_eq!(u[0].display(), "Vitaly (master)");
    }

    #[test]
    fn no_file_logs_in_first_user_into_synshell() {
        let (user, s) = decide(None, &users(), &sess()).unwrap();
        assert_eq!(user, "master");
        assert_eq!(s.kind, SessionKind::Synshell);
    }

    #[test]
    fn disabled_means_greeter() {
        let a = Autologin { enabled: false, user: "master".into(), session: SYNSHELL.into() };
        assert!(decide(Some(&a), &users(), &sess()).is_none());
    }

    #[test]
    fn chosen_user_and_session() {
        let a = Autologin { enabled: true, user: "anna".into(), session: "wayland/plasma".into() };
        let (user, s) = decide(Some(&a), &users(), &sess()).unwrap();
        assert_eq!(user, "anna");
        assert_eq!(s.exec, "startplasma-wayland");
    }

    #[test]
    fn missing_user_does_not_fall_back_to_someone_else() {
        let a = Autologin { enabled: true, user: "ghost".into(), session: SYNSHELL.into() };
        assert!(decide(Some(&a), &users(), &sess()).is_none());
        let a = Autologin { enabled: true, user: "svc".into(), session: SYNSHELL.into() };
        assert!(decide(Some(&a), &users(), &sess()).is_none());
    }

    #[test]
    fn removed_session_falls_back_to_synshell() {
        let a = Autologin { enabled: true, user: "master".into(), session: "wayland/gone".into() };
        assert_eq!(decide(Some(&a), &users(), &sess()).unwrap().1.kind, SessionKind::Synshell);
    }

    #[test]
    fn no_users_no_autologin() {
        assert!(decide(None, &[], &sess()).is_none());
    }

    #[test]
    fn effective_prefers_current_user_without_file() {
        assert_eq!(effective(None, &users(), Some("anna")).user, "anna");
        assert_eq!(effective(None, &users(), Some("root")).user, "master");
    }

    #[test]
    fn toml_roundtrip_and_broken_file_disables() {
        let a = Autologin { enabled: false, user: "anna".into(), session: "x11/i3".into() };
        assert_eq!(toml::from_str::<Autologin>(&to_toml(&a)).unwrap(), a);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("autologin.toml");
        std::fs::write(&f, "enabled = maybe").unwrap();
        std::env::set_var(FILE_ENV, &f);
        assert!(!load().unwrap().enabled);
        std::fs::write(&f, "user = \"anna\"\n").unwrap();
        let l = load().unwrap();
        assert!(l.enabled);
        assert_eq!(l.session, SYNSHELL);
        std::env::remove_var(FILE_ENV);
    }

    #[test]
    fn desktop_files() {
        let text = "[Desktop Entry]\nName=Plasma (Wayland)\nName[ru]=Plasma (Вэйланд)\nExec=/usr/lib/plasma-dbus-run-session-if-needed startplasma-wayland %U\nDesktopNames=KDE;\n[Desktop Action x]\nName=nope\n";
        let s = parse_session("plasma", text, SessionKind::Wayland, "ru_RU.UTF-8").unwrap();
        assert_eq!(s.id, "wayland/plasma");
        assert_eq!(s.name, "Plasma (Вэйланд)");
        assert_eq!(s.exec, "/usr/lib/plasma-dbus-run-session-if-needed startplasma-wayland");
        assert_eq!(s.desktop_names, "KDE");
        assert_eq!(parse_session("plasma", text, SessionKind::Wayland, "de").unwrap().name, "Plasma (Wayland)");
        let x = parse_session("i3", "[Desktop Entry]\nName=i3\nExec=i3\n", SessionKind::X11, "").unwrap();
        assert_eq!(x.id, "x11/i3");
        assert!(parse_session("h", "[Desktop Entry]\nName=h\nExec=h\nHidden=true\n", SessionKind::X11, "").is_none());
        let syn = parse_session("synshell", "[Desktop Entry]\nName=synshell\nExec=synshell-session\nDesktopNames=synshell\n", SessionKind::Wayland, "").unwrap();
        assert!(is_synshell(&syn));
    }

    #[test]
    fn session_ids() {
        assert!(valid_session_id("synshell"));
        assert!(valid_session_id("wayland/plasma"));
        assert!(!valid_session_id("wayland/../../etc"));
        assert!(!valid_session_id("a b"));
        assert!(!valid_session_id("a\"b"));
    }

    #[test]
    fn sessions_from_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let w = dir.path().join("wayland-sessions");
        std::fs::create_dir_all(&w).unwrap();
        std::fs::write(w.join("synshell.desktop"), "[Desktop Entry]\nName=synshell\nExec=synshell-session\nDesktopNames=synshell\n").unwrap();
        std::fs::write(w.join("sway.desktop"), "[Desktop Entry]\nName=Sway\nExec=sway\n").unwrap();
        std::fs::write(w.join("gone.desktop"), "[Desktop Entry]\nName=Gone\nExec=gone\nTryExec=/nonexistent/gone\n").unwrap();
        std::env::set_var(SESSION_DIRS_ENV, dir.path());
        let ids: Vec<String> = sessions().into_iter().map(|s| s.id).collect();
        std::env::remove_var(SESSION_DIRS_ENV);
        assert_eq!(ids, ["synshell", "wayland/sway"]);
    }
}
