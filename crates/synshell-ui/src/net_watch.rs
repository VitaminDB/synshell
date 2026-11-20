//! Программа впервые вышла в сеть: synnetd запоминает программы, которые уже
//! соединялись, и отдаёт новые в `traffic().new_apps`. Здесь — уведомление
//! «Разрешить» / «Запретить сеть» / «Соединения» (окно журнала в Мониторе
//! ресурсов). Свои программы — всем; системные службы — только тем, кто может
//! менять настройки сети (root, `wheel`).

use std::collections::HashSet;
use std::time::Duration;

use syngui::t;

use crate::ctx::ShellCtx;
use crate::notifications;

const POLL: Duration = Duration::from_secs(3);

pub fn start(_ctx: ShellCtx) {
    let _ = std::thread::Builder::new().name("shell-net-watch".into()).spawn(|| {
        let started = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let me = unsafe { libc::getuid() };
        let mut shown: HashSet<(String, u64)> = HashSet::new();
        loop {
            std::thread::sleep(POLL);
            if !std::path::Path::new(syn_netd::SOCKET).exists() {
                continue;
            }
            let Ok(t) = syn_netd::traffic() else { continue };
            if !t.enabled || !t.notify_new {
                continue;
            }
            // Только появившиеся после запуска оболочки (перезапуск не повторяет старые).
            for a in t.new_apps.into_iter().filter(|a| a.at + 10_000 >= started) {
                if !(a.uid == me || (t.all_users && a.uid < 1000)) || !shown.insert((a.app.clone(), a.at)) {
                    continue;
                }
                log::info!("сеть: программа {} впервые соединилась ({}:{})", a.app, a.remote, a.port);
                let can_block = t.all_users;
                syngui::async_runtime::run_on_main_thread(move || notify(ShellCtx::get(), a, can_block));
            }
        }
    });
}

fn notify(ctx: ShellCtx, a: syn_netd::NewApp, can_block: bool) {
    let entry = synshell_common::xdg::apps().iter().find(|e| e.exec.split_whitespace().next().map(|w| w.rsplit('/').next().unwrap_or(w)) == Some(a.app.as_str())).cloned();
    let name = entry.as_ref().map(|e| e.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| a.app.clone());
    let icon = entry.as_ref().map(|e| e.icon.clone()).filter(|i| !i.is_empty()).unwrap_or_else(|| "network-wired".into());
    let summary = t!("{name} впервые вышла в сеть", name = name);
    let dest = match (&a.host, a.remote.is_empty()) {
        (Some(h), _) => format!("{h} ({}:{})", a.remote, a.port),
        (None, false) => format!("{}:{}", a.remote, a.port),
        (None, true) => String::new(),
    };
    let body = if dest.is_empty() { t!("Процесс {c}. Запретить ей сеть можно и позже — в Мониторе ресурсов → «Сеть».", c = a.comm) } else { t!("Соединилась с {d} (процесс {c}).", d = dest, c = a.comm) };
    let mut actions = vec![("allow", t!("Разрешить"))];
    if can_block {
        actions.push(("block", t!("Запретить сеть")));
    }
    actions.push(("default", t!("Соединения")));
    let acts: Vec<(&str, &str)> = actions.iter().map(|(k, l)| (*k, l.as_str())).collect();
    let app = a.app.clone();
    notifications::local_actions(ctx, &format!("net-new:{}", a.app), &summary, &body, &icon, &acts, move |action| match action {
        "block" => {
            let app = app.clone();
            std::thread::spawn(move || {
                if let Err(e) = syn_netd::set_app_policy(&app, Some(syn_netd::AppPolicy::Block)) {
                    log::warn!("сеть: не запретить {app}: {e}");
                }
            });
        }
        // Имя в одинарных кавычках для sh -c (у программ Wine бывают пробелы).
        "default" => crate::actions::spawn(&format!("syn-systeminfo --connections '{}'", app.replace('\'', "'\\''"))),
        _ => {}
    });
}
