//! Части главного экрана: плитки игр, афиша выбранной игры, вкладки,
//! строка состояния, подсказки кнопок, пустые вкладки.

use std::sync::atomic::Ordering;

use synshell_ui::ui::{icon, mi, rx, InputArea};
use synshell_ui::ShellCtx;
use syngui::input::MouseButton;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::Named;

use crate::library::{Game, Source};
use crate::nav::{self, gs, Input, Zone};

/// Значок контроллера (Material Icons `sports_esports`).
pub const GAMEPAD: &str = "\u{EA28}";
const PLAY: &str = "\u{E037}";
const MORE: &str = "\u{E5D3}";
const STAR_BORDER: &str = "\u{E83A}";

/// Базовая ширина плитки по `[gameshell] tile_size` (обложка или значок).
pub fn base_width(icons: bool) -> f32 {
    let cfg = ShellCtx::get().cfg();
    let w = match cfg.gameshell.tile_size.trim() {
        "small" => 150.0,
        "large" => 236.0,
        _ => 188.0,
    };
    if icons {
        (w * 0.82f32).round()
    } else {
        w
    }
}

/// Размер плитки: ширина подогнана под экран (`nav::set_size`), обложка 2:3,
/// значок — квадрат.
pub fn tile_dims(icons: bool) -> (f32, f32) {
    let (cw, iw) = gs().tile_w.get_untracked();
    if icons {
        (iw, iw)
    } else {
        (cw, (cw * 1.5).round())
    }
}

/// Зазор между плитками.
pub const GAP: f32 = 22.0;

fn px(v: f32) -> StyleValue {
    StyleValue::px(v)
}

/// Цвет подложки без обложки — по имени (стабильный).
fn fallback_class(name: &str) -> String {
    let h = name.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    format!("gs-fb-{}", h % 8)
}

fn image(path: &std::path::Path, fit: ImageFit, class: &str) -> impl Widget {
    Image::new(path.to_string_lossy()).fit(fit).placeholder(false).class(class.to_string())
}

/// Значок игры (картинка или глиф).
pub fn game_icon(g: &Game, class: &str) -> Box<dyn Widget> {
    match &g.icon {
        Some(p) => Box::new(image(p, ImageFit::Contain, class)),
        None => Box::new(icon(if g.is_game { GAMEPAD } else { mi::APPS }).class(format!("{class}-glyph"))),
    }
}

/// Карточка плитки (без реакции на ввод).
fn card(g: &Game, icons: bool, selected: bool, fav: bool, launching: bool) -> impl Widget {
    let (w, h) = tile_dims(icons);
    let use_cover = !icons && ShellCtx::get().cfg().gameshell.tile_style.trim() != "icon";
    let mut stack = Stack::new().fit(StackFit::Expand);
    match (&g.cover, use_cover) {
        (Some(c), true) => stack = stack.child(image(c, ImageFit::Cover, "gs-cover")),
        _ => {
            let label = if icons { None } else { Some(Text::new(g.name.clone()).max_lines(3).class("gs-fb-name")) };
            let mut col = Column::new()
                .gap(12.0)
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().child(game_icon(g, "gs-fb-icon")).style("width", px(w * 0.46)).style("height", px(w * 0.46)));
            if let Some(l) = label {
                col = col.child(l);
            }
            stack = stack.child(DecoratedBox::new().child(col).class(format!("gs-fb {}", fallback_class(&g.name))));
        }
    }
    if fav {
        stack = stack.child(
            Column::new()
                .cross_axis_alignment(CrossAxisAlignment::End)
                .child(DecoratedBox::new().child(icon(mi::STAR).class("gs-badge-icon")).class("gs-badge")),
        );
    }
    if launching {
        stack = stack.child(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(CircularProgress::new().indeterminate().size(44.0).class("gs-spinner")),
                )
                .class("gs-tile-launch"),
        );
    }
    let frame = DecoratedBox::new()
        .clip(true)
        .child(stack)
        .class(if selected { "gs-tile gs-tile-sel" } else { "gs-tile" })
        .style("width", px(w))
        .style("height", px(h));
    let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(frame);
    // Подпись — у обложек с картинкой (на подложке имя уже есть) и у значков.
    if icons || (use_cover && g.cover.is_some()) || selected {
        col = col.child(Text::new(g.name.clone()).max_lines(1).class(if selected { "gs-tile-name gs-tile-name-sel" } else { "gs-tile-name" }).style("width", px(w)));
    }
    col
}

