//! Курьер: каналы мессенджеров (привязка WhatsApp), телефонный сеанс Claude
//! Code и какие уведомления пересылать. Настройки — `courier.toml` в каталоге
//! настроек synshell; после сохранения служба `syncourier` перечитывает их.

use std::cell::Cell;

use syn_courier::{ChannelState, ClaudeEvents, Config, NotifyMode, Request, Status};
use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;

use crate::state;
use crate::ui::*;

type R<T> = Option<std::result::Result<T, String>>;

#[derive(Clone, Copy)]
struct Sigs {
    status: RwSignal<R<Status>>,
    cfg: RwSignal<Config>,
}

thread_local! {
    static SIGS: Cell<Option<Sigs>> = const { Cell::new(None) };
}

fn sigs() -> Sigs {
    SIGS.with(|c| match c.get() {
        Some(s) => s,
        None => {
            let s = Sigs { status: use_signal(None), cfg: use_signal(Config::load()) };
            c.set(Some(s));
            s
        }
    })
}

fn reload(s: Sigs) {
    std::thread::spawn(move || {
        let r = syn_courier::status();
        run_on_main_thread(move || s.status.set(Some(r)));
    });
}

/// Запрос службе в фоне, затем свежее состояние.
fn req(s: Sigs, r: Request) {
    std::thread::spawn(move || {
        if let Err(e) = syn_courier::request(&r) {
            run_on_main_thread(move || state::toast(e));
        }
        let st = syn_courier::status();
        run_on_main_thread(move || s.status.set(Some(st)));
    });
}

/// Изменить настройки, сохранить и попросить службу перечитать.
fn change(s: Sigs, f: impl FnOnce(&mut Config)) {
    let mut c = s.cfg.get_untracked();
    f(&mut c);
    match c.save() {
        Ok(()) => {
            s.cfg.set(c);
            req(s, Request::Reload);
        }
        Err(e) => state::toast(t!("Не сохранилось: {e}", e = e)),
    }
}

/// QR-код привязки картинкой.
fn qr(data: &str) -> Option<W> {
    let code = qrcode::QrCode::new(data.as_bytes()).ok()?;
    let (w, quiet, scale) = (code.width() as u32, 4u32, 5u32);
    let side = (w + quiet * 2) * scale;
    let mut px = vec![255u8; (side * side * 4) as usize];
    for y in 0..w {
        for x in 0..w {
            if code[(x as usize, y as usize)] == qrcode::Color::Dark {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let o = ((((y + quiet) * scale + dy) * side + (x + quiet) * scale + dx) * 4) as usize;
                        px[o..o + 3].copy_from_slice(&[0, 0, 0]);
                    }
                }
            }
        }
    }
    let key = format!("courier-qr-{}", data.len() as u64 * 31 + data.bytes().map(|b| b as u64).sum::<u64>());
    Some(boxed(DecoratedBox::new().class("courier-qr").child(Image::from_rgba(key, side, side, px).fit(ImageFit::Contain).class("courier-qr-image"))))
}

