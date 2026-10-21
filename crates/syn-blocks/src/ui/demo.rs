//! Демо-состояния для скриншотов: `SYN_BLOCKS_DEMO=start|board|drag|over|lost|menu|clear|finishes`;
//! дополнительно `SYN_BLOCKS_WORLD=1..9` (мир/уровень), `SYN_BLOCKS_SKIN=wood|crystal|gold|lava`
//! и `SYN_BLOCKS_VARIANT=deluxe` (вид игры). Не влияет на обычный запуск.

use super::{Drag, Screen, View};
use crate::game::{cell_color, make_cell, make_cell_full, Board, Cell, Family, Game, Material, Piece, FINISH_COUNT, SIZE};
use crate::persist::Variant;
use syngui::prelude::*;

/// Переменная окружения; на Android окружения нет, поэтому значение можно
/// «вшить» при сборке: `SYN_BLOCKS_DEMO=board SYN_BLOCKS_WORLD=3 ./run-phone.sh`.
fn var(name: &str) -> Option<String> {
    if let Ok(v) = std::env::var(name) {
        return Some(v);
    }
    let built = match name {
        "SYN_BLOCKS_DEMO" => option_env!("SYN_BLOCKS_DEMO"),
        "SYN_BLOCKS_WORLD" => option_env!("SYN_BLOCKS_WORLD"),
        "SYN_BLOCKS_SKIN" => option_env!("SYN_BLOCKS_SKIN"),
        "SYN_BLOCKS_VARIANT" => option_env!("SYN_BLOCKS_VARIANT"),
        _ => None,
    };
    built.filter(|s| !s.is_empty()).map(str::to_string)
}