/// Плитка игры `index` раздела `section` (выделение, наведение, касание).
pub fn tile(section: usize, key: &str, index: usize, game: Game, icons: bool) -> impl Widget {
    let g = gs();
    let manage = key == "procs";
    let ga = game.clone();
    let body = rx(move || {
        let sections = g.sections.get();
        let (r, c) = g.sel.get();
        let rows = nav::nav_rows(&sections, g.cols.get());
        let selected = g.zone.get() == Zone::Content && rows.get(r).is_some_and(|row| row.section == section && row.start + c.min(row.len.saturating_sub(1)) == index);
        let fav = g.user.get().is_favorite(&ga.id);
        let launching = g.launching.get().is_some_and(|l| l.0 == ga.id);
        let card = card(&ga, icons, selected, fav, launching);
        if !manage {
            return Box::new(card);
        }
        // Вкладка «Запущенные»: закрыть или убить кнопками под плиткой.
        let _ = ShellCtx::get().windows.get();
        let Some(wid) = nav::window_of(&ga) else { return Box::new(card) };
        let name = ga.name.clone();
        let buttons = Row::new()
            .gap(10.0)
            .main_axis_alignment(MainAxisAlignment::Center)
            .child(button(mi::CLOSE, Some(t!("Закрыть").to_string()), "gs-btn-sm", move || nav::close_window(wid, name.clone())))
            .child(button(synshell_ui::desk::KILL, None, "gs-btn-sm gs-btn-danger", move || nav::kill_window(wid)));
        Box::new(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(card).child(buttons))
    });
    let gl = game.clone();
    Named::new(
        nav::tile_name(key, index),
        InputArea::new(body)
            .pointer()
            .on_hover(move |h| {
                if h && gs().input.get_untracked() == Input::Pointer {
                    nav::select_item(section, index);
                }
            })
            .on_click(move |b, _, _| {
                let g = gs();
                if g.input.get_untracked() != Input::Pointer {
                    g.input.set(Input::Pointer);
                }
                nav::select_item(section, index);
                match b {
                    MouseButton::Left => nav::launch(&gl),
                    MouseButton::Right => nav::dispatch(nav::Nav::Options, Input::Pointer),
                    _ => {}
                }
            }),
    )
}

/// «Сегодня», «вчера», «5 дн. назад», дата.
pub fn played_label(ts: u64) -> Option<String> {
    if ts == 0 {
        return None;
    }
    let now = crate::library::now();
    let d = now.saturating_sub(ts);
    Some(if d < 3600 {
        t!("Запускалась только что").to_string()
    } else if d < 86_400 {
        t!("Запускалась сегодня").to_string()
    } else if d < 2 * 86_400 {
        t!("Запускалась вчера").to_string()
    } else if d < 30 * 86_400 {
        t!("Запускалась {n} дн. назад", n = d / 86_400).to_string()
    } else {
        t!("Запускалась {d}", d = synshell_ui::clock::format(ts as i64, "%d.%m.%Y")).to_string()
    })
}

fn chip(text: String, glyph: Option<&'static str>, class: &str) -> impl Widget {
    let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if let Some(gl) = glyph {
        row = row.child(icon(gl).class("gs-chip-icon"));
    }
    DecoratedBox::new().child(row.child(Text::new(text).class("gs-chip-text"))).class(format!("gs-chip {class}"))
}

fn button(glyph: &'static str, label: Option<String>, class: &str, f: impl FnMut() + Send + 'static) -> impl Widget {
    let mut f = f;
    let mut row = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(glyph).class("gs-btn-icon"));
    if let Some(l) = label {
        row = row.child(Text::new(l).class("gs-btn-text"));
    }
    InputArea::new(DecoratedBox::new().child(row).class(format!("gs-btn {class}"))).pointer().on_click(move |b, _, _| {
        if b == MouseButton::Left {
            f();
        }
    })
}

