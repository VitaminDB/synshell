use std::path::Path;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use synshell_common::autologin::{self, Autologin};

use crate::state;
use crate::ui::*;

fn current_user() -> Option<String> {
    let uid = unsafe { libc::getuid() };
    autologin::login_users().into_iter().find(|u| u.uid == uid).map(|u| u.name).or_else(|| std::env::var("USER").ok())
}

fn save(a: Autologin) {
    std::thread::spawn(move || {
        let r = std::process::Command::new("pkexec")
            .arg(autologin::HELPER)
            .arg(if a.enabled { "true" } else { "false" })
            .arg(&a.user)
            .arg(&a.session)
            .output();
        let msg = match r {
            Ok(o) if o.status.success() => None,
            Ok(o) if matches!(o.status.code(), Some(126) | Some(127)) => Some(t!("Изменение отменено: нет прав администратора").to_string()),
            Ok(o) => Some(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => Some(format!("pkexec: {e}")),
        };
        run_on_main_thread(move || {
            match msg {
                None => state::toast(t!("Автоматический вход сохранён")),
                Some(e) => state::toast(t!("Автоматический вход: {e}", e = e)),
            }
            state::bump();
        });
    });
}

pub fn login() -> W {
    let users = autologin::login_users();
    let sessions = autologin::sessions();
    let saved = autologin::load();
    let cur = autologin::effective(saved.as_ref(), &users, current_user().as_deref());
    let installed = Path::new(autologin::HELPER).exists();

    let mut rows = Vec::new();
    let a = cur.clone();
    rows.push(row_inline(
        t!("Автоматический вход"),
        t!("При включении компьютера экран входа сразу открывает сеанс без пароля"),
        Toggle::with_state(cur.enabled).disabled(!installed || users.is_empty()).on_change(move |on| {
            save(Autologin { enabled: on, ..a.clone() });
        }),
    ));

    let mut dd = Dropdown::new().width(260.0).max_height(320.0);
    for u in &users {
        dd = dd.item(DropdownItem::new(u.name.clone(), u.display()));
    }
    if !users.iter().any(|u| u.name == cur.user) && !cur.user.is_empty() {
        dd = dd.item(DropdownItem::new(cur.user.clone(), t!("{u} (нет в системе)", u = cur.user)));
    }
    let a = cur.clone();
    rows.push(row(
        t!("Пользователь"),
        t!("Чей сеанс открывается без пароля"),
        dd.selected(cur.user.as_str()).disabled(!installed || !cur.enabled).on_change(move |v: &str| {
            if v != a.user {
                save(Autologin { user: v.to_string(), ..a.clone() });
            }
        }),
    ));

    let mut dd = Dropdown::new().width(260.0).max_height(320.0);
    for s in &sessions {
        let label = match s.kind {
            autologin::SessionKind::X11 => format!("{} (X11)", s.name),
            _ => s.name.clone(),
        };
        dd = dd.item(DropdownItem::new(s.id.clone(), label));
    }
    if !sessions.iter().any(|s| s.id == cur.session) {
        dd = dd.item(DropdownItem::new(cur.session.clone(), t!("{s} (не установлен)", s = cur.session)));
    }
    let a = cur.clone();
    rows.push(row(
        t!("Сеанс"),
        t!("Рабочий стол из /usr/share/wayland-sessions и xsessions"),
        dd.selected(cur.session.as_str()).disabled(!installed || !cur.enabled).on_change(move |v: &str| {
            if v != a.session {
                save(Autologin { session: v.to_string(), ..a.clone() });
            }
        }),
    ));

    let mut body = vec![group(t!("Автоматический вход"), rows)];
    if !installed {
        body.push(note(t!("Помощник {p} не установлен — переустановите synshell.", p = autologin::HELPER)));
    } else if users.is_empty() {
        body.push(note(t!("В системе нет обычных пользователей.")));
    }
    body.push(note(t!(
        "Настройка хранится в {f} и действует при следующем включении компьютера; после выхода из сеанса показывается \
         обычный экран входа. Изменение спрашивает пароль администратора.",
        f = autologin::file().display().to_string()
    )));
    page(t!("Вход в систему"), t!("Экран входа и автоматический вход"), body)
}
