//! Вызов из Wine (`syn-wine-helper missing …`, патч загрузчика ntdll) и повторный запуск программы.

use std::path::{Path, PathBuf};

use crate::pe::Arch;

#[derive(Debug, Clone, Default)]
pub struct Failed {
    pub machine: u16,
    /// NT-путь exe (`\??\C:\…`).
    pub exe: String,
    /// Текущий каталог Windows (`C:\…\`).
    pub cwd: String,
    /// Командная строка Windows как есть.
    pub cmdline: String,
    pub dlls: Vec<String>,
    /// Не хватило зависимости DLL, которую программа загружает сама (LoadLibrary): она ещё работает.
    pub running: bool,
    /// `run`: запустить именно так (`wine ARGV…`), а не по командной строке Windows.
    pub argv: Vec<String>,
}

impl Failed {
    /// `missing --machine 014c --exe … --cwd … --cmdline … -- a.dll b.dll`
    pub fn parse(args: &mut dyn Iterator<Item = String>) -> Failed {
        let mut f = Failed::default();
        while let Some(a) = args.next() {
            match a.as_str() {
                "--machine" => f.machine = args.next().and_then(|m| u16::from_str_radix(m.trim_start_matches("0x"), 16).ok()).unwrap_or(0),
                "--exe" => f.exe = args.next().unwrap_or_default(),
                "--cwd" => f.cwd = args.next().unwrap_or_default(),
                "--cmdline" => f.cmdline = args.next().unwrap_or_default(),
                "--running" => f.running = true,
                "--" => {
                    while let Some(d) = args.next() {
                        f.dlls.push(d);
                    }
                }
                other => f.dlls.push(other.to_string()),
            }
        }
        f.dlls = f.dlls.iter().map(|d| d.to_ascii_lowercase()).collect();
        f.dlls.dedup();
        f
    }

    pub fn arch(&self) -> Option<Arch> {
        Arch::from_machine(self.machine)
    }

    /// `C:\Games\Fable\Fable.exe`
    pub fn dos_exe(&self) -> String {
        self.exe.strip_prefix("\\??\\").unwrap_or(&self.exe).to_string()
    }

    /// «Fable.exe»
    pub fn title(&self) -> String {
        let e = self.dos_exe();
        e.rsplit(['\\', '/']).next().unwrap_or(&e).to_string()
    }

    /// Те же DLL уже ставились перед этим запуском — значит, установка не помогла.
    pub fn already_tried(&self) -> bool {
        let tried = std::env::var("SYN_WINE_HELPER_TRIED").unwrap_or_default();
        let tried: Vec<String> = tried.split(',').map(|s| s.to_ascii_lowercase()).collect();
        !self.dlls.is_empty() && self.dlls.iter().all(|d| tried.contains(d))
    }

    /// Программу запустил Steam — перезапуск в обход Steam не годится.
    pub fn under_steam() -> bool {
        std::env::var_os("SteamAppId").is_some() || std::env::var_os("SteamGameId").is_some()
    }

    /// Запуск заново с той же командной строкой и рабочим каталогом.
    pub fn relaunch(&self) -> std::io::Result<()> {
        let mut argv = split_cmdline(&self.cmdline);
        if !self.argv.is_empty() {
            argv = self.argv.clone();
        } else if argv.is_empty() {
            argv.push(self.dos_exe());
        } else {
            argv[0] = self.dos_exe();
        }
        let wine = std::env::var("WINELOADER").ok().filter(|w| Path::new(w).is_file()).unwrap_or_else(|| "wine".into());
        let mut c = std::process::Command::new(wine);
        c.args(&argv).env("SYN_WINE_HELPER_TRIED", self.dlls.join(","));
        if let Some(dir) = dos_to_unix(&self.cwd) {
            c.current_dir(dir);
        }
        c.stdin(std::process::Stdio::null());
        c.spawn().map(|_| ())
    }
}

/// Разбор командной строки Windows (правила CommandLineToArgvW).
pub fn split_cmdline(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    // argv[0]: до пробела или в кавычках, обратная косая черта — обычный символ
    let mut cur = String::new();
    while let Some(&c) = chars.peek() {
        if c == ' ' || c == '\t' {
            break;
        }
        chars.next();
        if c == '"' {
            for c in chars.by_ref() {
                if c == '"' {
                    break;
                }
                cur.push(c);
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() || !s.is_empty() {
        out.push(cur);
    }
    loop {
        while matches!(chars.peek(), Some(' ' | '\t')) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut arg = String::new();
        let mut quoted = false;
        while let Some(&c) = chars.peek() {
            match c {
                ' ' | '\t' if !quoted => break,
                '\\' => {
                    let mut n = 0;
                    while chars.peek() == Some(&'\\') {
                        chars.next();
                        n += 1;
                    }
                    if chars.peek() == Some(&'"') {
                        arg.extend(std::iter::repeat_n('\\', n / 2));
                        if n % 2 == 1 {
                            arg.push('"');
                            chars.next();
                        }
                    } else {
                        arg.extend(std::iter::repeat_n('\\', n));
                    }
                }
                '"' => {
                    chars.next();
                    if quoted && chars.peek() == Some(&'"') {
                        arg.push('"');
                        chars.next();
                    } else {
                        quoted = !quoted;
                    }
                }
                _ => {
                    arg.push(c);
                    chars.next();
                }
            }
        }
        out.push(arg);
    }
    out
}

/// `C:\Games\Fable\` → путь unix через dosdevices текущего префикса (только чтобы задать рабочий каталог).
pub fn dos_to_unix(dos: &str) -> Option<PathBuf> {
    let dos = dos.strip_prefix("\\??\\").unwrap_or(dos);
    let (drive, rest) = dos.split_once(':')?;
    if drive.len() != 1 {
        return None;
    }
    let prefix = std::env::var_os("WINEPREFIX").map(PathBuf::from).unwrap_or_else(|| synshell_common::paths::home().join(".wine"));
    let mut p = std::fs::canonicalize(prefix.join("dosdevices").join(format!("{}:", drive.to_ascii_lowercase()))).ok()?;
    for part in rest.split('\\').filter(|s| !s.is_empty()) {
        let exact = p.join(part);
        p = if exact.exists() {
            exact
        } else {
            std::fs::read_dir(&p).ok()?.flatten().map(|e| e.path()).find(|e| e.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case(part)))?
        };
    }
    p.is_dir().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::split_cmdline;

    #[test]
    fn cmdline() {
        assert_eq!(split_cmdline(r#""C:\Games\Fable\Fable.exe" -window "a b" c\"d"#), vec![r"C:\Games\Fable\Fable.exe", "-window", "a b", "c\"d"]);
        assert_eq!(split_cmdline(r"C:\x.exe"), vec![r"C:\x.exe"]);
        assert_eq!(split_cmdline(r#"x.exe "C:\dir\\" next"#), vec!["x.exe", r"C:\dir\", "next"]);
    }
}
