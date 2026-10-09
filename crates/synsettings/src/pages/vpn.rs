//! VPN: соединения NetworkManager (WireGuard, OpenVPN и другие плагины) — подключение, автоподключение,
//! импорт файла конфигурации, новое WireGuard-соединение с генерацией ключей, удаление.

use std::cell::Cell;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use synsystem::vpn::{self, Vpn, WireGuard};

use crate::state;
use crate::ui::*;

type R<T> = Option<std::result::Result<T, String>>;

#[derive(Clone, Copy)]
struct Sigs {
    list: RwSignal<R<Vec<Vpn>>>,
    /// Форма нового WireGuard открыта.
    adding: RwSignal<bool>,
    delete: RwSignal<Option<String>>,
}

thread_local! {
    static SIGS: Cell<Option<Sigs>> = const { Cell::new(None) };
}

fn sigs() -> Sigs {
    SIGS.with(|c| match c.get() {
        Some(s) => s,
        None => {
            let s = Sigs { list: use_signal(None), adding: use_signal(false), delete: use_signal(None) };
            c.set(Some(s));
            s
        }
    })
}

fn reload(s: Sigs) {
    std::thread::spawn(move || {
        let r = vpn::list();
        run_on_main_thread(move || s.list.set(Some(r)));
    });
}

fn act(label: impl AsRef<str>, f: impl FnOnce() -> std::result::Result<(), String> + Send + 'static) {
    let s = sigs();
    let label = tl(label);
    state::toast(format!("{label}…"));
    std::thread::spawn(move || {
        let r = f();
        run_on_main_thread(move || {
            state::toast(match r {
                Ok(()) => t!("{label}: готово", label = label),
                Err(e) => format!("{label}: {e}"),
            });
            reload(s);
        });
    });
}

pub fn vpn_page() -> W {
    let s = sigs();
    reload(s);
    let list = Reactive::new(move || -> Vec<W> {
        let mut rows: Vec<W> = Vec::new();
        match s.list.get() {
            None => rows.push(note(&t!("Чтение соединений…"))),
            Some(Err(e)) => rows.push(note(&t!("NetworkManager недоступен: {e}", e = e))),
            Some(Ok(l)) if l.is_empty() => rows.push(note(&t!("VPN-соединений нет. Импортируйте файл от провайдера или создайте WireGuard."))),
            Some(Ok(l)) => {
                for v in l {
                    rows.push(vpn_row(s, v));
                }
            }
        }
        vec![group(&t!("Соединения"), rows)]
    });
    let file = use_signal(String::new());
    let import = group(
        &t!("Импорт"),
        vec![row(
            t!("Файл конфигурации"),
            t!("WireGuard — .conf, OpenVPN — .ovpn (полный путь к файлу)"),
            Row::new()
                .gap(6.0)
                .child(TextField::new().placeholder(t!("~/Загрузки/vpn.conf")).on_change(move |t| file.set(t.to_string())).class("grow"))
                .child(button(&t!("Импорт"), move || {
                    let mut p = file.get_untracked();
                    if let Some(rest) = p.strip_prefix("~/") {
                        p = format!("{}/{rest}", std::env::var("HOME").unwrap_or_default());
                    }
                    act(&t!("Импорт VPN"), move || vpn::import(&p));
                })),
        )],
    );
    let add = Reactive::new(move || -> Vec<W> {
        if !s.adding.get() {
            return vec![group(
                "WireGuard",
                vec![row_inline(t!("Новое соединение"), t!("Ключи, адрес и сервер — от администратора VPN"), button(&t!("Создать"), move || s.adding.set(true)))],
            )];
        }
        vec![wireguard_form(s)]
    });
    page(
        "VPN",
        &t!("Виртуальные частные сети через NetworkManager: WireGuard, OpenVPN и другие плагины."),
        vec![boxed(list), boxed(add), import],
    )
}

