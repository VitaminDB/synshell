//! Интеграционные тесты ввода GameView через syngui TestHarness (без GPU).
//! Геометрия берётся из `ui::layout::compute` — те же числа, что у элемента.

use std::time::Duration;

use syn_blocks::game::{cell_color, Board, Family, Game, Piece, SIZE};
use syn_blocks::persist::Stats;
use syn_blocks::ui::layout::{self, Geo};
use syn_blocks::{GameCtx, GameView};
use syngui::prelude::*;
use syngui::testing::{press_key, TestHarness};

const W: f32 = 460.0;
const H: f32 = 820.0;

fn isolate_stats() {
    let dir = std::env::temp_dir().join(format!("syn_blocks_test_{}", std::process::id()));
    std::env::set_var("SYN_BLOCKS_DATA", &dir);
}

/// Пустое поле; в слотах — Mono, I2 (горизонтальное домино), O2.
fn setup(seed: u64) -> (TestHarness, GameCtx, Geo) {
    isolate_stats();
    syngui::signal::allow_signal_reads_on_this_thread();
    let ctx = GameCtx::new(seed, Stats::default());
    let mut g = ctx.game.get_untracked();
    g.board = Board::new();
    g.tray = [Some(Piece::of(Family::Mono)), Some(Piece::of(Family::I2)), Some(Piece::of(Family::O2))];
    g.over = false;
    ctx.game.set(g);
    provide_context(ctx);
    let mut h = TestHarness::new(Box::new(GameView));
    h.layout(W, H);
    let geo = layout::compute(Rect::new(Point::new(0.0, 0.0), Size::new(W, H)));
    (h, ctx, geo)
}

fn game(ctx: &GameCtx) -> Game {
    ctx.game.get_untracked()
}

/// Точка захвата: центр первой клетки фигуры в слоте.
fn grab_point(geo: &Geo, slot: usize, g: &Game) -> Point {
    let p = g.piece(slot).unwrap();
    let o = geo.piece_origin_in_slot(slot, &p.shape);
    Point::new(o.x + geo.tray_cell * 0.5, o.y + geo.tray_cell * 0.5)
}

/// Куда вести курсор, чтобы рамка фигуры встала якорем в `(r, c)`
/// (без тиков анимации подъём = 0, размер клетки = tray_cell).
fn drop_point(geo: &Geo, r: usize, c: usize) -> Point {
    Point::new(geo.grid.x + c as f32 * geo.cell + geo.tray_cell * 0.5, geo.grid.y + r as f32 * geo.cell + geo.tray_cell * 0.5)
}

fn drag(h: &mut TestHarness, from: Point, to: Point) -> Vec<EventResult> {
    let mid = Point::new((from.x + to.x) * 0.5, (from.y + to.y) * 0.5);
    vec![
        h.send_event(&Event::MouseDown { button: MouseButton::Left, position: from }),
        h.send_event(&Event::MouseMove(mid)),
        h.send_event(&Event::MouseMove(to)),
        h.send_event(&Event::MouseUp { button: MouseButton::Left, position: to }),
    ]
}

#[test]
fn drag_from_tray_places_piece() {
    let (mut h, ctx, geo) = setup(1);
    let g0 = game(&ctx);
    let results = drag(&mut h, grab_point(&geo, 1, &g0), drop_point(&geo, 3, 2));
    assert!(results.iter().all(|r| r.is_handled()), "все события drag должны быть Handled: {results:?}");
    let g = game(&ctx);
    assert_eq!(cell_color(g.board.get(3, 2)), Family::I2.color());
    assert_eq!(cell_color(g.board.get(3, 3)), Family::I2.color());
    assert_eq!(g.board.filled(), 2);
    assert!(g.tray[1].is_none());
    assert_eq!(g.scoring.score, 20);
}

#[test]
fn magnetic_snap_forgives_small_offset() {
    let (mut h, ctx, geo) = setup(2);
    let g0 = game(&ctx);
    let mut to = drop_point(&geo, 5, 5);
    to.x += geo.cell * 0.3;
    to.y -= geo.cell * 0.25;
    drag(&mut h, grab_point(&geo, 0, &g0), to);
    let g = game(&ctx);
    assert_eq!(cell_color(g.board.get(5, 5)), Family::Mono.color(), "фигура должна прилипнуть к ближайшей клетке");
}

