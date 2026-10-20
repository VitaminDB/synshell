//! Главный экран gameshell: одна непрозрачная поверхность под окнами (слой
//! Bottom, пространство имён `syngame-home` — композитор отдаёт ей
//! клавиатуру, когда окон нет). Сверху вкладки и строка состояния, на
//! главной — афиша выбранной игры и полки (запущено, недавние, избранное,
//! все игры), на остальных вкладках — сетка обложек; внизу подсказки
//! кнопок. Поверх — меню действий игры, подтверждение закрытия и заставка
//! запуска.

use std::cell::Cell;

use synshell_ui::ui::{icon, mi, rx, InputArea};
use synshell_ui::ShellCtx;
use syngui::input::MouseButton;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::ScrollbarPolicy;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceHooks, SurfaceId, SurfaceSpec};

use crate::nav::{self, gs, Input, SheetItem, Zone};
use crate::tiles;

thread_local! {
    static SURFACE: Cell<Option<SurfaceId>> = const { Cell::new(None) };
    static BUILT: Cell<Option<(u64, u64)>> = const { Cell::new(None) };
}

pub fn install(ctx: ShellCtx) {
    let g = nav::init(ctx);
    // Пересобрать поверхность при смене конфига и выводов.
    create_effect(move || {
        let outputs = syngui_layer::outputs().get();
        let generation = ctx.generation.get();
        // Поворот экрана меняет размер вывода — раскладка другая.
        let sizes = outputs.iter().fold(outputs.len() as u64, |h, o| h.wrapping_mul(1_000_003).wrapping_add((o.size.0 as u64) << 20 | o.size.1 as u64));
        let key = (generation, sizes);
        if BUILT.with(|b| b.get()) == Some(key) {
            return;
        }
        BUILT.with(|b| b.set(Some(key)));
        if let Some(id) = SURFACE.with(|s| s.take()) {
            syngui_layer::close_surface(id);
        }
        let Some(out) = synshell_ui::manager::primary_output(&ctx).or_else(|| outputs.first().map(|o| o.name.clone())) else { return };
        // Колонки сетки — по ширине вывода.
        let (w, _) = synshell_ui::manager::output_size(Some(&out));
        let (tw, _) = tiles::tile_dims(false);
        let pad = side_pad(w);
        let cols = (((w - 2.0 * pad + tiles::GAP) / (tw + tiles::GAP)).floor() as usize).max(2);
        g.cols.set(cols);
        let spec = SurfaceSpec {
            namespace: "syngame-home".into(),
            layer: Layer::Bottom,
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            size: (0, 0),
            margin: [0; 4],
            exclusive_zone: -1,
            keyboard: KeyboardInteractivity::OnDemand,
            output: Some(out.clone()),
            auto_size: false,
            clear_color: [0.04, 0.05, 0.08, 1.0],
        };
        let hooks = SurfaceHooks { on_key: Some(Box::new(nav::key)), ..Default::default() };
        let id = syngui_layer::create_surface_with(spec, hooks, move || Box::new(view(out.clone())));
        SURFACE.with(|s| s.set(Some(id)));
    });
    // Главный экран снова виден (игра закрыта или свёрнута) — библиотека
    // могла измениться (новая игра, время запуска).
    let was_visible = Cell::new(true);
    create_effect(move || {
        let _ = ctx.windows.get();
        let vis = nav::home_visible(&ctx);
        if vis && !was_visible.get() {
            nav::rescan(&ctx);
        }
        was_visible.set(vis);
    });
    synshell_ui::on_reload(|ctx, _| nav::rescan(&ctx));
}

fn side_pad(w: f32) -> f32 {
    if w < 900.0 {
        28.0
    } else {
        64.0
    }
}

