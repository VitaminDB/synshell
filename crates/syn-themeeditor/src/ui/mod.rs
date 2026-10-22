//! Окно редактора тем: свой заголовок (CSD), слева темы, в центре живой
//! предпросмотр, справа инспектор. В узком окне (телефон) — вкладки.

mod inspector;
mod preview;
mod sidebar;
mod widgets;

use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::widgets::buttons::Segment;
use syngui::widgets::{EventHook, KeyReply};
use syngui::GestureDetector;
use synshell_common::t;

use crate::draft::Draft;
use crate::icons;
use crate::state::{with_draft, Confirm, St};

pub type W = Box<dyn Widget>;

pub fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

pub fn icon(g: &str) -> syngui::widget::StyledWidget<Icon> {
    Icon::new(g).class("te-ic")
}

pub fn tip(w: impl Widget + 'static, text: impl Into<String>) -> W {
    boxed(Tooltip::new(w, text).delay_ms(450))
}

pub fn icon_button(glyph: &str, tooltip: impl Into<String>, class: &str, on_click: impl FnMut() + Send + 'static) -> W {
    tip(Button::new("").icon(glyph).class(&format!("te-btn te-round {class}")).on_click(on_click), tooltip)
}

/// Открыть импортированную тему как новый черновик.
pub fn import_draft(st: St, mut d: Draft) {
    d.id = crate::draft::free_id(&d.id);
    st.edit("import", move |x| *x = d);
    st.save();
}

/// Узкая раскладка (телефон): вкладки вместо колонок.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Preview,
    Edit,
    Themes,
}

fn brand() -> W {
    boxed(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("te-brand").child(icon(icons::PALETTE).class("te-ic te-brand-ic"))).child(Text::new(t!("Редактор тем")).class("te-brand-title")))
}

/// Название темы и отметка несохранённых правок.
fn title(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let _ = st.rev.get();
        let dirty = st.dirty.get();
        let (name, builtin) = with_draft(|d| (d.name(), d.is_builtin()));
        let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).class("te-title-row");
        row = row.child(Text::new(name).max_lines(1).elide(Elide::End).class("te-title"));
        if builtin {
            row = row.child(DecoratedBox::new().class("te-chip").child(Text::new(t!("встроенная")).class("te-chip-text")));
        }
        if dirty {
            row = row.child(tip(DecoratedBox::new().class("te-dirty-dot"), t!("Есть несохранённые правки")));
        }
        vec![boxed(row)]
    }))
}

fn history_buttons(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let _ = st.rev.get();
        let (u, r) = (st.can_undo(), st.can_redo());
        vec![boxed(
            Row::new()
                .gap(2.0)
                .child(tip(Button::new("").icon(icons::UNDO).class(if u { "te-btn te-round" } else { "te-btn te-round te-off" }).on_click(move || st.undo()), t!("Отменить (Ctrl+Z)")))
                .child(tip(Button::new("").icon(icons::REDO).class(if r { "te-btn te-round" } else { "te-btn te-round te-off" }).on_click(move || st.redo()), t!("Вернуть (Ctrl+Shift+Z)"))),
        )]
    }))
}

/// Тёмный/светлый вариант темы.
pub fn variant_switch(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let _ = st.rev.get();
        let v = st.variant.get();
        let (has_dark, has_light) = with_draft(|d| (d.has_variant("dark"), d.has_variant("light")));
        let seg = |label: String, glyph: &str, has: bool| Segment::with_icon(if has { label } else { format!("{label} +") }, glyph);
        vec![tip(
            SegmentedButton::new(vec![seg(t!("Тёмная"), icons::DARK, has_dark), seg(t!("Светлая"), icons::LIGHT, has_light)])
                .selected(if v == "dark" { 0 } else { 1 })
                .on_change(move |i| {
                    let to = if i == 0 { "dark" } else { "light" };
                    let from = if to == "dark" { "light" } else { "dark" };
                    if !with_draft(|d| d.has_variant(to)) && with_draft(|d| d.has_variant(from)) {
                        st.edit("derive", |d| d.derive_variant(from, to));
                        st.toast.set(t!("Вариант создан из другого: светлота цветов перевёрнута — поправьте по вкусу"));
                    }
                    st.set_variant(to);
                })
                .class("te-seg"),
            t!("Какой вариант темы править; «+» — создать из другого"),
        )]
    }))
}

