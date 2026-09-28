//! Стартовая страница и виды игры через TestHarness (без GPU):
//! выбор карточки → игра нужного вида; «В начало» и кнопка «Назад» возвращают.

use std::time::Duration;

use elle_block::persist::{self, Stats};
use elle_block::ui::layout;
use elle_block::ui::overlay::{self, Action};
use elle_block::ui::start;
use elle_block::{GameCtx, GameView, Screen, Variant};
use syngui::prelude::*;
use syngui::testing::{press_key, TestHarness};

const W: f32 = 460.0;
const H: f32 = 820.0;

fn setup(tag: &str) -> (TestHarness, GameCtx) {
    let dir = std::env::temp_dir().join(format!("elle_block_start_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::env::set_var("ELLE_BLOCK_DATA", &dir);
    syngui::signal::allow_signal_reads_on_this_thread();
    let ctx = GameCtx::new(3, Stats::default());
    ctx.screen.set(Screen::Start);
    provide_context(ctx);
    let mut h = TestHarness::new(Box::new(GameView));
    h.layout(W, H);
    (h, ctx)
}

fn center(r: Rect) -> Point {
    Point::new(r.origin.x + r.width() * 0.5, r.origin.y + r.height() * 0.5)
}

fn click(h: &mut TestHarness, p: Point) {
    h.send_event(&Event::MouseDown { button: MouseButton::Left, position: p });
    h.send_event(&Event::MouseUp { button: MouseButton::Left, position: p });
}

fn frames(h: &mut TestHarness, n: usize) {
    for _ in 0..n {
        h.animate(Duration::from_millis(16));
    }
}

#[test]
fn start_page_opens_chosen_variant_and_home_returns() {
    let (mut h, ctx) = setup("variant");
    let geo = layout::compute(Rect::new(Point::new(0.0, 0.0), Size::new(W, H)));
    let sg = start::layout(&geo);
    let _ = h.paint();
    // переход только что начался — карточки ещё не ловят тапы
    frames(&mut h, 40);
    // карточка «Эльвира»
    click(&mut h, center(sg.cards[1]));
    assert_eq!(ctx.screen.get_untracked(), Screen::Game);
    assert_eq!(ctx.variant.get_untracked(), Variant::Elvira);
    assert_eq!(ctx.stats.get_untracked().variant, 1, "выбор вида запоминается");
    let g = ctx.game.get_untracked();
    assert!(!g.over && g.tray.iter().all(|p| p.is_some()));
    assert!(g.tray.iter().flatten().all(|p| (1..=elle_block::game::FINISH_COUNT).contains(&p.finish)), "у фигур есть отделка");
    frames(&mut h, 40);
    let _ = h.paint();
    // меню паузы → «В начало»
    h.send_events(&press_key(Key::Escape));
    let card = overlay::menu_card(&geo);
    let home = card.buttons.iter().find(|b| b.action == Action::Home).unwrap().rect;
    click(&mut h, center(home));
    assert_eq!(ctx.screen.get_untracked(), Screen::Start);
    assert!(persist::load_game(Variant::Elvira).is_some(), "партия Эльвиры сохранена");
    assert!(persist::load_game(Variant::Standard).is_none(), "партии Стандарта ещё нет");
    frames(&mut h, 40);
    let _ = h.paint();
    // теперь «Стандарт»
    click(&mut h, center(sg.cards[0]));
    assert_eq!(ctx.variant.get_untracked(), Variant::Standard);
    assert_eq!(ctx.screen.get_untracked(), Screen::Game);
    frames(&mut h, 40);
    // кнопка «Назад» в игре — на стартовую страницу
    let r = h.send_event(&Event::BackPressed);
    assert!(r.is_handled());
    assert_eq!(ctx.screen.get_untracked(), Screen::Start);
    // на стартовой странице «Назад» отдаётся системе (выход)
    frames(&mut h, 40);
    assert!(!h.send_event(&Event::BackPressed).is_handled());
}

#[test]
fn start_toggles_and_keyboard_shortcuts() {
    let (mut h, ctx) = setup("toggles");
    let geo = layout::compute(Rect::new(Point::new(0.0, 0.0), Size::new(W, H)));
    let sg = start::layout(&geo);
    frames(&mut h, 40);
    let voice_before = ctx.stats.get_untracked().voice;
    click(&mut h, center(sg.toggles[2]));
    assert_ne!(ctx.stats.get_untracked().voice, voice_before);
    click(&mut h, center(sg.toggles[0]));
    assert!(!ctx.stats.get_untracked().sound);
    // клавиша 2 — «Эльвира»
    h.send_events(&press_key(Key::Num2));
    assert_eq!(ctx.variant.get_untracked(), Variant::Elvira);
    assert_eq!(ctx.screen.get_untracked(), Screen::Game);
}

#[test]
fn elvira_paints_every_state_without_atlas() {
    let (mut h, ctx) = setup("paint");
    let geo = layout::compute(Rect::new(Point::new(0.0, 0.0), Size::new(W, H)));
    let sg = start::layout(&geo);
    let _ = h.paint();
    frames(&mut h, 40);
    click(&mut h, center(sg.cards[1]));
    // кадры перехода и игра «Эльвиры» (текстур в тестах нет — отделки цветом)
    for _ in 0..6 {
        frames(&mut h, 5);
        let _ = h.paint();
    }
    // ход с очисткой: ряд 7 почти полон, домино его закрывает
    let mut g = ctx.game.get_untracked();
    g.board = elle_block::game::Board::new();
    for c in 0..6 {
        g.board.set(7, c, elle_block::game::make_cell_full(2, elle_block::game::Material::Plain, 5));
    }
    g.tray = [Some(elle_block::game::Piece::of(elle_block::game::Family::I2).with_finish(1)), None, Some(elle_block::game::Piece::of(elle_block::game::Family::Mono).with_finish(7))];
    g.scoring.combo = 2;
    ctx.game.set(g);
    let o = geo.piece_origin_in_slot(0, &elle_block::game::Family::I2.base());
    let from = Point::new(o.x + geo.tray_cell * 0.5, o.y + geo.tray_cell * 0.5);
    let to = Point::new(geo.grid.x + 6.0 * geo.cell + geo.tray_cell * 0.5, geo.grid.y + 7.0 * geo.cell + geo.tray_cell * 0.5);
    h.send_event(&Event::MouseDown { button: MouseButton::Left, position: from });
    h.send_event(&Event::MouseMove(Point::new(from.x + 8.0, from.y)));
    h.send_event(&Event::MouseMove(to));
    let _ = h.paint();
    h.send_event(&Event::MouseUp { button: MouseButton::Left, position: to });
    assert_eq!(ctx.game.get_untracked().scoring.lines, 1);
    for _ in 0..8 {
        frames(&mut h, 4);
        let _ = h.paint();
    }
    h.layout(340.0, 600.0);
    let _ = h.paint();
    h.layout(1400.0, 700.0);
    let _ = h.paint();
    // и стартовая страница в тех же размерах
    h.send_event(&Event::BackPressed);
    let _ = h.paint();
    frames(&mut h, 40);
    let _ = h.paint();
}
