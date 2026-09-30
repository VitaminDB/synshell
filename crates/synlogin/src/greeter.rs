//! Экран входа (`synlogin greeter`) — «оболочка» композитора, запущенного
//! демоном: часы, пользователи карточками, пароль, создание нового
//! пользователя, перезагрузка и выключение. Своя экранная клавиатура
//! (syngui `OnScreenKeyboard`) — отдельного демона клавиатуры здесь нет.
//!
//! Вход: пароль проверяется PAM (`synshell_ui::lock::authenticate`), имя
//! пишется в `/run/synlogin/request`, композитор завершается — демон
//! запускает сеанс пользователя.

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use syngui::widgets::input::on_screen_keyboard::{on_screen_keyboard, KeyboardLayout, KeyboardState};
use syngui::GestureDetector;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceSpec};

use crate::users::{self, User};

#[derive(Clone, PartialEq)]
enum Stage {
    Users,
    Password(User),
    Create,
}

#[derive(Clone, Copy)]
struct St {
    stage: RwSignal<Stage>,
    users: RwSignal<Vec<User>>,
    busy: RwSignal<bool>,
    error: RwSignal<String>,
    /// Какое поле заполняет экранная клавиатура.
    field: RwSignal<usize>,
    /// Поля: пароль / логин, имя, пароль, повтор.
    values: RwSignal<Vec<String>>,
    admin: RwSignal<bool>,
    osk: RwSignal<bool>,
    /// Состояние экранной клавиатуры (раскладка переживает смену поля).
    kbd: KeyboardState,
    /// Растёт при вводе с экранной клавиатуры — поля перестраиваются с
    /// новым текстом; ввод с физической клавиатуры их не пересобирает.
    osk_rev: RwSignal<u64>,
    now: RwSignal<i64>,
}

fn phone() -> bool {
    std::env::var("SYNSHELL_FORM_FACTOR").as_deref() == Ok("phone")
}

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn")).init();
    let (cfg, _) = synshell_common::Config::load();
    let mut mss = cfg.appearance.mss_variables();
    mss.push_str(&cfg.appearance.theme_mss_variables());
    mss.push_str(include_str!("../styles/greeter.mss"));
    let r = syngui_layer::run(syngui_layer::RunOptions::default(), &mss, move || {
        let st = St {
            stage: use_signal(Stage::Users),
            users: use_signal(users::login_users()),
            busy: use_signal(false),
            error: use_signal(String::new()),
            field: use_signal(0),
            values: use_signal(vec![String::new(); 4]),
            admin: use_signal(true),
            osk: use_signal(phone()),
            kbd: {
                let k = KeyboardState::new("");
                // D-pad не нужен: физическая клавиатура пишет в поле сама.
                k.active.set(false);
                k
            },
            osk_rev: use_signal(0),
            now: use_signal(synshell_ui::clock::unix_now()),
        };
        syngui_layer::add_timer(std::time::Duration::from_secs(5), move || {
            st.now.set(synshell_ui::clock::unix_now());
            Some(std::time::Duration::from_secs(5))
        });
        // Один пользователь — сразу к паролю.
        let us = st.users.get_untracked();
        if us.len() == 1 {
            st.stage.set(Stage::Password(us[0].clone()));
        }
        let outs = syngui_layer::outputs().get_untracked();
        for o in outs {
            let spec = SurfaceSpec {
                namespace: "synlogin".into(),
                layer: Layer::Overlay,
                anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
                size: (0, 0),
                margin: [0; 4],
                exclusive_zone: -1,
                keyboard: KeyboardInteractivity::Exclusive,
                output: Some(o.name.clone()),
                auto_size: false,
                clear_color: [0.0, 0.0, 0.0, 1.0],
            };
            syngui_layer::create_surface(spec, move || Box::new(view(st)));
        }
    });
    if let Err(e) = r {
        log::error!("экран входа: {e:#}");
        std::process::exit(1);
    }
}

fn set_value(st: St, i: usize, v: String) {
    let mut vals = st.values.get_untracked();
    if i < vals.len() {
        vals[i] = v;
    }
    st.values.set(vals);
}

fn reset_values(st: St) {
    st.values.set(vec![String::new(); 4]);
    st.error.set(String::new());
    select_field(st, 0);
    st.osk_rev.set(st.osk_rev.get_untracked() + 1);
}

/// Экранная клавиатура переключается на поле `i`.
fn select_field(st: St, i: usize) {
    st.field.set(i);
    st.kbd.text.set(st.values.get_untracked().get(i).cloned().unwrap_or_default());
}