#[test]
fn release_outside_board_returns_piece() {
    let (mut h, ctx, geo) = setup(3);
    let g0 = game(&ctx);
    drag(&mut h, grab_point(&geo, 2, &g0), Point::new(geo.bounds.width() - 10.0, geo.top.origin.y + 10.0));
    let g = game(&ctx);
    assert!(g.tray[2].is_some());
    assert_eq!(g.board.filled(), 0);
    // тики анимации возврата не должны падать
    for _ in 0..30 {
        h.animate(Duration::from_millis(16));
    }
}

#[test]
fn cannot_drop_on_occupied_cells() {
    let (mut h, ctx, geo) = setup(4);
    let mut g0 = game(&ctx);
    g0.board.set(2, 2, 3);
    g0.board.set(2, 3, 3);
    ctx.game.set(g0.clone());
    drag(&mut h, grab_point(&geo, 1, &g0), drop_point(&geo, 2, 2));
    let g = game(&ctx);
    assert!(g.tray[1].is_some(), "занятые клетки — фигура возвращается");
    assert_eq!(g.board.filled(), 2);
}

#[test]
fn refill_after_all_three_and_line_clear_scores() {
    let (mut h, ctx, geo) = setup(5);
    // ряд 7 почти полон: свободны колонки 5,6,7 → I2 в (7,5) и Mono в (7,7) закрывают ряд
    let mut g0 = game(&ctx);
    for c in 0..5 {
        g0.board.set(7, c, 2);
    }
    ctx.game.set(g0.clone());
    drag(&mut h, grab_point(&geo, 2, &g0), drop_point(&geo, 0, 0)); // O2 в угол
    let g1 = game(&ctx);
    drag(&mut h, grab_point(&geo, 1, &g1), drop_point(&geo, 7, 5)); // I2
    let g2 = game(&ctx);
    assert_eq!(cell_color(g2.board.get(7, 6)), Family::I2.color());
    drag(&mut h, grab_point(&geo, 0, &g2), drop_point(&geo, 7, 7)); // Mono закрывает ряд
    let g3 = game(&ctx);
    assert!(g3.board.rows()[7].iter().all(|&v| v == 0), "ряд 7 должен схлопнуться");
    assert_eq!(g3.board.filled(), 4, "остался только O2");
    assert!(g3.tray.iter().all(|s| s.is_some()), "после трёх фигур — добор");
    assert_eq!(g3.scoring.lines, 1);
    assert_eq!(g3.scoring.combo, 1);
    assert_eq!(g3.scoring.score, 40 + 20 + 10 + 100);
    // анимации очистки/частицы: несколько кадров без паники
    for _ in 0..60 {
        h.animate(Duration::from_millis(16));
    }
    let _ = h.paint();
}

#[test]
fn keyboard_mode_places_piece() {
    let (mut h, ctx, _geo) = setup(6);
    h.send_events(&press_key(Key::Num2)); // I2, курсор на first_fit = (0,0)
    h.send_events(&press_key(Key::Right));
    h.send_events(&press_key(Key::Right));
    h.send_events(&press_key(Key::Down));
    h.send_events(&press_key(Key::Enter));
    let g = game(&ctx);
    assert_eq!(cell_color(g.board.get(1, 2)), Family::I2.color());
    assert_eq!(cell_color(g.board.get(1, 3)), Family::I2.color());
    assert!(g.tray[1].is_none());
    // курсор не выходит за границы: домино 2 в ширину → максимум колонка 6
    h.send_events(&press_key(Key::Num3)); // O2
    for _ in 0..20 {
        h.send_events(&press_key(Key::Right));
        h.send_events(&press_key(Key::Down));
    }
    h.send_events(&press_key(Key::Enter));
    let g = game(&ctx);
    assert_eq!(cell_color(g.board.get(7, 7)), Family::O2.color());
    assert_eq!(cell_color(g.board.get(6, 6)), Family::O2.color());
}

