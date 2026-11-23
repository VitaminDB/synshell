//! Пульт Курьера — мини-приложение Telegram на syngui (WebAssembly).
//!
//! Открывается кнопкой меню бота «Пульт». Страницу и API (`/api/*`) раздаёт служба
//! `syncourier` (модуль `panel.rs`) через quick tunnel Cloudflare; каждый запрос несёт
//! `initData` Telegram, служба проверяет подпись и пускает только владельца бота.
//!
//! Экран: состояние Claude «courier» и каналов, быстрые действия (новый разговор, прервать,
//! снимок экрана), живые сеансы Claude (тап — писать этому сеансу), последние сообщения и
//! поле ввода. Ответы приходят в чат бота, как обычно.

mod api;

use serde::Deserialize;
use syngui::prelude::*;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

type W = Box<dyn Widget>;

fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

mod ic {
    pub const ADD: &str = "\u{E145}";
    pub const STOP: &str = "\u{E047}";
    pub const DESKTOP: &str = "\u{E30C}";
    pub const SEND: &str = "\u{E163}";
    pub const CLOSE: &str = "\u{E5CD}";
    pub const CHAT: &str = "\u{E0B7}";
    pub const FOLDER: &str = "\u{E2C7}";
    pub const REFRESH: &str = "\u{E5D5}";
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Claude {
    pub name: String,
    pub running: bool,
    pub busy: bool,
    pub cwd: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Channel {
    pub id: String,
    pub title: String,
    pub online: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Recent {
    pub dir: String,
    pub text: String,
    pub at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Agent {
    pub name: String,
    pub status: String,
    pub cwd: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct State {
    pub claude: Claude,
    pub channels: Vec<Channel>,
    pub recent: Vec<Recent>,
    pub agents: Vec<Agent>,
}

#[derive(Clone, Copy)]
pub struct St {
    pub state: RwSignal<Option<State>>,
    pub error: RwSignal<Option<String>>,
    /// Кому писать: `None` — Claude «courier», иначе имя живого сеанса.
    pub target: RwSignal<Option<String>>,
    pub draft: RwSignal<String>,
    /// Смена значения пересоздаёт поле ввода (очистка после отправки).
    pub field_gen: RwSignal<u64>,
    pub toast: RwSignal<String>,
    /// Идёт действие (кнопка показывает ожидание).
    pub pending: RwSignal<Option<&'static str>>,
}

impl St {
    fn new() -> Self {
        Self {
            state: use_signal(None),
            error: use_signal(None),
            target: use_signal(None),
            draft: use_signal(String::new()),
            field_gen: use_signal(0),
            toast: use_signal(String::new()),
            pending: use_signal(None),
        }
    }

    pub fn refresh(self) {
        api::get_state(move |r| match r {
            Ok(s) => {
                self.error.set(None);
                if self.state.get_untracked().as_ref() != Some(&s) {
                    self.state.set(Some(s));
                }
            }
            Err(e) => self.error.set(Some(e)),
        });
    }

    fn act(self, key: &'static str, path: &'static str, body: serde_json::Value, done: String) {
        if self.pending.get_untracked().is_some() {
            return;
        }
        self.pending.set(Some(key));
        api::haptic_tap();
        api::post(path, body, move |r| {
            self.pending.set(None);
            match r {
                Ok(()) => {
                    api::haptic_ok();
                    self.flash(&done);
                }
                Err(e) => {
                    api::haptic_err();
                    self.flash(&format!("Ошибка: {e}"));
                }
            }
            self.refresh();
        });
    }

    fn flash(self, text: &str) {
        self.toast.set(text.to_string());
        let t = text.to_string();
        api::after_ms(2500, move || {
            if self.toast.get_untracked() == t {
                self.toast.set(String::new());
            }
        });
    }

    fn send(self) {
        let text = self.draft.get_untracked().trim().to_string();
        if text.is_empty() {
            return;
        }
        let to = self.target.get_untracked().unwrap_or_default();
        self.draft.set(String::new());
        self.field_gen.update(|g| *g += 1);
        let who = if to.is_empty() { "Claude".to_string() } else { to.clone() };
        self.act("send", "send", serde_json::json!({ "text": text, "to": to }), format!("Отправлено: {who}"));
    }
}

fn status_word(c: &Claude) -> (&'static str, &'static str) {
    if c.busy {
        ("думает", "dot busy")
    } else if c.running {
        ("ждёт", "dot ok")
    } else {
        ("не запущен", "dot off")
    }
}

fn header(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let s = st.state.get();
        let err = st.error.get();
        let mut col = Column::new().gap(6.0);
        let mut top = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center);
        top = top.child(DecoratedBox::new().class("brand").child(Icon::new(ic::CHAT).class("brand-ic")));
        let mut titles = Column::new().gap(2.0).class("grow");
        titles = titles.child(Text::new("Курьер").class("title"));
        match (&s, &err) {
            (Some(s), _) => {
                let (word, _) = status_word(&s.claude);
                titles = titles.child(Text::new(format!("Claude «{}» {word} · {}", s.claude.name, s.claude.cwd)).max_lines(1).elide(Elide::End).class("muted small"));
            }
            (None, Some(e)) => titles = titles.child(Text::new(e.clone()).max_lines(2).class("err small")),
            (None, None) => titles = titles.child(Text::new("Связываюсь с компьютером…").class("muted small")),
        }
        top = top.child(titles);
        if let Some(s) = &s {
            top = top.child(DecoratedBox::new().class(status_word(&s.claude).1));
        }
        top = top.child(Button::new("").icon(ic::REFRESH).class("btn round flat").on_click(move || st.refresh()));
        col = col.child(top);
        if let Some(s) = &s {
            let mut chips = Row::new().gap(6.0);
            for c in &s.channels {
                chips = chips.child(DecoratedBox::new().class(if c.online { "chip on" } else { "chip" }).child(Text::new(c.title.clone()).class("chip-t")));
            }
            col = col.child(chips);
        }
        vec![boxed(DecoratedBox::new().class("card head").child(col))]
    }))
}

fn on_click_proxy(st: St, key: &'static str) {
    match key {
        "new" => st.act("new", "new", serde_json::json!({}), "Новый разговор начат".into()),
        "stop" => st.act("stop", "stop", serde_json::json!({}), "Остановлено".into()),
        "shot" => st.act("shot", "shot", serde_json::json!({}), "Снимок отправлен в чат".into()),
        _ => {}
    }
}

fn actions(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let busy = st.state.get().map(|s| s.claude.busy).unwrap_or(false);
        let waiting = st.pending.get();
        let t = |key: &'static str, glyph: &'static str, label: &'static str, class: &'static str| -> W {
            let lbl = if waiting == Some(key) { "Подождите…" } else { label };
            boxed(Button::new(lbl).icon(glyph).class(&format!("btn tile {class}")).on_click(move || on_click_proxy(st, key)))
        };
        vec![boxed(
            Row::new()
                .gap(8.0)
                .child(DecoratedBox::new().class("grow tile-wrap").child(t("new", ic::ADD, "Новый разговор", "")))
                .child(DecoratedBox::new().class("grow tile-wrap").child(t("stop", ic::STOP, "Прервать", if busy { "danger" } else { "" })))
                .child(DecoratedBox::new().class("grow tile-wrap").child(t("shot", ic::DESKTOP, "Снимок экрана", ""))),
        )]
    }))
}