/// Войти: проверить пароль (пустой — вход без пароля), сообщить демону и
/// завершить композитор.
fn login(st: St, user: User) {
    let pass = st.values.get_untracked()[0].clone();
    st.busy.set(true);
    st.error.set(String::new());
    std::thread::spawn(move || {
        let ok = if pass.is_empty() && users::has_empty_password(&user.name) {
            Ok(())
        } else {
            synshell_ui::lock::authenticate(&user.name, &pass).map_err(|_| "Неверный пароль".to_string())
        };
        run_on_main_thread(move || {
            st.busy.set(false);
            match ok {
                Ok(()) => finish(&user.name),
                Err(e) => {
                    st.error.set(e);
                    set_value(st, 0, String::new());
                    select_field(st, 0);
                    st.osk_rev.set(st.osk_rev.get_untracked() + 1);
                }
            }
        });
    });
}

fn finish(request: &str) {
    let _ = std::fs::create_dir_all("/run/synlogin");
    if let Err(e) = std::fs::write(crate::daemon::REQUEST, request) {
        log::error!("не записать запрос: {e}");
    }
    if synshell_common::ipc::send_action(synshell_common::Action::Quit).is_err() {
        syngui_layer::quit();
    }
}

fn create(st: St) {
    let v = st.values.get_untracked();
    let (login_name, full, p1, p2) = (v[0].trim().to_string(), v[1].trim().to_string(), v[2].clone(), v[3].clone());
    if p1 != p2 {
        st.error.set("Пароли не совпадают".into());
        return;
    }
    let admin = st.admin.get_untracked();
    st.busy.set(true);
    std::thread::spawn(move || {
        let r = users::create(&login_name, &full, &p1, admin);
        run_on_main_thread(move || {
            st.busy.set(false);
            match r {
                Ok(()) => {
                    st.users.set(users::login_users());
                    let u = users::find(&login_name);
                    reset_values(st);
                    if let Some(u) = u {
                        st.stage.set(Stage::Password(u));
                    } else {
                        st.stage.set(Stage::Users);
                    }
                }
                Err(e) => st.error.set(e),
            }
        });
    });
}

fn avatar(u: &User, class: &str) -> Box<dyn Widget> {
    let path = std::path::Path::new(&u.home).join(".face");
    if path.is_file() {
        return Box::new(Image::new(path.to_string_lossy()).fit(ImageFit::Cover).class(format!("{class} avatar-img"))) as Box<dyn Widget>;
    }
    let letter: String = u.display().chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    Box::new(DecoratedBox::new().child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(letter).class("avatar-letter"))).class(class.to_string()))
}

fn view(st: St) -> impl Widget {
    let clock = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let now = st.now.get();
        vec![Box::new(
            Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(synshell_ui::clock::format(now, "%H:%M")).class("time"))
                .child(Text::new(synshell_ui::clock::format(now, "%A, %e %B")).class("date")),
        )]
    });
    let body = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let s = st.stage.get();
        let key = match &s {
            Stage::Users => 1u64,
            Stage::Password(_) => 2,
            Stage::Create => 3,
        };
        vec![Box::new(
            AnimatedSwitcher::new(key, move || -> Box<dyn Widget> {
                match st.stage.get_untracked() {
                    Stage::Users => Box::new(users_view(st)),
                    Stage::Password(u) => Box::new(password_view(st, u)),
                    Stage::Create => Box::new(create_view(st)),
                }
            })
            .directional(true)
            .slide(40.0, 0.0)
            .duration_ms(260)
            .animate_size(false),
        )]
    });
    let keyboard = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let show = st.osk.get()
            && match st.stage.get() {
                Stage::Users => false,
                Stage::Password(u) => !users::has_empty_password(&u.name),
                Stage::Create => true,
            };
        if !show {
            return vec![];
        }
        let kb = on_screen_keyboard(st.kbd, KeyboardLayout::text_en_ru("OK"))
            .gap(5.0)
            .stretch(44.0)
            .on_change(move |t| {
                set_value(st, st.field.get_untracked(), t.to_string());
                st.osk_rev.set(st.osk_rev.get_untracked() + 1);
            })
            .on_submit(move |_| submit(st))
            .build();
        // Во всю ширину экрана без полей корня.
        let w = (syngui::viewport::viewport_size().get().width - 12.0).clamp(280.0, 720.0);
        vec![Box::new(DecoratedBox::new().child(kb).class("osk").style("width", w))]
    });
    let power = Row::new()
        .gap(10.0)
        .child(round_btn("\u{E8AC}", || finish("!poweroff")))
        .child(round_btn("\u{E5D5}", || finish("!reboot")))
        .child(round_btn("\u{E312}", move || st.osk.set(!st.osk.get_untracked())));
    Stack::new()
        .fit(StackFit::Expand)
        .child(DecoratedBox::new().class("bg"))
        .child(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(clock)
                .child(body)
                .child(Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(keyboard).child(power))
                .class("root"),
        )
}

fn submit(st: St) {
    match st.stage.get_untracked() {
        Stage::Password(u) => login(st, u),
        Stage::Create => {
            let f = st.field.get_untracked();
            if f < 3 {
                select_field(st, f + 1);
                st.osk_rev.set(st.osk_rev.get_untracked() + 1);
            } else {
                create(st);
            }
        }
        Stage::Users => {}
    }
}