fn view(output: String) -> impl Widget {
    let (w, h) = synshell_ui::manager::output_size(Some(&output));
    let pad = side_pad(w);
    // Узкий (портретный) экран: строка состояния — своей строкой, вкладки листаются вбок.
    let top: Box<dyn Widget> = if w < 760.0 {
        Box::new(
            Column::new()
                .gap(10.0)
                .child(Row::new().main_axis_alignment(MainAxisAlignment::End).child(tiles::status_bar()).style("padding-left", StyleValue::px(pad)).style("padding-right", StyleValue::px(pad)))
                .child(ScrollView::new().horizontal().scrollbar_policy(ScrollbarPolicy::Never).child(DecoratedBox::new().child(tiles::tabs_bar()).style("padding-left", StyleValue::px(pad)).style("padding-right", StyleValue::px(pad))))
                .class("gs-top"),
        )
    } else {
        Box::new(
            Row::new()
                .gap(16.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().child(tiles::tabs_bar()).class("grow"))
                .child(tiles::status_bar())
                .class("gs-top")
                .style("padding-left", StyleValue::px(pad))
                .style("padding-right", StyleValue::px(pad)),
        )
    };
    let body = body(pad, h);
    let content = Column::new()
        .child(top)
        .child(DecoratedBox::new().child(body).class("grow"))
        .child(DecoratedBox::new().child(tiles::hints()).style("padding-left", StyleValue::px(pad)).style("padding-right", StyleValue::px(pad)));
    Stack::new()
        .fit(StackFit::Expand)
        .child(tiles::background(output))
        .child(content)
        .child(modals())
        .child(launch_splash())
}

/// Тело вкладки: на главной — афиша и полки, иначе — сетка.
fn body(pad: f32, screen_h: f32) -> impl Widget {
    let g = gs();
    rx(move || {
        let sections = g.sections.get();
        let ctx = ShellCtx::get();
        let tabs = nav::tabs(&ctx, &g);
        let tab_key = tabs.get(g.tab.get()).map(|t| t.0.clone()).unwrap_or_default();
        if sections.iter().all(|s| s.items.is_empty()) {
            return Box::new(tiles::empty(&tab_key));
        }
        let mut col = Column::new().gap(26.0);
        // Афиша — над прокруткой, на месте.
        // Низкий экран (телефон боком) — без афиши: место полкам, картинка и так на фоне.
        let hero = (tab_key == "home" && ctx.cfg().gameshell.hero && screen_h >= 640.0).then(|| {
            DecoratedBox::new().child(tiles::hero()).style("height", StyleValue::px((screen_h * 0.30).max(180.0))).style("padding-left", StyleValue::px(pad)).style("padding-right", StyleValue::px(pad))
        });
        for (si, s) in sections.iter().enumerate() {
            if s.items.is_empty() {
                continue;
            }
            if s.grid {
                let cols = g.cols.get_untracked().max(1);
                let mut grid = Column::new().gap(tiles::GAP + 6.0);
                for (r, chunk) in s.items.chunks(cols).enumerate() {
                    let mut row = Row::new().gap(tiles::GAP);
                    for (c, game) in chunk.iter().enumerate() {
                        row = row.child(tiles::tile(si, &s.key, r * cols + c, game.clone(), s.icons));
                    }
                    grid = grid.child(row);
                }
                col = col.child(DecoratedBox::new().child(grid).style("padding-left", StyleValue::px(pad)).style("padding-right", StyleValue::px(pad)).class("gs-grid"));
            } else {
                let mut row = Row::new().gap(tiles::GAP);
                for (i, game) in s.items.iter().enumerate() {
                    row = row.child(tiles::tile(si, &s.key, i, game.clone(), s.icons));
                }
                let shelf = ScrollView::new()
                    .horizontal()
                    .scrollbar_policy(ScrollbarPolicy::Never)
                    .child(DecoratedBox::new().child(row).style("padding-left", StyleValue::px(pad)).style("padding-right", StyleValue::px(pad)).class("gs-shelf-row"));
                col = col.child(
                    Column::new()
                        .gap(12.0)
                        .child(Text::new(s.title.clone()).class("gs-section-title").style("padding-left", StyleValue::px(pad)))
                        .child(shelf),
                );
            }
        }
        let scroll = ScrollView::new().vertical().scrollbar_policy(ScrollbarPolicy::Never).child(DecoratedBox::new().child(col).class("gs-body"));
        match hero {
            Some(h) => Box::new(Column::new().child(h).child(DecoratedBox::new().child(scroll).class("grow"))),
            None => Box::new(scroll),
        }
    })
}

fn sheet_label(it: &SheetItem) -> (&'static str, String) {
    match it {
        SheetItem::Play => ("\u{E037}", t!("Играть").to_string()),
        SheetItem::Resume => ("\u{E037}", t!("Продолжить").to_string()),
        SheetItem::Favorite(true) => (mi::STAR, t!("Убрать из избранного").to_string()),
        SheetItem::Favorite(false) => (mi::STAR, t!("В избранное").to_string()),
        SheetItem::CloseGame => (mi::CLOSE, t!("Закрыть игру").to_string()),
        SheetItem::Hide => ("\u{E8F5}", t!("Скрыть из библиотеки").to_string()),
    }
}

