//! synphone — «Телефон»: набор, журнал, контакты (общие `synshell_common::contacts`), запись разговоров,
//! экран звонка с фото абонента и визуализацией голосов, настройки (запись, услуги оператора, импорт/экспорт
//! контактов) через демон модема `synmodemd`.
//!
//! Один процесс живёт весь сеанс (`synphone --background` из автозапуска): входящий показывает окно сразу,
//! закрытие окна его прячет (разговор и запись идут дальше). Повторный запуск (`synphone НОМЕР`, `--log`,
//! `--contacts`) передаёт команду живому процессу. На экране блокировки звонок показывает оболочка.
//!
//! `--screenshot СНИМОК.png [--size ШxВ] [--scale N] [--page dial|log|contacts|contact|edit|settings|call|incoming]`
//! — снимок окна с придуманными данными (без модема).

mod ipc;
mod prefs;
mod state;
mod ui;
mod voice;

use syn_modem::api::{self, Call, CallKind, CallRecord, CallState, Status};
use syn_modem::time as tm;
use synshell_common::contacts::{Contact, Phone};
use synshell_common::Config;
use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;

use ipc::Cmd;
use prefs::Prefs;
use state::{Page, St, Tab};

fn main() {
    synshell_common::i18n::init(&synshell_common::catalogs!("../i18n"));
    let mut cmd: Option<Cmd> = None;
    let mut background = false;
    let mut shot: Option<String> = None;
    let mut size = (430u32, 900u32);
    let mut scale = 2.0f64;
    let mut page = String::from("dial");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("synphone [NUMBER] [--log] [--contacts] [--background] [--screenshot FILE.png [--size WxH] [--scale N] [--page dial|log|contacts|contact|edit|settings|call|incoming]]");
                return;
            }
            "--log" => cmd = Some(Cmd::Log),
            "--contacts" => cmd = Some(Cmd::Contacts),
            "--background" => background = true,
            "--screenshot" => shot = args.next(),
            "--size" => {
                if let Some((w, h)) = args.next().as_deref().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = args.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            "--page" => page = args.next().unwrap_or(page),
            a if !a.starts_with("--") => cmd = Some(Cmd::Dial(a.to_string())),
            _ => {}
        }
    }
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    if let Some(out) = shot {
        screenshot(&out, &mss, size, scale, &page);
        return;
    }
    // процесс уже живёт — передать команду и выйти
    if ipc::forward(cmd.as_ref().unwrap_or(&Cmd::Show)) {
        return;
    }
    // фоновый запуск без модема на компьютере — не нужен
    if background && !std::path::Path::new(api::SOCKET).exists() && !synshell_common::csd::is_phone() {
        return;
    }
    let st = St::new(Prefs::load(), false);
    if let Some(c) = cmd {
        apply(st, c);
    }
    state::effects(st);
    ui::effects(st);
    state::reload_contacts(st);
    state::reload_recordings(st);
    state::watch(st);
    state::ticker(st);
    watch_contacts(st);
    ipc::listen(move |c| {
        run_on_main_thread(move || {
            apply(st, c);
            syngui::app::show_main_window();
        })
    });
    let win = st.window;
    App::new()
        .title(t!("Телефон"))
        .app_id("synphone")
        .size(420, 820)
        .min_size(320, 520)
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_window_state(win)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(synshell_common::live::styles(mss.clone(), theme))
        .start_hidden(background)
        .hide_on_close(true)
        .run(move |_| ui::root(st));
}

/// Команда от второго запуска.
fn apply(st: St, c: Cmd) {
    match c {
        Cmd::Show => {}
        Cmd::Log => {
            st.page.set(Page::None);
            st.tab.set(Tab::Log);
        }
        Cmd::Contacts => {
            st.page.set(Page::None);
            st.tab.set(Tab::Contacts);
        }
        Cmd::Dial(n) => {
            st.page.set(Page::None);
            st.tab.set(Tab::Dial);
            st.number.set(n);
        }
    }
}

/// Контакты могли поменять другие программы (SMS, импорт) — перечитать при изменении каталога.
fn watch_contacts(st: St) {
    std::thread::spawn(move || {
        let stamp = || -> u128 {
            let d = synshell_common::contacts::dir();
            let Ok(rd) = std::fs::read_dir(&d) else { return 0 };
            rd.flatten().filter_map(|e| e.metadata().ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|t| t.as_millis())).fold(0u128, |a, b| a.wrapping_add(b))
        };
        let mut last = stamp();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3));
            let s = stamp();
            if s != last {
                last = s;
                run_on_main_thread(move || state::reload_contacts(st));
            }
        }
    });
}

