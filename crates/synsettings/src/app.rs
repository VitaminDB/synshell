//! Корневой виджет окна: боковая навигация с поиском, страница, строка
//! состояния.

use syngui::prelude::*;
use syngui::widgets::{EventHook, KeyReply};
use syngui::input::Key;

use crate::pages::{self, PAGES};
use crate::state::{self, Ctx};
use crate::store;
use crate::sys;
use crate::ui::*;

fn sidebar(ctx: Ctx) -> W {
    let search = TextField::new()
        .placeholder(t!("Поиск настроек"))
        .prefix_icon(icons::SEARCH)
        .on_change(move |s| ctx.search.set(s.to_string()))
        .on_submit(move |s| {
            // Enter — открыть первую найденную страницу.
            if let Some(p) = PAGES.iter().find(|p| pages::matches(p, s)) {
                state::open_page(ctx, p.id);
            }
        });
    let nav = Reactive::new(move || -> Vec<W> {
        let q = ctx.search.get();
        let cur = ctx.page.get();
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("nav");
        let mut last_group = "";
        let mut any = false;
        for p in PAGES.iter().filter(|p| q.trim().is_empty() || pages::matches(p, &q)) {
            any = true;
            if p.group != last_group {
                last_group = p.group;
                col = col.child(Text::new(tl(p.group)).class("nav-group"));
            }
            let id = p.id;
            // Кнопка syngui центрирует подпись — пункт меню собран вручную.
            col = col.child(
                syngui::GestureDetector::new().on_click(move || state::open_page(ctx, id)).child(
                    DecoratedBox::new().class(if cur == id { "nav-item active" } else { "nav-item" }).child(
                        Row::new()
                            .gap(12.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(Icon::new(p.icon).class("nav-icon"))
                            .child(Text::new(tl(p.title)).class("nav-label")),
                    ),
                ),
            );
        }
        if !any {
            col = col.child(Text::new(t!("Ничего не найдено")).class("nav-empty"));
        }
        vec![boxed(col)]
    });
    boxed(
        Column::new()
            .gap(12.0)
            .class("sidebar")
            .child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class("brand")
                    .child(DecoratedBox::new().class("brand-logo").child(Icon::new(icons::SETTINGS).class("brand-icon")))
                    .child(Text::new(t!("Параметры")).class("brand-title")),
            )
            .child(search)
            .child(ScrollView::new().vertical().class("grow nav-scroll").child(nav)),
    )
}

fn error_banner(ctx: Ctx) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let Some(err) = ctx.error.get() else { return vec![] };
        vec![boxed(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("error-banner")
                .child(Icon::new(icons::WARNING).class("error-icon"))
                .child(
                    Column::new()
                        .gap(2.0)
                        .class("grow")
                        .child(Text::new(t!("Ошибка в config.toml — пока она не исправлена, композитор и оболочка работают со старыми настройками")).class("row-label"))
                        .child(Text::new(err).selectable(true).class("row-hint")),
                )
                .child(button(&t!("Открыть файл"), || sys::open_in_editor(&store::path()))),
        )]
    }))
}

fn footer(ctx: Ctx) -> W {
    boxed(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("footer")
            .child(Text::new(home_short(&store::path())).elide(Elide::Middle).class("row-hint grow"))
            .child(Reactive::new(move || -> Vec<W> {
                let t = ctx.toast.get();
                if t.is_empty() {
                    vec![]
                } else {
                    vec![boxed(Text::new(t).class("toast"))]
                }
            }))
            .child(
                Button::new(t!("Перечитать"))
                    .icon(icons::REFRESH)
                    .class("btn small")
                    .on_click(move || {
                        store::reload();
                        synshell_common::i18n::apply(&store::config());
                        state::notify_changed();
                        state::bump();
                        state::toast(t!("Файл перечитан"));
                    }),
            )
            .child(
                Button::new(t!("Открыть config.toml"))
                    .icon(icons::OPEN)
                    .class("btn small")
                    .on_click(|| {
                        let _ = store::save_now();
                        sys::open_in_editor(&store::path());
                    }),
            ),
    )
}

/// Узкое окно: ширина меньше этой — телефонная раскладка.
const NARROW_BELOW: f32 = 720.0;

pub fn root(ctx: Ctx) -> W {
    let narrow = syngui::viewport::viewport_below(NARROW_BELOW);
    let body = Reactive::new(move || -> Vec<W> {
        // Смена языка (страница «Язык», перечитанный конфиг) — перестроить всё окно.
        syngui::i18n::subscribe();
        let n = narrow.get();
        set_narrow(n);
        vec![if n { phone_root(ctx) } else { desktop_root(ctx) }]
    });
    crate::pages::capture_hook(boxed(body))
}

// ─── Телефон: список разделов → страница ────────────────────────────────────

fn go_back(ctx: Ctx) -> bool {
    // Сначала — открытый поверх страницы редактор кадра обоев.
    if crate::pages::close_wallpaper_editor() {
        return true;
    }
    // Подстраница (раздел «Оборудования») — обратно на страницу.
    if ctx.sub.get_untracked().is_some() {
        ctx.sub.set(None);
        return true;
    }
    if ctx.page_open.get_untracked() {
        ctx.page_open.set(false);
        true
    } else {
        false
    }
}

