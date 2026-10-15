//! Окно: своя рамка с вкладками в заголовке (компьютер) или вкладки под рамкой
//! композитора и ряд клавиш внизу (телефон); терминал активной вкладки.

use syngui::input::DragData;
use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::widgets::visual::terminal::TerminalCommand;
use syngui::widgets::*;

use crate::state::{shortcut, Ask, St, Tab};

pub type W = Box<dyn Widget>;

pub fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

pub mod icons {
    pub const TERMINAL: &str = "\u{EB8E}";
    pub const ADD: &str = "\u{E145}";
    pub const CLOSE: &str = "\u{E5CD}";
    pub const SETTINGS: &str = "\u{E8B8}";
    /// fiber_manual_record: в вкладке работает программа
    pub const DOT: &str = "\u{E061}";
    pub const PASTE: &str = "\u{E14F}";
    pub const COPY: &str = "\u{E14D}";
    /// clear_all: очистить экран
    pub const CLEAR: &str = "\u{E0B8}";
    /// open_in_new: новое окно
    pub const WINDOW: &str = "\u{E89E}";
    pub const LEFT: &str = "\u{E314}";
    pub const RIGHT: &str = "\u{E315}";
    pub const UP: &str = "\u{E316}";
    pub const DOWN: &str = "\u{E313}";
    pub const TEXT: &str = "\u{E262}";
    pub const PALETTE: &str = "\u{E40A}";
    pub const TAB: &str = "\u{E8D8}";
    pub const HISTORY: &str = "\u{E889}";
    pub const KEYBOARD: &str = "\u{E312}";
    pub const CHECK: &str = "\u{E5CA}";
}

pub fn icon_button(icon: &'static str, tip: String, class: &'static str, f: impl Fn() + Send + Sync + 'static) -> W {
    boxed(
        GestureDetector::new()
            .on_click(f)
            .cursor(syngui::CursorIcon::Pointer)
            .child(Tooltip::new(DecoratedBox::new().class(class).child(Icon::new(icon).class("ib-icon")), tip)),
    )
}

/// Узкое окно: ширина меньше этой — телефонная раскладка.
const NARROW_BELOW: f32 = 600.0;

pub fn root(st: St) -> W {
    let narrow = syngui::viewport::viewport_below(NARROW_BELOW);
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let phone = narrow.get();
        if st.phone.get_untracked() != phone {
            st.phone.set(phone);
        }
        // Телефон: рамку (закрыть, свернуть) рисует композитор. Окно может
        // быть ещё не создано — переключаем из цикла событий.
        run_on_main_thread(move || syngui::signal::set_decorations(phone));
        vec![if phone { phone_root(st) } else { desktop_root(st) }]
    }))
}

fn keys(st: St, child: W) -> W {
    boxed(
        EventHook::new()
            .on_key_down(move |k, m| {
                if k == Key::Escape && st.settings_open.get_untracked() {
                    st.settings_open.set(false);
                    return KeyReply::Handled;
                }
                if shortcut(st, k, m) {
                    KeyReply::Handled
                } else {
                    KeyReply::Ignore
                }
            })
            .child(child),
    )
}

fn layers(st: St, main: W) -> W {
    boxed(
        GestureDetector::new()
            .on_back(move || {
                if st.settings_open.get_untracked() {
                    st.settings_open.set(false);
                    return true;
                }
                false
            })
            .child(
                Stack::new()
                    .fit(StackFit::Expand)
                    .child(main)
                    .child(crate::settings_ui::layer(st))
                    .child(ask_dialog(st)),
            ),
    )
}

fn desktop_root(st: St) -> W {
    let main = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("window")
        .child(titlebar(st))
        .child(terminal_view(st));
    // Своя рамка окна (CSD): скругление и обводка как у окон композитора,
    // тень рисует он сам. Окно прозрачное — углы за скруглением пустые.
    let frame = DecoratedBox::new().class("window-frame").clip(true).child(keys(st, layers(st, boxed(main))));
    boxed(WindowResizeRegion::new().over_content(true).inset(5.0).child(frame))
}

fn phone_root(st: St) -> W {
    let main = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("window phone")
        .child(phone_bar(st))
        .child(terminal_view(st))
        .child(extra_keys(st));
    keys(st, layers(st, boxed(main)))
}

// ─── вкладки ───────────────────────────────────────────────────────────────