/// Афиша выбранной игры (главная).
pub fn hero() -> impl Widget {
    let g = gs();
    rx(move || {
        let Some(game) = nav::selected_game(&g) else {
            return Box::new(DecoratedBox::new().class("gs-hero gs-hero-empty"));
        };
        let ctx = ShellCtx::get();
        let _ = ctx.windows.get();
        let fav = g.user.get().is_favorite(&game.id);
        let running = nav::window_of(&game).is_some();
        let title: Box<dyn Widget> = match &game.logo {
            Some(l) => Box::new(DecoratedBox::new().child(image(l, ImageFit::Contain, "gs-hero-logo")).class("gs-hero-logo-box")),
            None => Box::new(Text::new(game.name.clone()).max_lines(2).class("gs-hero-title")),
        };
        let mut meta = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(chip(game.source_label(), Some(if game.source == Source::Steam { mi::COMPUTER } else { GAMEPAD }), ""));
        if running {
            meta = meta.child(chip(t!("Запущено").to_string(), Some(PLAY), "gs-chip-live"));
        }
        if let Some(p) = played_label(game.last_played) {
            meta = meta.child(Text::new(p).class("gs-hero-meta"));
        }
        let g1 = game.clone();
        let g2 = game.clone();

        let play = if running { t!("Продолжить") } else { t!("Играть") };
        let buttons = Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(button(PLAY, Some(play.to_string()), "gs-btn-primary", move || nav::launch(&g1)))
            .child(button(if fav { mi::STAR } else { STAR_BORDER }, None, if fav { "gs-btn-round gs-btn-on" } else { "gs-btn-round" }, move || nav::toggle_favorite(&g2)))
            .child(button(MORE, None, "gs-btn-round", move || nav::dispatch(nav::Nav::Options, Input::Pointer)));
        Box::new(Column::new().gap(16.0).main_axis_alignment(MainAxisAlignment::End).child(title).child(meta).child(buttons).class("gs-hero"))
    })
}

/// Фон: размытая афиша выбранной игры, обои или цвет.
pub fn background(output: String) -> impl Widget {
    let g = gs();
    let ctx = ShellCtx::get();
    let mode = ctx.cfg().gameshell.background.trim().to_string();
    let base: Box<dyn Widget> = match mode.as_str() {
        "wallpaper" => Box::new(synshell_ui::manager::wallpaper_view(output, use_signal(0u64), None)),
        _ => Box::new(DecoratedBox::new().class("gs-bg-color")),
    };
    let mut stack = Stack::new().fit(StackFit::Expand).child(base);
    if mode == "hero" {
        stack = stack.child(rx(move || {
            let path = nav::selected_game(&g).and_then(|x| x.backdrop());
            let key = path.as_ref().map(|p| p.to_string_lossy().bytes().fold(7u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64))).unwrap_or(0);
            Box::new(
                AnimatedSwitcher::new(key, move || match &path {
                    Some(p) => Box::new(image(p, ImageFit::Cover, "gs-bg-art")),
                    None => Box::new(DecoratedBox::new()),
                })
                .duration_ms(420)
                .exit_duration_ms(420),
            )
        }));
    }
    stack.child(DecoratedBox::new().class("gs-scrim"))
}

/// Вкладки (LB/RB по краям у геймпада).
pub fn tabs_bar() -> impl Widget {
    let g = gs();
    rx(move || {
        let ctx = ShellCtx::get();
        let cur = g.tab.get();
        let zone = g.zone.get();
        let input = g.input.get();
        let _ = g.lib.get();
        let _ = g.user.get();
        let _ = ctx.windows.get();
        let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
        // Кнопка быстрого меню — для касаний и мыши (геймпад — Start/Guide).
        row = row.child(
            InputArea::new(DecoratedBox::new().child(icon("\u{E5D2}").class("gs-tab-icon")).class("gs-tab gs-menu-btn"))
                .pointer()
                .on_click(|_, _, _| crate::overlay::open(crate::overlay::Section::Main)),
        );
        let hint = |s: &str| DecoratedBox::new().child(Text::new(s.to_string()).class("gs-key-text")).class("gs-key");
        match input {
            Input::Gamepad => row = row.child(hint("LB")),
            Input::Keyboard => row = row.child(hint("Q")),
            Input::Pointer => {}
        }
        for (i, (_, label, glyph)) in nav::tabs(&ctx, &g).into_iter().enumerate() {
            let active = i == cur;
            let class = match (active, zone == Zone::Tabs && active) {
                (true, true) => "gs-tab gs-tab-on gs-tab-focus",
                (true, false) => "gs-tab gs-tab-on",
                _ => "gs-tab",
            };
            row = row.child(Named::new(format!("gs-tab-{i}"),
                InputArea::new(DecoratedBox::new().child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(glyph).class("gs-tab-icon")).child(Text::new(label).class("gs-tab-text"))).class(class))
                    .pointer()
                    .on_click(move |_, _, _| {
                        let g = gs();
                        g.input.set(Input::Pointer);
                        g.tab.set(i);
                        g.sel.set((0, 0));
                        g.zone.set(Zone::Content);
                    }),
            ));
        }
        match input {
            Input::Gamepad => row = row.child(hint("RB")),
            Input::Keyboard => row = row.child(hint("E")),
            Input::Pointer => {}
        }
        Box::new(row)
    })
}