#[test]
fn escape_opens_menu_and_blocks_board_input() {
    let (mut h, ctx, geo) = setup(7);
    let g0 = game(&ctx);
    assert!(h.send_event(&Event::KeyDown(Key::Escape)).is_handled());
    drag(&mut h, grab_point(&geo, 0, &g0), drop_point(&geo, 4, 4));
    assert_eq!(game(&ctx).board.filled(), 0, "в меню ходы не принимаются");
    h.send_events(&press_key(Key::Escape));
    drag(&mut h, grab_point(&geo, 0, &g0), drop_point(&geo, 4, 4));
    assert_eq!(cell_color(game(&ctx).board.get(4, 4)), Family::Mono.color());
}

#[test]
fn new_game_key_resets_board_and_counts_game() {
    let (mut h, ctx, geo) = setup(8);
    let g0 = game(&ctx);
    drag(&mut h, grab_point(&geo, 0, &g0), drop_point(&geo, 0, 0));
    assert_eq!(game(&ctx).board.filled(), 1);
    h.send_events(&press_key(Key::N));
    let g = game(&ctx);
    assert_eq!(g.board.filled(), 0);
    assert_eq!(g.scoring.score, 0);
    assert!(g.tray.iter().all(|s| s.is_some()));
    let s = ctx.stats.get_untracked();
    assert_eq!(s.games, 1);
    assert_eq!(s.best, 10);
}

#[test]
fn game_over_flow_shows_card_and_restarts_on_click() {
    let (mut h, ctx, geo) = setup(9);
    // шахматное поле: после Mono в (0,0) остаются I2 и O2 — им некуда
    let mut g0 = game(&ctx);
    for r in 0..SIZE {
        for c in 0..SIZE {
            if (r + c) % 2 == 1 {
                g0.board.set(r, c, 4);
            }
        }
    }
    ctx.game.set(g0.clone());
    drag(&mut h, grab_point(&geo, 0, &g0), drop_point(&geo, 0, 0));
    assert!(game(&ctx).over);
    let frames = |h: &mut TestHarness, n: usize| {
        for _ in 0..n {
            h.animate(Duration::from_millis(16));
        }
        let _ = h.paint();
    };
    let card = syn_blocks::ui::overlay::over_card(&geo);
    let btn = card.buttons.iter().find(|b| b.action == syn_blocks::ui::overlay::Action::NewGame).unwrap().rect;
    let p = Point::new(btn.origin.x + btn.width() * 0.5, btn.origin.y + btn.height() * 0.5);
    let click = |h: &mut TestHarness, p: Point| {
        h.send_event(&Event::MouseDown { button: MouseButton::Left, position: p });
        h.send_event(&Event::MouseUp { button: MouseButton::Left, position: p });
    };
    // итоги сами не появляются: поле остаётся на виду
    frames(&mut h, 90);
    // первое касание (даже по месту кнопки) только открывает итоги
    click(&mut h, p);
    assert!(game(&ctx).over, "касание после проигрыша не должно начинать новую партию");
    // тот же тап сразу после открытия итогов тоже не срабатывает
    click(&mut h, p);
    assert!(game(&ctx).over);
    frames(&mut h, 40);
    click(&mut h, p);
    let g = game(&ctx);
    assert!(!g.over);
    assert_eq!(g.board.filled(), 0);
    assert_eq!(ctx.stats.get_untracked().games, 1);
}

#[test]
fn touch_drag_places_piece() {
    let (mut h, ctx, geo) = setup(10);
    let g0 = game(&ctx);
    let from = grab_point(&geo, 1, &g0);
    let to = drop_point(&geo, 2, 2);
    h.send_event(&Event::TouchStart { id: 7, position: from });
    h.send_event(&Event::TouchMove { id: 7, position: Point::new(from.x + 5.0, from.y - 5.0) });
    h.send_event(&Event::TouchMove { id: 7, position: to });
    h.send_event(&Event::TouchEnd { id: 7, position: to });
    let g = game(&ctx);
    assert_eq!(cell_color(g.board.get(2, 2)), Family::I2.color());
}