fn tab_chip(st: St, tab: Tab, current: bool, width: f32) -> W {
    let id = tab.id;
    let t = tab.clone();
    let head = Reactive::new(move || -> Vec<W> {
        let glyph = if t.busy.get() { Icon::new(icons::DOT).class("tab-icon busy") } else { Icon::new(icons::TERMINAL).class("tab-icon") };
        vec![boxed(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(glyph)
                .child(Text::new(t.label()).max_lines(1).elide(Elide::End).class("tab-title")),
        )]
    });
    let close = GestureDetector::new()
        .on_click(move || st.close_tab(id))
        .cursor(syngui::CursorIcon::Pointer)
        .child(DecoratedBox::new().class("tab-close").child(Icon::new(icons::CLOSE).class("tab-close-icon")));
    let chip = DecoratedBox::new().class(if current { "tab current" } else { "tab" }).style("width", width);
    boxed(
        GestureDetector::new()
            .on_click(move || st.select(id))
            .on_middle_click(move |_| st.close_tab(id))
            .child(
                chip.child(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(DecoratedBox::new().class("grow tab-head").child(head))
                        .child(close),
                ),
            ),
    )
}

/// Ширина вкладки на телефоне (строка вкладок прокручивается).
const PHONE_TAB_WIDTH: f32 = 150.0;

/// Ширина вкладки: делят место заголовка поровну, от 72 до 220 px.
fn tab_width(n: usize, room: f32) -> f32 {
    (room / n.max(1) as f32).clamp(72.0, 220.0)
}

fn titlebar(st: St) -> W {
    let tabs = Reactive::new(move || -> Vec<W> {
        let tabs = st.tabs.get();
        let cur = st.cur.get();
        // кнопки справа, «+» и место, за которое окно таскается
        let room = viewport_size().get().width - 230.0;
        let w = tab_width(tabs.len(), room);
        let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::End);
        for t in tabs {
            let current = t.id == cur;
            row = row.child(tab_chip(st, t, current, w));
        }
        row = row.child(icon_button(icons::ADD, t!("Новая вкладка (Ctrl+Shift+T)"), "new-tab", move || st.new_tab_here()));
        vec![boxed(row)]
    });
    let controls = Reactive::new(move || -> Vec<W> {
        let w = st.window.get();
        vec![boxed(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    boxed(
        DecoratedBox::new().class("titlebar").child(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(DecoratedBox::new().class("tabs").child(tabs))
                .child(DecoratedBox::new().class("grow").child(WindowDragRegion::new().child(DecoratedBox::new().class("drag-space"))))
                .child(DecoratedBox::new().class("tb-tools").child(icon_button(
                    icons::SETTINGS,
                    t!("Настройки (Ctrl+,)"),
                    "tb-btn",
                    move || st.settings_open.set(!st.settings_open.get_untracked()),
                )))
                .child(DecoratedBox::new().class("window-controls").child(controls)),
        ),
    )
}

/// Телефон: вкладки в прокручиваемой строке, «+» и настройки.
fn phone_bar(st: St) -> W {
    let tabs = Reactive::new(move || -> Vec<W> {
        let tabs = st.tabs.get();
        let cur = st.cur.get();
        let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for t in tabs {
            let current = t.id == cur;
            row = row.child(tab_chip(st, t, current, PHONE_TAB_WIDTH));
        }
        vec![boxed(row)]
    });
    boxed(
        DecoratedBox::new().class("phone-bar").child(
            Row::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("grow").child(ScrollView::new().horizontal().child(tabs)))
                .child(icon_button(icons::ADD, t!("Новая вкладка"), "tb-btn", move || st.new_tab_here()))
                .child(icon_button(icons::SETTINGS, t!("Настройки"), "tb-btn", move || st.settings_open.set(true))),
        ),
    )
}

/// Клавиши, которых нет на экранной клавиатуре.
fn extra_keys(st: St) -> W {
    // Клавиши делят ширину поровну: растягивается обёртка, не сам жест.
    let slot = |face: W, f: Box<dyn Fn() + Send + Sync>| -> W {
        boxed(DecoratedBox::new().class("grow").child(
            GestureDetector::new().on_click(move || f()).child(DecoratedBox::new().class("xkey").child(face)),
        ))
    };
    let key = |label: &'static str, bytes: &'static [u8]| slot(boxed(Text::new(label).class("xkey-text")), Box::new(move || st.send(bytes)));
    let arrow = |icon: &'static str, bytes: &'static [u8]| slot(boxed(Icon::new(icon).class("xkey-icon")), Box::new(move || st.send(bytes)));
    let paste = slot(boxed(Icon::new(icons::PASTE).class("xkey-icon")), Box::new(move || st.command(TerminalCommand::Paste)));
    boxed(
        DecoratedBox::new().class("xkeys").child(
            Row::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(key("Esc", b"\x1b"))
                .child(key("Tab", b"\t"))
                .child(key("^C", b"\x03"))
                .child(key("^D", b"\x04"))
                .child(arrow(icons::LEFT, b"\x1b[D"))
                .child(arrow(icons::UP, b"\x1b[A"))
                .child(arrow(icons::DOWN, b"\x1b[B"))
                .child(arrow(icons::RIGHT, b"\x1b[C"))
                .child(paste),
        ),
    )
}

// ─── терминал ──────────────────────────────────────────────────────────────

/// POSIX-кавычки для пути, брошенного в терминал.
fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+,:@%".contains(c)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn terminal_view(st: St) -> W {
    let term = Reactive::new(move || -> Vec<W> {
        let Some(tab) = st.active() else { return vec![] };
        let s = st.settings.get();
        // схема «как в системе» зависит от оформления — пересобраться вместе с темой
        let _ = st.theme.get();
        let scheme = st.scheme();
        // фокус — терминалу каждой показанной вкладки
        tab.session.reset_autofocus();
        let mut term = Terminal::new()
            .attach(tab.session.clone())
            .autofocus(true)
            .font_size(s.font_size)
            .line_height(s.line_height)
            .palette(scheme.ansi_colors())
            .scrollback(s.scrollback)
            .command_signal(tab.cmd)
            .on_key(move |k, m| shortcut(st, k, m))
            .class("term");
        if !s.font_family.trim().is_empty() {
            term = term.font_family(s.font_family.trim());
        }
        vec![boxed(term)]
    });
    // Файл из проводника — его путь в строку ввода (без Enter).
    boxed(
        DecoratedBox::new().class("term-area grow").child(
            DropArea::new()
                .accept_types(vec![DragData::TYPE_FILE.to_string()])
                .on_drop(move |data| {
                    let mut q = shell_quote(&data.payload);
                    q.push(' ');
                    st.send(q.as_bytes());
                })
                .child(
                    Stack::new()
                        .fit(StackFit::Expand)
                        .child(
                            GestureDetector::new()
                                .on_secondary_click(move |at| st.open_menu(at))
                                .child(DecoratedBox::new().class("grow term-wrap").child(term)),
                        )
                        .child(context_menu(st)),
                ),
        ),
    )
}

/// Контекстное меню терминала: копировать и вставить, очистить, вкладки.
fn context_menu(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let sel = st.menu_sel.get();
        let items = vec![
            MenuItem::new("copy", t!("Копировать")).icon(icons::COPY).shortcut("Ctrl+Shift+C").disabled(!sel),
            MenuItem::new("paste", t!("Вставить")).icon(icons::PASTE).shortcut("Ctrl+Shift+V"),
            MenuItem::separator(),
            MenuItem::new("clear", t!("Очистить")).icon(icons::CLEAR),
            MenuItem::separator(),
            MenuItem::new("new-tab", t!("Новая вкладка")).icon(icons::ADD).shortcut("Ctrl+Shift+T"),
            MenuItem::new("new-window", t!("Новое окно")).icon(icons::WINDOW).shortcut("Ctrl+Shift+N"),
            MenuItem::new("settings", t!("Настройки")).icon(icons::SETTINGS).shortcut("Ctrl+,"),
        ];
        vec![boxed(
            PopupMenu::new()
                .items(items)
                .position(st.menu_pos)
                .is_open(st.menu_open)
                .min_width(240.0)
                .on_select(move |id| st.menu_selected(id)),
        )]
    }))
}