fn sessions(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let Some(s) = st.state.get() else { return vec![] };
        let target = st.target.get();
        let mut col = Column::new().gap(6.0);
        col = col.child(Text::new("Сеансы Claude").class("section"));
        // Первая строка — свой разговор Курьера.
        let mine = target.is_none();
        let (word, dot) = status_word(&s.claude);
        col = col.child(session_row(st, None, &format!("{} (Курьер)", s.claude.name), word, dot, &s.claude.cwd, mine));
        if s.agents.is_empty() {
            col = col.child(Text::new("Других живых сеансов нет").class("muted small pad"));
        }
        for a in &s.agents {
            let (word, dot) = match a.status.as_str() {
                "busy" => ("занят", "dot busy"),
                "idle" => ("свободен", "dot ok"),
                _ => ("—", "dot off"),
            };
            let on = target.as_deref() == Some(a.name.as_str());
            col = col.child(session_row(st, Some(a.name.clone()), &a.name, word, dot, &a.cwd, on));
        }
        vec![boxed(DecoratedBox::new().class("card").child(col))]
    }))
}

fn session_row(st: St, name: Option<String>, title: &str, word: &str, dot: &str, cwd: &str, on: bool) -> W {
    let row = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class(dot))
        .child(
            Column::new()
                .gap(1.0)
                .class("grow")
                .child(Text::new(title.to_string()).max_lines(1).elide(Elide::End).class("s-name"))
                .child(Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(ic::FOLDER).class("ic-sm")).child(Text::new(cwd.to_string()).max_lines(1).elide(Elide::End).class("muted small"))),
        )
        .child(Text::new(word.to_string()).class("muted small"));
    boxed(
        syngui::GestureDetector::new()
            .on_click(move || {
                api::haptic_select();
                st.target.set(name.clone());
            })
            .child(DecoratedBox::new().class(if on { "s-row on" } else { "s-row" }).child(row)),
    )
}

