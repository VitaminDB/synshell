//! Запуск программ: окружение сеанса, автозапуск, оболочка (с перезапуском).

use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::state::{Core, State};

/// Переменные окружения для детей композитора.
pub fn session_env(core: &Core) -> Vec<(String, String)> {
    let mut env = vec![
        ("WAYLAND_DISPLAY".to_string(), core.socket_name.clone()),
        ("SYNSHELL_SOCKET".to_string(), core.ipc.path().display().to_string()),
        ("XDG_CURRENT_DESKTOP".to_string(), "synshell".to_string()),
        ("XDG_SESSION_DESKTOP".to_string(), "synshell".to_string()),
        ("XDG_SESSION_TYPE".to_string(), "wayland".to_string()),
        ("MOZ_ENABLE_WAYLAND".to_string(), "1".to_string()),
        ("QT_QPA_PLATFORM".to_string(), "wayland;xcb".to_string()),
        ("QT_WAYLAND_DISABLE_WINDOWDECORATION".to_string(), "1".to_string()),
        ("SDL_VIDEODRIVER".to_string(), "wayland,x11".to_string()),
        ("_JAVA_AWT_WM_NONREPARENTING".to_string(), "1".to_string()),
        ("ELECTRON_OZONE_PLATFORM_HINT".to_string(), "auto".to_string()),
        ("XCURSOR_THEME".to_string(), core.config.appearance.cursor_theme.clone()),
        ("XCURSOR_SIZE".to_string(), core.config.appearance.cursor_size.to_string()),
    ];
    if std::env::var_os("QT_QPA_PLATFORMTHEME").is_none() {
        if let Some(theme) = qt_platform_theme() {
            env.push(("QT_QPA_PLATFORMTHEME".to_string(), theme.to_string()));
        }
    }
    // Глобальное меню на панели: GIMP отдаёт меню наружу (GtkApplication,
    // org.gtk.Menus) только с этой переменной.
    if core.config.panels.iter().any(|p| p.applets.iter().any(|a| a.kind == "appmenu")) {
        env.push(("GIMP_GTK_MENUBAR".to_string(), "1".to_string()));
    }
    if let Some(x) = core.xwayland.as_ref().and_then(|x| x.display) {
        env.push(("DISPLAY".to_string(), format!(":{x}")));
    }
    for (k, v) in &core.config.general.environment {
        env.push((k.clone(), v.clone()));
    }
    env
}

/// Тема платформы Qt: без неё Qt-приложения не читают цвета и стиль из
/// kdeglobals (у KDE-программ тёмный фон и тёмный текст). Плагин KDE
/// подхватывается сам только при XDG_CURRENT_DESKTOP=KDE.
fn qt_platform_theme() -> Option<&'static str> {
    const DIRS: [&str; 4] = ["/usr/lib/qt6/plugins", "/usr/lib/qt/plugins", "/usr/lib64/qt6/plugins", "/usr/lib/x86_64-linux-gnu/qt6/plugins"];
    let has = |file: &str| DIRS.iter().any(|d| std::path::Path::new(d).join("platformthemes").join(file).is_file());
    if has("KDEPlasmaPlatformTheme6.so") {
        Some("kde")
    } else if has("libqt6ct.so") {
        Some("qt6ct")
    } else {
        None
    }
}