fn tryon_button(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let on = st.tryon.get();
        vec![tip(
            Button::new(if on { t!("Примерка") } else { t!("Примерить") })
                .icon(icons::VISIBILITY)
                .class(if on { "te-btn te-btn-on" } else { "te-btn" })
                .on_click(move || st.set_tryon(!st.tryon.get_untracked())),
            if on { t!("Вся система показывает черновик. Нажмите, чтобы вернуть свою тему") } else { t!("Показать черновик на всей системе: панель, окна, программы") },
        )]
    }))
}

fn save_buttons(st: St) -> W {
    boxed(
        Row::new()
            .gap(6.0)
            .child(tip(Button::new(t!("Сохранить")).icon(icons::SAVE).class("te-btn").on_click(move || st.save()), t!("Сохранить в ~/.config/synshell/themes (Ctrl+S)")))
            .child(tip(Button::new(t!("Применить")).icon(icons::CHECK).class("te-btn te-primary").on_click(move || st.apply()), t!("Сохранить и сделать темой системы"))),
    )
}

fn titlebar(st: St, wide: bool) -> W {
    let controls = Reactive::new(move || -> Vec<W> {
        let w = st.window.get();
        vec![boxed(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    let mut row = Row::new().cross_axis_alignment(CrossAxisAlignment::Center).gap(8.0);
    if wide {
        row = row.child(DecoratedBox::new().class("te-tb-left").child(brand()));
    }
    row = row
        .child(DecoratedBox::new().class("grow te-drag-wrap").child(WindowDragRegion::new().child(DecoratedBox::new().class("te-drag").child(title(st)))))
        .child(history_buttons(st));
    if wide {
        row = row.child(variant_switch(st)).child(tryon_button(st)).child(save_buttons(st));
    } else {
        row = row.child(icon_button(icons::SAVE, t!("Сохранить"), "", move || st.save()));
    }
    row = row.child(DecoratedBox::new().class("te-window-controls").child(controls));
    boxed(DecoratedBox::new().class("te-titlebar").child(row))
}

fn toast_layer(st: St) -> W {
    let toast = Reactive::new(move || -> Vec<W> {
        let text = st.toast.get();
        if text.is_empty() {
            return vec![];
        }
        // Спрятать через 3.5 с, если за это время не пришло новое.
        let shown = text.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(3500));
            run_on_main_thread(move || {
                if st.toast.get_untracked() == shown {
                    st.toast.set(String::new());
                }
            });
        });
        vec![boxed(DecoratedBox::new().class("te-toast").child(Text::new(text).max_lines(3).class("te-toast-text")))]
    });
    boxed(Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center).class("te-toast-wrap").child(toast))
}

fn confirm_layer(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let Some(c) = st.confirm.get() else { return vec![] };
        let (title, body, ok, danger) = match c {
            Confirm::Delete => {
                let builtin_twin = with_draft(|d| synshell_common::theme::builtin_ids().any(|b| b == d.id));
                (
                    if builtin_twin { t!("Вернуть встроенную тему?") } else { t!("Удалить тему?") },
                    if builtin_twin {
                        t!("Ваша версия будет удалена с диска, останется встроенная.")
                    } else {
                        t!("Каталог темы будет удалён с диска. Это нельзя отменить — можно сначала экспортировать.")
                    },
                    if builtin_twin { t!("Вернуть") } else { t!("Удалить") },
                    true,
                )
            }
            Confirm::DiscardAndOpen => (t!("Есть несохранённые правки"), t!("Открыть другую тему без сохранения? Правки пропадут."), t!("Не сохранять"), true),
        };
        let card = Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("te-dialog")
            .child(Text::new(title).class("te-dialog-title"))
            .child(Text::new(body).max_lines(6).class("te-dialog-body"))
            .child(
                Row::new()
                    .gap(8.0)
                    .main_axis_alignment(MainAxisAlignment::End)
                    .child(Button::new(t!("Отмена")).class("te-btn").on_click(move || st.confirm.set(None)))
                    .child(Button::new(ok).class(if danger { "te-btn te-danger" } else { "te-btn te-primary" }).on_click(move || {
                        st.confirm.set(None);
                        match c {
                            Confirm::Delete => st.delete(),
                            Confirm::DiscardAndOpen => {
                                st.dirty.set(false);
                                st.open_pending();
                            }
                        }
                    })),
            );
        vec![boxed(
            GestureDetector::new()
                .on_click(move || st.confirm.set(None))
                .child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).class("te-scrim").child(GestureDetector::new().on_click(|| {}).child(card))),
        )]
    }))
}

/// Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y / Ctrl+S, Esc — выйти из пипетки и диалога.
fn keys(st: St, inner: impl Widget + 'static) -> W {
    boxed(EventHook::new().on_key_down(move |k, m| {
        if k == Key::Escape {
            if st.confirm.get_untracked().is_some() {
                st.confirm.set(None);
                return KeyReply::Handled;
            }
            if st.pipette.get_untracked() {
                st.pipette.set(false);
                return KeyReply::Handled;
            }
            return KeyReply::Ignore;
        }
        if !m.ctrl {
            return KeyReply::Ignore;
        }
        match k {
            Key::Z if m.shift => st.redo(),
            Key::Z => st.undo(),
            Key::Y => st.redo(),
            Key::S => st.save(),
            _ => return KeyReply::Ignore,
        }
        KeyReply::Handled
    }).child(inner))
}

fn wide_signal() -> RwSignal<bool> {
    let narrow = viewport_below(980.0);
    let w = use_signal(!narrow.get_untracked());
    create_effect(move || {
        let v = !narrow.get();
        if w.get_untracked() != v {
            w.set(v);
        }
    });
    w
}

pub fn root(st: St) -> W {
    let wide = wide_signal();
    let tab = use_signal(Tab::Preview);
    boxed(Reactive::new(move || -> Vec<W> {
        let w = wide.get();
        let body: W = if w {
            boxed(
                Row::new()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .class("grow")
                    .child(sidebar::sidebar(st))
                    .child(DecoratedBox::new().class("grow te-center").child(preview::preview(st)))
                    .child(inspector::inspector(st)),
            )
        } else {
            narrow_body(st, tab)
        };
        let main = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class(if w { "te-window" } else { "te-window te-phone" }).child(titlebar(st, w)).child(body);
        let stack = Stack::new().fit(StackFit::Expand).child(main).child(toast_layer(st)).child(confirm_layer(st));
        let frame = DecoratedBox::new().class(if w { "te-frame" } else { "te-frame te-flat" }).clip(true).child(keys(st, stack));
        vec![boxed(WindowResizeRegion::new().over_content(true).inset(5.0).enabled(w).child(frame))]
    }))
}

/// Телефон: вкладки «Вид», «Правка», «Темы».
fn narrow_body(st: St, tab: RwSignal<Tab>) -> W {
    let tabs = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let t = tab.get();
        vec![boxed(
            SegmentedButton::new(vec![Segment::with_icon(t!("Вид"), icons::VISIBILITY), Segment::with_icon(t!("Правка"), icons::BRUSH), Segment::with_icon(t!("Темы"), icons::PALETTE)])
                .selected(match t {
                    Tab::Preview => 0,
                    Tab::Edit => 1,
                    Tab::Themes => 2,
                })
                .on_change(move |i| tab.set([Tab::Preview, Tab::Edit, Tab::Themes][i.min(2)]))
                .class("te-seg te-tabs"),
        )]
    });
    let content = Reactive::new(move || -> Vec<W> {
        match tab.get() {
            Tab::Preview => vec![preview::preview(st)],
            Tab::Edit => vec![boxed(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow").child(DecoratedBox::new().class("te-phone-tools").child(Row::new().gap(8.0).child(variant_switch(st)).child(tryon_button(st)))).child(inspector::inspector(st)))],
            Tab::Themes => vec![sidebar::sidebar(st)],
        }
    });
    boxed(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow").child(DecoratedBox::new().class("te-tabs-wrap").child(tabs)).child(DecoratedBox::new().class("grow").child(content)))
}