pub fn courier() -> W {
    let s = sigs();
    reload(s);
    // Пока ждём сканирования — обновлять состояние.
    std::thread::spawn(move || {
        for _ in 0..600 {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let linking = matches!(syn_courier::status(), Ok(ref st) if st.channels.iter().any(|c| matches!(c.state, ChannelState::Linking | ChannelState::Connecting)));
            if linking {
                let st = syn_courier::status();
                run_on_main_thread(move || s.status.set(Some(st)));
            }
        }
    });

    let channels = Reactive::new(move || -> Vec<W> {
        let mut rows: Vec<W> = Vec::new();
        match s.status.get() {
            None => rows.push(hint(t!("Связываюсь со службой…"))),
            Some(Err(e)) => {
                rows.push(hint(t!("Служба Курьера не запущена: {e}", e = e)));
                rows.push(row_inline(
                    t!("Запустить службу"),
                    "systemctl --user enable --now syncourier",
                    primary_button(&t!("Запустить"), move || {
                        std::thread::spawn(move || {
                            let _ = std::process::Command::new("systemctl").args(["--user", "enable", "--now", "syncourier"]).status();
                            std::thread::sleep(std::time::Duration::from_millis(800));
                            let st = syn_courier::status();
                            run_on_main_thread(move || s.status.set(Some(st)));
                        });
                    }),
                ));
            }
            Some(Ok(st)) => {
                for c in &st.channels {
                    if c.id == "telegram" {
                        rows.extend(telegram_rows(s, c));
                        continue;
                    }
                    let (hint_text, control): (String, W) = match c.state {
                        ChannelState::Online => (
                            t!("Подключён · {a}", a = c.account.clone().unwrap_or_default()),
                            boxed(button(&t!("Отключить"), move || req(s, Request::Unlink { channel: "whatsapp".into() }))),
                        ),
                        ChannelState::Linking => (t!("Отсканируйте QR-код телефоном"), boxed(button(&t!("Отмена"), move || req(s, Request::Unlink { channel: "whatsapp".into() })))),
                        ChannelState::Connecting => (t!("Подключается…"), boxed(Text::new(""))),
                        ChannelState::Error => (c.error.clone().unwrap_or_default(), boxed(primary_button(&t!("Подключить"), move || req(s, Request::Link { channel: "whatsapp".into() })))),
                        ChannelState::Off | ChannelState::Unlinked => (
                            t!("Не подключён — привязка как связанное устройство по QR-коду"),
                            boxed(primary_button(&t!("Подключить"), move || req(s, Request::Link { channel: "whatsapp".into() }))),
                        ),
                    };
                    rows.push(row_inline(&c.title, hint_text, control));
                    if c.state == ChannelState::Linking {
                        if let Some(q) = c.qr.as_deref().and_then(qr) {
                            rows.push(boxed(Row::new().main_axis_alignment(MainAxisAlignment::Center).class("setting-row").child(q)));
                            rows.push(hint(t!("На телефоне: WhatsApp → Настройки → Связанные устройства → Привязка устройства.")));
                        }
                    }
                }
                rows.push(row_inline(t!("Почта"), t!("Скоро"), Text::new("")));
                let d = s.cfg.get().delivery;
                let sel = match d {
                    syn_courier::Delivery::Last => 0,
                    syn_courier::Delivery::Whatsapp => 1,
                    syn_courier::Delivery::Telegram => 2,
                    syn_courier::Delivery::All => 3,
                };
                rows.push(row(
                    t!("Куда отправлять"),
                    t!("Когда подключены оба мессенджера: ответы и уведомления. В WhatsApp сообщения в «Сообщения себе» приходят без уведомлений телефона, бот Telegram — с уведомлениями"),
                    SegmentedButton::new(vec![t!("Откуда писал"), "WhatsApp".to_string(), "Telegram".to_string(), t!("В оба")]).selected(sel).on_change(move |i| {
                        use syn_courier::Delivery;
                        change(s, |c| c.delivery = [Delivery::Last, Delivery::Whatsapp, Delivery::Telegram, Delivery::All][i.min(3)])
                    }),
                ));
            }
        }
        vec![group(&t!("Каналы"), rows)]
    });

    let claude = Reactive::new(move || -> Vec<W> {
        let c = s.cfg.get().claude;
        let st = s.status.get().and_then(|r| r.ok()).map(|s| s.claude);
        let state_hint = match &st {
            Some(x) if x.busy => t!("думает…"),
            Some(x) if x.running => t!("запущен, ждёт сообщения"),
            Some(_) => t!("запустится с первым сообщением"),
            None => String::new(),
        };
        let name = use_signal(c.name.clone());
        let cwd = use_signal(c.cwd.clone());
        let model = use_signal(c.model.clone());
        vec![group(
            "Claude Code",
            vec![
                row_inline(t!("Разговор с Claude"), state_hint, Toggle::with_state(c.enabled).on_change(move |v| change(s, |c| c.claude.enabled = v))),
                row(t!("Имя сеанса"), t!("Так его видят другие сеансы (SendMessage)"), TextField::new().text(c.name.clone()).on_change(move |t| name.set(t.trim().to_string())).class("grow")),
                row(t!("Рабочая папка"), t!("Где Claude работает по умолчанию; /new ПАПКА — сменить на время"), TextField::new().text(c.cwd.clone()).on_change(move |t| cwd.set(t.trim().to_string())).class("grow")),
                row(t!("Модель"), t!("opus, sonnet, haiku или полное имя; пусто — как в Claude Code"), TextField::new().text(c.model.clone()).placeholder("opus").on_change(move |t| model.set(t.trim().to_string())).class("grow")),
                row(
                    t!("Разрешения"),
                    t!("Как у ваших сеансов — тогда они принимают сообщения друг друга"),
                    {
                        let mut dd = Dropdown::new().width(260.0);
                        for (v, l) in [("bypassPermissions", t!("Без вопросов (bypass)")), ("acceptEdits", t!("Правки без вопросов")), ("default", t!("Спрашивать (ответить из мессенджера нельзя)"))] {
                            dd = dd.item(DropdownItem::new(v, l));
                        }
                        dd.selected(c.permission_mode.clone()).on_change(move |v: &str| {
                            let v = v.to_string();
                            change(s, |c| c.claude.permission_mode = v)
                        })
                    },
                ),
                boxed(Row::new().gap(8.0).class("setting-row").child(DecoratedBox::new().class("grow")).child(primary_button(&t!("Сохранить"), move || {
                    let (n, d, m) = (name.get_untracked(), cwd.get_untracked(), model.get_untracked());
                    change(s, |c| {
                        if !n.is_empty() {
                            c.claude.name = n;
                        }
                        c.claude.cwd = if d.is_empty() { "~".into() } else { d };
                        c.claude.model = m;
                    });
                }))),
            ],
        )]
    });

    let notify = Reactive::new(move || -> Vec<W> {
        let n = s.cfg.get().notify;
        let mode = match n.mode {
            NotifyMode::Claude => 0,
            NotifyMode::Away => 1,
            NotifyMode::All => 2,
        };
        let ev = match n.claude {
            ClaudeEvents::Away => 0,
            ClaudeEvents::Always => 1,
            ClaudeEvents::Never => 2,
        };
        let exclude = use_signal(n.exclude.join(", "));
        vec![group(
            &t!("Уведомления"),
            vec![
                row(
                    t!("Уведомления системы"),
                    t!("Что пересылать из уведомлений программ"),
                    SegmentedButton::new(vec![t!("Не пересылать"), t!("Когда меня нет"), t!("Все")]).selected(mode).on_change(move |i| {
                        change(s, |c| c.notify.mode = [NotifyMode::Claude, NotifyMode::Away, NotifyMode::All][i.min(2)])
                    }),
                ),
                row(
                    t!("События Claude Code"),
                    t!("Ход закончен, нужен ответ или разрешение"),
                    SegmentedButton::new(vec![t!("Когда меня нет"), t!("Всегда"), t!("Никогда")]).selected(ev).on_change(move |i| {
                        change(s, |c| c.notify.claude = [ClaudeEvents::Away, ClaudeEvents::Always, ClaudeEvents::Never][i.min(2)])
                    }),
                ),
                row(
                    t!("«Меня нет»"),
                    t!("Экран заблокирован или столько минут без клавиатуры и мыши"),
                    SpinBox::new().range(1.0, 120.0).step(1.0).width(120.0).value(n.away_minutes as f64).on_change(move |v| change(s, |c| c.notify.away_minutes = v.round().max(1.0) as u32)),
                ),
                row(
                    t!("Не пересылать от программ"),
                    t!("Имена программ через запятую"),
                    Row::new()
                        .gap(6.0)
                        .child(TextField::new().text(n.exclude.join(", ")).placeholder("Firefox, Telegram").on_change(move |t| exclude.set(t.to_string())).class("grow"))
                        .child(button(&t!("Сохранить"), move || {
                            let list: Vec<String> = exclude.get_untracked().split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
                            change(s, |c| c.notify.exclude = list);
                        })),
                ),
            ],
        )]
    });

    page(
        t!("Курьер"),
        t!("Связь с компьютером через мессенджеры (WhatsApp, Telegram): уведомления и разговор с Claude Code. /help — команды."),
        vec![boxed(channels), boxed(claude), boxed(notify)],
    )
}