pub fn apply(v: &mut View) {
    let Some(mode) = var("SYN_BLOCKS_DEMO") else { return };
    v.frozen = true;
    let variant = if var("SYN_BLOCKS_VARIANT").as_deref() == Some("deluxe") { Variant::Deluxe } else { Variant::Standard };
    v.variant = variant;
    v.ctx.variant.set(variant);
    if mode == "start" {
        v.screen = Screen::Start;
        v.ctx.screen.set(Screen::Start);
        v.start_saved = [true, false];
        return;
    }
    v.screen = Screen::Game;
    v.ctx.screen.set(Screen::Game);
    let mut g = Game::new(2027);
    g.board = Board::new();
    // живописное поле: почти полный нижний ряд и колонка, россыпь кубиков
    let fill: &[(usize, usize, Cell)] = &[
        (7, 0, 5), (7, 1, 5), (7, 2, 5), (7, 3, 2), (7, 4, 2), (7, 5, 7),
        (6, 0, 4), (6, 1, 4), (6, 5, 7), (6, 6, 1), (6, 7, 1),
        (5, 0, 4), (5, 6, 1), (5, 7, 1),
        (4, 0, 2), (4, 1, 2), (4, 2, 2), (4, 7, 3),
        (3, 0, 6), (3, 7, 3), (2, 0, 6), (2, 1, 6), (2, 7, 3), (1, 7, 3),
        (0, 3, 5), (0, 4, 5), (1, 3, 1), (1, 4, 1),
    ];
    for &(r, c, col) in fill {
        g.board.set(r, c, col);
    }
    // образцы материалов на поле
    for (r, c, m) in [(0, 3, Material::Wood), (0, 4, Material::Wood), (1, 3, Material::Crystal), (1, 4, Material::Crystal), (6, 6, Material::Gold), (6, 7, Material::Gold), (5, 6, Material::Lava), (5, 7, Material::Lava)] {
        g.board.set(r, c, make_cell(cell_color(g.board.get(r, c)), m));
    }
    // отделки «Люкса»: каждой клетке своя, по диагоналям
    for r in 0..SIZE {
        for c in 0..SIZE {
            let val = g.board.get(r, c);
            if val != 0 {
                let finish = 1 + ((r * 3 + c * 5) % FINISH_COUNT as usize) as u8;
                g.board.set(r, c, make_cell_full(cell_color(val), crate::game::cell_material(val), finish));
            }
        }
    }
    g.tray = [
        Some(Piece::of(Family::I2).with_material(Material::Gold).with_finish(1)),
        Some(Piece::of(Family::T4).with_material(Material::Wood).with_finish(8)),
        Some(Piece::of(Family::L4).with_material(Material::Lava).with_finish(13)),
    ];
    g.scoring.score = 1480;
    g.scoring.combo = 3;
    g.scoring.lines = 9;
    g.scoring.moves = 23;
    g.scoring.best_combo = 3;
    let mut stats = v.ctx.stats.get_untracked();
    stats.best = stats.best.max(2654);
    stats.games = stats.games.max(12);
    v.ctx.stats.set(stats);
    v.fx.score_shown = g.scoring.score as f32;

    if let Some(l) = var("SYN_BLOCKS_WORLD").and_then(|s| s.parse::<u32>().ok()) {
        let l = l.max(1);
        v.level = l;
        let wi = super::world::world_index_for(l, variant);
        v.world = super::world::WorldFade { from: wi, to: wi, born: -100.0 };
        g.scoring.score = super::world::threshold(l) + 600;
    }
    if let Some(sk) = var("SYN_BLOCKS_SKIN") {
        let m = match sk.as_str() {
            "wood" => Some(Material::Wood),
            "crystal" => Some(Material::Crystal),
            "gold" => Some(Material::Gold),
            "lava" => Some(Material::Lava),
            _ => None,
        };
        if let Some(m) = m {
            let mut s = super::world::SkinMode::new(m, -100.0);
            s.moves_left = 9;
            v.skin = Some(s);
        }
    }
    v.fx.score_shown = g.scoring.score as f32;
    match mode.as_str() {
        "drag" => {
            // I2 над клетками (7,6)-(7,7): закроет ряд 7
            let geo = v.geo;
            let target = Point::new(geo.grid.x + 6.0 * geo.cell + geo.cell * 0.5, geo.grid.y + 7.0 * geo.cell + geo.cell * 0.5 + geo.cell * 0.55);
            v.drag = Some(Drag { slot: 0, start: target, cur: target, active: true, touch: true, touch_id: None, grab: (0.5, 0.5), lift: 1.0 });
            v.pointer = target;
        }
        "over" | "lost" => {
            for r in 0..SIZE {
                for c in 0..SIZE {
                    if g.board.get(r, c) == 0 && (r * 3 + c) % 4 != 0 {
                        g.board.set(r, c, ((r + c) % 7 + 1) as Cell);
                    }
                }
            }
            g.over = true;
            v.fx.over_at = Some(-10.0);
            v.over_card = mode == "over";
            v.fx.overlay_t = if v.over_card { 1.0 } else { 0.0 };
        }
        "menu" => {
            v.menu_open = true;
            v.fx.overlay_t = 1.0;
        }
        "clear" => {
            // через 3.6 с I2 из слота 0 закроет ряд 7 при комбо ×3 → ×4
            g.scoring.combo = 3;
            g.scoring.misses = 0;
            v.demo_move_at = Some((3.6, 0, 7, 6));
            v.frozen = false;
        }
        "finishes" => {
            // витрина: все отделки по порядку, три ряда по восемь
            g.board = Board::new();
            for i in 0..FINISH_COUNT as usize {
                g.board.set(i / 8, i % 8, make_cell_full(1 + (i % 7) as u8, Material::Plain, i as u8 + 1));
            }
            g.tray = [
                Some(Piece::of(Family::O2).with_finish(7)),
                Some(Piece::of(Family::L4).with_material(Material::Gold).with_finish(1)),
                Some(Piece::of(Family::I3).with_finish(16)),
            ];
        }
        _ => {}
    }
    v.ctx.game.set(g);
}

/// Отложенный демо-ход (вызывается из `tick`).
pub fn tick(v: &mut View) {
    if let Some((at, slot, r, c)) = v.demo_move_at {
        if v.now >= at {
            v.demo_move_at = None;
            v.commit(slot, r, c);
        }
    }
}