fn theme(cfg: &Config) -> String {
    synshell_common::style::AppStyle::new(cfg, "synphone").base(include_str!("../styles/synphone.mss")).build()
}

/// Снимок окна с придуманными данными.
fn screenshot(out: &str, mss: &str, size: (u32, u32), scale: f64, page: &str) {
    let page = page.to_string();
    if matches!(page.as_str(), "call") {
        std::env::set_var("SYNPHONE_DEMO_TAP", "1");
    }
    let r = syngui::shot::render_png(
        move || {
            let st = St::new(Prefs::default(), true);
            demo(st, &page);
            state::effects(st);
            ui::effects(st);
            if page == "call" {
                voice::set_call(true);
                std::thread::sleep(std::time::Duration::from_millis(1200));
            }
            ui::root(st)
        },
        mss,
        size,
        scale,
        out,
    );
    match r {
        Ok((w, h)) => println!("{out}: {w}x{h}"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

fn demo(st: St, page: &str) {
    let now = tm::now();
    let mut status = Status { present: true, radio: true, operator: "Demo Mobile".into(), plmn: "001/01".into(), technology: "LTE".into(), bars: Some(4), ..Default::default() };
    status.registration = api::Registration::Home;
    match page {
        "call" => status.calls = vec![Call { id: 1, number: "+1 555 0142".into(), state: CallState::Active, incoming: true, answered_at: Some(now - 83), started_at: now - 90 }],
        "incoming" => status.calls = vec![Call { id: 1, number: "+1 555 0198".into(), state: CallState::Incoming, incoming: true, answered_at: None, started_at: now - 3 }],
        _ => {}
    }
    st.status.set(Some(status));
    let c = |uid: &str, name: &str, num: &str, kind: &str, fav: bool| Contact { uid: uid.into(), name: name.into(), phones: vec![Phone { number: num.into(), kind: kind.into() }], favorite: fav, ..Default::default() };
    let mut anna = c("demo-anna", "Anna Smith", "+1 555 0142", "CELL", true);
    anna.emails.push("anna@example.org".into());
    anna.phones.push(Phone { number: "+1 555 0100".into(), kind: "WORK".into() });
    anna.note = "Book club on Thursdays".into();
    st.contacts.set(vec![
        anna,
        c("demo-ben", "Ben Carter", "+1 555 0198", "CELL", true),
        c("demo-chloe", "Chloe Martin", "+1 555 0117", "HOME", false),
        c("demo-david", "David Lee", "+1 555 0163", "WORK", true),
        c("demo-emma", "Emma Wilson", "+1 555 0129", "CELL", false),
        c("demo-frank", "Frank Moore", "+1 555 0181", "CELL", false),
    ]);
    let rec = |id: u64, number: &str, kind: CallKind, ago: i64, duration: u32, new: bool| CallRecord { id, number: number.into(), kind, time: now - ago, duration, new };
    st.log.set(vec![
        rec(1, "+1 555 0142", CallKind::Incoming, 25 * 60, 312, false),
        rec(2, "+1 555 0198", CallKind::Missed, 2 * 3600, 0, page != "log"),
        rec(3, "+1 555 0117", CallKind::Outgoing, 5 * 3600, 74, false),
        rec(4, "", CallKind::Rejected, 26 * 3600, 0, false),
        rec(5, "+1 555 0177", CallKind::Outgoing, 28 * 3600, 0, false),
        rec(6, "+1 555 0142", CallKind::Incoming, 3 * 86400, 1265, false),
        rec(7, "+1 555 0129", CallKind::Outgoing, 4 * 86400, 45, false),
        rec(8, "+1 555 0181", CallKind::Missed, 6 * 86400, 0, false),
    ]);
    match page {
        "log" => st.tab.set(Tab::Log),
        "contacts" => st.tab.set(Tab::Contacts),
        "contact" => {
            st.tab.set(Tab::Contacts);
            st.page.set(Page::Contact("demo-anna".into()));
        }
        "edit" => st.page.set(Page::Edit(Some("demo-anna".into()), String::new())),
        "settings" => {
            st.page.set(Page::Settings);
            st.services.set(Some(api::CallServices {
                waiting: Some(true),
                clir: "network".into(),
                forwards: vec![api::Forward { reason: "busy".into(), active: true, number: "+1 555 0100".into(), timer: None }, api::Forward { reason: "no-reply".into(), active: false, number: String::new(), timer: Some(20) }],
                error: String::new(),
            }));
        }
        "call" => st.rec.set(Some((std::path::PathBuf::from("/tmp/demo.ogg"), now - 41))),
        _ => st.number.set("+1 555 01".into()),
    }
}
