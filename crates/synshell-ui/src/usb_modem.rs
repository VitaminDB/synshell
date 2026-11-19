//! Телефон по USB как модем: служба synnetd без решения пользователя модем не поднимает
//! (иначе телефон, подключённый ради файлов и зарядки, сам начинал раздавать интернет).
//! Здесь — вопрос: уведомление «Подключить как модем» / «Только зарядка и файлы»; ответ
//! служба запоминает для этого устройства (поменять — Параметры → Подключения к интернету).

use std::collections::HashSet;
use std::time::Duration;

use syngui::t;

use crate::ctx::ShellCtx;
use crate::notifications;

const POLL: Duration = Duration::from_secs(3);

pub fn start(_ctx: ShellCtx) {
    let _ = std::thread::Builder::new().name("shell-usb-modem".into()).spawn(|| {
        // Уже спросили (до отключения устройства).
        let mut asked: HashSet<String> = HashSet::new();
        loop {
            std::thread::sleep(POLL);
            if !std::path::Path::new(syn_netd::SOCKET).exists() {
                continue;
            }
            let Ok(st) = syn_netd::status() else { continue };
            let present: HashSet<String> = st.usb_modems.iter().map(|m| m.key.clone()).collect();
            asked.retain(|k| present.contains(k));
            if !st.config.usb_dhcp {
                continue;
            }
            for m in st.usb_modems.into_iter().filter(|m| m.choice.is_none() && !m.managed_elsewhere) {
                if !asked.insert(m.key.clone()) {
                    continue;
                }
                log::info!("usb-модем: {} ({}, {}) — спрашиваю, подключать ли как модем", m.name, m.iface, m.key);
                syngui::async_runtime::run_on_main_thread(move || ask(ShellCtx::get(), m));
            }
        }
    });
}

fn ask(ctx: ShellCtx, m: syn_netd::UsbModem) {
    let name = if m.name.is_empty() { m.iface.clone() } else { m.name.clone() };
    let summary = t!("Телефон по USB: {name}", name = name);
    let body = t!("Подключить как модем — интернет через телефон? Пока вы не выбрали, интернет через него не идёт: работают зарядка и файлы.");
    let actions = [("modem", t!("Подключить как модем")), ("no", t!("Только зарядка и файлы")), ("default", t!("Параметры"))];
    let acts: Vec<(&str, &str)> = actions.iter().map(|(a, l)| (*a, l.as_str())).collect();
    let (key, dev) = (m.key.clone(), name.clone());
    notifications::local_actions(ctx, &format!("usb-modem:{}", m.key), &summary, &body, "phone", &acts, move |action| {
        let choice = match action {
            "modem" => syn_netd::UsbChoice::Modem,
            "no" => syn_netd::UsbChoice::No,
            _ => {
                crate::actions::spawn("synsettings uplinks");
                return;
            }
        };
        let (key, dev) = (key.clone(), dev.clone());
        std::thread::spawn(move || {
            if let Err(e) = syn_netd::set_usb_choice(&key, &dev, Some(choice)) {
                log::warn!("usb-модем: не сохранить выбор: {e}");
            }
        });
    });
}