/// Строка состояния: сеть, батарея, громкость, геймпад, часы.
pub fn status_bar() -> impl Widget {
    rx(move || {
        let ctx = ShellCtx::get();
        let cfg = ctx.cfg();
        let now = ctx.now.get();
        let mut row = Row::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for item in &cfg.gameshell.status {
            match item.trim() {
                "network" => {
                    let n = ctx.network.get();
                    let glyph = match n.kind.as_str() {
                        "wifi" => mi::WIFI,
                        "ethernet" => mi::ETHERNET,
                        _ => mi::NET_OFF,
                    };
                    row = row.child(icon(glyph).class("gs-status-icon"));
                }
                "battery" => {
                    if let Some(b) = ctx.battery.get() {
                        let glyph = if b.charging { mi::BATTERY_CHARGING } else if b.percent <= 15 { mi::BATTERY_ALERT } else { mi::BATTERY_FULL };
                        row = row.child(Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(glyph).class("gs-status-icon")).child(Text::new(format!("{}%", b.percent)).class("gs-status-text")));
                    }
                }
                "volume" => {
                    if let Some(v) = ctx.volume.get() {
                        let glyph = if v.muted || v.percent == 0 { mi::VOLUME_MUTE } else { mi::VOLUME_UP };
                        row = row.child(icon(glyph).class("gs-status-icon"));
                    }
                }
                "gamepad" => {
                    if crate::gamepad::CONNECTED.load(Ordering::Relaxed) > 0 {
                        row = row.child(icon(GAMEPAD).class("gs-status-icon"));
                    }
                }
                "clock" => row = row.child(Text::new(synshell_ui::clock::format(now, "%H:%M")).class("gs-clock")),
                _ => {}
            }
        }
        Box::new(row)
    })
}

fn key_hint(input: Input, pad: &'static str, pad_class: &'static str, key: &'static str, label: String) -> impl Widget {
    let glyph = match input {
        Input::Gamepad => DecoratedBox::new().child(Text::new(pad).class("gs-pad-text")).class(format!("gs-pad {pad_class}")),
        _ => DecoratedBox::new().child(Text::new(key).class("gs-key-text")).class("gs-key".to_string()),
    };
    Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(glyph).child(Text::new(label).class("gs-hint-text"))
}

/// Подсказки кнопок внизу (для геймпада и клавиатуры).
pub fn hints() -> impl Widget {
    let g = gs();
    rx(move || {
        let input = g.input.get();
        if input == Input::Pointer || !ShellCtx::get().cfg().gameshell.hints {
            return Box::new(DecoratedBox::new().class("gs-hints-off"));
        }
        let sheet = g.sheet.get().is_some() || g.confirm.get().is_some();
        let mut row = Row::new().gap(26.0).cross_axis_alignment(CrossAxisAlignment::Center).main_axis_alignment(MainAxisAlignment::End);
        if sheet {
            row = row.child(key_hint(input, "A", "gs-pad-a", "Enter", t!("Выбрать").to_string())).child(key_hint(input, "B", "gs-pad-b", "Esc", t!("Отмена").to_string()));
        } else {
            let running = nav::selected_game(&g).is_some_and(|x| nav::window_of(&x).is_some());
            row = row
                .child(key_hint(input, "A", "gs-pad-a", "Enter", if running { t!("Продолжить") } else { t!("Играть") }.to_string()))
                .child(key_hint(input, "X", "gs-pad-x", "X", t!("Действия").to_string()))
                .child(key_hint(input, "Y", "gs-pad-y", "F", t!("Избранное").to_string()))
                .child(key_hint(input, "B", "gs-pad-b", "Esc", t!("Назад").to_string()))
                .child(key_hint(input, "☰", "gs-pad-menu", "M", t!("Меню").to_string()));
        }
        Box::new(DecoratedBox::new().child(row).class("gs-hints"))
    })
}

/// Пустая вкладка.
pub fn empty(key: &str) -> impl Widget {
    let (title, body) = match key {
        "favorites" => (t!("В избранном пусто"), t!("Выделите игру и нажмите Y (F на клавиатуре)")),
        "steam" => (t!("Игр Steam не найдено"), t!("Установите игры в Steam — они появятся здесь")),
        "apps" | "android" => (t!("Приложений не найдено"), String::new().into()),
        _ => (
            t!("Игр пока нет"),
            t!("Игры находятся в Steam, среди программ с категорией Game и в [[gameshell.game]] config.toml"),
        ),
    };
    Column::new()
        .gap(14.0)
        .main_axis_alignment(MainAxisAlignment::Center)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(icon(GAMEPAD).class("gs-empty-icon"))
        .child(Text::new(title).class("gs-empty-title"))
        .child(Text::new(body).class("gs-empty-text"))
        .class("gs-empty")
}
