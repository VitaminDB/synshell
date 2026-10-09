//! Окно пароля polkit для программ сеанса: `pkexec` без своего агента (GParted из
//! меню и любые другие). Агент сеанса — [`synsystem::polkit_agent`] (субъект
//! `unix-session`), он живёт в процессе оболочки. Его запросы приходят из рабочего
//! потока, поэтому окно открывается через сигнал в главном потоке.

use std::sync::Arc;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use syngui::GestureDetector;
use syngui_layer::{Anchor, KeyInfo, KeyboardInteractivity, Layer, SurfaceHooks, SurfaceId, SurfaceSpec};
use synsystem::polkit_agent::{self, AuthRequest, Prompter};

use crate::ctx::ShellCtx;
use crate::ui::{icon, mi, rx};

/// Открытый запрос пароля.
#[derive(Clone)]
struct AuthView(Arc<AuthRequest>);

impl PartialEq for AuthView {
    fn eq(&self, o: &Self) -> bool {
        self.0.id == o.0.id
    }
}

/// Окно пароля: запросы агента → сигнал, который читает поверхность.
struct AuthPrompter(RwSignal<Option<AuthView>>);

impl Prompter for AuthPrompter {
    fn ask(&self, req: AuthRequest) {
        let s = self.0;
        let v = AuthView(Arc::new(req));
        run_on_main_thread(move || s.set(Some(v)));
    }
    fn cancel(&self, id: u64) {
        let s = self.0;
        run_on_main_thread(move || {
            if s.get_untracked().is_some_and(|a| a.0.id == id) {
                s.set(None);
            }
        });
    }
}

thread_local! {
    static SURFACE: std::cell::Cell<Option<SurfaceId>> = const { std::cell::Cell::new(None) };
}

/// Зарегистрировать агент сеанса и открывать окно, пока polkit ждёт пароль.
pub fn start(ctx: ShellCtx) {
    let view = use_signal(None::<AuthView>);
    polkit_agent::set_prompter(AuthPrompter(view));
    create_effect(move || {
        let open = view.get().is_some();
        match (open, SURFACE.with(|s| s.get())) {
            (true, None) => {
                let spec = SurfaceSpec {
                    namespace: "syndesktop-auth".into(),
                    layer: Layer::Overlay,
                    anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
                    size: (0, 0),
                    margin: [0; 4],
                    exclusive_zone: -1,
                    keyboard: KeyboardInteractivity::Exclusive,
                    output: crate::manager::primary_output(&ctx),
                    auto_size: false,
                    clear_color: [0.0; 4],
                };
                let hooks = SurfaceHooks {
                    // Esc — отмена, как у кнопки «Отмена».
                    on_key: Some(Box::new(move |k: &KeyInfo| {
                        if k.pressed && k.keysym == "Escape" {
                            respond(view, None);
                            return true;
                        }
                        false
                    })),
                    on_closed: Some(Box::new(move || {
                        SURFACE.with(|s| s.set(None));
                    })),
                    ..Default::default()
                };
                let id = syngui_layer::create_surface_with(spec, hooks, move || Box::new(card(view)));
                SURFACE.with(|s| s.set(Some(id)));
            }
            (false, Some(id)) => {
                SURFACE.with(|s| s.set(None));
                syngui_layer::close_surface(id);
            }
            _ => {}
        }
    });
    // Регистрация блокирует поток до ответа polkitd — не задерживаем запуск оболочки.
    std::thread::spawn(|| {
        if let Err(e) = polkit_agent::ensure_session() {
            log::warn!("polkit-агент сеанса: {e}");
        }
    });
}

/// Отдать ответ агенту (`None` — отмена) и не трогать окно: его закроет `cancel`.
fn respond(view: RwSignal<Option<AuthView>>, pw: Option<String>) {
    if let Some(v) = view.get_untracked() {
        v.0.respond(pw);
    }
}