// ─── вопрос перед закрытием ────────────────────────────────────────────────

fn ask_dialog(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let Some(ask) = st.ask.get() else { return vec![] };
        let (title, body) = match ask {
            Ask::Tab(_) => (t!("Закрыть вкладку?"), t!("В ней работает программа — она будет завершена.")),
            Ask::Window => (t!("Закрыть окно?"), t!("В терминале работают программы — они будут завершены.")),
        };
        vec![boxed(
            Dialog::new(title)
                .icon(icons::TERMINAL)
                .body(body)
                .width(400.0)
                .is_open(st.ask_open)
                .action(DialogAction::new(t!("Отмена"), move || {
                    st.ask_open.set(false);
                    st.ask.set(None);
                }))
                .action(
                    DialogAction::new(t!("Закрыть"), move || {
                        st.ask_open.set(false);
                        st.ask.set(None);
                        match ask {
                            Ask::Tab(id) => st.remove_tab(id),
                            Ask::Window => syngui::signal::quit_app(),
                        }
                    })
                    .primary(),
                ),
        )]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote() {
        assert_eq!(shell_quote("/home/a/file.txt"), "/home/a/file.txt");
        assert_eq!(shell_quote("/tmp/a b"), "'/tmp/a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn widths() {
        assert_eq!(tab_width(1, 1000.0), 220.0);
        assert_eq!(tab_width(10, 1000.0), 100.0);
        assert_eq!(tab_width(50, 1000.0), 72.0);
    }
}