fn vpn_row(s: Sigs, v: Vpn) -> W {
    let kind = if v.kind == "wireguard" { "WireGuard".to_string() } else { t!("VPN (плагин)") };
    if s.delete.get().as_deref() == Some(v.uuid.as_str()) {
        let u = v.uuid.clone();
        return row_inline(
            &t!("Удалить «{name}»?", name = v.name),
            "",
            Row::new().gap(6.0).child(button(&t!("Отмена"), move || s.delete.set(None))).child(primary_button(&t!("Удалить"), move || {
                s.delete.set(None);
                let u = u.clone();
                act(n_!("Удаление VPN"), move || vpn::delete(&u));
            })),
        );
    }
    let (u1, u2, u3) = (v.uuid.clone(), v.uuid.clone(), v.uuid.clone());
    let auto = v.autoconnect;
    row_inline(
        &v.name,
        &format!("{kind} · {}{}", if v.active { t!("подключено") } else { t!("отключено") }, if auto { t!(" · автоподключение") } else { "".to_string() }),
        Row::new()
            .gap(6.0)
            .child(Toggle::with_state(v.active).on_change(move |on| {
                let u = u1.clone();
                act(if on { n_!("Подключение VPN") } else { n_!("Отключение VPN") }, move || if on { vpn::up(&u) } else { vpn::down(&u) });
            }))
            .child(button(if auto { n_!("Не автоматически") } else { n_!("Автоматически") }, move || {
                let u = u2.clone();
                act(n_!("Автоподключение"), move || vpn::set_autoconnect(&u, !auto));
            }))
            .child(icon_button(icons::DELETE, move || s.delete.set(Some(u3.clone())))),
    )
}

fn wireguard_form(s: Sigs) -> W {
    let [name, private, address, dns, peer, endpoint, allowed, psk]: [RwSignal<String>; 8] = std::array::from_fn(|_| use_signal(String::new()));
    let pubkey = use_signal(String::new());
    let field = |label: &str, hint: &str, placeholder: &str, sig: RwSignal<String>| {
        row(label, hint, TextField::new().placeholder(placeholder).on_change(move |t| sig.set(t.to_string())).class("grow"))
    };
    let gen = Reactive::new(move || -> Vec<W> {
        let p = pubkey.get();
        let label = if p.is_empty() { t!("Создать ключи").to_string() } else { t!("Создать заново").to_string() };
        let mut r = Column::new().gap(6.0).child(button(&label, move || {
            std::thread::spawn(move || {
                let r = vpn::wireguard_keys();
                run_on_main_thread(move || match r {
                    Ok((k, p)) => {
                        private.set(k);
                        pubkey.set(p);
                    }
                    Err(e) => state::toast(e),
                });
            });
        }));
        if !p.is_empty() {
            r = r.child(Text::new(t!("Открытый ключ (отдайте администратору сервера): {p}", p = p)).max_lines(3).class("row-hint"));
        }
        vec![boxed(r)]
    });
    group(
        &t!("Новое соединение WireGuard"),
        vec![
            field(&t!("Имя"), &t!("Латиницей, до 15 символов (имя интерфейса)"), "wg-home", name),
            row(t!("Закрытый ключ"), t!("Или создайте пару ключей и отдайте открытый администратору сервера"), boxed(Column::new().gap(6.0).child(
                Reactive::new(move || -> Vec<W> {
                    let v = private.get();
                    vec![boxed(TextField::new().text(v).obscure(true).placeholder("PrivateKey").on_change(move |t| private.set(t.to_string())).class("grow"))]
                }),
            ).child(gen))),
            field(&t!("Адрес"), &t!("Адрес этого устройства в VPN"), "10.0.0.2/32", address),
            field("DNS", &t!("Необязательно"), "10.0.0.1", dns),
            field(&t!("Открытый ключ сервера"), "", "PublicKey", peer),
            field(&t!("Сервер"), &t!("Адрес и порт"), "vpn.example.com:51820", endpoint),
            field(&t!("Маршрутизировать"), &t!("Сети через VPN; пусто — весь трафик"), "0.0.0.0/0, ::/0", allowed),
            field(&t!("Общий ключ"), &t!("Необязательно (PresharedKey)"), "", psk),
            boxed(
                Row::new()
                    .gap(8.0)
                    .class("setting-row")
                    .child(DecoratedBox::new().class("grow"))
                    .child(button(&t!("Отмена"), move || s.adding.set(false)))
                    .child(primary_button(&t!("Сохранить"), move || {
                        let w = WireGuard {
                            name: name.get_untracked(),
                            private_key: private.get_untracked(),
                            address: address.get_untracked(),
                            dns: dns.get_untracked(),
                            peer_public_key: peer.get_untracked(),
                            endpoint: endpoint.get_untracked(),
                            allowed_ips: allowed.get_untracked(),
                            preshared_key: psk.get_untracked(),
                        };
                        s.adding.set(false);
                        act(&t!("Новое WireGuard"), move || vpn::add_wireguard(&w));
                    })),
            ),
        ],
    )
}