/// Кто спрашивает — по действию polkit: значок и понятное название.
fn source(action: &str) -> (&'static str, &'static str) {
    let pre = |p: &str| action.starts_with(p);
    if pre("org.freedesktop.policykit.exec") {
        ("\u{E86F}", n_!("Запуск программы от имени администратора"))
    } else if pre("org.freedesktop.udisks2.") {
        ("\u{E1DB}", n_!("Диски и разделы"))
    } else if pre("org.synshell.synpkg") || pre("org.archlinux.pacman") {
        ("\u{E1A1}", n_!("Установка и удаление программ"))
    } else if pre("org.freedesktop.NetworkManager.") || pre("net.connman.") {
        ("\u{E63E}", n_!("Сеть"))
    } else if pre("org.freedesktop.login1.") {
        ("\u{E8AC}", n_!("Питание и сеансы"))
    } else if pre("org.freedesktop.timedate1.") {
        ("\u{E192}", n_!("Дата и время"))
    } else if pre("org.freedesktop.systemd1.") {
        ("\u{E8B8}", n_!("Системные службы"))
    } else if pre("org.bluez.") {
        ("\u{E1A7}", "Bluetooth")
    } else if pre("org.freedesktop.packagekit.") || pre("org.freedesktop.fwupd.") {
        ("\u{E923}", n_!("Обновление системы"))
    } else {
        ("\u{EF3D}", n_!("Системное действие"))
    }
}

/// Полное имя пользователя из `/etc/passwd` (поле GECOS до запятой).
fn real_name(user: &str) -> Option<String> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    let line = passwd.lines().find(|l| l.split(':').next() == Some(user))?;
    let gecos = line.split(':').nth(4)?.split(',').next()?.trim();
    (!gecos.is_empty() && gecos != user).then(|| gecos.to_string())
}

fn avatar_path(user: &str) -> Option<std::path::PathBuf> {
    let home = std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|p| p.lines().find(|l| l.split(':').next() == Some(user)).and_then(|l| l.split(':').nth(5).map(std::path::PathBuf::from)));
    home.iter()
        .flat_map(|h| [h.join(".face"), h.join(".face.icon")])
        .chain(std::iter::once(std::path::PathBuf::from(format!("/var/lib/AccountsService/icons/{user}"))))
        .find(|p| p.is_file())
}

fn avatar(user: &str) -> Box<dyn Widget> {
    match avatar_path(user) {
        Some(p) => Box::new(Image::new(p.to_string_lossy()).fit(ImageFit::Cover).class("auth-avatar")),
        None => Box::new(
            DecoratedBox::new()
                .child(Text::new(user.chars().next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default()).class("auth-avatar-letter"))
                .class("auth-avatar"),
        ),
    }
}

/// Кнопка по размеру содержимого (`InputArea` занял бы всю высоту окна).
fn button(label: impl Into<String>, class: &'static str, on: impl FnMut() + Send + 'static) -> impl Widget {
    GestureDetector::new().on_click(on).child(DecoratedBox::new().child(Text::new(label.into()).class("auth-btn-label")).class(class))
}