/// Telegram: токен бота, QR-код привязки, отключение.
fn telegram_rows(s: Sigs, c: &syn_courier::ChannelStatus) -> Vec<W> {
    let mut rows: Vec<W> = Vec::new();
    let token = use_signal(String::new());
    let save_token = move || {
        let t = token.get_untracked();
        if t.trim().is_empty() {
            return;
        }
        state::toast(t!("Проверяю токен…"));
        std::thread::spawn(move || {
            let r = syn_courier::request(&Request::SetToken { channel: "telegram".into(), token: t });
            run_on_main_thread(move || {
                match r {
                    Ok(_) => state::toast(t!("Бот принят — отсканируйте QR-код")),
                    Err(e) => state::toast(e),
                }
                reload(s);
            });
        });
    };
    match c.state {
        ChannelState::Online => rows.push(row_inline(
            "Telegram",
            t!("Подключён · {a}", a = c.account.clone().unwrap_or_default()),
            button(&t!("Отключить"), move || req(s, Request::Unlink { channel: "telegram".into() })),
        )),
        ChannelState::Linking => {
            rows.push(row_inline(
                "Telegram",
                t!("Бот {a}: отсканируйте QR-код камерой телефона и нажмите «Старт»", a = c.account.clone().unwrap_or_default()),
                button(&t!("Сменить бота"), move || req(s, Request::Unlink { channel: "telegram".into() })),
            ));
            if let Some(q) = c.qr.as_deref().and_then(qr) {
                rows.push(boxed(Row::new().main_axis_alignment(MainAxisAlignment::Center).class("setting-row").child(q)));
            }
            if let Some(link) = &c.qr {
                rows.push(hint(t!("Или откройте ссылку на телефоне: {l}", l = link)));
            }
        }
        _ => {
            let hint_text = c.error.clone().unwrap_or_else(|| t!("Личный бот: в Telegram откройте @BotFather → /newbot, придумайте имя — он пришлёт токен; вставьте его сюда"));
            rows.push(row(
                "Telegram",
                hint_text,
                Row::new()
                    .gap(6.0)
                    .child(TextField::new().placeholder("123456789:AA…").obscure(true).on_change(move |t| token.set(t.trim().to_string())).on_submit(move |_| save_token()).class("grow"))
                    .child(primary_button(&t!("Подключить"), save_token)),
            ));
        }
    }
    rows
}

fn hint(text: impl AsRef<str>) -> W {
    boxed(Text::new(tl(text)).class("row-hint pad"))
}