/// Меню действий игры и подтверждение закрытия.
fn modals() -> impl Widget {
    let g = gs();
    rx(move || {
        if let Some((wid, name)) = g.confirm.get() {
            let card = Column::new()
                .gap(18.0)
                .child(Text::new(t!("Закрыть «{n}»?", n = name)).class("gs-modal-title"))
                .child(Text::new(t!("Несохранённый прогресс может пропасть.")).class("gs-modal-text"))
                .child(
                    Row::new()
                        .gap(12.0)
                        .main_axis_alignment(MainAxisAlignment::End)
                        .child(modal_btn(t!("Отмена").to_string(), false, move || gs().confirm.set(None)))
                        .child(modal_btn(t!("Закрыть").to_string(), true, move || nav::close_window(wid, name.clone()))),
                );
            return Box::new(backdrop(Box::new(card), || gs().confirm.set(None)));
        }
        let Some(sh) = g.sheet.get() else { return Box::new(DecoratedBox::new().class("gs-none")) };
        let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(sh.game.name.clone()).max_lines(2).class("gs-modal-title"));
        for (i, it) in sh.items.iter().enumerate() {
            let (glyph, label) = sheet_label(it);
            let it2 = it.clone();
            let game = sh.game.clone();
            col = col.child(
                InputArea::new(
                    DecoratedBox::new()
                        .child(Row::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(glyph).class("gs-item-icon")).child(Text::new(label).class("gs-item-text")))
                        .class(if i == sh.sel { "gs-item gs-item-sel" } else { "gs-item" }),
                )
                .pointer()
                .on_hover(move |h| {
                    let g = gs();
                    if h && g.input.get_untracked() == Input::Pointer {
                        if let Some(mut s) = g.sheet.get_untracked() {
                            s.sel = i;
                            g.sheet.set(Some(s));
                        }
                    }
                })
                .on_click(move |b, _, _| {
                    if b == MouseButton::Left {
                        nav::sheet_activate(&it2, &game);
                    }
                }),
            );
        }
        Box::new(backdrop(Box::new(col), || gs().sheet.set(None)))
    })
}

fn modal_btn(label: String, primary: bool, f: impl Fn() + Send + 'static) -> impl Widget {
    InputArea::new(DecoratedBox::new().child(Text::new(label).class("gs-btn-text")).class(if primary { "gs-btn gs-btn-primary" } else { "gs-btn" }))
        .pointer()
        .on_click(move |b, _, _| {
            if b == MouseButton::Left {
                f();
            }
        })
}

fn backdrop(card: Box<dyn Widget>, close: fn()) -> impl Widget {
    Stack::new()
        .fit(StackFit::Expand)
        .child(InputArea::new(DecoratedBox::new().class("gs-modal-scrim")).on_press(move |_, _, _| close()))
        .child(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(InputArea::new(DecoratedBox::new().child(card).class("gs-modal")).absorb()),
        )
}

/// Заставка запуска: афиша и «Запуск…», пока не появится окно игры.
fn launch_splash() -> impl Widget {
    let g = gs();
    rx(move || {
        let Some((id, _)) = g.launching.get() else { return Box::new(DecoratedBox::new().class("gs-none")) };
        let lib = g.lib.get_untracked();
        let Some(game) = lib.find(&id).cloned() else { return Box::new(DecoratedBox::new().class("gs-none")) };
        let mut stack = Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().class("gs-splash-bg"));
        if let Some(p) = game.backdrop() {
            stack = stack.child(Image::new(p.to_string_lossy()).fit(ImageFit::Cover).placeholder(false).class("gs-splash-art"));
        }
        stack = stack.child(DecoratedBox::new().class("gs-splash-scrim")).child(
            Column::new()
                .gap(20.0)
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(tiles::game_icon(&game, "gs-splash-icon"))
                .child(Text::new(game.name.clone()).class("gs-splash-title"))
                .child(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(CircularProgress::new().indeterminate().size(26.0).class("gs-spinner")).child(Text::new(t!("Запуск…")).class("gs-splash-text"))),
        );
        let _ = Zone::Content;
        Box::new(Presence::new(true, stack).enter(Motion::fade()).duration_ms(220))
    })
}