fn card(view: RwSignal<Option<AuthView>>) -> impl Widget {
    // Введённое (для кнопки «Подтвердить»), показ пароля, ожидание ответа PAM, подробности.
    let pw = use_signal(String::new());
    let show = use_signal(false);
    let busy = use_signal(false);
    let details = use_signal(false);
    // Новый запрос (в том числе после неверного пароля) — поле с нуля, ожидание снято.
    create_effect(move || {
        let _ = view.get();
        pw.set(String::new());
        busy.set(false);
    });
    let submit = move || {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        respond(view, Some(pw.get_untracked()));
    };

    let head = rx(move || {
        let v = view.get();
        let (msg, action) = v.as_ref().map(|v| (v.0.message.clone(), v.0.action_id.clone())).unwrap_or_default();
        let (glyph, what) = source(&action);
        Box::new(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(
                    DecoratedBox::new()
                        .child(
                            Column::new()
                                .main_axis_alignment(MainAxisAlignment::Center)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(icon("\u{EF3D}").class("auth-badge-icon")),
                        )
                        .class("auth-badge"),
                )
                .child(Text::new(t!("Требуется подтверждение")).class("auth-title"))
                .child(Text::new(msg).max_lines(4).class("auth-message"))
                .child(
                    DecoratedBox::new()
                        .child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(glyph).class("auth-chip-icon")).child(Text::new(syngui::i18n::t(what)).class("auth-chip-label")))
                        .class("auth-chip"),
                ),
        )
    });

    let who = rx(move || {
        let user = view.get().map(|v| v.0.user.clone()).unwrap_or_default();
        let me = std::env::var("USER").unwrap_or_default();
        let name = real_name(&user).unwrap_or_else(|| user.clone());
        let hint = if user == "root" {
            t!("Пароль суперпользователя (root)").to_string()
        } else if user == me {
            t!("{user} · ваш пароль", user = user)
        } else {
            t!("{user} · пароль администратора", user = user)
        };
        Box::new(
            DecoratedBox::new()
                .child(
                    Row::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(avatar(&user))
                        .child(Column::new().gap(2.0).child(Text::new(name).class("auth-user")).child(Text::new(hint).class("auth-user-hint"))),
                )
                .class("auth-who"),
        )
    });

    let field = rx(move || {
        let v = view.get();
        let shown = show.get();
        let wait = busy.get();
        let had_error = v.as_ref().is_some_and(|v| v.0.error.is_some());
        let prompt = v.as_ref().map(|v| v.0.prompt.trim_end_matches(':').trim().to_string()).filter(|p| !p.is_empty() && !p.eq_ignore_ascii_case("password")).unwrap_or_else(|| t!("Пароль").into());
        let f = TextField::with_text(pw.get_untracked())
            .obscure(!shown)
            .autofocus(true)
            .disabled(wait)
            .placeholder(prompt)
            .prefix_icon(mi::LOCK)
            .suffix_icon(if shown { "\u{E8F5}" } else { "\u{E8F4}" })
            .on_suffix_click(move || show.set(!show.get_untracked()))
            .on_change(move |t| pw.set(t.to_string()))
            .on_submit(move |_| submit())
            .class("auth-field");
        // После неверного пароля поле «встряхивается».
        Box::new(DecoratedBox::new().child(f).class(if had_error && !wait { "auth-field-box auth-shake" } else { "auth-field-box" }))
    });

    let status = rx(move || {
        let e = view.get().and_then(|v| v.0.error.clone()).filter(|e| !e.trim().is_empty());
        match e {
            Some(e) if !busy.get() => Box::new(
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(icon("\u{E000}").class("auth-error-icon"))
                    .child(Text::new(e).class("auth-error")),
            ),
            _ => Box::new(DecoratedBox::new()),
        }
    });

    let more = rx(move || {
        let open = details.get();
        let action = view.get().map(|v| v.0.action_id.clone()).unwrap_or_default();
        let toggle = GestureDetector::new().on_click(move || details.set(!details.get_untracked())).child(
            Row::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(t!("Подробности")).class("auth-link"))
                .child(icon(if open { "\u{E5CE}" } else { "\u{E5CF}" }).class("auth-link-icon")),
        );
        let mut col = Column::new().gap(6.0).child(toggle);
        if open {
            col = col.child(Text::new(t!("Действие: {action}", action = action)).selectable(true).class("auth-details"));
        }
        Box::new(col)
    });

    let actions = rx(move || {
        let wait = busy.get();
        let ok: Box<dyn Widget> = if wait {
            Box::new(button(t!("Проверка…"), "auth-btn auth-btn-primary auth-btn-busy", || {}))
        } else {
            Box::new(button(t!("Подтвердить"), "auth-btn auth-btn-primary", submit))
        };
        Box::new(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(button(t!("Отмена"), "auth-btn", move || respond(view, None)))
                .child(ok),
        )
    });

    let body = Column::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(head)
        .child(who)
        .child(Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(field).child(status))
        .child(more)
        .child(actions);
    Stack::new()
        .fit(StackFit::Expand)
        .child(DecoratedBox::new().class("auth-scrim"))
        .child(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().child(body).class("auth-card")),
        )
}