fn command(core: &Core, cmdline: &str) -> Command {
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c").arg(cmdline);
    if core.nested {
        // Во вложенном режиме DISPLAY хоста не нужен клиентам.
        cmd.env_remove("DISPLAY");
    }
    for (k, v) in session_env(core) {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::null());
    unsafe {
        // Своя группа процессов: не получать сигналы композитора, и чтобы
        // закрытие терминала не убивало запущенное.
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd
}

/// Запустить команду оболочкой (`sh -c`), не дожидаясь.
pub fn spawn_shell(core: &Core, cmdline: &str) {
    let cmdline = cmdline.trim();
    if cmdline.is_empty() {
        return;
    }
    tracing::info!(cmd = cmdline, "запуск");
    match command(core, cmdline).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        Ok(mut child) => {
            // Дожидаемся в фоне, чтобы не копить зомби.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::warn!(cmd = cmdline, ?e, "не удалось запустить"),
    }
}

pub fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

/// Процесс оболочки: перезапуск при падении (с защитой от петли).
#[derive(Default)]
pub struct ShellProcess {
    child: Option<Child>,
    restarts: Vec<Instant>,
    stopping: bool,
}

impl ShellProcess {
    pub fn stop(&mut self) {
        self.stopping = true;
        if let Some(c) = &mut self.child {
            unsafe {
                libc::kill(c.id() as i32, libc::SIGTERM);
            }
            let deadline = Instant::now() + Duration::from_millis(500);
            while Instant::now() < deadline {
                if let Ok(Some(_)) = c.try_wait() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = c.kill();
            let _ = c.wait();
        }
        self.child = None;
    }
}

impl State {
    /// Перезапустить оболочку по запросу (свежий бинарник после обновления).
    pub fn restart_shell(&mut self) {
        tracing::info!("перезапуск оболочки");
        self.core.shell.stop();
        self.core.shell.stopping = false;
        self.core.shell.restarts.clear();
        self.start_shell();
    }

    /// Запустить (или перезапустить) оболочку из `general.shell`.
    pub fn start_shell(&mut self) {
        let cmdline = self.core.config.general.shell.trim().to_string();
        if cmdline.is_empty() || self.core.shell.stopping {
            return;
        }
        let log = log_dir().join("shell.log");
        let stderr = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .map(Stdio::from)
            .unwrap_or_else(|_| Stdio::null());
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf()));
        let mut cmd = command(&self.core, &cmdline);
        // Рядом собранная оболочка важнее установленной (для разработки).
        if let Some(dir) = exe_dir {
            let path = std::env::var("PATH").unwrap_or_default();
            cmd.env("PATH", format!("{}:{path}", dir.display()));
        }
        match cmd.stdout(Stdio::null()).stderr(stderr).spawn() {
            Ok(child) => {
                tracing::info!(pid = child.id(), cmd = cmdline, "оболочка запущена");
                self.core.shell.child = Some(child);
            }
            Err(e) => tracing::error!(?e, cmd = cmdline, "оболочка не запустилась"),
        }
    }

    /// Проверить, жива ли оболочка; упала — перезапустить (не чаще 5 раз за минуту).
    pub fn watch_shell(&mut self) {
        let exited = match &mut self.core.shell.child {
            Some(c) => matches!(c.try_wait(), Ok(Some(_))),
            None => false,
        };
        if !exited || self.core.shell.stopping {
            return;
        }
        let status = self.core.shell.child.take().and_then(|mut c| c.wait().ok());
        tracing::warn!(?status, "оболочка завершилась");
        let now = Instant::now();
        self.core.shell.restarts.retain(|t| now.duration_since(*t) < Duration::from_secs(60));
        if self.core.shell.restarts.len() >= 5 {
            tracing::error!("оболочка падает слишком часто — не перезапускаю");
            return;
        }
        self.core.shell.restarts.push(now);
        self.start_shell();
    }

    /// Автозапуск: команды из конфига и `~/.config/autostart/*.desktop`.
    pub fn run_autostart(&mut self) {
        let cmds = self.core.config.general.autostart.clone();
        for c in cmds {
            spawn_shell(&self.core, &c);
        }
        if self.core.config.general.xdg_autostart {
            for cmd in xdg_autostart_commands() {
                spawn_shell(&self.core, &cmd);
            }
        }
    }
}

pub fn log_dir() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state"));
    let dir = base.join("synwm");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Команды XDG Autostart: пользовательские перекрывают системные по имени
/// файла; учитываются Hidden, OnlyShowIn/NotShowIn, TryExec.
fn xdg_autostart_commands() -> Vec<String> {
    let mut dirs = vec![synshell_common::paths::config_dir().parent().map(|p| p.join("autostart")).unwrap_or_default()];
    let sys = std::env::var("XDG_CONFIG_DIRS").unwrap_or_else(|_| "/etc/xdg".into());
    dirs.extend(sys.split(':').map(|d| std::path::PathBuf::from(d).join("autostart")));
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let fname = path.file_name().unwrap().to_string_lossy().to_string();
            if !seen.insert(fname) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let mut in_entry = false;
            let mut exec = None;
            let mut hidden = false;
            let mut only: Option<String> = None;
            let mut not: Option<String> = None;
            let mut try_exec = None;
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with('[') {
                    in_entry = line == "[Desktop Entry]";
                    continue;
                }
                if !in_entry {
                    continue;
                }
                let Some((k, v)) = line.split_once('=') else { continue };
                match k.trim() {
                    "Exec" => exec = Some(v.trim().to_string()),
                    "Hidden" => hidden = v.trim() == "true",
                    "OnlyShowIn" => only = Some(v.to_string()),
                    "NotShowIn" => not = Some(v.to_string()),
                    "TryExec" => try_exec = Some(v.trim().to_string()),
                    _ => {}
                }
            }
            if hidden {
                continue;
            }
            let desk = |s: &Option<String>| s.as_ref().is_some_and(|l| l.split(';').any(|d| d == "synshell"));
            if only.is_some() && !desk(&only) {
                continue;
            }
            if desk(&not) {
                continue;
            }
            if let Some(t) = try_exec {
                if !(std::path::Path::new(&t).is_file() || which(&t)) {
                    continue;
                }
            }
            if let Some(e) = exec {
                out.push(strip_field_codes(&e));
            }
        }
    }
    out
}

/// Убрать поля %f %u %i… из Exec.
pub fn strip_field_codes(exec: &str) -> String {
    let mut out = String::new();
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('%') => out.push('%'),
                Some(_) | None => {}
            }
        } else {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    #[test]
    fn field_codes() {
        assert_eq!(super::strip_field_codes("firefox %u"), "firefox");
        assert_eq!(super::strip_field_codes("app --x=100%% %F"), "app --x=100%");
    }
}