fn recent(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let Some(s) = st.state.get() else { return vec![] };
        if s.recent.is_empty() {
            return vec![];
        }
        let mut col = Column::new().gap(6.0);
        col = col.child(Text::new("Последние сообщения").class("section"));
        for r in s.recent.iter().rev().take(12) {
            let text: String = r.text.chars().take(280).collect();
            let class = if r.dir == "in" { "msg in" } else { "msg out" };
            col = col.child(DecoratedBox::new().class(class).child(Text::new(text).max_lines(6).elide(Elide::End).class("msg-t")));
        }
        vec![boxed(DecoratedBox::new().class("card").child(col))]
    }))
}

fn composer(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let target = st.target.get();
        let _ = st.field_gen.get();
        let mut col = Column::new().gap(6.0);
        if let Some(t) = &target {
            col = col.child(
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(format!("Кому: {t}")).class("to grow"))
                    .child(Button::new("").icon(ic::CLOSE).class("btn round flat small-btn").on_click(move || st.target.set(None))),
            );
        }
        let placeholder = match &target {
            Some(t) => format!("Сообщение сеансу {t}"),
            None => "Сообщение Claude".to_string(),
        };
        let field = TextField::new()
            .text(st.draft.get_untracked())
            .placeholder(placeholder)
            .on_change(move |t| st.draft.set(t.to_string()))
            .on_submit(move |_| st.send())
            .class("field grow");
        col = col.child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("grow").child(field))
                .child(Button::new("").icon(ic::SEND).class("btn round primary send").on_click(move || st.send())),
        );
        vec![boxed(DecoratedBox::new().class("composer").child(col))]
    }))
}

fn toast(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let t = st.toast.get();
        if t.is_empty() {
            return vec![];
        }
        vec![boxed(DecoratedBox::new().class("toast").child(Text::new(t).class("toast-t")))]
    }))
}

fn root(st: St) -> W {
    let body = Column::new().gap(10.0).class("page").child(header(st)).child(actions(st)).child(sessions(st)).child(recent(st));
    boxed(
        DecoratedBox::new().class("root").child(
            Column::new()
                .child(ScrollView::new().vertical().child(body).class("grow"))
                .child(toast(st))
                .child(composer(st)),
        ),
    )
}

/// Тема: цвета Telegram (если открыто как мини-приложение), иначе тёмная по умолчанию.
fn theme_mss() -> String {
    let c = api::theme();
    format!(
        ":root {{ --bg: {}; --card: {}; --fg: {}; --muted: {}; --accent: {}; --accent-fg: {}; --link: {}; --danger: {}; }}\n{}",
        c.bg,
        c.card,
        c.fg,
        c.muted,
        c.accent,
        c.accent_fg,
        c.link,
        c.danger,
        include_str!("../styles/panel.mss")
    )
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub fn run_app() {
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();
    api::telegram_ready();
    let mss = theme_mss();
    let theme = use_signal(mss.clone());
    App::new()
        .title("Пульт Курьера")
        .app_id("syn-courier-panel")
        .with_font_url("fonts/NotoSans-Regular.ttf")
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .with_dynamic_theme(theme)
        .run(move |_ctx| {
            let st = St::new();
            st.refresh();
            api::every_ms(3000, move || st.refresh());
            api::on_theme_changed(move || theme.set(theme_mss()));
            root(st)
        });
}