fn phone_list(ctx: Ctx) -> W {
    let search = TextField::new()
        .placeholder(t!("Поиск настроек"))
        .prefix_icon(icons::SEARCH)
        .on_change(move |s| ctx.search.set(s.to_string()))
        .on_submit(move |s| {
            if let Some(p) = PAGES.iter().find(|p| pages::matches(p, s)) {
                state::open_page(ctx, p.id);
                ctx.page_open.set(true);
            }
        })
        .class("phone-search");
    let list = Reactive::new(move || -> Vec<W> {
        let q = ctx.search.get();
        let mut out: Vec<W> = Vec::new();
        let mut group: Option<(&str, Column)> = None;
        let flush = |g: Option<(&str, Column)>, out: &mut Vec<W>| {
            if let Some((title, col)) = g {
                out.push(boxed(Column::new().gap(8.0).child(Text::new(tl(title)).class("group-title")).child(col)));
            }
        };
        for p in PAGES.iter().filter(|p| q.trim().is_empty() || pages::matches(p, &q)) {
            if group.as_ref().map(|g| g.0) != Some(p.group) {
                flush(group.take(), &mut out);
                group = Some((p.group, Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("group-card")));
            }
            let id = p.id;
            let item = syngui::GestureDetector::new()
                .on_click(move || {
                    state::open_page(ctx, id);
                    ctx.page_open.set(true);
                })
                .child(
                    DecoratedBox::new().class("phone-nav-item").child(
                        Row::new()
                            .gap(14.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(DecoratedBox::new().class("phone-nav-badge").child(Icon::new(p.icon).class("phone-nav-icon")))
                            .child(Text::new(tl(p.title)).class("phone-nav-label grow"))
                            .child(Icon::new(icons::CHEVRON_RIGHT).class("phone-nav-chevron")),
                    ),
                );
            if let Some((_, col)) = group.take() {
                group = Some((p.group, col.child(item)));
            }
        }
        flush(group.take(), &mut out);
        if out.is_empty() {
            out.push(boxed(Text::new(t!("Ничего не найдено")).class("nav-empty")));
        }
        let mut col = Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for w in out {
            col = col.child(w);
        }
        vec![boxed(col)]
    });
    boxed(
        Column::new()
            .gap(14.0)
            .class("phone-list")
            .child(Text::new(t!("Параметры")).class("phone-title"))
            .child(search)
            .child(error_banner(ctx))
            .child(ScrollView::new().vertical().class("grow").child(list)),
    )
}

fn phone_page(ctx: Ctx) -> W {
    let content = Reactive::new(move || -> Vec<W> {
        ctx.rev.get();
        let id = ctx.page.get();
        if ctx.capture.get_untracked().is_some() {
            ctx.capture.set(None);
        }
        vec![(pages::find(&id).build)()]
    });
    let title = Reactive::new(move || -> Vec<W> {
        let t = match ctx.sub.get() {
            Some((_, t)) => t,
            None => tl(pages::find(&ctx.page.get()).title),
        };
        vec![boxed(Text::new(t).max_lines(1).class("phone-bar-title grow"))]
    });
    let bar = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("phone-bar")
        .child(syngui::GestureDetector::new().on_click(move || {
            go_back(ctx);
        }).child(DecoratedBox::new().class("phone-back").child(Icon::new(icons::BACK).class("phone-back-icon"))))
        .child(title);
    boxed(
        Column::new()
            .gap(0.0)
            .class("grow phone-page")
            .child(bar)
            .child(error_banner(ctx))
            .child(DecoratedBox::new().class("grow page-host").child(content)),
    )
}

fn phone_root(ctx: Ctx) -> W {
    let stack = Reactive::new(move || -> Vec<W> {
        let open = ctx.page_open.get();
        vec![boxed(
            AnimatedSwitcher::new(if open { 2u64 } else { 1 }, move || if open { phone_page(ctx) } else { phone_list(ctx) })
                .directional(true)
                .slide(48.0, 0.0)
                .duration_ms(260)
                .exit_duration_ms(180)
                .animate_size(false)
                .class("grow"),
        )]
    });
    // «Назад»: жест телефона (XF86Back), Escape, кнопка Android.
    boxed(
        EventHook::new()
            .on_key_down(move |k, _| {
                if matches!(k, Key::Escape) && ctx.capture.get_untracked().is_none() && go_back(ctx) {
                    KeyReply::Handled
                } else {
                    KeyReply::Ignore
                }
            })
            .child(syngui::GestureDetector::new().on_back(move || go_back(ctx)).child(Column::new().class("root phone-root").child(stack))),
    )
}

// ─── Рабочий стол: боковая панель и страница ─────────────────────────────────

fn desktop_root(ctx: Ctx) -> W {
    let content = Reactive::new(move || -> Vec<W> {
        ctx.rev.get();
        let id = ctx.page.get();
        // Ушли со страницы — перестать слушать сочетание.
        if ctx.capture.get_untracked().is_some() {
            ctx.capture.set(None);
        }
        vec![(pages::find(&id).build)()]
    });
    let main = Column::new()
        .gap(0.0)
        .class("grow main")
        .child(error_banner(ctx))
        .child(DecoratedBox::new().class("grow page-host").child(content))
        .child(footer(ctx));
    boxed(Row::new().gap(0.0).class("root").child(sidebar(ctx)).child(main))
}
