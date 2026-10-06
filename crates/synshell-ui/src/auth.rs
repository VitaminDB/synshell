//! Окно пароля polkit для программ сеанса: `pkexec` без своего агента (GParted из
//! меню и любые другие). Агент сеанса — [`synsystem::polkit_agent`] (субъект
//! `unix-session`), он живёт в процессе оболочки. Его запросы приходят из рабочего
//! потока, поэтому окно открывается через сигнал в главном потоке.

use std::sync::Arc;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use syngui_layer::{Anchor, KeyInfo, KeyboardInteractivity, Layer, SurfaceHooks, SurfaceId, SurfaceSpec};
use synsystem::polkit_agent::{self, AuthRequest, Prompter};

use crate::ctx::ShellCtx;
use crate::ui::{icon, mi, InputArea};

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

fn card(view: RwSignal<Option<AuthView>>) -> impl Widget {
    let header = move || {
        let v = view.get();
        let msg = v.as_ref().map(|v| v.0.message.clone()).unwrap_or_default();
        let user = v.as_ref().map(|v| v.0.user.clone()).unwrap_or_default();
        Column::new()
            .gap(4.0)
            .child(Text::new(msg).class("auth-title"))
            .child(Text::new(format!("Пароль пользователя «{user}»")).class("auth-sub"))
    };
    let status = move || {
        let e = view.get().and_then(|v| v.0.error.clone()).unwrap_or_default();
        Text::new(e).class("auth-error")
    };
    let field = move || {
        // Новый запрос — новое поле: после неверного пароля оно очищается.
        let _ = view.get();
        TextField::new()
            .obscure(true)
            .autofocus(true)
            .placeholder("Пароль")
            .on_submit(move |t| respond(view, Some(t.to_string())))
            .class("auth-field")
    };
    let cancel = InputArea::new(DecoratedBox::new().child(Text::new("Отмена")).class("auth-btn"))
        .pointer()
        .on_click(move |_, _, _| respond(view, None));
    let body = Column::new()
        .gap(14.0)
        .child(
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(icon(mi::LOCK).class("auth-icon"))
                .child(header),
        )
        .child(field)
        .child(status)
        .child(Row::new().gap(8.0).main_axis_alignment(MainAxisAlignment::End).child(cancel));
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
