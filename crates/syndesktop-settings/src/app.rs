//! Корневой виджет окна: боковая навигация с поиском, страница, строка
//! состояния.

use syngui::prelude::*;

use crate::pages::{self, PAGES};
use crate::state::{self, Ctx};
use crate::store;
use crate::sys;
use crate::ui::*;

fn sidebar(ctx: Ctx) -> W {
    let search = TextField::new()
        .placeholder("Поиск настроек")
        .prefix_icon(icons::SEARCH)
        .on_change(move |s| ctx.search.set(s.to_string()))
        .on_submit(move |s| {
            // Enter — открыть первую найденную страницу.
            if let Some(p) = PAGES.iter().find(|p| pages::matches(p, s)) {
                ctx.page.set(p.id.to_string());
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
                col = col.child(Text::new(p.group).class("nav-group"));
            }
            let id = p.id;
            // Кнопка syngui центрирует подпись — пункт меню собран вручную.
            col = col.child(
                syngui::GestureDetector::new().on_click(move || ctx.page.set(id.to_string())).child(
                    DecoratedBox::new().class(if cur == id { "nav-item active" } else { "nav-item" }).child(
                        Row::new()
                            .gap(12.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(Icon::new(p.icon).class("nav-icon"))
                            .child(Text::new(p.title).class("nav-label")),
                    ),
                ),
            );
        }
        if !any {
            col = col.child(Text::new("Ничего не найдено").class("nav-empty"));
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
                    .child(Text::new("Параметры").class("brand-title")),
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
                        .child(Text::new("Ошибка в config.toml — пока она не исправлена, композитор и оболочка работают со старыми настройками").class("row-label"))
                        .child(Text::new(err).selectable(true).class("row-hint")),
                )
                .child(button("Открыть файл", || sys::open_in_editor(&store::path()))),
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
                Button::new("Перечитать")
                    .icon(icons::REFRESH)
                    .class("btn small")
                    .on_click(move || {
                        store::reload();
                        state::notify_changed();
                        state::bump();
                        state::toast("Файл перечитан");
                    }),
            )
            .child(
                Button::new("Открыть config.toml")
                    .icon(icons::OPEN)
                    .class("btn small")
                    .on_click(|| {
                        let _ = store::save_now();
                        sys::open_in_editor(&store::path());
                    }),
            ),
    )
}

pub fn root(ctx: Ctx) -> W {
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
    crate::pages::capture_hook(boxed(Row::new().gap(0.0).class("root").child(sidebar(ctx)).child(main)))
}
