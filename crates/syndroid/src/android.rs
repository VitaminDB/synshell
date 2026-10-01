//! Действия внутри работающего Android через его же утилиты (`setprop`, `am`, `cmd`, `settings`) — от root
//! в контейнере. Окна создаёт hwcomposer образа (Wayland-клиент): он показывает то, что перечислено в свойстве
//! `waydroid.active_apps` — `Waydroid` (весь Android одним окном) или пакет приложения.

use std::process::Stdio;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::container;

/// Выполнить команду в контейнере, вернуть stdout.
pub fn run(init_pid: i32, argv: &[&str]) -> Result<String> {
    let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let out = container::command_in(init_pid, &argv)?.stdin(Stdio::null()).output()?;
    if !out.status.success() {
        bail!(
            "{}: {}",
            argv.join(" "),
            String::from_utf8_lossy(if out.stderr.is_empty() { &out.stdout } else { &out.stderr }).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn setprop(pid: i32, k: &str, v: &str) -> Result<()> {
    run(pid, &["/system/bin/setprop", k, v]).map(drop)
}

/// `settings put global …`
fn settings_global(pid: i32, k: &str, v: &str) -> Result<()> {
    run(pid, &["/system/bin/settings", "put", "global", k, v]).map(drop)
}

/// hwcomposer пересоздаёт окна только на новом кадре — дёрнуть шторку (так делает и Waydroid).
fn refresh(pid: i32) {
    let _ = run(pid, &["/system/bin/cmd", "statusbar", "expand-notifications"]);
    std::thread::sleep(std::time::Duration::from_millis(400));
    let _ = run(pid, &["/system/bin/cmd", "statusbar", "collapse"]);
}

/// Режим окон: каждое приложение своим окном (freeform) или весь Android одним.
pub fn apply_window_mode(pid: i32, multi: bool) -> Result<()> {
    setprop(pid, "persist.waydroid.multi_windows", if multi { "true" } else { "false" })
}

/// Весь Android одним окном.
pub fn show_full_ui(pid: i32) -> Result<()> {
    setprop(pid, "waydroid.active_apps", "Waydroid")?;
    settings_global(pid, "policy_control", "null*")?;
    refresh(pid);
    Ok(())
}

/// Запускаемое приложение (активность с категорией LAUNCHER).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct App {
    pub package: String,
    pub activity: String,
}

/// Приложения с ярлыком в лаунчере.
pub fn launchable(pid: i32) -> Result<Vec<App>> {
    let out = run(
        pid,
        &[
            "/system/bin/cmd",
            "package",
            "query-activities",
            "--brief",
            "-a",
            "android.intent.action.MAIN",
            "-c",
            "android.intent.category.LAUNCHER",
        ],
    )?;
    let mut v: Vec<App> = out
        .lines()
        .filter_map(|l| l.trim().split_once('/'))
        .map(|(p, a)| App { package: p.to_string(), activity: a.to_string() })
        .collect();
    v.sort_by(|a, b| a.package.cmp(&b.package));
    v.dedup_by(|a, b| a.package == b.package);
    Ok(v)
}

/// Открыть приложение (окно — через `waydroid.active_apps`).
pub fn launch(pid: i32, package: &str, multi: bool) -> Result<()> {
    if package.is_empty() || package.contains(['/', ' ']) {
        bail!("неверное имя пакета «{package}»");
    }
    let act = run(
        pid,
        &[
            "/system/bin/cmd",
            "package",
            "resolve-activity",
            "--brief",
            "-c",
            "android.intent.category.LAUNCHER",
            package,
        ],
    )?;
    let component = act.lines().map(str::trim).rfind(|l| l.contains('/')).map(str::to_string);
    let Some(component) = component else { bail!("у {package} нет активности для запуска") };
    setprop(pid, "waydroid.active_apps", package)?;
    run(pid, &["/system/bin/am", "start", "-n", &component])?;
    // Как у Waydroid: в одном окне — без строки состояния Android, в мультиоконном — полноэкранно
    settings_global(pid, "policy_control", if multi { "immersive.full=*" } else { "immersive.status=*" })?;
    Ok(())
}

pub fn force_stop(pid: i32, package: &str) -> Result<()> {
    run(pid, &["/system/bin/am", "force-stop", package]).map(drop)
}