fn round_btn(glyph: &'static str, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    GestureDetector::new().on_click(f).child(DecoratedBox::new().child(Icon::new(glyph).class("round-icon")).class("round"))
}

fn users_view(st: St) -> impl Widget {
    let mut row = Row::new().gap(18.0);
    let list = st.users.get_untracked();
    let n = list.len();
    for u in list {
        let u2 = u.clone();
        row = row.child(GestureDetector::new().on_click(move || {
            reset_values(st);
            // Без пароля — сразу вход, без экрана пароля.
            if users::has_empty_password(&u2.name) {
                login(st, u2.clone());
            } else {
                st.stage.set(Stage::Password(u2.clone()));
            }
        }).child(
            DecoratedBox::new()
                .child(Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(avatar(&u, "avatar")).child(Text::new(u.display()).max_lines(1).class("user-name")))
                .class("user-card"),
        ));
    }
    let add = GestureDetector::new().on_click(move || {
        reset_values(st);
        st.stage.set(Stage::Create);
    }).child(DecoratedBox::new().child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new("\u{E7FE}").class("pill-icon")).child(Text::new("Новый пользователь").class("pill-text"))).class("pill"));
    Column::new()
        .gap(22.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new("Кто входит?").class("title"))
        .child(if n <= 3 { Box::new(row.class("users-row")) as Box<dyn Widget> } else { Box::new(ScrollView::new().horizontal().child(row.class("users-row"))) })
        .child(add)
}

/// Поле, заполняемое и с клавиатуры, и экранной клавиатурой.
fn field(st: St, i: usize, placeholder: &str, secret: bool) -> impl Widget {
    let v = st.values.get_untracked()[i].clone();
    let focus = st.field.get_untracked() == i;
    GestureDetector::new().on_press(move |_| select_field(st, i)).child(
        TextField::with_text(v)
            .placeholder(placeholder)
            .obscure(secret)
            .autofocus(focus)
            .on_change(move |t| {
                set_value(st, i, t.to_string());
                if st.field.get_untracked() == i {
                    st.kbd.text.set(t.to_string());
                }
            })
            .on_submit(move |_| submit(st))
            .class("field"),
    )
}

fn password_view(st: St, u: User) -> impl Widget {
    let err = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let e = st.error.get();
        let busy = st.busy.get();
        vec![Box::new(Text::new(if busy { "Проверка…".to_string() } else { e }).class("error"))]
    });
    // Экранная клавиатура пишет в значения — поле перестраивается следом.
    let pass = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.osk_rev.get();
        vec![Box::new(field(st, 0, "Пароль", true))]
    });
    let u2 = u.clone();
    // Пользователь без пароля: поля нет, «Войти» входит сразу.
    let no_password = users::has_empty_password(&u.name);
    let mut col = Column::new()
        .gap(14.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(avatar(&u, "avatar big"))
        .child(Text::new(u.display()).class("title"));
    if !no_password {
        col = col.child(DecoratedBox::new().child(pass).class("field-box"));
    }
    col.child(err)
        .child(
            Row::new()
                .gap(10.0)
                .child(GestureDetector::new().on_click(move || {
                    reset_values(st);
                    st.stage.set(Stage::Users);
                }).child(DecoratedBox::new().child(Text::new("Другой пользователь").class("pill-text")).class("pill")))
                .child(GestureDetector::new().on_click(move || login(st, u2.clone())).child(DecoratedBox::new().child(Text::new("Войти").class("pill-text")).class("pill pill-primary"))),
        )
}

fn create_view(st: St) -> impl Widget {
    let fields = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.osk_rev.get();
        vec![Box::new(
            Column::new()
                .gap(10.0)
                .child(field(st, 0, "Логин (латиницей)", false))
                .child(field(st, 1, "Полное имя", false))
                .child(field(st, 2, "Пароль (можно пустой)", true))
                .child(field(st, 3, "Повтор пароля", true)),
        )]
    });
    let err = Reactive::new(move || -> Vec<Box<dyn Widget>> { vec![Box::new(Text::new(st.error.get()).class("error"))] });
    let admin = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let on = st.admin.get();
        vec![Box::new(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Toggle::with_state(on).on_change(move |v| st.admin.set(v)))
                .child(Text::new("Администратор (sudo, установка программ)").class("hint")),
        )]
    });
    Column::new()
        .gap(14.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new("Новый пользователь").class("title"))
        .child(DecoratedBox::new().child(fields).class("field-box"))
        .child(admin)
        .child(err)
        .child(
            Row::new()
                .gap(10.0)
                .child(GestureDetector::new().on_click(move || {
                    reset_values(st);
                    st.stage.set(Stage::Users);
                }).child(DecoratedBox::new().child(Text::new("Отмена").class("pill-text")).class("pill")))
                .child(GestureDetector::new().on_click(move || create(st)).child(DecoratedBox::new().child(Text::new("Создать").class("pill-text")).class("pill pill-primary"))),
        )
}