#[test]
fn paints_every_state_without_panic() {
    let (mut h, ctx, geo) = setup(11);
    let _ = h.paint();
    let g0 = game(&ctx);
    // держим фигуру над полем и рисуем кадр с превью
    h.send_event(&Event::MouseDown { button: MouseButton::Left, position: grab_point(&geo, 1, &g0) });
    h.send_event(&Event::MouseMove(drop_point(&geo, 4, 4)));
    h.animate(Duration::from_millis(16));
    let _ = h.paint();
    h.send_event(&Event::MouseUp { button: MouseButton::Left, position: drop_point(&geo, 4, 4) });
    h.send_events(&press_key(Key::Escape));
    for _ in 0..20 {
        h.animate(Duration::from_millis(16));
    }
    let _ = h.paint();
    // маленькое и широкое окно
    h.layout(340.0, 600.0);
    let _ = h.paint();
    h.layout(1400.0, 700.0);
    let _ = h.paint();
}

#[test]
fn abilities_hammer_and_reroll() {
    use syn_blocks::game::MAX_ENERGY;
    let (mut h, ctx, geo) = setup(12);
    let mut g0 = game(&ctx);
    g0.board.set(3, 3, 5);
    g0.energy = MAX_ENERGY;
    ctx.game.set(g0.clone());
    // молот: W, затем клик по кубику
    h.send_events(&press_key(Key::W));
    let cr = geo.cell_rect(3, 3);
    let p = Point::new(cr.origin.x + cr.width() * 0.5, cr.origin.y + cr.height() * 0.5);
    h.send_event(&Event::MouseDown { button: MouseButton::Left, position: p });
    h.send_event(&Event::MouseUp { button: MouseButton::Left, position: p });
    let g = game(&ctx);
    assert_eq!(g.board.get(3, 3), 0, "молот должен убрать кубик");
    assert_eq!(g.energy, MAX_ENERGY - 4);
    // обмен по кнопке панели
    let before = g.tray;
    let b = geo.abil_btns[0];
    let pb = Point::new(b.origin.x + b.width() * 0.5, b.origin.y + b.height() * 0.5);
    h.send_event(&Event::MouseDown { button: MouseButton::Left, position: pb });
    h.send_event(&Event::MouseUp { button: MouseButton::Left, position: pb });
    let g = game(&ctx);
    assert_ne!(g.tray, before, "обмен меняет фигуры");
    assert_eq!(g.energy, MAX_ENERGY - 4 - 3);
    // без энергии бомба не срабатывает
    let mut g2 = game(&ctx);
    g2.energy = 0;
    ctx.game.set(g2);
    h.send_events(&press_key(Key::E));
    h.send_event(&Event::MouseDown { button: MouseButton::Left, position: p });
    h.send_event(&Event::MouseUp { button: MouseButton::Left, position: p });
    assert_eq!(game(&ctx).energy, 0);
    for _ in 0..30 {
        h.animate(Duration::from_millis(16));
    }
    let _ = h.paint();
}

#[test]
fn difficulty_choice_applies_to_new_game() {
    let (mut h, ctx, geo) = setup(13);
    h.send_events(&press_key(Key::Escape));
    let card = syn_blocks::ui::overlay::menu_card(&geo);
    let hard = card.buttons.iter().find(|b| b.action == syn_blocks::ui::overlay::Action::Difficulty(2)).unwrap().rect;
    let p = Point::new(hard.origin.x + hard.width() * 0.5, hard.origin.y + hard.height() * 0.5);
    h.send_event(&Event::MouseDown { button: MouseButton::Left, position: p });
    h.send_event(&Event::MouseUp { button: MouseButton::Left, position: p });
    assert_eq!(ctx.stats.get_untracked().difficulty, 2);
    h.send_events(&press_key(Key::Escape));
    h.send_events(&press_key(Key::N));
    let g = game(&ctx);
    assert_eq!(g.difficulty, syn_blocks::game::Difficulty::Hard);
    assert_eq!(g.scoring.grace, 1);
}

#[test]
fn game_is_saved_after_move_and_restored() {
    let (mut h, ctx, geo) = setup(21);
    let g0 = game(&ctx);
    drag(&mut h, grab_point(&geo, 1, &g0), drop_point(&geo, 3, 3));
    h.animate(Duration::from_millis(16));
    // папка данных общая для параллельных тестов процесса, поэтому здесь
    // только «записано и читается»; точный формат — persist::tests::game_roundtrip
    assert!(syn_blocks::persist::load_game(syn_blocks::Variant::Standard).is_some(), "партия записана");
    assert_eq!(cell_color(game(&ctx).board.get(3, 3)), Family::I2.color());
}
